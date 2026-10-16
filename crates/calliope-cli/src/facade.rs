//! Loading models for the CLI (`_run_setup_model`) and the run steps it calls on
//! [`calliope_model::Model`].

use std::path::{Path, PathBuf};

use calliope_model::{BuildOptions, Model};
use calliope_preprocess::ReadOptions;
use calliope_yaml::{AttrTree, Node, Value};

use crate::error::{CliError, CliResult};
use crate::generate::read_rich_yaml;

/// Options handed to `solve` (Python: `model.solve(**kwargs)`).
#[derive(Debug, Clone, Default)]
pub struct SolveOptions {
    /// Directory for solver logs (`solve.save_logs`).
    pub save_logs: Option<PathBuf>,
    /// Directory for per-SPORE results (`solve.spores.save_per_spore_path`).
    pub save_per_spore_path: Option<PathBuf>,
}

/// How to load a model (`_run_setup_model`).
#[derive(Debug, Clone, Default)]
pub struct LoadOptions {
    /// Model file.
    pub model_file: String,
    /// `--scenario`.
    pub scenario: Option<String>,
    /// `--model_format`.
    pub model_format: Option<String>,
    /// `--override_dict` (YAML text or file).
    pub override_dict: Option<String>,
}

fn err(e: &calliope_model::Error) -> CliError {
    CliError::from_error(e)
}

/// `_run_setup_model`: a model from YAML (with scenario and overrides) or NetCDF.
pub fn load_model(o: &LoadOptions) -> CliResult<Model> {
    let ext = o.model_file.rsplit('.').next().unwrap_or("");
    let format = match o.model_format.as_deref() {
        Some(f) => f.to_owned(),
        None => match ext {
            "yaml" | "yml" => "yaml".to_owned(),
            "nc" | "nc4" | "netcdf" => "netcdf".to_owned(),
            _ => {
                return Err(CliError::new(format!(
                    "Cannot determine model file format based on file extension for \"{}\". \
                     Set format explicitly with --model_format.",
                    o.model_file
                )));
            }
        },
    };
    let overrides = o.override_dict.as_deref().map(read_rich_yaml).transpose()?;
    match format.as_str() {
        "yaml" => {
            let opts = ReadOptions {
                scenario: o.scenario.clone(),
                override_dict: overrides,
                ..ReadOptions::default()
            };
            Model::from_yaml(Path::new(&o.model_file), &opts).map_err(|e| err(&e))
        }
        "netcdf" => {
            if o.scenario.is_some() || o.override_dict.is_some() {
                return Err(CliError::new(
                    "When loading a pre-built model from NetCDF, the --scenario and \
                     --override_dict options are not available.",
                ));
            }
            Model::from_netcdf(Path::new(&o.model_file)).map_err(|e| err(&e))
        }
        other => Err(CliError::new(format!("Invalid model format: {other}"))),
    }
}

/// `model.build()`.
pub fn build(model: &mut Model) -> CliResult<()> {
    model.build(&BuildOptions::default()).map_err(|e| err(&e))
}

/// `model.backend.to_lp(path)`.
pub fn to_lp(model: &Model, path: &Path) -> CliResult<()> {
    model
        .backend()
        .ok_or_else(|| CliError::new("The model has not been built."))?
        .to_lp(path)
        .map_err(|e| err(&e))
}

/// `model.solve(**kwargs)`; returns the termination condition.
pub fn solve(model: &mut Model, opts: &SolveOptions) -> CliResult<String> {
    let mut overrides = AttrTree::new();
    let path = |p: &Path| Node::new(Value::Str(p.to_string_lossy().into_owned()));
    if let Some(d) = &opts.save_logs {
        overrides
            .set_key("save_logs", path(d))
            .map_err(|e| CliError::new(e.to_string()))?;
    }
    if let Some(d) = &opts.save_per_spore_path {
        overrides
            .set_key("spores.save_per_spore_path", path(d))
            .map_err(|e| CliError::new(e.to_string()))?;
    }
    model
        .solve(&calliope_model::SolveOptions {
            overrides,
            ..calliope_model::SolveOptions::default()
        })
        .map_err(|e| err(&e))?;
    Ok(model
        .runtime
        .termination_condition
        .clone()
        .unwrap_or_default())
}

/// `model.to_csv(dir)`.
pub fn to_csv(model: &Model, dir: &Path) -> CliResult<()> {
    model.to_csv(dir, true, false).map_err(|e| err(&e))
}

/// `model.to_netcdf(path)`.
pub fn to_netcdf(model: &Model, path: &Path) -> CliResult<()> {
    model.to_netcdf(path).map_err(|e| err(&e))
}
