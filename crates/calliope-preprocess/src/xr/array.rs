//! `DataArray`: a dense labelled array in C order, with xarray's alignment rules.

use calliope_array::{Coord, Label};

use super::data::{DType, Data, Obj, SparseF64};

/// How shared dimensions are aligned.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Join {
    /// Sorted union (pandas `Index.union`), unless the indexes are equal.
    Outer,
    /// Left labels that are also on the right, in left order.
    Inner,
    /// Left labels.
    Left,
}

/// pandas `Index.union` (sort=None): equal indexes and an empty side are kept as they are;
/// otherwise the union is sorted (if the labels can be compared).
pub fn union_labels(a: &[Label], b: &[Label]) -> Vec<Label> {
    if a == b || b.is_empty() {
        return a.to_vec();
    }
    if a.is_empty() {
        return b.to_vec();
    }
    let mut out: Vec<Label> = a.to_vec();
    let seen: std::collections::HashSet<&Label> = a.iter().collect();
    out.extend(b.iter().filter(|l| !seen.contains(l)).cloned());
    if comparable(&out) {
        out.sort();
    }
    out
}

/// True if all labels have one kind (pandas can sort them).
pub fn comparable(labels: &[Label]) -> bool {
    let kind = |l: &Label| match l {
        Label::Int(_) | Label::Float(_) => 0,
        Label::Datetime(_) => 1,
        Label::Str(_) => 2,
    };
    labels.windows(2).all(|w| kind(&w[0]) == kind(&w[1]))
}

/// Joins two coordinates of one dimension.
pub fn join_coords(a: &Coord, b: &Coord, join: Join) -> Coord {
    if a == b {
        return a.clone();
    }
    let labels = match join {
        Join::Outer => union_labels(a.labels(), b.labels()),
        Join::Inner => a
            .labels()
            .iter()
            .filter(|l| b.code(l).is_some())
            .cloned()
            .collect(),
        Join::Left => return a.clone(),
    };
    Coord::new(a.name(), labels).expect("joined labels are unique")
}

pub(crate) fn strides(shape: &[usize]) -> Vec<usize> {
    let mut s = vec![1usize; shape.len()];
    for i in (0..shape.len().saturating_sub(1)).rev() {
        s[i] = s[i + 1] * shape[i + 1];
    }
    s
}

/// Maps the cells of `dst` (in C order) to flat positions in `src`, whose dims must all be
/// in `dst` (`None` where a label of `dst` is not in `src`). Iterates without allocating per
/// cell.
#[derive(Debug, Clone)]
pub(crate) struct Mapping {
    /// Per `dst` axis: offset into `src` per label (`None`: not in `src`), or `None` for
    /// axes `src` does not have.
    per_axis: Vec<Option<Vec<Option<usize>>>>,
    dst_shape: Vec<usize>,
    total: usize,
    any_missing: bool,
}

impl Mapping {
    pub(crate) fn new(src: &[Coord], dst: &[Coord]) -> Self {
        let src_shape: Vec<usize> = src.iter().map(Coord::len).collect();
        let src_strides = strides(&src_shape);
        let dst_shape: Vec<usize> = dst.iter().map(Coord::len).collect();
        let total: usize = dst_shape.iter().product();
        let mut per_axis: Vec<Option<Vec<Option<usize>>>> = vec![None; dst.len()];
        for (si, sc) in src.iter().enumerate() {
            let di = dst
                .iter()
                .position(|d| d.name() == sc.name())
                .expect("source dims are a subset of destination dims");
            let offs: Vec<Option<usize>> = if dst[di] == *sc {
                (0..sc.len()).map(|c| Some(c * src_strides[si])).collect()
            } else {
                dst[di]
                    .labels()
                    .iter()
                    .map(|l| sc.code(l).map(|c| c as usize * src_strides[si]))
                    .collect()
            };
            per_axis[di] = Some(offs);
        }
        let any_missing = total > 0
            && per_axis
                .iter()
                .flatten()
                .any(|offs| offs.iter().any(Option::is_none));
        Self {
            per_axis,
            dst_shape,
            total,
            any_missing,
        }
    }

    /// Number of `dst` cells.
    pub(crate) fn len(&self) -> usize {
        self.total
    }

