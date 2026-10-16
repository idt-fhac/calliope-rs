//! Evidence that a result-array difference is an alternative optimum.
//!
//! For each solve of a fixture (one in base mode, one per SPORE), the problem Calliope solved
//! is rebuilt with calliope-rs: the same model, and in SPORES mode the scores
//! (`spores_score_cumulative`) and cost budget (`spores_baseline_cost_tracked`) recorded in
//! Calliope's results for that SPORE. calliope-rs solves it, then Calliope's solution (its
//! decision-variable values from `results.nc`) is put into calliope-rs's LP:
//!
//! - every row and column bound must hold for it, within [`FEAS_TOL`] (scaled by
//!   `max(1, |bound|)`);
//! - its objective value must equal calliope-rs's optimum within [`OPT_TOL`] (relative).
//!
//! Then Calliope's point is feasible and optimal in the port's LP, so the two runs differ only
//! in which optimal point the solver returned.

use calliope_array::{LArray, Label};
use calliope_math::eval::Value;
use calliope_model::{Model, Results};

/// Largest allowed row or bound violation, relative to `max(1, |bound|)`.
pub const FEAS_TOL: f64 = 1e-6;
/// Largest allowed relative difference between Calliope's objective value in the port's LP
/// and the port's optimum.
pub const OPT_TOL: f64 = 1e-6;

/// The check for one solve.
#[derive(Debug, Clone)]
pub struct SolveCheck {
    /// SPORE number (`None` in base mode).
    pub spore: Option<i64>,
    /// Objective of the solve.
    pub objective: String,
    /// calliope-rs's optimum of the rebuilt problem.
    pub rust_optimum: f64,
    /// Calliope's solution, evaluated with calliope-rs's objective.
    pub python_in_rust: f64,
    /// Calliope's reported objective value.
    pub python_reported: f64,
    /// Largest scaled row violation of Calliope's solution.
    pub row_violation: f64,
    /// Largest scaled column-bound violation of Calliope's solution.
    pub col_violation: f64,
    /// LP columns without a value in Calliope's results.
    pub missing: usize,
    /// Number of LP columns.
    pub cols: usize,
    /// Largest difference between the scores calliope-rs's own run used and Calliope's
    /// (SPORES only; `None` when not compared).
    pub score_diff: Option<f64>,
}

impl SolveCheck {
    /// Relative gap between Calliope's point and the port's optimum.
    pub fn gap(&self) -> f64 {
        (self.python_in_rust - self.rust_optimum).abs() / self.rust_optimum.abs().max(1.0)
    }

    /// Calliope's point is feasible and optimal in the port's LP.
    pub fn ok(&self) -> bool {
        self.missing == 0
            && self.row_violation <= FEAS_TOL
            && self.col_violation <= FEAS_TOL
            && self.gap() <= OPT_TOL
    }

    /// One report line.
    pub fn line(&self) -> String {
        let which = self
            .spore
            .map_or_else(|| "solve".to_owned(), |s| format!("SPORE {s}"));
        let scores = match self.score_diff {
            Some(0.0) => "; the port's run used the same scores".to_owned(),
            Some(d) => format!("; the port's run used other scores (max diff {d:.3e})"),
            None => String::new(),
        };
        format!(
            "{} {which} ({}): Calliope's point in the port's LP: max row violation {:.1e}, bound violation {:.1e}, {} of {} columns without value; objective {:.10e} vs port optimum {:.10e} (rel gap {:.1e}; Calliope reported {:.10e}){scores}",
            if self.ok() { "ok" } else { "NOT OPTIMAL" },
            self.objective,
            self.row_violation,
            self.col_violation,
            self.missing,
            self.cols,
            self.python_in_rust,
            self.rust_optimum,
            self.gap(),
            self.python_reported,
        )
    }
}

fn err(e: impl ToString) -> String {
    e.to_string()
}

/// `a.sel(dim=label)` if `a` has `dim`.
fn at(a: &LArray<f64>, dim: &str, label: Option<&Label>) -> Result<LArray<f64>, String> {
    match label {
        Some(l) if a.coord(dim).is_some() => a.sel_one(dim, l).map_err(err),
        _ => Ok(a.clone()),
    }
}

/// Value of `src` at the labels of cell `codes` of `like` (NaN if `src` lacks a label).
fn value_at(src: &LArray<f64>, like: &LArray<f64>, codes: &[u32]) -> f64 {
    let mut out = Vec::with_capacity(src.dims().len());
    for d in src.dims() {
        let Ok(ax) = like.axis(d.name()) else {
            return f64::NAN;
        };
        let label = like.dims()[ax].label(codes[ax]);
        let Some(c) = d.code(label) else {
            return f64::NAN;
        };
        out.push(c);
    }
    if out.len() == like.dims().len() {
        src.get(&out)
    } else {
        f64::NAN
    }
}

/// Largest absolute difference between two score arrays over cells where either is defined
/// (`inf` where only one is).
fn score_diff(a: &LArray<f64>, b: &LArray<f64>) -> f64 {
    let mut worst: f64 = 0.0;
    for (codes, &x) in a.cells() {
        let y = value_at(b, a, &codes);
        let d = match (x.is_nan(), y.is_nan()) {
            (true, true) => 0.0,
            (false, false) => (x - y).abs(),
            _ => f64::INFINITY,
        };
        worst = worst.max(d);
    }
    worst
}

