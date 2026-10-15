//! Comparison of canonical LPs.

use super::canon::{CRow, CSos, CVar, CanonOptions, Canonical, canonicalise};
use super::parse::{LpError, parse_lp};
use std::collections::HashMap;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

/// Comparison options.
#[derive(Debug, Clone, Copy)]
pub struct DiffOptions {
    /// Relative tolerance for numbers (default 1e-9).
    pub rtol: f64,
    /// Canonicalisation options.
    pub canon: CanonOptions,
}

impl Default for DiffOptions {
    fn default() -> Self {
        Self {
            rtol: 1e-9,
            canon: CanonOptions::default(),
        }
    }
}

/// How two LPs were compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Rows and variables matched by name.
    Named,
    /// At least one side has anonymous names (HiGHS); matched by colour refinement.
    Structural,
}

/// Result of a comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct LpDiff {
    /// Comparison mode used.
    pub mode: Mode,
    /// Human-readable differences; empty when equal.
    pub differences: Vec<String>,
}

impl LpDiff {
    /// True when the LPs are equal under the tolerance.
    pub fn is_equal(&self) -> bool {
        self.differences.is_empty()
    }
}

/// True when `a` and `b` are equal within relative tolerance `rtol`.
pub fn close(a: f64, b: f64, rtol: f64) -> bool {
    if a == b {
        return true;
    }
    if !a.is_finite() || !b.is_finite() {
        return false;
    }
    (a - b).abs() <= rtol * a.abs().max(b.abs())
}

/// Parse, canonicalise and compare two LP texts.
pub fn diff_lp(a: &str, b: &str, opts: &DiffOptions) -> Result<LpDiff, LpError> {
    let ca = canonicalise(&parse_lp(a)?, opts.canon);
    let cb = canonicalise(&parse_lp(b)?, opts.canon);
    Ok(diff_canonical(&ca, &cb, opts))
}

/// Compare two canonical LPs.
pub fn diff_canonical(a: &Canonical, b: &Canonical, opts: &DiffOptions) -> LpDiff {
    if a.anonymous || b.anonymous {
        structural(a, b, opts)
    } else {
        named(a, b, opts)
    }
}

fn var_diff(va: &CVar, vb: &CVar, rtol: f64, out: &mut Vec<String>) {
    if va.kind != vb.kind {
        out.push(format!(
            "var {}: kind {:?} vs {:?}",
            va.name, va.kind, vb.kind
        ));
    }
    if !close(va.lb, vb.lb, rtol) || !close(va.ub, vb.ub, rtol) {
        out.push(format!(
            "var {}: bounds [{}, {}] vs [{}, {}]",
            va.name, va.lb, va.ub, vb.lb, vb.ub
        ));
    }
}

fn terms_diff(
    ctx: &str,
    a: &Canonical,
    ta: &[(usize, f64)],
    b: &Canonical,
    tb: &[(usize, f64)],
    rtol: f64,
    out: &mut Vec<String>,
) {
    let ma: HashMap<&str, f64> = ta
        .iter()
        .map(|(v, c)| (a.vars[*v].name.as_str(), *c))
        .collect();
    let mb: HashMap<&str, f64> = tb
        .iter()
        .map(|(v, c)| (b.vars[*v].name.as_str(), *c))
        .collect();
    let mut names: Vec<&str> = ma.keys().chain(mb.keys()).copied().collect();
    names.sort_unstable();
    names.dedup();
    for n in names {
        match (ma.get(n), mb.get(n)) {
            (Some(x), Some(y)) if !close(*x, *y, rtol) => {
                out.push(format!("{ctx}: coefficient of {n}: {x} vs {y}"));
            }
            (Some(x), None) => out.push(format!("{ctx}: term {x} {n} only in A")),
            (None, Some(y)) => out.push(format!("{ctx}: term {y} {n} only in B")),
            _ => {}
        }
    }
}

/// True when `s` is `r` multiplied by -1 (same feasible set, opposite orientation).
fn row_negated_equal(a: &Canonical, r: &CRow, b: &Canonical, s: &CRow, rtol: f64) -> bool {
    if r.terms.len() != s.terms.len() || !close(r.lo, -s.hi, rtol) || !close(r.hi, -s.lo, rtol) {
        return false;
    }
    r.terms
        .iter()
        .zip(&s.terms)
        .all(|(x, y)| a.vars[x.0].name == b.vars[y.0].name && close(x.1, -y.1, rtol))
}

