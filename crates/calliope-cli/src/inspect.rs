//! `calliope inspect`: one math component of a built model — its definition, the expanded
//! equations (equation × sub-expression × slice combinations), the active index set (the
//! `foreach` combinations its `where` keeps) and, for the first index items, the LP rows,
//! columns or expressions they became.

use std::fmt::Write as _;

use calliope_array::{Coord, LArray};
use calliope_lp::LpModel;
use calliope_lp::writer::fmt_num;
use calliope_math::eval::Lin;
use calliope_math::eval::build::label_str;
use calliope_math::{Group, ParseMode, parse_math};
use calliope_model::{Backend, Model};
use calliope_schema::{AttrStr, CalliopeBuildMath};
use indexmap::IndexMap;

use crate::error::{CliError, CliResult};

/// What to show.
#[derive(Debug, Clone)]
pub struct InspectOptions {
    /// The component name.
    pub component: String,
    /// How many index items to show in full.
    pub max: usize,
}

struct Meta {
    group: Group,
    title: String,
    description: String,
    foreach: Vec<String>,
    where_: String,
}

fn position<V>(m: &IndexMap<AttrStr, V>, name: &str) -> Option<usize> {
    m.keys().position(|k| k.as_str() == name)
}

fn meta(math: &CalliopeBuildMath, name: &str) -> Option<Meta> {
    let list = |f: &[AttrStr]| f.iter().map(ToString::to_string).collect();
    if let Some(i) = position(&math.variables, name) {
        let c = &math.variables[i];
        return Some(Meta {
            group: Group::Variables,
            title: c.title.clone(),
            description: c.description.clone(),
            foreach: list(&c.foreach),
            where_: c.where_.clone(),
        });
    }
    if let Some(i) = position(&math.global_expressions, name) {
        let c = &math.global_expressions[i];
        return Some(Meta {
            group: Group::GlobalExpressions,
            title: c.title.clone(),
            description: c.description.clone(),
            foreach: list(&c.foreach),
            where_: c.where_.clone(),
        });
    }
    if let Some(i) = position(&math.constraints, name) {
        let c = &math.constraints[i];
        return Some(Meta {
            group: Group::Constraints,
            title: c.title.clone(),
            description: c.description.clone(),
            foreach: list(&c.foreach),
            where_: c.where_.clone(),
        });
    }
    if let Some(i) = position(&math.piecewise_constraints, name) {
        let c = &math.piecewise_constraints[i];
        return Some(Meta {
            group: Group::PiecewiseConstraints,
            title: c.title.clone(),
            description: c.description.clone(),
            foreach: list(&c.foreach),
            where_: c.where_.clone(),
        });
    }
    if let Some(i) = position(&math.objectives, name) {
        let c = &math.objectives[i];
        return Some(Meta {
            group: Group::Objectives,
            title: c.title.clone(),
            description: c.description.clone(),
            foreach: Vec::new(),
            where_: String::new(),
        });
    }
    None
}

/// A math string on one line (YAML block strings keep their line breaks).
fn one_line(s: &str) -> String {
    let joined = s.split_whitespace().collect::<Vec<_>>().join(" ");
    joined.replace("( ", "(").replace(" )", ")")
}

fn index_str(dims: &[Coord], codes: &[u32]) -> String {
    let items: Vec<String> = dims
        .iter()
        .zip(codes)
        .map(|(d, c)| label_str(d.label(*c)))
        .collect();
    format!("[{}]", items.join(", "))
}

fn col_name(lp: &LpModel, c: u32) -> String {
    lp.cols
        .get(c as usize)
        .map_or_else(|| format!("c{c}"), |col| col.label.to_string())
}

fn signed(v: f64) -> String {
    if v < 0.0 {
        format!("- {}", fmt_num(-v))
    } else {
        format!("+ {}", fmt_num(v))
    }
}

fn lin_str(lp: &LpModel, l: &Lin) -> String {
    let mut s = String::new();
    for &(c, a) in &l.terms {
        let _ = write!(s, "{} {} ", signed(a), col_name(lp, c));
    }
    if l.constant != 0.0 || l.terms.is_empty() {
        let _ = write!(s, "{} ", signed(l.constant));
    }
    s.trim_end().to_owned()
}

fn row_str(lp: &LpModel, r: u32) -> String {
    let r = r as usize;
    if r >= lp.num_rows() {
        return "(deleted)".to_owned();
    }
    let row = &lp.rows[r];
    let (cols, vals) = lp.row(r);
    let mut body = String::new();
    for (&c, &a) in cols.iter().zip(vals) {
        let _ = write!(
            body,
            "{} {} ",
            signed(a),
            col_name(lp, u32::try_from(c).unwrap_or(0))
        );
    }
    let body = body.trim_end();
    let (lb, ub) = (row.lb, row.ub);
    if lb == ub {
        format!("{body} == {}", fmt_num(lb))
    } else if lb.is_finite() && ub.is_finite() {
        format!("{} <= {body} <= {}", fmt_num(lb), fmt_num(ub))
    } else if lb.is_finite() {
        format!("{body} >= {}", fmt_num(lb))
    } else if ub.is_finite() {
        format!("{body} <= {}", fmt_num(ub))
    } else {
        format!("{body} (free)")
    }
}

/// The active cells of an array, as (index text, detail) pairs, and their count.
fn active<T: calliope_array::Element>(
    a: &LArray<T>,
    keep: impl Fn(&T) -> Option<String>,
    max: usize,
) -> (usize, Vec<(String, String)>) {
    let mut n = 0;
    let mut shown = Vec::new();
    for (codes, v) in a.cells() {
        if let Some(detail) = keep(v) {
            n += 1;
            if shown.len() < max {
                shown.push((index_str(a.dims(), &codes), detail));
            }
        }
    }
    (n, shown)
}

