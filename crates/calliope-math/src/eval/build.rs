//! Building the optimisation problem (`BackendModel.add_optimisation_components`): variables,
//! global expressions, constraints, piecewise constraints and objectives, emitted into an
//! [`LpModel`] with Calliope's (Pyomo's) names.

use std::collections::HashSet;
use std::sync::Arc;

use calliope_array::{Coord, Element, LArray, Label};
use calliope_lp::{ComponentKind, LinExpr, LpModel, ObjSense};
use calliope_schema::{BoundValue, Build, CalliopeBuildMath, CheckErrors, Domain, Sense};
use indexmap::IndexMap;

use super::EvalError;
use super::expr::Evaluator;
use super::inputs::{Inputs, numeric};
use super::lin::Lin;
use super::value::Value;
use crate::ast::CmpOp;
use crate::component::{Equation, Group, ParseMode, ParsedComponent, parse_math};
use crate::ir::{Body, Ir, LoweredComponent, LoweredEquation, lower};

type Result<T> = std::result::Result<T, EvalError>;

/// The built problem and the labelled arrays behind it.
#[derive(Debug, Clone)]
pub struct BuiltModel {
    /// The LP/MILP.
    pub lp: LpModel,
    /// Variables and global expressions (cells are [`Lin`] over `lp` columns).
    pub results: IndexMap<String, Value>,
    /// Constraint rows: for each constraint, the row id of each built cell.
    pub constraints: IndexMap<String, LArray<Option<u32>>>,
    /// Objectives (all built ones; `lp.objective` is the one `build.objective` names).
    pub objectives: IndexMap<String, (ObjSense, Lin)>,
}

/// `BackendModel.add_optimisation_components`.
pub fn build_model(inputs: &Inputs, math: &CalliopeBuildMath, build: &Build) -> Result<BuiltModel> {
    let (comps, diags) = parse_math(math, ParseMode::Build);
    let errors: Vec<String> = diags
        .iter()
        .filter(|d| d.is_error())
        .map(crate::MathDiagnostic::short)
        .collect();
    if !errors.is_empty() {
        return Err(EvalError(format!(
            "Errors during math string parsing:\n * {}",
            errors.join("\n * ")
        )));
    }
    let config = serde_json::to_value(build).map_err(|e| EvalError(e.to_string()))?;
    let mut b = Builder {
        inputs,
        math,
        config,
        objective: build.objective.clone(),
        out: BuiltModel {
            lp: LpModel::new(),
            results: IndexMap::new(),
            constraints: IndexMap::new(),
            objectives: IndexMap::new(),
        },
    };
    let timing = std::env::var_os("CALLIOPE_BUILD_TIMING").is_some();
    for comp in &comps {
        let start = std::time::Instant::now();
        match comp.group {
            Group::Variables => b.variable(comp)?,
            Group::GlobalExpressions => b.global_expression(comp)?,
            Group::Constraints => b.constraint(comp)?,
            Group::PiecewiseConstraints => b.piecewise(comp)?,
            Group::Objectives => b.objective(comp)?,
            Group::Postprocessed | Group::Checks => {}
        }
        if timing {
            eprintln!(
                "{:>8.3} s  {}:{}",
                start.elapsed().as_secs_f64(),
                comp.group.as_str(),
                comp.name
            );
        }
    }
    Ok(b.out)
}

/// The outcome of the math `checks` (`BackendModel._check_inputs`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CheckOutcome {
    /// Messages of failed `warn` checks.
    pub warnings: Vec<String>,
    /// Messages of failed `raise` checks.
    pub errors: Vec<String>,
}

impl CheckOutcome {
    /// Calliope's `ModelError` text if any `raise` check failed.
    pub fn error_message(&self) -> Option<String> {
        (!self.errors.is_empty()).then(|| bullets("Errors during model processing:", &self.errors))
    }

    /// Calliope's warning text if any `warn` check failed.
    pub fn warning_message(&self) -> Option<String> {
        (!self.warnings.is_empty()).then(|| {
            bullets(
                "Possible issues found during model processing:",
                &self.warnings,
            )
        })
    }
}

