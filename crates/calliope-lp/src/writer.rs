//! CPLEX LP writer for [`LpModel`] in Pyomo's flavour (names `variables(x)(i__j)`,
//! row prefixes `c_l_`/`c_u_`/`c_e_`, objective constant on `ONE_VAR_CONSTANT`), so that
//! [`crate::lp_format::canonicalise`] yields the same canonical form as Calliope's `to_lp`.

use crate::lp_format::ONE_VAR_CONSTANT;
use crate::model::{LpModel, ObjSense};
use std::fmt::Write as _;
use std::io;

/// Name style for the LP file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LpNames {
    /// Pyomo-style labelled names (Calliope's `verbose_strings`).
    #[default]
    Verbose,
    /// Anonymous `c<N>` / `r<N>` names like the HiGHS writer.
    Anonymous,
}

/// Format a number the way LP readers accept it (shortest round-trip repr).
pub fn fmt_num(v: f64) -> String {
    if v == f64::INFINITY {
        "+inf".into()
    } else if v == f64::NEG_INFINITY {
        "-inf".into()
    } else if v == 0.0 || (1e-4..1e15).contains(&v.abs()) {
        format!("{v}")
    } else {
        format!("{v:e}")
    }
}

fn signed(v: f64) -> String {
    let s = fmt_num(v);
    if s.starts_with('-') || s.starts_with('+') {
        s
    } else {
        format!("+{s}")
    }
}

/// Render `model` as CPLEX LP text.
pub fn write_lp_string(model: &LpModel, names: LpNames) -> String {
    let col_name = |j: usize| match names {
        LpNames::Verbose => model.cols[j].label.to_string(),
        LpNames::Anonymous => format!("c{j}"),
    };
    let col_names: Vec<String> = (0..model.num_cols()).map(col_name).collect();
    let mut s = String::with_capacity(64 * (model.num_nz() + model.num_cols()));
    s.push_str("\\* Source calliope-rs *\\\n\n");
    s.push_str(match model.objective.sense {
        ObjSense::Minimize => "min \n",
        ObjSense::Maximize => "max \n",
    });
    let obj_name = match names {
        LpNames::Verbose => model.objective.label.to_string(),
        LpNames::Anonymous => "obj".into(),
    };
    let _ = writeln!(s, "{obj_name}:");
    let obj = model.objective.expr.merged();
    for &(j, a) in &obj {
        let _ = writeln!(s, "{} {}", signed(a), col_names[j]);
    }
    let constant = model.objective.expr.constant;
    if constant != 0.0 || obj.is_empty() {
        let _ = writeln!(s, "{} {ONE_VAR_CONSTANT}", signed(constant));
    }
    s.push_str("\ns.t.\n\n");
    for (r, row) in model.rows.iter().enumerate() {
        let base = match names {
            LpNames::Verbose => row.label.to_string(),
            LpNames::Anonymous => format!("r{r}"),
        };
        let (cols, vals) = model.row(r);
        let body = |s: &mut String| {
            for (&j, &a) in cols.iter().zip(vals) {
                let _ = writeln!(s, "{} {}", signed(a), col_names[j]);
            }
            if cols.is_empty() {
                let _ = writeln!(s, "+0 {ONE_VAR_CONSTANT}");
            }
        };
        let (lb, ub) = (row.lb, row.ub);
        let mut one = |prefix: &str, op: &str, rhs: f64| {
            let _ = writeln!(s, "{prefix}{base}_:");
            body(&mut s);
            let _ = writeln!(s, "{op} {}\n", fmt_num(rhs));
        };
        if lb == ub {
            one("c_e_", "=", lb);
        } else if lb.is_finite() && ub.is_finite() {
            one("r_l_", ">=", lb);
            one("r_u_", "<=", ub);
        } else if lb.is_finite() {
            one("c_l_", ">=", lb);
        } else if ub.is_finite() {
            one("c_u_", "<=", ub);
        }
        // free rows (both bounds infinite) are omitted, as in Pyomo
    }
    s.push_str("bounds\n");
    let _ = writeln!(s, "   1 <= {ONE_VAR_CONSTANT} <= 1");
    let mut generals = Vec::new();
    let mut binaries = Vec::new();
    for (j, c) in model.cols.iter().enumerate() {
        if c.integer && c.lb == 0.0 && c.ub == 1.0 {
            binaries.push(j);
            continue;
        }
        if c.integer {
            generals.push(j);
        }
        let _ = writeln!(
            s,
            "   {} <= {} <= {}",
            fmt_num(c.lb),
            col_names[j],
            fmt_num(c.ub)
        );
    }
    for (title, list) in [("general", &generals), ("binary", &binaries)] {
        if !list.is_empty() {
            let _ = writeln!(s, "{title}");
            for &j in list {
                let _ = writeln!(s, "  {}", col_names[j]);
            }
        }
    }
    if !model.sos2.is_empty() {
        s.push_str("SOS\n\n");
        for (k, set) in model.sos2.iter().enumerate() {
            let name = match names {
                LpNames::Verbose => set.label.to_string(),
                LpNames::Anonymous => format!("s{k}"),
            };
            let _ = writeln!(s, "{name}_s: S2::");
            for &(j, w) in &set.members {
                let _ = writeln!(s, "  {}:{}", col_names[j], fmt_num(w));
            }
            s.push('\n');
        }
    }
    s.push_str("end\n");
    s
}

/// Write `model` as a CPLEX LP file.
pub fn write_lp(model: &LpModel, path: &std::path::Path, names: LpNames) -> io::Result<()> {
    std::fs::write(path, write_lp_string(model, names))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lp_format::{DiffOptions, diff_lp};
    use crate::model::{ComponentKind, Label, LinExpr, RowSense};

    const PYOMO: &str = "\\* Source Pyomo model name=None *\\\n\nmin\nobjectives(obj)(0):\n+2.0 ONE_VAR_CONSTANT\n+3 variables(x)(b__a)\n\ns.t.\n\nc_l_constraints(r)(b__a)_:\n+1 variables(x)(b__a)\n+1 variables(y)(a)\n>= 5.0\n\nc_e_constraints(e)(a)_:\n+1 variables(y)(a)\n= 1.0\n\nbounds\n   1 <= ONE_VAR_CONSTANT <= 1\n   0.0 <= variables(x)(b__a) <= 10\n   -inf <= variables(y)(a) <= +inf\ngeneral\n  variables(x)(b__a)\nend\n";

    fn small() -> LpModel {
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
    fn written_lp_matches_pyomo_canonically() {
        let text = write_lp_string(&small(), LpNames::Verbose);
        let d = diff_lp(PYOMO, &text, &DiffOptions::default()).unwrap();
        assert!(d.is_equal(), "{:?}\n{text}", d.differences);
        let anon = write_lp_string(&small(), LpNames::Anonymous);
        let d = diff_lp(PYOMO, &anon, &DiffOptions::default()).unwrap();
        assert!(d.is_equal(), "{:?}\n{anon}", d.differences);
    }

    #[test]
    fn numbers() {
        assert_eq!(fmt_num(5.0), "5");
        assert_eq!(fmt_num(1e-12), "1e-12");
        assert_eq!(fmt_num(0.25), "0.25");
        assert_eq!(signed(-2.5), "-2.5");
    }
}