    /// True if some `dst` cell has no source.
    pub(crate) fn any_missing(&self) -> bool {
        self.any_missing
    }

    /// The source position of every `dst` cell, in C order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = Option<usize>> + '_ {
        let mut idx = vec![0usize; self.dst_shape.len()];
        (0..self.total).map(move |_| {
            let mut flat = Some(0usize);
            for (ax, offs) in self.per_axis.iter().enumerate() {
                if let Some(offs) = offs {
                    flat = match (flat, offs[idx[ax]]) {
                        (Some(f), Some(o)) => Some(f + o),
                        _ => None,
                    };
                }
            }
            for ax in (0..idx.len()).rev() {
                idx[ax] += 1;
                if idx[ax] < self.dst_shape[ax] {
                    break;
                }
                idx[ax] = 0;
            }
            flat
        })
    }
}

/// Sparse values over `src` placed on `dst` (a superset of the dims, any labels and order):
/// values whose labels are not in `dst` are dropped, new dims are broadcast.
fn conform_sparse(s: &SparseF64, src: &[Coord], dst: &[Coord]) -> SparseF64 {
    let src_shape: Vec<usize> = src.iter().map(Coord::len).collect();
    let src_strides = strides(&src_shape);
    let dst_shape: Vec<usize> = dst.iter().map(Coord::len).collect();
    let dst_strides = strides(&dst_shape);
    let total: usize = dst_shape.iter().product();
    // Per src axis: dst offset per src code.
    let maps: Vec<Vec<Option<usize>>> = src
        .iter()
        .map(|sc| {
            let di = dst
                .iter()
                .position(|d| d.name() == sc.name())
                .expect("source dims are a subset of destination dims");
            if dst[di] == *sc {
                (0..sc.len()).map(|c| Some(c * dst_strides[di])).collect()
            } else {
                sc.labels()
                    .iter()
                    .map(|l| dst[di].code(l).map(|c| c as usize * dst_strides[di]))
                    .collect()
            }
        })
        .collect();
    // Offsets of every combination of the new (broadcast) dims.
    let mut extra = vec![0usize];
    for (di, d) in dst.iter().enumerate() {
        if src.iter().any(|c| c.name() == d.name()) {
            continue;
        }
        let (n, stride) = (dst_shape[di], dst_strides[di]);
        extra = extra
            .iter()
            .flat_map(|&o| (0..n).map(move |c| o + c * stride))
            .collect();
    }
    if total == 0 {
        extra.clear();
    }
    let mut pairs = Vec::with_capacity(s.nnz() * extra.len());
    'cells: for (p, v) in s.iter() {
        let mut base = 0usize;
        for (ax, m) in maps.iter().enumerate() {
            let code = (p / src_strides[ax]) % src_shape[ax];
            match m[code] {
                Some(o) => base += o,
                None => continue 'cells,
            }
        }
        for &e in &extra {
            pairs.push((base + e, v));
        }
    }
    SparseF64::from_pairs(total, pairs)
}

pub(crate) fn conform_data(data: Data, src: &[Coord], dst: &[Coord], fill: DType) -> Data {
    if src == dst {
        return data;
    }
    if let Data::Sparse(s) = &data {
        return Data::Sparse(conform_sparse(s, src, dst));
    }
    let map = Mapping::new(src, dst);
    data.gather_iter(map.iter(), map.len(), map.any_missing(), fill)
}

/// A dense labelled array: dims (with their labels) and values in C order.
#[derive(Debug, Clone, PartialEq)]
pub struct DataArray {
    /// The dimensions, in order.
    pub coords: Vec<Coord>,
    /// Values over the full product of `coords`, in C order.
    pub data: Data,
}

impl DataArray {
    /// An array; `data` must have one value per cell.
    pub fn new(coords: Vec<Coord>, data: Data) -> Self {
        debug_assert_eq!(
            coords.iter().map(Coord::len).product::<usize>(),
            data.len(),
            "data length must match the shape"
        );
        Self { coords, data }
    }

    /// A 0-d array.
    pub fn scalar(v: Obj) -> Self {
        Self::new(Vec::new(), Data::infer(vec![v]))
    }

