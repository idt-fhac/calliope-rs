//! HiGHS backend over an [`LpModel`]: solve, results, in-place updates.
//!
//! Mirrors `calliope.backend.highs_backend_model.HighsBackendModel._solve`:
//! options are reset before every solve, `small_matrix_value=1e-12` is re-applied,
//! the solver state is cleared unless `warmstart`, `solver_options` are passed through,
//! and `save_logs` sets `log_file=<dir>/highs.log`.
//!
//! SOS2 sets are not supported by HiGHS; they are passed as an equivalent MILP (one binary
//! per segment), appended after the model's own columns and rows so that model indices and
//! HiGHS indices coincide.

use crate::ffi::{self, Highs, ModelData, ModelStatus, OptionValue};
use calliope_lp::{ColId, ComponentKind, LinExpr, LpModel, ObjSense, RowId};
use std::collections::HashMap;
use std::path::PathBuf;

/// Backend errors.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    /// From HiGHS.
    #[error(transparent)]
    Highs(#[from] ffi::HighsError),
    /// Invalid request (unknown component, no solution, ...).
    #[error("{0}")]
    Invalid(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, BackendError>;

/// Solve settings (the subset of Calliope's `config.solve` the backend uses).
#[derive(Debug, Clone, Default)]
pub struct SolveConfig {
    /// Passed to HiGHS verbatim, in order.
    pub solver_options: Vec<(String, OptionValue)>,
    /// Directory for `highs.log`.
    pub save_logs: Option<PathBuf>,
    /// Keep the previous basis/solution as a starting point.
    pub warmstart: bool,
}

/// Outcome of one solve.
#[derive(Debug, Clone, PartialEq)]
pub struct SolveOutcome {
    /// HiGHS model status.
    pub status: ModelStatus,
    /// Calliope's `termination_condition` (lower-cased HiGHS status string).
    pub termination_condition: String,
    /// Objective value (with constant); `None` unless optimal.
    pub objective: Option<f64>,
    /// Primal values per model column; `None` unless optimal.
    pub col_value: Option<Vec<f64>>,
    /// Row duals per model row; `None` when HiGHS has no valid duals (e.g. a MILP).
    pub row_dual: Option<Vec<f64>>,
}

impl SolveOutcome {
    /// True when the solve was optimal.
    pub fn is_optimal(&self) -> bool {
        self.status == ModelStatus::OPTIMAL
    }
}

/// Something a parameter value feeds into; the evaluator registers these and recomputes
/// their values on `update_input`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Target {
    /// Matrix coefficient.
    Coeff(RowId, ColId),
    /// Row lower bound.
    RowLower(RowId),
    /// Row upper bound.
    RowUpper(RowId),
    /// Column lower bound.
    ColLower(ColId),
    /// Column upper bound.
    ColUpper(ColId),
    /// Objective coefficient.
    ObjCoeff(ColId),
    /// Objective constant.
    ObjConstant,
}

/// Parameter → [`Target`] dependency index.
#[derive(Debug, Clone, Default)]
pub struct DependencyIndex {
    map: HashMap<String, Vec<Target>>,
}

impl DependencyIndex {
    /// Record that `param` feeds into `target`.
    pub fn register(&mut self, param: &str, target: Target) {
        self.map.entry(param.to_string()).or_default().push(target);
    }

    /// Targets that depend on `param` (sorted, deduplicated).
    pub fn targets(&self, param: &str) -> Vec<Target> {
        let mut v = self.map.get(param).cloned().unwrap_or_default();
        v.sort_unstable();
        v.dedup();
        v
    }

    /// Rows touched by `param` (via coefficients or bounds).
    pub fn rows(&self, param: &str) -> Vec<RowId> {
        let mut rows: Vec<RowId> = self
            .targets(param)
            .into_iter()
            .filter_map(|t| match t {
                Target::Coeff(r, _) | Target::RowLower(r) | Target::RowUpper(r) => Some(r),
                _ => None,
            })
            .collect();
        rows.dedup();
        rows
    }
}

/// HiGHS backend.
#[derive(Debug)]
pub struct HighsBackend {
    model: LpModel,
    highs: Highs,
    /// HiGHS holds the current model (else it is passed on the next solve).
    synced: bool,
    /// Bounds before `fix_variable` (restored by `unfix_variable`).
    fixed: HashMap<ColId, (f64, f64)>,
    /// Bounds of deactivated rows.
    deactivated: HashMap<RowId, (f64, f64)>,
    /// Parameter dependencies registered by the evaluator.
    pub deps: DependencyIndex,
    last: Option<SolveOutcome>,
}

/// SOS2 sets as MILP: extra columns/rows appended to the model's arrays.
struct Reformulated {
    col_lower: Vec<f64>,
    col_upper: Vec<f64>,
    col_cost: Vec<f64>,
    integer: Vec<bool>,
    row_lower: Vec<f64>,
    row_upper: Vec<f64>,
    start: Vec<usize>,
    index: Vec<usize>,
    value: Vec<f64>,
}

/// Upper bound of an SOS2 member for the big-M link: its own finite bound, else the rhs of
/// a convexity row (`sum(members) == b`, all coefficients 1, nonnegative members).
fn sos_member_bounds(model: &LpModel, members: &[ColId]) -> Result<Vec<f64>> {
    let mut convex: Option<f64> = None;
    let set: std::collections::HashSet<ColId> = members.iter().copied().collect();
    if members.iter().any(|&c| !model.cols[c].ub.is_finite()) {
        for r in 0..model.num_rows() {
            let (cols, vals) = model.row(r);
            if cols.len() == members.len()
                && cols.iter().all(|c| set.contains(c))
                && vals.iter().all(|&v| v == 1.0)
                && model.rows[r].ub.is_finite()
            {
                convex = Some(model.rows[r].ub);
                break;
            }
        }
    }
    members
        .iter()
        .map(|&c| {
            let col = &model.cols[c];
            if col.lb < 0.0 {
                return Err(BackendError::Invalid(format!(
                    "SOS2 member `{}` has a negative lower bound",
                    col.label
                )));
            }
            if col.ub.is_finite() {
                Ok(col.ub)
            } else {
                convex.ok_or_else(|| {
                    BackendError::Invalid(format!(
                        "SOS2 member `{}` has no finite upper bound",
                        col.label
                    ))
                })
            }
        })
        .collect()
}

fn reformulate(model: &LpModel) -> Result<Reformulated> {
    let n = model.num_cols();
    let mut r = Reformulated {
        col_lower: model.cols.iter().map(|c| c.lb).collect(),
        col_upper: model.cols.iter().map(|c| c.ub).collect(),
        col_cost: model.objective_dense(),
        integer: model.cols.iter().map(|c| c.integer).collect(),
        row_lower: model.rows.iter().map(|x| x.lb).collect(),
        row_upper: model.rows.iter().map(|x| x.ub).collect(),
        start: model.row_start.clone(),
        index: model.row_index.clone(),
        value: model.row_value.clone(),
    };
    if r.start.is_empty() {
        r.start.push(0);
    }
    let mut next_col = n;
    for set in &model.sos2 {
        let mut members = set.members.clone();
        members.sort_by(|a, b| a.1.total_cmp(&b.1));
        let cols: Vec<ColId> = members.iter().map(|m| m.0).collect();
        let k = cols.len();
        if k < 2 {
            continue;
        }
        let ubs = sos_member_bounds(model, &cols)?;
        // z_s for segments s = 0..k-2
        let z0 = next_col;
        for _ in 0..k - 1 {
            r.col_lower.push(0.0);
            r.col_upper.push(1.0);
            r.col_cost.push(0.0);
            r.integer.push(true);
        }
        next_col += k - 1;
        // sum z = 1
        for s in 0..k - 1 {
            r.index.push(z0 + s);
            r.value.push(1.0);
        }
        r.start.push(r.index.len());
        r.row_lower.push(1.0);
        r.row_upper.push(1.0);
        // lambda_i - U_i (z_{i-1} + z_i) <= 0
        for (i, (&c, &u)) in cols.iter().zip(&ubs).enumerate() {
            r.index.push(c);
            r.value.push(1.0);
            if i > 0 {
                r.index.push(z0 + i - 1);
                r.value.push(-u);
            }
            if i < k - 1 {
                r.index.push(z0 + i);
                r.value.push(-u);
            }
            r.start.push(r.index.len());
            r.row_lower.push(f64::NEG_INFINITY);
            r.row_upper.push(0.0);
        }
    }
    Ok(r)
}

impl HighsBackend {
    /// Wrap a model; it is passed to HiGHS on the first solve.
    pub fn new(model: LpModel) -> Self {
        Self {
            model,
            highs: Highs::new(),
            synced: false,
            fixed: HashMap::new(),
            deactivated: HashMap::new(),
            deps: DependencyIndex::default(),
            last: None,
        }
    }

    /// The model as currently held (including in-place updates).
    pub fn model(&self) -> &LpModel {
        &self.model
    }

    /// Direct access to the HiGHS instance (for options not covered here).
    pub fn highs_mut(&mut self) -> &mut Highs {
        &mut self.highs
    }

    /// Last solve outcome.
    pub fn last_outcome(&self) -> Option<&SolveOutcome> {
        self.last.as_ref()
    }

    /// True when the model has integer columns or SOS sets.
    pub fn has_integer_or_binary_variables(&self) -> bool {
        self.model.is_mip()
    }

    fn sync(&mut self) -> Result<()> {
        if self.synced {
            return Ok(());
        }
        let r = reformulate(&self.model)?;
        let mip = r.integer.iter().any(|&b| b);
        self.highs.pass_model(&ModelData {
            maximize: self.model.objective.sense == ObjSense::Maximize,
            offset: self.model.objective.expr.constant,
            col_cost: &r.col_cost,
            col_lower: &r.col_lower,
            col_upper: &r.col_upper,
            row_lower: &r.row_lower,
            row_upper: &r.row_upper,
            a_start: &r.start,
            a_index: &r.index,
            a_value: &r.value,
            integer: mip.then_some(&r.integer[..]),
        })?;
        self.synced = true;
        Ok(())
    }

    /// Provide a primal start (for operate/SPORES warm starts or MIP starts).
    ///
    /// # Panics
    /// If `col_value` does not have one entry per model column.
    pub fn set_start(&mut self, col_value: &[f64]) -> Result<()> {
        assert_eq!(col_value.len(), self.model.num_cols());
        self.sync()?;
        let mut full = col_value.to_vec();
        full.resize(self.highs.num_col(), 0.0);
        self.highs.set_solution(&full)?;
        Ok(())
    }

    /// Solve the model.
    pub fn solve(&mut self, config: &SolveConfig) -> Result<&SolveOutcome> {
        self.sync()?;
        let h = &mut self.highs;
        h.reset_options()?;
        h.set_option("small_matrix_value", &OptionValue::Double(1e-12))?;
        if !config.warmstart {
            h.clear_solver()?;
        }
        for (k, v) in &config.solver_options {
            h.set_option(k, v)?;
        }
        if let Some(dir) = &config.save_logs {
            std::fs::create_dir_all(dir)
                .map_err(|e| BackendError::Invalid(format!("save_logs: {e}")))?;
            let path = dir.join("highs.log");
            h.set_option(
                "log_file",
                &OptionValue::Str(path.to_string_lossy().into_owned()),
            )?;
        }
        h.run()?;
        let status = h.model_status();
        let (ncol, nrow) = (self.model.num_cols(), self.model.num_rows());
        // bench/BUGS.md #7: a MILP stopped by a limit (time, solution count, ...) with a
        // feasible incumbent keeps its solution; Python discards it.
        let feasible_mip = status != ModelStatus::OPTIMAL
            && self.model.is_mip()
            && h.primal_valid()
            && status.feasible_limit();
        let outcome = if feasible_mip {
            let mut sol = h.solution()?;
            sol.col_value.truncate(ncol);
            SolveOutcome {
                status,
                termination_condition: status.termination_condition(),
                objective: Some(h.objective_value()),
                col_value: Some(sol.col_value),
                row_dual: None,
            }
        } else if status == ModelStatus::OPTIMAL {
            let mut sol = h.solution()?;
            sol.col_value.truncate(ncol);
            sol.row_dual.truncate(nrow);
            let duals_ok = h.dual_valid() && !self.model.is_mip();
            SolveOutcome {
                status,
                termination_condition: status.termination_condition(),
                objective: Some(h.objective_value()),
                col_value: Some(sol.col_value),
                row_dual: duals_ok.then_some(sol.row_dual),
            }
        } else {
            SolveOutcome {
                status,
                termination_condition: status.termination_condition(),
                objective: None,
                col_value: None,
                row_dual: None,
            }
        };
        self.last = Some(outcome);
        Ok(self.last.as_ref().expect("just set"))
    }

    /// Brings the held model to `new` in place when only values changed: same columns
    /// (labels, integrality), rows (labels), sparsity pattern and SOS sets. Coefficients,
    /// bounds and the objective are changed through HiGHS' `change*` calls, so the solver
    /// keeps its instance (and basis, for warm starts). Fixed columns and deactivated rows
    /// keep their state; their saved bounds take the new values. Only rows and columns whose
    /// label is `affected` (and the objective, if its label is) are changed, so manual
    /// changes elsewhere (`update_variable_bounds`) survive. Returns `false`, changing
    /// nothing, if the structure differs.
    pub fn update_values(
        &mut self,
        new: &LpModel,
        affected: &dyn Fn(&calliope_lp::Label) -> bool,
    ) -> Result<bool> {
        let m = &self.model;
        let same = m.num_cols() == new.num_cols()
            && m.num_rows() == new.num_rows()
            && m.row_start == new.row_start
            && m.row_index == new.row_index
            && m.sos2 == new.sos2
            && m.cols
                .iter()
                .zip(&new.cols)
                .all(|(a, b)| a.label == b.label && a.integer == b.integer)
            && m.rows
                .iter()
                .zip(&new.rows)
                .all(|(a, b)| a.label == b.label);
        if !same {
            return Ok(false);
        }
        for r in 0..new.num_rows() {
            if !affected(&new.rows[r].label) {
                continue;
            }
            let (s, e) = (new.row_start[r], new.row_start[r + 1]);
            for k in s..e {
                let v = new.row_value[k];
                if self.model.row_value[k].to_bits() != v.to_bits() {
                    self.model.row_value[k] = v;
                    if self.synced {
                        self.highs.change_coeff(r, new.row_index[k], v)?;
                    }
                }
            }
            let (lb, ub) = (new.rows[r].lb, new.rows[r].ub);
            let row = &self.model.rows[r];
            if let Some(saved) = self.deactivated.get_mut(&r) {
                *saved = (lb, ub);
            } else if row.lb.to_bits() != lb.to_bits() || row.ub.to_bits() != ub.to_bits() {
                self.set_row_bounds(r, lb, ub)?;
            }
        }
        for c in 0..new.num_cols() {
            if !affected(&new.cols[c].label) {
                continue;
            }
            let (lb, ub) = (new.cols[c].lb, new.cols[c].ub);
            if let Some(saved) = self.fixed.get_mut(&c) {
                *saved = (lb, ub);
                continue;
            }
            let col = &self.model.cols[c];
            if col.lb.to_bits() != lb.to_bits() || col.ub.to_bits() != ub.to_bits() {
                self.set_col_bounds(c, lb, ub)?;
            }
        }
        let (a, b) = (&self.model.objective, &new.objective);
        if (affected(&a.label) || affected(&b.label))
            && (a.sense != b.sense
                || a.label != b.label
                || a.expr.constant.to_bits() != b.expr.constant.to_bits()
                || a.expr.merged() != b.expr.merged())
        {
            self.set_objective(b.label.clone(), b.sense, b.expr.clone())?;
        }
        Ok(true)
    }

    /// Replaces the held model by a structurally different one (passed to HiGHS on the next
    /// solve). Rows deactivated before stay deactivated, fixed columns stay fixed, and
    /// columns not `affected` keep their bounds, where a row or column with the same label
    /// exists in `new`.
    pub fn replace_model(
        &mut self,
        mut new: LpModel,
        affected: &dyn Fn(&calliope_lp::Label) -> bool,
    ) {
        // Bounds of unaffected columns and rows are kept (manual changes survive).
        let col_bounds: HashMap<calliope_lp::Label, (f64, f64)> = self
            .model
            .cols
            .iter()
            .filter(|c| !affected(&c.label))
            .map(|c| (c.label.clone(), (c.lb, c.ub)))
            .collect();
        for c in &mut new.cols {
            if let Some(&(lb, ub)) = col_bounds.get(&c.label) {
                (c.lb, c.ub) = (lb, ub);
            }
        }
        let fixed: HashMap<calliope_lp::Label, (f64, f64)> = self
            .fixed
            .iter()
            .map(|(&c, &b)| (self.model.cols[c].label.clone(), b))
            .collect();
        let deactivated: std::collections::HashSet<calliope_lp::Label> = self
            .deactivated
            .keys()
            .map(|&r| self.model.rows[r].label.clone())
            .collect();
        let x: HashMap<calliope_lp::Label, f64> =
            match self.last.as_ref().and_then(|o| o.col_value.as_ref()) {
                Some(x) if !fixed.is_empty() => self
                    .model
                    .cols
                    .iter()
                    .zip(x)
                    .map(|(c, v)| (c.label.clone(), *v))
                    .collect(),
                _ => HashMap::new(),
            };
        self.model = new;
        self.synced = false;
        self.fixed.clear();
        self.deactivated.clear();
        for c in 0..self.model.num_cols() {
            let label = &self.model.cols[c].label;
            if let (Some(&saved), Some(&v)) = (fixed.get(label), x.get(label)) {
                self.fixed.insert(c, saved);
                self.model.cols[c].lb = v;
                self.model.cols[c].ub = v;
            }
        }
        for r in 0..self.model.num_rows() {
            if deactivated.contains(&self.model.rows[r].label) {
                let row = &mut self.model.rows[r];
                self.deactivated.insert(r, (row.lb, row.ub));
                row.lb = f64::NEG_INFINITY;
                row.ub = f64::INFINITY;
            }
        }
    }

    /// True if column `col` is fixed by [`Self::fix_variable`].
    pub fn is_fixed(&self, col: ColId) -> bool {
        self.fixed.contains_key(&col)
    }

    /// True if row `row` is deactivated.
    pub fn is_deactivated(&self, row: RowId) -> bool {
        self.deactivated.contains_key(&row)
    }

    /// Write the model as HiGHS sees it (`.lp` or `.mps`), like Calliope's `to_lp`.
    pub fn write_model(&mut self, path: &std::path::Path) -> Result<()> {
        self.sync()?;
        self.highs.write_model(&path.to_string_lossy())?;
        Ok(())
    }

    // ---- in-place updates -------------------------------------------------------------

    /// Set a list of target values (from re-evaluating parameter-dependent terms).
    /// Changes are applied to the model and, if loaded, to HiGHS in place.
    pub fn apply(&mut self, edits: &[(Target, f64)]) -> Result<()> {
        for &(t, v) in edits {
            match t {
                Target::Coeff(r, c) => self.set_coeff(r, c, v)?,
                Target::RowLower(r) => {
                    let ub = self.model.rows[r].ub;
                    self.set_row_bounds(r, v, ub)?;
                }
                Target::RowUpper(r) => {
                    let lb = self.model.rows[r].lb;
                    self.set_row_bounds(r, lb, v)?;
                }
                Target::ColLower(c) => {
                    let ub = self.model.cols[c].ub;
                    self.set_col_bounds(c, v, ub)?;
                }
                Target::ColUpper(c) => {
                    let lb = self.model.cols[c].lb;
                    self.set_col_bounds(c, lb, v)?;
                }
                Target::ObjCoeff(c) => {
                    let terms = &mut self.model.objective.expr.terms;
                    terms.retain(|t| t.0 != c);
                    if v != 0.0 {
                        terms.push((c, v));
                    }
                    if self.synced {
                        self.highs.change_col_cost(c, v)?;
                    }
                }
                Target::ObjConstant => {
                    self.model.objective.expr.constant = v;
                    if self.synced {
                        self.highs.change_objective_offset(v)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Change one matrix coefficient (a structural change if it was zero; HiGHS handles it).
    pub fn set_coeff(&mut self, row: RowId, col: ColId, value: f64) -> Result<()> {
        let (s, e) = (self.model.row_start[row], self.model.row_start[row + 1]);
        let pos = self.model.row_index[s..e].iter().position(|&c| c == col);
        match (pos, value != 0.0) {
            (Some(p), true) => self.model.row_value[s + p] = value,
            (Some(p), false) => {
                self.model.row_index.remove(s + p);
                self.model.row_value.remove(s + p);
                for st in &mut self.model.row_start[row + 1..] {
                    *st -= 1;
                }
            }
            (None, true) => {
                let p = s + self.model.row_index[s..e].partition_point(|&c| c < col);
                self.model.row_index.insert(p, col);
                self.model.row_value.insert(p, value);
                for st in &mut self.model.row_start[row + 1..] {
                    *st += 1;
                }
            }
            (None, false) => return Ok(()),
        }
        if self.synced {
            self.highs.change_coeff(row, col, value)?;
        }
        Ok(())
    }

    /// Change row bounds.
    pub fn set_row_bounds(&mut self, row: RowId, lb: f64, ub: f64) -> Result<()> {
        if let Some(saved) = self.deactivated.get_mut(&row) {
            // keep the row inactive; remember the new bounds for re-activation
            *saved = (lb, ub);
            return Ok(());
        }
        self.model.rows[row].lb = lb;
        self.model.rows[row].ub = ub;
        if self.synced {
            self.highs.change_row_bounds(row, lb, ub)?;
        }
        Ok(())
    }

    /// Change column bounds.
    pub fn set_col_bounds(&mut self, col: ColId, lb: f64, ub: f64) -> Result<()> {
        self.model.cols[col].lb = lb;
        self.model.cols[col].ub = ub;
        if !self.model.sos2.is_empty() {
            // big-M of the SOS2 reformulation may depend on these bounds
            self.synced = false;
        }
        if self.synced {
            self.highs.change_col_bounds(col, lb, ub)?;
        }
        Ok(())
    }

    fn component_cols(&self, name: &str) -> Result<Vec<ColId>> {
        let cols = self.model.cols_of(ComponentKind::Variable, name);
        if cols.is_empty() {
            return Err(BackendError::Invalid(format!("Unknown variable: {name}")));
        }
        Ok(cols)
    }

    /// `update_variable_bounds`: new `min`/`max` per column of variable `name`
    /// (`None` or NaN keeps the current bound). `values` is aligned with
    /// [`LpModel::cols_of`] order, or has length 1 to broadcast.
    pub fn update_variable_bounds(
        &mut self,
        name: &str,
        min: Option<&[f64]>,
        max: Option<&[f64]>,
    ) -> Result<()> {
        let cols = self.component_cols(name)?;
        let pick = |v: Option<&[f64]>, i: usize| -> Option<f64> {
            let v = v?;
            let x = if v.len() == 1 { v[0] } else { *v.get(i)? };
            (!x.is_nan()).then_some(x)
        };
        for (i, &c) in cols.iter().enumerate() {
            let col = &self.model.cols[c];
            let lb = pick(min, i).unwrap_or(col.lb);
            let ub = pick(max, i).unwrap_or(col.ub);
            self.set_col_bounds(c, lb, ub)?;
        }
        Ok(())
    }

    /// `fix_variable`: fix columns of `name` (where `mask` is true, or all) to the last
    /// optimal values.
    pub fn fix_variable(&mut self, name: &str, mask: Option<&[bool]>) -> Result<()> {
        let x = self
            .last
            .as_ref()
            .and_then(|o| o.col_value.clone())
            .ok_or_else(|| {
                BackendError::Invalid(
                    "Cannot fix variable values without already having solved the model successfully."
                        .into(),
                )
            })?;
        let cols = self.component_cols(name)?;
        for (i, &c) in cols.iter().enumerate() {
            if mask.is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            let col = &self.model.cols[c];
            self.fixed.entry(c).or_insert((col.lb, col.ub));
            self.set_col_bounds(c, x[c], x[c])?;
        }
        Ok(())
    }

    /// `unfix_variable`: restore the bounds from before `fix_variable` (HiGHS in Python
    /// cannot do this).
    pub fn unfix_variable(&mut self, name: &str, mask: Option<&[bool]>) -> Result<()> {
        let cols = self.component_cols(name)?;
        for (i, &c) in cols.iter().enumerate() {
            if mask.is_some_and(|m| !m.get(i).copied().unwrap_or(false)) {
                continue;
            }
            if let Some((lb, ub)) = self.fixed.remove(&c) {
                self.set_col_bounds(c, lb, ub)?;
            }
        }
        Ok(())
    }

    /// Deactivate (free) or reactivate all rows of constraint `name`.
    pub fn set_constraint_active(&mut self, name: &str, active: bool) -> Result<()> {
        let mut rows = self.model.rows_of(ComponentKind::Constraint, name);
        rows.extend(self.model.rows_of(ComponentKind::Piecewise, name));
        if rows.is_empty() {
            return Err(BackendError::Invalid(format!("Unknown constraint: {name}")));
        }
        for r in rows {
            if active {
                if let Some((lb, ub)) = self.deactivated.remove(&r) {
                    self.set_row_bounds(r, lb, ub)?;
                }
            } else if !self.deactivated.contains_key(&r) {
                let row = &self.model.rows[r];
                let saved = (row.lb, row.ub);
                self.set_row_bounds(r, f64::NEG_INFINITY, f64::INFINITY)?;
                self.deactivated.insert(r, saved);
            }
        }
        Ok(())
    }

    /// Replace the objective (`set_objective` with a newly evaluated expression).
    pub fn set_objective(
        &mut self,
        label: calliope_lp::Label,
        sense: ObjSense,
        expr: LinExpr,
    ) -> Result<()> {
        if self.synced {
            let old: Vec<ColId> = self
                .model
                .objective
                .expr
                .terms
                .iter()
                .map(|t| t.0)
                .collect();
            for c in old {
                self.highs.change_col_cost(c, 0.0)?;
            }
            for (c, a) in expr.merged() {
                self.highs.change_col_cost(c, a)?;
            }
            self.highs.change_objective_offset(expr.constant)?;
            self.highs
                .change_objective_sense(sense == ObjSense::Maximize)?;
        }
        self.model.set_objective(label, sense, expr);
        Ok(())
    }

    /// `delete_component` for variables and constraints. Deleting variables also drops
    /// their matrix/objective entries. Indices of later columns/rows shift; the returned map
    /// gives old → new indices. The model is re-passed to HiGHS on the next solve.
    pub fn delete_component(
        &mut self,
        kind: ComponentKind,
        name: &str,
    ) -> Result<Vec<Option<usize>>> {
        let map = match kind {
            ComponentKind::Variable => {
                let cols = self.model.cols_of(kind, name);
                let m = self.model.delete_cols(&cols);
                self.fixed = remap(&self.fixed, &m);
                m
            }
            ComponentKind::Constraint | ComponentKind::Piecewise => {
                let rows = self.model.rows_of(kind, name);
                let m = self.model.delete_rows(&rows);
                self.deactivated = remap(&self.deactivated, &m);
                m
            }
            _ => {
                return Err(BackendError::Invalid(format!(
                    "cannot delete a component of kind {kind:?}"
                )));
            }
        };
        self.synced = false;
        self.last = None;
        Ok(map)
    }
}

fn remap<V: Copy>(m: &HashMap<usize, V>, map: &[Option<usize>]) -> HashMap<usize, V> {
    m.iter()
        .filter_map(|(&k, &v)| map[k].map(|n| (n, v)))
        .collect()
}
