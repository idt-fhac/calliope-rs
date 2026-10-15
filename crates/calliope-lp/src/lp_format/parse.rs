//! Tokenising parser for CPLEX LP files as written by Pyomo and HiGHS.

use std::collections::HashMap;
use std::fmt;

/// Parse error with a short description.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LpError(pub String);

impl fmt::Display for LpError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "LP parse error: {}", self.0)
    }
}

impl std::error::Error for LpError {}

fn err<T>(msg: impl Into<String>) -> Result<T, LpError> {
    Err(LpError(msg.into()))
}

/// Optimisation direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sense {
    /// Minimise.
    Min,
    /// Maximise.
    Max,
}

/// One constraint row as written: `lo <= sum(coef * var) <= hi`.
#[derive(Debug, Clone, PartialEq)]
pub struct RawRow {
    /// Row label.
    pub name: String,
    /// Lower activity bound (`-inf` when absent).
    pub lo: f64,
    /// Upper activity bound (`inf` when absent).
    pub hi: f64,
    /// Terms in file order (may repeat a variable).
    pub terms: Vec<(String, f64)>,
}

/// An SOS set.
#[derive(Debug, Clone, PartialEq)]
pub struct RawSos {
    /// Set label.
    pub name: String,
    /// 1 or 2.
    pub kind: u8,
    /// Members with weights.
    pub members: Vec<(String, f64)>,
}

