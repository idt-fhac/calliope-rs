//! Per-key fast paths (ADR 0001): operations that work on whole rows over the dense time
//! axis instead of cell by cell. Each applies when the operands' coordinates are equal (as
//! they are for arrays from one dataset) and returns `None` otherwise, so the caller falls
//! back to the reference implementation in `ops.rs`.

use std::collections::HashMap;

use crate::array::{Element, LArray, pick_dense};
use crate::coord::Coord;

/// Sorts `(key, row)` pairs and assembles an array (rows that are all missing dropped).
pub(crate) fn assemble<V: Element>(
    dims: Vec<Coord>,
    dense: Option<usize>,
    mut rows: Vec<(Vec<u32>, Vec<V>)>,
) -> LArray<V> {
    rows.retain(|(_, r)| r.iter().any(|v| !v.is_missing()));
    rows.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let n_sparse = dims.len() - usize::from(dense.is_some());
    let dlen = dense.map_or(1, |d| dims[d].len());
    let mut keys = Vec::with_capacity(rows.len() * n_sparse);
    let mut values = Vec::with_capacity(rows.len() * dlen);
    for (k, r) in rows {
        keys.extend(k);
        values.extend(r);
    }
    if n_sparse == 0 && values.is_empty() && dims.iter().all(|d| !d.is_empty()) {
        values = vec![V::missing(); dlen];
    }
    LArray {
        dims,
        dense,
        keys,
        values,
    }
}

impl<T: Element> LArray<T> {
    fn dense_name(&self) -> Option<&str> {
        self.dense.map(|d| self.dims[d].name())
    }

    /// `zip_with` per key: shared coordinates equal, at most one (common) time dimension.
    pub(crate) fn zip_fast<U: Element, V: Element>(
        &self,
        other: &LArray<U>,
        f: &impl Fn(&T, &U) -> V,
    ) -> Option<LArray<V>> {
        for d in &self.dims {
            if let Some(e) = other.coord(d.name())
                && d != e
            {
                return None;
            }
        }
        let mut dims = self.dims.clone();
        dims.extend(
            other
                .dims
                .iter()
                .filter(|e| self.coord(e.name()).is_none())
                .cloned(),
        );
        let rd = pick_dense(&dims);
        let rd_name = rd.map(|i| dims[i].name());
        if self.dense_name().is_some_and(|n| Some(n) != rd_name)
            || other.dense_name().is_some_and(|n| Some(n) != rd_name)
        {
            return None;
        }
        let rs: Vec<&str> = (0..dims.len())
            .filter(|i| Some(*i) != rd)
            .map(|i| dims[i].name())
            .collect();
        let pos = |name: &str| rs.iter().position(|n| *n == name).expect("in result");
        let a_names: Vec<&str> = self
            .sparse_axes()
            .into_iter()
            .map(|i| self.dims[i].name())
            .collect();
        let b_names: Vec<&str> = other
            .sparse_axes()
            .into_iter()
            .map(|i| other.dims[i].name())
            .collect();
        let a_pos: Vec<usize> = a_names.iter().map(|n| pos(n)).collect();
        let shared: Vec<(usize, usize)> = b_names
            .iter()
            .enumerate()
            .filter_map(|(j, n)| a_names.iter().position(|m| m == n).map(|i| (j, i)))
            .collect();
        let b_only: Vec<(usize, usize)> = b_names
            .iter()
            .enumerate()
            .filter(|(_, n)| !a_names.contains(n))
            .map(|(j, n)| (j, pos(n)))
            .collect();
        let mut index: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
        for j in 0..other.n_keys() {
            let kb = other.key(j);
            index
                .entry(shared.iter().map(|&(bj, _)| kb[bj]).collect())
                .or_default()
                .push(j);
        }
        let w = rd.map_or(1, |i| dims[i].len());
        let (a_t, b_t) = (self.dense.is_some(), other.dense.is_some());
        let mut rows = Vec::new();
        let mut probe = Vec::with_capacity(shared.len());
        for i in 0..self.n_keys() {
            let ka = self.key(i);
            probe.clear();
            probe.extend(shared.iter().map(|&(_, ai)| ka[ai]));
            let Some(matches) = index.get(&probe) else {
                continue;
            };
            let ra = self.row(i);
            for &j in matches {
                let kb = other.key(j);
                let mut key = vec![0u32; rs.len()];
                for (k, &p) in ka.iter().zip(&a_pos) {
                    key[p] = *k;
                }
                for &(bj, p) in &b_only {
                    key[p] = kb[bj];
                }
                let rb = other.row(j);
                let row: Vec<V> = (0..w)
                    .map(|t| f(&ra[if a_t { t } else { 0 }], &rb[if b_t { t } else { 0 }]))
                    .collect();
                rows.push((key, row));
            }
        }
        Some(assemble(dims, rd, rows))
    }

