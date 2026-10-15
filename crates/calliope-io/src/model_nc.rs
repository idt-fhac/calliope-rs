//! Whole-model NetCDF files (`Model.to_netcdf` / `calliope.io.read_netcdf`): the groups
//! `inputs`, `results` and `attrs`, with Calliope's attribute (de)serialisation.

use std::path::Path;

use calliope_array::{Coord, LArray};
use calliope_preprocess::xr::{Data, Dataset, Var};
use indexmap::IndexMap;

use crate::IoError;
use crate::attrs::{Attrs, group_attrs, serialise, var_attrs};
use crate::netcdf_io::{read_group_dataset, write_group_raw};

/// One group: data plus group and variable attributes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct NcGroup {
    /// Coordinates and data variables.
    pub ds: Dataset,
    /// Group (dataset) attributes.
    pub attrs: Attrs,
    /// Attributes per data variable or coordinate name (absent = none).
    pub var_attrs: IndexMap<String, Attrs>,
}

/// The three groups of a Calliope model file.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ModelNetcdf {
    /// Model inputs.
    pub inputs: NcGroup,
    /// Results (empty before a solve or after a non-optimal one).
    pub results: NcGroup,
    /// Model attributes: `definition`, `config`, `math`, `runtime` dicts, etc.
    pub attrs: Attrs,
}

fn write_one(file: &mut netcdf::FileMut, name: &str, g: &NcGroup) -> Result<(), IoError> {
    let empty = Attrs::new();
    let var_attrs = |v: &str| serialise(g.var_attrs.get(v).unwrap_or(&empty));
    write_group_raw(file, name, &g.ds, &serialise(&g.attrs), &var_attrs)
}

/// Write a model file readable by `calliope.read_netcdf` (every group, variable and
/// coordinate carries the `serialised_*` attribute lists, as `save_netcdf` writes them).
pub fn write_netcdf(path: &Path, model: &ModelNetcdf) -> Result<(), IoError> {
    let mut file = netcdf::create(path)?;
    write_one(&mut file, "inputs", &model.inputs)?;
    write_one(&mut file, "results", &model.results)?;
    write_one(
        &mut file,
        "attrs",
        &NcGroup {
            attrs: model.attrs.clone(),
            ..NcGroup::default()
        },
    )?;
    Ok(())
}

fn read_one(file: &netcdf::File, name: &str) -> Result<NcGroup, IoError> {
    let Some(g) = file.group(name)? else {
        return Ok(NcGroup::default());
    };
    let ds = read_group_dataset(&g)?;
    let mut va = IndexMap::new();
    for v in g.variables() {
        let a = var_attrs(&v)?;
        if !a.is_empty() {
            va.insert(v.name(), a);
        }
    }
    Ok(NcGroup {
        ds,
        attrs: group_attrs(&g)?,
        var_attrs: va,
    })
}

/// Read a model file written by Python Calliope or [`write_netcdf`]; attributes are
/// deserialised (dicts parsed from YAML, bools, `None`, single-element lists, sets).
pub fn read_netcdf(path: &Path) -> Result<ModelNetcdf, IoError> {
    let file = netcdf::open(path)?;
    let inputs = read_one(&file, "inputs")?;
    let results = read_one(&file, "results")?;
    let attrs = read_one(&file, "attrs")?.attrs;
    Ok(ModelNetcdf {
        inputs,
        results,
        attrs,
    })
}

/// Results arrays (float, sparse) as a dense dataset over shared coordinates. Arrays must
/// use the same coordinate per dimension name (they come from one set of inputs).
pub fn results_dataset(arrays: &IndexMap<String, LArray<f64>>) -> Result<Dataset, IoError> {
    let mut ds = Dataset::new();
    for (name, a) in arrays {
        for c in a.dims() {
            match ds.coords.get(c.name()) {
                Some(have) if have != c => {
                    return Err(IoError::Format(format!(
                        "`{name}` has different `{}` labels than earlier arrays",
                        c.name()
                    )));
                }
                Some(_) => {}
                None => {
                    ds.coords.insert(c.name().to_string(), Coord::clone(c));
                }
            }
        }
        ds.vars.insert(
            name.clone(),
            Var {
                dims: a.dim_names().iter().map(|d| (*d).to_string()).collect(),
                data: Data::Float(a.to_dense()),
            },
        );
    }
    Ok(ds)
}
