//! The optimisation problem of a model and its solver (`model.backend`): build, solve,
//! results, and the backend interface.
//!
//! The problem is evaluated from the math into an [`LpModel`] (`calliope_math::eval`) and
//! handed to HiGHS ([`HighsBackend`]). The backend interface changes it in place:
//!
//! - `update_input` changes an input array with Calliope's semantics (new values are
//!   broadcast over the old array's dimensions and NaN keeps the old value). The
//!   dependency index ([`Dependencies`]) gives the components that refer to the input,
//!   directly or through other components; if any part of the problem does, the problem
//!   is re-evaluated and only the rows, columns and objective of those components are
//!   changed in HiGHS (`changeCoeff`/`changeColBounds`/`changeRowBounds`/costs). Only a
//!   change of the sparsity pattern (a `where` that now selects other cells) re-passes
//!   the model.
//! - `update_variable_bounds`, `fix_variable`/`unfix_variable` (which HiGHS lacks in
//!   Python), constraint (de)activation, `set_objective` and `delete_component` work on
//!   the held model directly.

use std::collections::HashSet;
use std::path::Path;

use calliope_array::{Coord, LArray};
use calliope_highs::{HighsBackend, OptionValue, SolveConfig, SolveOutcome};
use calliope_lp::{ComponentKind, Label as LpLabel, LinExpr, LpModel, LpNames};
use calliope_math::Group;
use calliope_math::eval::{BuiltModel, Dependencies, Inputs, Lin, Value, build_model, postprocess};
use calliope_schema::{Build, CalliopeBuildMath, Solve};
use calliope_yaml::{Node, Value as YValue};
use indexmap::IndexMap;

use crate::Error;
use crate::results::{Results, apply_zero_threshold, lin_values};

/// A built optimisation problem and its solver.
#[derive(Debug)]
pub struct Backend {
    config: Build,
    math: CalliopeBuildMath,
    inputs: Inputs,
    /// Arrays of the built components (`built.lp` is moved into `solver`).
    built: BuiltModel,
    solver: HighsBackend,
    deps: Dependencies,
    objective: String,
    verbose_strings: bool,
    /// Components removed by `delete_component`.
    deleted: Vec<(Group, String)>,
}

fn eval_err(e: calliope_math::eval::EvalError) -> Error {
    Error::Backend(e.0)
}

fn kind_of(group: Group) -> Option<ComponentKind> {
    match group {
        Group::Variables => Some(ComponentKind::Variable),
        Group::Constraints => Some(ComponentKind::Constraint),
        Group::PiecewiseConstraints => Some(ComponentKind::Piecewise),
        Group::Objectives => Some(ComponentKind::Objective),
        _ => None,
    }
}

/// HiGHS option values from `config.solve.solver_options`.
pub fn solver_options(opts: &IndexMap<String, Node>) -> Vec<(String, OptionValue)> {
    opts.iter()
        .map(|(k, v)| {
            let o = match &v.value {
                YValue::Bool(b) => OptionValue::Bool(*b),
                YValue::Int(i) => OptionValue::Int(*i),
                YValue::Float(f) => OptionValue::Double(*f),
                YValue::Str(s) => OptionValue::Str(s.clone()),
                other => OptionValue::Str(other.to_string()),
            };
            (k.clone(), o)
        })
        .collect()
}

impl Backend {
    /// `get_model_backend(...).add_optimisation_components()`.
    pub fn new(inputs: Inputs, math: CalliopeBuildMath, config: Build) -> Result<Self, Error> {
        let mut built = build_model(&inputs, &math, &config).map_err(eval_err)?;
        if !built.objectives.contains_key(&config.objective) {
            // bench/BUGS.md #14: Python's HiGHS backend silently solves a zero objective.
            tracing::warn!(
                "The objective `{}` is not defined in the math; solving a feasibility problem (zero objective).",
                config.objective
            );
        }
        let lp = std::mem::take(&mut built.lp);
        let deps = Dependencies::new(&math);
        Ok(Self {
            objective: config.objective.clone(),
            config,
            math,
            inputs,
            built,
            solver: HighsBackend::new(lp),
            deps,
            verbose_strings: false,
            deleted: Vec::new(),
        })
    }

