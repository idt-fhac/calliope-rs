//! The preprocessing pipeline (`calliope.read_yaml` / `read_dict` and `Model.__init__` up to
//! the cleaned input dataset).

use std::path::Path;

use calliope_schema::{
    CalliopeBuildMath, CalliopeConfig, CalliopeModelDef, CalliopeRuntime, Validated,
};
use calliope_yaml::{AttrTree, read_yaml_file};
use indexmap::IndexMap;

use crate::data_tables::DataTable;
use crate::error::ModelError;
use crate::model_data::{ModelDataBuilder, ModelDataCleaner};
use crate::model_definition::prepare_model_definition_with;
use crate::model_math::{build_math, get_math_priority};
use crate::xr::Dataset;

/// A preprocessed model: the input dataset and the model attributes.
#[derive(Debug, Clone)]
pub struct ModelInputs {
    /// The cleaned input dataset (`model.inputs`).
    pub inputs: Dataset,
    /// `config`.
    pub config: Validated<CalliopeConfig>,
    /// The model definition (with what the data tables imply).
    pub definition: Validated<CalliopeModelDef>,
    /// `math.init`.
    pub math_init: IndexMap<String, AttrTree>,
    /// `math.build`.
    pub math: Validated<CalliopeBuildMath>,
    /// `runtime`.
    pub runtime: CalliopeRuntime,
    /// Warnings raised while preprocessing (`ModelWarning`s).
    pub warnings: Vec<String>,
}

/// Options of [`read_yaml`] / [`read_dict`].
#[derive(Debug, Clone, Default)]
pub struct ReadOptions {
    /// Comma-separated scenario/override names.
    pub scenario: Option<String>,
    /// Overrides applied after the scenario.
    pub override_dict: Option<AttrTree>,
    /// Extra named math (`math.init` entries).
    pub math_dict: Option<AttrTree>,
    /// `config.init` keyword overrides.
    pub init_kwargs: AttrTree,
}

/// `calliope.read_yaml`.
pub fn read_yaml(path: &Path, opts: &ReadOptions) -> Result<ModelInputs, ModelError> {
    let def = read_yaml_file(path, false).map_err(|e| ModelError::new(e.to_string()))?;
    read_dict(&def, Some(path), opts)
}

/// `calliope.read_dict`, then cleaning as `Model.__init__` does.
pub fn read_dict(
    model_definition: &AttrTree,
    definition_path: Option<&Path>,
    opts: &ReadOptions,
) -> Result<ModelInputs, ModelError> {
    let prepared = prepare_model_definition_with(
        model_definition,
        opts.scenario.as_deref(),
        opts.override_dict.as_ref(),
        opts.math_dict.as_ref(),
        definition_path,
        &opts.init_kwargs,
    )?;
    let init = &prepared.config.init;
    let priority = get_math_priority(init);
    let math = build_math(&priority, &prepared.math_init, None)?;
    let mut tables = Vec::new();
    for (name, t) in &prepared.definition.data_tables.0 {
        tables.push(DataTable::new(name, t, definition_path)?);
    }
    let mut builder = ModelDataBuilder::new(init, prepared.definition.clone(), &math, &mut tables)?;
    builder.build()?;
    let mut warnings = std::mem::take(&mut builder.warnings);
    let definition = builder.model_definition.clone();
    let runtime = CalliopeRuntime {
        applied_overrides: Some(prepared.applied_overrides.clone()),
        scenario: prepared.scenario.clone(),
        math_priority: priority
            .iter()
            .filter_map(|p| calliope_schema::AttrStr::new(p.clone()).ok())
            .collect(),
        ..CalliopeRuntime::default()
    };
    let mut cleaner = ModelDataCleaner::new(init, builder.dataset, &math, runtime);
    cleaner.clean()?;
    warnings.extend(cleaner.warnings);
    let (inputs, runtime) = (cleaner.dataset, cleaner.runtime);
    Ok(ModelInputs {
        inputs,
        config: prepared.config.clone(),
        definition,
        math_init: prepared.math_init,
        math,
        runtime,
        warnings,
    })
}
