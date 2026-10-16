//! Shared helpers of the golden-fixture tests: reading `golden/*/meta.json` into the read
//! and solve options Calliope used.

#![allow(dead_code)]

use std::path::{Path, PathBuf};

use calliope_preprocess::ReadOptions;
use calliope_yaml::{AttrTree, Node, Value};

pub fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

pub fn reference() -> Option<PathBuf> {
    let root = std::env::var("CALLIOPE_REFERENCE")
        .map_or_else(|_| repo().join("../calliope"), PathBuf::from);
    root.join("src/calliope").is_dir().then_some(root)
}

pub fn json_node(v: &serde_json::Value) -> Node {
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

pub fn json_tree(v: &serde_json::Value) -> AttrTree {
    let mut t = AttrTree::new();
    if let serde_json::Value::Object(m) = v {
        for (k, v) in m {
            t.set_key(k, json_node(v)).expect("valid key");
        }
    }
    t
}

pub fn read_meta(dir: &Path) -> serde_json::Value {
    // Python's json writes bare `Infinity`/`NaN`.
    let text = std::fs::read_to_string(dir.join("meta.json"))
        .unwrap()
        .replace("-Infinity", "\"__-inf__\"")
        .replace("Infinity", "\"__inf__\"")
        .replace("NaN", "\"__nan__\"");
    serde_json::from_str(&text).unwrap()
}

/// Sets `math_paths.base` (where present) to `path`.
pub fn redirect_base_math(tree: &mut AttrTree, path: &Path) {
    let p = Node::new(Value::Str(path.display().to_string()));
    for key in ["math_paths.base", "config.init.math_paths.base"] {
        if tree.get_key_opt(key).is_some() {
            tree.set_key(key, p.clone()).unwrap();
        }
    }
}

/// The model, read options and objective of a fixture; `None` if it is expected to fail.
pub fn fixture(
    root: &Path,
    dir: &Path,
    tmp: &Path,
) -> Result<Option<(PathBuf, ReadOptions, Option<String>)>, String> {
    let meta = read_meta(dir);
    if let Some(fx) = meta.get("fixture") {
        if meta.get("expected_error").is_some_and(|e| !e.is_null()) {
            return Ok(None);
        }
        let tree = |k: &str| fx.get(k).filter(|v| v.is_object()).map(json_tree);
        let opts = ReadOptions {
            scenario: fx
                .get("scenario")
                .and_then(|s| s.as_str())
                .map(str::to_owned),
            override_dict: tree("override_dict"),
            math_dict: tree("math_dict"),
            init_kwargs: tree("init_kwargs").unwrap_or_default(),
        };
        return Ok(Some((root.join(fx["model"].as_str().unwrap()), opts, None)));
    }
    // tests/math fixture: barebones base math (from the attrs) plus custom math.
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
    let base_path = tmp.join("barebones.yaml");
    std::fs::write(&base_path, calliope_yaml::to_yaml(base)).unwrap();
    let mut opts = ReadOptions {
        scenario: meta["scenario"].as_str().map(str::to_owned),
        override_dict: meta
            .get("override_dict")
            .filter(|v| v.is_object())
            .map(json_tree),
        math_dict: None,
        init_kwargs: json_tree(&meta["kwargs"]),
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
    Ok(Some((
        root.join("tests/common/test_model/model.yaml"),
        opts,
        meta["objective"].as_str().map(str::to_owned),
    )))
}

/// `solve_kwargs` of an example fixture as a `config.solve` override tree.
pub fn solve_overrides(meta: &serde_json::Value) -> AttrTree {
    meta.get("fixture")
        .and_then(|f| f.get("solve_kwargs"))
        .filter(|v| v.is_object())
        .map(json_tree)
        .unwrap_or_default()
}

/// Golden fixture directories (`golden/*`, and `golden/math_lp/*` with `math_lp`), filtered by
/// `GOLDEN_FILTER`.
pub fn fixture_dirs(math_lp: bool) -> Vec<PathBuf> {
    let filter = std::env::var("GOLDEN_FILTER").unwrap_or_default();
    let g = repo().join("golden");
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&g)
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    if math_lp {
        entries.extend(
            std::fs::read_dir(g.join("math_lp"))
                .unwrap()
                .map(|e| e.unwrap().path()),
        );
    }
    let mut dirs: Vec<PathBuf> = entries
        .into_iter()
        .filter(|p| p.join("meta.json").is_file())
        .filter(|p| p.to_string_lossy().contains(&filter))
        .collect();
    dirs.sort();
    dirs
}
