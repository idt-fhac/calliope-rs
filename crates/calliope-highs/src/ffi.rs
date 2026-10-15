//! Minimal safe wrapper over the HiGHS C API (`highs-sys`).
//!
//! This is the only module of the workspace with `unsafe` code. Every call passes
//! slices whose lengths are checked against the model dimensions first.
#![allow(unsafe_code)]

use highs_sys as sys;
use std::ffi::{CStr, CString, c_void};
use std::fmt;
use std::ptr::{NonNull, null};

/// HiGHS integer type (32-bit in the default build).
pub type HInt = sys::HighsInt;

/// Error from a HiGHS call.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HighsError {
    /// A C API call returned `kHighsStatusError`.
    #[error("HiGHS call `{0}` failed")]
    Call(&'static str),
    /// Setting a solver option failed (unknown name or invalid value).
    #[error("invalid HiGHS option `{name}` = {value}")]
    Option {
        /// Option name.
        name: String,
        /// Value as given.
        value: String,
    },
    /// The model is larger than HiGHS' 32-bit index type.
    #[error("model too large for HiGHS: {0}")]
    TooLarge(&'static str),
    /// A string contained an interior NUL byte.
    #[error("string with NUL byte: {0:?}")]
    Nul(String),
}

/// Result alias.
pub type Result<T> = std::result::Result<T, HighsError>;

/// Status of a call that succeeded (possibly with a warning).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    /// `kHighsStatusOk`.
    Ok,
    /// `kHighsStatusWarning`.
    Warning,
}

fn check(name: &'static str, status: HInt) -> Result<CallStatus> {
    match status {
        sys::kHighsStatusOk => Ok(CallStatus::Ok),
        sys::kHighsStatusWarning => Ok(CallStatus::Warning),
        _ => Err(HighsError::Call(name)),
    }
}

fn cstr(s: &str) -> Result<CString> {
    CString::new(s).map_err(|_| HighsError::Nul(s.to_string()))
}

/// Convert a length/index to `HighsInt`.
pub fn hint(v: usize, what: &'static str) -> Result<HInt> {
    HInt::try_from(v).map_err(|_| HighsError::TooLarge(what))
}

fn hints(v: &[usize], what: &'static str) -> Result<Vec<HInt>> {
    v.iter().map(|&x| hint(x, what)).collect()
}

/// A solver option value (HiGHS options are bool, int, double or string).
#[derive(Debug, Clone, PartialEq)]
pub enum OptionValue {
    /// Boolean option.
    Bool(bool),
    /// Integer option.
    Int(i64),
    /// Floating point option.
    Double(f64),
    /// String option (also accepted for any type and parsed by HiGHS).
    Str(String),
}

impl fmt::Display for OptionValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bool(b) => write!(f, "{b}"),
            Self::Int(i) => write!(f, "{i}"),
            Self::Double(d) => write!(f, "{d}"),
            Self::Str(s) => f.write_str(s),
        }
    }
}

/// HiGHS model status (`kHighsModelStatus*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModelStatus(pub HInt);

impl ModelStatus {
    /// `kHighsModelStatusOptimal`.
    pub const OPTIMAL: Self = Self(sys::kHighsModelStatusOptimal);

    /// Same text as `Highs::modelStatusToString` (Calliope lower-cases it).
    pub fn as_str(self) -> &'static str {
        match self.0 {
            0 => "Not Set",
            1 => "Load error",
            2 => "Model error",
            3 => "Presolve error",
            4 => "Solve error",
            5 => "Postsolve error",
            6 => "Empty",
            7 => "Optimal",
            8 => "Infeasible",
            9 => "Primal infeasible or unbounded",
            10 => "Unbounded",
            11 => "Bound on objective reached",
            12 => "Target for objective reached",
            13 => "Time limit reached",
            14 => "Iteration limit reached",
            16 => "Solution limit reached",
            17 => "Interrupted by user",
            18 => "Memory limit reached",
            _ => "Unknown",
        }
    }

    /// Calliope's `termination_condition` string.
    pub fn termination_condition(self) -> String {
        self.as_str().to_lowercase()
    }
}