fn named(a: &Canonical, b: &Canonical, opts: &DiffOptions) -> LpDiff {
    let rtol = opts.rtol;
    let mut out = Vec::new();
    if a.sense != b.sense {
        out.push(format!("objective sense: {:?} vs {:?}", a.sense, b.sense));
    }
    if !close(a.offset, b.offset, rtol) {
        out.push(format!("objective offset: {} vs {}", a.offset, b.offset));
    }
    terms_diff("objective", a, &a.obj, b, &b.obj, rtol, &mut out);

    let vb: HashMap<&str, &CVar> = b.vars.iter().map(|v| (v.name.as_str(), v)).collect();
    let va: HashMap<&str, &CVar> = a.vars.iter().map(|v| (v.name.as_str(), v)).collect();
    for v in &a.vars {
        match vb.get(v.name.as_str()) {
            Some(w) => var_diff(v, w, rtol, &mut out),
            None => out.push(format!("var {} only in A", v.name)),
        }
    }
    for v in &b.vars {
        if !va.contains_key(v.name.as_str()) {
            out.push(format!("var {} only in B", v.name));
        }
    }

    let rb: HashMap<&str, &CRow> = b.rows.iter().map(|r| (r.name.as_str(), r)).collect();
    let ra: HashMap<&str, &CRow> = a.rows.iter().map(|r| (r.name.as_str(), r)).collect();
    for r in &a.rows {
        match rb.get(r.name.as_str()) {
            Some(s) => {
                if row_negated_equal(a, r, b, s, rtol) {
                    continue;
                }
                if !close(r.lo, s.lo, rtol) || !close(r.hi, s.hi, rtol) {
                    out.push(format!(
                        "row {}: range [{}, {}] vs [{}, {}]",
                        r.name, r.lo, r.hi, s.lo, s.hi
                    ));
                }
                terms_diff(
                    &format!("row {}", r.name),
                    a,
                    &r.terms,
                    b,
                    &s.terms,
                    rtol,
                    &mut out,
                );
            }
            None => out.push(format!("row {} only in A", r.name)),
        }
    }
    for r in &b.rows {
        if !ra.contains_key(r.name.as_str()) {
            out.push(format!("row {} only in B", r.name));
        }
    }

    let sb: HashMap<&str, _> = b.sos.iter().map(|s| (s.name.as_str(), s)).collect();
    for s in &a.sos {
        match sb.get(s.name.as_str()) {
            Some(t) => {
                let same = s.kind == t.kind
                    && s.members.len() == t.members.len()
                    && s.members.iter().zip(&t.members).all(|(x, y)| {
                        a.vars[x.0].name == b.vars[y.0].name && close(x.1, y.1, rtol)
                    });
                if !same {
                    out.push(format!("sos {} differs", s.name));
                }
            }
            None => out.push(format!("sos {} only in A", s.name)),
        }
    }
    for s in &b.sos {
        if !a.sos.iter().any(|t| t.name == s.name) {
            out.push(format!("sos {} only in B", s.name));
        }
    }
    LpDiff {
        mode: Mode::Named,
        differences: out,
    }
}

// ---------------------------------------------------------------------------
// Structural comparison (Weisfeiler-Leman colour refinement)
// ---------------------------------------------------------------------------

fn h<T: Hash>(x: &T) -> u64 {
    let mut s = DefaultHasher::new();
    x.hash(&mut s);
    s.finish()
}

/// Hash of a number quantised to about 9 significant digits.
///
/// The grid is offset by an irrational fraction of a cell so that "nice" decimal values (which
/// sit exactly on a rounding boundary) stay in one cell when the two writers add 1e-16 noise.
fn hq(x: f64) -> u64 {
    if x == 0.0 {
        return 0;
    }
    if x.is_infinite() {
        return h(&(x > 0.0));
    }
    let a = x.abs();
    let mut e = a.log10().floor() as i32;
    let mut m = a / 10f64.powi(e);
    if m < 1.0 {
        e -= 1;
        m *= 10.0;
    } else if m >= 10.0 {
        e += 1;
        m /= 10.0;
    }
    let mut cell = (m * 1e8 + 0.414_213_562).floor() as i64;
    if cell >= 1_000_000_000 {
        cell = 100_000_000;
        e += 1;
    }
    h(&(x < 0.0, e, cell))
}