/// Check every solve of `model` (read, not built) against Calliope's `golden` results;
/// `rust` are the port's own results of the fixture (for comparing SPORES scores).
pub fn check(
    model: &mut Model,
    golden: &Results,
    rust: &Results,
) -> Result<Vec<SolveCheck>, String> {
    model
        .build(&calliope_model::BuildOptions::default())
        .map_err(err)?;
    let solve = model.config.solve.clone();
    let base_objective = model.config.build.objective.clone();
    let variables: Vec<String> = model
        .math
        .variables
        .keys()
        .map(ToString::to_string)
        .collect();
    let spores: Vec<Option<Label>> = golden
        .arrays
        .values()
        .find_map(|a| a.coord("spores").map(|c| c.labels().to_vec()))
        .map_or_else(|| vec![None], |l| l.into_iter().map(Some).collect());
    let mut out = Vec::new();
    for spore in spores {
        let backend = model.backend_mut().expect("built");
        let objective = match &spore {
            None => backend.objective().to_owned(),
            Some(l) => {
                let get = |name: &str| -> Result<LArray<f64>, String> {
                    let a = golden
                        .get(name)
                        .ok_or_else(|| format!("golden results lack `{name}`"))?;
                    at(a, "spores", Some(l))
                };
                backend
                    .update_input("spores_score", &Value::Num(get("spores_score_cumulative")?))
                    .map_err(err)?;
                backend
                    .update_input(
                        "spores_baseline_cost",
                        &Value::Num(get("spores_baseline_cost_tracked")?),
                    )
                    .map_err(err)?;
                let o = if matches!(l, Label::Int(0)) {
                    base_objective.clone()
                } else {
                    "min_spores".to_owned()
                };
                backend.set_objective(&o).map_err(err)?;
                o
            }
        };
        let results = backend.solve(&solve, false).map_err(err)?;
        if results.termination_condition != "optimal" {
            return Err(format!(
                "the rebuilt problem ends {:?}",
                results.termination_condition
            ));
        }
        let lp = backend.lp();
        let x_rust = backend
            .solver()
            .last_outcome()
            .and_then(|o| o.col_value.clone())
            .ok_or("no solution")?;
        let mut x = vec![f64::NAN; lp.num_cols()];
        for name in &variables {
            let Ok(cols) = backend.variable_columns(name) else {
                continue;
            };
            let py = golden
                .get(name)
                .map(|a| at(a, "spores", spore.as_ref()))
                .transpose()?;
            for (codes, &c) in cols.cells() {
                if c.is_nan() {
                    continue;
                }
                #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
                let c = c as usize;
                x[c] = py.as_ref().map_or(f64::NAN, |p| value_at(p, &cols, &codes));
            }
        }
        let missing = x.iter().filter(|v| v.is_nan()).count();
        let filled: Vec<f64> = x
            .iter()
            .map(|v| if v.is_nan() { 0.0 } else { *v })
            .collect();
        let scaled = |v: f64, bound: f64| v / bound.abs().max(1.0);
        let mut row_violation: f64 = 0.0;
        for (row, act) in lp.rows.iter().zip(lp.row_activity(&filled)) {
            if act < row.lb {
                row_violation = row_violation.max(scaled(row.lb - act, row.lb));
            }
            if act > row.ub {
                row_violation = row_violation.max(scaled(act - row.ub, row.ub));
            }
        }
        let mut col_violation: f64 = 0.0;
        for (col, &v) in lp.cols.iter().zip(&filled) {
            if v < col.lb {
                col_violation = col_violation.max(scaled(col.lb - v, col.lb));
            }
            if v > col.ub {
                col_violation = col_violation.max(scaled(v - col.ub, col.ub));
            }
        }
        let python_reported = golden
            .get(&objective)
            .map(|a| at(a, "spores", spore.as_ref()))
            .transpose()?
            .map_or(f64::NAN, |a| a.get(&[]));
        let score = match (&spore, rust.get("spores_score_cumulative")) {
            (Some(l), Some(r)) => {
                let g = at(
                    golden
                        .get("spores_score_cumulative")
                        .expect("checked above"),
                    "spores",
                    Some(l),
                )?;
                match at(r, "spores", Some(l)) {
                    Ok(r) => Some(score_diff(&g, &r).max(score_diff(&r, &g))),
                    Err(_) => Some(f64::INFINITY),
                }
            }
            _ => None,
        };
        out.push(SolveCheck {
            spore: spore.as_ref().and_then(|l| match l {
                Label::Int(i) => Some(*i),
                _ => None,
            }),
            objective,
            rust_optimum: lp.objective.expr.eval(&x_rust),
            python_in_rust: lp.objective.expr.eval(&filled),
            python_reported,
            row_violation,
            col_violation,
            missing,
            cols: lp.num_cols(),
            score_diff: score,
        });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::{RunOutput, Runner, load_manifest};
    use crate::runner::RustRunner;

    #[test]
    fn accepts_calliopes_point_and_rejects_a_worse_one() {
        let reference = crate::reference();
        if !reference.join("src/calliope").exists() {
            eprintln!("skipping: no Calliope checkout");
            return;
        }
        let f = load_manifest(&crate::workspace_root().join("golden"))
            .unwrap()
            .into_iter()
            .find(|f| f.name == "test_model__simple_supply__two_hours")
            .unwrap();
        let runner = RustRunner { reference };
        let (ok, lines) = runner
            .verify_alternative_optimum(&f, &RunOutput::default())
            .unwrap();
        assert!(ok, "{lines:?}");

        // Calliope's point with more capacity: not optimal (or beyond a bound).
        let mut golden = Model::from_netcdf(&f.file("results.nc")).unwrap().results;
        let cap = golden.arrays.get_mut("flow_cap").unwrap();
        *cap = cap.map(|v| v * 2.0 + 1.0);
        let mut model = Model::from_netcdf(&f.file("inputs.nc")).unwrap();
        let checks = check(&mut model, &golden, &Results::default()).unwrap();
        assert!(checks.iter().any(|c| !c.ok()), "{:?}", checks);
    }
}
