//! Values and typed value vectors with numpy's dtypes and pandas/xarray missing-value rules.

use std::cmp::Ordering;
use std::fmt;
use std::sync::Arc;

use calliope_array::Label;

/// One value of an `object` array (or any array, read cell by cell).
#[derive(Debug, Clone)]
pub enum Obj {
    /// Missing (`NaN`, `None`, `NaT`).
    Nan,
    /// A bool.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float (never NaN: NaN is [`Obj::Nan`]).
    Float(f64),
    /// A string.
    Str(Arc<str>),
    /// A datetime in nanoseconds since the epoch.
    Datetime(i64),
}

impl Obj {
    /// A string value.
    pub fn str(s: &str) -> Self {
        Obj::Str(Arc::from(s))
    }

    /// A float; NaN becomes [`Obj::Nan`].
    pub fn float(f: f64) -> Self {
        if f.is_nan() { Obj::Nan } else { Obj::Float(f) }
    }

    /// True for the missing value.
    pub fn is_null(&self) -> bool {
        matches!(self, Obj::Nan)
    }

    /// The value as a float (`True` = 1), if numeric.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Obj::Nan => Some(f64::NAN),
            Obj::Bool(b) => Some(f64::from(u8::from(*b))),
            #[allow(clippy::cast_precision_loss)]
            Obj::Int(i) => Some(*i as f64),
            Obj::Float(f) => Some(*f),
            Obj::Str(_) | Obj::Datetime(_) => None,
        }
    }

    /// The string, if a string.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Obj::Str(s) => Some(s),
            _ => None,
        }
    }

    /// Python truthiness (`bool(x)`; NaN is truthy, as in numpy).
    pub fn truthy(&self) -> bool {
        match self {
            Obj::Nan => true,
            Obj::Bool(b) => *b,
            Obj::Int(i) => *i != 0,
            Obj::Float(f) => *f != 0.0,
            Obj::Str(s) => !s.is_empty(),
            Obj::Datetime(_) => true,
        }
    }

    /// Python `==` (`1 == 1.0 == True`; NaN is never equal).
    pub fn py_eq(&self, other: &Obj) -> bool {
        match (self, other) {
            (Obj::Nan, _) | (_, Obj::Nan) => false,
            (Obj::Str(a), Obj::Str(b)) => a == b,
            (Obj::Datetime(a), Obj::Datetime(b)) => a == b,
            (a, b) => match (a.as_f64(), b.as_f64()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }

    /// `str(x)` as numpy's `astype(str)` writes it.
    pub fn to_py_str(&self) -> String {
        match self {
            Obj::Nan => "nan".to_owned(),
            Obj::Bool(b) => if *b { "True" } else { "False" }.to_owned(),
            Obj::Int(i) => i.to_string(),
            Obj::Float(f) => calliope_yaml::python_float(*f),
            Obj::Str(s) => s.to_string(),
            Obj::Datetime(ns) => format_datetime(*ns),
        }
    }

    /// The value as a coordinate label.
    pub fn to_label(&self) -> Label {
        match self {
            Obj::Nan => Label::Float(f64::NAN),
            Obj::Bool(b) => Label::Int(i64::from(*b)),
            Obj::Int(i) => Label::Int(*i),
            Obj::Float(f) => Label::Float(*f),
            Obj::Str(s) => Label::Str(s.clone()),
            Obj::Datetime(d) => Label::Datetime(*d),
        }
    }

    /// A coordinate label as a value.
    pub fn from_label(l: &Label) -> Obj {
        match l {
            Label::Str(s) => Obj::Str(s.clone()),
            Label::Int(i) => Obj::Int(*i),
            Label::Float(f) => Obj::float(*f),
            Label::Datetime(d) => Obj::Datetime(*d),
        }
    }

    fn rank(&self) -> u8 {
        match self {
            Obj::Nan => 4,
            Obj::Bool(_) | Obj::Int(_) | Obj::Float(_) => 0,
            Obj::Datetime(_) => 1,
            Obj::Str(_) => 2,
        }
    }

    /// Total order used for sorting values (numbers, datetimes, strings, missing last).
    pub fn total_cmp(&self, other: &Obj) -> Ordering {
        match (self, other) {
            (Obj::Str(a), Obj::Str(b)) => a.cmp(b),
            (Obj::Datetime(a), Obj::Datetime(b)) => a.cmp(b),
            (a, b) if a.rank() == 0 && b.rank() == 0 => {
                let (x, y) = (a.as_f64().unwrap_or(0.0), b.as_f64().unwrap_or(0.0));
                x.total_cmp(&y)
            }
            (a, b) => a.rank().cmp(&b.rank()),
        }
    }
}

/// Structural equality: missing equals missing, `1 != 1.0` (types must match).
impl PartialEq for Obj {
    fn eq(&self, other: &Obj) -> bool {
        match (self, other) {
            (Obj::Nan, Obj::Nan) => true,
            (Obj::Bool(a), Obj::Bool(b)) => a == b,
            (Obj::Int(a), Obj::Int(b)) | (Obj::Datetime(a), Obj::Datetime(b)) => a == b,
            (Obj::Float(a), Obj::Float(b)) => a == b,
            (Obj::Str(a), Obj::Str(b)) => a == b,
            _ => false,
        }
    }
}

impl fmt::Display for Obj {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_py_str())
    }
}

