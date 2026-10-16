//! xarray operations on [`LArray`].
//!
//! The implementations work cell by cell with hashing. They are the reference semantics; the
//! math engine adds fast paths for its hot loops (lookups at `foreach` cells, ADR 0001).

use std::collections::{BTreeMap, HashMap};

use crate::array::{Element, LArray, increment};
use crate::coord::{ArrayError, Coord, Join, Label};

/// Remaining dims, their positions, and the cells grouped by their codes on those dims.
type Grouped<'a, T> = (Vec<Coord>, Vec<usize>, BTreeMap<Vec<u32>, Vec<&'a T>>);

/// For each dimension of an operand, the map from result codes to operand codes.
struct Alignment {
    dims: Vec<Coord>,
    /// `a_axes[i]`: position in the result of operand dim `i`.
    a_axes: Vec<usize>,
    b_axes: Vec<usize>,
    /// Per operand dim: operand code → result code (None if dropped by the join).
    a_fwd: Vec<Vec<Option<u32>>>,
    b_fwd: Vec<Vec<Option<u32>>>,
    /// Per result dim: result code → operand code (None if the label is not in the operand,
    /// or the operand lacks the dim).
    a_back: Vec<Option<Vec<Option<u32>>>>,
    b_back: Vec<Option<Vec<Option<u32>>>>,
}

fn code_maps(from: &Coord, to: &Coord) -> (Vec<Option<u32>>, Vec<Option<u32>>) {
    let fwd = from.labels().iter().map(|l| to.code(l)).collect();
    let back = to.labels().iter().map(|l| from.code(l)).collect();
    (fwd, back)
}

fn align(a: &[Coord], b: &[Coord], join: Join) -> Alignment {
    let mut dims: Vec<Coord> = a
        .iter()
        .map(|d| match b.iter().find(|e| e.name() == d.name()) {
            Some(e) => d.join(e, join),
            None => d.clone(),
        })
        .collect();
    dims.extend(
        b.iter()
            .filter(|e| !a.iter().any(|d| d.name() == e.name()))
            .cloned(),
    );
    let pos = |name: &str| {
        dims.iter()
            .position(|d| d.name() == name)
            .expect("in result")
    };
    let a_axes: Vec<usize> = a.iter().map(|d| pos(d.name())).collect();
    let b_axes: Vec<usize> = b.iter().map(|d| pos(d.name())).collect();
    let mut a_fwd = Vec::new();
    let mut a_back = vec![None; dims.len()];
    for (d, &ax) in a.iter().zip(&a_axes) {
        let (f, bk) = code_maps(d, &dims[ax]);
        a_fwd.push(f);
        a_back[ax] = Some(bk);
    }
    let mut b_fwd = Vec::new();
    let mut b_back = vec![None; dims.len()];
    for (d, &ax) in b.iter().zip(&b_axes) {
        let (f, bk) = code_maps(d, &dims[ax]);
        b_fwd.push(f);
        b_back[ax] = Some(bk);
    }
    Alignment {
        dims,
        a_axes,
        b_axes,
        a_fwd,
        b_fwd,
        a_back,
        b_back,
    }
}

/// Maps operand codes to result codes (None if a label was dropped).
fn forward(codes: &[u32], fwd: &[Vec<Option<u32>>]) -> Option<Vec<u32>> {
    codes.iter().zip(fwd).map(|(c, m)| m[*c as usize]).collect()
}

/// Maps result codes back to an operand's codes (None if not in the operand).
fn backward(codes: &[u32], back: &[Option<Vec<Option<u32>>>], axes: &[usize]) -> Option<Vec<u32>> {
    axes.iter()
        .map(|&ax| back[ax].as_ref().and_then(|m| m[codes[ax] as usize]))
        .collect()
}