    /// Dimension names.
    pub fn dims(&self) -> Vec<&str> {
        self.coords.iter().map(Coord::name).collect()
    }

    /// The coordinate of a dimension.
    pub fn coord(&self, dim: &str) -> Option<&Coord> {
        self.coords.iter().find(|c| c.name() == dim)
    }

    /// True if the array has this dimension.
    pub fn has_dim(&self, dim: &str) -> bool {
        self.coord(dim).is_some()
    }

    /// The shape.
    pub fn shape(&self) -> Vec<usize> {
        self.coords.iter().map(Coord::len).collect()
    }

    /// The dtype.
    pub fn dtype(&self) -> DType {
        self.data.dtype()
    }

    /// Reshapes to `dst` (a superset of the dims, any labels): reorders, broadcasts and
    /// reindexes. Cells without a source become missing (`fill` for bool/int data).
    pub fn conform(&self, dst: &[Coord], fill: DType) -> DataArray {
        if self.coords == dst {
            return self.clone();
        }
        if let Data::Sparse(s) = &self.data {
            return DataArray::new(
                dst.to_vec(),
                Data::Sparse(conform_sparse(s, &self.coords, dst)),
            );
        }
        let map = Mapping::new(&self.coords, dst);
        DataArray::new(
            dst.to_vec(),
            self.data
                .gather_iter(map.iter(), map.len(), map.any_missing(), fill),
        )
    }

    /// [`DataArray::conform`] by value: no copy when the coordinates are already `dst`.
    pub fn conform_into(self, dst: &[Coord], fill: DType) -> DataArray {
        if self.coords == dst {
            return self;
        }
        self.conform(dst, fill)
    }

    /// Reorders dimensions (`transpose`).
    pub fn transpose(&self, order: &[&str]) -> DataArray {
        let dst: Vec<Coord> = order
            .iter()
            .map(|d| self.coord(d).expect("transpose to known dims").clone())
            .collect();
        self.conform(&dst, DType::Float)
    }

    /// `reindex(dim=labels)` with missing values for new labels.
    pub fn reindex(&self, coord: &Coord) -> DataArray {
        self.clone().reindex_into(coord)
    }

    /// [`DataArray::reindex`] by value.
    pub fn reindex_into(self, coord: &Coord) -> DataArray {
        let dst: Vec<Coord> = self
            .coords
            .iter()
            .map(|c| {
                if c.name() == coord.name() {
                    coord.clone()
                } else {
                    c.clone()
                }
            })
            .collect();
        self.conform_into(&dst, DType::Float)
    }

    /// Adds a new leading dimension (`expand_dims`).
    pub fn expand_dims(&self, coord: Coord) -> DataArray {
        let mut dst = vec![coord];
        dst.extend(self.coords.iter().cloned());
        self.conform(&dst, DType::Float)
    }

    /// Aligns two arrays on their shared dims and returns the broadcast dims (left dims,
    /// then the right's new dims), with both arrays conformed to them.
    pub fn broadcast_pair(
        a: &DataArray,
        b: &DataArray,
        join: Join,
    ) -> (Vec<Coord>, DataArray, DataArray) {
        let dims = Self::broadcast_dims(a, b, join);
        let a2 = a.conform(&dims, DType::Float);
        let b2 = b.conform(&dims, DType::Float);
        (dims, a2, b2)
    }

    /// [`DataArray::broadcast_pair`] taking the left array by value.
    fn broadcast_pair_into(
        a: DataArray,
        b: &DataArray,
        join: Join,
    ) -> (Vec<Coord>, DataArray, DataArray) {
        let dims = Self::broadcast_dims(&a, b, join);
        let a2 = a.conform_into(&dims, DType::Float);
        let b2 = b.conform(&dims, DType::Float);
        (dims, a2, b2)
    }

    /// The dims of [`DataArray::broadcast_pair`]: the left dims (labels joined), then the
    /// right's new dims.
    fn broadcast_dims(a: &DataArray, b: &DataArray, join: Join) -> Vec<Coord> {
        let mut dims: Vec<Coord> = a
            .coords
            .iter()
            .map(|c| match b.coord(c.name()) {
                Some(bc) => join_coords(c, bc, join),
                None => c.clone(),
            })
            .collect();
        for c in &b.coords {
            if a.coord(c.name()).is_none() {
                dims.push(c.clone());
            }
        }
        dims
    }