/// Formats nanoseconds since the epoch like pandas (`2005-01-01 00:00:00`).
pub fn format_datetime(ns: i64) -> String {
    if ns == NAT {
        return "NaT".to_owned();
    }
    let secs = ns.div_euclid(1_000_000_000);
    let nanos = u32::try_from(ns.rem_euclid(1_000_000_000)).unwrap_or(0);
    match chrono::DateTime::from_timestamp(secs, nanos) {
        Some(dt) => {
            let dt = dt.naive_utc();
            if nanos == 0 {
                dt.format("%Y-%m-%d %H:%M:%S").to_string()
            } else {
                dt.format("%Y-%m-%d %H:%M:%S%.f").to_string()
            }
        }
        None => format!("{ns}ns"),
    }
}

/// `NaT` in nanoseconds (numpy's `datetime64` minimum).
pub const NAT: i64 = i64::MIN;

/// A numpy dtype, as far as Calliope inputs use them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DType {
    /// `bool`.
    Bool,
    /// `int64`.
    Int,
    /// `float64`.
    Float,
    /// `datetime64[ns]`.
    Datetime,
    /// `object` (strings, mixed values, NaN).
    Object,
}

impl DType {
    /// numpy's name.
    pub fn name(self) -> &'static str {
        match self {
            DType::Bool => "bool",
            DType::Int => "int64",
            DType::Float => "float64",
            DType::Datetime => "datetime64[ns]",
            DType::Object => "object",
        }
    }

    /// The dtype after inserting missing values: xarray alignment (`reindex`, `where`,
    /// `concat`) turns `bool` and `int` into `float64`.
    pub fn with_missing(self) -> DType {
        match self {
            DType::Bool | DType::Int => DType::Float,
            d => d,
        }
    }

    /// `xarray.core.dtypes.maybe_promote`, used by `Dataset.from_dataframe` for incomplete
    /// MultiIndexes: `bool` becomes `object`, `int` becomes `float64`.
    pub fn maybe_promote(self) -> DType {
        match self {
            DType::Bool => DType::Object,
            DType::Int => DType::Float,
            d => d,
        }
    }

    /// `numpy.result_type` with xarray's promote-to-object rules.
    pub fn result_type(self, other: DType) -> DType {
        use DType::*;
        match (self, other) {
            (a, b) if a == b => a,
            (Object, _) | (_, Object) => Object,
            (Datetime, _) | (_, Datetime) => Object,
            (Float, _) | (_, Float) => Float,
            (Int, _) | (_, Int) => Int,
            _ => Bool,
        }
    }
}

impl fmt::Display for DType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// `float64` values stored sparsely: the flat positions (ascending) and values of the
/// non-missing cells of an array of `len` cells. Equivalent to a dense `float64` vector with
/// NaN everywhere else.
#[derive(Debug, Clone, Default)]
pub struct SparseF64 {
    /// Number of cells.
    pub len: usize,
    /// Flat positions of the stored values, strictly ascending.
    pub idx: Vec<usize>,
    /// Values (never NaN).
    pub val: Vec<f64>,
}

