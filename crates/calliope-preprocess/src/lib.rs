//! Model definition to input dataset: templates, scenarios, data tables, time
//! (Calliope `preprocess/*`).
//!
//! [`read_yaml`] runs the whole pipeline and returns the cleaned input dataset
//! ([`xr::Dataset`], dense like xarray's, see `docs/adr/0002-preprocess-dataset.md`).
//! Math `checks` are evaluated with the `where` parser in `calliope-math`, as Python does at
//! build time.

pub mod backend;
pub mod data_tables;
mod error;
mod model;
pub mod model_data;
mod model_definition;
pub mod model_math;
pub mod table;
mod templates;
pub mod time;
pub mod xr;

pub use error::ModelError;
pub use model::{ModelInputs, ReadOptions, read_dict, read_yaml};
pub use model_definition::{
    PreparedModel, load_scenario_overrides, prepare_model_definition, prepare_model_definition_with,
};
pub use templates::TemplateSolver;