fn bullets(head: &str, items: &[String]) -> String {
    let mut sorted: Vec<&String> = items.iter().collect();
    sorted.sort();
    sorted.dedup();
    let mut s = head.to_owned();
    for i in sorted {
        s.push_str("\n * ");
        s.push_str(i);
    }
    s
}

/// Evaluates the active math `checks`: a check fails if its `where` is true anywhere that
/// `active` is.
pub fn run_checks(
    inputs: &Inputs,
    math: &CalliopeBuildMath,
    build: &Build,
) -> Result<CheckOutcome> {
    let (comps, diags) = parse_math(math, ParseMode::Build);
    if let Some(d) = diags.iter().find(|d| d.is_error()) {
        return Err(EvalError(d.short()));
    }
    let config = serde_json::to_value(build).map_err(|e| EvalError(e.to_string()))?;
    let results = IndexMap::new();
    let ev = Evaluator {
        inputs,
        math,
        config: &config,
        results: &results,
    };
    let active = match inputs.get("active") {
        Some(a) => a.to_bool()?,
        None => LArray::scalar(true),
    };
    let mut out = CheckOutcome::default();
    for comp in comps.iter().filter(|c| c.group == Group::Checks) {
        let low = lower(comp, math);
        let evaluated = ev.eval(&low.mask.where_, None)?.to_bool()?;
        if evaluated.count() > 0 && evaluated.and(&active).count() > 0 {
            let check = &math.checks[comp.name.as_str()];
            match check.errors {
                CheckErrors::Raise => out.errors.push(check.message.clone()),
                CheckErrors::Warn => out.warnings.push(check.message.clone()),
            }
        }
    }
    Ok(out)
}

/// `pre_validate_math_strings`: parses every math string, active or not. Returns Calliope's
/// error text if any fails to parse.
pub fn pre_validate_math_strings(math: &CalliopeBuildMath) -> std::result::Result<(), String> {
    let (_, diags) = parse_math(math, ParseMode::PreValidate);
    let mut by_comp: IndexMap<String, Vec<String>> = IndexMap::new();
    for d in diags.iter().filter(|d| d.is_error()) {
        by_comp
            .entry(format!("{}:{}", d.path.group, d.path.component))
            .or_default()
            .push(d.short());
    }
    if by_comp.is_empty() {
        return Ok(());
    }
    let mut s = "Errors during math string parsing (marker indicates where parsing stopped, but may not point to the root cause of the issue):".to_owned();
    for (k, v) in by_comp {
        s.push_str(&format!("\n * {k}:"));
        let mut v = v;
        v.sort();
        v.dedup();
        for e in v {
            s.push_str(&format!("\n    * {e}"));
        }
    }
    Err(s)
}

struct Builder<'a> {
    inputs: &'a Inputs,
    math: &'a CalliopeBuildMath,
    config: serde_json::Value,
    objective: String,
    out: BuiltModel,
}

/// The index labels of each dimension, as Calliope prints them in names (datetimes with
/// minute resolution, `_datetime_as_string`).
fn index_labels(dims: &[Coord]) -> Vec<Vec<Arc<str>>> {
    dims.iter()
        .map(|c| c.labels().iter().map(|l| Arc::from(label_str(l))).collect())
        .collect()
}

/// `str()` of a coordinate label in a component name.
pub fn label_str(l: &Label) -> String {
    match l {
        Label::Str(s) => s.to_string(),
        Label::Int(i) => i.to_string(),
        Label::Float(f) => calliope_yaml::python_float(*f),
        Label::Datetime(ns) => {
            let (y, m, d, hh, mm) = crate::helpers::array::datetime_parts(*ns);
            format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}")
        }
    }
}

/// The true cells of a mask in C order of its dims.
fn true_cells(mask: &LArray<bool>) -> Vec<Vec<u32>> {
    let mut cells: Vec<Vec<u32>> = mask.cells().filter(|(_, v)| **v).map(|(c, _)| c).collect();
    let dense_last = mask
        .dense_dim()
        .is_none_or(|d| mask.dims().last().is_some_and(|l| l.name() == d));
    if !dense_last {
        cells.sort();
    }
    cells
}

