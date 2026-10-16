//! Phase 4 exit check: the problem the port builds equals Calliope's LP for every golden
//! fixture, after canonicalisation (`calliope_lp::lp_format`).
//!
//! - `golden/math_lp/*` (the 122 `tests/math` fixtures): compared by name with `pyomo.lp`.
//! - `golden/*` (37 example/test-model fixtures): compared by name with `pyomo.lp`, or by
//!   structure with `highs.lp` where there is no Pyomo file. Fixtures whose preprocessing
//!   is expected to fail have no LP and are skipped.
//!
//! The models are read from the Calliope reference checkout (`../calliope`, or
//! `CALLIOPE_REFERENCE`); without it the test is skipped. `GOLDEN_FILTER=<substring>` limits
//! the fixtures; `GOLDEN_VERBOSE=1` prints all differences; `GOLDEN_WRITE=<dir>` writes the
//! port's LP files there.

use std::path::{Path, PathBuf};
use std::time::Instant;

use calliope_lp::lp_format::{CanonOptions, DiffOptions, canonicalise, diff_canonical, parse_lp};
use calliope_lp::{LpNames, write_lp_string};
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

fn json_tree(v: &serde_json::Value) -> AttrTree {
    let mut t = AttrTree::new();
    if let serde_json::Value::Object(m) = v {
        for (k, v) in m {
            t.set_key(k, json_node(v)).expect("valid key");
        }
    }
    t
}

fn read_meta(dir: &Path) -> serde_json::Value {
    // Python's json writes bare `Infinity`/`NaN`.
    let text = std::fs::read_to_string(dir.join("meta.json"))
        .unwrap()
        .replace("-Infinity", "\"__-inf__\"")
        .replace("Infinity", "\"__inf__\"")
        .replace("NaN", "\"__nan__\"");
    serde_json::from_str(&text).unwrap()
}

/// Sets `math_paths.base` (where present) to `path`.
fn redirect_base_math(tree: &mut AttrTree, path: &Path) {
    let p = Node::new(Value::Str(path.display().to_string()));
    for key in ["math_paths.base", "config.init.math_paths.base"] {
        if tree.get_key_opt(key).is_some() {
            tree.set_key(key, p.clone()).unwrap();
        }
    }
}

/// The model, read options and objective of a fixture; `None` if it is expected to fail.
fn fixture(
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

/// Pyomo adds `c_e_ONE_VAR_CONSTANT: ONE_VAR_CONSTANT = 1` when no row has variables
/// (a CBC workaround), which is not part of the problem.
fn drop_dummy_row(mut raw: calliope_lp::lp_format::RawLp) -> calliope_lp::lp_format::RawLp {
    raw.rows.retain(|r| r.name != "c_e_ONE_VAR_CONSTANT");
    raw
}

struct Outcome {
    differences: Vec<String>,
    build_s: f64,
}

fn run(root: &Path, dir: &Path) -> Result<Option<Outcome>, String> {
    let tmp = tempfile::tempdir().unwrap();
    let Some((model, opts, objective)) = fixture(root, dir, tmp.path())? else {
        return Ok(None);
    };
    let meta = read_meta(dir);
    // `pyomo.lp` of the example fixtures is written after the solve: in operate mode that
    // is the last window, so the first window (what `build` makes) is in `highs.lp`. It is
    // missing where Pyomo's `to_lp` failed (bench/BUGS.md #14).
    let pyomo_ok = meta["errors"].get("pyomo_lp").is_none()
        && !(meta.get("fixture").is_some()
            && meta["fixture"]["name"]
                .as_str()
                .is_some_and(|n| n.contains("operate")));
    let reference = ["pyomo.lp", "highs.lp"]
        .iter()
        .filter(|f| pyomo_ok || **f != "pyomo.lp")
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
        .ok_or("no LP file")?;
    let m = read_yaml(&model, &opts).map_err(|e| format!("preprocess: {e}"))?;
    let mut build = m.config.build.clone();
    if let Some(o) = objective {
        build.objective = o;
    }
    let t = Instant::now();
    let built = calliope_model::build_with(&m, &build).map_err(|e| format!("build: {e}"))?;
    let build_s = t.elapsed().as_secs_f64();
    let ours = write_lp_string(&built.lp, LpNames::Verbose);
    if let Ok(out) = std::env::var("GOLDEN_WRITE") {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(Path::new(&out).join(format!("{name}.lp")), &ours).unwrap();
    }
    let theirs = std::fs::read_to_string(&reference).unwrap();
    let a = canonicalise(
        &drop_dummy_row(parse_lp(&theirs).map_err(|e| e.0)?),
        CanonOptions::default(),
    );
    let b = canonicalise(&parse_lp(&ours).map_err(|e| e.0)?, CanonOptions::default());
    let d = diff_canonical(&a, &b, &DiffOptions::default());
    Ok(Some(Outcome {
        differences: d.differences,
        build_s,
    }))
}

fn fixture_dirs() -> Vec<PathBuf> {
    let filter = std::env::var("GOLDEN_FILTER").unwrap_or_default();
    let g = repo().join("golden");
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&g)
        .unwrap()
        .chain(std::fs::read_dir(g.join("math_lp")).unwrap())
        .map(|e| e.unwrap().path())
        .filter(|p| p.join("meta.json").is_file())
        .filter(|p| p.to_string_lossy().contains(&filter))
        .collect();
    dirs.sort();
    dirs
}

#[test]
fn built_lp_matches_calliope() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let verbose = std::env::var("GOLDEN_VERBOSE").is_ok();
    let (mut passed, mut skipped) = (0, 0);
    let mut failed = Vec::new();
    for dir in fixture_dirs() {
        let name = dir
            .strip_prefix(repo().join("golden"))
            .unwrap()
            .display()
            .to_string();
        match run(&root, &dir) {
            Ok(None) => skipped += 1,
            Ok(Some(o)) if o.differences.is_empty() => {
                passed += 1;
                println!("ok    {name} ({:.3} s)", o.build_s);
            }
            Ok(Some(o)) => {
                println!("DIFF  {name}: {} differences", o.differences.len());
                for d in o
                    .differences
                    .iter()
                    .take(if verbose { usize::MAX } else { 6 })
                {
                    let d: String = d.chars().take(300).collect();
                    println!("        {d}");
                }
                failed.push(name);
            }
            Err(e) => {
                let e: String = e.chars().take(600).collect();
                println!("FAIL  {name}: {e}");
                failed.push(name);
            }
        }
    }
    println!(
        "{passed} equal, {} differ, {skipped} skipped (expected errors)",
        failed.len()
    );
    assert!(failed.is_empty(), "{} differ: {failed:?}", failed.len());
}
