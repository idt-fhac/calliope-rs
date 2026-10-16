//! NetCDF read/write of a dataset group, in the layout xarray writes for Calliope
//! (`bool` as `int8`, strings as variable-length strings, datetimes as `int64` with CF
//! `units`).

use std::path::Path;

use calliope_array::{Coord, Label};
use calliope_preprocess::xr::{Data, Dataset, NAT, Obj};
use netcdf::types::{FloatType, IntType, NcVariableType};
use netcdf::{AttributeValue, Extents, GroupMut};

use crate::IoError;

const NA_STRING: &str = "<NA>";

/// CF time units, largest first, with their length in ns (`xarray.coding.times`).
const TIME_UNITS: [(&str, i64); 7] = [
    ("days", 86_400_000_000_000),
    ("hours", 3_600_000_000_000),
    ("minutes", 60_000_000_000),
    ("seconds", 1_000_000_000),
    ("milliseconds", 1_000_000),
    ("microseconds", 1_000),
    ("nanoseconds", 1),
];

/// xarray's CF encoding of a datetime array (`infer_datetime_units`): the reference is the
/// first non-NaT value, the unit the largest that divides every step between consecutive
/// non-NaT values; NaT is stored as `i64::MIN` without a `_FillValue`.
fn encode_datetimes(v: &[i64]) -> (String, Vec<i64>) {
    let valid: Vec<i64> = v.iter().copied().filter(|t| *t != NAT).collect();
    let Some(&reference) = valid.first() else {
        return ("days since 1970-01-01".to_owned(), v.to_vec());
    };
    let (unit, len) = TIME_UNITS
        .iter()
        .copied()
        .find(|(_, len)| valid.windows(2).all(|w| (w[1] - w[0]) % len == 0))
        .unwrap_or(("nanoseconds", 1));
    let values = v
        .iter()
        .map(|t| {
            if *t == NAT {
                NAT
            } else {
                (t - reference) / len
            }
        })
        .collect();
    let since = calliope_preprocess::xr::format_datetime(reference);
    (format!("{unit} since {since}"), values)
}

fn flat_to_index(mut flat: usize, shape: &[usize]) -> Vec<usize> {
    let mut idx = vec![0; shape.len()];
    for i in (0..shape.len()).rev() {
        idx[i] = flat % shape[i];
        flat /= shape[i];
    }
    idx
}

fn extents_of(idx: &[usize]) -> Extents {
    Extents::from(idx.to_vec())
}

/// How a variable is encoded: coordinates are stored plainly; data variables get
/// Calliope's `save_netcdf` encoding (zlib level 4 for numbers, `<NA>` fill for strings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    Coord,
    Data,
}

fn put_data(
    group: &mut GroupMut<'_>,
    name: &str,
    dims: &[&str],
    data: &Data,
    shape: &[usize],
    role: Role,
    attrs: &[(String, AttributeValue)],
) -> Result<(), IoError> {
    let compress = role == Role::Data && !shape.is_empty();
    match data {
        Data::Float(v) => {
            let mut var = group.add_variable::<f64>(name, dims)?;
            if compress {
                var.set_compression(4, true)?;
            }
            var.set_fill_value(f64::NAN)?;
            put_attrs(&mut var, attrs)?;
            var.put_values(v, ..)?;
        }
        Data::Int(v) => {
            let mut var = group.add_variable::<i64>(name, dims)?;
            if compress {
                var.set_compression(4, true)?;
            }
            put_attrs(&mut var, attrs)?;
            var.put_values(v, ..)?;
        }
        Data::Bool(v) => {
            // `{"dtype": "int8", "_FillValue": None}` as in `save_netcdf`
            let vals: Vec<i8> = v.iter().map(|b| i8::from(*b)).collect();
            let mut var = group.add_variable::<i8>(name, dims)?;
            put_attrs(&mut var, attrs)?;
            var.put_values(&vals, ..)?;
        }
        Data::Datetime(v) => {
            let mut var = group.add_variable::<i64>(name, dims)?;
            if compress {
                var.set_compression(4, true)?;
            }
            let (units, values) = encode_datetimes(v);
            var.put_attribute("units", units)?;
            var.put_attribute("calendar", "proleptic_gregorian")?;
            put_attrs(&mut var, attrs)?;
            var.put_values(&values, ..)?;
        }
        Data::Object(v) => {
            let mut var = group.add_string_variable(name, dims)?;
            if role == Role::Data {
                var.put_attribute("_FillValue", vec![NA_STRING.to_owned()])?;
            }
            put_attrs(&mut var, attrs)?;
            for (i, o) in v.iter().enumerate() {
                let s = match o {
                    Obj::Nan => NA_STRING.to_owned(),
                    o => o.to_py_str(),
                };
                if shape.is_empty() {
                    var.put_string(&s, ..)?;
                } else {
                    var.put_string(&s, extents_of(&flat_to_index(i, shape)))?;
                }
            }
        }
    }
    Ok(())
}