    /// Mask of non-missing values.
    pub fn notnull(&self) -> DataArray {
        DataArray::new(self.coords.clone(), Data::Bool(self.data.notnull()))
    }

    /// Mask of missing values.
    pub fn isnull(&self) -> DataArray {
        DataArray::new(
            self.coords.clone(),
            Data::Bool(self.data.notnull().into_iter().map(|b| !b).collect()),
        )
    }

    /// Truthiness of every cell.
    pub fn truthy(&self) -> Vec<bool> {
        match &self.data {
            Data::Bool(v) => v.clone(),
            d => (0..d.len()).map(|i| d.get(i).truthy()).collect(),
        }
    }

    /// `where(cond)`: keeps values where `cond` is true; missing elsewhere (inner join).
    pub fn where_(&self, cond: &DataArray) -> DataArray {
        self.clone().where_into(cond)
    }

    /// [`DataArray::where_`] by value: the values are masked in place.
    pub fn where_into(self, cond: &DataArray) -> DataArray {
        if cond.coords.iter().any(|c| !self.has_dim(c.name())) {
            return self.where_generic(cond);
        }
        let dims = Self::broadcast_dims(&self, cond, Join::Inner);
        let a = self.conform_into(&dims, DType::Float);
        // `cond` conformed to `dims` has no missing cells (inner join), so every cell maps.
        let truth = cond.truthy();
        let map = Mapping::new(&cond.coords, &dims);
        let keep = |j: Option<usize>| j.is_none_or(|j| truth[j]);
        let data = match a.data {
            Data::Float(mut v) => {
                for (x, j) in v.iter_mut().zip(map.iter()) {
                    if !keep(j) {
                        *x = f64::NAN;
                    }
                }
                Data::Float(v)
            }
            Data::Sparse(mut s) => {
                let src_shape: Vec<usize> = dims.iter().map(Coord::len).collect();
                let st = strides(&src_shape);
                // Position in `cond` of a cell of `dims`.
                let offs: Vec<(usize, Vec<Option<usize>>)> = Mapping::new(&cond.coords, &dims)
                    .per_axis
                    .into_iter()
                    .enumerate()
                    .filter_map(|(ax, o)| o.map(|o| (ax, o)))
                    .collect();
                let mut k = 0;
                for i in 0..s.idx.len() {
                    let p = s.idx[i];
                    let mut j = Some(0usize);
                    for (ax, o) in &offs {
                        let code = (p / st[*ax]) % src_shape[*ax];
                        j = j.and_then(|j| o[code].map(|o| j + o));
                    }
                    if keep(j) {
                        s.idx[k] = p;
                        s.val[k] = s.val[i];
                        k += 1;
                    }
                }
                s.idx.truncate(k);
                s.val.truncate(k);
                Data::Sparse(s)
            }
            Data::Datetime(mut v) => {
                for (x, j) in v.iter_mut().zip(map.iter()) {
                    if !keep(j) {
                        *x = super::data::NAT;
                    }
                }
                Data::Datetime(v)
            }
            Data::Object(mut v) => {
                for (x, j) in v.iter_mut().zip(map.iter()) {
                    if !keep(j) {
                        *x = Obj::Nan;
                    }
                }
                Data::Object(v)
            }
            Data::Bool(v) => Data::Float(
                v.iter()
                    .zip(map.iter())
                    .map(|(x, j)| {
                        if keep(j) {
                            f64::from(u8::from(*x))
                        } else {
                            f64::NAN
                        }
                    })
                    .collect(),
            ),
            #[allow(clippy::cast_precision_loss)]
            Data::Int(v) => Data::Float(
                v.iter()
                    .zip(map.iter())
                    .map(|(x, j)| if keep(j) { *x as f64 } else { f64::NAN })
                    .collect(),
            ),
        };
        DataArray::new(dims, data)
    }