    /// The build configuration.
    pub fn config(&self) -> &Build {
        &self.config
    }

    /// The math the problem was built from.
    pub fn math(&self) -> &CalliopeBuildMath {
        &self.math
    }

    /// The inputs the problem is built on (`backend.inputs`).
    pub fn inputs(&self) -> &Inputs {
        &self.inputs
    }

    /// The problem as currently held (with in-place changes).
    pub fn lp(&self) -> &LpModel {
        self.solver.model()
    }

    /// The dependency index.
    pub fn dependencies(&self) -> &Dependencies {
        &self.deps
    }

    /// The active objective (`backend.objective`).
    pub fn objective(&self) -> &str {
        &self.objective
    }

    /// The solver.
    pub fn solver(&self) -> &HighsBackend {
        &self.solver
    }

    /// `has_integer_or_binary_variables`.
    pub fn has_integer_or_binary_variables(&self) -> bool {
        self.solver.has_integer_or_binary_variables()
    }

    /// `_solve`: solves and loads the results (empty unless a solution exists).
    pub fn solve(&mut self, solve: &Solve, warmstart: bool) -> Result<Results, Error> {
        let cfg = SolveConfig {
            solver_options: solver_options(&solve.solver_options),
            save_logs: solve.save_logs.clone(),
            warmstart,
        };
        let outcome = self.solver.solve(&cfg)?.clone();
        if !outcome.is_optimal() {
            tracing::warn!("Model solution was non-optimal.");
        }
        self.load_results(&outcome, solve)
    }

    /// `load_results`: variables, global expressions, shadow prices, objectives; zero
    /// threshold; postprocessed arrays; all-NaN arrays dropped.
    pub fn load_results(&self, outcome: &SolveOutcome, solve: &Solve) -> Result<Results, Error> {
        let mut out = Results {
            arrays: IndexMap::new(),
            termination_condition: outcome.termination_condition.clone(),
        };
        let Some(x) = outcome.col_value.as_deref() else {
            return Ok(out);
        };
        let a = &mut out.arrays;
        for (name, v) in &self.built.results {
            if let Value::Expr(arr) = v {
                a.insert(name.clone(), lin_values(arr, x));
            }
        }
        for c in solve.shadow_prices.iter() {
            let Some(rows) = self.built.constraints.get(c.as_str()) else {
                tracing::warn!(
                    "Constraint `{c}` is not in the optimisation problem; its shadow prices are not tracked."
                );
                continue;
            };
            let duals = outcome.row_dual.as_deref();
            a.insert(
                format!("shadow_price_{c}"),
                rows.map(|r| match (r, duals) {
                    (Some(r), Some(d)) if (*r as usize) < d.len() => d[*r as usize],
                    _ => f64::NAN,
                }),
            );
        }
        for (name, (_, lin)) in &self.built.objectives {
            a.insert(name.clone(), lin_values(&LArray::scalar(lin.clone()), x));
        }
        if solve.zero_threshold != 0.0 {
            for v in a.values_mut() {
                *v = apply_zero_threshold(v, solve.zero_threshold);
            }
        }
        if solve.postprocessing_active {
            let values: IndexMap<String, Value> = a
                .iter()
                .map(|(k, v)| (k.clone(), Value::Num(v.clone())))
                .collect();
            for (name, v) in
                postprocess(&self.inputs, &self.math, &self.config, &values).map_err(eval_err)?
            {
                a.insert(name, v);
            }
        }
        a.retain(|_, v| v.count() > 0);
        Ok(out)
    }

    /// The build configuration with the active objective.
    fn build_config(&self) -> Build {
        let mut b = self.config.clone();
        b.objective.clone_from(&self.objective);
        b
    }

