//! The `LArray` storage (ADR 0001): sparse over the non-time dimensions, dense over the time
//! dimension.

use std::fmt::Debug;

use crate::coord::{ArrayError, Coord};

/// A value type with a "missing" state.
pub trait Element: Clone + Debug + PartialEq + Send + Sync + 'static {
    /// The missing value.
    fn missing() -> Self;
    /// True for the missing value.
    fn is_missing(&self) -> bool;
}

/// `NaN` is missing, as in xarray.
impl Element for f64 {
    fn missing() -> Self {
        f64::NAN
    }
    fn is_missing(&self) -> bool {
        self.is_nan()
    }
}

/// Boolean arrays have no missing state: an absent cell is `false`. This is how Calliope
/// uses them (`fillna(False)` on bool lookups, NaN → `False` in `where` strings).
impl Element for bool {
    fn missing() -> Self {
        false
    }
    fn is_missing(&self) -> bool {
        !*self
    }
}

/// `None` is missing (string lookups, integer data with holes).
impl<T: Clone + Debug + PartialEq + Send + Sync + 'static> Element for Option<T> {
    fn missing() -> Self {
        None
    }
    fn is_missing(&self) -> bool {
        self.is_none()
    }
}

/// A labelled N-d array.
///
/// - `dims` are the dimensions in xarray order (they decide C order, e.g. of LP rows).
/// - At most one dimension is *dense*: the time axis (the first dimension whose labels are
///   datetimes). Every stored key has a value for each of its labels.
/// - All other dimensions are *sparse*: only keys (tuples of label codes) that have data are
///   stored, sorted lexicographically in `dims` order.
/// - A cell whose key is not stored is missing. Missing values may also be stored explicitly.
#[derive(Debug, Clone)]
pub struct LArray<T> {
    pub(crate) dims: Vec<Coord>,
    pub(crate) dense: Option<usize>,
    /// Flat keys, `n_sparse` codes per entry.
    pub(crate) keys: Vec<u32>,
    /// `n_keys × dense_len` values.
    pub(crate) values: Vec<T>,
}

impl<T: Element> LArray<T> {
    /// A 0-d array.
    pub fn scalar(v: T) -> Self {
        Self {
            dims: Vec::new(),
            dense: None,
            keys: Vec::new(),
            values: vec![v],
        }
    }

    /// An array with no data (all missing).
    pub fn empty(dims: Vec<Coord>) -> Result<Self, ArrayError> {
        check_dims(&dims)?;
        let dense = pick_dense(&dims);
        let mut a = Self {
            dims,
            dense,
            keys: Vec::new(),
            values: Vec::new(),
        };
        if a.n_sparse() == 0 && a.dims.iter().all(|d| !d.is_empty()) {
            // A dense-only array has a single (empty) key.
            a.values = vec![T::missing(); a.dense_len()];
        }
        Ok(a)
    }

    /// Builds an array from values in C order over the full product of `dims` (as numpy
    /// stores it). Keys whose values are all missing are not stored.
    pub fn from_dense(dims: Vec<Coord>, values: Vec<T>) -> Result<Self, ArrayError> {
        check_dims(&dims)?;
        let total: usize = dims.iter().map(Coord::len).product();
        if values.len() != total {
            return Err(ArrayError::Shape(format!(
                "{} values for shape of {total} cells",
                values.len()
            )));
        }
        let mut entries = Vec::new();
        let shape: Vec<usize> = dims.iter().map(Coord::len).collect();
        let mut idx = vec![0u32; dims.len()];
        for v in values {
            if !v.is_missing() {
                entries.push((idx.clone(), v));
            }
            increment(&mut idx, &shape);
        }
        Self::from_cells(dims, entries)
    }

