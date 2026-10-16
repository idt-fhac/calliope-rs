//! Evaluation of the lowered math ([`crate::ir`]) over model inputs, and building the
//! optimisation problem from it.
//!
//! - [`lin`]: affine expressions, the cell type of decision-variable arrays;
//! - [`value`]: evaluated values and their arithmetic (xarray broadcasting by name);
//! - [`inputs`]: the input arrays as the backend sees them;
//! - [`expr`]: the interpreter over [`crate::ir::Ir`];
//! - [`build`]: variables, global expressions, constraints, piecewise constraints and
//!   objectives, emitted into a [`calliope_lp::LpModel`]; math `checks`;
//!   `pre_validate_math_strings`.

pub mod build;
pub mod expr;
pub mod inputs;
pub mod lin;
pub mod value;

pub use build::{
    BuiltModel, CheckOutcome, build_model, label_str, pre_validate_math_strings, run_checks,
};
pub use expr::Evaluator;
pub use inputs::Inputs;
pub use lin::Lin;
pub use value::Value;

/// An evaluation error, with Calliope's message where it has one.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{0}")]
pub struct EvalError(pub String);