    /// `zip_union` per key, for operands with the same dims (names, order, coordinates).
    pub(crate) fn zip_union_fast<U: Element, V: Element>(
        &self,
        other: &LArray<U>,
        f: &impl Fn(Option<&T>, Option<&U>) -> V,
    ) -> Option<LArray<V>> {
        if self.dims != other.dims {
            return None;
        }
        let w = self.dense_len();
        let (mut i, mut j) = (0, 0);
        let (na, nb) = (self.n_keys(), other.n_keys());
        let mut rows = Vec::with_capacity(na.max(nb));
        while i < na || j < nb {
            let ord = match (i < na, j < nb) {
                (true, true) => self.key(i).cmp(other.key(j)),
                (true, false) => std::cmp::Ordering::Less,
                _ => std::cmp::Ordering::Greater,
            };
            let (key, row): (Vec<u32>, Vec<V>) = match ord {
                std::cmp::Ordering::Less => {
                    let r = self.row(i);
                    i += 1;
                    (
                        self.key(i - 1).to_vec(),
                        (0..w).map(|t| f(cell(r, t), None)).collect(),
                    )
                }
                std::cmp::Ordering::Greater => {
                    let r = other.row(j);
                    j += 1;
                    (
                        other.key(j - 1).to_vec(),
                        (0..w).map(|t| f(None, cell(r, t))).collect(),
                    )
                }
                std::cmp::Ordering::Equal => {
                    let (ra, rb) = (self.row(i), other.row(j));
                    i += 1;
                    j += 1;
                    (
                        self.key(i - 1).to_vec(),
                        (0..w).map(|t| f(cell(ra, t), cell(rb, t))).collect(),
                    )
                }
            };
            rows.push((key, row));
        }
        Some(assemble(self.dims.clone(), self.dense, rows))
    }

    /// `fillna(value)` per key: every key of the full product of the sparse dims.
    pub(crate) fn fillna_fast(&self, value: &T) -> LArray<T> {
        let sparse = self.sparse_axes();
        let shape: Vec<usize> = sparse.iter().map(|&a| self.dims[a].len()).collect();
        let total: usize = shape.iter().product();
        let w = self.dense_len();
        let mut rows = Vec::with_capacity(total);
        let mut idx = vec![0u32; sparse.len()];
        let mut k = 0;
        for _ in 0..total {
            let row: Vec<T> = if k < self.n_keys() && self.key(k) == idx.as_slice() {
                k += 1;
                self.row(k - 1)
                    .iter()
                    .map(|v| {
                        if v.is_missing() {
                            value.clone()
                        } else {
                            v.clone()
                        }
                    })
                    .collect()
            } else {
                vec![value.clone(); w]
            };
            rows.push((idx.clone(), row));
            crate::array::increment(&mut idx, &shape);
        }
        if self.dims.iter().any(Coord::is_empty) {
            rows.clear();
        }
        assemble(self.dims.clone(), self.dense, rows)
    }

    /// `reduce` per key: groups keys by their codes on the kept sparse dims and reduces
    /// whole time rows (or each row, when the time dim is reduced).
    pub(crate) fn reduce_fast<U: Element>(
        &self,
        over: &[&str],
        f: &impl Fn(&[&T]) -> U,
    ) -> LArray<U> {
        let keep: Vec<usize> = (0..self.dims.len())
            .filter(|&i| !over.contains(&self.dims[i].name()))
            .collect();
        let dims: Vec<Coord> = keep.iter().map(|&i| self.dims[i].clone()).collect();
        let dense_kept = self.dense.filter(|d| keep.contains(d));
        let rd = dense_kept.map(|d| keep.iter().position(|k| *k == d).expect("kept"));
        let sparse = self.sparse_axes();
        let kept_sparse: Vec<usize> = sparse
            .iter()
            .enumerate()
            .filter(|(_, a)| keep.contains(a))
            .map(|(i, _)| i)
            .collect();
        let mut groups: HashMap<Vec<u32>, Vec<usize>> = HashMap::new();
        let mut order: Vec<Vec<u32>> = Vec::new();
        for i in 0..self.n_keys() {
            let k = self.key(i);
            let g: Vec<u32> = kept_sparse.iter().map(|&s| k[s]).collect();
            groups
                .entry(g.clone())
                .or_insert_with(|| {
                    order.push(g);
                    Vec::new()
                })
                .push(i);
        }
        let w = self.dense_len();
        let mut buf: Vec<&T> = Vec::new();
        let mut rows = Vec::with_capacity(order.len());
        for g in order {
            let members = &groups[&g];
            let row: Vec<U> = if dense_kept.is_some() {
                (0..w)
                    .map(|t| {
                        buf.clear();
                        buf.extend(
                            members
                                .iter()
                                .map(|&i| &self.row(i)[t])
                                .filter(|v| !v.is_missing()),
                        );
                        if buf.is_empty() {
                            U::missing()
                        } else {
                            f(&buf)
                        }
                    })
                    .collect()
            } else {
                buf.clear();
                for &i in members {
                    buf.extend(self.row(i).iter().filter(|v| !v.is_missing()));
                }
                vec![if buf.is_empty() {
                    U::missing()
                } else {
                    f(&buf)
                }]
            };
            rows.push((g, row));
        }
        assemble(dims, rd, rows)
    }
}

fn cell<X: Element>(row: &[X], t: usize) -> Option<&X> {
    let v = &row[t];
    (!v.is_missing()).then_some(v)
}