fn product(dims: &[Coord], axes: &[usize]) -> Vec<Vec<u32>> {
    let shape: Vec<usize> = axes.iter().map(|&a| dims[a].len()).collect();
    let total: usize = shape.iter().product();
    let mut out = Vec::with_capacity(total);
    let mut idx = vec![0u32; axes.len()];
    for _ in 0..total {
        out.push(idx.clone());
        increment(&mut idx, &shape);
    }
    out
}

impl<T: Element> LArray<T> {
    /// A binary operation where a missing operand gives a missing result (NaN propagation),
    /// broadcasting by dimension name. Result dims: `self`'s, then `other`'s new ones.
    pub fn zip_with<U: Element, V: Element>(
        &self,
        other: &LArray<U>,
        join: Join,
        f: impl Fn(&T, &U) -> V,
    ) -> LArray<V> {
        let al = align(&self.dims, &other.dims, join);
        let shared: Vec<usize> = al
            .b_axes
            .iter()
            .copied()
            .filter(|ax| al.a_axes.contains(ax))
            .collect();
        let b_only: Vec<usize> = al
            .b_axes
            .iter()
            .copied()
            .filter(|ax| !al.a_axes.contains(ax))
            .collect();
        let mut index: HashMap<Vec<u32>, Vec<(Vec<u32>, &U)>> = HashMap::new();
        for (codes, v) in other.cells() {
            let Some(rc) = forward(&codes, &al.b_fwd) else {
                continue;
            };
            let mut full = vec![0u32; al.dims.len()];
            for (ax, c) in al.b_axes.iter().zip(&rc) {
                full[*ax] = *c;
            }
            let key: Vec<u32> = shared.iter().map(|&ax| full[ax]).collect();
            let rest: Vec<u32> = b_only.iter().map(|&ax| full[ax]).collect();
            index.entry(key).or_default().push((rest, v));
        }
        let mut cells = Vec::new();
        for (codes, av) in self.cells() {
            let Some(rc) = forward(&codes, &al.a_fwd) else {
                continue;
            };
            let mut full = vec![0u32; al.dims.len()];
            for (ax, c) in al.a_axes.iter().zip(&rc) {
                full[*ax] = *c;
            }
            let key: Vec<u32> = shared.iter().map(|&ax| full[ax]).collect();
            if let Some(matches) = index.get(&key) {
                for (rest, bv) in matches {
                    let mut out = full.clone();
                    for (ax, c) in b_only.iter().zip(rest) {
                        out[*ax] = *c;
                    }
                    let v = f(av, bv);
                    if !v.is_missing() {
                        cells.push((out, v));
                    }
                }
            }
        }
        LArray::from_cells(al.dims, cells).expect("aligned cells are valid")
    }