impl SparseF64 {
    /// The non-NaN values of `v`.
    pub fn from_dense(v: &[f64]) -> Self {
        let mut s = SparseF64 {
            len: v.len(),
            ..SparseF64::default()
        };
        for (i, x) in v.iter().enumerate() {
            if !x.is_nan() {
                s.idx.push(i);
                s.val.push(*x);
            }
        }
        s
    }

    /// From `(position, value)` pairs in any order (NaN values are dropped; for repeated
    /// positions the last value wins).
    pub fn from_pairs(len: usize, mut pairs: Vec<(usize, f64)>) -> Self {
        if !pairs.windows(2).all(|w| w[0].0 < w[1].0) {
            pairs.sort_by_key(|p| p.0);
            let mut dedup: Vec<(usize, f64)> = Vec::with_capacity(pairs.len());
            for p in pairs {
                match dedup.last_mut() {
                    Some(last) if last.0 == p.0 => *last = p,
                    _ => dedup.push(p),
                }
            }
            pairs = dedup;
        }
        let (idx, val) = pairs.into_iter().filter(|p| !p.1.is_nan()).unzip();
        SparseF64 { len, idx, val }
    }

    /// Value at `i` (NaN if not stored).
    pub fn get(&self, i: usize) -> f64 {
        self.idx.binary_search(&i).map_or(f64::NAN, |k| self.val[k])
    }

    /// Number of stored values.
    pub fn nnz(&self) -> usize {
        self.idx.len()
    }

    /// Dense values (NaN where not stored).
    pub fn to_dense(&self) -> Vec<f64> {
        let mut v = vec![f64::NAN; self.len];
        for (&i, &x) in self.idx.iter().zip(&self.val) {
            v[i] = x;
        }
        v
    }

    /// Stored `(position, value)` pairs.
    pub fn iter(&self) -> impl Iterator<Item = (usize, f64)> + '_ {
        self.idx.iter().copied().zip(self.val.iter().copied())
    }
}

/// Sparse storage pays off when at most this share of the cells holds a value (16 bytes per
/// stored value against 8 per cell).
const SPARSE_MAX_DENSITY: f64 = 0.25;

/// A flat vector of values of one dtype.
#[derive(Debug, Clone)]
pub enum Data {
    /// `bool` (no missing values).
    Bool(Vec<bool>),
    /// `int64` (no missing values).
    Int(Vec<i64>),
    /// `float64` (NaN is missing).
    Float(Vec<f64>),
    /// `datetime64[ns]` ([`NAT`] is missing).
    Datetime(Vec<i64>),
    /// `object`.
    Object(Vec<Obj>),
    /// `float64`, only the non-missing values stored ([`SparseF64`]).
    Sparse(SparseF64),
}

/// Values compare as numpy arrays would with `==` on every cell (so NaN is never equal);
/// sparse and dense `float64` data compare by value.
impl PartialEq for Data {
    fn eq(&self, other: &Data) -> bool {
        match (self, other) {
            (Data::Bool(a), Data::Bool(b)) => a == b,
            (Data::Int(a), Data::Int(b)) | (Data::Datetime(a), Data::Datetime(b)) => a == b,
            (Data::Float(a), Data::Float(b)) => a == b,
            (Data::Object(a), Data::Object(b)) => a == b,
            (Data::Sparse(a), Data::Sparse(b)) => a.to_dense() == b.to_dense(),
            (Data::Sparse(a), Data::Float(b)) | (Data::Float(b), Data::Sparse(a)) => {
                a.to_dense() == *b
            }
            _ => false,
        }
    }
}

impl Data {
    /// Number of values.
    pub fn len(&self) -> usize {
        match self {
            Data::Bool(v) => v.len(),
            Data::Int(v) => v.len(),
            Data::Float(v) => v.len(),
            Data::Datetime(v) => v.len(),
            Data::Object(v) => v.len(),
            Data::Sparse(s) => s.len,
        }
    }

