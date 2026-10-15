//! Canonical form of an LP: renamed, sorted, merged, numerically normalised.

use super::parse::{RawLp, Sense};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fmt::Write as _;

/// Pyomo's name for the fixed variable that carries objective constants.
pub const ONE_VAR_CONSTANT: &str = "ONE_VAR_CONSTANT";

/// Variable domain.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Kind {
    /// Continuous.
    Continuous,
    /// General integer.
    Integer,
    /// Binary (bounds fixed to `[0, 1]`).
    Binary,
}

/// A canonical variable.
#[derive(Debug, Clone, PartialEq)]
pub struct CVar {
    /// Canonical name.
    pub name: String,
    /// Domain.
    pub kind: Kind,
    /// Lower bound.
    pub lb: f64,
    /// Upper bound.
    pub ub: f64,
}

/// A canonical row `lo <= sum <= hi`; terms sorted by variable index.
#[derive(Debug, Clone, PartialEq)]
pub struct CRow {
    /// Canonical name (row-type prefix and trailing `_` removed).
    pub name: String,
    /// Lower bound.
    pub lo: f64,
    /// Upper bound.
    pub hi: f64,
    /// `(variable index, coefficient)`.
    pub terms: Vec<(usize, f64)>,
}

/// A canonical SOS set.
#[derive(Debug, Clone, PartialEq)]
pub struct CSos {
    /// Canonical name.
    pub name: String,
    /// 1 or 2.
    pub kind: u8,
    /// Members sorted by weight then name.
    pub members: Vec<(usize, f64)>,
}

/// Canonical LP.
#[derive(Debug, Clone, PartialEq)]
pub struct Canonical {
    /// Objective direction.
    pub sense: Sense,
    /// Objective constant (includes `ONE_VAR_CONSTANT` contributions).
    pub offset: f64,
    /// Objective terms sorted by variable index.
    pub obj: Vec<(usize, f64)>,
    /// Variables referenced by the objective, a row or an SOS set; sorted by name.
    pub vars: Vec<CVar>,
    /// Rows sorted by name.
    pub rows: Vec<CRow>,
    /// SOS sets sorted by name.
    pub sos: Vec<CSos>,
    /// True when variable names carry no information (HiGHS `c0`, `c1`, ...).
    pub anonymous: bool,
    /// True when index sorting was disabled because it made names collide.
    pub index_sort_disabled: bool,
}

/// Canonicalisation options.
#[derive(Debug, Clone, Copy)]
pub struct CanonOptions {
    /// Sort the index tuple inside `component(name)(i__j__k)`.
    pub sort_index: bool,
}

impl Default for CanonOptions {
    fn default() -> Self {
        Self { sort_index: true }
    }
}

/// Split `component(name)(idx)suffix`; `None` for names of any other shape.
fn split_name(name: &str) -> Option<(&str, &str, &str, &str)> {
    let open1 = name.find('(')?;
    let close1 = open1 + name[open1..].find(')')?;
    let rest = &name[close1 + 1..];
    let rest_open = rest.strip_prefix('(')?;
    let close2 = rest_open.find(')')?;
    Some((
        &name[..open1],
        &name[open1 + 1..close1],
        &rest_open[..close2],
        &rest_open[close2 + 1..],
    ))
}

fn rename(name: &str, sort_index: bool) -> String {
    match split_name(name) {
        Some((comp, n, idx, suffix)) => {
            let idx = if sort_index {
                let mut parts: Vec<&str> = idx.split("__").collect();
                parts.sort_unstable();
                parts.join("__")
            } else {
                idx.to_string()
            };
            format!("{comp}({n})({idx}){suffix}")
        }
        None => name.to_string(),
    }
}