    /// A binary operation defined wherever either operand has a value (`fillna`, `|`).
    /// Each side is broadcast over the dimensions it lacks.
    pub fn zip_union<U: Element, V: Element>(
        &self,
        other: &LArray<U>,
        join: Join,
        f: impl Fn(Option<&T>, Option<&U>) -> V,
    ) -> LArray<V> {
        let al = align(&self.dims, &other.dims, join);
        let b_only: Vec<usize> = (0..al.dims.len())
            .filter(|ax| !al.a_axes.contains(ax))
            .collect();
        let a_only: Vec<usize> = (0..al.dims.len())
            .filter(|ax| !al.b_axes.contains(ax))
            .collect();
        let mut out: BTreeMap<Vec<u32>, V> = BTreeMap::new();
        let lookup_b = |full: &[u32]| -> Option<U> {
            backward(full, &al.b_back, &al.b_axes)
                .map(|c| other.get(&c))
                .filter(|v| !v.is_missing())
        };
        let lookup_a = |full: &[u32]| -> Option<T> {
            backward(full, &al.a_back, &al.a_axes)
                .map(|c| self.get(&c))
                .filter(|v| !v.is_missing())
        };
        let b_combos = product(&al.dims, &b_only);
        for (codes, av) in self.cells() {
            let Some(rc) = forward(&codes, &al.a_fwd) else {
                continue;
            };
            for combo in &b_combos {
                let mut full = vec![0u32; al.dims.len()];
                for (ax, c) in al.a_axes.iter().zip(&rc) {
                    full[*ax] = *c;
                }
                for (ax, c) in b_only.iter().zip(combo) {
                    full[*ax] = *c;
                }
                let bv = lookup_b(&full);
                let v = f(Some(av), bv.as_ref());
                out.insert(full, v);
            }
        }
        let a_combos = product(&al.dims, &a_only);
        for (codes, bv) in other.cells() {
            let Some(rc) = forward(&codes, &al.b_fwd) else {
                continue;
            };
            for combo in &a_combos {
                let mut full = vec![0u32; al.dims.len()];
                for (ax, c) in al.b_axes.iter().zip(&rc) {
                    full[*ax] = *c;
                }
                for (ax, c) in a_only.iter().zip(combo) {
                    full[*ax] = *c;
                }
                if out.contains_key(&full) {
                    continue;
                }
                let av = lookup_a(&full);
                out.insert(full, f(av.as_ref(), Some(bv)));
            }
        }
        LArray::from_cells(al.dims, out.into_iter().filter(|(_, v)| !v.is_missing()))
            .expect("aligned cells are valid")
    }

    /// `where(cond)`: keeps values where `cond` is true; elsewhere missing. Broadcasts by name
    /// with an inner join, as xarray does when `other` is not given.
    pub fn where_(&self, cond: &LArray<bool>) -> LArray<T> {
        self.zip_with(
            cond,
            Join::Inner,
            |v, c| if *c { v.clone() } else { T::missing() },
        )
    }

    /// `where(cond, other)` with a scalar `other`: defined on the whole product of the
    /// broadcast dims.
    pub fn where_or(&self, cond: &LArray<bool>, other: T) -> LArray<T> {
        let al = align(&self.dims, &cond.dims, Join::Inner);
        let all: Vec<usize> = (0..al.dims.len()).collect();
        let cells = product(&al.dims, &all).into_iter().filter_map(|full| {
            let c = backward(&full, &al.b_back, &al.b_axes).is_some_and(|c| cond.get(&c));
            let v = if c {
                backward(&full, &al.a_back, &al.a_axes).map_or_else(T::missing, |c| self.get(&c))
            } else {
                other.clone()
            };
            (!v.is_missing()).then_some((full, v))
        });
        LArray::from_cells(al.dims, cells.collect::<Vec<_>>()).expect("valid cells")
    }

    /// `fillna(value)`: every missing cell of the full product becomes `value`.
    pub fn fillna(&self, value: T) -> LArray<T> {
        if value.is_missing() {
            return self.clone();
        }
        let filled: Vec<T> = self
            .to_dense()
            .into_iter()
            .map(|v| if v.is_missing() { value.clone() } else { v })
            .collect();
        LArray::from_dense(self.dims.clone(), filled).expect("same shape")
    }

    /// `fillna(other)` with an array: missing cells take `other`'s value (outer broadcast).
    pub fn fillna_with(&self, other: &LArray<T>) -> LArray<T> {
        self.zip_union(other, Join::Left, |a, b| match (a, b) {
            (Some(a), _) => a.clone(),
            (None, Some(b)) => b.clone(),
            (None, None) => T::missing(),
        })
    }

