//! Solver-independent sparse LP/MILP model.
//!
//! [`LpModel`] is what the math evaluator emits and what the HiGHS backend consumes.
//! Columns and rows carry a [`Label`] (component name plus index tuple) so results can be
//! mapped back to labelled arrays and so the LP writer can produce Pyomo-style names
//! (`variables(flow_cap)(a__b)`) that [`crate::lp_format`] canonicalises like Calliope's.
//!
//! The constraint matrix is stored row-wise (CSR): rows are appended with their terms,
//! which matches how the evaluator produces them; HiGHS accepts row-wise matrices directly.

use std::fmt;
use std::sync::Arc;

/// Index of a column (variable) in an [`LpModel`].
pub type ColId = usize;
/// Index of a row (constraint) in an [`LpModel`].
pub type RowId = usize;

/// What kind of math component a label belongs to; selects the LP name prefix.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum ComponentKind {
    /// `variables`.
    Variable,
    /// `constraints`.
    Constraint,
    /// `piecewise_constraints` (SOS2 sets and their auxiliary columns/rows).
    Piecewise,
    /// `objectives`.
    Objective,
    /// Anything else (e.g. helper columns); written with this prefix verbatim.
    Other(&'static str),
}

impl ComponentKind {
    /// Pyomo block name used in LP files.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Variable => "variables",
            Self::Constraint => "constraints",
            Self::Piecewise => "piecewise_constraints",
            Self::Objective => "objectives",
            Self::Other(p) => p,
        }
    }
}

/// Name of one column, row or SOS set: component plus index tuple.
///
/// Index items are `Arc<str>` so coordinate strings can be shared across many labels.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Label {
    /// Component kind (selects the LP name prefix).
    pub kind: ComponentKind,
    /// Component name, e.g. `flow_cap`.
    pub component: Arc<str>,
    /// Index tuple in the component's dimension order (C-order of `foreach`).
    pub index: Box<[Arc<str>]>,
    /// Suffix appended after the index in LP names (Pyomo piecewise `_v(0)` etc.); usually empty.
    pub suffix: Option<Arc<str>>,
}

impl Label {
    /// Label without suffix.
    pub fn new(
        kind: ComponentKind,
        component: impl Into<Arc<str>>,
        index: impl IntoIterator<Item = Arc<str>>,
    ) -> Self {
        Self {
            kind,
            component: component.into(),
            index: index.into_iter().collect(),
            suffix: None,
        }
    }

    /// Convenience constructor from string slices.
    pub fn from_strs(kind: ComponentKind, component: &str, index: &[&str]) -> Self {
        Self::new(kind, component, index.iter().map(|s| Arc::from(*s)))
    }

    /// Same label with a suffix.
    #[must_use]
    pub fn with_suffix(mut self, suffix: impl Into<Arc<str>>) -> Self {
        self.suffix = Some(suffix.into());
        self
    }
}

impl fmt::Display for Label {
    /// Pyomo-style LP name: `prefix(component)(i__j)suffix`, with LP-unsafe characters
    /// replaced by `_` (as Pyomo's symbol map does).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.kind == ComponentKind::Other("") {
            // verbatim name (e.g. imported from an LP file)
            return f.write_str(&self.component);
        }
        let mut s = String::new();
        s.push_str(self.kind.prefix());
        s.push('(');
        s.push_str(&self.component);
        s.push_str(")(");
        for (i, item) in self.index.iter().enumerate() {
            if i > 0 {
                s.push_str("__");
            }
            s.push_str(item);
        }
        if self.index.is_empty() {
            s.push('0');
        }
        s.push(')');
        if let Some(sfx) = &self.suffix {
            s.push_str(sfx);
        }
        f.write_str(&lp_safe(&s))
    }
}

/// Replace characters that CPLEX LP names cannot contain, like Pyomo's LP writer.
pub fn lp_safe(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            ' ' | '-' | ':' | '+' | '*' | '/' | '^' | '<' | '>' | '=' | '[' | ']' | ',' | '.'
            | '\'' | '"' => '_',
            c => c,
        })
        .collect()
}

/// A column (decision variable).
#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    /// Name.
    pub label: Label,
    /// Lower bound (`-inf` allowed).
    pub lb: f64,
    /// Upper bound (`inf` allowed).
    pub ub: f64,
    /// Integer (binary = integer with bounds `[0, 1]`).
    pub integer: bool,
}

/// A row `lb <= a·x <= ub` (equality when `lb == ub`).
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    /// Name.
    pub label: Label,
    /// Lower activity bound (`-inf` for `<=` rows).
    pub lb: f64,
    /// Upper activity bound (`inf` for `>=` rows).
    pub ub: f64,
}

