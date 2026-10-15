//! Model definition to input dataset: templates, scenarios, data tables, time
//! (Calliope `preprocess/*`).
//!
//! Phase 2 provides template resolution ([`TemplateSolver`]) and the scenario-free part of
//! `prepare_model_definition` ([`prepare_model_definition`]); the rest follows in Phase 3.

mod error;
mod model_definition;
mod templates;

pub use error::ModelError;
pub use model_definition::{PreparedModel, prepare_model_definition};
pub use templates::TemplateSolver;