    /// Groups non-missing cells by the codes of the dims that remain after removing `over`.
    fn group_cells(&self, over: &[&str]) -> Result<Grouped<'_, T>, ArrayError> {
        for d in over {
            self.axis(d)?;
        }
        let keep: Vec<usize> = (0..self.dims.len())
            .filter(|&i| !over.contains(&self.dims[i].name()))
            .collect();
        let dims = keep.iter().map(|&i| self.dims[i].clone()).collect();
        let mut groups: BTreeMap<Vec<u32>, Vec<&T>> = BTreeMap::new();
        for (codes, v) in self.cells() {
            groups
                .entry(keep.iter().map(|&i| codes[i]).collect())
                .or_default()
                .push(v);
        }
        Ok((dims, keep, groups))
    }

    /// Reduces over the dimensions in `over`: groups the non-missing cells by their codes on
    /// the remaining dimensions and applies `f` to each group (cells in storage order). Groups
    /// without cells are missing (`skipna=True, min_count=1`).
    pub fn reduce<U: Element>(
        &self,
        over: &[&str],
        f: impl Fn(&[&T]) -> U,
    ) -> Result<LArray<U>, ArrayError> {
        let (dims, _, groups) = self.group_cells(over)?;
        let cells: Vec<(Vec<u32>, U)> = groups.into_iter().map(|(k, vs)| (k, f(&vs))).collect();
        LArray::from_cells(dims, cells)
    }

    /// `roll(dim=shift)` (numpy semantics; coordinates stay in place).
    pub fn roll(&self, dim: &str, shift: i64) -> Result<LArray<T>, ArrayError> {
        let ax = self.axis(dim)?;
        let n = i64::try_from(self.dims[ax].len()).unwrap_or(i64::MAX);
        if n == 0 {
            return Ok(self.clone());
        }
        let cells: Vec<(Vec<u32>, T)> = self
            .cells()
            .map(|(mut codes, v)| {
                let c = (i64::from(codes[ax]) + shift).rem_euclid(n);
                codes[ax] = u32::try_from(c).expect("fits");
                (codes, v.clone())
            })
            .collect();
        LArray::from_cells(self.dims.clone(), cells)
    }

    /// `sel` along one dimension.
    pub fn sel(&self, dim: &str, selection: &Selection) -> Result<LArray<T>, ArrayError> {
        let ax = self.axis(dim)?;
        let coord = &self.dims[ax];
        let labels: Vec<Label> = match selection {
            Selection::Labels(ls) => {
                for l in ls {
                    if coord.code(l).is_none() {
                        return Err(ArrayError::MissingLabel {
                            dim: dim.to_owned(),
                            label: l.to_string(),
                        });
                    }
                }
                ls.clone()
            }
            Selection::Slice(start, end) => {
                let (lo, hi) = slice_bounds(coord, start.as_ref(), end.as_ref())?;
                coord.labels()[lo..hi].to_vec()
            }
        };
        let new = Coord::new(dim, labels)?;
        let mut dims = self.dims.clone();
        dims[ax] = new.clone();
        let map: Vec<Option<u32>> = coord.labels().iter().map(|l| new.code(l)).collect();
        let cells: Vec<(Vec<u32>, T)> = self
            .cells()
            .filter_map(|(mut codes, v)| {
                codes[ax] = map[codes[ax] as usize]?;
                Some((codes, v.clone()))
            })
            .collect();
        LArray::from_cells(dims, cells)
    }

    /// `sel` of a single label: the dimension is dropped.
    pub fn sel_one(&self, dim: &str, label: &Label) -> Result<LArray<T>, ArrayError> {
        let ax = self.axis(dim)?;
        let code = self.dims[ax]
            .code(label)
            .ok_or_else(|| ArrayError::MissingLabel {
                dim: dim.to_owned(),
                label: label.to_string(),
            })?;
        let mut dims = self.dims.clone();
        dims.remove(ax);
        let cells: Vec<(Vec<u32>, T)> = self
            .cells()
            .filter(|(codes, _)| codes[ax] == code)
            .map(|(mut codes, v)| {
                codes.remove(ax);
                (codes, v.clone())
            })
            .collect();
        LArray::from_cells(dims, cells)
    }

    /// Renames a dimension.
    pub fn rename(&self, dim: &str, new: &str) -> Result<LArray<T>, ArrayError> {
        let ax = self.axis(dim)?;
        let mut out = self.clone();
        out.dims[ax] = self.dims[ax].renamed(new);
        Ok(out)
    }

    /// `xr.concat(arrays, dim=new)`: stacks arrays with the same dims along a new first
    /// dimension; other coordinates are outer-joined.
    pub fn concat(arrays: &[LArray<T>], new: Coord) -> Result<LArray<T>, ArrayError> {
        if arrays.len() != new.len() {
            return Err(ArrayError::Shape(format!(
                "{} arrays for {} labels of `{}`",
                arrays.len(),
                new.len(),
                new.name()
            )));
        }
        let Some(first) = arrays.first() else {
            return LArray::from_cells(vec![new], Vec::new());
        };
        let mut dims: Vec<Coord> = first.dims.clone();
        for a in &arrays[1..] {
            if a.dims.len() != dims.len() {
                return Err(ArrayError::Shape(
                    "concat needs arrays with the same dims".into(),
                ));
            }
            for d in &mut dims {
                let other = a
                    .coord(d.name())
                    .ok_or_else(|| ArrayError::MissingDim(d.name().to_owned()))?;
                *d = d.join(other, Join::Outer);
            }
        }
        let mut cells = Vec::new();
        for (i, a) in arrays.iter().enumerate() {
            let fwd: Vec<(usize, Vec<Option<u32>>)> = dims
                .iter()
                .map(|d| {
                    let ax = a.axis(d.name()).expect("checked");
                    (ax, code_maps(&a.dims[ax], d).0)
                })
                .collect();
            for (codes, v) in a.cells() {
                let mut out = vec![u32::try_from(i).expect("fits")];
                for (ax, m) in &fwd {
                    out.push(m[codes[*ax] as usize].expect("outer join keeps labels"));
                }
                cells.push((out, v.clone()));
            }
        }
        let mut all = vec![new];
        all.extend(dims);
        LArray::from_cells(all, cells)
    }
}