/// Row-wise model data passed to [`Highs::pass_model`]. Lengths are validated.
#[derive(Debug, Clone, Copy)]
pub struct ModelData<'a> {
    /// True to maximise.
    pub maximize: bool,
    /// Objective constant.
    pub offset: f64,
    /// Objective coefficients (length `ncol`).
    pub col_cost: &'a [f64],
    /// Column lower bounds.
    pub col_lower: &'a [f64],
    /// Column upper bounds.
    pub col_upper: &'a [f64],
    /// Row lower bounds (length `nrow`).
    pub row_lower: &'a [f64],
    /// Row upper bounds.
    pub row_upper: &'a [f64],
    /// CSR row starts (length `nrow` or `nrow + 1`).
    pub a_start: &'a [usize],
    /// CSR column indices.
    pub a_index: &'a [usize],
    /// CSR values.
    pub a_value: &'a [f64],
    /// Integrality per column (`None` for a pure LP).
    pub integer: Option<&'a [bool]>,
}

/// Primal and dual solution vectors.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Solution {
    /// Column primal values.
    pub col_value: Vec<f64>,
    /// Column duals (reduced costs).
    pub col_dual: Vec<f64>,
    /// Row activities.
    pub row_value: Vec<f64>,
    /// Row duals.
    pub row_dual: Vec<f64>,
}

/// An owned HiGHS instance.
pub struct Highs {
    ptr: NonNull<c_void>,
}

impl fmt::Debug for Highs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Highs")
            .field("num_col", &self.num_col())
            .field("num_row", &self.num_row())
            .finish()
    }
}

// SAFETY: a Highs instance is not tied to the creating thread; we never share `&mut`.
unsafe impl Send for Highs {}

impl Drop for Highs {
    fn drop(&mut self) {
        // SAFETY: ptr came from Highs_create and is destroyed exactly once.
        unsafe { sys::Highs_destroy(self.ptr.as_ptr()) }
    }
}

impl Default for Highs {
    fn default() -> Self {
        Self::new()
    }
}

impl Highs {
    /// Create an instance.
    ///
    /// # Panics
    /// If HiGHS fails to allocate.
    pub fn new() -> Self {
        // SAFETY: no preconditions.
        let p = unsafe { sys::Highs_create() };
        Self {
            ptr: NonNull::new(p).expect("Highs_create returned NULL"),
        }
    }

    fn p(&self) -> *mut c_void {
        self.ptr.as_ptr()
    }

    /// HiGHS version string, e.g. `1.15.0`.
    pub fn version() -> String {
        // SAFETY: returns a static NUL-terminated string.
        unsafe { CStr::from_ptr(sys::Highs_version()) }
            .to_string_lossy()
            .into_owned()
    }

    /// Set an option (type-checked by HiGHS).
    pub fn set_option(&mut self, name: &str, value: &OptionValue) -> Result<()> {
        let c = cstr(name)?;
        // SAFETY: valid instance and NUL-terminated strings.
        let st = unsafe {
            match value {
                OptionValue::Bool(b) => {
                    sys::Highs_setBoolOptionValue(self.p(), c.as_ptr(), *b as HInt)
                }
                OptionValue::Int(i) => match HInt::try_from(*i) {
                    Ok(i) => sys::Highs_setIntOptionValue(self.p(), c.as_ptr(), i),
                    Err(_) => sys::kHighsStatusError,
                },
                OptionValue::Double(d) => {
                    // HiGHS rejects a double for an int option; Python passes ints as ints,
                    // so retry integral doubles through the generic string setter.
                    let s = sys::Highs_setDoubleOptionValue(self.p(), c.as_ptr(), *d);
                    if s == sys::kHighsStatusError {
                        let v = cstr(&format!("{d}"))?;
                        sys::Highs_setOptionValue(self.p(), c.as_ptr(), v.as_ptr())
                    } else {
                        s
                    }
                }
                OptionValue::Str(s) => {
                    let v = cstr(s)?;
                    sys::Highs_setOptionValue(self.p(), c.as_ptr(), v.as_ptr())
                }
            }
        };
        check("setOptionValue", st)
            .map(|_| ())
            .map_err(|_| HighsError::Option {
                name: name.to_string(),
                value: value.to_string(),
            })
    }