    /// Re-evaluates the problem and brings the solver to it, changing only the rows,
    /// columns and objective of `affected` components in place.
    fn refresh(&mut self, affected: &[(Group, String)]) -> Result<(), Error> {
        let mut built =
            build_model(&self.inputs, &self.math, &self.build_config()).map_err(eval_err)?;
        let mut lp = std::mem::take(&mut built.lp);
        for (group, name) in &self.deleted {
            remove_component(&mut built, name, *group, &mut |kind, n| {
                Ok(match kind {
                    ComponentKind::Variable => {
                        let cols = lp.cols_of(kind, n);
                        lp.delete_cols(&cols)
                    }
                    _ => {
                        let rows = lp.rows_of(kind, n);
                        lp.delete_rows(&rows)
                    }
                })
            })?;
        }
        let set: HashSet<(ComponentKind, &str)> = affected
            .iter()
            .filter_map(|(g, n)| kind_of(*g).map(|k| (k, n.as_str())))
            .collect();
        let pred = |l: &LpLabel| set.contains(&(l.kind, &*l.component));
        if !self.solver.update_values(&lp, &pred)? {
            tracing::info!(
                "The optimisation problem's structure changed; passing it to the solver again."
            );
            self.solver.replace_model(lp, &pred);
        }
        self.built = built;
        Ok(())
    }

    // ---- inputs --------------------------------------------------------------------

    /// `get_parameter` / `get_lookup`: the input as the backend holds it.
    pub fn get_parameter(&self, name: &str) -> Result<&Value, Error> {
        self.inputs
            .get(name)
            .ok_or_else(|| Error::Key(format!("Unknown parameter: {name}")))
    }

    /// `update_input`: new values for a parameter or lookup.
    ///
    /// As in Calliope, the new values are broadcast over the dimensions of the existing
    /// array and NaN cells keep the existing values. Then every component that depends on
    /// the input is brought up to date (see the module docs).
    pub fn update_input(&mut self, name: &str, new: &Value) -> Result<(), Error> {
        let old = self.inputs.get(name).ok_or_else(|| {
            Error::Key(format!("Component name `{name}` not found in math schema."))
        })?;
        let merged = merge_update(old, new, &self.inputs.coords)?;
        self.inputs.arrays.insert(name.to_owned(), merged);
        let affected = self.deps.affected_by_input(name);
        if affected.iter().any(|(g, _)| *g != Group::Postprocessed) {
            tracing::info!(
                "(parameters, {name}) | The optimisation problem components {:?} will be re-built.",
                affected
                    .iter()
                    .filter(|(g, _)| *g != Group::Postprocessed)
                    .map(|(_, n)| n.as_str())
                    .collect::<Vec<_>>()
            );
            self.refresh(&affected)?;
        }
        Ok(())
    }

    // ---- variables -----------------------------------------------------------------

    fn variable(&self, name: &str) -> Result<&LArray<Lin>, Error> {
        match self.built.results.get(name) {
            Some(Value::Expr(a)) if self.math.variables.contains_key(name) => Ok(a),
            _ => Err(Error::Key(format!("Unknown variable: {name}"))),
        }
    }

    fn last_solution(&self) -> Result<&[f64], Error> {
        self.solver
            .last_outcome()
            .and_then(|o| o.col_value.as_deref())
            .ok_or_else(|| Error::Backend("The optimisation problem has no solution yet.".into()))
    }

    /// `get_variable(name, as_backend_objs=False)`: values at the last solution.
    pub fn get_variable(&self, name: &str) -> Result<LArray<f64>, Error> {
        let a = self.variable(name)?;
        Ok(lin_values(a, self.last_solution()?))
    }

    /// The column of every cell of a variable.
    pub fn variable_columns(&self, name: &str) -> Result<LArray<f64>, Error> {
        #[allow(clippy::cast_precision_loss)]
        Ok(self
            .variable(name)?
            .map(|l| l.terms.first().map_or(f64::NAN, |t| t.0 as f64)))
    }

