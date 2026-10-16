//! `Dataset`: named arrays sharing one coordinate per dimension.

use calliope_array::{Coord, Label};
use indexmap::IndexMap;

use super::array::{DataArray, Join, join_coords};
use super::data::{DType, Data};

/// A data variable: its dimension names and values (C order over the dataset coordinates).
#[derive(Debug, Clone, PartialEq)]
pub struct Var {
    /// Dimension names, in order.
    pub dims: Vec<String>,
    /// Values.
    pub data: Data,
}

/// How variables that appear in several merged datasets are combined.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compat {
    /// The first one wins.
    Override,
    /// Values are combined; non-missing values must agree.
    NoConflicts,
}

/// An xarray-like dataset (dimension coordinates only).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Dataset {
    /// One coordinate per dimension, in order of creation.
    pub coords: IndexMap<String, Coord>,
    /// Data variables, in order.
    pub vars: IndexMap<String, Var>,
}

impl Dataset {
    /// An empty dataset.
    pub fn new() -> Self {
        Self::default()
    }

    /// True if there are no variables and no coordinates.
    pub fn is_empty(&self) -> bool {
        self.vars.is_empty() && self.coords.is_empty()
    }

    /// A variable as an array.
    pub fn get(&self, name: &str) -> Option<DataArray> {
        let v = self.vars.get(name)?;
        Some(DataArray::new(
            v.dims.iter().map(|d| self.coords[d].clone()).collect(),
            v.data.clone(),
        ))
    }

    /// True if the variable exists.
    pub fn contains(&self, name: &str) -> bool {
        self.vars.contains_key(name)
    }

    /// The coordinate of a dimension.
    pub fn coord(&self, dim: &str) -> Option<&Coord> {
        self.coords.get(dim)
    }

    /// `ds[name] = da`: existing dimensions keep the dataset's labels (the array is
    /// reindexed to them); new dimensions are added.
    pub fn set(&mut self, name: &str, da: DataArray) {
        let dst: Vec<Coord> = da
            .coords
            .iter()
            .map(|c| {
                self.coords
                    .get(c.name())
                    .cloned()
                    .unwrap_or_else(|| c.clone())
            })
            .collect();
        let da = da.conform_into(&dst, DType::Float);
        for c in &dst {
            self.coords
                .entry(c.name().to_owned())
                .or_insert_with(|| c.clone());
        }
        let dims = dst.iter().map(|c| c.name().to_owned()).collect();
        if let Some(v) = self.vars.get_mut(name) {
            *v = Var {
                dims,
                data: da.data,
            };
        } else {
            self.vars.insert(
                name.to_owned(),
                Var {
                    dims,
                    data: da.data,
                },
            );
        }
    }

    /// A variable as an array, moved out: the variable keeps its place (and dims) but holds
    /// no values until [`Dataset::set`] puts an array back.
    pub fn take(&mut self, name: &str) -> Option<DataArray> {
        let v = self.vars.get_mut(name)?;
        let data = std::mem::replace(&mut v.data, Data::Float(Vec::new()));
        Some(DataArray::new(
            v.dims.iter().map(|d| self.coords[d].clone()).collect(),
            data,
        ))
    }

    /// Removes a variable.
    pub fn remove(&mut self, name: &str) -> Option<Var> {
        self.vars.shift_remove(name)
    }

    /// Replaces the labels of a dimension coordinate positionally (`assign_coords` with
    /// values of the same length), or adds it.
    pub fn assign_coord(&mut self, coord: Coord) {
        if let Some(c) = self.coords.get_mut(coord.name()) {
            debug_assert_eq!(c.len(), coord.len());
            *c = coord;
        } else {
            self.coords.insert(coord.name().to_owned(), coord);
        }
    }

    /// Names of variables with exactly (or at least) these dimensions.
    pub fn vars_with_dims(&self, dims: &[&str], exact: bool) -> Vec<String> {
        self.vars
            .iter()
            .filter(|(_, v)| {
                if exact {
                    v.dims.len() == dims.len() && dims.iter().all(|d| v.dims.iter().any(|x| x == d))
                } else {
                    dims.iter().all(|d| v.dims.iter().any(|x| x == d))
                }
            })
            .map(|(k, _)| k.clone())
            .collect()
    }

    /// Keeps only the given labels along `dim`, in that order, for all variables.
    pub fn isel_labels(&mut self, dim: &str, labels: Vec<Label>) {
        let Some(old) = self.coords.get(dim).cloned() else {
            return;
        };
        let new = Coord::new(dim, labels).expect("selected labels are unique");
        if new == old {
            return;
        }
        let names: Vec<String> = self.vars.keys().cloned().collect();
        for name in names {
            if self.vars[&name].dims.iter().any(|d| d == dim) {
                let da = self.take(&name).expect("exists").reindex_into(&new);
                self.vars[&name].data = da.data;
            }
        }
        self.coords.insert(dim.to_owned(), new);
    }

