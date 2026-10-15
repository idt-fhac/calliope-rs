//! The typecheck on all built-in math, the math gallery and the example-model math.
//!
//! Every diagnostic on this math has been reviewed (findings M1-M9 of Phase 4a); this test pins them, so
//! a new diagnostic (a false positive or a new finding) fails until it is reviewed.

mod common;

use calliope_math::typecheck;

fn report() -> Vec<String> {
    let mut out = Vec::new();
    for (name, files) in common::math_sets() {
        let math = common::layered(&files).unwrap();
        let report = typecheck(&math, None);
        for d in &report.diagnostics {
            let sev = if d.is_error() { "error" } else { "warning" };
            out.push(format!("{name}: {sev} {}: {}", d.code, d.short()));
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn builtin_math_diagnostics() {
    // No error or warning on any built-in, gallery or example-model math when the input
    // dimensions are unknown.
    let got = report();
    assert!(got.is_empty(), "{}", got.join("\n"));
}

#[derive(serde::Deserialize)]
struct ModelInputs {
    mode: String,
    extra_math: Vec<String>,
    dims: std::collections::BTreeMap<String, Vec<String>>,
}

fn math_file(name: &str) -> String {
    match name {
        "additional_math" => "src/calliope/example_models/urban_scale/additional_math.yaml".into(),
        other => format!("src/calliope/math/{other}.yaml"),
    }
}

fn model_report() -> Vec<String> {
    let models: std::collections::BTreeMap<String, ModelInputs> =
        serde_json::from_str(include_str!("fixtures/input_dims.json")).unwrap();
    let mut out = Vec::new();
    for (name, m) in &models {
        let mut files = vec![math_file("base")];
        if m.mode != "base" {
            files.push(math_file(&m.mode));
        }
        files.extend(m.extra_math.iter().map(|e| math_file(e)));
        let Some(math) = common::layered(&files) else {
            return Vec::new();
        };
        let schema = calliope_math::InputSchema {
            dims: m.dims.clone(),
        };
        for d in &typecheck(&math, Some(&schema)).diagnostics {
            let sev = if d.is_error() { "error" } else { "warning" };
            out.push(format!("{name}: {sev} {}: {}", d.code, d.short()));
        }
    }
    out
}

#[test]
fn example_model_diagnostics() {
    let got = model_report();
    if got.is_empty() && common::reference().is_none() {
        return;
    }
    // `total_curtailment` divides by `sum(source_use_max * $source_scaler, over=timesteps)`;
    // in these models `source_use_max` has no `timesteps` dimension, so the sum is a no-op
    // (finding M1: a likely upstream bug).
    let expected: Vec<String> = ["milp", "urban_scale"]
        .iter()
        .map(|m| {
            format!(
                "{m}: warning math::helper::sum-missing-dimension: postprocessed.total_curtailment.equations[0].expression: The array does not have dimension `timesteps`; `sum` ignores it [`timesteps`]"
            )
        })
        .collect();
    assert_eq!(got, expected);
}
