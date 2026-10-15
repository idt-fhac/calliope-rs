//! Independent check of the HiGHS backend: each golden LP (as written by Calliope right
//! after `build`) → `LpModel` → HiGHS must reproduce the golden objective and termination.

use calliope_highs::{HighsBackend, OptionValue, SolveConfig};
use calliope_lp::LpModel;
use calliope_lp::lp_format::parse_lp;
use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

fn options(meta: &Value) -> Vec<(String, OptionValue)> {
    let mut out = vec![("output_flag".to_string(), OptionValue::Bool(false))];
    if let Some(o) = meta["fixture"]["solve_kwargs"]["solver_options"].as_object() {
        for (k, v) in o {
            let v = match v {
                Value::Bool(b) => OptionValue::Bool(*b),
                Value::Number(n) if n.is_i64() => OptionValue::Int(n.as_i64().unwrap()),
                Value::Number(n) => OptionValue::Double(n.as_f64().unwrap()),
                Value::String(s) => OptionValue::Str(s.clone()),
                other => panic!("option {k}: {other}"),
            };
            out.push((k.clone(), v));
        }
    }
    out
}

/// The objective of the LP as built: the results objective (first entry for operate/SPORES
/// runs), else the value HiGHS reported (e.g. `check_feasibility`, which is not in results).
fn expected(meta: &Value) -> Option<f64> {
    let name = meta["objective_name"].as_str()?;
    match &meta["objectives"][name] {
        Value::Number(n) => n.as_f64(),
        Value::Array(a) => a.first()?.as_f64(),
        _ => meta["solver_objective_last"].as_f64(),
    }
}

#[test]
fn golden_lp_objectives() {
    let mut checked = Vec::new();
    for e in fs::read_dir(golden()).unwrap() {
        let dir = e.unwrap().path();
        let meta_path = dir.join("meta.json");
        if dir.file_name().unwrap() == "math_lp" || !meta_path.exists() {
            continue;
        }
        let text = fs::read_to_string(&meta_path)
            .unwrap()
            .replace("NaN", "null")
            .replace("-Infinity", "null")
            .replace("Infinity", "null");
        let meta: Value = serde_json::from_str(&text).unwrap();
        let lp = ["highs.lp", "pyomo.lp"]
            .iter()
            .map(|f| dir.join(f))
            .find(|p| p.exists() && fs::metadata(p).unwrap().len() > 100);
        let Some(lp) = lp else { continue };
        let term = meta["termination_condition"].as_str().unwrap_or("");
        let model = LpModel::from_raw(&parse_lp(&fs::read_to_string(&lp).unwrap()).unwrap());
        let mut be = HighsBackend::new(model);
        let cfg = SolveConfig {
            solver_options: options(&meta),
            ..SolveConfig::default()
        };
        let out = be.solve(&cfg).unwrap().clone();
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(out.termination_condition, term, "{name}");
        if let Some(want) = expected(&meta).filter(|_| term == "optimal") {
            let got = out.objective.unwrap();
            let rel = (got - want).abs() / want.abs().max(1.0);
            // MILPs solved to a relative gap may stop at a different incumbent.
            let gap = meta["fixture"]["solve_kwargs"]["solver_options"]["mip_rel_gap"]
                .as_f64()
                .unwrap_or(0.0);
            assert!(
                rel <= 1e-6_f64.max(gap),
                "{name}: objective {got} != golden {want} (rel {rel:e})"
            );
        }
        eprintln!("{name}: {} {:?}", out.termination_condition, out.objective);
        checked.push(name);
    }
    assert!(checked.len() >= 30, "checked only {checked:?}");
}