impl LArray<f64> {
    /// `sum(over, skipna=True, min_count=...)`. With `min_count = 0` an empty group sums to 0
    /// (over the full product of the remaining dims); otherwise groups with fewer values are
    /// NaN.
    pub fn sum(&self, over: &[&str], min_count: usize) -> Result<LArray<f64>, ArrayError> {
        let (dims, _, groups) = self.group_cells(over)?;
        let cells: Vec<(Vec<u32>, f64)> = groups
            .into_iter()
            .filter(|(_, vs)| vs.len() >= min_count.max(1))
            .map(|(k, vs)| (k, vs.into_iter().sum()))
            .collect();
        let out = LArray::from_cells(dims, cells)?;
        Ok(if min_count == 0 { out.fillna(0.0) } else { out })
    }

    /// `mean(over, skipna=True)`.
    pub fn mean(&self, over: &[&str]) -> Result<LArray<f64>, ArrayError> {
        let (dims, _, groups) = self.group_cells(over)?;
        #[allow(clippy::cast_precision_loss)]
        let cells: Vec<(Vec<u32>, f64)> = groups
            .into_iter()
            .map(|(k, vs)| {
                let n = vs.len() as f64;
                (k, vs.into_iter().sum::<f64>() / n)
            })
            .collect();
        LArray::from_cells(dims, cells)
    }

