//! Candidate A: dense `ndarray` over the full product of coordinates (xarray's layout).
//! Affine expressions are COO triples keyed by the flat index of their cell.

use std::collections::HashMap;
use std::time::Instant;

use ndarray::{ArrayD, ArrayViewD, Axis, IxDyn, Zip};

use crate::{Csc, Model};

type Dim = &'static str;

/// Dimension sizes. All arrays share the labels of a dimension (outer-join alignment of label
/// sets is the same code in both candidates and left out of the spike).
#[derive(Clone)]
pub struct Sizes(HashMap<Dim, usize>);

#[derive(Clone, Debug)]
pub struct LArray<T> {
    pub dims: Vec<Dim>,
    pub data: ArrayD<T>,
}

fn union(a: &[Dim], b: &[Dim]) -> Vec<Dim> {
    let mut out = a.to_vec();
    out.extend(b.iter().filter(|d| !a.contains(d)));
    out
}

impl<T: Clone> LArray<T> {
    pub fn new(dims: Vec<Dim>, sizes: &Sizes, fill: T) -> Self {
        let shape: Vec<usize> = dims.iter().map(|d| sizes.0[d]).collect();
        Self {
            dims,
            data: ArrayD::from_elem(IxDyn(&shape), fill),
        }
    }

    /// Calls `f` with a broadcast view in the order of `dims` (a superset of `self.dims`).
    pub fn view_as<R>(
        &self,
        dims: &[Dim],
        sizes: &Sizes,
        f: impl FnOnce(ArrayViewD<'_, T>) -> R,
    ) -> R {
        let mut order: Vec<usize> = (0..self.dims.len()).collect();
        order.sort_by_key(|&i| dims.iter().position(|d| *d == self.dims[i]));
        let mut v = self.data.view().permuted_axes(IxDyn(&order));
        for (i, d) in dims.iter().enumerate() {
            if !self.dims.contains(d) {
                v = v.insert_axis(Axis(i));
            }
        }
        let shape: Vec<usize> = dims.iter().map(|d| sizes.0[d]).collect();
        f(v.broadcast(IxDyn(&shape)).expect("broadcastable"))
    }

    fn axis(&self, dim: Dim) -> Axis {
        Axis(
            self.dims
                .iter()
                .position(|d| *d == dim)
                .expect("dim present"),
        )
    }
}

impl LArray<f64> {
    pub fn zip_with(&self, other: &Self, sizes: &Sizes, f: impl Fn(f64, f64) -> f64) -> Self {
        let dims = union(&self.dims, &other.dims);
        let data = self.view_as(&dims, sizes, |a| {
            other.view_as(&dims, sizes, |b| {
                Zip::from(&a).and(&b).map_collect(|&x, &y| f(x, y))
            })
        });
        Self { dims, data }
    }

    pub fn fillna(&self, v: f64) -> Self {
        Self {
            dims: self.dims.clone(),
            data: self.data.mapv(|x| if x.is_nan() { v } else { x }),
        }
    }

    /// `where(mask)`: NaN where the mask is false.
    pub fn where_(&self, mask: &LArray<bool>, sizes: &Sizes) -> Self {
        let dims = union(&self.dims, &mask.dims);
        let data = self.view_as(&dims, sizes, |a| {
            mask.view_as(&dims, sizes, |m| {
                Zip::from(&a)
                    .and(&m)
                    .map_collect(|&x, &k| if k { x } else { f64::NAN })
            })
        });
        Self { dims, data }
    }

    /// `sum(dim, min_count=1)`: NaN-skipping; all-NaN gives NaN.
    pub fn sum(&self, dim: Dim) -> Self {
        let ax = self.axis(dim);
        let data = self.data.map_axis(ax, |lane| {
            let mut s = 0.0;
            let mut n = 0;
            for &x in lane {
                if !x.is_nan() {
                    s += x;
                    n += 1;
                }
            }
            if n == 0 { f64::NAN } else { s }
        });
        let dims = self.dims.iter().copied().filter(|d| *d != dim).collect();
        Self { dims, data }
    }

    /// `roll(dim=shift)`, as `numpy.roll`.
    pub fn roll(&self, dim: Dim, shift: isize) -> Self {
        let ax = self.axis(dim);
        let n = self.data.len_of(ax) as isize;
        let mut out = self.data.clone();
        for i in 0..n {
            let src = (i - shift).rem_euclid(n) as usize;
            out.index_axis_mut(ax, i as usize)
                .assign(&self.data.index_axis(ax, src));
        }
        Self {
            dims: self.dims.clone(),
            data: out,
        }
    }