    /// `get_variable_bounds`: current lower and upper bounds.
    pub fn get_variable_bounds(&self, name: &str) -> Result<(LArray<f64>, LArray<f64>), Error> {
        let a = self.variable(name)?;
        let cols = &self.solver.model().cols;
        let pick = |f: fn(&calliope_lp::Column) -> f64| {
            a.map(|l| l.terms.first().map_or(f64::NAN, |t| f(&cols[t.0 as usize])))
        };
        Ok((pick(|c| c.lb), pick(|c| c.ub)))
    }

    /// `update_variable_bounds`: non-NaN values become the new bounds. Bounds set by
    /// parameters must be changed with `update_input`.
    pub fn update_variable_bounds(
        &mut self,
        name: &str,
        min: Option<&LArray<f64>>,
        max: Option<&LArray<f64>>,
    ) -> Result<(), Error> {
        let a = self.variable(name)?.clone();
        let def = &self.math.variables[name];
        for (which, new, bound) in [("min", min, &def.bounds.min), ("max", max, &def.bounds.max)] {
            if new.is_none() {
                continue;
            }
            if let calliope_schema::BoundValue::Param(p) = bound
                && self.inputs.get(p.as_str()).is_some_and(Value::any_defined)
            {
                return Err(Error::Backend(format!(
                    "Cannot update variable bounds that have been set by parameters. Use `update_input('{p}')` to update the {which} bound of {name}."
                )));
            }
        }
        let min = min.map(|m| broadcast_num(m, a.dims())).transpose()?;
        let max = max.map(|m| broadcast_num(m, a.dims())).transpose()?;
        for (codes, l) in a.cells() {
            let Some(&(col, _)) = l.terms.first() else {
                continue;
            };
            let col = col as usize;
            let c = &self.solver.model().cols[col];
            let lb = min.as_ref().map_or(f64::NAN, |m| m.get(&codes));
            let ub = max.as_ref().map_or(f64::NAN, |m| m.get(&codes));
            let (lb, ub) = (
                if lb.is_nan() { c.lb } else { lb },
                if ub.is_nan() { c.ub } else { ub },
            );
            self.solver.set_col_bounds(col, lb, ub)?;
        }
        Ok(())
    }

    fn mask_cols(&self, name: &str, where_: Option<&LArray<bool>>) -> Result<Vec<bool>, Error> {
        let a = self.variable(name)?;
        let cols = self.solver.model().cols_of(ComponentKind::Variable, name);
        let w = where_.map(|w| broadcast_bool(w, a.dims())).transpose()?;
        let selected: HashSet<usize> = a
            .cells()
            .filter(|(codes, _)| w.as_ref().is_none_or(|w| w.get(codes)))
            .filter_map(|(_, l)| l.terms.first().map(|t| t.0 as usize))
            .collect();
        Ok(cols.iter().map(|c| selected.contains(c)).collect())
    }

    /// `fix_variable`: fixes the variable (where `where_` is true) to its last solved value.
    pub fn fix_variable(&mut self, name: &str, where_: Option<&LArray<bool>>) -> Result<(), Error> {
        let mask = self.mask_cols(name, where_)?;
        self.solver.fix_variable(name, Some(&mask))?;
        Ok(())
    }

    /// `unfix_variable`: restores the bounds from before `fix_variable` (Python's HiGHS
    /// backend raises instead).
    pub fn unfix_variable(
        &mut self,
        name: &str,
        where_: Option<&LArray<bool>>,
    ) -> Result<(), Error> {
        let mask = self.mask_cols(name, where_)?;
        self.solver.unfix_variable(name, Some(&mask))?;
        Ok(())
    }

    // ---- expressions, constraints, objectives --------------------------------------