/// Fold single-variable rows into variable bounds and drop trivially satisfied empty rows.
///
/// HiGHS (through Calliope's backend) states `x <= u` and `x = v` as bounds where Pyomo
/// writes a row; both describe the same feasible set.
pub fn fold_singleton_rows(c: &Canonical) -> Canonical {
    let mut vars = c.vars.clone();
    let mut rows = Vec::new();
    for r in &c.rows {
        match r.terms.as_slice() {
            [] if r.lo <= 0.0 && r.hi >= 0.0 => {}
            [(v, a)] => {
                let (mut lo, mut hi) = (r.lo / a, r.hi / a);
                if *a < 0.0 {
                    std::mem::swap(&mut lo, &mut hi);
                }
                let var = &mut vars[*v];
                var.lb = var.lb.max(lo);
                var.ub = var.ub.min(hi);
            }
            _ => rows.push(r.clone()),
        }
    }
    let mut used = vec![false; vars.len()];
    for (v, _) in &c.obj {
        used[*v] = true;
    }
    for r in &rows {
        for (v, _) in &r.terms {
            used[*v] = true;
        }
    }
    for s in &c.sos {
        for (v, _) in &s.members {
            used[*v] = true;
        }
    }
    let mut remap = vec![usize::MAX; vars.len()];
    let mut kept = Vec::new();
    for (i, v) in vars.into_iter().enumerate() {
        if used[i] {
            remap[i] = kept.len();
            kept.push(v);
        }
    }
    let fix = |t: &[(usize, f64)]| t.iter().map(|(v, x)| (remap[*v], *x)).collect::<Vec<_>>();
    Canonical {
        vars: kept,
        obj: fix(&c.obj),
        rows: rows
            .into_iter()
            .map(|r| CRow {
                terms: fix(&r.terms),
                ..r
            })
            .collect(),
        sos: c
            .sos
            .iter()
            .map(|s| CSos {
                members: fix(&s.members),
                ..s.clone()
            })
            .collect(),
        ..c.clone()
    }
}

struct Graph<'a> {
    c: &'a Canonical,
    var_col: Vec<u64>,
    row_col: Vec<u64>,
    /// Rows with `lo + hi == 0` (or free) whose orientation is chosen by the refinement.
    ambiguous: Vec<bool>,
    /// Per variable: rows it appears in, with the coefficient.
    var_adj: Vec<Vec<(usize, f64)>>,
    row_adj: Vec<Vec<(usize, f64)>>,
}

impl<'a> Graph<'a> {
    fn new(c: &'a Canonical) -> Self {
        let mut obj = vec![0.0; c.vars.len()];
        for (v, x) in &c.obj {
            obj[*v] = *x;
        }
        let var_col = c
            .vars
            .iter()
            .enumerate()
            .map(|(i, v)| h(&(v.kind as u8, hq(v.lb), hq(v.ub), hq(obj[i]))))
            .collect();
        let mut row_sign = Vec::new();
        let mut ambiguous = Vec::new();
        let mut row_col = Vec::new();
        for r in &c.rows {
            let sum = r.lo + r.hi;
            let amb = sum.is_nan() || sum.abs() <= 1e-9 * r.lo.abs().max(r.hi.abs()).min(1e300);
            let sign = if !amb && sum < 0.0 { -1.0 } else { 1.0 };
            let (lo, hi) = if sign < 0.0 {
                (-r.hi, -r.lo)
            } else {
                (r.lo, r.hi)
            };
            row_sign.push(sign);
            ambiguous.push(amb);
            row_col.push(h(&(hq(lo), hq(hi), r.terms.len())));
        }
        let mut var_adj = vec![Vec::new(); c.vars.len()];
        let mut row_adj = vec![Vec::new(); c.rows.len()];
        for (ri, r) in c.rows.iter().enumerate() {
            for (v, x) in &r.terms {
                let x = x * row_sign[ri];
                var_adj[*v].push((ri, x));
                row_adj[ri].push((*v, x));
            }
        }
        Self {
            c,
            var_col,
            row_col,
            ambiguous,
            var_adj,
            row_adj,
        }
    }

    fn classes(&self) -> usize {
        let mut v = self.var_col.clone();
        v.extend(&self.row_col);
        v.sort_unstable();
        v.dedup();
        v.len()
    }

