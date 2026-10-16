//! Results parity: every golden example/test-model fixture is read, built and solved, and its
//! objective values (per operate timestep or SPORE) and termination condition compared with
//! Calliope's (relative 1e-6). Array-by-array comparison of `results.nc` is in
//! `xtask parity`.
//!
//! Needs the Calliope reference checkout (`../calliope` or `CALLIOPE_REFERENCE`).
//! `GOLDEN_FILTER=<substring>` limits the fixtures.

use std::time::Instant;

use calliope_model::{BuildOptions, Model, SolveOptions};

mod common;
use common::{fixture, fixture_dirs, read_meta, reference, solve_overrides};

/// Objective tolerance per fixture where the difference is classified (docs/findings.md,
/// `xtask/src/known.rs`): a zero objective (any feasible point is optimal), a degenerate
/// SPORE propagated by evolving-average scoring, and a MILP solved to a 5% gap.
fn tolerance(name: &str) -> f64 {
    match name {
        "national_scale_check_feasibility" => f64::INFINITY,
        "national_scale_spores_evolving_average" => 1e-4,
        "urban_scale_milp" => 0.05,
        _ => 1e-6,
    }
}

fn close(a: f64, b: f64, rel: f64) -> bool {
    a == b || (a - b).abs() <= rel * a.abs().max(b.abs()).max(1e-12)
}

#[test]
fn objectives_match_calliope() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let mut failed = Vec::new();
    for dir in fixture_dirs(false) {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        let tmp = tempfile::tempdir().unwrap();
        let meta = read_meta(&dir);
        let Some((path, opts, _)) = fixture(&root, &dir, tmp.path()).unwrap() else {
            println!("skip  {name} (expected preprocessing error)");
            continue;
        };
        let t = Instant::now();
        let outcome = (|| -> Result<Model, String> {
            let mut m = Model::from_yaml(&path, &opts).map_err(|e| e.to_string())?;
            m.build(&BuildOptions::default())
                .map_err(|e| e.to_string())?;
            let mut overrides = solve_overrides(&meta);
            overrides
                .set_key(
                    "solver_options.output_flag",
                    calliope_yaml::Node::new(calliope_yaml::Value::Bool(false)),
                )
                .unwrap();
            m.solve(&SolveOptions {
                overrides,
                ..SolveOptions::default()
            })
            .map_err(|e| e.to_string())?;
            Ok(m)
        })();
        let secs = t.elapsed().as_secs_f64();
        let m = match outcome {
            Ok(m) => m,
            Err(e) => {
                println!("FAIL  {name}: {}", e.chars().take(400).collect::<String>());
                failed.push(name);
                continue;
            }
        };
        let mut diffs = Vec::new();
        let want_term = meta["termination_condition"].as_str().unwrap_or("");
        let got_term = m.runtime.termination_condition.clone().unwrap_or_default();
        if want_term != got_term {
            diffs.push(format!("termination {want_term:?} vs {got_term:?}"));
        }
        if let Some(objs) = meta["objectives"].as_object() {
            for (k, v) in objs {
                let want: Vec<f64> = match v {
                    serde_json::Value::Array(a) => {
                        a.iter().filter_map(serde_json::Value::as_f64).collect()
                    }
                    other => other.as_f64().into_iter().collect(),
                };
                let Some(got) = m.results.get(k) else {
                    diffs.push(format!("{k}: missing"));
                    continue;
                };
                let got = got.to_dense();
                if got.len() != want.len() {
                    diffs.push(format!("{k}: {} values vs {}", got.len(), want.len()));
                    continue;
                }
                if let Some(i) = (0..got.len()).find(|&i| !close(want[i], got[i], tolerance(&name)))
                {
                    diffs.push(format!("{k}[{i}]: {} vs {}", want[i], got[i]));
                }
            }
        }
        if diffs.is_empty() {
            println!("ok    {name} ({secs:.2} s)");
        } else {
            println!("DIFF  {name} ({secs:.2} s): {}", diffs.join("; "));
            failed.push(name);
        }
    }
    assert!(failed.is_empty(), "{} differ: {failed:?}", failed.len());
}
