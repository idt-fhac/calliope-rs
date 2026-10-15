//! Dataset comparison (SPEC §4 "Inputs"): same variables, coordinates (label order) and
//! dtypes; values within a tolerance; missing equals missing.

use calliope_preprocess::xr::{DType, Dataset, Obj};

/// One difference between two datasets.
#[derive(Debug, Clone, PartialEq)]
pub enum Diff {
    /// A variable only in the expected dataset.
    MissingVar(String),
    /// A variable only in the actual dataset.
    ExtraVar(String),
    /// A coordinate differs (labels or their order).
    Coord {
        /// Dimension.
        dim: String,
        /// Expected labels.
        expected: String,
        /// Actual labels.
        actual: String,
    },
    /// A variable has other dimensions (or dimension order).
    Dims {
        /// Variable.
        var: String,
        /// Expected dims.
        expected: Vec<String>,
        /// Actual dims.
        actual: Vec<String>,
    },
    /// A variable has another dtype (as stored in the file).
    DType {
        /// Variable.
        var: String,
        /// Expected dtype.
        expected: DType,
        /// Actual dtype.
        actual: DType,
    },
    /// Values differ.
    Values {
        /// Variable.
        var: String,
        /// Number of differing cells.
        count: usize,
        /// First differing cell: (flat index, expected, actual).
        first: (usize, String, String),
    },
}

impl std::fmt::Display for Diff {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Diff::MissingVar(v) => write!(f, "missing variable `{v}`"),
            Diff::ExtraVar(v) => write!(f, "extra variable `{v}`"),
            Diff::Coord {
                dim,
                expected,
                actual,
            } => {
                write!(f, "coordinate `{dim}`: expected {expected}, got {actual}")
            }
            Diff::Dims {
                var,
                expected,
                actual,
            } => {
                write!(f, "`{var}` dims: expected {expected:?}, got {actual:?}")
            }
            Diff::DType {
                var,
                expected,
                actual,
            } => {
                write!(f, "`{var}` dtype: expected {expected}, got {actual}")
            }
            Diff::Values { var, count, first } => write!(
                f,
                "`{var}`: {count} values differ, first at {}: expected {}, got {}",
                first.0, first.1, first.2
            ),
        }
    }
}

fn close(a: &Obj, b: &Obj, tol: f64) -> bool {
    match (a, b) {
        (Obj::Nan, Obj::Nan) => true,
        (Obj::Str(x), Obj::Str(y)) => x == y,
        (Obj::Datetime(x), Obj::Datetime(y)) => x == y,
        _ => match (a.as_f64(), b.as_f64()) {
            (Some(x), Some(y)) => x == y || (x - y).abs() <= tol * x.abs().max(y.abs()).max(1.0),
            _ => false,
        },
    }
}

/// Compares `actual` against `expected`. Variable order is not compared.
pub fn compare_datasets(expected: &Dataset, actual: &Dataset, tol: f64) -> Vec<Diff> {
    let mut diffs = Vec::new();
    for (dim, c) in &expected.coords {
        match actual.coords.get(dim) {
            Some(a) if a.labels() == c.labels() => {}
            a => diffs.push(Diff::Coord {
                dim: dim.clone(),
                expected: format!("{:?}", c.labels()),
                actual: a.map_or_else(|| "nothing".to_owned(), |a| format!("{:?}", a.labels())),
            }),
        }
    }
    for name in expected.vars.keys() {
        if !actual.vars.contains_key(name) {
            diffs.push(Diff::MissingVar(name.clone()));
        }
    }
    for name in actual.vars.keys() {
        if !expected.vars.contains_key(name) {
            diffs.push(Diff::ExtraVar(name.clone()));
        }
    }
    for (name, e) in &expected.vars {
        let Some(a) = actual.vars.get(name) else {
            continue;
        };
        if e.dims != a.dims {
            diffs.push(Diff::Dims {
                var: name.clone(),
                expected: e.dims.clone(),
                actual: a.dims.clone(),
            });
        }
        if e.data.dtype() != a.data.dtype() {
            diffs.push(Diff::DType {
                var: name.clone(),
                expected: e.data.dtype(),
                actual: a.data.dtype(),
            });
        }
        // Compare values cell by cell on the expected layout.
        let (Some(ea), Some(aa)) = (expected.get(name), actual.get(name)) else {
            continue;
        };
        if ea.dims().iter().any(|d| !aa.has_dim(d)) || aa.dims().len() != ea.dims().len() {
            continue;
        }
        let order: Vec<&str> = ea.dims();
        let aa = aa.transpose(&order);
        let aa = aa.conform(&ea.coords, DType::Float);
        let mut count = 0;
        let mut first = None;
        for i in 0..ea.data.len() {
            let (x, y) = (ea.data.get(i), aa.data.get(i));
            if !close(&x, &y, tol) {
                count += 1;
                if first.is_none() {
                    first = Some((i, x.to_py_str(), y.to_py_str()));
                }
            }
        }
        if let Some(first) = first {
            diffs.push(Diff::Values {
                var: name.clone(),
                count,
                first,
            });
        }
    }
    diffs
}