    fn refine(&mut self) {
        // Orientation of ambiguous rows: the sign giving the smaller neighbour hash.
        let mut flip = vec![1.0; self.row_adj.len()];
        let mut new_rows = Vec::with_capacity(self.row_adj.len());
        for (ri, adj) in self.row_adj.iter().enumerate() {
            let hash_with = |s: f64| {
                let mut m: Vec<u64> = adj
                    .iter()
                    .map(|(v, x)| h(&(hq(x * s), self.var_col[*v])))
                    .collect();
                m.sort_unstable();
                h(&(self.row_col[ri], m))
            };
            let plus = hash_with(1.0);
            if self.ambiguous[ri] {
                let minus = hash_with(-1.0);
                if minus < plus {
                    flip[ri] = -1.0;
                }
                new_rows.push(plus.min(minus));
            } else {
                new_rows.push(plus);
            }
        }
        let new_vars: Vec<u64> = self
            .var_adj
            .iter()
            .zip(&self.var_col)
            .map(|(adj, col)| {
                let mut m: Vec<u64> = adj
                    .iter()
                    .map(|(r, x)| h(&(hq(x * flip[*r]), self.row_col[*r])))
                    .collect();
                m.sort_unstable();
                h(&(*col, m))
            })
            .collect();
        self.row_col = new_rows;
        self.var_col = new_vars;
    }
}

fn structural(a: &Canonical, b: &Canonical, opts: &DiffOptions) -> LpDiff {
    let (a, b) = (&fold_singleton_rows(a), &fold_singleton_rows(b));
    let mut out = Vec::new();
    if a.sense != b.sense {
        out.push(format!("objective sense: {:?} vs {:?}", a.sense, b.sense));
    }
    if !close(a.offset, b.offset, opts.rtol) {
        out.push(format!("objective offset: {} vs {}", a.offset, b.offset));
    }
    if a.rows.len() != b.rows.len() {
        out.push(format!("row count: {} vs {}", a.rows.len(), b.rows.len()));
    }
    if a.vars.len() != b.vars.len() {
        out.push(format!(
            "variable count: {} vs {}",
            a.vars.len(),
            b.vars.len()
        ));
    }
    if a.sos.len() != b.sos.len() {
        out.push(format!("sos count: {} vs {}", a.sos.len(), b.sos.len()));
    }
    let (mut ga, mut gb) = (Graph::new(a), Graph::new(b));
    let (mut na, mut nb) = (ga.classes(), gb.classes());
    for _ in 0..200 {
        ga.refine();
        gb.refine();
        let (ma, mb) = (ga.classes(), gb.classes());
        let stable = ma == na && mb == nb;
        (na, nb) = (ma, mb);
        if stable {
            break;
        }
    }
    let describe = |g: &Graph<'_>, other: &Graph<'_>, side: &str, out: &mut Vec<String>| {
        let mut count: HashMap<u64, i64> = HashMap::new();
        for &c in &other.row_col {
            *count.entry(c).or_insert(0) += 1;
        }
        let mut unmatched = 0;
        for (i, &c) in g.row_col.iter().enumerate() {
            let e = count.entry(c).or_insert(0);
            if *e > 0 {
                *e -= 1;
            } else {
                unmatched += 1;
                if unmatched <= 10 {
                    out.push(format!(
                        "row with no counterpart in the other LP (only in {side}): {}",
                        g.c.row_text(&g.c.rows[i])
                    ));
                }
            }
        }
        if unmatched > 10 {
            out.push(format!(
                "... {} more unmatched rows in {side}",
                unmatched - 10
            ));
        }
        let mut vcount: HashMap<u64, i64> = HashMap::new();
        for &c in &other.var_col {
            *vcount.entry(c).or_insert(0) += 1;
        }
        let n_var = g
            .var_col
            .iter()
            .filter(|c| {
                let e = vcount.entry(**c).or_insert(0);
                if *e > 0 {
                    *e -= 1;
                    false
                } else {
                    true
                }
            })
            .count();
        if n_var > 0 {
            out.push(format!(
                "{n_var} variable(s) in {side} have no structural counterpart"
            ));
        }
    };
    let mut sa = ga.row_col.clone();
    let mut sb = gb.row_col.clone();
    sa.sort_unstable();
    sb.sort_unstable();
    let mut va = ga.var_col.clone();
    let mut vb = gb.var_col.clone();
    va.sort_unstable();
    vb.sort_unstable();
    if sa != sb || va != vb {
        describe(&ga, &gb, "A", &mut out);
        describe(&gb, &ga, "B", &mut out);
        if out.is_empty() {
            out.push("structures differ".to_string());
        }
    }
    LpDiff {
        mode: Mode::Structural,
        differences: out,
    }
}