/// Inequality sense, for constructing rows from `sense rhs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RowSense {
    /// `a·x <= rhs`.
    Le,
    /// `a·x >= rhs`.
    Ge,
    /// `a·x == rhs`.
    Eq,
}

impl RowSense {
    /// `(lb, ub)` for this sense and right-hand side.
    pub fn bounds(self, rhs: f64) -> (f64, f64) {
        match self {
            Self::Le => (f64::NEG_INFINITY, rhs),
            Self::Ge => (rhs, f64::INFINITY),
            Self::Eq => (rhs, rhs),
        }
    }
}

/// Objective direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ObjSense {
    /// Minimise.
    #[default]
    Minimize,
    /// Maximise.
    Maximize,
}

/// Affine expression `sum(coef * x[col]) + constant`.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LinExpr {
    /// `(column, coefficient)`; may repeat a column (summed on use).
    pub terms: Vec<(ColId, f64)>,
    /// Constant term.
    pub constant: f64,
}

impl LinExpr {
    /// Value at a primal solution.
    pub fn eval(&self, x: &[f64]) -> f64 {
        self.terms
            .iter()
            .fold(self.constant, |acc, &(c, a)| acc + a * x[c])
    }

    /// Terms with repeated columns summed and zeros dropped, sorted by column.
    pub fn merged(&self) -> Vec<(ColId, f64)> {
        merge_terms(self.terms.iter().copied())
    }
}

/// Objective function.
#[derive(Debug, Clone, PartialEq)]
pub struct Objective {
    /// Name (`objectives(<name>)(0)` in LP files).
    pub label: Label,
    /// Direction.
    pub sense: ObjSense,
    /// Linear part and constant.
    pub expr: LinExpr,
}

impl Default for Objective {
    fn default() -> Self {
        Self {
            label: Label::new(ComponentKind::Objective, "objective", []),
            sense: ObjSense::Minimize,
            expr: LinExpr::default(),
        }
    }
}

/// An SOS2 set (as used by Calliope's piecewise constraints).
#[derive(Debug, Clone, PartialEq)]
pub struct Sos2 {
    /// Name.
    pub label: Label,
    /// Members with strictly increasing weights.
    pub members: Vec<(ColId, f64)>,
}

/// Sparse LP/MILP: columns, row-wise constraint matrix, objective, SOS2 sets.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LpModel {
    /// Columns in solver order.
    pub cols: Vec<Column>,
    /// Rows in solver order.
    pub rows: Vec<Row>,
    /// CSR row starts; `row_start.len() == rows.len() + 1` (`[0]` when empty).
    pub row_start: Vec<usize>,
    /// CSR column indices.
    pub row_index: Vec<ColId>,
    /// CSR values.
    pub row_value: Vec<f64>,
    /// Objective.
    pub objective: Objective,
    /// SOS2 sets.
    pub sos2: Vec<Sos2>,
}

/// Sum duplicate columns, drop exact zeros, sort by column.
pub fn merge_terms(terms: impl IntoIterator<Item = (ColId, f64)>) -> Vec<(ColId, f64)> {
    let mut v: Vec<(ColId, f64)> = terms.into_iter().collect();
    v.sort_by_key(|t| t.0);
    let mut out: Vec<(ColId, f64)> = Vec::with_capacity(v.len());
    for (c, a) in v {
        match out.last_mut() {
            Some(last) if last.0 == c => last.1 += a,
            _ => out.push((c, a)),
        }
    }
    out.retain(|t| t.1 != 0.0);
    out
}

impl LpModel {
    /// Empty model.
    pub fn new() -> Self {
        Self {
            row_start: vec![0],
            ..Self::default()
        }
    }

    /// Number of columns.
    pub fn num_cols(&self) -> usize {
        self.cols.len()
    }

    /// Number of rows.
    pub fn num_rows(&self) -> usize {
        self.rows.len()
    }

    /// Number of stored nonzeros.
    pub fn num_nz(&self) -> usize {
        self.row_value.len()
    }

    /// True when any column is integer or an SOS set exists.
    pub fn is_mip(&self) -> bool {
        self.cols.iter().any(|c| c.integer) || !self.sos2.is_empty()
    }

    fn ensure_start(&mut self) {
        if self.row_start.is_empty() {
            self.row_start.push(0);
        }
    }

    /// Append a column.
    pub fn add_col(&mut self, label: Label, lb: f64, ub: f64, integer: bool) -> ColId {
        self.cols.push(Column {
            label,
            lb,
            ub,
            integer,
        });
        self.cols.len() - 1
    }

