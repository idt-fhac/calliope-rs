//! SPORES mode (`Model._solve_spores`): a cost-optimal baseline, then `number` solves of
//! the `min_spores` objective under a cost slack, re-scoring technology capacities between
//! solves with one of four algorithms (Lombardi et al., 2023).
//!
//! Deviations (bench/BUGS.md): `tracking_parameter` tracks the techs whose value is true
//! (#2; Python tracks every tech with a value, so `false` is tracked too); `random` scoring
//! draws from numpy's legacy MT19937 with a seed (0 unless [`crate::Model::set_random_seed`]),
//! so runs are reproducible and match Python seeded with `np.random.seed(seed)`.

use std::path::Path;

use calliope_array::{Coord, LArray, Label};
use calliope_io::{ModelNetcdf, NcGroup};
use calliope_math::eval::Value;
use calliope_preprocess::xr::{Dataset, Var};
use calliope_schema::{Solve, SolveSpores, SporesScoring};
use indexmap::IndexMap;

use crate::backend::broadcast;
use crate::operate::input_num;
use crate::results::{Part, Results, concat};
use crate::{Error, Model};

fn num(v: &Value) -> Result<LArray<f64>, Error> {
    v.to_num().map_err(|e| Error::Backend(e.0))
}

/// `inputs[tracking_parameter]` (true where tracked) `& inputs.active`, over `dims`.
fn tracked(model: &Model, spores: &SolveSpores, dims: &[Coord]) -> Result<Vec<bool>, Error> {
    let ds = &model.inputs;
    let truthy = |name: &str| -> Option<LArray<f64>> {
        let v = ds.vars.get(name)?;
        let d: Vec<Coord> = v.dims.iter().map(|d| ds.coords[d].clone()).collect();
        let vals = (0..v.data.len())
            .map(|i| {
                let o = v.data.get(i);
                if o.is_null() {
                    f64::NAN
                } else {
                    f64::from(u8::from(o.truthy()))
                }
            })
            .collect();
        LArray::from_dense(d, vals).ok()
    };
    let active = truthy("active").map(|a| broadcast(&a, dims)).transpose()?;
    let param = match &spores.tracking_parameter {
        Some(p) => Some(match truthy(p) {
            Some(a) => broadcast(&a, dims)?,
            // `inputs.get(name, DataArray(True))`: a missing parameter tracks everything.
            None => LArray::scalar(1.0),
        }),
        None => None,
    };
    let full = full_cells(dims);
    Ok(full
        .iter()
        .map(|c| {
            let a = active.as_ref().is_none_or(|a| a.get(c) == 1.0);
            let p = param.as_ref().is_none_or(|p| {
                let v = if p.dims().is_empty() {
                    p.get(&[])
                } else {
                    p.get(c)
                };
                v == 1.0
            });
            a && p
        })
        .collect())
}

/// Every cell of `dims` in C order.
fn full_cells(dims: &[Coord]) -> Vec<Vec<u32>> {
    let shape: Vec<usize> = dims.iter().map(Coord::len).collect();
    let total: usize = shape.iter().product();
    let mut out = Vec::with_capacity(total);
    let mut idx = vec![0u32; dims.len()];
    for _ in 0..total {
        out.push(idx.clone());
        for i in (0..idx.len()).rev() {
            idx[i] += 1;
            if (idx[i] as usize) < shape[i] {
                break;
            }
            idx[i] = 0;
        }
    }
    out
}

fn dense(a: &LArray<f64>, dims: &[Coord]) -> Result<Vec<f64>, Error> {
    let b = broadcast(a, dims)?;
    Ok(full_cells(dims).iter().map(|c| b.get(c)).collect())
}