    /// Builds an array from `(codes in dims order, value)` cells; later duplicates win.
    pub fn from_cells(
        dims: Vec<Coord>,
        cells: impl IntoIterator<Item = (Vec<u32>, T)>,
    ) -> Result<Self, ArrayError> {
        check_dims(&dims)?;
        let dense = pick_dense(&dims);
        let n_sparse = dims.len() - usize::from(dense.is_some());
        let dlen = dense.map_or(1, |d| dims[d].len());
        // Keys packed into a u64 (in lexicographic order) when the codes fit.
        let bits: Vec<u32> = dims
            .iter()
            .enumerate()
            .filter(|(i, _)| Some(*i) != dense)
            .map(|(_, d)| usize::BITS - d.len().leading_zeros())
            .collect();
        if bits.iter().sum::<u32>() <= 64 {
            return Self::from_cells_packed(dims, dense, &bits, cells);
        }
        let mut map: std::collections::BTreeMap<Vec<u32>, Vec<T>> = Default::default();
        for (codes, v) in cells {
            if codes.len() != dims.len() {
                return Err(ArrayError::Shape("cell has wrong number of codes".into()));
            }
            for (c, d) in codes.iter().zip(&dims) {
                if *c as usize >= d.len() {
                    return Err(ArrayError::Shape(format!(
                        "code {c} out of range for `{}`",
                        d.name()
                    )));
                }
            }
            let (key, t) = split_codes(&codes, dense);
            let slot = map.entry(key).or_insert_with(|| vec![T::missing(); dlen]);
            slot[t] = v;
        }
        let mut keys = Vec::with_capacity(map.len() * n_sparse);
        let mut values = Vec::with_capacity(map.len() * dlen);
        for (k, v) in map {
            keys.extend(k);
            values.extend(v);
        }
        if n_sparse == 0 && values.is_empty() && dims.iter().all(|d| !d.is_empty()) {
            values = vec![T::missing(); dlen];
        }
        Ok(Self {
            dims,
            dense,
            keys,
            values,
        })
    }

    fn from_cells_packed(
        dims: Vec<Coord>,
        dense: Option<usize>,
        bits: &[u32],
        cells: impl IntoIterator<Item = (Vec<u32>, T)>,
    ) -> Result<Self, ArrayError> {
        let n_sparse = bits.len();
        let dlen = dense.map_or(1, |d| dims[d].len());
        let mut slot: std::collections::HashMap<u64, usize> = Default::default();
        let mut rows: Vec<(u64, Vec<T>)> = Vec::new();
        for (codes, v) in cells {
            if codes.len() != dims.len() {
                return Err(ArrayError::Shape("cell has wrong number of codes".into()));
            }
            let mut packed = 0u64;
            let mut b = bits.iter();
            let mut t = 0usize;
            for (i, (c, d)) in codes.iter().zip(&dims).enumerate() {
                if *c as usize >= d.len() {
                    return Err(ArrayError::Shape(format!(
                        "code {c} out of range for `{}`",
                        d.name()
                    )));
                }
                if Some(i) == dense {
                    t = *c as usize;
                } else {
                    let w = *b.next().expect("one width per sparse dim");
                    packed = if w == 64 {
                        u64::from(*c)
                    } else {
                        (packed << w) | u64::from(*c)
                    };
                }
            }
            let idx = *slot.entry(packed).or_insert_with(|| {
                rows.push((packed, vec![T::missing(); dlen]));
                rows.len() - 1
            });
            rows[idx].1[t] = v;
        }
        rows.sort_unstable_by_key(|r| r.0);
        let mut keys = Vec::with_capacity(rows.len() * n_sparse);
        let mut values = Vec::with_capacity(rows.len() * dlen);
        for (p, r) in rows {
            let mut shift = 0u32;
            let start = keys.len();
            keys.resize(start + n_sparse, 0);
            for (k, w) in bits.iter().enumerate().rev() {
                let mask = if *w >= 64 { u64::MAX } else { (1u64 << w) - 1 };
                keys[start + k] = u32::try_from((p >> shift) & mask).expect("fits");
                shift += w;
            }
            values.extend(r);
        }
        if n_sparse == 0 && values.is_empty() && dims.iter().all(|d| !d.is_empty()) {
            values = vec![T::missing(); dlen];
        }
        Ok(Self {
            dims,
            dense,
            keys,
            values,
        })
    }