/// Contents of an LP file before canonicalisation.
#[derive(Debug, Clone, PartialEq)]
pub struct RawLp {
    /// Objective direction.
    pub sense: Sense,
    /// Objective terms.
    pub obj_terms: Vec<(String, f64)>,
    /// Constant in the objective.
    pub obj_const: f64,
    /// Constraint rows.
    pub rows: Vec<RawRow>,
    /// Explicit bounds `(lb, ub)`; variables absent have `(0, inf)`.
    pub bounds: HashMap<String, (f64, f64)>,
    /// Names in the `general` section.
    pub generals: Vec<String>,
    /// Names in the `binary` section.
    pub binaries: Vec<String>,
    /// SOS sets.
    pub sos: Vec<RawSos>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Section {
    Objective,
    Constraints,
    Bounds,
    General,
    Binary,
    Semi,
    Sos,
    End,
}

fn is_op(t: &str) -> bool {
    matches!(t, "<=" | ">=" | "=" | "=<" | "=>" | "<" | ">")
}

fn op_kind(t: &str) -> char {
    match t {
        "<=" | "=<" | "<" => '<',
        ">=" | "=>" | ">" => '>',
        _ => '=',
    }
}

/// Parse a number token; `None` if the token is not numeric.
fn number(t: &str) -> Option<f64> {
    let body = t.trim_start_matches(['+', '-']);
    let first = body.chars().next()?;
    let lower = body.to_ascii_lowercase();
    let is_inf = lower == "inf" || lower == "infinity";
    if !(first.is_ascii_digit() || first == '.' || is_inf) {
        return None;
    }
    let v = if is_inf {
        f64::INFINITY
    } else {
        body.parse::<f64>().ok()?
    };
    Some(if t.starts_with('-') { -v } else { v })
}

fn section_of(tokens: &[String], i: usize) -> Option<(Section, usize)> {
    let t = tokens[i].to_ascii_lowercase();
    let next = tokens.get(i + 1).map(|s| s.to_ascii_lowercase());
    match t.as_str() {
        "min" | "minimize" | "minimise" | "max" | "maximize" | "maximise" => {
            Some((Section::Objective, 1))
        }
        "st" | "s.t." | "st." => Some((Section::Constraints, 1)),
        "subject" if next.as_deref() == Some("to") => Some((Section::Constraints, 2)),
        "such" if next.as_deref() == Some("that") => Some((Section::Constraints, 2)),
        "bounds" | "bound" => Some((Section::Bounds, 1)),
        "general" | "generals" | "gen" => Some((Section::General, 1)),
        "binary" | "binaries" | "bin" => Some((Section::Binary, 1)),
        "semi" | "semis" | "semi-continuous" => Some((Section::Semi, 1)),
        "sos" | "sos1" | "sos2" => Some((Section::Sos, 1)),
        "end" => Some((Section::End, 1)),
        _ => None,
    }
}

/// Parse an expression segment into terms and a constant.
fn parse_expr(tokens: &[String]) -> Result<(Vec<(String, f64)>, f64), LpError> {
    let mut terms = Vec::new();
    let mut constant = 0.0;
    let mut sign = 1.0;
    let mut pending: Option<f64> = None;
    for t in tokens {
        if t == "+" || t == "-" {
            if let Some(p) = pending.take() {
                constant += p;
            }
            sign = if t == "-" { -1.0 } else { 1.0 };
        } else if let Some(n) = number(t) {
            if let Some(p) = pending.take() {
                constant += p;
            }
            pending = Some(sign * n);
            sign = 1.0;
        } else {
            terms.push((t.clone(), pending.take().unwrap_or(sign)));
            sign = 1.0;
        }
    }
    if let Some(p) = pending {
        constant += p;
    }
    Ok((terms, constant))
}

fn parse_row(name: &str, tokens: &[String]) -> Result<RawRow, LpError> {
    let mut segs: Vec<&[String]> = Vec::new();
    let mut ops: Vec<char> = Vec::new();
    let mut start = 0;
    for (i, t) in tokens.iter().enumerate() {
        if is_op(t) {
            segs.push(&tokens[start..i]);
            ops.push(op_kind(t));
            start = i + 1;
        }
    }
    segs.push(&tokens[start..]);
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    let terms = match ops.len() {
        1 => {
            let (lt, lc) = parse_expr(segs[0])?;
            let (rt, rc) = parse_expr(segs[1])?;
            let (expr, c, rhs, op) = match (lt.is_empty(), rt.is_empty()) {
                (false, true) => (lt, lc, rc, ops[0]),
                (true, false) => {
                    let flipped = match ops[0] {
                        '<' => '>',
                        '>' => '<',
                        o => o,
                    };
                    (rt, rc, lc, flipped)
                }
                (true, true) => (lt, lc, rc, ops[0]),
                (false, false) => return err(format!("row {name}: variables on both sides")),
            };
            let r = rhs - c;
            match op {
                '<' => hi = r,
                '>' => lo = r,
                _ => {
                    lo = r;
                    hi = r;
                }
            }
            expr
        }
        2 => {
            let (t0, c0) = parse_expr(segs[0])?;
            let (t1, c1) = parse_expr(segs[1])?;
            let (t2, c2) = parse_expr(segs[2])?;
            if !t0.is_empty() || !t2.is_empty() || ops[0] != ops[1] {
                return err(format!("row {name}: unsupported range form"));
            }
            if ops[0] == '>' {
                hi = c0 - c1;
                lo = c2 - c1;
            } else {
                lo = c0 - c1;
                hi = c2 - c1;
            }
            t1
        }
        _ => return err(format!("row {name}: expected 1 or 2 comparison operators")),
    };
    Ok(RawRow {
        name: name.to_string(),
        lo,
        hi,
        terms,
    })
}

fn parse_bounds(
    tokens: &[String],
    bounds: &mut HashMap<String, (f64, f64)>,
) -> Result<(), LpError> {
    let default = (0.0, f64::INFINITY);
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Some(n) = number(t) {
            // `num op name [op num]`
            let (Some(op), Some(name)) = (tokens.get(i + 1), tokens.get(i + 2)) else {
                return err("truncated bound");
            };
            let e = bounds.entry(name.clone()).or_insert(default);
            match op_kind(op) {
                '<' => e.0 = n,
                '>' => e.1 = n,
                _ => *e = (n, n),
            }
            i += 3;
            if let (Some(op2), Some(n2)) =
                (tokens.get(i), tokens.get(i + 1).and_then(|s| number(s)))
                && is_op(op2)
            {
                match op_kind(op2) {
                    '<' => e.1 = n2,
                    '>' => e.0 = n2,
                    _ => *e = (n2, n2),
                }
                i += 2;
            }
        } else if tokens
            .get(i + 1)
            .is_some_and(|s| s.eq_ignore_ascii_case("free"))
        {
            bounds.insert(t.clone(), (f64::NEG_INFINITY, f64::INFINITY));
            i += 2;
        } else {
            let (Some(op), Some(n)) =
                (tokens.get(i + 1), tokens.get(i + 2).and_then(|s| number(s)))
            else {
                return err(format!("cannot parse bound at {t}"));
            };
            let e = bounds.entry(t.clone()).or_insert(default);
            match op_kind(op) {
                '<' => e.1 = n,
                '>' => e.0 = n,
                _ => *e = (n, n),
            }
            i += 3;
        }
    }
    Ok(())
}