/// Strip Pyomo row prefixes (`c_l_`, `c_u_`, `c_e_`, `r_l_`, `r_u_`) and the trailing `_`.
fn strip_row_name(name: &str) -> &str {
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

fn is_generic_name(n: &str) -> bool {
    n.len() > 1 && n.starts_with('c') && n[1..].bytes().all(|b| b.is_ascii_digit())
}

fn clean(x: f64) -> f64 {
    if x == 0.0 { 0.0 } else { x }
}

/// Canonicalise a parsed LP.
pub fn canonicalise(raw: &RawLp, opts: CanonOptions) -> Canonical {
    let mut sort_index = opts.sort_index;
    loop {
        match try_canonicalise(raw, sort_index) {
            Ok(mut c) => {
                c.index_sort_disabled = opts.sort_index && !sort_index;
                return c;
            }
            Err(()) if sort_index => sort_index = false,
            Err(()) => unreachable!("collisions cannot occur without sorting"),
        }
    }
}

fn try_canonicalise(raw: &RawLp, sort_index: bool) -> Result<Canonical, ()> {
    // Variable renaming (original -> canonical); collisions disable sorting.
    let mut all: BTreeSet<&str> = BTreeSet::new();
    all.extend(raw.bounds.keys().map(String::as_str));
    all.extend(raw.generals.iter().map(String::as_str));
    all.extend(raw.binaries.iter().map(String::as_str));
    all.extend(raw.obj_terms.iter().map(|t| t.0.as_str()));
    for r in &raw.rows {
        all.extend(r.terms.iter().map(|t| t.0.as_str()));
    }
    for s in &raw.sos {
        all.extend(s.members.iter().map(|t| t.0.as_str()));
    }
    let anonymous = !all.is_empty()
        && all
            .iter()
            .all(|n| is_generic_name(n) || *n == ONE_VAR_CONSTANT);
    let mut map: HashMap<&str, String> = HashMap::new();
    let mut seen: HashMap<String, &str> = HashMap::new();
    for &n in &all {
        let c = if anonymous {
            n.to_string()
        } else {
            rename(n, sort_index)
        };
        if let Some(prev) = seen.insert(c.clone(), n)
            && prev != n
        {
            return Err(());
        }
        map.insert(n, c);
    }

    // Fixed constant variable.
    let const_value = raw.bounds.get(ONE_VAR_CONSTANT).map_or(1.0, |b| b.0);

    let generals: BTreeSet<&str> = raw.generals.iter().map(String::as_str).collect();
    let binaries: BTreeSet<&str> = raw.binaries.iter().map(String::as_str).collect();

    // Merge terms: name -> coefficient, folding ONE_VAR_CONSTANT into a constant.
    let merge = |terms: &[(String, f64)]| -> (BTreeMap<String, f64>, f64) {
        let mut m: BTreeMap<String, f64> = BTreeMap::new();
        let mut constant = 0.0;
        for (n, c) in terms {
            if n == ONE_VAR_CONSTANT {
                constant += c * const_value;
            } else {
                *m.entry(map[n.as_str()].clone()).or_insert(0.0) += c;
            }
        }
        m.retain(|_, c| *c != 0.0);
        (m, constant)
    };

    let (obj_map, obj_extra) = merge(&raw.obj_terms);
    let mut rows_named: Vec<(String, f64, f64, BTreeMap<String, f64>)> = Vec::new();
    let mut used: BTreeSet<String> = obj_map.keys().cloned().collect();
    let mut name_count: HashMap<String, usize> = HashMap::new();
    for r in &raw.rows {
        let (m, k) = merge(&r.terms);
        used.extend(m.keys().cloned());
        let mut name = if anonymous {
            String::new()
        } else {
            rename(strip_row_name(&r.name), sort_index)
        };
        if !anonymous {
            let n = name_count.entry(name.clone()).or_insert(0);
            *n += 1;
            if *n > 1 {
                if sort_index {
                    return Err(());
                }
                name = format!("{name}#{n}");
            }
        }
        rows_named.push((name, clean(r.lo - k), clean(r.hi - k), m));
    }
    let mut sos_named = Vec::new();
    for s in &raw.sos {
        let members: Vec<(String, f64)> = s
            .members
            .iter()
            .map(|(n, w)| (map[n.as_str()].clone(), *w))
            .collect();
        used.extend(members.iter().map(|m| m.0.clone()));
        sos_named.push((rename(&s.name, sort_index), s.kind, members));
    }

    // Variables.
    let mut var_names: Vec<String> = used.into_iter().collect();
    var_names.sort();
    let index: HashMap<&str, usize> = var_names
        .iter()
        .enumerate()
        .map(|(i, n)| (n.as_str(), i))
        .collect();
    let orig_of: HashMap<&str, &str> = map.iter().map(|(o, c)| (c.as_str(), *o)).collect();
    let vars: Vec<CVar> = var_names
        .iter()
        .map(|n| {
            let o = orig_of[n.as_str()];
            let (mut lb, mut ub) = raw.bounds.get(o).copied().unwrap_or((0.0, f64::INFINITY));
            let mut kind = if binaries.contains(o) {
                Kind::Binary
            } else if generals.contains(o) {
                Kind::Integer
            } else {
                Kind::Continuous
            };
            if kind == Kind::Integer && lb == 0.0 && ub == 1.0 {
                kind = Kind::Binary;
            }
            if kind == Kind::Binary {
                (lb, ub) = (0.0, 1.0);
            }
            CVar {
                name: n.clone(),
                kind,
                lb: clean(lb),
                ub: clean(ub),
            }
        })
        .collect();

    let to_terms = |m: &BTreeMap<String, f64>| -> Vec<(usize, f64)> {
        let mut v: Vec<(usize, f64)> = m.iter().map(|(n, c)| (index[n.as_str()], *c)).collect();
        v.sort_by_key(|t| t.0);
        v
    };
    let mut rows: Vec<CRow> = rows_named
        .iter()
        .map(|(name, lo, hi, m)| CRow {
            name: name.clone(),
            lo: *lo,
            hi: *hi,
            terms: to_terms(m),
        })
        .collect();
    if !anonymous {
        rows.sort_by(|a, b| a.name.cmp(&b.name));
    }
    let mut sos: Vec<CSos> = sos_named
        .iter()
        .map(|(name, kind, members)| {
            let mut m: Vec<(usize, f64)> = members
                .iter()
                .map(|(n, w)| (index[n.as_str()], *w))
                .collect();
            m.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
            CSos {
                name: name.clone(),
                kind: *kind,
                members: m,
            }
        })
        .collect();
    sos.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(Canonical {
        sense: raw.sense,
        offset: clean(raw.obj_const + obj_extra),
        obj: to_terms(&obj_map),
        vars,
        rows,
        sos,
        anonymous,
        index_sort_disabled: false,
    })
}

fn num(x: f64) -> String {
    if x.is_infinite() {
        if x > 0.0 { "inf".into() } else { "-inf".into() }
    } else {
        format!("{x}")
    }
}

impl Canonical {
    fn expr(&self, terms: &[(usize, f64)]) -> String {
        let mut s = String::new();
        for (v, c) in terms {
            let _ = write!(
                s,
                "{}{} {} ",
                if *c >= 0.0 { "+" } else { "" },
                num(*c),
                self.vars[*v].name
            );
        }
        s.trim_end().to_string()
    }

    /// One-line description of a row, for diff messages.
    pub fn row_text(&self, r: &CRow) -> String {
        format!("{} <= {} <= {}", num(r.lo), self.expr(&r.terms), num(r.hi))
    }

    /// Write as CPLEX LP (Pyomo-like layout) using canonical names.
    ///
    /// Parsing and canonicalising the result gives back an equal [`Canonical`].
    pub fn to_lp(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{}",
            if self.sense == Sense::Min {
                "min"
            } else {
                "max"
            }
        );
        let _ = writeln!(s, "obj:");
        let mut obj = self.expr(&self.obj);
        let _ = write!(obj, " {:+e} {ONE_VAR_CONSTANT}", self.offset);
        let _ = writeln!(s, "{obj}\ns.t.");
        for (i, r) in self.rows.iter().enumerate() {
            // Row names may collide when anonymous, so number them.
            let e = self.expr(&r.terms);
            let label = if self.anonymous {
                format!("r{i}")
            } else {
                r.name.clone()
            };
            if r.lo == r.hi {
                let _ = writeln!(s, "c_e_{label}_:\n{e}\n= {:e}", r.lo);
            } else {
                let _ = writeln!(s, "r_l_{label}_:\n{} <= {e} <= {}", num(r.lo), num(r.hi));
            }
        }
        let _ = writeln!(s, "bounds\n 1 <= {ONE_VAR_CONSTANT} <= 1");
        for v in &self.vars {
            let _ = writeln!(s, " {} <= {} <= {}", num(v.lb), v.name, num(v.ub));
        }
        for (title, kind) in [("general", Kind::Integer), ("binary", Kind::Binary)] {
            let _ = writeln!(s, "{title}");
            for v in self.vars.iter().filter(|v| v.kind == kind) {
                let _ = writeln!(s, " {}", v.name);
            }
        }
        let _ = writeln!(s, "SOS");
        for o in &self.sos {
            let _ = writeln!(s, "{}: S{}::", o.name, o.kind);
            for (v, w) in &o.members {
                let _ = writeln!(s, " {}:{w:e}", self.vars[*v].name);
            }
        }
        let _ = writeln!(s, "end");
        s
    }

    /// Readable, deterministic text form (what `lp-diff --canon` prints).
    pub fn to_text(&self) -> String {
        let mut s = String::new();
        let _ = writeln!(
            s,
            "{}",
            if self.sense == Sense::Min {
                "min"
            } else {
                "max"
            }
        );
        let _ = writeln!(s, "  {} + {}", self.expr(&self.obj), num(self.offset));
        let _ = writeln!(s, "rows");
        for r in &self.rows {
            let _ = writeln!(s, "  {}: {}", r.name, self.row_text(r));
        }
        let _ = writeln!(s, "vars");
        for v in &self.vars {
            let _ = writeln!(
                s,
                "  {} in [{}, {}] {:?}",
                v.name,
                num(v.lb),
                num(v.ub),
                v.kind
            );
        }
        for o in &self.sos {
            let _ = writeln!(s, "sos{} {}: {}", o.kind, o.name, self.expr(&o.members));
        }
        s
    }
}