    /// `where(cond)` through a full broadcast (when `cond` adds dimensions).
    fn where_generic(&self, cond: &DataArray) -> DataArray {
        let (dims, a, c) = DataArray::broadcast_pair(self, cond, Join::Inner);
        let mask = c.truthy();
        let idx: Vec<Option<usize>> = mask
            .iter()
            .enumerate()
            .map(|(i, m)| m.then_some(i))
            .collect();
        let fill = a.dtype().with_missing();
        let mut data = a.data.gather(&idx, fill);
        if data.dtype() != fill {
            data = data.cast(fill).unwrap_or(data);
        }
        DataArray::new(dims, data)
    }

    /// `where(cond, other)`: values where `cond` is true, `other` elsewhere.
    pub fn where_else(&self, cond: &DataArray, other: &DataArray) -> DataArray {
        let (dims, a, c) = DataArray::broadcast_pair(self, cond, Join::Inner);
        let (dims, a, o) = {
            let a2 = DataArray::new(dims, a.data);
            let (d, a3, o) = DataArray::broadcast_pair(&a2, other, Join::Inner);
            (d, a3, o)
        };
        let c = c.conform(&dims, DType::Float);
        let mask = c.truthy();
        let dtype = a.dtype().result_type(o.dtype());
        let objs: Vec<Obj> = (0..a.data.len())
            .map(|i| {
                if mask[i] {
                    a.data.get(i)
                } else {
                    o.data.get(i)
                }
            })
            .collect();
        DataArray::new(dims, Data::from_objs(dtype, objs))
    }

    /// `fillna(other)`: missing values taken from `other` (left join).
    pub fn fillna(&self, other: &DataArray) -> DataArray {
        self.clone().fillna_into(other)
    }

    /// [`DataArray::fillna`] by value: typed values are filled in place.
    pub fn fillna_into(self, other: &DataArray) -> DataArray {
        let (dims, a, o) = DataArray::broadcast_pair_into(self, other, Join::Left);
        let dtype = a.dtype().result_type(o.dtype());
        let data = match (a.data, o.data) {
            (Data::Float(mut x), Data::Float(y)) => {
                for (x, y) in x.iter_mut().zip(y) {
                    if x.is_nan() {
                        *x = y;
                    }
                }
                Data::Float(x)
            }
            (Data::Float(mut x), Data::Sparse(y)) => {
                for (i, v) in y.iter() {
                    if x[i].is_nan() {
                        x[i] = v;
                    }
                }
                Data::Float(x)
            }
            (Data::Sparse(x), Data::Float(mut y)) => {
                for (i, v) in x.iter() {
                    y[i] = v;
                }
                Data::Float(y)
            }
            (Data::Sparse(x), Data::Sparse(y)) => {
                if y.nnz() == 0 {
                    Data::Sparse(x)
                } else {
                    let len = x.len;
                    let mut pairs: Vec<(usize, f64)> = y.iter().collect();
                    pairs.extend(x.iter()); // later wins: `x` over `y`
                    Data::Sparse(SparseF64::from_pairs(len, pairs))
                }
            }
            // No missing values on the left.
            (Data::Bool(x), Data::Bool(_)) => Data::Bool(x),
            (Data::Int(x), Data::Int(_)) => Data::Int(x),
            (Data::Datetime(mut x), Data::Datetime(y)) => {
                for (x, y) in x.iter_mut().zip(y) {
                    if *x == super::data::NAT {
                        *x = y;
                    }
                }
                Data::Datetime(x)
            }
            (x, y) => {
                let objs: Vec<Obj> = (0..x.len())
                    .map(|i| if x.is_null(i) { y.get(i) } else { x.get(i) })
                    .collect();
                Data::from_objs(dtype, objs)
            }
        };
        DataArray::new(dims, data)
    }

    /// `fillna(scalar)`.
    pub fn fillna_scalar(&self, v: Obj) -> DataArray {
        self.fillna(&DataArray::scalar(v))
    }

