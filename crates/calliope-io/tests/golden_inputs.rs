//! Phase 3 exit check: the preprocessed input dataset matches `golden/<fixture>/inputs.nc`
//! (written by Python Calliope 0.7.0) for every fixture: same variables, coordinates (label
//! order), dimension order, dtypes, and values: bit for bit for the fixtures (CSV floats
//! are parsed like pandas' `precise_xstrtod`), within 1e-12 for the math LP models (one
//! `source_unit: per_cap` scaling differs in the last bit).
//!
//! The example and test models are read from the Calliope reference checkout
//! (`../calliope`, or `CALLIOPE_REFERENCE`); without it the test is skipped.
//! `GOLDEN_FILTER=<substring>` limits the fixtures; `GOLDEN_VERBOSE=1` prints all diffs.

use std::path::{Path, PathBuf};

use calliope_io::{compare_datasets, read_group, write_model_netcdf};
use calliope_preprocess::{ReadOptions, read_yaml};
use calliope_yaml::{AttrTree, Node, Value};

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn reference() -> Option<PathBuf> {
    let root = std::env::var("CALLIOPE_REFERENCE")
        .map_or_else(|_| repo().join("../calliope"), PathBuf::from);
    root.join("src/calliope").is_dir().then_some(root)
}

fn json_node(v: &serde_json::Value) -> Node {
    Node::new(match v {
        serde_json::Value::Null => Value::Null,
        serde_json::Value::Bool(b) => Value::Bool(*b),
        serde_json::Value::Number(n) => match n.as_i64() {
            Some(i) => Value::Int(i),
            None => Value::Float(n.as_f64().unwrap_or(f64::NAN)),
        },
        serde_json::Value::String(s) => match s.as_str() {
            "__inf__" => Value::Float(f64::INFINITY),
            "__-inf__" => Value::Float(f64::NEG_INFINITY),
            "__nan__" => Value::Float(f64::NAN),
            _ => Value::Str(s.clone()),
        },
        serde_json::Value::Array(a) => Value::Seq(a.iter().map(json_node).collect()),
        serde_json::Value::Object(_) => Value::Map(json_tree(v)),
    })
}

/// `AttrDict(dict)`: dotted keys become nested keys.
pub fn json_tree(v: &serde_json::Value) -> AttrTree {
    let mut t = AttrTree::new();
    if let serde_json::Value::Object(m) = v {
        for (k, v) in m {
            t.set_key(k, json_node(v)).expect("valid key");
        }
    }
    t
}

fn options(fx: &serde_json::Value) -> ReadOptions {
    let tree = |k: &str| fx.get(k).filter(|v| v.is_object()).map(json_tree);
    ReadOptions {
        scenario: fx
            .get("scenario")
            .and_then(|s| s.as_str())
            .map(str::to_owned),
        override_dict: tree("override_dict"),
        math_dict: tree("math_dict"),
        init_kwargs: tree("init_kwargs").unwrap_or_default(),
        data_table_dfs: Default::default(),
    }
}

fn verbose() -> bool {
    std::env::var("GOLDEN_VERBOSE").is_ok()
}

fn run_fixture(root: &Path, dir: &Path) -> Result<Vec<String>, String> {
    // Python's json writes bare `Infinity`/`NaN`.
    let text = std::fs::read_to_string(dir.join("meta.json"))
        .unwrap()
        .replace("-Infinity", "\"__-inf__\"")
        .replace("Infinity", "\"__inf__\"")
        .replace("NaN", "\"__nan__\"");
    let meta: serde_json::Value = serde_json::from_str(&text).unwrap();
    let fx = &meta["fixture"];
    let model = root.join(fx["model"].as_str().unwrap());
    let opts = options(fx);
    let expected_error = meta.get("expected_error").filter(|e| !e.is_null());
    let result = read_yaml(&model, &opts);
    match (result, expected_error) {
        (Err(_), Some(_)) => Ok(Vec::new()),
        (Ok(_), Some(e)) => Err(format!("expected an error: {}", e["type"])),
        (Err(e), None) => Err(format!("error: {e}")),
        (Ok(m), None) => {
            let expected =
                read_group(&dir.join("inputs.nc"), "inputs").map_err(|e| e.to_string())?;
            // Round trip through our own NetCDF writer, so file dtypes are compared.
            let tmp = tempfile::tempdir().unwrap();
            let path = tmp.path().join("inputs.nc");
            write_model_netcdf(&path, &m.inputs, &[]).map_err(|e| e.to_string())?;
            let actual = read_group(&path, "inputs").map_err(|e| e.to_string())?;
            if verbose() {
                println!(
                    "      {} vars / {} coords expected, {} / {} actual",
                    expected.vars.len(),
                    expected.coords.len(),
                    actual.vars.len(),
                    actual.coords.len()
                );
                let e: Vec<&String> = expected.vars.keys().collect();
                let a: Vec<&String> = actual.vars.keys().collect();
                if e != a {
                    println!("      variable order differs:\n        {e:?}\n        {a:?}");
                }
            }
            Ok(compare_datasets(&expected, &actual, 0.0)
                .iter()
                .map(ToString::to_string)
                .collect())
        }
    }
}