    /// `group_sum`: sum over `dim` within groups (`groups[i]` is the group of item `i`),
    /// giving the new dimension `new_dim`; empty or all-NaN groups give NaN.
    pub fn group_sum(&self, dim: Dim, groups: &[usize], new_dim: Dim, sizes: &mut Sizes) -> Self {
        let n_groups = groups.iter().max().map_or(0, |g| g + 1);
        sizes.0.insert(new_dim, n_groups);
        let ax = self.axis(dim);
        let mut shape = self.data.shape().to_vec();
        shape[ax.0] = n_groups;
        let mut out = ArrayD::from_elem(IxDyn(&shape), f64::NAN);
        for (i, &g) in groups.iter().enumerate() {
            let src = self.data.index_axis(ax, i);
            let mut dst = out.index_axis_mut(ax, g);
            Zip::from(&mut dst).and(&src).for_each(|d, &s| {
                if !s.is_nan() {
                    *d = if d.is_nan() { s } else { *d + s };
                }
            });
        }
        let mut dims = self.dims.clone();
        dims[ax.0] = new_dim;
        Self { dims, data: out }
    }
}

impl LArray<bool> {
    pub fn any(&self, dim: Dim) -> Self {
        let ax = self.axis(dim);
        let data = self.data.map_axis(ax, |lane| lane.iter().any(|&b| b));
        Self {
            dims: self.dims.iter().copied().filter(|d| *d != dim).collect(),
            data,
        }
    }

    pub fn or(&self, other: &Self, sizes: &Sizes) -> Self {
        let dims = union(&self.dims, &other.dims);
        let data = self.view_as(&dims, sizes, |a| {
            other.view_as(&dims, sizes, |b| {
                Zip::from(&a).and(&b).map_collect(|&x, &y| x || y)
            })
        });
        Self { dims, data }
    }

    /// Broadcast to `dims` and materialise (a `foreach` mask).
    pub fn expand(&self, dims: &[Dim], sizes: &Sizes) -> Self {
        Self {
            dims: dims.to_vec(),
            data: self.view_as(dims, sizes, |v| v.to_owned()),
        }
    }
}

/// An affine expression: terms `coef * x[col]` in the cell with flat index `flat`.
pub struct Aff {
    pub dims: Vec<Dim>,
    pub shape: Vec<usize>,
    pub flat: Vec<u64>,
    pub col: Vec<u32>,
    pub coef: Vec<f64>,
}

impl Aff {
    /// A variable over `mask` (C-order column numbering from `first_col`).
    pub fn variable(mask: &LArray<bool>, first_col: u32) -> Self {
        let std = mask.data.as_standard_layout();
        let mut flat = Vec::new();
        for (i, &b) in std.iter().enumerate() {
            if b {
                flat.push(i as u64);
            }
        }
        let n = flat.len();
        Self {
            dims: mask.dims.clone(),
            shape: mask.data.shape().to_vec(),
            flat,
            col: (first_col..first_col + n as u32).collect(),
            coef: vec![1.0; n],
        }
    }

    fn strides(shape: &[usize]) -> Vec<u64> {
        let mut s = vec![1u64; shape.len()];
        for i in (0..shape.len().saturating_sub(1)).rev() {
            s[i] = s[i + 1] * shape[i + 1] as u64;
        }
        s
    }

    /// Multiplies each term by a parameter broadcast to the expression's dims.
    pub fn mul(mut self, p: &LArray<f64>) -> Self {
        let pdata = p.data.as_standard_layout();
        let pstr = Self::strides(p.data.shape());
        let own = Self::strides(&self.shape);
        // Stride in `p` for each of our dims (0 if p lacks the dim).
        let map: Vec<u64> = self
            .dims
            .iter()
            .map(|d| p.dims.iter().position(|x| x == d).map_or(0, |i| pstr[i]))
            .collect();
        let slice = pdata.as_slice().expect("standard layout");
        for (f, c) in self.flat.iter().zip(self.coef.iter_mut()) {
            let mut rem = *f;
            let mut off = 0u64;
            for (s, m) in own.iter().zip(&map) {
                off += (rem / s) * m;
                rem %= s;
            }
            *c *= slice[off as usize];
        }
        self
    }

    /// `sum(over=dim)`: drops the dimension from the cell index.
    pub fn sum(mut self, dim: Dim) -> Self {
        let ax = self
            .dims
            .iter()
            .position(|d| *d == dim)
            .expect("dim present");
        let own = Self::strides(&self.shape);
        let len = self.shape[ax] as u64;
        for f in &mut self.flat {
            let hi = *f / (own[ax] * len);
            let lo = *f % own[ax];
            *f = hi * own[ax] + lo;
        }
        self.dims.remove(ax);
        self.shape.remove(ax);
        self
    }

    pub fn sub(mut self, other: Aff) -> Self {
        assert_eq!(self.dims, other.dims);
        self.flat.extend(other.flat);
        self.col.extend(other.col);
        self.coef.extend(other.coef.into_iter().map(|c| -c));
        self
    }

