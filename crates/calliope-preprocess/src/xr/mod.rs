//! A small dense re-implementation of the xarray/pandas behaviour that Calliope's
//! preprocessing relies on: numpy dtypes and their promotion when values go missing,
//! pandas `Index.union` ordering for outer joins, `merge`, `fillna`, `where`, `any`.
//!
//! Preprocessing reproduces Python's input dataset exactly (dtypes, label order, dimension
//! order), so it works on dense arrays like xarray does; the result converts to
//! [`calliope_array::LArray`]s for the math engine.

mod array;
mod data;
mod dataset;

pub use array::{DataArray, Join, comparable, join_coords, union_labels};
pub use data::{DType, Data, NAT, Obj, format_datetime, parse_py_float};
pub use dataset::{Compat, Dataset, Var};
