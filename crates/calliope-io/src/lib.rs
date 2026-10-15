//! NetCDF and CSV read/write with Calliope attribute (de)serialisation.
//!
//! Phase 3 provides what the input-dataset parity check needs: writing and reading a
//! dataset group in the layout Python Calliope writes ([`write_model_netcdf`],
//! [`read_group`]) and comparing datasets ([`compare_datasets`]). Full `to_netcdf` /
//! `read_netcdf` parity (results, attribute serialisation) follows in Phase 5.

mod compare;
mod netcdf_io;

pub use compare::{Diff, compare_datasets};
pub use netcdf_io::{read_group, read_group_attrs, write_group, write_model_netcdf};

/// I/O errors.
#[derive(Debug, thiserror::Error)]
pub enum IoError {
    /// From libnetcdf.
    #[error("NetCDF: {0}")]
    NetCdf(#[from] netcdf::Error),
    /// Unexpected content.
    #[error("{0}")]
    Format(String),
}