    /// `get_global_expression(name, eval_body=True)`: values at the last solution.
    pub fn get_global_expression(&self, name: &str) -> Result<LArray<f64>, Error> {
        match self.built.results.get(name) {
            Some(Value::Expr(a)) if self.math.global_expressions.contains_key(name) => {
                Ok(lin_values(a, self.last_solution()?))
            }
            _ => Err(Error::Key(format!("Unknown global_expression: {name}"))),
        }
    }

    /// The affine cells of a variable or global expression.
    pub fn expression(&self, name: &str) -> Option<&LArray<Lin>> {
        match self.built.results.get(name) {
            Some(Value::Expr(a)) => Some(a),
            _ => None,
        }
    }

    /// `get_constraint(name, as_backend_objs=False)`: `lb`, `ub` and, after a solve, the
    /// body value of every row.
    pub fn get_constraint(&self, name: &str) -> Result<ConstraintValues, Error> {
        let rows = self
            .built
            .constraints
            .get(name)
            .ok_or_else(|| Error::Key(format!("Unknown constraint: {name}")))?;
        let m = self.solver.model();
        let activity = self
            .solver
            .last_outcome()
            .and_then(|o| o.col_value.as_deref())
            .map(|x| m.row_activity(x));
        let pick = |f: &dyn Fn(usize) -> f64| {
            rows.map(|r| match r {
                Some(r) if (*r as usize) < m.num_rows() => f(*r as usize),
                _ => f64::NAN,
            })
        };
        Ok(ConstraintValues {
            lb: pick(&|r| m.rows[r].lb),
            ub: pick(&|r| m.rows[r].ub),
            body: activity.map(|act| pick(&|r| act[r])),
        })
    }

    /// `get_objective(name, eval_body=True)`: the objective's value at the last solution.
    pub fn get_objective(&self, name: &str) -> Result<f64, Error> {
        let (_, lin) = self
            .built
            .objectives
            .get(name)
            .ok_or_else(|| Error::Key(format!("Unknown objective: {name}")))?;
        let x = self.last_solution()?;
        Ok(lin.constant
            + lin
                .terms
                .iter()
                .map(|&(c, k)| k * x[c as usize])
                .sum::<f64>())
    }

    /// `set_objective`: makes a built objective the one that is optimised.
    pub fn set_objective(&mut self, name: &str) -> Result<(), Error> {
        let (sense, lin) = self
            .built
            .objectives
            .get(name)
            .ok_or_else(|| Error::Key(format!("Unknown objective: {name}")))?
            .clone();
        let expr = LinExpr {
            terms: lin.terms.iter().map(|&(c, a)| (c as usize, a)).collect(),
            constant: lin.constant,
        };
        self.solver.set_objective(
            LpLabel::new(ComponentKind::Objective, name, []),
            sense,
            expr,
        )?;
        self.objective = name.to_owned();
        tracing::info!("(objectives, {name}) | Objective activated.");
        Ok(())
    }

    /// Deactivates (frees the rows of) a constraint, or activates it again.
    pub fn set_constraint_active(&mut self, name: &str, active: bool) -> Result<(), Error> {
        if !self.built.constraints.contains_key(name)
            && !self.math.piecewise_constraints.contains_key(name)
        {
            return Err(Error::Key(format!("Unknown constraint: {name}")));
        }
        self.solver.set_constraint_active(name, active)?;
        Ok(())
    }

    /// `delete_component`: removes a variable, global expression, constraint or objective
    /// from the problem. It stays deleted when later updates re-evaluate the problem.
    /// Unknown names are ignored, as in Calliope.
    pub fn delete_component(&mut self, name: &str, group: Group) -> Result<(), Error> {
        if let Some(kind @ (ComponentKind::Constraint | ComponentKind::Piecewise)) = kind_of(group)
            && self.solver.model().rows_of(kind, name).is_empty()
            && !self.built.constraints.contains_key(name)
        {
            return Ok(());
        }
        let solver = &mut self.solver;
        let deleted = remove_component(&mut self.built, name, group, &mut |kind, n| {
            Ok(solver.delete_component(kind, n)?)
        })?;
        if deleted {
            self.deleted.push((group, name.to_owned()));
        }
        Ok(())
    }

