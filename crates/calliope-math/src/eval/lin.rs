//! Affine expressions over decision variables: the cell type of expression arrays.

use calliope_array::Element;

/// `constant + sum(coef * x[col])`, the value of one cell of a variable, global expression
/// or constraint side.
///
/// Like Pyomo's expression objects, terms are never simplified away: `0 * x` keeps the term
/// with coefficient 0, so "has terms" is exactly Pyomo's `is_potentially_variable` (which
/// decides how constraints are oriented and whether a cell is a plain number). A cell with no
/// terms is a number; with no terms and a NaN constant it is missing.
#[derive(Debug, Clone, PartialEq)]
pub struct Lin {
    /// The constant.
    pub constant: f64,
    /// `(column, coefficient)`, possibly repeating a column.
    pub terms: Vec<(u32, f64)>,
}

impl Element for Lin {
    fn missing() -> Self {
        Lin {
            constant: f64::NAN,
            terms: Vec::new(),
        }
    }
    fn is_missing(&self) -> bool {
        self.terms.is_empty() && self.constant.is_nan()
    }
}

impl Lin {
    /// A number.
    pub fn num(c: f64) -> Self {
        Lin {
            constant: c,
            terms: Vec::new(),
        }
    }

    /// One decision variable.
    pub fn var(col: u32) -> Self {
        Lin {
            constant: 0.0,
            terms: vec![(col, 1.0)],
        }
    }

    /// True if a decision variable is involved (Pyomo's `is_potentially_variable`).
    pub fn is_expr(&self) -> bool {
        !self.terms.is_empty()
    }

    /// `self + other`.
    #[must_use]
    pub fn add(&self, other: &Lin) -> Lin {
        let mut terms = Vec::with_capacity(self.terms.len() + other.terms.len());
        terms.extend_from_slice(&self.terms);
        terms.extend_from_slice(&other.terms);
        Lin {
            constant: self.constant + other.constant,
            terms,
        }
    }

    /// `self - other`.
    #[must_use]
    pub fn sub(&self, other: &Lin) -> Lin {
        self.add(&other.scale(-1.0))
    }

    /// `k * self`.
    #[must_use]
    pub fn scale(&self, k: f64) -> Lin {
        Lin {
            constant: self.constant * k,
            terms: self.terms.iter().map(|&(c, a)| (c, a * k)).collect(),
        }
    }

    /// `self * other` if at most one side involves variables.
    pub fn mul(&self, other: &Lin) -> Option<Lin> {
        match (self.is_expr(), other.is_expr()) {
            (true, true) => None,
            (_, false) => Some(self.scale(other.constant)),
            (false, true) => Some(other.scale(self.constant)),
        }
    }

    /// Sum of many cells (skipping none: missing cells are filtered by the caller).
    pub fn sum<'a>(cells: impl IntoIterator<Item = &'a Lin>) -> Lin {
        let mut out = Lin::num(0.0);
        for c in cells {
            out.constant += c.constant;
            out.terms.extend_from_slice(&c.terms);
        }
        out
    }
}