    /// Reset all options to their defaults.
    pub fn reset_options(&mut self) -> Result<()> {
        // SAFETY: valid instance.
        check("resetOptions", unsafe { sys::Highs_resetOptions(self.p()) }).map(|_| ())
    }

    /// Clear solver state (basis, solution) so the next run is a cold start.
    pub fn clear_solver(&mut self) -> Result<()> {
        // SAFETY: valid instance.
        check("clearSolver", unsafe { sys::Highs_clearSolver(self.p()) }).map(|_| ())
    }

    /// Replace the incumbent model.
    ///
    /// # Panics
    /// If slice lengths are inconsistent.
    pub fn pass_model(&mut self, m: &ModelData<'_>) -> Result<CallStatus> {
        let ncol = m.col_cost.len();
        let nrow = m.row_lower.len();
        assert_eq!(m.col_lower.len(), ncol);
        assert_eq!(m.col_upper.len(), ncol);
        assert_eq!(m.row_upper.len(), nrow);
        assert!(m.a_start.len() == nrow || m.a_start.len() == nrow + 1);
        assert_eq!(m.a_index.len(), m.a_value.len());
        assert!(m.a_index.iter().all(|&j| j < ncol));
        let nnz = m.a_value.len();
        let start = hints(&m.a_start[..nrow], "nnz")?;
        let index = hints(m.a_index, "columns")?;
        let integ: Option<Vec<HInt>> = m.integer.map(|v| {
            assert_eq!(v.len(), ncol);
            v.iter()
                .map(|&b| {
                    if b {
                        sys::kHighsVarTypeInteger
                    } else {
                        sys::kHighsVarTypeContinuous
                    }
                })
                .collect()
        });
        let sense = if m.maximize {
            sys::kHighsObjSenseMaximize
        } else {
            sys::kHighsObjSenseMinimize
        };
        let ptr_or_null = |v: &[HInt]| if v.is_empty() { null() } else { v.as_ptr() };
        // SAFETY: all arrays have the lengths HiGHS reads (checked above); q_* are NULL
        // with q_num_nz = 0; integrality is NULL or ncol long.
        let st = unsafe {
            sys::Highs_passModel(
                self.p(),
                hint(ncol, "columns")?,
                hint(nrow, "rows")?,
                hint(nnz, "nnz")?,
                0,
                sys::kHighsMatrixFormatRowwise,
                sys::kHighsHessianFormatTriangular,
                sense,
                m.offset,
                m.col_cost.as_ptr(),
                m.col_lower.as_ptr(),
                m.col_upper.as_ptr(),
                m.row_lower.as_ptr(),
                m.row_upper.as_ptr(),
                ptr_or_null(&start),
                ptr_or_null(&index),
                if nnz == 0 { null() } else { m.a_value.as_ptr() },
                null(),
                null(),
                null(),
                integ.as_ref().map_or(null(), |v| v.as_ptr()),
            )
        };
        check("passModel", st)
    }

    /// Solve the incumbent model.
    pub fn run(&mut self) -> Result<CallStatus> {
        // SAFETY: valid instance.
        check("run", unsafe { sys::Highs_run(self.p()) })
    }

    /// Model status after [`Self::run`].
    pub fn model_status(&self) -> ModelStatus {
        // SAFETY: valid instance.
        ModelStatus(unsafe { sys::Highs_getModelStatus(self.p()) })
    }

    /// Objective function value (includes the offset).
    pub fn objective_value(&self) -> f64 {
        // SAFETY: valid instance.
        unsafe { sys::Highs_getObjectiveValue(self.p()) }
    }