    // ---- output --------------------------------------------------------------------

    /// `verbose_strings`: LP files name rows and columns by their component and index.
    pub fn verbose_strings(&mut self) {
        self.verbose_strings = true;
    }

    /// The LP file text.
    pub fn lp_string(&self) -> String {
        let names = if self.verbose_strings {
            LpNames::Verbose
        } else {
            LpNames::Anonymous
        };
        calliope_lp::write_lp_string(self.solver.model(), names)
    }

    /// Writes the problem with HiGHS' own writer (`.lp` or `.mps`), in HiGHS' column and
    /// row order, as Python's HiGHS backend `to_lp` does without `verbose_strings`.
    pub fn write_solver_model(&mut self, path: &Path) -> Result<(), Error> {
        self.solver.write_model(path)?;
        Ok(())
    }

    /// `to_lp`: writes the problem as a CPLEX LP file (`.lp` extension required).
    pub fn to_lp(&self, path: &Path) -> Result<(), Error> {
        if path.extension().and_then(|e| e.to_str()) != Some("lp") {
            return Err(Error::Value("File extension must be `.lp`".into()));
        }
        std::fs::write(path, self.lp_string())
            .map_err(|e| Error::Io(calliope_io::IoError::Format(e.to_string())))
    }
}

/// `get_constraint(as_backend_objs=False)`.
#[derive(Debug, Clone)]
pub struct ConstraintValues {
    /// Row lower bounds (`-inf` if none).
    pub lb: LArray<f64>,
    /// Row upper bounds (`inf` if none).
    pub ub: LArray<f64>,
    /// Row activity at the last solution.
    pub body: Option<LArray<f64>>,
}

type Deleter<'a> = dyn FnMut(ComponentKind, &str) -> Result<Vec<Option<usize>>, Error> + 'a;

/// Removes a component from the built arrays; `delete` removes its columns or rows from
/// the problem and returns the old → new index map. Returns whether it existed.
fn remove_component(
    built: &mut BuiltModel,
    name: &str,
    group: Group,
    delete: &mut Deleter<'_>,
) -> Result<bool, Error> {
    match group {
        Group::Variables => {
            if !matches!(built.results.get(name), Some(Value::Expr(_))) {
                return Ok(false);
            }
            let map = delete(ComponentKind::Variable, name)?;
            built.results.shift_remove(name);
            remap_cols(built, &map);
            Ok(true)
        }
        Group::Constraints | Group::PiecewiseConstraints => {
            let kind = kind_of(group).expect("constraint kind");
            let had = built.constraints.contains_key(name);
            let map = delete(kind, name)?;
            if !had && map.iter().all(Option::is_some) {
                return Ok(false);
            }
            built.constraints.shift_remove(name);
            for rows in built.constraints.values_mut() {
                *rows = rows.map(|r| {
                    r.and_then(|r| map.get(r as usize).copied().flatten())
                        .map(|r| u32::try_from(r).expect("fits"))
                });
            }
            Ok(true)
        }
        Group::GlobalExpressions => Ok(built.results.shift_remove(name).is_some()),
        Group::Objectives => Ok(built.objectives.shift_remove(name).is_some()),
        Group::Postprocessed | Group::Checks => Ok(false),
    }
}