    /// Append a row `lb <= sum(terms) <= ub`; repeated columns are summed, zeros dropped.
    ///
    /// # Panics
    /// If a term references a column that does not exist.
    pub fn add_row(
        &mut self,
        label: Label,
        lb: f64,
        ub: f64,
        terms: impl IntoIterator<Item = (ColId, f64)>,
    ) -> RowId {
        self.ensure_start();
        for (c, a) in merge_terms(terms) {
            assert!(
                c < self.cols.len(),
                "row term references unknown column {c}"
            );
            self.row_index.push(c);
            self.row_value.push(a);
        }
        self.row_start.push(self.row_index.len());
        self.rows.push(Row { label, lb, ub });
        self.rows.len() - 1
    }

    /// Append a row `terms sense rhs`.
    pub fn add_row_sense(
        &mut self,
        label: Label,
        terms: impl IntoIterator<Item = (ColId, f64)>,
        sense: RowSense,
        rhs: f64,
    ) -> RowId {
        let (lb, ub) = sense.bounds(rhs);
        self.add_row(label, lb, ub, terms)
    }

    /// Terms of a row as `(columns, values)`.
    pub fn row(&self, r: RowId) -> (&[ColId], &[f64]) {
        let (s, e) = (self.row_start[r], self.row_start[r + 1]);
        (&self.row_index[s..e], &self.row_value[s..e])
    }

    /// Replace the objective.
    pub fn set_objective(&mut self, label: Label, sense: ObjSense, expr: LinExpr) {
        self.objective = Objective { label, sense, expr };
    }

    /// Append an SOS2 set.
    pub fn add_sos2(&mut self, label: Label, members: Vec<(ColId, f64)>) -> usize {
        self.sos2.push(Sos2 { label, members });
        self.sos2.len() - 1
    }

    /// Dense objective coefficient vector (duplicates summed).
    pub fn objective_dense(&self) -> Vec<f64> {
        let mut c = vec![0.0; self.cols.len()];
        for &(j, a) in &self.objective.expr.terms {
            c[j] += a;
        }
        c
    }

    /// Column-wise (CSC) copy of the matrix: `(col_start, row_index, value)`.
    pub fn to_csc(&self) -> (Vec<usize>, Vec<RowId>, Vec<f64>) {
        let n = self.cols.len();
        let mut count = vec![0usize; n + 1];
        for &c in &self.row_index {
            count[c + 1] += 1;
        }
        for j in 0..n {
            count[j + 1] += count[j];
        }
        let start = count.clone();
        let mut next = count;
        let mut idx = vec![0; self.row_index.len()];
        let mut val = vec![0.0; self.row_index.len()];
        for r in 0..self.rows.len() {
            let (cols, vals) = self.row(r);
            for (&c, &v) in cols.iter().zip(vals) {
                idx[next[c]] = r;
                val[next[c]] = v;
                next[c] += 1;
            }
        }
        (start, idx, val)
    }

    /// Row activities `A·x`.
    pub fn row_activity(&self, x: &[f64]) -> Vec<f64> {
        (0..self.rows.len())
            .map(|r| {
                let (c, v) = self.row(r);
                c.iter().zip(v).map(|(&j, &a)| a * x[j]).sum()
            })
            .collect()
    }

    /// Column ids whose label belongs to `component` (any kind matching `kind`).
    pub fn cols_of(&self, kind: ComponentKind, component: &str) -> Vec<ColId> {
        self.cols
            .iter()
            .enumerate()
            .filter(|(_, c)| c.label.kind == kind && &*c.label.component == component)
            .map(|(i, _)| i)
            .collect()
    }

    /// Row ids whose label belongs to `component`.
    pub fn rows_of(&self, kind: ComponentKind, component: &str) -> Vec<RowId> {
        self.rows
            .iter()
            .enumerate()
            .filter(|(_, r)| r.label.kind == kind && &*r.label.component == component)
            .map(|(i, _)| i)
            .collect()
    }

    /// Remove rows (keeping the order of the rest); returns the old→new row map.
    pub fn delete_rows(&mut self, del: &[RowId]) -> Vec<Option<RowId>> {
        let mut mask = vec![false; self.rows.len()];
        for &r in del {
            mask[r] = true;
        }
        let mut map = vec![None; self.rows.len()];
        let (mut start, mut idx, mut val, mut rows) = (vec![0], vec![], vec![], vec![]);
        for (r, row) in self.rows.iter().enumerate() {
            if mask[r] {
                continue;
            }
            let (c, v) = self.row(r);
            idx.extend_from_slice(c);
            val.extend_from_slice(v);
            start.push(idx.len());
            map[r] = Some(rows.len());
            rows.push(row.clone());
        }
        self.rows = rows;
        self.row_start = start;
        self.row_index = idx;
        self.row_value = val;
        map
    }