    /// `group_sum` (Calliope helper): sums `self` over the cells that share a group label in
    /// `groupby` (whose dims are a subset of `self`'s), giving a new first dimension
    /// `group_dim` with the sorted group labels. Cells with no group are dropped; groups sum
    /// with `min_count=1`.
    pub fn group_sum(
        &self,
        groupby: &LArray<Option<Label>>,
        group_dim: &str,
    ) -> Result<LArray<f64>, ArrayError> {
        let g_axes: Vec<usize> = groupby
            .dims
            .iter()
            .map(|d| self.axis(d.name()))
            .collect::<Result<_, _>>()?;
        // Map self codes on the grouping dims to groupby codes.
        let maps: Vec<Vec<Option<u32>>> = groupby
            .dims
            .iter()
            .zip(&g_axes)
            .map(|(gd, &ax)| code_maps(&self.dims[ax], gd).0)
            .collect();
        let mut labels: Vec<Label> = groupby.cells().filter_map(|(_, v)| v.clone()).collect();
        labels.sort();
        labels.dedup();
        if labels.is_empty() {
            return Err(ArrayError::Shape(
                "Failed to group data: the grouping array is all missing".into(),
            ));
        }
        let gcoord = Coord::new(group_dim, labels)?;
        let keep: Vec<usize> = (0..self.dims.len())
            .filter(|i| !g_axes.contains(i))
            .collect();
        let mut sums: BTreeMap<Vec<u32>, f64> = BTreeMap::new();
        for (codes, v) in self.cells() {
            let gcodes: Option<Vec<u32>> = g_axes
                .iter()
                .zip(&maps)
                .map(|(&ax, m)| m[codes[ax] as usize])
                .collect();
            let Some(gcodes) = gcodes else { continue };
            let Some(label) = groupby.get(&gcodes) else {
                continue;
            };
            let g = gcoord.code(&label).expect("collected");
            let mut key = vec![g];
            key.extend(keep.iter().map(|&i| codes[i]));
            *sums.entry(key).or_insert(0.0) += v;
        }
        let mut dims = vec![gcoord];
        dims.extend(keep.iter().map(|&i| self.dims[i].clone()));
        LArray::from_cells(dims, sums)
    }

    /// `resample(dim=freq).<method>()` on a datetime dimension with a fixed frequency (in
    /// nanoseconds). Bins start at midnight of the first label (pandas `origin="start_day"`),
    /// are closed and labelled on the left, and empty bins are NaN. `sum` uses
    /// `min_count=1`, as Calliope does.
    pub fn resample(
        &self,
        dim: &str,
        freq_ns: i64,
        method: Resample,
    ) -> Result<LArray<f64>, ArrayError> {
        let ax = self.axis(dim)?;
        let coord = &self.dims[ax];
        let (new, bin_of) = resample_bins(coord, freq_ns)?;
        let mut dims = self.dims.clone();
        dims[ax] = new;
        let mut groups: BTreeMap<Vec<u32>, Vec<f64>> = BTreeMap::new();
        for (mut codes, v) in self.cells() {
            codes[ax] = bin_of[codes[ax] as usize];
            groups.entry(codes).or_default().push(*v);
        }
        #[allow(clippy::cast_precision_loss)]
        let cells: Vec<(Vec<u32>, f64)> = groups
            .into_iter()
            .map(|(k, vs)| {
                let v = match method {
                    Resample::First => vs[0],
                    Resample::Sum => vs.iter().sum(),
                    Resample::Mean => vs.iter().sum::<f64>() / vs.len() as f64,
                };
                (k, v)
            })
            .collect();
        LArray::from_cells(dims, cells)
    }
}

impl LArray<bool> {
    /// `any(over)`.
    pub fn any(&self, over: &[&str]) -> Result<LArray<bool>, ArrayError> {
        let (dims, _, groups) = self.group_cells(over)?;
        LArray::from_cells(
            dims,
            groups.into_keys().map(|k| (k, true)).collect::<Vec<_>>(),
        )
    }

    /// Element-wise OR (missing = false), outer broadcast.
    pub fn or(&self, other: &LArray<bool>) -> LArray<bool> {
        self.zip_union(other, Join::Outer, |a, b| {
            a.copied().unwrap_or(false) || b.copied().unwrap_or(false)
        })
    }

    /// Element-wise AND (missing = false).
    pub fn and(&self, other: &LArray<bool>) -> LArray<bool> {
        self.zip_with(other, Join::Inner, |a, b| *a && *b)
    }

