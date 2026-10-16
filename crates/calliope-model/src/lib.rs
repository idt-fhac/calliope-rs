//! Calliope model lifecycle: init, build, solve, operate and SPORES modes.
//!
//! - [`Model`]: `calliope.Model` — read from YAML, a dict or NetCDF; `build`, `solve`,
//!   `results`, `to_netcdf`/`to_csv`, `info`, runtime attributes and timings.
//! - [`Backend`]: the built problem and its HiGHS solver (`model.backend`), with the
//!   backend interface (`update_input`, `fix_variable`, ...).
//! - [`results`]: result arrays, postprocessing and `xr.concat` of operate windows and
//!   SPORES.
//! - [`build`] / [`build_with`]: the problem alone, without a solver.

pub mod backend;
mod model;
mod operate;
pub mod results;
mod rng;
mod spores;

pub use backend::{Backend, ConstraintValues};
pub use model::{BuildOptions, Model, SolveOptions};
pub use results::Results;
pub use rng::Mt19937;

use calliope_math::eval::{BuiltModel, build_model};
use calliope_preprocess::backend::{backend_dataset, check_inputs, math_inputs};
use calliope_preprocess::{ModelError, ModelInputs};
use calliope_schema::Build;

/// Errors of the model lifecycle, by Calliope exception class.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// `calliope.exceptions.ModelError`.
    #[error("{0}")]
    Model(String),
    /// `calliope.exceptions.BackendError`.
    #[error("{0}")]
    Backend(String),
    /// `KeyError` (unknown component).
    #[error("{0}")]
    Key(String),
    /// `ValueError`.
    #[error("{0}")]
    Value(String),
    /// File I/O.
    #[error(transparent)]
    Io(#[from] calliope_io::IoError),
}

impl From<ModelError> for Error {
    fn from(e: ModelError) -> Self {
        Error::Model(e.to_string())
    }
}

impl From<calliope_highs::BackendError> for Error {
    fn from(e: calliope_highs::BackendError) -> Self {
        Error::Backend(e.to_string())
    }
}

impl From<calliope_array::ArrayError> for Error {
    fn from(e: calliope_array::ArrayError) -> Self {
        Error::Value(e.to_string())
    }
}

impl From<calliope_schema::SchemaError> for Error {
    fn from(e: calliope_schema::SchemaError) -> Self {
        Error::Model(e.to_string())
    }
}

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
