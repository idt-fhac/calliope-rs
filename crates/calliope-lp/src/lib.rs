//! Sparse LP/MILP assembly, HiGHS interface and LP writer.
//!
//! - [`model`]: the solver-independent [`model::LpModel`] the math evaluator emits into;
//! - [`writer`]: CPLEX LP writer with Pyomo-compatible names;
//! - [`lp_format`]: the LP reader, canonicaliser and differ.

pub mod lp_format;
pub mod model;
pub mod writer;

pub use model::{
    ColId, Column, ComponentKind, Label, LinExpr, LpModel, ObjSense, Objective, Row, RowId,
    RowSense, Sos2,
};
pub use writer::{LpNames, write_lp, write_lp_string};
