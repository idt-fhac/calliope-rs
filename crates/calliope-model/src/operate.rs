//! Operate (rolling-horizon dispatch) mode (`Model._solve_operate`).
//!
//! The time series is cut into windows that start every `window` and each cover
//! `horizon`; each window is solved in turn and only its first `window` of results is
//! kept, except for the last window, which keeps everything. Storage levels are carried
//! from one window to the next (`_recalculate_storage_initial`).
//!
//! Calliope keeps one backend and updates every changed time series input per window
//! (rebuilding all referencing components), and rebuilds for the shorter last horizon. The
//! port builds each window's problem directly from the sliced inputs, which gives the same
//! problem (the LP names carry the window's own timestamps, where Calliope keeps the first
//! window's) at a fraction of the cost of the updates.
//!
//! Deviations (bench/BUGS.md): a single window (window ≥ the time span) is solved and
//! returned (#1, Python raises `UnboundLocalError`); `horizon < window` is rejected (#8,
//! Python silently drops the uncovered timesteps).

use calliope_array::{LArray, Label};
use calliope_math::eval::Value;
use calliope_preprocess::model_data::drop_undefined;
use calliope_preprocess::time::{Freq, parse_freq};
use calliope_preprocess::xr::Dataset;
use calliope_schema::Solve;
use indexmap::IndexMap;

use crate::backend::broadcast;
use crate::results::{Part, Results, concat};
use crate::{Error, Model};

/// Result arrays by name.
type Arrays = IndexMap<String, LArray<f64>>;

/// One window: its first timestep and its last (inclusive) horizon timestep, in ns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Window {
    pub start: i64,
    pub end: i64,
}

fn tick(s: &str, what: &str) -> Result<i64, Error> {
    match parse_freq(s) {
        Some(Freq::Tick(ns)) if ns > 0 => Ok(ns),
        _ => Err(Error::Model(format!(
            "Invalid operate {what} `{s}`: expected a fixed frequency such as `24h`."
        ))),
    }
}

fn timesteps(ds: &Dataset) -> Result<Vec<i64>, Error> {
    let c = ds
        .coord("timesteps")
        .ok_or_else(|| Error::Model("Operate mode needs a `timesteps` dimension.".into()))?;
    c.labels()
        .iter()
        .map(|l| match l {
            Label::Datetime(t) => Ok(*t),
            _ => Err(Error::Model(
                "Operate mode needs datetime timesteps.".into(),
            )),
        })
        .collect()
}

/// `windowsteps` and `horizonsteps` (`_prepare_operate_mode_inputs`).
pub(crate) fn windows(model: &Model) -> Result<Vec<Window>, Error> {
    if model.config.init.time_cluster.is_some() {
        return Err(Error::Model(
            "Unable to run this model in operate (i.e. dispatch) mode because time clustering is in use".into(),
        ));
    }
    let op = &model.config.build.operate;
    let window = tick(&op.window, "window")?;
    let horizon = tick(&op.horizon, "horizon")?;
    if horizon < window {
        return Err(Error::Model(format!(
            "Operate mode horizon (`{}`) must be at least as long as its window (`{}`).",
            op.horizon, op.window
        )));
    }
    let ts = timesteps(&model.inputs)?;
    let (Some(&first), Some(&last)) = (ts.first(), ts.last()) else {
        return Err(Error::Model(
            "Operate mode needs at least one timestep.".into(),
        ));
    };
    let mut out = Vec::new();
    let mut start = first;
    while start <= last {
        out.push(Window {
            start,
            end: (start + horizon).min(last + 1) - 1,
        });
        start += window;
    }
    Ok(out)
}

/// The inputs of window `idx`, cleaned of undefined members.
pub(crate) fn window_dataset(model: &Model, idx: usize) -> Result<Dataset, Error> {
    let w = *windows(model)?
        .get(idx)
        .ok_or_else(|| Error::Model(format!("No operate window {idx}")))?;
    let mut ds = model.inputs.clone();
    let keep: Vec<Label> = timesteps(&ds)?
        .into_iter()
        .filter(|t| *t >= w.start && *t <= w.end)
        .map(Label::Datetime)
        .collect();
    ds.isel_labels("timesteps", keep);
    let active = ds
        .get("active")
        .ok_or_else(|| Error::Model("The input data has no `active` array".into()))?;
    Ok(drop_undefined(ds, &active))
}

/// A numeric input of the model as an array (NaN where missing).
pub(crate) fn input_num(ds: &Dataset, name: &str) -> Option<LArray<f64>> {
    let v = ds.vars.get(name)?;
    let dims: Vec<_> = v.dims.iter().map(|d| ds.coords[d].clone()).collect();
    let vals: Vec<f64> = (0..v.data.len())
        .map(|i| v.data.get(i).as_f64().unwrap_or(f64::NAN))
        .collect();
    LArray::from_dense(dims, vals).ok()
}