    /// True if empty.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The dtype.
    pub fn dtype(&self) -> DType {
        match self {
            Data::Bool(_) => DType::Bool,
            Data::Int(_) => DType::Int,
            Data::Float(_) | Data::Sparse(_) => DType::Float,
            Data::Datetime(_) => DType::Datetime,
            Data::Object(_) => DType::Object,
        }
    }

    /// The value at `i`.
    pub fn get(&self, i: usize) -> Obj {
        match self {
            Data::Bool(v) => Obj::Bool(v[i]),
            Data::Int(v) => Obj::Int(v[i]),
            Data::Float(v) => Obj::float(v[i]),
            Data::Datetime(v) => {
                if v[i] == NAT {
                    Obj::Nan
                } else {
                    Obj::Datetime(v[i])
                }
            }
            Data::Object(v) => v[i].clone(),
            Data::Sparse(s) => Obj::float(s.get(i)),
        }
    }

    /// True if the value at `i` is missing.
    pub fn is_null(&self, i: usize) -> bool {
        match self {
            Data::Bool(_) | Data::Int(_) => false,
            Data::Float(v) => v[i].is_nan(),
            Data::Datetime(v) => v[i] == NAT,
            Data::Object(v) => v[i].is_null(),
            Data::Sparse(s) => s.idx.binary_search(&i).is_err(),
        }
    }

    /// True if every value is missing.
    pub fn all_null(&self) -> bool {
        match self {
            Data::Bool(v) => v.is_empty(),
            Data::Int(v) => v.is_empty(),
            Data::Float(v) => v.iter().all(|x| x.is_nan()),
            Data::Datetime(v) => v.iter().all(|x| *x == NAT),
            Data::Object(v) => v.iter().all(Obj::is_null),
            Data::Sparse(s) => s.idx.is_empty(),
        }
    }

    /// The data with sparse values made dense (other data unchanged).
    pub fn into_dense(self) -> Data {
        match self {
            Data::Sparse(s) => Data::Float(s.to_dense()),
            d => d,
        }
    }

