//! Results: solved values as labelled arrays, `xr.concat` of the per-window (operate) and
//! per-SPORE results, and conversion from and to datasets.

use calliope_array::{ArrayError, Coord, LArray, Label};
use calliope_math::eval::Lin;
use calliope_preprocess::xr::{Data, Dataset};
use indexmap::IndexMap;

use crate::Error;

/// Results of a solve (`model.results`), arrays in Python's order.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Results {
    /// Result arrays (float, NaN where undefined).
    pub arrays: IndexMap<String, LArray<f64>>,
    /// Calliope's termination condition.
    pub termination_condition: String,
}

impl Results {
    /// True if there are no result arrays (`not model.results`).
    pub fn is_empty(&self) -> bool {
        self.arrays.is_empty()
    }

    /// An array by name.
    pub fn get(&self, name: &str) -> Option<&LArray<f64>> {
        self.arrays.get(name)
    }

    /// The results as a dense dataset (coordinates shared by name).
    pub fn to_dataset(&self) -> Result<Dataset, Error> {
        Ok(calliope_io::results_dataset(&self.arrays)?)
    }

    /// Results from a dataset (e.g. a `results` group read from NetCDF).
    pub fn from_dataset(ds: &Dataset, termination_condition: String) -> Result<Self, Error> {
        let mut arrays = IndexMap::new();
        for (name, var) in &ds.vars {
            let dims: Vec<Coord> = var.dims.iter().map(|d| ds.coords[d].clone()).collect();
            let values: Vec<f64> = match &var.data {
                Data::Float(v) => v.clone(),
                #[allow(clippy::cast_precision_loss)]
                Data::Int(v) => v.iter().map(|i| *i as f64).collect(),
                Data::Bool(v) => v.iter().map(|b| f64::from(u8::from(*b))).collect(),
                other => {
                    return Err(Error::Model(format!(
                        "results array `{name}` has non-numeric dtype {}",
                        other.dtype().name()
                    )));
                }
            };
            arrays.insert(name.clone(), LArray::from_dense(dims, values)?);
        }
        Ok(Self {
            arrays,
            termination_condition,
        })
    }
}

/// The value of every cell of an affine array at the primal solution `x` (missing cells
/// stay NaN).
pub fn lin_values(a: &LArray<Lin>, x: &[f64]) -> LArray<f64> {
    a.map(|l| {
        if l.terms.is_empty() {
            l.constant
        } else {
            l.constant + l.terms.iter().map(|&(c, k)| k * x[c as usize]).sum::<f64>()
        }
    })
}

/// `_apply_zero_threshold`: `|v| < threshold` → 0.
pub fn apply_zero_threshold(a: &LArray<f64>, threshold: f64) -> LArray<f64> {
    a.map(|&v| if v.abs() < threshold { 0.0 } else { v })
}

/// One dataset of an `xr.concat` and the labels of the concatenation dimension it covers.
#[derive(Debug, Clone)]
pub struct Part<'a> {
    /// Labels along the concatenation dimension (one for a new dimension).
    pub labels: Vec<Label>,
    /// The arrays (with the concatenation dimension sliced to `labels`, if they have it).
    pub arrays: &'a IndexMap<String, LArray<f64>>,
}

/// `xr.concat(parts, dim, data_vars="all")`: arrays that have `dim` keep their dimension
/// order and are taken at each part's labels; arrays without it get `dim` as first
/// dimension and are broadcast along the part's labels; an array missing from a part is
/// NaN there. Other dimensions are outer-joined.
pub fn concat(
    parts: &[Part<'_>],
    dim: &str,
    coord_labels: Vec<Label>,
) -> Result<IndexMap<String, LArray<f64>>, Error> {
    let concat_coord = Coord::new(dim, coord_labels)?;
    let mut names: Vec<&String> = Vec::new();
    for p in parts {
        for n in p.arrays.keys() {
            if !names.contains(&n) {
                names.push(n);
            }
        }
    }
    let mut out = IndexMap::new();
    for name in names {
        let present: Vec<&LArray<f64>> = parts.iter().filter_map(|p| p.arrays.get(name)).collect();
        // Target dims: the first array's dims, with `dim` prepended if it lacks it.
        let first = present[0];
        let mut dims: Vec<Coord> = Vec::new();
        if first.coord(dim).is_none() {
            dims.push(concat_coord.clone());
        }
        for d in first.dims() {
            if d.name() == dim {
                dims.push(concat_coord.clone());
            } else {
                let mut c = d.clone();
                for other in &present[1..] {
                    if let Some(o) = other.coord(d.name())
                        && *o != c
                    {
                        c = c.join(o, calliope_array::Join::Outer);
                    }
                }
                dims.push(c);
            }
        }
        let axis = dims.iter().position(|d| d.name() == dim).expect("added");
        let mut cells: Vec<(Vec<u32>, f64)> = Vec::new();
        for p in parts {
            let Some(a) = p.arrays.get(name) else {
                continue;
            };
            let targets: Vec<u32> = p
                .labels
                .iter()
                .map(|l| concat_coord.code(l).ok_or_else(|| missing_label(dim, l)))
                .collect::<Result<_, _>>()?;
            // Map each array axis to (target axis, code map).
            let mut maps: Vec<(usize, Vec<Option<u32>>)> = Vec::new();
            for d in a.dims() {
                let t = dims
                    .iter()
                    .position(|x| x.name() == d.name())
                    .ok_or_else(|| {
                        Error::Model(format!(
                            "`{name}` has different dims in concatenated results"
                        ))
                    })?;
                let m: Vec<Option<u32>> = if d.name() == dim {
                    d.labels()
                        .iter()
                        .map(|l| p.labels.contains(l).then(|| concat_coord.code(l)).flatten())
                        .collect()
                } else {
                    d.labels().iter().map(|l| dims[t].code(l)).collect()
                };
                maps.push((t, m));
            }
            let has_dim = a.coord(dim).is_some();
            for (codes, v) in a.cells() {
                if v.is_nan() {
                    continue;
                }
                let mut target = vec![0u32; dims.len()];
                let mut ok = true;
                for ((t, m), c) in maps.iter().zip(&codes) {
                    match m[*c as usize] {
                        Some(x) => target[*t] = x,
                        None => ok = false,
                    }
                }
                if !ok {
                    continue;
                }
                if has_dim {
                    cells.push((target, *v));
                } else {
                    for &t in &targets {
                        let mut c = target.clone();
                        c[axis] = t;
                        cells.push((c, *v));
                    }
                }
            }
        }
        out.insert(name.clone(), LArray::from_cells(dims, cells)?);
    }
    Ok(out)
}

fn missing_label(dim: &str, l: &Label) -> Error {
    Error::Model(format!("`{l:?}` is not a label of `{dim}`"))
}

/// `da.sel({dim: labels})` for every array that has `dim` (others are kept as they are).
pub fn select_labels(
    arrays: &IndexMap<String, LArray<f64>>,
    dim: &str,
    labels: &[Label],
) -> Result<IndexMap<String, LArray<f64>>, ArrayError> {
    let mut out = IndexMap::new();
    for (n, a) in arrays {
        let v = if a.coord(dim).is_some() {
            a.sel(dim, &calliope_array::Selection::Labels(labels.to_vec()))?
        } else {
            a.clone()
        };
        out.insert(n.clone(), v);
    }
    Ok(out)
}
