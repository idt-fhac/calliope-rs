//! Mapping a solution back to labelled arrays, like `BackendModel.load_results`.
//!
//! The evaluator knows, for every math component, its dimensions and which column, row or
//! affine expression each defined cell maps to; it passes that as a [`Layout`]. For callers
//! that only have an [`LpModel`] with labels, [`Layout::from_labels`] matches the label index
//! strings against the coordinates.
//!
//! [`load_results`] then assembles, in Python's order: variables, global expressions,
//! shadow prices (`shadow_price_<constraint>`), objectives; applies `zero_threshold`; calls
//! the [`Postprocess`] hook (the evaluator's `postprocessed` math components); and drops
//! arrays that are entirely missing.

use crate::backend::SolveOutcome;
use calliope_array::{ArrayError, Coord, LArray, Label as CLabel};
use calliope_lp::{ComponentKind, LinExpr, LpModel};
use indexmap::IndexMap;
use std::collections::HashMap;

/// Where the cells of one component come from.
#[derive(Debug, Clone)]
pub struct Layout<T> {
    /// Dimensions in the component's order.
    pub dims: Vec<Coord>,
    /// `(codes in dims order, source)` for each defined cell.
    pub cells: Vec<(Vec<u32>, T)>,
}

/// String form of a coordinate label as it appears in LP labels.
pub fn label_string(l: &CLabel) -> String {
    match l {
        CLabel::Str(s) => s.to_string(),
        CLabel::Int(i) => i.to_string(),
        CLabel::Float(f) => f.to_string(),
        CLabel::Datetime(ns) => format_datetime(*ns),
    }
}

