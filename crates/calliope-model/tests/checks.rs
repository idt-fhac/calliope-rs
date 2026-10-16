//! Port of `tests/math/checks_test.py`: every case of `tests/math/math_checks_config.yaml`
//! raises (or warns with) its check's message when the model is built.
//!
//! Read from the Calliope reference checkout (`../calliope` or `CALLIOPE_REFERENCE`); skipped
//! without it.

use std::path::PathBuf;

use calliope_preprocess::backend::check_inputs;
use calliope_preprocess::{ReadOptions, read_yaml};
use calliope_schema::CheckErrors;
use calliope_yaml::{AttrTree, Node, Value};

fn reference() -> Option<PathBuf> {
    let root = std::env::var("CALLIOPE_REFERENCE").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        PathBuf::from,
    );
    root.join("src/calliope").is_dir().then_some(root)
}

#[test]
fn math_checks_raise_and_warn() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let config =
        calliope_yaml::read_yaml_file(root.join("tests/math/math_checks_config.yaml"), false)
            .unwrap();
    let model_path = root.join("tests/common/test_model/model.yaml");
    let mut n = 0;
    let mut failed = Vec::new();
    for (section, extra) in [
        ("base", None),
        ("milp", Some("milp")),
        ("operate", Some("operate")),
    ] {
        let Some(checks) = config.get(section).and_then(Node::as_map) else {
            continue;
        };
        for (check_name, payload) in checks.iter() {
            let cases: Vec<Node> = match &payload.value {
                Value::Seq(s) => s.clone(),
                _ => vec![payload.clone()],
            };
            for (i, case) in cases.iter().enumerate() {
                let case = case.as_map().expect("a case is a mapping");
                let mut override_dict: AttrTree = case
                    .get("override_dict")
                    .and_then(Node::as_map)
                    .cloned()
                    .unwrap_or_default();
                if let Some(m) = extra {
                    override_dict
                        .set_key(
                            "config.init.extra_math",
                            Node::new(Value::Seq(vec![Node::new(Value::Str(m.to_owned()))])),
                        )
                        .unwrap();
                }
                let opts = ReadOptions {
                    scenario: case.get("scenario").and_then(|s| match &s.value {
                        Value::Str(s) => Some(s.clone()),
                        _ => None,
                    }),
                    override_dict: Some(override_dict),
                    ..ReadOptions::default()
                };
                let id = format!("{section}::{check_name}[{i}]");
                n += 1;
                let model = match read_yaml(&model_path, &opts) {
                    Ok(m) => m,
                    Err(e) => {
                        failed.push(format!("{id}: preprocess failed: {e}"));
                        continue;
                    }
                };
                let check = &model.math.checks[check_name.as_str()];
                let outcome = check_inputs(&model);
                let ok = match (check.errors, &outcome) {
                    (CheckErrors::Raise, Err(e)) => e.to_string().contains(&check.message),
                    (CheckErrors::Warn, Ok(w)) => w.iter().any(|w| w.contains(&check.message)),
                    _ => false,
                };
                if !ok {
                    failed.push(format!("{id}: {outcome:?}"));
                }
            }
        }
    }
    assert!(n >= 20, "only {n} cases");
    assert!(
        failed.is_empty(),
        "{} of {n} failed:\n{}",
        failed.len(),
        failed.join("\n")
    );
}

/// `pre_validate_math_strings` accepts all shipped math, alone and combined.
#[test]
fn pre_validation_accepts_shipped_math() {
    use calliope_preprocess::model_math::{PRE_DEFINED_MATH, build_math};
    let init: indexmap::IndexMap<String, AttrTree> = PRE_DEFINED_MATH
        .iter()
        .map(|(k, v)| {
            (
                (*k).to_owned(),
                calliope_yaml::read_yaml_str(v, false).unwrap(),
            )
        })
        .collect();
    for extra in ["milp", "operate", "spores", "storage_inter_cluster"] {
        let math = build_math(&["base".to_owned(), extra.to_owned()], &init, None).unwrap();
        calliope_math::eval::pre_validate_math_strings(&math)
            .unwrap_or_else(|e| panic!("{extra}: {e}"));
    }
}