fn put_attrs(
    var: &mut netcdf::VariableMut<'_>,
    attrs: &[(String, AttributeValue)],
) -> Result<(), IoError> {
    for (k, v) in attrs {
        var.put_attribute(k, v.clone())?;
    }
    Ok(())
}

fn coord_data(c: &Coord) -> Data {
    let labels = c.labels();
    if labels.iter().all(|l| matches!(l, Label::Datetime(_))) && !labels.is_empty() {
        Data::Datetime(
            labels
                .iter()
                .map(|l| if let Label::Datetime(d) = l { *d } else { NAT })
                .collect(),
        )
    } else if labels.iter().all(|l| matches!(l, Label::Int(_))) && !labels.is_empty() {
        Data::Int(
            labels
                .iter()
                .map(|l| if let Label::Int(d) = l { *d } else { 0 })
                .collect(),
        )
    } else if labels
        .iter()
        .all(|l| matches!(l, Label::Float(_) | Label::Int(_)))
        && !labels.is_empty()
    {
        Data::Float(
            labels
                .iter()
                .map(|l| match l {
                    Label::Float(f) => *f,
                    #[allow(clippy::cast_precision_loss)]
                    Label::Int(i) => *i as f64,
                    _ => f64::NAN,
                })
                .collect(),
        )
    } else {
        Data::Object(labels.iter().map(Obj::from_label).collect())
    }
}

/// Writes a dataset as a group of a new or existing NetCDF file (plain string attributes
/// on the group, no variable attributes).
pub fn write_group(
    file: &mut netcdf::FileMut,
    group: &str,
    ds: &Dataset,
    attrs: &[(&str, String)],
) -> Result<(), IoError> {
    let attrs: Vec<(String, AttributeValue)> = attrs
        .iter()
        .map(|(k, v)| ((*k).to_string(), AttributeValue::Str(v.clone())))
        .collect();
    write_group_raw(file, group, ds, &attrs, &|_| Vec::new())
}

/// Writes a dataset group with raw group attributes and per-variable (and coordinate)
/// attributes from `var_attrs`.
pub(crate) fn write_group_raw(
    file: &mut netcdf::FileMut,
    group: &str,
    ds: &Dataset,
    attrs: &[(String, AttributeValue)],
    var_attrs: &dyn Fn(&str) -> Vec<(String, AttributeValue)>,
) -> Result<(), IoError> {
    let mut g = file.add_group(group)?;
    for (k, v) in attrs {
        g.add_attribute(k, v.clone())?;
    }
    for (name, c) in &ds.coords {
        g.add_dimension(name, c.len())?;
    }
    for (name, c) in &ds.coords {
        put_data(
            &mut g,
            name,
            &[name.as_str()],
            &coord_data(c),
            &[c.len()],
            Role::Coord,
            &var_attrs(name),
        )?;
    }
    for (name, var) in &ds.vars {
        let dims: Vec<&str> = var.dims.iter().map(String::as_str).collect();
        let shape: Vec<usize> = var.dims.iter().map(|d| ds.coords[d].len()).collect();
        put_data(
            &mut g,
            name,
            &dims,
            &var.data,
            &shape,
            Role::Data,
            &var_attrs(name),
        )?;
    }
    Ok(())
}

/// Writes `inputs` (and empty `results`/`attrs` groups with `attrs`) like `Model.to_netcdf`.
pub fn write_model_netcdf(
    path: &Path,
    inputs: &Dataset,
    attrs: &[(&str, String)],
) -> Result<(), IoError> {
    let mut file = netcdf::create(path)?;
    write_group(&mut file, "inputs", inputs, &[])?;
    write_group(&mut file, "results", &Dataset::new(), &[])?;
    write_group(&mut file, "attrs", &Dataset::new(), attrs)?;
    Ok(())
}

fn attr_str(v: &netcdf::Variable<'_>, name: &str) -> Option<String> {
    match v.attribute_value(name)?.ok()? {
        AttributeValue::Str(s) => Some(s),
        _ => None,
    }
}

fn time_unit_ns(units: &str) -> Option<(i64, i64)> {
    let (unit, since) = units.split_once(" since ")?;
    let mult: i64 = match unit.trim() {
        "nanoseconds" => 1,
        "microseconds" => 1_000,
        "milliseconds" => 1_000_000,
        "seconds" => 1_000_000_000,
        "minutes" => 60_000_000_000,
        "hours" => 3_600_000_000_000,
        "days" => 86_400_000_000_000,
        _ => return None,
    };
    let origin = calliope_preprocess::time::parse_iso8601(since.trim())?;
    Some((mult, origin))
}