/// `YYYY-MM-DD HH:MM:SS` (pandas' `str(Timestamp)` without fractional seconds).
fn format_datetime(ns: i64) -> String {
    let secs = ns.div_euclid(1_000_000_000);
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // civil_from_days (Howard Hinnant)
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// Index of every label string of `c`; datetimes also match without seconds
/// (`2005-01-01 00:00`) and with `_` in place of ` `, `-` and `:` (LP-safe names).
fn coord_lookup(c: &Coord) -> HashMap<String, u32> {
    let mut m = HashMap::new();
    for (i, l) in c.labels().iter().enumerate() {
        let code = u32::try_from(i).unwrap_or(u32::MAX);
        let s = label_string(l);
        if let CLabel::Datetime(_) = l {
            let short = s.strip_suffix(":00").unwrap_or(&s).to_string();
            m.insert(calliope_lp::model::lp_safe(&short), code);
            m.insert(short, code);
            m.insert(calliope_lp::model::lp_safe(&s), code);
        }
        m.entry(calliope_lp::model::lp_safe(&s)).or_insert(code);
        m.insert(s, code);
    }
    m
}

impl Layout<usize> {
    /// Cells of component `name` of `kind`, located by matching label index strings to
    /// `dims` (columns for variables, rows for constraints).
    pub fn from_labels(
        model: &LpModel,
        kind: ComponentKind,
        name: &str,
        dims: Vec<Coord>,
    ) -> Result<Self, ArrayError> {
        let lookups: Vec<HashMap<String, u32>> = dims.iter().map(coord_lookup).collect();
        let labels: Vec<(usize, &calliope_lp::Label)> = match kind {
            ComponentKind::Variable => model
                .cols
                .iter()
                .enumerate()
                .map(|(i, c)| (i, &c.label))
                .collect(),
            _ => model
                .rows
                .iter()
                .enumerate()
                .map(|(i, r)| (i, &r.label))
                .collect(),
        };
        let mut cells = Vec::new();
        for (id, l) in labels {
            if l.kind != kind || &*l.component != name || l.suffix.is_some() {
                continue;
            }
            if l.index.len() != dims.len() {
                return Err(ArrayError::Shape(format!(
                    "`{l}` has {} index items for {} dims",
                    l.index.len(),
                    dims.len()
                )));
            }
            let codes = l
                .index
                .iter()
                .zip(&lookups)
                .zip(&dims)
                .map(|((s, lk), d)| {
                    lk.get(&**s).copied().ok_or_else(|| {
                        ArrayError::Shape(format!("`{s}` is not a label of `{}`", d.name()))
                    })
                })
                .collect::<Result<Vec<u32>, _>>()?;
            cells.push((codes, id));
        }
        Ok(Self { dims, cells })
    }
}

fn gather(layout: &Layout<usize>, values: &[f64]) -> Result<LArray<f64>, ArrayError> {
    LArray::from_cells(
        layout.dims.clone(),
        layout
            .cells
            .iter()
            .map(|(codes, i)| (codes.clone(), values[*i])),
    )
}

/// Primal values of a variable.
pub fn variable_values(layout: &Layout<usize>, x: &[f64]) -> Result<LArray<f64>, ArrayError> {
    gather(layout, x)
}

/// Duals of a constraint; all-NaN when no valid duals exist (MILP, non-optimal).
pub fn shadow_prices(
    layout: &Layout<usize>,
    duals: Option<&[f64]>,
) -> Result<LArray<f64>, ArrayError> {
    match duals {
        Some(d) => gather(layout, d),
        None => LArray::from_cells(
            layout.dims.clone(),
            layout.cells.iter().map(|(c, _)| (c.clone(), f64::NAN)),
        ),
    }
}

/// Values of affine expressions (global expressions, objectives) at `x`.
pub fn expression_values(layout: &Layout<LinExpr>, x: &[f64]) -> Result<LArray<f64>, ArrayError> {
    LArray::from_cells(
        layout.dims.clone(),
        layout.cells.iter().map(|(c, e)| (c.clone(), e.eval(x))),
    )
}

/// Values with `|v| < threshold` set to zero (NaN kept), like `_apply_zero_threshold`.
pub fn apply_zero_threshold(a: &LArray<f64>, threshold: f64) -> LArray<f64> {
    if threshold == 0.0 {
        return a.clone();
    }
    a.map(|&v| if v.abs() < threshold { 0.0 } else { v })
}

/// Hook for the `postprocessed` math components, evaluated on results + inputs.
pub trait Postprocess {
    /// New arrays to add, in math `order`; `results` holds the cleaned results so far.
    fn postprocess(
        &self,
        results: &IndexMap<String, LArray<f64>>,
    ) -> Result<Vec<(String, LArray<f64>)>, String>;
}

/// What to load, by component name.
#[derive(Debug, Clone, Default)]
pub struct ResultLayouts {
    /// Decision variables.
    pub variables: Vec<(String, Layout<usize>)>,
    /// Global expressions.
    pub global_expressions: Vec<(String, Layout<LinExpr>)>,
    /// Constraints whose duals are tracked (`config.solve.shadow_prices`).
    pub shadow_prices: Vec<(String, Layout<usize>)>,
    /// Objectives (0-d layouts, evaluated on the solution like Python does).
    pub objectives: Vec<(String, Layout<LinExpr>)>,
}

/// Loaded results.
#[derive(Debug, Clone, Default)]
pub struct Results {
    /// Arrays in Python's order.
    pub arrays: IndexMap<String, LArray<f64>>,
    /// Lower-cased HiGHS status.
    pub termination_condition: String,
}

/// Errors while loading results.
#[derive(Debug, thiserror::Error)]
pub enum ResultsError {
    /// Array construction failed.
    #[error(transparent)]
    Array(#[from] ArrayError),
    /// The postprocess hook failed.
    #[error("postprocessing: {0}")]
    Postprocess(String),
}

/// `load_results`: an empty set of arrays unless the solve was optimal (Python returns an
/// empty dataset then, with only `termination_condition`).
pub fn load_results(
    outcome: &SolveOutcome,
    layouts: &ResultLayouts,
    zero_threshold: f64,
    postprocess: Option<&dyn Postprocess>,
) -> Result<Results, ResultsError> {
    let mut out = Results {
        arrays: IndexMap::new(),
        termination_condition: outcome.termination_condition.clone(),
    };
    let Some(x) = outcome.col_value.as_deref() else {
        return Ok(out);
    };
    let a = &mut out.arrays;
    for (n, l) in &layouts.variables {
        a.insert(n.clone(), variable_values(l, x)?);
    }
    for (n, l) in &layouts.global_expressions {
        a.insert(n.clone(), expression_values(l, x)?);
    }
    for (n, l) in &layouts.shadow_prices {
        a.insert(
            format!("shadow_price_{n}"),
            shadow_prices(l, outcome.row_dual.as_deref())?,
        );
    }
    for (n, l) in &layouts.objectives {
        a.insert(n.clone(), expression_values(l, x)?);
    }
    if zero_threshold != 0.0 {
        for v in a.values_mut() {
            *v = apply_zero_threshold(v, zero_threshold);
        }
    }
    if let Some(p) = postprocess {
        let extra = p.postprocess(a).map_err(ResultsError::Postprocess)?;
        for (n, v) in extra {
            a.insert(n, v);
        }
    }
    a.retain(|_, v| v.count() > 0);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn datetime_strings() {
        assert_eq!(
            format_datetime(1_104_537_600_000_000_000),
            "2005-01-01 00:00:00"
        );
        assert_eq!(
            format_datetime(1_104_541_200_000_000_000),
            "2005-01-01 01:00:00"
        );
    }
}