    /// The dimensions, in order.
    pub fn dims(&self) -> &[Coord] {
        &self.dims
    }

    /// Dimension names, in order.
    pub fn dim_names(&self) -> Vec<&str> {
        self.dims.iter().map(Coord::name).collect()
    }

    /// The coordinate of a dimension.
    pub fn coord(&self, dim: &str) -> Option<&Coord> {
        self.dims.iter().find(|d| d.name() == dim)
    }

    /// Position of a dimension.
    pub fn axis(&self, dim: &str) -> Result<usize, ArrayError> {
        self.dims
            .iter()
            .position(|d| d.name() == dim)
            .ok_or_else(|| ArrayError::MissingDim(dim.to_owned()))
    }

    /// The name of the dense (time) dimension, if any.
    pub fn dense_dim(&self) -> Option<&str> {
        self.dense.map(|d| self.dims[d].name())
    }

    /// The shape of the full product.
    pub fn shape(&self) -> Vec<usize> {
        self.dims.iter().map(Coord::len).collect()
    }

    pub(crate) fn n_sparse(&self) -> usize {
        self.dims.len() - usize::from(self.dense.is_some())
    }

    pub(crate) fn dense_len(&self) -> usize {
        self.dense.map_or(1, |d| self.dims[d].len())
    }

    /// Number of stored keys.
    pub fn n_keys(&self) -> usize {
        if self.n_sparse() == 0 {
            usize::from(!self.values.is_empty())
        } else {
            self.keys.len() / self.n_sparse()
        }
    }

    pub(crate) fn key(&self, i: usize) -> &[u32] {
        let n = self.n_sparse();
        &self.keys[i * n..(i + 1) * n]
    }

    pub(crate) fn row(&self, i: usize) -> &[T] {
        let w = self.dense_len();
        &self.values[i * w..(i + 1) * w]
    }

    /// Positions of the sparse dimensions within `dims`.
    pub(crate) fn sparse_axes(&self) -> Vec<usize> {
        (0..self.dims.len())
            .filter(|&i| Some(i) != self.dense)
            .collect()
    }

    /// Number of non-missing cells.
    pub fn count(&self) -> usize {
        self.values.iter().filter(|v| !v.is_missing()).count()
    }