    /// Integer info value (e.g. `primal_solution_status`).
    pub fn int_info(&self, name: &str) -> Result<HInt> {
        let c = cstr(name)?;
        let mut v: HInt = 0;
        // SAFETY: valid instance, string and out-pointer.
        check("getIntInfoValue", unsafe {
            sys::Highs_getIntInfoValue(self.p(), c.as_ptr(), &mut v)
        })?;
        Ok(v)
    }

    /// Double info value (e.g. `mip_gap`).
    pub fn double_info(&self, name: &str) -> Result<f64> {
        let c = cstr(name)?;
        let mut v = 0.0;
        // SAFETY: valid instance, string and out-pointer.
        check("getDoubleInfoValue", unsafe {
            sys::Highs_getDoubleInfoValue(self.p(), c.as_ptr(), &mut v)
        })?;
        Ok(v)
    }

    /// True when a feasible primal solution is available (`value_valid` in highspy).
    pub fn primal_valid(&self) -> bool {
        self.int_info("primal_solution_status")
            .is_ok_and(|s| s == sys::kHighsSolutionStatusFeasible)
    }

    /// True when a feasible dual solution is available (`dual_valid` in highspy).
    pub fn dual_valid(&self) -> bool {
        self.int_info("dual_solution_status")
            .is_ok_and(|s| s == sys::kHighsSolutionStatusFeasible)
    }

    /// Number of columns of the incumbent model.
    pub fn num_col(&self) -> usize {
        // SAFETY: valid instance.
        usize::try_from(unsafe { sys::Highs_getNumCol(self.p()) }).unwrap_or(0)
    }

    /// Number of rows of the incumbent model.
    pub fn num_row(&self) -> usize {
        // SAFETY: valid instance.
        usize::try_from(unsafe { sys::Highs_getNumRow(self.p()) }).unwrap_or(0)
    }

    /// Current primal/dual solution.
    pub fn solution(&self) -> Result<Solution> {
        let (n, m) = (self.num_col(), self.num_row());
        let mut s = Solution {
            col_value: vec![0.0; n],
            col_dual: vec![0.0; n],
            row_value: vec![0.0; m],
            row_dual: vec![0.0; m],
        };
        // SAFETY: buffers sized to the incumbent model's dimensions.
        check("getSolution", unsafe {
            sys::Highs_getSolution(
                self.p(),
                s.col_value.as_mut_ptr(),
                s.col_dual.as_mut_ptr(),
                s.row_value.as_mut_ptr(),
                s.row_dual.as_mut_ptr(),
            )
        })?;
        Ok(s)
    }

    /// Provide a primal starting solution (MIP start / warm start).
    ///
    /// # Panics
    /// If `col_value` does not have one entry per column.
    pub fn set_solution(&mut self, col_value: &[f64]) -> Result<CallStatus> {
        assert_eq!(col_value.len(), self.num_col());
        // SAFETY: col_value has num_col entries; other arrays may be NULL.
        check("setSolution", unsafe {
            sys::Highs_setSolution(self.p(), col_value.as_ptr(), null(), null(), null())
        })
    }

    /// Change one matrix coefficient.
    pub fn change_coeff(&mut self, row: usize, col: usize, value: f64) -> Result<()> {
        // SAFETY: valid instance; HiGHS range-checks indices.
        check("changeCoeff", unsafe {
            sys::Highs_changeCoeff(self.p(), hint(row, "row")?, hint(col, "col")?, value)
        })
        .map(|_| ())
    }

    /// Change bounds of one column.
    pub fn change_col_bounds(&mut self, col: usize, lb: f64, ub: f64) -> Result<()> {
        // SAFETY: valid instance; HiGHS range-checks indices.
        check("changeColBounds", unsafe {
            sys::Highs_changeColBounds(self.p(), hint(col, "col")?, lb, ub)
        })
        .map(|_| ())
    }

    /// Change bounds of one row.
    pub fn change_row_bounds(&mut self, row: usize, lb: f64, ub: f64) -> Result<()> {
        // SAFETY: valid instance; HiGHS range-checks indices.
        check("changeRowBounds", unsafe {
            sys::Highs_changeRowBounds(self.p(), hint(row, "row")?, lb, ub)
        })
        .map(|_| ())
    }