/// Reads an array at the cells of a target dims list (whose dims are a superset).
struct Picker<'a, T: Element> {
    array: &'a LArray<T>,
    /// Per array dim: position in the target and a target-code → array-code map (None if
    /// the coordinates are equal).
    axes: Vec<(usize, Option<Vec<Option<u32>>>)>,
}

impl<'a, T: Element> Picker<'a, T> {
    fn new(array: &'a LArray<T>, target: &[Coord]) -> Option<Self> {
        let mut axes = Vec::new();
        for d in array.dims() {
            let p = target.iter().position(|t| t.name() == d.name())?;
            let map = (target[p] != *d).then(|| {
                target[p]
                    .labels()
                    .iter()
                    .map(|l| d.code(l))
                    .collect::<Vec<_>>()
            });
            axes.push((p, map));
        }
        Some(Self { array, axes })
    }

    fn at(&self, codes: &[u32]) -> T {
        let mut own = Vec::with_capacity(self.axes.len());
        for (p, map) in &self.axes {
            let c = codes[*p];
            match map {
                None => own.push(c),
                Some(m) => match m[c as usize] {
                    Some(x) => own.push(x),
                    None => return T::missing(),
                },
            }
        }
        self.array.get(&own)
    }
}

fn extra_dims(v: &Value, target: &[Coord]) -> Vec<String> {
    v.dim_names()
        .into_iter()
        .filter(|d| !target.iter().any(|t| t.name() == d))
        .collect()
}

fn py_set(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|i| format!("'{i}'")).collect();
    format!("{{{}}}", inner.join(", "))
}