/// `_spores_update_model`: the new `spores_score`.
fn new_scores(
    model: &mut Model,
    all: &[Results],
    spores: &SolveSpores,
) -> Result<LArray<f64>, Error> {
    let latest = all.last().expect("baseline");
    let flow_cap = latest
        .get("flow_cap")
        .ok_or_else(|| Error::Backend("SPORES scoring needs `flow_cap` results.".into()))?
        .clone();
    let dims = flow_cap.dims().to_vec();
    let cells = full_cells(&dims);
    let techs = tracked(model, spores, &dims)?;
    let backend = model.backend.as_ref().expect("built");
    let old = dense(&num(backend.get_parameter("spores_score")?)?, &dims)?
        .into_iter()
        .map(|v| if v.is_nan() { 0.0 } else { v })
        .collect::<Vec<_>>();
    let cap = dense(&flow_cap, &dims)?;
    // `flow_cap.where(spores_techs)`.
    let prev: Vec<f64> = cap
        .iter()
        .zip(&techs)
        .map(|(&v, &t)| if t { v } else { f64::NAN })
        .collect();
    let factor = spores.score_threshold_factor;
    let integer = |prev: &[f64]| -> Vec<f64> {
        // `factor * previous_cap.max([nodes, techs])`, per remaining index.
        let key = |c: &[u32]| -> Vec<u32> {
            c.iter()
                .zip(&dims)
                .filter(|(_, d)| d.name() != "nodes" && d.name() != "techs")
                .map(|(x, _)| *x)
                .collect()
        };
        let mut maxes: IndexMap<Vec<u32>, f64> = IndexMap::new();
        for (c, &v) in cells.iter().zip(prev) {
            let m = maxes.entry(key(c)).or_insert(f64::NAN);
            if !v.is_nan() && (m.is_nan() || v > *m) {
                *m = v;
            }
        }
        cells
            .iter()
            .zip(prev)
            .zip(&techs)
            .zip(&old)
            .map(|(((c, &v), &t), &o)| {
                let min_relevant = factor * maxes[&key(c)];
                let s = if v > min_relevant { 1000.0 } else { 0.0 };
                if t { s + o } else { f64::NAN }
            })
            .collect()
    };
    let scores: Vec<f64> = match spores.scoring_algorithm {
        SporesScoring::Integer => integer(&prev),
        SporesScoring::RelativeDeployment => {
            let max = input_num(&model.inputs, "flow_cap_max");
            let Some(max) = max else {
                return Err(Error::Backend("Cannot score SPORES with `relative_deployment` when `flow_cap_max` is undefined for some or all tracked technologies.".into()));
            };
            let max = dense(&max, &dims)?;
            if max
                .iter()
                .zip(&techs)
                .any(|(&m, &t)| t && m == f64::INFINITY)
            {
                return Err(Error::Backend("Cannot score SPORES with `relative_deployment` when `flow_cap_max` is undefined for some or all tracked technologies.".into()));
            }
            cap.iter()
                .zip(&max)
                .zip(&techs)
                .zip(&old)
                .map(|(((&c, &m), &t), &o)| {
                    let rel = c / m;
                    let s = if rel > factor { rel } else { 0.0 };
                    if t { s + o } else { f64::NAN }
                })
                .collect()
        }
        SporesScoring::Random => {
            // One draw per cell of the full array, in C order (`np.random.choice`).
            let draws: Vec<f64> = (0..prev.len())
                .map(|_| model.rng.choice2(0.0, 1000.0))
                .collect();
            prev.iter()
                .zip(&draws)
                .zip(&techs)
                .zip(&old)
                .map(|(((&p, &d), &t), &o)| {
                    let s = if p.is_nan() { 0.0 } else { d };
                    if t { s + o } else { f64::NAN }
                })
                .collect()
        }
        SporesScoring::EvolvingAverage => {
            let n = all.len();
            let mut avg = vec![0.0; cap.len()];
            for r in all {
                let fc = r.get("flow_cap").map(|a| dense(a, &dims)).transpose()?;
                for (s, v) in avg
                    .iter_mut()
                    .zip(fc.unwrap_or_else(|| vec![f64::NAN; cap.len()]))
                {
                    *s += v;
                }
            }
            #[allow(clippy::cast_precision_loss)]
            let avg: Vec<f64> = avg.iter().map(|s| s / n as f64).collect();
            let change: Vec<f64> = avg
                .iter()
                .zip(&cap)
                .map(|(&a, &c)| (a - c).abs() / a)
                .collect();
            let total: f64 = change.iter().filter(|v| !v.is_nan()).sum();
            if total == 0.0 {
                integer(&prev)
            } else {
                change
                    .iter()
                    .zip(&techs)
                    .map(|(&r, &t)| {
                        let cleaned = if r.is_nan() || !t { 0.0 } else { r.max(0.001) };
                        if cleaned > 0.0 { 1.0 / cleaned } else { 0.0 }
                    })
                    .collect()
            }
        }
    };
    Ok(LArray::from_dense(dims, scores)?)
}