    /// Remove columns, dropping their matrix/objective/SOS entries; returns old→new map.
    pub fn delete_cols(&mut self, del: &[ColId]) -> Vec<Option<ColId>> {
        let mut map: Vec<Option<ColId>> = vec![None; self.cols.len()];
        let mut mask = vec![false; self.cols.len()];
        for &c in del {
            mask[c] = true;
        }
        let mut k = 0;
        for (j, m) in mask.iter().enumerate() {
            if !m {
                map[j] = Some(k);
                k += 1;
            }
        }
        let mut cols = Vec::with_capacity(k);
        for (j, c) in self.cols.drain(..).enumerate() {
            if !mask[j] {
                cols.push(c);
            }
        }
        self.cols = cols;
        let (mut start, mut idx, mut val) = (vec![0], vec![], vec![]);
        for r in 0..self.rows.len() {
            let (s, e) = (self.row_start[r], self.row_start[r + 1]);
            for p in s..e {
                if let Some(n) = map[self.row_index[p]] {
                    idx.push(n);
                    val.push(self.row_value[p]);
                }
            }
            start.push(idx.len());
        }
        self.row_start = start;
        self.row_index = idx;
        self.row_value = val;
        let remap = |terms: &mut Vec<(ColId, f64)>| {
            terms.retain(|t| map[t.0].is_some());
            for t in terms.iter_mut() {
                t.0 = map[t.0].unwrap_or(t.0);
            }
        };
        remap(&mut self.objective.expr.terms);
        for s in &mut self.sos2 {
            remap(&mut s.members);
        }
        map
    }
}

fn label_from_lp_name(name: &str) -> Label {
    let parsed = (|| {
        let open1 = name.find('(')?;
        let close1 = open1 + name[open1..].find(')')?;
        let rest = name[close1 + 1..].strip_prefix('(')?;
        let close2 = rest.find(')')?;
        let kind = match &name[..open1] {
            "variables" => ComponentKind::Variable,
            "constraints" => ComponentKind::Constraint,
            "piecewise_constraints" => ComponentKind::Piecewise,
            "objectives" => ComponentKind::Objective,
            _ => return None,
        };
        let idx = &rest[..close2];
        let index: Vec<Arc<str>> =
            if idx.is_empty() || (kind == ComponentKind::Objective && idx == "0") {
                Vec::new()
            } else {
                idx.split("__").map(Arc::from).collect()
            };
        let mut l = Label::new(kind, &name[open1 + 1..close1], index);
        let sfx = &rest[close2 + 1..];
        if !sfx.is_empty() {
            l.suffix = Some(Arc::from(sfx));
        }
        Some(l)
    })();
    parsed.unwrap_or_else(|| Label::new(ComponentKind::Other(""), name, []))
}