impl Builder<'_> {
    fn ev(&self) -> Evaluator<'_> {
        Evaluator {
            inputs: self.inputs,
            math: self.math,
            config: &self.config,
            results: &self.out.results,
        }
    }

    /// `generate_top_level_where_array`: `active` × defined `foreach` items × `where`,
    /// reduced to `foreach` with "any". `None` if it is false everywhere.
    fn top_where(&self, low: &LoweredComponent) -> Result<Option<LArray<bool>>> {
        let ev = self.ev();
        let foreach = &low.mask.foreach;
        let w = ev.foreach_mask(foreach)?;
        if w.count() == 0 {
            return Ok(None);
        }
        let w = w.and(&ev.eval(&low.mask.where_, None)?.to_bool()?);
        if w.count() == 0 {
            return Ok(None);
        }
        drop_dims_not_in_foreach(&w, foreach).map(Some)
    }

    /// The equation's `where` on top of the component's, reduced to `foreach`.
    fn equation_where(
        &self,
        top: &LArray<bool>,
        eq: &LoweredEquation,
        foreach: &[String],
    ) -> Result<Option<LArray<bool>>> {
        let ev = self.ev();
        let mut w = top.clone();
        for wh in &eq.wheres {
            w = w.and(&ev.eval(wh, None)?.to_bool()?);
        }
        if w.count() == 0 {
            return Ok(None);
        }
        drop_dims_not_in_foreach(&w, foreach).map(Some)
    }

    fn variable(&mut self, comp: &ParsedComponent) -> Result<()> {
        let low = lower(comp, self.math);
        let Some(w) = self.top_where(&low)? else {
            self.out.results.insert(
                comp.name.clone(),
                Value::Expr(LArray::scalar(Lin::missing())),
            );
            return Ok(());
        };
        let def = &self.math.variables[comp.name.as_str()];
        let bound = |b: &BoundValue| -> Result<LArray<f64>> {
            match b {
                BoundValue::Value(n) => Ok(LArray::scalar(numeric(n))),
                BoundValue::Param(p) => self.ev().input_values(p.as_str())?.to_num(),
            }
        };
        let (lb, ub) = (bound(&def.bounds.min)?, bound(&def.bounds.max)?);
        let dims = w.dims().to_vec();
        let pick = |a: &LArray<f64>, which: &str| -> Result<()> {
            Picker::new(a, &dims).map(|_| ()).ok_or_else(|| {
                EvalError(format!(
                    "(variables, {}) | The {which} bound is indexed over dimensions not present in `foreach`",
                    comp.name
                ))
            })
        };
        pick(&lb, "lower")?;
        pick(&ub, "upper")?;
        let (lbp, ubp) = (
            Picker::new(&lb, &dims).expect("checked"),
            Picker::new(&ub, &dims).expect("checked"),
        );
        let integer = def.domain == Domain::Integer;
        let labels = index_labels(&dims);
        let name: Arc<str> = Arc::from(comp.name.as_str());
        let mut cells = Vec::new();
        for codes in true_cells(&w) {
            let label = calliope_lp::Label::new(
                ComponentKind::Variable,
                name.clone(),
                codes
                    .iter()
                    .zip(&labels)
                    .map(|(c, l)| l[*c as usize].clone()),
            );
            let col = self
                .out
                .lp
                .add_col(label, lbp.at(&codes), ubp.at(&codes), integer);
            cells.push((codes, Lin::var(u32::try_from(col).expect("fits u32"))));
        }
        let arr = LArray::from_cells(dims, cells).map_err(|e| EvalError(e.to_string()))?;
        self.out.results.insert(comp.name.clone(), Value::Expr(arr));
        Ok(())
    }

    /// `_eval_equations` for expression-valued components.
    fn global_expression(&mut self, comp: &ParsedComponent) -> Result<()> {
        let low = lower(comp, self.math);
        let value = match self.top_where(&low)? {
            None => LArray::scalar(Lin::missing()),
            Some(w) => {
                let foreach = &low.mask.foreach;
                let mut acc: LArray<Lin> =
                    LArray::empty(w.dims().to_vec()).map_err(|e| EvalError(e.to_string()))?;
                for eq in &low.equations {
                    let Some(ew) = self.equation_where(&w, eq, foreach)? else {
                        continue;
                    };
                    overlap(&eq.name, &acc, &ew)?;
                    let Body::Expression(e) = &eq.body else {
                        unreachable!("global expressions have no comparison")
                    };
                    let v = self.eval_expression(&eq.name, e, &ew)?;
                    let v = v.to_expr()?;
                    acc = acc.fillna_with(&conform(&v, w.dims())?);
                }
                if acc.count() == 0 {
                    LArray::scalar(Lin::missing())
                } else {
                    acc
                }
            }
        };
        self.out
            .results
            .insert(comp.name.clone(), Value::Expr(value));
        Ok(())
    }

    /// Evaluates an equation expression masked by `ew` and checks it is defined where `ew`
    /// is true (`raise_error_on_where_expr_mismatch`).
    fn eval_expression(&self, eq_name: &str, e: &Ir, ew: &LArray<bool>) -> Result<Value> {
        let mask = mask_of(ew);
        let v = self.ev().eval(e, mask)?;
        let extra = extra_dims(&v, ew.dims());
        if !extra.is_empty() {
            return Err(EvalError(format!(
                "{eq_name} | The linear expression array is indexed over dimensions not present in `foreach`: {}",
                py_set(&extra)
            )));
        }
        let missing = match &v {
            Value::Num(a) => any_missing_at(a, ew),
            Value::Expr(a) => any_missing_at(a, ew),
            Value::Label(a) => any_missing_at(a, ew),
            _ => false,
        };
        if missing {
            return Err(EvalError(format!(
                "{eq_name} | Missing a linear expression for some coordinates selected by 'where'. Adapting 'where' might help."
            )));
        }
        Ok(v)
    }

    fn constraint(&mut self, comp: &ParsedComponent) -> Result<()> {
        let low = lower(comp, self.math);
        let Some(w) = self.top_where(&low)? else {
            return Ok(());
        };
        let foreach = &low.mask.foreach;
        let dims = w.dims().to_vec();
        let labels = index_labels(&dims);
        let name: Arc<str> = Arc::from(comp.name.as_str());
        let mut done: LArray<Option<u32>> =
            LArray::empty(dims.clone()).map_err(|e| EvalError(e.to_string()))?;
        let mut rows: Vec<(Vec<u32>, Option<u32>)> = Vec::new();
        for eq in &low.equations {
            let Some(ew) = self.equation_where(&w, eq, foreach)? else {
                continue;
            };
            overlap(&eq.name, &done, &ew)?;
            let Body::Constraint { lhs, op, rhs } = &eq.body else {
                return Err(EvalError(format!(
                    "{} | constraint equations need a comparison",
                    eq.name
                )));
            };
            let mask = mask_of(&ew);
            let ev = self.ev();
            let (l, r) = (ev.eval(lhs, mask)?, ev.eval(rhs, mask)?);
            for (side, v) in [("left", &l), ("right", &r)] {
                let extra = extra_dims(v, ew.dims());
                if !extra.is_empty() {
                    return Err(EvalError(format!(
                        "({}, {}) | The {side}-hand side of the equation is indexed over dimensions not present in `foreach`: {}",
                        eq.name,
                        comp.equations
                            .iter()
                            .find(|e| e.name == eq.name)
                            .map_or("", |e| e.expression.source.trim()),
                        py_set(&extra)
                    )));
                }
            }
            let (l, r) = (l.to_expr()?, r.to_expr()?);
            let (lp_, rp) = (
                Picker::new(&l, ew.dims()).expect("checked"),
                Picker::new(&r, ew.dims()).expect("checked"),
            );
            let mut booleans = Vec::new();
            let mut eq_rows = Vec::new();
            for codes in true_cells(&ew) {
                let (lv, rv) = (lp_.at(&codes), rp.at(&codes));
                if !lv.is_expr() && !rv.is_expr() {
                    booleans.push(codes);
                    continue;
                }
                if lv.is_missing() || rv.is_missing() {
                    return Err(EvalError(format!(
                        "{} | Missing a linear expression for some coordinates selected by 'where'. Adapting 'where' might help.",
                        eq.name
                    )));
                }
                let label = calliope_lp::Label::new(
                    ComponentKind::Constraint,
                    name.clone(),
                    codes
                        .iter()
                        .zip(&labels)
                        .map(|(c, l)| l[*c as usize].clone()),
                );
                let row = add_constraint_row(&mut self.out.lp, label, *op, &lv, &rv);
                eq_rows.push((codes, row));
            }
            if !booleans.is_empty() {
                let idx: Vec<String> = booleans
                    .iter()
                    .map(|c| {
                        let parts: Vec<String> = c
                            .iter()
                            .zip(&labels)
                            .map(|(c, l)| format!("'{}'", l[*c as usize]))
                            .collect();
                        format!("({})", parts.join(", "))
                    })
                    .collect();
                return Err(EvalError(format!(
                    "(constraints, {}) | constraint array includes item(s) that resolves to a simple boolean. There must be a math component defined on at least one side of the equation: [{}]",
                    comp.name,
                    idx.join(" ")
                )));
            }
            let built: Vec<(Vec<u32>, Option<u32>)> = eq_rows
                .iter()
                .map(|(c, r)| (c.clone(), Some(r.unwrap_or(u32::MAX))))
                .collect();
            done = done.fillna_with(
                &LArray::from_cells(dims.clone(), built).map_err(|e| EvalError(e.to_string()))?,
            );
            rows.extend(eq_rows);
        }
        let rows = LArray::from_cells(dims, rows).map_err(|e| EvalError(e.to_string()))?;
        self.out.constraints.insert(comp.name.clone(), rows);
        Ok(())
    }

    fn objective(&mut self, comp: &ParsedComponent) -> Result<()> {
        let low = lower(comp, self.math);
        let Some(w) = self.top_where(&low)? else {
            return Ok(());
        };
        let def = &self.math.objectives[comp.name.as_str()];
        let sense = match def.sense {
            Sense::Minimise | Sense::Minimize => ObjSense::Minimize,
            Sense::Maximise | Sense::Maximize => ObjSense::Maximize,
        };
        let mut value: Option<Lin> = None;
        for eq in &low.equations {
            let Some(ew) = self.equation_where(&w, eq, &low.mask.foreach)? else {
                continue;
            };
            if value.is_some() {
                return Err(EvalError(format!(
                    "{} | trying to set two equations for the same component.",
                    eq.name
                )));
            }
            let Body::Expression(e) = &eq.body else {
                unreachable!("objectives have no comparison")
            };
            let v = self.eval_expression(&eq.name, e, &ew)?.to_expr()?;
            value = Some(v.get(&[]));
        }
        let Some(lin) = value else { return Ok(()) };
        if comp.name == self.objective {
            let mut terms: Vec<(usize, f64)> =
                lin.terms.iter().map(|&(c, a)| (c as usize, a)).collect();
            terms.shrink_to_fit();
            self.out.lp.set_objective(
                calliope_lp::Label::new(ComponentKind::Objective, comp.name.as_str(), []),
                sense,
                LinExpr {
                    terms,
                    constant: lin.constant,
                },
            );
        }
        self.out.objectives.insert(comp.name.clone(), (sense, lin));
        Ok(())
    }

    #[allow(clippy::too_many_lines)]
    fn piecewise(&mut self, comp: &ParsedComponent) -> Result<()> {
        let name = comp.name.as_str();
        if comp.foreach.iter().any(|d| d == "breakpoints") {
            return Err(EvalError(format!(
                "(piecewise_constraints, {name}) | `breakpoints` dimension should not be in `foreach`. Instead, index `x_values` and `y_values` parameters over `breakpoints`."
            )));
        }
        let low = lower(comp, self.math);
        let Some(w) = self.top_where(&low)? else {
            return Ok(());
        };
        let axes = comp.piecewise.as_ref().expect("parsed with its axes");
        let mut exprs = Vec::new();
        let mut vals = Vec::new();
        for (axis, expr, values) in [
            ("x", &axes.x_expression, &axes.x_values),
            ("y", &axes.y_expression, &axes.y_values),
        ] {
            let mut c = comp.clone();
            c.equations = vec![Equation {
                name: format!("piecewise_constraints:{name}:0"),
                expression: expr.clone(),
                wheres: Vec::new(),
                sub_expressions: IndexMap::new(),
                slices: IndexMap::new(),
            }];
            let l = lower(&c, self.math);
            let Body::Expression(e) = &l.equations[0].body else {
                unreachable!("an arithmetic string")
            };
            let v = self
                .eval_expression(&l.equations[0].name, e, &w)?
                .to_expr()?;
            let raw = match self.inputs.get(values) {
                Some(v) => v.to_num()?,
                None => {
                    return Err(EvalError(format!(
                        "(piecewise_constraints, {name}) | Unknown parameter: {values}"
                    )));
                }
            };
            if raw.coord("breakpoints").is_none() {
                return Err(EvalError(format!(
                    "(piecewise_constraints, {name}) | `{axis}_values` must be indexed over the `breakpoints` dimension."
                )));
            }
            exprs.push(v);
            vals.push(raw);
        }
        let dims = w.dims().to_vec();
        let mut with_bp = dims.clone();
        let bp = self
            .inputs
            .coords
            .get("breakpoints")
            .cloned()
            .or_else(|| vals[0].coord("breakpoints").cloned())
            .expect("checked");
        with_bp.push(bp.clone());
        let pickers: Vec<Picker<'_, f64>> = vals
            .iter()
            .map(|v| {
                Picker::new(v, &with_bp).ok_or_else(|| {
                    EvalError(format!(
                        "(piecewise_constraints, {name}) | breakpoint values are indexed over dimensions not present in `foreach`"
                    ))
                })
            })
            .collect::<Result<_>>()?;
        let (xp, yp) = (
            Picker::new(&exprs[0], &dims).expect("checked"),
            Picker::new(&exprs[1], &dims).expect("checked"),
        );
        let labels = index_labels(&dims);
        for codes in true_cells(&w) {
            let series = |p: &Picker<'_, f64>| -> Vec<f64> {
                (0..bp.len())
                    .map(|i| {
                        let mut c = codes.clone();
                        c.push(u32::try_from(i).expect("fits"));
                        p.at(&c)
                    })
                    .filter(|v| !v.is_nan())
                    .collect()
            };
            let (xs, ys) = (series(&pickers[0]), series(&pickers[1]));
            let prefix = format!(
                "(piecewise_constraints, {name}) | Errors in generating piecewise constraint: "
            );
            if xs.len() != ys.len() {
                return Err(EvalError(format!(
                    "{prefix}The number of breakpoints ({}) differs from the number of function values ({})",
                    xs.len(),
                    ys.len()
                )));
            }
            if xs.windows(2).any(|p| p[1] < p[0]) {
                return Err(EvalError(format!(
                    "{prefix}The list of breakpoints is not nondecreasing: {xs:?}"
                )));
            }
            let (x, y) = (xp.at(&codes), yp.at(&codes));
            let index: Vec<Arc<str>> = codes
                .iter()
                .zip(&labels)
                .map(|(c, l)| l[*c as usize].clone())
                .collect();
            let label = |sfx: String| {
                calliope_lp::Label::new(ComponentKind::Piecewise, name, index.iter().cloned())
                    .with_suffix(sfx)
            };
            let v: Vec<usize> = (0..xs.len())
                .map(|i| {
                    self.out
                        .lp
                        .add_col(label(format!("_v({i})")), 0.0, f64::INFINITY, false)
                })
                .collect();
            for (i, (pts, io)) in [(&xs, &x), (&ys, &y)].into_iter().enumerate() {
                let mut terms: Vec<(usize, f64)> =
                    v.iter().copied().zip(pts.iter().copied()).collect();
                terms.extend(io.terms.iter().map(|&(c, a)| (c as usize, -a)));
                self.out
                    .lp
                    .add_row(label(format!("_c({i})")), io.constant, io.constant, terms);
            }
            self.out.lp.add_row(
                label("_c(2)".to_owned()),
                1.0,
                1.0,
                v.iter().map(|&c| (c, 1.0)),
            );
            #[allow(clippy::cast_precision_loss)]
            let members = v
                .iter()
                .enumerate()
                .map(|(i, &c)| (c, (i + 1) as f64))
                .collect();
            self.out.lp.add_sos2(
                calliope_lp::Label::new(ComponentKind::Piecewise, name, index.iter().cloned()),
                members,
            );
        }
        Ok(())
    }
}