#[test]
fn golden_inputs_match() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let filter = std::env::var("GOLDEN_FILTER").unwrap_or_default();
    let verbose = verbose();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(repo().join("golden"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("meta.json").is_file() && p.join("../MANIFEST.json").is_file())
        .filter(|p| p.to_string_lossy().contains(&filter))
        .collect();
    dirs.sort();
    let mut failed = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        match run_fixture(&root, dir) {
            Ok(diffs) if diffs.is_empty() => println!("ok    {name}"),
            Ok(diffs) => {
                println!("DIFF  {name}: {} differences", diffs.len());
                for d in diffs.iter().take(if verbose { usize::MAX } else { 8 }) {
                    println!("        {d}");
                }
                failed.push(name);
            }
            Err(e) => {
                println!("FAIL  {name}: {e}");
                failed.push(name);
            }
        }
    }
    assert!(
        failed.is_empty(),
        "{} of {} fixtures differ: {failed:?}",
        failed.len(),
        dirs.len()
    );
}

/// The comparison catches differences: two different golden datasets, and a perturbed one.
#[test]
fn comparison_detects_differences() {
    let g = repo().join("golden");
    let a = g.join("national_scale/inputs.nc");
    let b = g.join("national_scale_time_resampling/inputs.nc");
    if !a.is_file() || !b.is_file() {
        return;
    }
    let a = read_group(&a, "inputs").unwrap();
    let b = read_group(&b, "inputs").unwrap();
    assert!(compare_datasets(&a, &a, 1e-12).is_empty());
    assert!(!compare_datasets(&a, &b, 1e-12).is_empty());
    let mut c = a.clone();
    if let calliope_preprocess::xr::Data::Float(v) = &mut c.vars["flow_cap_max"].data {
        let i = v.iter().position(|x| !x.is_nan()).unwrap();
        v[i] *= 1.0 + 1e-9;
    }
    let diffs = compare_datasets(&a, &c, 1e-12);
    assert_eq!(diffs.len(), 1, "{diffs:?}");
    let mut d = a.clone();
    d.vars["active"].data =
        calliope_preprocess::xr::Data::Float(vec![1.0; d.vars["active"].data.len()]);
    assert!(!compare_datasets(&a, &d, 1e-12).is_empty());
}

/// Sets `math_paths.base` (where present) to `path`, at the top level and under
/// `config.init`.
fn redirect_base_math(tree: &mut AttrTree, path: &Path) {
    let p = Node::new(Value::Str(path.display().to_string()));
    for key in ["math_paths.base", "config.init.math_paths.base"] {
        if tree.get_key_opt(key).is_some() {
            tree.set_key(key, p.clone()).unwrap();
        }
    }
}