impl LpModel {
    /// Build a model from a parsed LP file (columns in order of first appearance:
    /// objective, rows, bounds, integer sections, SOS). `ONE_VAR_CONSTANT` is folded into
    /// the objective constant. Pyomo row prefixes and trailing `_` are stripped from row names.
    pub fn from_raw(raw: &crate::lp_format::RawLp) -> Self {
        use crate::lp_format::{ONE_VAR_CONSTANT, Sense};
        use std::collections::HashMap;
        let mut m = Self::new();
        let mut ids: HashMap<&str, ColId> = HashMap::new();
        let one = raw.bounds.get(ONE_VAR_CONSTANT).map_or(1.0, |b| b.0);
        let ints: std::collections::HashSet<&str> = raw
            .generals
            .iter()
            .chain(&raw.binaries)
            .map(String::as_str)
            .collect();
        let bins: std::collections::HashSet<&str> =
            raw.binaries.iter().map(String::as_str).collect();
        let mut bound_names: Vec<&str> = raw.bounds.keys().map(String::as_str).collect();
        bound_names.sort_unstable();
        let names = raw
            .obj_terms
            .iter()
            .map(|t| t.0.as_str())
            .chain(
                raw.rows
                    .iter()
                    .flat_map(|r| r.terms.iter().map(|t| t.0.as_str())),
            )
            .chain(bound_names)
            .chain(raw.generals.iter().map(String::as_str))
            .chain(raw.binaries.iter().map(String::as_str))
            .chain(
                raw.sos
                    .iter()
                    .flat_map(|s| s.members.iter().map(|t| t.0.as_str())),
            );
        for n in names {
            if n == ONE_VAR_CONSTANT || ids.contains_key(n) {
                continue;
            }
            let binary = bins.contains(n);
            let (lb, ub) = if binary {
                (0.0, 1.0)
            } else {
                raw.bounds.get(n).copied().unwrap_or((0.0, f64::INFINITY))
            };
            let id = m.add_col(label_from_lp_name(n), lb, ub, ints.contains(n));
            ids.insert(n, id);
        }
        let mut constant = raw.obj_const;
        let mut terms = Vec::new();
        for (n, a) in &raw.obj_terms {
            if n == ONE_VAR_CONSTANT {
                constant += a * one;
            } else {
                terms.push((ids[n.as_str()], *a));
            }
        }
        let sense = match raw.sense {
            Sense::Min => ObjSense::Minimize,
            Sense::Max => ObjSense::Maximize,
        };
        m.set_objective(
            Label::new(ComponentKind::Other(""), "objective", []),
            sense,
            LinExpr { terms, constant },
        );
        for r in &raw.rows {
            let mut shift = 0.0;
            let mut terms = Vec::with_capacity(r.terms.len());
            for (n, a) in &r.terms {
                if n == ONE_VAR_CONSTANT {
                    shift += a * one;
                } else {
                    terms.push((ids[n.as_str()], *a));
                }
            }
            let base = strip_row_prefix(&r.name);
            m.add_row(label_from_lp_name(base), r.lo - shift, r.hi - shift, terms);
        }
        for s in &raw.sos {
            let members = s
                .members
                .iter()
                .map(|(n, w)| (ids[n.as_str()], *w))
                .collect();
            let name = s.name.strip_suffix("_s").unwrap_or(&s.name);
            m.add_sos2(label_from_lp_name(name), members);
        }
        m
    }
}

fn strip_row_prefix(name: &str) -> &str {
    let b = name.as_bytes();
    let s = if b.len() > 4
        && matches!(b[0], b'c' | b'r')
        && b[1] == b'_'
        && matches!(b[2], b'l' | b'u' | b'e')
        && b[3] == b'_'
    {
        &name[4..]
    } else {
        name
    };
    s.strip_suffix('_').unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(crate) fn small() -> LpModel {
        let mut m = LpModel::new();
        let x = m.add_col(
            Label::from_strs(ComponentKind::Variable, "x", &["b", "a"]),
            0.0,
            10.0,
            true,
        );
        let y = m.add_col(
            Label::from_strs(ComponentKind::Variable, "y", &["a"]),
            f64::NEG_INFINITY,
            f64::INFINITY,
            false,
        );
        m.add_row_sense(
            Label::from_strs(ComponentKind::Constraint, "r", &["b", "a"]),
            [(x, 1.0), (y, 1.0)],
            RowSense::Ge,
            5.0,
        );
        m.add_row_sense(
            Label::from_strs(ComponentKind::Constraint, "e", &["a"]),
            [(y, 1.0)],
            RowSense::Eq,
            1.0,
        );
        m.set_objective(
            Label::from_strs(ComponentKind::Objective, "obj", &[]),
            ObjSense::Minimize,
            LinExpr {
                terms: vec![(x, 3.0)],
                constant: 2.0,
            },
        );
        m
    }

    #[test]
    fn build_and_csc() {
        let m = small();
        assert_eq!((m.num_cols(), m.num_rows(), m.num_nz()), (2, 2, 3));
        let (s, i, v) = m.to_csc();
        assert_eq!(s, vec![0, 1, 3]);
        assert_eq!(i, vec![0, 0, 1]);
        assert_eq!(v, vec![1.0, 1.0, 1.0]);
        assert_eq!(m.row_activity(&[4.0, 1.0]), vec![5.0, 1.0]);
        assert_eq!(m.cols[0].label.to_string(), "variables(x)(b__a)");
        assert!(m.is_mip());
    }

    #[test]
    fn merge_and_delete() {
        assert_eq!(
            merge_terms([(2, 1.0), (0, 1.0), (2, -1.0), (0, 2.0)]),
            vec![(0, 3.0)]
        );
        let mut m = small();
        let map = m.delete_cols(&[0]);
        assert_eq!(map, vec![None, Some(0)]);
        assert_eq!(m.row(0), (&[0usize][..], &[1.0][..]));
        assert!(m.objective.expr.terms.is_empty());
        m.delete_rows(&[0]);
        assert_eq!(m.num_rows(), 1);
        assert_eq!(m.rows[0].label.component.as_ref(), "e");
    }
}
