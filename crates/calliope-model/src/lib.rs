//! Calliope model lifecycle: init, build, solve, operate and SPORES modes.
//!
//! So far: [`build`], which builds the optimisation problem of a preprocessed model.

use calliope_math::eval::{BuiltModel, build_model};
use calliope_preprocess::backend::{backend_dataset, check_inputs, math_inputs};
use calliope_preprocess::{ModelError, ModelInputs};
use calliope_schema::Build;

/// `Model.build`: the backend inputs (operate mode's first window, undefined members
/// dropped) evaluated with the model's math and `config.build`.
pub fn build(model: &ModelInputs) -> Result<BuiltModel, ModelError> {
    build_with(model, &model.config.build)
}

/// `Model.build(**kwargs)`: as [`build`], with a changed build configuration.
///
/// The math `checks` run first (`BackendModel._check_inputs`): a failed `raise` check is an
/// error; failed `warn` checks are logged.
pub fn build_with(model: &ModelInputs, config: &Build) -> Result<BuiltModel, ModelError> {
    for w in check_inputs(model)? {
        tracing::warn!("{w}");
    }
    let ds = backend_dataset(model)?;
    let inputs = math_inputs(&ds, &model.math)?;
    build_model(&inputs, &model.math, config).map_err(|e| ModelError::new(e.0))
}
