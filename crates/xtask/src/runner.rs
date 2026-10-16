//! The calliope-rs [`Runner`]: reads a fixture's `meta.json` (the arguments Calliope was
//! called with), then preprocesses, builds, solves and writes `inputs.nc`, `model.lp` and
//! `results.nc` with `calliope_model::Model`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use calliope_model::{BuildOptions, Model, SolveOptions};
use calliope_preprocess::ReadOptions;
use calliope_yaml::{AttrTree, Node, Value};

use crate::parity::{Fixture, RunError, RunOutput, Runner};

/// Runs calliope-rs in-process.
#[derive(Debug, Clone)]
pub struct RustRunner {
    /// The Calliope checkout the fixture models are read from.
    pub reference: PathBuf,
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
            // Keys are dotted paths, as Calliope's override dicts allow.
            let _ = t.set_key(k, json_node(v));
        }
    }
    t
}

/// `meta.json` with Python's bare `Infinity`/`NaN` made parseable.
pub fn read_meta(dir: &Path) -> Result<serde_json::Value, String> {
    let text = std::fs::read_to_string(dir.join("meta.json"))
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .replace("-Infinity", "\"__-inf__\"")
        .replace("Infinity", "\"__inf__\"")
        .replace("NaN", "\"__nan__\"");
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

fn failed(e: impl ToString) -> RunError {
    RunError::Failed(e.to_string())
}

impl Runner for RustRunner {
    fn name(&self) -> &str {
        "calliope-rs"
    }

    fn run(&self, f: &Fixture, work: &Path) -> Result<RunOutput, RunError> {
        let meta = read_meta(&f.dir).map_err(failed)?;
        let fx = &meta["fixture"];
        let tree = |k: &str| fx.get(k).filter(|v| v.is_object()).map(json_tree);
        let opts = ReadOptions {
            scenario: fx["scenario"].as_str().map(str::to_owned),
            override_dict: tree("override_dict"),
            math_dict: tree("math_dict"),
            init_kwargs: tree("init_kwargs").unwrap_or_default(),
            data_table_dfs: Default::default(),
        };
        let model_path = self.reference.join(
            fx["model"]
                .as_str()
                .ok_or_else(|| failed("meta.json: no model"))?,
        );
        let out_dir = work.join(&f.name);
        std::fs::create_dir_all(&out_dir).map_err(failed)?;
        let mut model = Model::from_yaml(&model_path, &opts).map_err(failed)?;
        if let Some(seed) = fx["numpy_seed"].as_u64() {
            model.set_random_seed(u32::try_from(seed).map_err(failed)?);
        }
        let inputs_nc = out_dir.join("inputs.nc");
        model.to_netcdf(&inputs_nc).map_err(failed)?;
        model.build(&BuildOptions::default()).map_err(failed)?;
        let backend = model.backend_mut().expect("built");
        backend.verbose_strings();
        let lp = out_dir.join("model.lp");
        backend.to_lp(&lp).map_err(failed)?;
        let mut overrides = tree("solve_kwargs").unwrap_or_default();
        let _ = overrides.set_key("solver_options.output_flag", Node::new(Value::Bool(false)));
        model
            .solve(&SolveOptions {
                overrides,
                ..SolveOptions::default()
            })
            .map_err(failed)?;
        let results_nc = out_dir.join("results.nc");
        model.to_netcdf(&results_nc).map_err(failed)?;
        let mut objectives = BTreeMap::new();
        for name in model.math.objectives.keys() {
            if let Some(a) = model.results.get(name.as_str()) {
                objectives.insert(name.to_string(), a.to_dense());
            }
        }
        Ok(RunOutput {
            inputs_nc: Some(inputs_nc),
            lp: Some(lp),
            results_nc: Some(results_nc),
            objectives,
            termination: model.runtime.termination_condition.clone(),
        })
    }
}