fn remap_cols(built: &mut BuiltModel, map: &[Option<usize>]) {
    let fix = |l: &Lin| -> Lin {
        let terms: Vec<(u32, f64)> = l
            .terms
            .iter()
            .filter_map(|&(c, a)| {
                map.get(c as usize)
                    .copied()
                    .flatten()
                    .map(|n| (u32::try_from(n).expect("fits"), a))
            })
            .collect();
        if terms.is_empty() && !l.terms.is_empty() {
            // Every variable of the cell was deleted: what is left is its constant.
            Lin::num(if l.constant.is_nan() { 0.0 } else { l.constant })
        } else {
            Lin {
                constant: l.constant,
                terms,
            }
        }
    };
    for v in built.results.values_mut() {
        if let Value::Expr(a) = v {
            *a = a.map(fix);
        }
    }
    for (_, l) in built.objectives.values_mut() {
        *l = fix(l);
    }
}

/// Reads `a` at the cells of `dims` (matching labels by dimension name); dims `a` lacks are
/// broadcast, labels it lacks are missing.
pub(crate) fn broadcast<T: calliope_array::Element>(
    a: &LArray<T>,
    dims: &[Coord],
) -> Result<LArray<T>, Error> {
    for d in a.dims() {
        if !dims.iter().any(|t| t.name() == d.name()) {
            return Err(Error::Value(format!(
                "new values are indexed over `{}`, which the target does not have",
                d.name()
            )));
        }
    }
    let maps: Vec<(usize, Vec<Option<u32>>)> = a
        .dims()
        .iter()
        .map(|d| {
            let p = dims
                .iter()
                .position(|t| t.name() == d.name())
                .expect("checked");
            (p, dims[p].labels().iter().map(|l| d.code(l)).collect())
        })
        .collect();
    let shape: Vec<usize> = dims.iter().map(Coord::len).collect();
    let total: usize = shape.iter().product();
    let mut cells = Vec::new();
    let mut idx = vec![0u32; dims.len()];
    for _ in 0..total {
        let own: Option<Vec<u32>> = maps.iter().map(|(p, m)| m[idx[*p] as usize]).collect();
        if let Some(own) = own {
            let v = a.get(&own);
            if !v.is_missing() {
                cells.push((idx.clone(), v));
            }
        }
        for i in (0..idx.len()).rev() {
            idx[i] += 1;
            if (idx[i] as usize) < shape[i] {
                break;
            }
            idx[i] = 0;
        }
    }
    Ok(LArray::from_cells(dims.to_vec(), cells)?)
}

fn broadcast_num(a: &LArray<f64>, dims: &[Coord]) -> Result<LArray<f64>, Error> {
    broadcast(a, dims)
}

fn broadcast_bool(a: &LArray<bool>, dims: &[Coord]) -> Result<LArray<bool>, Error> {
    broadcast(a, dims)
}

/// `new_values.broadcast_like(input_da).fillna(input_da)` over the backend's coordinates.
pub(crate) fn merge_update(
    old: &Value,
    new: &Value,
    coords: &IndexMap<String, Coord>,
) -> Result<Value, Error> {
    let target_dims = |a: Vec<String>, b: Vec<String>| -> Result<Vec<Coord>, Error> {
        let mut names = b;
        for n in a {
            if !names.contains(&n) {
                names.push(n);
            }
        }
        names
            .iter()
            .map(|n| {
                coords
                    .get(n)
                    .cloned()
                    .ok_or_else(|| Error::Value(format!("Unknown dimension `{n}`")))
            })
            .collect()
    };
    let dims = target_dims(old.dim_names(), new.dim_names())?;
    Ok(match old {
        Value::Bool(o) => {
            let n = broadcast(&new.to_bool().map_err(eval_err)?, &dims)?;
            // Bool arrays have no missing state: the new values replace the old.
            let _ = o;
            Value::Bool(n)
        }
        Value::Label(o) => {
            let n = broadcast(&new.to_label().map_err(eval_err)?, &dims)?;
            Value::Label(n.fillna_with(&broadcast(o, &dims)?))
        }
        _ => {
            let n = broadcast(&new.to_num().map_err(eval_err)?, &dims)?;
            let o = broadcast(&old.to_num().map_err(eval_err)?, &dims)?;
            Value::Num(n.fillna_with(&o))
        }
    })
}