pub(crate) fn read_var(v: &netcdf::Variable<'_>) -> Result<Data, IoError> {
    let shape: Vec<usize> = v.dimensions().iter().map(netcdf::Dimension::len).collect();
    let total: usize = shape.iter().product();
    let units = attr_str(v, "units");
    let time = units.as_deref().and_then(time_unit_ns);
    Ok(match v.vartype() {
        NcVariableType::String => {
            let mut out = Vec::with_capacity(total);
            for i in 0..total {
                let s = if shape.is_empty() {
                    v.get_string(..)?
                } else {
                    v.get_string(extents_of(&flat_to_index(i, &shape)))?
                };
                out.push(if s == NA_STRING {
                    Obj::Nan
                } else {
                    Obj::str(&s)
                });
            }
            Data::Object(out)
        }
        NcVariableType::Int(IntType::I8 | IntType::U8) => {
            let vals: Vec<i8> = v.get_values(..)?;
            Data::Bool(vals.iter().map(|x| *x != 0).collect())
        }
        NcVariableType::Int(_) => {
            let vals: Vec<i64> = v.get_values(..)?;
            match time {
                Some((mult, origin)) => {
                    let fill: Option<i64> = v.fill_value()?;
                    Data::Datetime(
                        vals.iter()
                            .map(|x| {
                                if Some(*x) == fill || *x == NAT {
                                    NAT
                                } else {
                                    origin + x * mult
                                }
                            })
                            .collect(),
                    )
                }
                None => Data::Int(vals),
            }
        }
        NcVariableType::Float(FloatType::F32 | FloatType::F64) => {
            let vals: Vec<f64> = v.get_values(..)?;
            match time {
                #[allow(clippy::cast_possible_truncation)]
                Some((mult, origin)) => Data::Datetime(
                    vals.iter()
                        .map(|x| {
                            if x.is_nan() {
                                NAT
                            } else {
                                origin + (*x * mult as f64).round() as i64
                            }
                        })
                        .collect(),
                ),
                None => {
                    let fill: Option<f64> = v.fill_value()?;
                    Data::Float(
                        vals.into_iter()
                            .map(|x| {
                                if fill.is_some_and(|f| f == x) {
                                    f64::NAN
                                } else {
                                    x
                                }
                            })
                            .collect(),
                    )
                }
            }
        }
        other => {
            return Err(IoError::Format(format!(
                "unsupported NetCDF type {:?}",
                other.size()
            )));
        }
    })
}

/// Reads one group of a NetCDF file into a dataset (dimension coordinates and variables).
pub fn read_group(path: &Path, group: &str) -> Result<Dataset, IoError> {
    let file = netcdf::open(path)?;
    let g = file
        .group(group)?
        .ok_or_else(|| IoError::Format(format!("no group `{group}` in {}", path.display())))?;
    read_group_dataset(&g)
}

/// Reads an open group into a dataset.
pub(crate) fn read_group_dataset(g: &netcdf::Group<'_>) -> Result<Dataset, IoError> {
    let mut ds = Dataset::new();
    let dim_names: Vec<String> = g.dimensions().map(|d| d.name()).collect();
    for d in &dim_names {
        let labels: Vec<Label> = match g.variable(d) {
            Some(v) => {
                let data = read_var(&v)?;
                (0..data.len()).map(|i| data.get(i).to_label()).collect()
            }
            None => {
                let n = g.dimension(d).map_or(0, |x| x.len());
                (0..n)
                    .map(|i| Label::Int(i64::try_from(i).unwrap_or(0)))
                    .collect()
            }
        };
        ds.coords.insert(
            d.clone(),
            Coord::new(d, labels).map_err(|e| IoError::Format(e.to_string()))?,
        );
    }
    for v in g.variables() {
        let name = v.name();
        if dim_names.contains(&name) {
            continue;
        }
        let dims: Vec<String> = v.dimensions().iter().map(netcdf::Dimension::name).collect();
        let data = read_var(&v)?;
        ds.vars
            .insert(name, calliope_preprocess::xr::Var { dims, data });
    }
    Ok(ds)
}

/// String attributes of a group.
pub fn read_group_attrs(path: &Path, group: &str) -> Result<Vec<(String, String)>, IoError> {
    let file = netcdf::open(path)?;
    let Some(g) = file.group(group)? else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for a in g.attributes() {
        if let Ok(AttributeValue::Str(s)) = a.value() {
            out.push((a.name().to_owned(), s));
        }
    }
    Ok(out)
}