fn run_math_lp_fixture(root: &Path, dir: &Path) -> Result<Vec<String>, String> {
    let meta: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(dir.join("meta.json"))
            .unwrap()
            .replace("-Infinity", "\"__-inf__\"")
            .replace("Infinity", "\"__inf__\"")
            .replace("NaN", "\"__nan__\""),
    )
    .unwrap();
    // The barebones base math lived in a pytest tmp dir; its content is in the attrs.
    let golden = dir.join("inputs.nc");
    let attrs = calliope_io::read_group_attrs(&golden, "attrs").map_err(|e| e.to_string())?;
    let math_yaml = &attrs
        .iter()
        .find(|(k, _)| k == "math")
        .ok_or("no math attr")?
        .1;
    let math = calliope_yaml::read_yaml_str(math_yaml, false).map_err(|e| e.to_string())?;
    let base = math
        .get_key_opt("init.base")
        .and_then(Node::as_map)
        .ok_or("no init.base")?;
    let tmp = tempfile::tempdir().unwrap();
    let base_path = tmp.path().join("barebones.yaml");
    std::fs::write(&base_path, calliope_yaml::to_yaml(base)).unwrap();

    let mut opts = ReadOptions {
        scenario: meta["scenario"].as_str().map(str::to_owned),
        override_dict: meta
            .get("override_dict")
            .filter(|v| v.is_object())
            .map(json_tree),
        math_dict: None,
        init_kwargs: json_tree(&meta["kwargs"]),
        data_table_dfs: Default::default(),
    };
    if let Some(o) = &mut opts.override_dict {
        redirect_base_math(o, &base_path);
    }
    redirect_base_math(&mut opts.init_kwargs, &base_path);
    if let Some(md) = meta.get("math_dict").filter(|v| v.is_object()) {
        let mut m = AttrTree::new();
        m.insert("custom_math", Node::from(json_tree(md)));
        opts.math_dict = Some(m);
        let mut extra: Vec<Node> = opts
            .init_kwargs
            .get("extra_math")
            .and_then(Node::as_seq)
            .map(<[Node]>::to_vec)
            .unwrap_or_default();
        extra.push(Node::new(Value::Str("custom_math".to_owned())));
        opts.init_kwargs
            .insert("extra_math", Node::new(Value::Seq(extra)));
    }
    let model = root.join("tests/common/test_model/model.yaml");
    let m = read_yaml(&model, &opts).map_err(|e| format!("error: {e}"))?;
    let mut expected = read_group(&golden, "inputs").map_err(|e| e.to_string())?;
    // These goldens were saved after `Model.build()`, whose `_drop_undefined` call adds
    // `__active` to `model.inputs` in place (a Python side effect, not an input).
    expected.remove("__active");
    let path = tmp.path().join("inputs.nc");
    write_model_netcdf(&path, &m.inputs, &[]).map_err(|e| e.to_string())?;
    let actual = read_group(&path, "inputs").map_err(|e| e.to_string())?;
    // meta.json is written with sorted keys, which loses the order of `add_dims` (and so
    // the dimension order of the table's arrays). Only for such overrides, a pure
    // dimension-order difference is accepted.
    let add_dims_in_override = serde_json::to_string(&meta["override_dict"])
        .unwrap()
        .contains("add_dims");
    Ok(compare_datasets(&expected, &actual, 1e-12)
        .into_iter()
        .filter(|d| match d {
            calliope_io::Diff::Dims {
                expected, actual, ..
            } if add_dims_in_override => {
                let mut e = expected.clone();
                let mut a = actual.clone();
                e.sort();
                a.sort();
                e != a
            }
            _ => true,
        })
        .map(|d| d.to_string())
        .collect())
}

/// The 122 `tests/math` fixtures (barebones math + custom math per component).
#[test]
fn golden_math_lp_inputs_match() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let base = repo().join("golden/math_lp");
    if !base.is_dir() {
        return;
    }
    let filter = std::env::var("GOLDEN_FILTER").unwrap_or_default();
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&base)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("inputs.nc").is_file())
        .filter(|p| p.to_string_lossy().contains(&filter))
        .collect();
    dirs.sort();
    let mut failed = Vec::new();
    for dir in &dirs {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        match run_math_lp_fixture(&root, dir) {
            Ok(d) if d.is_empty() => {}
            Ok(d) => {
                println!("DIFF  {name}: {} differences", d.len());
                for x in d.iter().take(6) {
                    println!("        {x}");
                }
                failed.push(name);
            }
            Err(e) => {
                println!("FAIL  {name}: {e}");
                failed.push(name);
            }
        }
    }
    println!(
        "{} of {} math_lp fixtures match",
        dirs.len() - failed.len(),
        dirs.len()
    );
    assert!(
        failed.is_empty(),
        "{} of {} differ: {failed:?}",
        failed.len(),
        dirs.len()
    );
}