/// None for a 0-d true mask (`where_array.equals(DataArray(True))`: no masking).
fn mask_of(ew: &LArray<bool>) -> Option<&LArray<bool>> {
    (!(ew.dims().is_empty() && ew.get(&[]))).then_some(ew)
}

/// `drop_dims_not_in_foreach`: "any" over the other dims, in `foreach` order.
fn drop_dims_not_in_foreach(w: &LArray<bool>, foreach: &[String]) -> Result<LArray<bool>> {
    let names: Vec<String> = w.dims().iter().map(|c| c.name().to_owned()).collect();
    let extra: Vec<&str> = names
        .iter()
        .filter(|d| !foreach.contains(d))
        .map(String::as_str)
        .collect();
    let reduced = if extra.is_empty() {
        w.clone()
    } else {
        w.any(&extra).map_err(|e| EvalError(e.to_string()))?
    };
    let have: HashSet<&str> = reduced.dims().iter().map(Coord::name).collect();
    if foreach.iter().any(|d| !have.contains(d.as_str())) {
        // A `foreach` dim the mask does not have: Calliope's mask always has them all
        // (combine_active_and_foreach), so this only happens for an all-false mask.
        return Ok(LArray::scalar(false));
    }
    let order: Vec<&str> = foreach.iter().map(String::as_str).collect();
    if reduced.dim_names() == order {
        Ok(reduced)
    } else {
        reduced
            .transpose(&order)
            .map_err(|e| EvalError(e.to_string()))
    }
}

