//! The `Model` facade the CLI talks to.
//!
//! [`load_model`] returns a [`Model`]. The current implementation covers preprocessing only
//! (YAML load with scenarios and overrides, inputs from NetCDF); `build` and `solve` return a
//! clear error. Swap [`load_model`] for the evaluator-backed implementation when it exists.

use std::path::{Path, PathBuf};

use calliope_preprocess::{ModelInputs, ReadOptions, read_yaml};

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

/// What the CLI needs from a model.
pub trait Model {
    /// `Model.info()`.
    fn info(&self) -> String;
    /// `config.solve.spores.save_per_spore_path`.
    fn spores_save_path(&self) -> Option<PathBuf>;
    /// `Model.build()`.
    fn build(&mut self) -> CliResult<()>;
    /// `Model.solve(...)`; returns the termination condition.
    fn solve(&mut self, opts: &SolveOptions) -> CliResult<String>;
    /// `Model.backend.to_lp(path)`.
    fn to_lp(&self, path: &Path) -> CliResult<()>;
    /// `Model.to_csv(dir)`.
    fn to_csv(&self, dir: &Path) -> CliResult<()>;
    /// `Model.to_netcdf(path)`.
    fn to_netcdf(&self, path: &Path) -> CliResult<()>;
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

const UNAVAILABLE: &str = "building and solving models is not available yet in this version of calliope-rs (preprocessing only)";

/// A model that went through preprocessing only.
#[derive(Debug)]
pub struct PreprocessedModel {
    name: Option<String>,
    dims: Vec<(String, usize)>,
    n_active: usize,
    spores_path: Option<PathBuf>,
    /// Warnings raised while preprocessing.
    pub warnings: Vec<String>,
}

impl PreprocessedModel {
    fn from_dataset(
        ds: &calliope_preprocess::xr::Dataset,
        name: Option<String>,
        spores_path: Option<PathBuf>,
        warnings: Vec<String>,
    ) -> Self {
        let dims = ds
            .coords
            .iter()
            .map(|(k, c)| (k.clone(), c.len()))
            .collect();
        let n_active = ds.vars.get("active").map_or(0, |v| {
            (0..v.data.len())
                .filter(|&i| v.data.get(i).truthy())
                .count()
        });
        Self {
            name,
            dims,
            n_active,
            spores_path,
            warnings,
        }
    }
}

impl Model for PreprocessedModel {
    fn info(&self) -> String {
        let dims = self
            .dims
            .iter()
            .map(|(k, n)| format!("'{k}': {n}"))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "Model name:   {}\nModel size:   {{{dims}}} ({} valid node:tech:carrier combinations)",
            self.name.as_deref().unwrap_or("None"),
            self.n_active
        )
    }
    fn spores_save_path(&self) -> Option<PathBuf> {
        self.spores_path.clone()
    }
    fn build(&mut self) -> CliResult<()> {
        Err(CliError::new(UNAVAILABLE))
    }
    fn solve(&mut self, _opts: &SolveOptions) -> CliResult<String> {
        Err(CliError::new(UNAVAILABLE))
    }
    fn to_lp(&self, _path: &Path) -> CliResult<()> {
        Err(CliError::new(UNAVAILABLE))
    }
    fn to_csv(&self, _dir: &Path) -> CliResult<()> {
        Err(CliError::new(UNAVAILABLE))
    }
    fn to_netcdf(&self, _path: &Path) -> CliResult<()> {
        Err(CliError::new(UNAVAILABLE))
    }
}

/// `_run_setup_model`.
pub fn load_model(o: &LoadOptions) -> CliResult<Box<dyn Model>> {
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
            let m: ModelInputs =
                read_yaml(Path::new(&o.model_file), &opts).map_err(|e| CliError::from_error(&e))?;
            let name = m.config.init.name.clone();
            let spores = m.config.solve.spores.save_per_spore_path.clone();
            Ok(Box::new(PreprocessedModel::from_dataset(
                &m.inputs,
                name,
                spores,
                m.warnings.clone(),
            )))
        }
        "netcdf" => {
            if o.scenario.is_some() || o.override_dict.is_some() {
                return Err(CliError::new(
                    "When loading a pre-built model from NetCDF, the --scenario and \
                     --override_dict options are not available.",
                ));
            }
            let nc = calliope_io::read_netcdf(Path::new(&o.model_file))
                .map_err(|e| CliError::from_error(&e))?;
            Ok(Box::new(PreprocessedModel::from_dataset(
                &nc.inputs.ds,
                None,
                None,
                Vec::new(),
            )))
        }
        other => Err(CliError::new(format!("Invalid model format: {other}"))),
    }
}