    /// The data, dense (borrowed unless sparse).
    pub fn dense(&self) -> std::borrow::Cow<'_, Data> {
        match self {
            Data::Sparse(s) => std::borrow::Cow::Owned(Data::Float(s.to_dense())),
            d => std::borrow::Cow::Borrowed(d),
        }
    }

    /// `float64` data stored sparsely if few cells hold a value; other data unchanged.
    pub fn into_sparse_if_sparse(self) -> Data {
        match self {
            Data::Float(v) => {
                let nnz = v.iter().filter(|x| !x.is_nan()).count();
                #[allow(clippy::cast_precision_loss)]
                if !v.is_empty() && (nnz as f64) <= SPARSE_MAX_DENSITY * v.len() as f64 {
                    Data::Sparse(SparseF64::from_dense(&v))
                } else {
                    Data::Float(v)
                }
            }
            d => d,
        }
    }

    /// All values.
    pub fn objs(&self) -> Vec<Obj> {
        (0..self.len()).map(|i| self.get(i)).collect()
    }

    /// `n` missing values of a dtype that can hold them.
    pub fn missing(dtype: DType, n: usize) -> Data {
        match dtype {
            DType::Bool | DType::Int | DType::Float => Data::Float(vec![f64::NAN; n]),
            DType::Datetime => Data::Datetime(vec![NAT; n]),
            DType::Object => Data::Object(vec![Obj::Nan; n]),
        }
    }

    /// Values at `idx` (`None` = missing). With any missing value, `bool`/`int` become
    /// `fill_dtype` (`float64` for xarray alignment, see [`DType::with_missing`]).
    pub fn gather(&self, idx: &[Option<usize>], fill_dtype: DType) -> Data {
        let any_missing = idx.iter().any(Option::is_none);
        self.gather_iter(idx.iter().copied(), idx.len(), any_missing, fill_dtype)
    }

    /// [`Data::gather`] over `n` positions from an iterator; `any_missing` must be true iff
    /// it yields a `None`.
    pub fn gather_iter(
        &self,
        idx: impl Iterator<Item = Option<usize>>,
        n: usize,
        any_missing: bool,
        fill_dtype: DType,
    ) -> Data {
        if let Data::Sparse(s) = self {
            let pairs: Vec<(usize, f64)> = idx
                .enumerate()
                .filter_map(|(k, i)| i.map(|i| (k, s.get(i))))
                .filter(|p| !p.1.is_nan())
                .collect();
            return Data::Sparse(SparseF64::from_pairs(n, pairs));
        }
        if !any_missing {
            let mut idx = idx.map(|i| i.unwrap_or(0));
            return match self {
                Data::Bool(v) => Data::Bool(idx.by_ref().map(|i| v[i]).collect()),
                Data::Int(v) => Data::Int(idx.by_ref().map(|i| v[i]).collect()),
                Data::Float(v) => Data::Float(idx.by_ref().map(|i| v[i]).collect()),
                Data::Datetime(v) => Data::Datetime(idx.by_ref().map(|i| v[i]).collect()),
                Data::Object(v) => Data::Object(idx.by_ref().map(|i| v[i].clone()).collect()),
                Data::Sparse(_) => unreachable!("handled above"),
            };
        }
        match (self, fill_dtype) {
            (Data::Float(v), _) => Data::Float(idx.map(|i| i.map_or(f64::NAN, |i| v[i])).collect()),
            (Data::Datetime(v), _) => {
                Data::Datetime(idx.map(|i| i.map_or(NAT, |i| v[i])).collect())
            }
            (Data::Object(v), _) => {
                Data::Object(idx.map(|i| i.map_or(Obj::Nan, |i| v[i].clone())).collect())
            }
            (d, DType::Object) => {
                Data::Object(idx.map(|i| i.map_or(Obj::Nan, |i| d.get(i))).collect())
            }
            (Data::Bool(v), _) => Data::Float(
                idx.map(|i| i.map_or(f64::NAN, |i| f64::from(u8::from(v[i]))))
                    .collect(),
            ),
            #[allow(clippy::cast_precision_loss)]
            (Data::Int(v), _) => {
                Data::Float(idx.map(|i| i.map_or(f64::NAN, |i| v[i] as f64)).collect())
            }
            (Data::Sparse(_), _) => unreachable!("handled above"),
        }
    }

    /// [`Data::cast`] by value: no copy when the dtype is already `dtype`.
    pub fn cast_into(self, dtype: DType) -> Result<Data, String> {
        if self.dtype() == dtype {
            return Ok(self);
        }
        self.cast(dtype)
    }

    /// Values converted to `dtype` (numpy `astype`, without the string case).
    pub fn cast(&self, dtype: DType) -> Result<Data, String> {
        if self.dtype() == dtype {
            return Ok(self.clone());
        }
        let n = self.len();
        Ok(match dtype {
            DType::Object => Data::Object(self.objs()),
            DType::Float => {
                let mut out = Vec::with_capacity(n);
                for o in self.objs() {
                    out.push(match o {
                        Obj::Str(s) => parse_py_float(&s)
                            .ok_or_else(|| format!("could not convert string to float: '{s}'"))?,
                        Obj::Datetime(_) => {
                            return Err("cannot cast datetime to float".to_owned());
                        }
                        o => o.as_f64().unwrap_or(f64::NAN),
                    });
                }
                Data::Float(out)
            }
            DType::Int => {
                let mut out = Vec::with_capacity(n);
                for o in self.objs() {
                    out.push(match o {
                        Obj::Bool(b) => i64::from(b),
                        Obj::Int(i) => i,
                        #[allow(clippy::cast_possible_truncation)]
                        Obj::Float(f) if f.is_finite() => f as i64,
                        Obj::Str(s) => s
                            .trim()
                            .parse()
                            .map_err(|_| format!("invalid literal for int(): '{s}'"))?,
                        Obj::Nan | Obj::Float(_) => {
                            return Err("cannot convert float NaN to integer".to_owned());
                        }
                        Obj::Datetime(d) => d,
                    });
                }
                Data::Int(out)
            }
            DType::Bool => Data::Bool(self.objs().iter().map(Obj::truthy).collect()),
            DType::Datetime => {
                let mut out = Vec::with_capacity(n);
                for o in self.objs() {
                    out.push(match o {
                        Obj::Nan => NAT,
                        Obj::Datetime(d) | Obj::Int(d) => d,
                        other => return Err(format!("cannot cast `{other}` to datetime")),
                    });
                }
                Data::Datetime(out)
            }
        })
    }

    /// pandas' inference for a list of Python values (`pd.Series(list)`, `infer_objects`).
    pub fn infer(objs: Vec<Obj>) -> Data {
        let mut has = [false; 6]; // nan, bool, int, float, str, datetime
        for o in &objs {
            let k = match o {
                Obj::Nan => 0,
                Obj::Bool(_) => 1,
                Obj::Int(_) => 2,
                Obj::Float(_) => 3,
                Obj::Str(_) => 4,
                Obj::Datetime(_) => 5,
            };
            has[k] = true;
        }
        let [nan, b, i, f, s, d] = has;
        if s || (b && (i || f || nan)) || (d && (b || i || f || s)) {
            return Data::Object(objs);
        }
        if d {
            return Data::Datetime(
                objs.iter()
                    .map(|o| if let Obj::Datetime(x) = o { *x } else { NAT })
                    .collect(),
            );
        }
        if b {
            return Data::Bool(objs.iter().map(Obj::truthy).collect());
        }
        if i && !f && !nan {
            return Data::Int(
                objs.iter()
                    .map(|o| if let Obj::Int(x) = o { *x } else { 0 })
                    .collect(),
            );
        }
        if objs.is_empty() {
            return Data::Object(objs);
        }
        Data::Float(
            objs.iter()
                .map(|o| o.as_f64().unwrap_or(f64::NAN))
                .collect(),
        )
    }

    /// `pd.Series(list)` / numpy array creation of all-missing data: `float64`.
    pub fn from_objs_infer_series(objs: Vec<Obj>) -> Data {
        if objs.iter().all(Obj::is_null) {
            return Data::Float(vec![f64::NAN; objs.len()]);
        }
        Data::infer(objs)
    }

    /// Builds data of a given dtype from values (values must fit).
    pub fn from_objs(dtype: DType, objs: Vec<Obj>) -> Data {
        let d = Data::Object(objs);
        match d.cast(dtype) {
            Ok(c) => c,
            Err(_) => d,
        }
    }

    /// Mask of non-missing values.
    pub fn notnull(&self) -> Vec<bool> {
        match self {
            Data::Bool(v) => vec![true; v.len()],
            Data::Int(v) => vec![true; v.len()],
            Data::Float(v) => v.iter().map(|x| !x.is_nan()).collect(),
            Data::Sparse(s) => {
                let mut m = vec![false; s.len];
                for &i in &s.idx {
                    m[i] = true;
                }
                m
            }
            d => (0..d.len()).map(|i| !d.is_null(i)).collect(),
        }
    }

    /// The float values (bools and ints converted), if numeric.
    pub fn as_f64_vec(&self) -> Option<Vec<f64>> {
        match self {
            Data::Float(v) => Some(v.clone()),
            #[allow(clippy::cast_precision_loss)]
            Data::Int(v) => Some(v.iter().map(|x| *x as f64).collect()),
            Data::Bool(v) => Some(v.iter().map(|x| f64::from(u8::from(*x))).collect()),
            Data::Object(v) => v.iter().map(Obj::as_f64).collect(),
            Data::Datetime(_) => None,
            Data::Sparse(s) => Some(s.to_dense()),
        }
    }
}

/// Python `float(s)`: accepts `inf`, `-inf`, `nan`, `infinity` in any case and surrounding
/// whitespace.
pub fn parse_py_float(s: &str) -> Option<f64> {
    let t = s.trim();
    let lower = t.to_ascii_lowercase();
    match lower.trim_start_matches(['+', '-']) {
        "inf" | "infinity" => {
            return Some(if lower.starts_with('-') {
                f64::NEG_INFINITY
            } else {
                f64::INFINITY
            });
        }
        "nan" => return Some(f64::NAN),
        _ => {}
    }
    let cleaned: String = t.chars().filter(|c| *c != '_').collect();
    if cleaned.is_empty() || cleaned.contains(['x', 'X']) {
        return None;
    }
    cleaned.parse::<f64>().ok()
}