    /// Change one objective coefficient.
    pub fn change_col_cost(&mut self, col: usize, cost: f64) -> Result<()> {
        // SAFETY: valid instance; HiGHS range-checks indices.
        check("changeColCost", unsafe {
            sys::Highs_changeColCost(self.p(), hint(col, "col")?, cost)
        })
        .map(|_| ())
    }

    /// Change the objective constant.
    pub fn change_objective_offset(&mut self, offset: f64) -> Result<()> {
        // SAFETY: valid instance.
        check("changeObjectiveOffset", unsafe {
            sys::Highs_changeObjectiveOffset(self.p(), offset)
        })
        .map(|_| ())
    }

    /// Change the objective sense.
    pub fn change_objective_sense(&mut self, maximize: bool) -> Result<()> {
        let s = if maximize {
            sys::kHighsObjSenseMaximize
        } else {
            sys::kHighsObjSenseMinimize
        };
        // SAFETY: valid instance.
        check("changeObjectiveSense", unsafe {
            sys::Highs_changeObjectiveSense(self.p(), s)
        })
        .map(|_| ())
    }

    /// Delete rows by index set (indices must be sorted ascending).
    pub fn delete_rows(&mut self, rows: &[usize]) -> Result<()> {
        let set = hints(rows, "row")?;
        // SAFETY: set has the given number of entries.
        check("deleteRowsBySet", unsafe {
            sys::Highs_deleteRowsBySet(self.p(), hint(set.len(), "rows")?, set.as_ptr())
        })
        .map(|_| ())
    }

    /// Delete columns by index set (indices must be sorted ascending).
    pub fn delete_cols(&mut self, cols: &[usize]) -> Result<()> {
        let set = hints(cols, "col")?;
        // SAFETY: set has the given number of entries.
        check("deleteColsBySet", unsafe {
            sys::Highs_deleteColsBySet(self.p(), hint(set.len(), "cols")?, set.as_ptr())
        })
        .map(|_| ())
    }

    /// Write the incumbent model (format from the extension: `.lp`, `.mps`).
    pub fn write_model(&mut self, path: &str) -> Result<CallStatus> {
        let c = cstr(path)?;
        // SAFETY: valid instance and path string.
        check("writeModel", unsafe {
            sys::Highs_writeModel(self.p(), c.as_ptr())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tiny_lp() {
        // min x + y  s.t. x + 2y >= 2, 3x + y >= 3, x,y >= 0  → x = 0.8, y = 0.6
        let mut h = Highs::new();
        h.set_option("output_flag", &OptionValue::Bool(false))
            .unwrap();
        h.pass_model(&ModelData {
            maximize: false,
            offset: 1.0,
            col_cost: &[1.0, 1.0],
            col_lower: &[0.0, 0.0],
            col_upper: &[f64::INFINITY, f64::INFINITY],
            row_lower: &[2.0, 3.0],
            row_upper: &[f64::INFINITY, f64::INFINITY],
            a_start: &[0, 2, 4],
            a_index: &[0, 1, 0, 1],
            a_value: &[1.0, 2.0, 3.0, 1.0],
            integer: None,
        })
        .unwrap();
        h.run().unwrap();
        assert_eq!(h.model_status(), ModelStatus::OPTIMAL);
        assert_eq!(h.model_status().termination_condition(), "optimal");
        assert!((h.objective_value() - 2.4).abs() < 1e-9);
        let s = h.solution().unwrap();
        assert!((s.col_value[0] - 0.8).abs() < 1e-9);
        assert!(h.dual_valid() && h.primal_valid());
        assert!((s.row_dual[0] - 0.4).abs() < 1e-9);
        assert!(
            h.set_option("no_such_option", &OptionValue::Int(1))
                .is_err()
        );
        h.set_option("threads", &OptionValue::Double(1.0)).unwrap();
        assert!(Highs::version().starts_with("1.15"));
    }
}