    /// `dropna(dim, how="all", subset=[var])`: drops labels of `dim` where `var` is
    /// missing everywhere.
    pub fn dropna(&mut self, dim: &str, var: &str) {
        let Some(da) = self.get(var) else { return };
        let Some(coord) = self.coords.get(dim).cloned() else {
            return;
        };
        if !da.has_dim(dim) {
            if da.all_null() {
                self.isel_labels(dim, Vec::new());
            }
            return;
        }
        let others: Vec<&str> = da.dims().into_iter().filter(|d| *d != dim).collect();
        let keep = da.notnull().any(&others);
        let mask = keep.truthy();
        let labels: Vec<Label> = coord
            .labels()
            .iter()
            .zip(mask)
            .filter(|(_, m)| *m)
            .map(|(l, _)| l.clone())
            .collect();
        self.isel_labels(dim, labels);
    }

    /// `xr.merge(datasets, join=join, compat=compat)`.
    pub fn merge(datasets: &[&Dataset], join: Join, compat: Compat) -> Result<Dataset, String> {
        let mut coords: IndexMap<String, Coord> = IndexMap::new();
        for ds in datasets {
            for (name, c) in &ds.coords {
                match coords.get(name) {
                    Some(prev) => {
                        let j = join_coords(prev, c, join);
                        coords.insert(name.clone(), j);
                    }
                    None => {
                        coords.insert(name.clone(), c.clone());
                    }
                }
            }
        }
        let mut out = Dataset {
            coords: coords.clone(),
            vars: IndexMap::new(),
        };
        let mut order: Vec<String> = Vec::new();
        for ds in datasets {
            for name in ds.vars.keys() {
                if !order.contains(name) {
                    order.push(name.clone());
                }
            }
        }
        for name in order {
            let mut acc: Option<DataArray> = None;
            for ds in datasets {
                let Some(da) = ds.get(&name) else { continue };
                let dst: Vec<Coord> = da.coords.iter().map(|c| coords[c.name()].clone()).collect();
                let da = da.conform(&dst, DType::Float);
                acc = Some(match acc {
                    None => da,
                    Some(prev) => match compat {
                        Compat::Override => prev,
                        Compat::NoConflicts => {
                            check_conflicts(&name, &prev, &da)?;
                            prev.fillna(&da)
                        }
                    },
                });
            }
            if let Some(da) = acc {
                out.set(&name, da);
            }
        }
        Ok(out)
    }

    /// `Dataset.fillna(other)`: missing values of shared variables are taken from `other`.
    pub fn fillna(mut self, other: &Dataset) -> Dataset {
        for name in self.names() {
            if let Some(o) = other.get(&name) {
                let a = self.take(&name).expect("exists");
                self.set(&name, a.fillna_into(&o));
            }
        }
        self
    }

    /// `xr.merge([new, self], join="outer", compat="override").fillna(self)`, in place:
    /// variables of `new` come first and win, their missing values are filled from `self`;
    /// other variables are reindexed to the joined coordinates only where these changed.
    pub fn merge_over(&mut self, new: Dataset) {
        let mut coords: IndexMap<String, Coord> = new.coords.clone();
        for (name, c) in &self.coords {
            let j = match coords.get(name) {
                Some(prev) => join_coords(prev, c, Join::Outer),
                None => c.clone(),
            };
            coords.insert(name.clone(), j);
        }
        let mut old = std::mem::take(&mut self.vars);
        let old_coords = std::mem::replace(&mut self.coords, coords);
        let dims_of = |dims: &[String], coords: &IndexMap<String, Coord>| -> Vec<Coord> {
            dims.iter().map(|d| coords[d].clone()).collect()
        };
        let mut new = new;
        let new_names = new.names();
        for name in &new_names {
            let v = new.vars.shift_remove(name).expect("listed");
            let da = DataArray::new(dims_of(&v.dims, &new.coords), v.data);
            let da = da.conform_into(&dims_of(&v.dims, &self.coords), DType::Float);
            let filled = match old.shift_remove(name) {
                Some(o) => {
                    let o = DataArray::new(dims_of(&o.dims, &old_coords), o.data);
                    da.fillna_into(&o)
                }
                None => da,
            };
            self.set(name, filled);
        }
        for (name, v) in old {
            let src = dims_of(&v.dims, &old_coords);
            let dst = dims_of(&v.dims, &self.coords);
            let data = super::array::conform_data(v.data, &src, &dst, DType::Float);
            self.vars.insert(name, Var { dims: v.dims, data });
        }
    }

    /// Data variable names.
    pub fn names(&self) -> Vec<String> {
        self.vars.keys().cloned().collect()
    }
}

fn check_conflicts(name: &str, a: &DataArray, b: &DataArray) -> Result<(), String> {
    let (_, a2, b2) = DataArray::broadcast_pair(a, b, Join::Inner);
    for i in 0..a2.data.len() {
        if !a2.data.is_null(i) && !b2.data.is_null(i) && !a2.data.get(i).py_eq(&b2.data.get(i)) {
            return Err(format!(
                "conflicting values for variable '{name}' on objects to be combined. \
                 You can skip this check by specifying compat='override'."
            ));
        }
    }
    Ok(())
}