/// An evaluated expression with the target's dims, in the target's order.
fn conform(v: &LArray<Lin>, dims: &[Coord]) -> Result<LArray<Lin>> {
    let order: Vec<&str> = dims.iter().map(Coord::name).collect();
    if v.dim_names() == order {
        return Ok(v.clone());
    }
    if v.dims().len() == dims.len() {
        return v.transpose(&order).map_err(|e| EvalError(e.to_string()));
    }
    // Broadcast over the dims it lacks.
    let mask: LArray<bool> = LArray::from_cells(
        dims.to_vec(),
        true_cells_full(dims).into_iter().map(|c| (c, true)),
    )
    .map_err(|e| EvalError(e.to_string()))?;
    let b = v.where_(&mask);
    b.transpose(&order).map_err(|e| EvalError(e.to_string()))
}

fn true_cells_full(dims: &[Coord]) -> Vec<Vec<u32>> {
    let shape: Vec<usize> = dims.iter().map(Coord::len).collect();
    let total: usize = shape.iter().product();
    let mut out = Vec::with_capacity(total);
    let mut idx = vec![0u32; dims.len()];
    for _ in 0..total {
        out.push(idx.clone());
        for i in (0..idx.len()).rev() {
            idx[i] += 1;
            if (idx[i] as usize) < shape[i] {
                break;
            }
            idx[i] = 0;
        }
    }
    out
}