    /// Rows are the true cells of `rows` (same dims), numbered in C order.
    pub fn to_csc(&self, rows: &LArray<bool>, n_cols: usize) -> Csc {
        assert_eq!(self.dims, rows.dims);
        let std = rows.data.as_standard_layout();
        let mut row_of = vec![u32::MAX; std.len()];
        let mut n = 0u32;
        for (i, &b) in std.iter().enumerate() {
            if b {
                row_of[i] = n;
                n += 1;
            }
        }
        let triples = self
            .flat
            .iter()
            .zip(&self.col)
            .zip(&self.coef)
            .filter_map(|((f, c), v)| {
                let r = row_of[*f as usize];
                (r != u32::MAX).then_some((r, *c, *v))
            })
            .collect();
        Csc::from_triples(n as usize, n_cols, triples)
    }
}

/// Inputs in dense form.
pub struct Inputs {
    pub sizes: Sizes,
    pub carrier_out: LArray<bool>,
    pub carrier_in: LArray<bool>,
    pub eff: LArray<f64>,
    pub ts: LArray<f64>,
}

pub fn inputs(m: &Model) -> Inputs {
    let sizes = Sizes(HashMap::from([
        ("nodes", m.n_nodes),
        ("techs", m.n_techs),
        ("carriers", m.n_carriers),
        ("timesteps", m.hours),
    ]));
    let mut carrier_out = LArray::new(vec!["nodes", "techs", "carriers"], &sizes, false);
    for &(n, t, c) in &m.carrier_out {
        carrier_out.data[[n, t, c].as_slice()] = true;
    }
    let mut carrier_in = LArray::new(vec!["nodes", "techs", "carriers"], &sizes, false);
    for &(n, t, c) in &m.carrier_in {
        carrier_in.data[[n, t, c].as_slice()] = true;
    }
    let mut eff = LArray::new(vec!["nodes", "techs"], &sizes, f64::NAN);
    for &(n, t, v) in &m.eff {
        eff.data[[n, t].as_slice()] = v;
    }
    let mut ts = LArray::new(vec!["nodes", "techs", "timesteps"], &sizes, f64::NAN);
    for (n, t, v) in &m.ts {
        for (h, x) in v.iter().enumerate() {
            ts.data[[*n, *t, h].as_slice()] = *x;
        }
    }
    Inputs {
        sizes,
        carrier_out,
        carrier_in,
        eff,
        ts,
    }
}

pub fn build(m: &Model) -> Csc {
    let t0 = Instant::now();
    let inp = inputs(m);
    eprintln!("dense: inputs {:.3}s", t0.elapsed().as_secs_f64());
    let s = &inp.sizes;
    let var_dims = ["nodes", "techs", "carriers", "timesteps"];
    // Variables: foreach x where (masks materialised over the full product).
    let out_mask = inp.carrier_out.expand(&var_dims, s);
    let in_mask = inp.carrier_in.expand(&var_dims, s);
    let flow_out = Aff::variable(&out_mask, 0);
    let flow_in = Aff::variable(&in_mask, flow_out.col.len() as u32);
    let n_cols = flow_out.col.len() + flow_in.col.len();
    drop((out_mask, in_mask));
    // Coefficient: fillna(ts, 1) / fillna(eff, 1).
    let coef = inp
        .ts
        .fillna(1.0)
        .zip_with(&inp.eff.fillna(1.0), s, |a, b| a / b);
    let expr = flow_out.mul(&coef).sum("techs").sub(flow_in.sum("techs"));
    let row_dims = ["nodes", "carriers", "timesteps"];
    let rows = inp
        .carrier_in
        .any("techs")
        .or(&inp.carrier_out.any("techs"), s)
        .expand(&row_dims, s);
    expr.to_csc(&rows, n_cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes() -> Sizes {
        Sizes(HashMap::from([("a", 2), ("b", 3)]))
    }

    #[test]
    fn sum_min_count_and_where() {
        let s = sizes();
        let mut x = LArray::new(vec!["a", "b"], &s, f64::NAN);
        x.data[[0, 1].as_slice()] = 2.0;
        let y = x.sum("b");
        assert_eq!(y.data[[0].as_slice()], 2.0);
        assert!(y.data[[1].as_slice()].is_nan());
        let mut m = LArray::new(vec!["b"], &s, true);
        m.data[[1].as_slice()] = false;
        assert!(x.where_(&m, &s).data[[0, 1].as_slice()].is_nan());
    }

    #[test]
    fn roll_and_group_sum() {
        let mut s = sizes();
        let mut x = LArray::new(vec!["a", "b"], &s, 0.0);
        for j in 0..3 {
            x.data[[0, j].as_slice()] = j as f64;
        }
        let r = x.roll("b", 1);
        assert_eq!(
            r.data
                .index_axis(Axis(0), 0)
                .iter()
                .copied()
                .collect::<Vec<f64>>(),
            vec![2.0, 0.0, 1.0]
        );
        let g = x.group_sum("b", &[0, 1, 1], "g", &mut s);
        assert_eq!(
            g.data
                .index_axis(Axis(0), 0)
                .iter()
                .copied()
                .collect::<Vec<f64>>(),
            vec![0.0, 3.0]
        );
    }
}