/// `_spores_save_model`: one NetCDF per SPORE (`spore_<n>.nc`, inputs in `spore_0`).
fn save_spore(
    model: &Model,
    results: &Results,
    path: Option<&Path>,
    spore: i64,
) -> Result<(), Error> {
    let Some(dir) = path else { return Ok(()) };
    if !matches!(
        results.termination_condition.as_str(),
        "optimal" | "feasible"
    ) {
        tracing::info!(
            "Optimisation model | Infeasible or unbounded problem | No SPORE {spore} results to save to file."
        );
        return Ok(());
    }
    std::fs::create_dir_all(dir).map_err(|e| {
        Error::Io(calliope_io::IoError::Format(format!(
            "{}: {e}",
            dir.display()
        )))
    })?;
    tracing::info!("Optimisation model | Saving SPORE {spore} to file.");
    let coord = Coord::new("spores", vec![Label::Int(spore)])?;
    let expanded = concat(
        &[Part {
            labels: vec![Label::Int(spore)],
            arrays: &results.arrays,
        }],
        "spores",
        coord.labels().to_vec(),
    )?;
    let inputs = if spore == 0 {
        expand_inputs(&model.inputs, &coord)
    } else {
        Dataset::new()
    };
    let nc = ModelNetcdf {
        inputs: NcGroup {
            ds: inputs,
            ..NcGroup::default()
        },
        results: NcGroup {
            ds: calliope_io::results_dataset(&expanded)?,
            ..NcGroup::default()
        },
        attrs: model.dump_all_attrs(),
    };
    calliope_io::write_netcdf(&dir.join(format!("spore_{spore}.nc")), &nc)?;
    Ok(())
}

/// `inputs.expand_dims(spores=[n])`.
fn expand_inputs(ds: &Dataset, coord: &Coord) -> Dataset {
    let mut out = Dataset::new();
    out.coords.insert("spores".into(), coord.clone());
    for (k, c) in &ds.coords {
        out.coords.insert(k.clone(), c.clone());
    }
    for name in ds.vars.keys() {
        let da = ds.get(name).expect("exists").expand_dims(coord.clone());
        let mut dims = vec!["spores".to_owned()];
        dims.extend(ds.vars[name].dims.iter().cloned());
        out.vars.insert(
            name.clone(),
            Var {
                dims,
                data: da.data,
            },
        );
    }
    out
}