/// `_recalculate_storage_initial`: the storage level at the last kept timestep relative
/// to the storage capacity (NaN where the capacity is not positive).
fn storage_initial(
    model: &Model,
    kept: &IndexMap<String, LArray<f64>>,
) -> Result<Option<LArray<f64>>, Error> {
    let Some(storage) = kept.get("storage") else {
        return Ok(None);
    };
    if storage.count() == 0 {
        return Ok(None);
    }
    let ts = storage
        .coord("timesteps")
        .ok_or_else(|| Error::Model("`storage` has no timesteps".into()))?;
    let last = ts.label(u32::try_from(ts.len() - 1).expect("fits")).clone();
    let end = storage.sel_one("timesteps", &last)?;
    let dims = end.dims().to_vec();
    let cap = match input_num(&model.inputs, "storage_cap") {
        Some(c) => broadcast(&c, &dims)?,
        None => LArray::empty(dims.clone())?,
    };
    let cells: Vec<(Vec<u32>, f64)> = end
        .cells()
        .filter_map(|(c, v)| {
            let k = cap.get(&c);
            (k > 0.0).then(|| (c, v / k))
        })
        .collect();
    Ok(Some(LArray::from_cells(dims, cells)?))
}

/// `_solve_operate`.
pub(crate) fn solve_operate(model: &mut Model, solve: &Solve) -> Result<Results, Error> {
    let wins = windows(model)?;
    if model.start_window_idx != 0 || model.backend.is_none() {
        tracing::info!("Optimisation model | Resetting model to first time window.");
        model.start_window_idx = 0;
        let ds = window_dataset(model, 0)?;
        model.backend = Some(model.make_backend(&ds, None)?);
    }
    tracing::info!("Optimisation model | Running first time window.");
    let mut results = model.backend.as_mut().expect("built").solve(solve, false)?;
    // Per window: the timesteps kept and the results at them.
    let mut kept: Vec<(Vec<Label>, Arrays)> = Vec::new();
    let mut terminations = vec![results.termination_condition.clone()];
    let mut cur_labels = ds_labels(&window_dataset(model, 0)?);
    let mut window_len = cur_labels.len();
    // The `storage_initial` the backend holds (Calliope's update path keeps the last
    // update; a rebuild starts from the inputs again).
    let mut held_storage_initial: Option<Value> = None;
    for (idx, w) in wins.iter().enumerate().skip(1) {
        model.start_window_idx = idx;
        tracing::info!(
            "Optimisation model | Running time window starting at {}.",
            calliope_preprocess::xr::format_datetime(w.start)
        );
        let prev_labels: Vec<Label> = cur_labels
            .iter()
            .filter(|l| matches!(l, Label::Datetime(t) if *t < w.start))
            .cloned()
            .collect();
        let prev = slice(&results, &prev_labels);
        let ds = window_dataset(model, idx)?;
        cur_labels = ds_labels(&ds);
        let n = cur_labels.len();
        let base = calliope_preprocess::backend::math_inputs(&ds, &model.math)?;
        if n != window_len {
            tracing::info!(
                "Optimisation model | Reaching the end of the timeseries. Re-building model with shorter time horizon."
            );
            held_storage_initial = None;
        }
        window_len = n;
        if let Some(new) = storage_initial(model, &prev)? {
            let old = held_storage_initial
                .clone()
                .or_else(|| base.get("storage_initial").cloned())
                .unwrap_or_else(|| Value::num(f64::NAN));
            let merged = crate::backend::merge_update(&old, &Value::Num(new), &base.coords)?;
            held_storage_initial = Some(merged);
        }
        kept.push((prev_labels, prev));
        model.backend = Some(model.make_backend(&ds, held_storage_initial.as_ref())?);
        results = model.backend.as_mut().expect("built").solve(solve, false)?;
        terminations.push(results.termination_condition.clone());
    }
    let last_start = wins.last().map_or(i64::MIN, |w| w.start);
    let last_labels: Vec<Label> = cur_labels
        .iter()
        .filter(|l| matches!(l, Label::Datetime(t) if *t >= last_start))
        .cloned()
        .collect();
    let last = slice(&results, &last_labels);
    kept.push((last_labels, last));
    let mut all_labels = Vec::new();
    for (l, _) in &kept {
        all_labels.extend(l.iter().cloned());
    }
    let parts: Vec<Part<'_>> = kept
        .iter()
        .map(|(labels, arrays)| Part {
            labels: labels.clone(),
            arrays,
        })
        .collect();
    let arrays = concat(&parts, "timesteps", all_labels)?;
    terminations.sort();
    terminations.dedup();
    Ok(Results {
        arrays,
        termination_condition: terminations.join(","),
    })
}

fn ds_labels(ds: &Dataset) -> Vec<Label> {
    ds.coord("timesteps")
        .map(|c| c.labels().to_vec())
        .unwrap_or_default()
}

fn slice(results: &Results, labels: &[Label]) -> IndexMap<String, LArray<f64>> {
    crate::results::select_labels(&results.arrays, "timesteps", labels)
        .unwrap_or_else(|_| results.arrays.clone())
}