fn any_missing_at<T: Element>(a: &LArray<T>, ew: &LArray<bool>) -> bool {
    let Some(p) = Picker::new(a, ew.dims()) else {
        return false;
    };
    ew.cells()
        .filter(|(_, v)| **v)
        .any(|(c, _)| p.at(&c).is_missing())
}

/// "trying to set two equations for the same index".
fn overlap<T: Element>(eq_name: &str, acc: &LArray<T>, ew: &LArray<bool>) -> Result<()> {
    if acc.count() == 0 {
        return Ok(());
    }
    let p = Picker::new(acc, ew.dims()).expect("same dims");
    let hits: Vec<String> = ew
        .cells()
        .filter(|(c, v)| **v && !p.at(c).is_missing())
        .map(|(c, _)| {
            let parts: Vec<String> = c
                .iter()
                .zip(ew.dims())
                .map(|(i, d)| format!("'{}'", label_str(d.label(*i))))
                .collect();
            format!("({})", parts.join(", "))
        })
        .collect();
    if hits.is_empty() {
        return Ok(());
    }
    if ew.dims().is_empty() {
        return Err(EvalError(format!(
            "{eq_name} | trying to set two equations for the same component."
        )));
    }
    Err(EvalError(format!(
        "{eq_name} | trying to set two equations for the same index:\n{}",
        hits.join("\n")
    )))
}