fn parse_sos(tokens: &[String]) -> Result<Vec<RawSos>, LpError> {
    let mut sets: Vec<RawSos> = Vec::new();
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        let kind = tokens
            .get(i + 1)
            .map(|s| s.to_ascii_lowercase())
            .filter(|s| s == "s1::" || s == "s2::");
        if let (Some(label), Some(k)) = (t.strip_suffix(':'), kind) {
            sets.push(RawSos {
                name: label.to_string(),
                kind: if k == "s1::" { 1 } else { 2 },
                members: Vec::new(),
            });
            i += 2;
        } else if let Some((v, w)) = t.rsplit_once(':') {
            let Some(set) = sets.last_mut() else {
                return err("SOS member before set header");
            };
            let w = number(w).ok_or_else(|| LpError(format!("bad SOS weight in {t}")))?;
            set.members.push((v.to_string(), w));
            i += 1;
        } else {
            return err(format!("unexpected SOS token {t}"));
        }
    }
    Ok(sets)
}

/// Parse an LP file.
pub fn parse_lp(text: &str) -> Result<RawLp, LpError> {
    let mut tokens: Vec<String> = Vec::new();
    for line in text.lines() {
        let line = line.split('\\').next().unwrap_or("");
        tokens.extend(line.split_whitespace().map(str::to_string));
    }
    let mut lp = RawLp {
        sense: Sense::Min,
        obj_terms: Vec::new(),
        obj_const: 0.0,
        rows: Vec::new(),
        bounds: HashMap::new(),
        generals: Vec::new(),
        binaries: Vec::new(),
        sos: Vec::new(),
    };
    // Split into (section, tokens).
    let mut chunks: Vec<(Section, Vec<String>)> = Vec::new();
    let mut i = 0;
    let mut cur: Option<usize> = None;
    while i < tokens.len() {
        if let Some((sec, n)) = section_of(&tokens, i) {
            if sec == Section::Objective {
                lp.sense = if tokens[i].to_ascii_lowercase().starts_with("max") {
                    Sense::Max
                } else {
                    Sense::Min
                };
            }
            chunks.push((sec, Vec::new()));
            cur = Some(chunks.len() - 1);
            i += n;
            if sec == Section::End {
                break;
            }
        } else if let Some(c) = cur {
            chunks[c].1.push(tokens[i].clone());
            i += 1;
        } else {
            return err(format!("token {:?} before objective section", tokens[i]));
        }
    }
    if chunks.first().map(|c| c.0) != Some(Section::Objective) {
        return err("missing objective section");
    }
    for (sec, toks) in &chunks {
        match sec {
            Section::Objective => {
                let body = match toks.first() {
                    Some(t) if t.ends_with(':') => &toks[1..],
                    _ => &toks[..],
                };
                let (t, c) = parse_expr(body)?;
                lp.obj_terms = t;
                lp.obj_const = c;
            }
            Section::Constraints => {
                let mut label: Option<&str> = None;
                let mut start = 0;
                for j in 0..=toks.len() {
                    let is_label = j < toks.len() && toks[j].ends_with(':');
                    if is_label || j == toks.len() {
                        if let Some(l) = label {
                            lp.rows.push(parse_row(l, &toks[start..j])?);
                        } else if j > 0 {
                            return err("constraint without label");
                        }
                        if is_label {
                            label = Some(toks[j].trim_end_matches(':'));
                            start = j + 1;
                        }
                    }
                }
            }
            Section::Bounds => parse_bounds(toks, &mut lp.bounds)?,
            Section::General => lp.generals.extend(toks.iter().cloned()),
            Section::Binary => lp.binaries.extend(toks.iter().cloned()),
            Section::Semi => {
                if !toks.is_empty() {
                    return err("semi-continuous variables are not supported");
                }
            }
            Section::Sos => lp.sos.extend(parse_sos(toks)?),
            Section::End => {}
        }
    }
    Ok(lp)
}