/// `_solve_spores`.
pub(crate) fn solve_spores(model: &mut Model, solve: &Solve) -> Result<Results, Error> {
    let spores = solve.spores.clone();
    let save_path = spores.save_per_spore_path.clone();
    tracing::info!("Optimisation model | Resetting SPORES parameters.");
    let default_score = model
        .math
        .parameters
        .get("spores_score")
        .map_or(0.0, |p| calliope_math::eval::inputs::numeric(&p.default));
    let objective = model.config.build.objective.clone();
    model
        .backend
        .as_mut()
        .expect("built")
        .update_input("spores_score", &Value::num(default_score))?;
    let mut latest: i64 = 0;
    let baseline = if !spores.use_latest_results {
        tracing::info!("Optimisation model | Running baseline model.");
        let b = model.backend.as_mut().expect("built");
        b.set_objective(&objective)?;
        let r = b.solve(solve, false)?;
        save_spore(model, &r, save_path.as_deref(), 0)?;
        r
    } else if let Some(sp) = model
        .results
        .arrays
        .values()
        .find_map(|a| a.coord("spores").cloned())
    {
        latest = sp
            .labels()
            .iter()
            .filter_map(|l| match l {
                Label::Int(i) => Some(*i),
                _ => None,
            })
            .max()
            .unwrap_or(0);
        tracing::info!("Optimisation model | Restarting SPORES from SPORE {latest} results.");
        let mut arrays = IndexMap::new();
        for (k, a) in &model.results.arrays {
            let v = if a.coord("spores").is_some() {
                a.sel_one("spores", &Label::Int(latest))?
            } else {
                a.clone()
            };
            arrays.insert(k.clone(), v);
        }
        let r = Results {
            arrays,
            termination_condition: model.results.termination_condition.clone(),
        };
        if let Some(cum) = r.get("spores_score_cumulative") {
            model
                .backend
                .as_mut()
                .expect("built")
                .update_input("spores_score", &Value::Num(cum.clone()))?;
        }
        r
    } else {
        tracing::info!("Optimisation model | Using existing baseline model results.");
        model.results.clone()
    };
    if latest >= spores.number {
        return Err(Error::Model(format!(
            "Cannot restart SPORES from SPORE {latest} as it is greater or equal to the configured number of SPORES to run ({}).",
            spores.number
        )));
    }
    if baseline.is_empty() {
        return Err(Error::Model("Cannot run SPORES without baseline results. This issue may be caused by an infeasible baseline model.Ensure your baseline model can solve successfully by running it in `base` mode.".into()));
    }
    let base_default = model
        .math
        .parameters
        .get("spores_baseline_cost")
        .map_or(f64::INFINITY, |p| {
            calliope_math::eval::inputs::numeric(&p.default)
        });
    let tracked = baseline
        .get("spores_baseline_cost_tracked")
        .map(|a| a.get(&[]))
        .or_else(|| input_num(&model.inputs, "spores_baseline_cost").map(|a| a.get(&[])));
    let constraining = match tracked {
        Some(c) if c != 0.0 && !c.is_nan() && c != base_default => c,
        _ => baseline.get(&objective).map_or(f64::NAN, |a| a.get(&[])),
    };
    let b = model.backend.as_mut().expect("built");
    b.update_input("spores_baseline_cost", &Value::num(constraining))?;
    b.set_objective("min_spores")?;
    tracing::info!(
        "Optimisation model | Running SPORES with `{:?}` scoring algorithm.",
        spores.scoring_algorithm
    );
    let mut all = vec![baseline];
    let mut labels = vec![Label::Int(latest)];
    for spore in latest + 1..=spores.number {
        tracing::info!("Optimisation model | Running SPORE {spore}.");
        let score = new_scores(model, &all, &spores)?;
        let b = model.backend.as_mut().expect("built");
        b.update_input("spores_score", &Value::Num(score))?;
        let r = b.solve(solve, false)?;
        save_spore(model, &r, save_path.as_deref(), spore)?;
        if r.is_empty() {
            tracing::warn!("Stopping SPORES run after SPORE {spore} due to model infeasibility.");
            break;
        }
        all.push(r);
        labels.push(Label::Int(spore));
    }
    let parts: Vec<Part<'_>> = all
        .iter()
        .zip(&labels)
        .map(|(r, l)| Part {
            labels: vec![l.clone()],
            arrays: &r.arrays,
        })
        .collect();
    let mut arrays = concat(&parts, "spores", labels.clone())?;
    if latest > 0 && spores.use_latest_results {
        let prev_labels: Vec<Label> = model
            .results
            .arrays
            .values()
            .find_map(|a| a.coord("spores").map(|c| c.labels().to_vec()))
            .unwrap_or_default();
        let new_labels: Vec<Label> = labels[1..].to_vec();
        let mut all_labels = prev_labels.clone();
        all_labels.extend(new_labels.iter().cloned());
        arrays = concat(
            &[
                Part {
                    labels: prev_labels,
                    arrays: &model.results.arrays,
                },
                Part {
                    labels: new_labels,
                    arrays: &arrays,
                },
            ],
            "spores",
            all_labels,
        )?;
    }
    let mut terms: Vec<String> = all
        .iter()
        .map(|r| r.termination_condition.clone())
        .filter(|t| !t.is_empty())
        .collect();
    terms.sort();
    terms.dedup();
    Ok(Results {
        arrays,
        termination_condition: terms.join(","),
    })
}