    /// Reduces `dims` with `f` over the values of each group (in C order).
    pub fn reduce(&self, dims: &[&str], f: impl Fn(&[Obj]) -> Obj, out: DType) -> DataArray {
        let keep: Vec<Coord> = self
            .coords
            .iter()
            .filter(|c| !dims.contains(&c.name()))
            .cloned()
            .collect();
        let keep_shape: Vec<usize> = keep.iter().map(Coord::len).collect();
        let keep_strides = strides(&keep_shape);
        let n_out: usize = keep_shape.iter().product();
        let mut groups: Vec<Vec<Obj>> = vec![Vec::new(); n_out];
        let shape = self.shape();
        let total: usize = shape.iter().product();
        let keep_axes: Vec<Option<usize>> = self
            .coords
            .iter()
            .map(|c| keep.iter().position(|k| k.name() == c.name()))
            .collect();
        let mut idx = vec![0usize; shape.len()];
        for i in 0..total {
            let mut o = 0;
            for (ax, ka) in keep_axes.iter().enumerate() {
                if let Some(ka) = ka {
                    o += idx[ax] * keep_strides[*ka];
                }
            }
            groups[o].push(self.data.get(i));
            for ax in (0..idx.len()).rev() {
                idx[ax] += 1;
                if idx[ax] < shape[ax] {
                    break;
                }
                idx[ax] = 0;
            }
        }
        let objs: Vec<Obj> = groups.iter().map(|g| f(g)).collect();
        DataArray::new(keep, Data::from_objs(out, objs))
    }

    /// `any(dims)`: true if any value is truthy (NaN is truthy, as in numpy).
    pub fn any(&self, dims: &[&str]) -> DataArray {
        self.reduce(dims, |g| Obj::Bool(g.iter().any(Obj::truthy)), DType::Bool)
    }

    /// `sum(dims)` with `skipna` and `min_count`.
    pub fn sum(&self, dims: &[&str], min_count: usize) -> DataArray {
        self.reduce(
            dims,
            |g| {
                let vals: Vec<f64> = g
                    .iter()
                    .filter(|o| !o.is_null())
                    .filter_map(Obj::as_f64)
                    .collect();
                if vals.len() < min_count {
                    Obj::Nan
                } else {
                    Obj::float(vals.iter().sum())
                }
            },
            DType::Float,
        )
    }

    /// Selects labels along a dimension (`sel(dim=[...])`); unknown labels are an error.
    pub fn sel(&self, dim: &str, labels: &[Label]) -> Result<DataArray, String> {
        let Some(c) = self.coord(dim) else {
            return Ok(self.clone());
        };
        for l in labels {
            if c.code(l).is_none() {
                return Err(format!("{l} not found in `{dim}`"));
            }
        }
        let new = Coord::new(dim, labels.to_vec()).map_err(|e| e.to_string())?;
        Ok(self.reindex(&new))
    }

    /// Element-wise map to a new dtype.
    pub fn map(&self, f: impl Fn(Obj) -> Obj, out: DType) -> DataArray {
        let objs: Vec<Obj> = (0..self.data.len()).map(|i| f(self.data.get(i))).collect();
        DataArray::new(self.coords.clone(), Data::from_objs(out, objs))
    }

    /// Element-wise binary operation (inner join, broadcasting).
    pub fn zip_with(
        &self,
        other: &DataArray,
        f: impl Fn(Obj, Obj) -> Obj,
        out: DType,
    ) -> DataArray {
        let (dims, a, b) = DataArray::broadcast_pair(self, other, Join::Inner);
        let n = a.data.len();
        let cell = |i: usize| f(a.data.get(i), b.data.get(i));
        // Typed results without an `object` vector where the cast cannot fail.
        let data = match out {
            DType::Bool => Data::Bool((0..n).map(|i| cell(i).truthy()).collect()),
            DType::Float => {
                let mut v = Vec::with_capacity(n);
                for i in 0..n {
                    match cell(i) {
                        o @ (Obj::Nan | Obj::Bool(_) | Obj::Int(_) | Obj::Float(_)) => {
                            v.push(o.as_f64().unwrap_or(f64::NAN));
                        }
                        _ => {
                            v.clear();
                            break;
                        }
                    }
                }
                if v.len() == n {
                    Data::Float(v)
                } else {
                    Data::from_objs(out, (0..n).map(cell).collect())
                }
            }
            _ => Data::from_objs(out, (0..n).map(cell).collect()),
        };
        DataArray::new(dims, data)
    }

    /// True if every value is missing.
    pub fn all_null(&self) -> bool {
        self.data.all_null()
    }

    /// The single value of a 1-cell array.
    pub fn item(&self) -> Obj {
        self.data.get(0)
    }
}
