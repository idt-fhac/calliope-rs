//! Labelled N-d arrays with the xarray semantics Calliope relies on.
//!
//! # Storage (ADR 0001)
//!
//! An [`LArray`] is sparse over its non-time dimensions and dense over its time dimension
//! (the first dimension whose labels are datetimes). Only keys that have data are stored, so
//! a `[nodes, techs, carriers, timesteps]` array costs `active (node, tech, carrier) × T`.
//!
//! # The xarray behaviour reproduced here
//!
//! This is the written specification of what the port relies on; the tests in
//! `tests/xarray_oracle.rs` check it against xarray on generated cases.
//!
//! - **Missing values.** A cell is missing if it is not stored, or stores the missing value
//!   (`NaN` for `f64`, `None` for `Option`). `bool` arrays have no missing state: absent is
//!   `false`, which is how Calliope treats bool lookups (`fillna(False)`) and `where` strings
//!   (NaN → `False`).
//! - **Broadcasting by name.** Binary operations align dimensions by name. The result has the
//!   left operand's dims, then the right operand's new dims. Labels of a shared dimension are
//!   joined: equal coordinates are kept as they are; otherwise an inner join keeps the left
//!   labels that are also on the right (left order) and an outer join takes the sorted union.
//!   Arithmetic uses the inner join (xarray's `arithmetic_join` default); Calliope's inputs
//!   share one set of coordinates, so the join only matters for hand-built arrays.
//! - **NaN propagation.** [`LArray::zip_with`] is missing wherever an operand is missing.
//!   [`LArray::zip_union`] is defined wherever either operand is (`fillna`, `|`).
//! - **`where(cond)`** keeps values where `cond` is true and is missing elsewhere (inner join).
//!   **`where(cond, other)`** fills the rest of the broadcast product with `other`.
//! - **`fillna(v)`** fills every missing cell of the full product.
//! - **`sum(over, min_count)`** skips missing values (`skipna=True`). With `min_count=1`, as
//!   in Calliope's `sum` helper, a group with no values is missing; with `min_count=0`,
//!   xarray's default, it is 0. `mean` skips missing values. `any` is true if any cell is.
//! - **`roll(dim=n)`** moves data by `n` positions with wrap-around (numpy `roll`); coordinates
//!   do not move.
//! - **`isin`** gives a bool array; **`sel`** takes a list of labels (order kept; unknown labels
//!   are an error) or a label slice that is inclusive at both ends (open ends allowed; labels
//!   that are absent from a sorted coordinate bound the slice like `searchsorted`).
//! - **`group_sum(array, groupby, group_dim)`**: cells are grouped by the label `groupby` has at
//!   their coordinates; the group dimension comes first, with sorted group labels; cells
//!   without a group are dropped; groups sum with `min_count=1`.
//! - **`resample(dim=freq)`** with a fixed frequency: bins start at midnight of the first
//!   timestamp (pandas `origin="start_day"`), are closed and labelled on the left, cover the
//!   whole span (empty bins are missing); `first` takes the first non-missing value, `mean`
//!   skips missing values, `sum` uses `min_count=1`.
//! - **`concat(arrays, dim)`** stacks arrays with the same dims along a new first dimension;
//!   other coordinates are outer-joined.
//!
//! Not reproduced: `skipna=False`, calendar (non-fixed) resampling frequencies, partial
//! datetime string selection (done in preprocessing), and object-dtype mixing of types.

mod array;
mod coord;
mod fast;
mod ops;

pub use array::{Element, LArray};
pub use coord::{ArrayError, Coord, Join, Label};
pub use ops::{Resample, Selection};
