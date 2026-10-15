//! NetCDF and CSV read/write with Calliope attribute (de)serialisation.
//!
//! - [`write_netcdf`] / [`read_netcdf`]: whole-model files (`inputs`, `results`, `attrs`
//!   groups) that Python `calliope.read_netcdf` reads, and that Python writes;
//! - [`attrs`]: `_serialise` / `_deserialise` of attributes;
//! - [`save_csv`] / [`model_to_csv`]: CSV export like `Model.to_csv`;
//! - [`write_group`] / [`read_group`] / [`compare_datasets`]: single groups (input parity).

pub mod attrs;
mod compare;
mod csv_out;
mod model_nc;
mod netcdf_io;

pub use attrs::{AttrValue, Attrs};
pub use compare::{Diff, compare_datasets};
pub use csv_out::{attrs_to_yaml, model_to_csv, save_csv};
pub use model_nc::{ModelNetcdf, NcGroup, read_netcdf, results_dataset, write_netcdf};
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