    /// Element-wise NOT over the full product.
    pub fn not(&self) -> LArray<bool> {
        let dense: Vec<bool> = self.to_dense().into_iter().map(|b| !b).collect();
        LArray::from_dense(self.dims.clone(), dense).expect("same shape")
    }
}

impl LArray<Option<Label>> {
    /// The labels of a coordinate as a 1-d array (an xarray dimension coordinate).
    pub fn from_coord(coord: &Coord) -> LArray<Option<Label>> {
        let cells: Vec<(Vec<u32>, Option<Label>)> = coord
            .labels()
            .iter()
            .enumerate()
            .map(|(i, l)| (vec![u32::try_from(i).expect("fits")], Some(l.clone())))
            .collect();
        LArray::from_cells(vec![coord.clone()], cells).expect("valid cells")
    }

    /// `isin(values)`.
    pub fn isin(&self, values: &[Label]) -> LArray<bool> {
        self.map(|v| v.as_ref().is_some_and(|l| values.contains(l)))
            .compact()
    }
}

/// A `sel` along one dimension.
#[derive(Debug, Clone, PartialEq)]
pub enum Selection {
    /// These labels, in this order.
    Labels(Vec<Label>),
    /// A label slice, inclusive at both ends (`None` = open).
    Slice(Option<Label>, Option<Label>),
}

/// Aggregation for [`LArray::resample`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resample {
    /// First non-missing value of the bin.
    First,
    /// Mean of the bin, skipping missing values.
    Mean,
    /// Sum of the bin with `min_count=1`.
    Sum,
}

fn slice_bounds(
    coord: &Coord,
    start: Option<&Label>,
    end: Option<&Label>,
) -> Result<(usize, usize), ArrayError> {
    let sorted = coord.labels().windows(2).all(|w| w[0] <= w[1]);
    let missing = |l: &Label| ArrayError::MissingLabel {
        dim: coord.name().to_owned(),
        label: l.to_string(),
    };
    let lo = match start {
        None => 0,
        Some(l) => match coord.code(l) {
            Some(c) => c as usize,
            None if sorted => coord.labels().partition_point(|x| x < l),
            None => return Err(missing(l)),
        },
    };
    let hi = match end {
        None => coord.len(),
        Some(l) => match coord.code(l) {
            Some(c) => c as usize + 1,
            None if sorted => coord.labels().partition_point(|x| x <= l),
            None => return Err(missing(l)),
        },
    };
    Ok((lo, hi.max(lo)))
}

const DAY_NS: i64 = 86_400_000_000_000;

fn resample_bins(coord: &Coord, freq_ns: i64) -> Result<(Coord, Vec<u32>), ArrayError> {
    if freq_ns <= 0 {
        return Err(ArrayError::Shape(
            "resampling frequency must be positive".into(),
        ));
    }
    let times: Vec<i64> = coord
        .labels()
        .iter()
        .map(|l| match l {
            Label::Datetime(t) => Ok(*t),
            other => Err(ArrayError::Shape(format!(
                "`{}` is not a datetime dimension ({other})",
                coord.name()
            ))),
        })
        .collect::<Result<_, _>>()?;
    let (Some(&first), Some(&last)) = (times.iter().min(), times.iter().max()) else {
        return Ok((coord.clone(), Vec::new()));
    };
    let origin = first.div_euclid(DAY_NS) * DAY_NS;
    let bin = |t: i64| (t - origin).div_euclid(freq_ns);
    let (b0, b1) = (bin(first), bin(last));
    let labels: Vec<Label> = (b0..=b1)
        .map(|b| Label::Datetime(origin + b * freq_ns))
        .collect();
    let new = Coord::new(coord.name(), labels)?;
    let bin_of = times
        .iter()
        .map(|&t| u32::try_from(bin(t) - b0).expect("fits"))
        .collect();
    Ok((new, bin_of))
}
