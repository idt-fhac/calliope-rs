//! CSV export in the layout of `calliope.io.save_csv` / `Model.to_csv`.
//!
//! One file per data variable, `<group>_<name>.csv`. An N-d array is written like
//! `DataArray.to_series().to_csv()`: one column per dimension (C order over the full
//! product) and a value column named after the array; missing values are dropped with
//! `dropna`. A 0-d array is written like `pd.Series(value).to_csv(index=False)`: header `0`.

use std::fmt::Write as _;
use std::path::Path;

use calliope_array::Label;
use calliope_preprocess::xr::{Data, Dataset, Obj, format_datetime};

use crate::IoError;
use crate::attrs::{AttrValue, Attrs};

fn quote(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

fn label_str(l: &Label) -> String {
    match l {
        Label::Str(s) => s.to_string(),
        Label::Int(i) => i.to_string(),
        Label::Float(f) => calliope_yaml::python_float(*f),
        Label::Datetime(ns) => format_datetime(*ns),
    }
}

/// pandas `to_csv` cell text; `None` for a missing value.
fn cell(data: &Data, i: usize) -> Option<String> {
    if data.is_null(i) {
        return None;
    }
    Some(match data.get(i) {
        Obj::Nan => return None,
        Obj::Datetime(ns) => format_datetime(ns),
        o => o.to_py_str(),
    })
}

fn io_err(e: std::io::Error) -> IoError {
    IoError::Format(e.to_string())
}

/// `save_csv`: write every data variable of `ds` to `dir/<group>_<name>.csv`.
pub fn save_csv(
    ds: &Dataset,
    group: &str,
    dir: &Path,
    dropna: bool,
    allow_overwrite: bool,
) -> Result<(), IoError> {
    if dir.exists() && !allow_overwrite {
        return Err(IoError::Format(format!(
            "directory {} already exists",
            dir.display()
        )));
    }
    std::fs::create_dir_all(dir).map_err(io_err)?;
    for (name, var) in &ds.vars {
        let mut out = String::new();
        if var.dims.is_empty() {
            out.push_str("0\n");
            match cell(&var.data, 0) {
                Some(v) => {
                    let _ = writeln!(out, "{}", quote(&v));
                }
                None if !dropna => out.push('\n'),
                None => {}
            }
        } else {
            let coords: Vec<&calliope_array::Coord> =
                var.dims.iter().map(|d| &ds.coords[d]).collect();
            let header: Vec<String> = var
                .dims
                .iter()
                .map(|d| quote(d))
                .chain([quote(name)])
                .collect();
            let _ = writeln!(out, "{}", header.join(","));
            let shape: Vec<usize> = coords.iter().map(|c| c.len()).collect();
            let mut idx = vec![0usize; shape.len()];
            for i in 0..var.data.len() {
                let v = cell(&var.data, i);
                if v.is_some() || !dropna {
                    for (k, c) in coords.iter().enumerate() {
                        out.push_str(&quote(&label_str(&c.labels()[idx[k]])));
                        out.push(',');
                    }
                    out.push_str(&v.map(|v| quote(&v)).unwrap_or_default());
                    out.push('\n');
                }
                for k in (0..shape.len()).rev() {
                    idx[k] += 1;
                    if idx[k] < shape[k] {
                        break;
                    }
                    idx[k] = 0;
                }
            }
        }
        std::fs::write(dir.join(format!("{group}_{name}.csv")), out).map_err(io_err)?;
    }
    Ok(())
}

/// Model attributes as YAML (`attrs.yaml` of `Model.to_csv`).
pub fn attrs_to_yaml(attrs: &Attrs) -> String {
    use calliope_yaml::{AttrTree, Node, Value};
    let mut tree = AttrTree::new();
    for (k, v) in attrs {
        let node = match v {
            AttrValue::Str(s) => Value::Str(s.clone()),
            AttrValue::Int(i) => Value::Int(*i),
            AttrValue::Float(f) => Value::Float(*f),
            AttrValue::Bool(b) => Value::Bool(*b),
            AttrValue::None => Value::Null,
            AttrValue::List(l) | AttrValue::Set(l) => {
                Value::Seq(l.iter().map(|s| Node::new(Value::Str(s.clone()))).collect())
            }
            AttrValue::Dict(d) => Value::Map(d.clone()),
            AttrValue::Floats(f) => {
                Value::Seq(f.iter().map(|x| Node::new(Value::Float(*x))).collect())
            }
            AttrValue::Ints(i) => Value::Seq(i.iter().map(|x| Node::new(Value::Int(*x))).collect()),
        };
        tree.insert(k.clone(), Node::new(node));
    }
    calliope_yaml::to_yaml(&tree)
}

/// `Model.to_csv`: inputs, results (if any) and `attrs.yaml`.
pub fn model_to_csv(
    model: &crate::ModelNetcdf,
    dir: &Path,
    dropna: bool,
    allow_overwrite: bool,
) -> Result<(), IoError> {
    save_csv(&model.inputs.ds, "inputs", dir, dropna, allow_overwrite)?;
    if !model.results.ds.vars.is_empty() {
        save_csv(&model.results.ds, "results", dir, dropna, true)?;
    }
    std::fs::write(dir.join("attrs.yaml"), attrs_to_yaml(&model.attrs)).map_err(io_err)
}