fn foreach_size(backend: &Backend, foreach: &[String]) -> usize {
    foreach
        .iter()
        .map(|d| backend.inputs().coords.get(d).map_or(1, Coord::len))
        .product()
}

/// The report for one component of a built model.
pub fn inspect(model: &Model, o: &InspectOptions) -> CliResult<String> {
    let backend = model
        .backend()
        .ok_or_else(|| CliError::new("The model has not been built."))?;
    let math: &CalliopeBuildMath = &model.math;
    let name = o.component.as_str();
    let m = meta(math, name).ok_or_else(|| {
        CliError::new(format!(
            "`{name}` is not an active variable, global expression, constraint, piecewise \
             constraint or objective of this model's math."
        ))
    })?;
    let mut out = String::new();
    let _ = writeln!(out, "{}.{name}", m.group.as_str());
    if !m.title.is_empty() {
        let _ = writeln!(out, "  title:       {}", m.title);
    }
    if !m.description.is_empty() {
        let _ = writeln!(out, "  description: {}", m.description);
    }
    let _ = writeln!(out, "  foreach:     [{}]", m.foreach.join(", "));
    if !m.where_.is_empty() {
        let _ = writeln!(out, "  where:       {}", m.where_);
    }

    // Expanded equations.
    let (parsed, _) = parse_math(math, ParseMode::Build);
    if let Some(c) = parsed.iter().find(|c| c.group == m.group && c.name == name) {
        if !c.equations.is_empty() {
            let _ = writeln!(out, "\nexpanded equations ({}):", c.equations.len());
        }
        for eq in &c.equations {
            let _ = writeln!(out, "  {}", eq.name);
            let wheres: Vec<String> = eq
                .wheres
                .iter()
                .map(|w| one_line(&w.source))
                .filter(|w| !w.is_empty() && w != "True")
                .collect();
            if !wheres.is_empty() {
                let _ = writeln!(out, "    where:      {}", wheres.join(" AND "));
            }
            let _ = writeln!(out, "    expression: {}", one_line(&eq.expression.source));
            for (k, v) in eq.sub_expressions.iter().chain(&eq.slices) {
                let _ = writeln!(out, "      ${k} = {}", one_line(&v.source));
            }
        }
        if let Some(p) = &c.piecewise {
            let _ = writeln!(
                out,
                "\npiecewise:\n  x: {} at {}\n  y: {} at {}",
                one_line(&p.x_expression.source),
                p.x_values,
                one_line(&p.y_expression.source),
                p.y_values
            );
        }
    }

    // Active index set and what each item became.
    let lp = backend.lp();
    let total = foreach_size(backend, &m.foreach);
    let (n, shown) = match m.group {
        Group::Variables => {
            let a = backend
                .expression(name)
                .ok_or_else(|| CliError::new(format!("`{name}` was not built.")))?;
            active(
                a,
                |l: &Lin| {
                    let &(c, _) = l.terms.first()?;
                    let col = lp.cols.get(c as usize)?;
                    let kind = if col.integer {
                        if col.lb == 0.0 && col.ub == 1.0 {
                            " (binary)"
                        } else {
                            " (integer)"
                        }
                    } else {
                        ""
                    };
                    Some(format!(
                        "{} <= {} <= {}{kind}",
                        fmt_num(col.lb),
                        col.label,
                        fmt_num(col.ub)
                    ))
                },
                o.max,
            )
        }
        Group::GlobalExpressions => {
            let a = backend
                .expression(name)
                .ok_or_else(|| CliError::new(format!("`{name}` was not built.")))?;
            active(a, |l: &Lin| Some(format!("= {}", lin_str(lp, l))), o.max)
        }
        Group::Constraints => {
            let a = backend
                .constraint_rows(name)
                .ok_or_else(|| CliError::new(format!("`{name}` was not built.")))?;
            active(a, |r: &Option<u32>| r.map(|r| row_str(lp, r)), o.max)
        }
        Group::PiecewiseConstraints => {
            let rows = lp.rows_of(calliope_lp::ComponentKind::Piecewise, name);
            let shown = rows
                .iter()
                .take(o.max)
                .map(|&r| {
                    (
                        lp.rows[r].label.to_string(),
                        row_str(lp, u32::try_from(r).unwrap_or(0)),
                    )
                })
                .collect();
            (rows.len(), shown)
        }
        Group::Objectives => {
            let (sense, lin) = backend
                .objective_expression(name)
                .ok_or_else(|| CliError::new(format!("`{name}` was not built.")))?;
            let active = if backend.objective() == name {
                " (active)"
            } else {
                ""
            };
            let _ = writeln!(
                out,
                "\nobjective: {sense:?}{active}, {} terms",
                lin.terms.len()
            );
            let mut l = lin.clone();
            l.terms.truncate(o.max);
            let more = if lin.terms.len() > o.max { " ..." } else { "" };
            let _ = writeln!(out, "  {}{more}", lin_str(lp, &l));
            return Ok(out);
        }
        Group::Postprocessed | Group::Checks => (0, Vec::new()),
    };
    let what = match m.group {
        Group::PiecewiseConstraints => "rows",
        _ => "index items",
    };
    if m.group == Group::PiecewiseConstraints {
        let _ = writeln!(out, "\nactive {what}: {n}");
    } else {
        let _ = writeln!(
            out,
            "\nactive index set: {n} of {total} foreach combinations"
        );
    }
    for (idx, detail) in &shown {
        let _ = writeln!(out, "  {idx}\n      {detail}");
    }
    if n > shown.len() {
        let _ = writeln!(out, "  ... {} more (--max)", n - shown.len());
    }
    Ok(out)
}
