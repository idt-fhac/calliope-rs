//! HiGHS solver backend for [`calliope_lp::LpModel`].
//!
//! - [`ffi`]: a small safe wrapper over the HiGHS C API (`highs-sys`, HiGHS 1.15);
//! - [`backend`]: [`HighsBackend`] — solve with Calliope's option handling, results
//!   (primal values, duals, termination condition, objective) and in-place updates.
//!
//! Building needs `cmake` and libclang (`LIBCLANG_PATH`) for `highs-sys`.

pub mod backend;
pub mod ffi;

pub use backend::{BackendError, DependencyIndex, HighsBackend, SolveConfig, SolveOutcome, Target};
pub use ffi::{Highs, HighsError, ModelStatus, OptionValue};