    /// Iterates over non-missing cells as `(codes in dims order, value)`, in storage order.
    pub fn cells(&self) -> impl Iterator<Item = (Vec<u32>, &T)> + '_ {
        let w = self.dense_len();
        let sparse = self.sparse_axes();
        (0..self.n_keys()).flat_map(move |i| {
            let key = self.key(i).to_vec();
            let sparse = sparse.clone();
            self.row(i)
                .iter()
                .enumerate()
                .filter(|(_, v)| !v.is_missing())
                .map(move |(t, v)| {
                    let mut codes = vec![0u32; self.dims.len()];
                    for (a, c) in sparse.iter().zip(&key) {
                        codes[*a] = *c;
                    }
                    if let Some(d) = self.dense {
                        codes[d] = u32::try_from(t).expect("dense index fits u32");
                    }
                    let _ = w;
                    (codes, v)
                })
        })
    }

    /// The value at a cell (missing if not stored).
    pub fn get(&self, codes: &[u32]) -> T {
        let (key, t) = split_codes(codes, self.dense);
        match self.find_key(&key) {
            Some(i) => self.row(i)[t].clone(),
            None => T::missing(),
        }
    }

    pub(crate) fn find_key(&self, key: &[u32]) -> Option<usize> {
        let n = self.n_sparse();
        if n == 0 {
            return (!self.values.is_empty()).then_some(0);
        }
        let (mut lo, mut hi) = (0, self.n_keys());
        while lo < hi {
            let mid = (lo + hi) / 2;
            match self.key(mid).cmp(key) {
                std::cmp::Ordering::Less => lo = mid + 1,
                std::cmp::Ordering::Greater => hi = mid,
                std::cmp::Ordering::Equal => return Some(mid),
            }
        }
        None
    }

    /// The values over the full product in C order, with missing cells filled in
    /// (numpy's layout; for tests, output and small arrays).
    pub fn to_dense(&self) -> Vec<T> {
        let shape = self.shape();
        let total: usize = shape.iter().product();
        let mut out = vec![T::missing(); total];
        let mut strides = vec![1usize; shape.len()];
        for i in (0..shape.len().saturating_sub(1)).rev() {
            strides[i] = strides[i + 1] * shape[i + 1];
        }
        for (codes, v) in self.cells() {
            let flat: usize = codes
                .iter()
                .zip(&strides)
                .map(|(c, s)| *c as usize * s)
                .sum();
            out[flat] = v.clone();
        }
        out
    }

    /// Reorders the dimensions (xarray `transpose`).
    pub fn transpose(&self, order: &[&str]) -> Result<Self, ArrayError> {
        if order.len() != self.dims.len() {
            return Err(ArrayError::Shape(format!(
                "transpose needs all of {:?}",
                self.dim_names()
            )));
        }
        let perm: Vec<usize> = order
            .iter()
            .map(|d| self.axis(d))
            .collect::<Result<_, _>>()?;
        let dims: Vec<Coord> = perm.iter().map(|&i| self.dims[i].clone()).collect();
        let cells: Vec<(Vec<u32>, T)> = self
            .cells()
            .map(|(codes, v)| (perm.iter().map(|&i| codes[i]).collect(), v.clone()))
            .collect();
        Self::from_cells(dims, cells)
    }

    /// Maps every stored value (missing cells stay unstored).
    pub fn map<U: Element>(&self, f: impl Fn(&T) -> U) -> LArray<U> {
        LArray {
            dims: self.dims.clone(),
            dense: self.dense,
            keys: self.keys.clone(),
            values: self.values.iter().map(f).collect(),
        }
    }

    /// Drops keys whose values are all missing.
    pub fn compact(mut self) -> Self {
        let n = self.n_sparse();
        if n == 0 {
            return self;
        }
        let w = self.dense_len();
        let mut keys = Vec::with_capacity(self.keys.len());
        let mut values = Vec::with_capacity(self.values.len());
        for i in 0..self.n_keys() {
            if self.row(i).iter().any(|v| !v.is_missing()) {
                keys.extend_from_slice(self.key(i));
                values.extend_from_slice(&self.values[i * w..(i + 1) * w]);
            }
        }
        self.keys = keys;
        self.values = values;
        self
    }
}

impl<T: Element> PartialEq for LArray<T> {
    /// Same dims and coordinates, and the same value in every cell (missing equals missing).
    fn eq(&self, other: &Self) -> bool {
        self.dims == other.dims && {
            let a = self.to_dense();
            let b = other.to_dense();
            a.iter()
                .zip(&b)
                .all(|(x, y)| (x.is_missing() && y.is_missing()) || x == y)
        }
    }
}

fn check_dims(dims: &[Coord]) -> Result<(), ArrayError> {
    for (i, d) in dims.iter().enumerate() {
        if dims[..i].iter().any(|e| e.name() == d.name()) {
            return Err(ArrayError::Shape(format!(
                "dimension `{}` given twice",
                d.name()
            )));
        }
    }
    Ok(())
}

pub(crate) fn pick_dense(dims: &[Coord]) -> Option<usize> {
    dims.iter().position(Coord::is_datetime)
}

pub(crate) fn split_codes(codes: &[u32], dense: Option<usize>) -> (Vec<u32>, usize) {
    let mut key = Vec::with_capacity(codes.len());
    let mut t = 0;
    for (i, c) in codes.iter().enumerate() {
        if Some(i) == dense {
            t = *c as usize;
        } else {
            key.push(*c);
        }
    }
    (key, t)
}

pub(crate) fn increment(idx: &mut [u32], shape: &[usize]) {
    for i in (0..idx.len()).rev() {
        idx[i] += 1;
        if (idx[i] as usize) < shape[i] {
            return;
        }
        idx[i] = 0;
    }
}
