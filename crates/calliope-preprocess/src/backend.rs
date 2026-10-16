//! What the backend is built from: the input dataset as math-engine arrays
//! (`BackendModel._add_inputs`), sliced to the first operate window and cleaned of undefined
//! members (`Model.build`), and the math `checks` run on it.

use calliope_array::{Coord, LArray, Label};
use calliope_math::eval::{Inputs, Value, run_checks};
use calliope_schema::{CalliopeBuildMath, Mode};
use indexmap::IndexMap;

use crate::error::ModelError;
use crate::model::ModelInputs;
use crate::model_data::drop_undefined;
use crate::time::{Freq, parse_freq};
use crate::xr::{Data, Dataset, NAT, Obj};

/// The dataset `Model.build` hands the backend: operate mode's first window
/// (`_prepare_operate_mode_inputs`), then `_drop_undefined` on `active`.
pub fn backend_dataset(model: &ModelInputs) -> Result<Dataset, ModelError> {
    let mut ds = model.inputs.clone();
    if model.config.init.mode == Mode::Operate {
        if model.config.init.time_cluster.is_some() {
            return Err(ModelError::new(
                "Unable to run this model in operate (i.e. dispatch) mode because time clustering is in use",
            ));
        }
        let operate = &model.config.build.operate;
        let horizon = match parse_freq(&operate.horizon) {
            Some(Freq::Tick(ns)) => ns,
            _ => {
                return Err(ModelError::new(format!(
                    "Invalid operate horizon `{}`",
                    operate.horizon
                )));
            }
        };
        if let Some(ts) = ds.coord("timesteps").cloned() {
            let times: Vec<i64> = ts
                .labels()
                .iter()
                .filter_map(|l| match l {
                    Label::Datetime(t) => Some(*t),
                    _ => None,
                })
                .collect();
            if let (Some(&first), Some(&last)) = (times.first(), times.last()) {
                // horizonsteps[0] = min(first + horizon, last + 1ns) - 1ns
                let end = (first + horizon).min(last + 1) - 1;
                let keep: Vec<Label> = times
                    .iter()
                    .filter(|t| **t >= first && **t <= end)
                    .map(|t| Label::Datetime(*t))
                    .collect();
                ds.isel_labels("timesteps", keep);
            }
        }
    }
    let active = ds
        .get("active")
        .ok_or_else(|| ModelError::new("The input data has no `active` array"))?;
    Ok(drop_undefined(ds, &active))
}

/// The dataset as math-engine inputs (`_add_inputs`: math parameters and lookups only).
pub fn math_inputs(ds: &Dataset, math: &CalliopeBuildMath) -> Result<Inputs, ModelError> {
    let coords: IndexMap<String, Coord> = ds.coords.clone();
    let mut arrays = IndexMap::new();
    for (name, var) in &ds.vars {
        let dims: Vec<Coord> = var.dims.iter().map(|d| ds.coords[d].clone()).collect();
        let err = |e: calliope_array::ArrayError| ModelError::new(format!("{name}: {e}"));
        let v = match &var.data {
            Data::Float(v) => Value::Num(
                LArray::from_cells(
                    dims.clone(),
                    v.iter()
                        .enumerate()
                        .filter(|(_, x)| !x.is_nan())
                        .map(|(i, x)| (codes(&dims, i), *x)),
                )
                .map_err(err)?,
            ),
            Data::Sparse(s) => Value::Num(
                LArray::from_cells(dims.clone(), s.iter().map(|(i, x)| (codes(&dims, i), x)))
                    .map_err(err)?,
            ),
            #[allow(clippy::cast_precision_loss)]
            Data::Int(v) => Value::Num(
                LArray::from_dense(dims, v.iter().map(|i| *i as f64).collect()).map_err(err)?,
            ),
            Data::Bool(v) => Value::Bool(LArray::from_dense(dims, v.clone()).map_err(err)?),
            Data::Datetime(v) => Value::Label(
                LArray::from_dense(
                    dims,
                    v.iter()
                        .map(|t| (*t != NAT).then_some(Label::Datetime(*t)))
                        .collect(),
                )
                .map_err(err)?,
            ),
            Data::Object(v) => Value::Label(
                LArray::from_dense(dims, v.iter().map(obj_label).collect()).map_err(err)?,
            ),
        };
        arrays.insert(name.clone(), v);
    }
    Ok(Inputs::new(coords, arrays, math))
}

/// The per-dimension codes of flat (C order) position `i` over `dims`.
fn codes(dims: &[Coord], mut i: usize) -> Vec<u32> {
    let mut out = vec![0u32; dims.len()];
    for (k, d) in dims.iter().enumerate().rev() {
        let n = d.len();
        out[k] = u32::try_from(i % n).expect("codes fit in u32");
        i /= n;
    }
    out
}

fn obj_label(o: &Obj) -> Option<Label> {
    match o {
        Obj::Nan => None,
        Obj::Bool(b) => Some(Label::Int(i64::from(*b))),
        Obj::Int(i) => Some(Label::Int(*i)),
        Obj::Float(f) => Some(Label::Float(*f)),
        Obj::Str(s) => Some(Label::Str(s.clone())),
        Obj::Datetime(t) => Some(Label::Datetime(*t)),
    }
}

/// `BackendModel._check_inputs`: runs the math `checks` on the backend inputs; failed
/// `raise` checks are an error, failed `warn` checks are returned as warnings.
pub fn check_inputs(model: &ModelInputs) -> Result<Vec<String>, ModelError> {
    let ds = backend_dataset(model)?;
    let inputs = math_inputs(&ds, &model.math)?;
    let out = run_checks(&inputs, &model.math, &model.config.build)
        .map_err(|e| ModelError::new(e.to_string()))?;
    if let Some(e) = out.error_message() {
        return Err(ModelError::new(e));
    }
    Ok(out.warning_message().into_iter().collect())
}