/// Adds the row Pyomo writes for `lhs op rhs` (`kernel.constraint.expr`): a side without
/// variables becomes the bound, otherwise `lhs - rhs` against 0; infinite bounds are
/// dropped, and a row without finite bounds is not written. Returns the row id, if any.
fn add_constraint_row(
    lp: &mut LpModel,
    label: calliope_lp::Label,
    op: CmpOp,
    lhs: &Lin,
    rhs: &Lin,
) -> Option<u32> {
    // `a >= b` is Pyomo's `b <= a`.
    let (a0, a1, eq) = match op {
        CmpOp::Ge => (rhs, lhs, false),
        CmpOp::Eq => (lhs, rhs, true),
        _ => (lhs, rhs, false),
    };
    let (body, lb, ub): (Lin, Option<f64>, Option<f64>) = if !a1.is_expr() {
        let b = a1.constant;
        (a0.clone(), eq.then_some(b), Some(b))
    } else if !a0.is_expr() {
        let b = a0.constant;
        (a1.clone(), Some(b), eq.then_some(b))
    } else {
        (a0.sub(a1), eq.then_some(0.0), Some(0.0))
    };
    let lb = lb.filter(|v| *v != f64::NEG_INFINITY);
    let ub = ub.filter(|v| *v != f64::INFINITY);
    if lb.is_none() && ub.is_none() {
        return None;
    }
    let offset = body.constant;
    let lo = lb.map_or(f64::NEG_INFINITY, |v| v - offset);
    let hi = ub.map_or(f64::INFINITY, |v| v - offset);
    let terms = body.terms.iter().map(|&(c, a)| (c as usize, a));
    Some(u32::try_from(lp.add_row(label, lo, hi, terms)).expect("fits u32"))
}
