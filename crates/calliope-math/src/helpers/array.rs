//! Helper function semantics on [`LArray`]s (`ParsingHelperFunction.as_array`).
//!
//! Each function reproduces the xarray operations of its Python counterpart, including the
//! errors Calliope raises. Values are passed as [`Value`], so that a dtype mismatch (e.g. a
//! numeric array given to `any`) is reported as Calliope reports it. The evaluation step
//! adds affine expressions (decision variables) as a further kind of value; `sum`, `roll`,
//! `where`, `select_from_lookup_arrays`, `group_sum` and `sum_next_n` are the helpers that
//! must then also accept those, with the same key/coordinate logic as here.

use std::collections::BTreeMap;

use calliope_array::{ArrayError, Coord, Element, Join, LArray, Label};

/// An evaluated helper argument or result.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// A numeric array (`float`).
    Float(LArray<f64>),
    /// A boolean array.
    Bool(LArray<bool>),
    /// An array of labels: strings, datetimes or dimension items (`object` dtype).
    Label(LArray<Option<Label>>),
}

impl Value {
    /// The numpy dtype name Calliope would print.
    pub fn dtype(&self) -> &'static str {
        match self {
            Value::Float(_) => "float64",
            Value::Bool(_) => "bool",
            Value::Label(_) => "object",
        }
    }

    /// The dimension names.
    pub fn dim_names(&self) -> Vec<String> {
        let names = match self {
            Value::Float(a) => a.dim_names(),
            Value::Bool(a) => a.dim_names(),
            Value::Label(a) => a.dim_names(),
        };
        names.into_iter().map(str::to_owned).collect()
    }
}

/// Errors from helper functions, with Calliope's messages.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum HelperError {
    /// Calliope raises `BackendError`.
    #[error("{0}")]
    Backend(String),
    /// Calliope raises `IndexError`.
    #[error("{0}")]
    Index(String),
    /// Calliope raises `ValueError`.
    #[error("{0}")]
    Value(String),
    /// An array operation failed.
    #[error(transparent)]
    Array(#[from] ArrayError),
}

type Result<T> = std::result::Result<T, HelperError>;

fn present<'a>(dims: &[String], over: &'a [&'a str]) -> Vec<&'a str> {
    over.iter()
        .copied()
        .filter(|o| dims.iter().any(|d| d == o))
        .collect()
}

/// `any(array, over=...)`: OR over the dimensions in `over` that the array has.
pub fn any(array: &Value, name: &str, over: &[&str]) -> Result<LArray<bool>> {
    let Value::Bool(a) = array else {
        return Err(HelperError::Backend(format!(
            "Input to `any` must be a boolean array. Received {name} of dtype {}",
            array.dtype()
        )));
    };
    Ok(a.any(&present(&array.dim_names(), over))?)
}

/// `sum(array, over=...)`: sum over the dimensions in `over` that the array has, skipping
/// NaN, NaN where a group has no value (`min_count=1`). Booleans count as 0/1 everywhere.
pub fn sum(array: &Value, over: &[&str]) -> Result<LArray<f64>> {
    let over = present(&array.dim_names(), over);
    let numeric = match array {
        Value::Float(a) => a.clone(),
        Value::Bool(a) => bool_to_float(a),
        Value::Label(_) => {
            return Err(HelperError::Backend(
                "Cannot sum an array of labels".to_owned(),
            ));
        }
    };
    Ok(numeric.sum(&over, 1)?)
}

/// Booleans as 0/1 over the full product (xarray has no missing booleans).
pub fn bool_to_float(a: &LArray<bool>) -> LArray<f64> {
    let dense: Vec<f64> = a
        .to_dense()
        .into_iter()
        .map(|b| if b { 1.0 } else { 0.0 })
        .collect();
    LArray::from_dense(a.dims().to_vec(), dense).expect("same shape")
}

/// `reduce_carrier_dim(array, flow_direction)`: the array where `carrier_<direction>` is
/// true, summed over `carriers`.
pub fn reduce_carrier_dim(array: &LArray<f64>, carrier: &LArray<bool>) -> Result<LArray<f64>> {
    let masked = array.where_(carrier);
    let names = masked.dim_names();
    let over: Vec<&str> = names.into_iter().filter(|d| *d == "carriers").collect();
    Ok(masked.sum(&over, 1)?)
}

/// `select_from_lookup_arrays(array, dim=lookup, ...)`: vectorised indexing. For each cell
/// of the lookups, the array value at the coordinates the lookups name; NaN where a lookup is
/// missing or names an item not on the dimension.
pub fn select_from_lookup_arrays<T: Element>(
    array: &LArray<T>,
    array_name: &str,
    lookups: &[(String, String, &LArray<Option<Label>>)],
) -> Result<LArray<T>> {
    let keys: Vec<&str> = lookups.iter().map(|(d, _, _)| d.as_str()).collect();
    let mut key_set: Vec<&str> = keys.clone();
    key_set.sort_unstable();
    let set_repr = format!(
        "{{{}}}",
        key_set
            .iter()
            .map(|k| format!("'{k}'"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let missing: Vec<&str> = key_set
        .iter()
        .copied()
        .filter(|k| array.coord(k).is_none())
        .collect();
    if !missing.is_empty() {
        let m = format!(
            "{{{}}}",
            missing
                .iter()
                .map(|k| format!("'{k}'"))
                .collect::<Vec<_>>()
                .join(", ")
        );
        return Err(HelperError::Backend(format!(
            "Cannot select items from `{array_name}` on the dimensions {set_repr} since the array is not indexed over the dimensions {m}"
        )));
    }
    if lookups
        .iter()
        .any(|(_, _, l)| key_set.iter().any(|k| l.coord(k).is_none()))
    {
        return Err(HelperError::Backend(format!(
            "All lookup arrays used to select items from `{array_name}` must be indexed over the dimensions {set_repr}"
        )));
    }
    for (dim, lname, l) in lookups {
        let coord = array.coord(dim).expect("checked");
        if !l
            .cells()
            .any(|(_, v)| v.as_ref().is_some_and(|x| coord.code(x).is_some()))
        {
            let received: Vec<String> = l
                .cells()
                .filter_map(|(_, v)| v.as_ref().map(ToString::to_string))
                .collect();
            return Err(HelperError::Index(format!(
                "Trying to select items on the dimension {dim} from the {lname} lookup array, but no matches found. Received: {received:?}"
            )));
        }
    }
    // Result dims: the array's, then any lookup dims it lacks.
    let mut dims: Vec<Coord> = array.dims().to_vec();
    for (_, _, l) in lookups {
        for d in l.dims() {
            if !dims.iter().any(|x| x.name() == d.name()) {
                dims.push(d.clone());
            }
        }
    }
    let pos = |name: &str| dims.iter().position(|d| d.name() == name).expect("dim");
    let arr_axes: Vec<usize> = array.dims().iter().map(|d| pos(d.name())).collect();
    let lookup_axes: Vec<Vec<usize>> = lookups
        .iter()
        .map(|(_, _, l)| l.dims().iter().map(|d| pos(d.name())).collect())
        .collect();
    let key_axes_in_array: Vec<usize> = keys
        .iter()
        .map(|k| array.axis(k).expect("checked"))
        .collect();
    let mut cells = Vec::new();
    let shape: Vec<usize> = dims.iter().map(Coord::len).collect();
    let total: usize = shape.iter().product();
    let mut idx = vec![0u32; dims.len()];
    for _ in 0..total {
        let mut acodes: Vec<u32> = arr_axes.iter().map(|&ax| idx[ax]).collect();
        let mut ok = true;
        for (i, (dim, _, l)) in lookups.iter().enumerate() {
            let lcodes: Vec<u32> = lookup_axes[i].iter().map(|&ax| idx[ax]).collect();
            let target = l
                .get(&lcodes)
                .and_then(|label| array.coord(dim).expect("checked").code(&label));
            match target {
                Some(c) => acodes[key_axes_in_array[i]] = c,
                None => {
                    ok = false;
                    break;
                }
            }
        }
        if ok {
            let v = array.get(&acodes);
            if !v.is_missing() {
                cells.push((idx.clone(), v));
            }
        }
        for i in (0..idx.len()).rev() {
            idx[i] += 1;
            if (idx[i] as usize) < shape[i] {
                break;
            }
            idx[i] = 0;
        }
    }
    Ok(LArray::from_cells(dims, cells)?)
}

/// `get_val_at_index(dim=index)`: the dimension item at an integer position (negative
/// counts from the end).
pub fn get_val_at_index(coord: &Coord, index: i64) -> Result<Label> {
    let n = i64::try_from(coord.len()).unwrap_or(i64::MAX);
    let i = if index < 0 { n + index } else { index };
    if !(0..n).contains(&i) {
        return Err(HelperError::Index(format!(
            "index {index} is out of bounds for axis 0 with size {n}"
        )));
    }
    Ok(coord.label(u32::try_from(i).expect("in range")).clone())
}

/// `roll(array, dim=shift, ...)`: shifts data along each dimension (numpy `roll`, labels
/// stay). Shifts are truncated to integers, as Python's `int()` does.
pub fn roll<T: Element>(array: &LArray<T>, shifts: &[(String, f64)]) -> Result<LArray<T>> {
    let mut out = array.clone();
    for (dim, shift) in shifts {
        #[allow(clippy::cast_possible_truncation)]
        let s = shift.trunc() as i64;
        out = out.roll(dim, s)?;
    }
    Ok(out)
}

/// The condition of `where(array, condition)`: `condition.fillna(False).astype(bool)`.
pub fn as_condition(condition: &Value) -> LArray<bool> {
    match condition {
        Value::Bool(c) => c.clone(),
        Value::Float(c) => c.map(|v| !v.is_nan() && *v != 0.0).compact(),
        Value::Label(c) => c.map(Option::is_some).compact(),
    }
}

/// `where(array, condition)`: the array broadcast over the condition's dims, NaN where the
/// condition is false.
pub fn where_(array: &LArray<f64>, condition: &Value) -> LArray<f64> {
    array.where_(&as_condition(condition))
}

/// `map_dim(dim, mapper)`: true where the dimension item equals the mapper's value.
pub fn map_dim(dim: &Coord, mapper: &LArray<Option<Label>>) -> LArray<bool> {
    let items = LArray::from_coord(dim);
    items
        .zip_with(mapper, Join::Inner, |a, b| a.is_some() && a == b)
        .compact()
}

/// `group_sum(array, groupby, group_dim)`: sums over each group of `groupby` labels; the
/// new `group_dim` dimension comes first with sorted labels.
pub fn group_sum(
    array: &LArray<f64>,
    groupby: &LArray<Option<Label>>,
    group_dim: &str,
) -> Result<LArray<f64>> {
    Ok(array.group_sum(groupby, group_dim)?)
}

/// The integer or string dtype of a `group_datetime` grouping dimension (the math
/// `dimensions.<group>.dtype`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupDtype {
    /// `string`
    String,
    /// `integer`
    Integer,
}

/// `group_datetime(array, over, group)`: `group_sum` by a datetime attribute of the `over`
/// dimension (`date`, `time`, `hour`, `month`, ...).
pub fn group_datetime(
    array: &LArray<f64>,
    over: &str,
    group: &str,
    dtype: GroupDtype,
) -> Result<LArray<f64>> {
    let coord = array
        .coord(over)
        .ok_or_else(|| ArrayError::MissingDim(over.to_owned()))?;
    let groupby = datetime_groupby(coord, group, dtype)?;
    group_sum(array, &groupby, group)
}

/// The `group_datetime` grouping array: the datetime attribute `group` of each label of
/// `coord`.
pub fn datetime_groupby(
    coord: &Coord,
    group: &str,
    dtype: GroupDtype,
) -> Result<LArray<Option<Label>>> {
    let over = coord.name();
    let labels: Vec<Option<Label>> = coord
        .labels()
        .iter()
        .map(|l| match l {
            Label::Datetime(ns) => datetime_attr(*ns, group, dtype),
            _ => Err(HelperError::Value(format!(
                "`group_datetime` needs a datetime dimension, `{over}` is not"
            ))),
        })
        .collect::<Result<_>>()?;
    Ok(LArray::from_cells(
        vec![coord.clone()],
        labels
            .into_iter()
            .enumerate()
            .map(|(i, l)| (vec![u32::try_from(i).expect("fits")], l)),
    )?)
}

fn datetime_attr(ns: i64, attr: &str, dtype: GroupDtype) -> Result<Option<Label>> {
    const NS_PER_DAY: i64 = 86_400_000_000_000;
    let days = ns.div_euclid(NS_PER_DAY);
    let in_day = ns.rem_euclid(NS_PER_DAY) / 1_000_000_000;
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (in_day / 3600, in_day / 60 % 60, in_day % 60);
    let dow = (days + 3).rem_euclid(7); // 1970-01-01 was a Thursday (Monday = 0)
    let doy = days - days_from_civil(y, 1, 1) + 1;
    let int = |v: i64| -> Option<Label> {
        Some(match dtype {
            GroupDtype::Integer => Label::Int(v),
            GroupDtype::String => Label::str(&v.to_string()),
        })
    };
    Ok(match attr {
        "date" => Some(Label::str(&format!("{y:04}-{m:02}-{d:02}"))),
        "time" => Some(Label::str(&format!("{hh:02}:{mm:02}:{ss:02}"))),
        "year" => int(y),
        "month" => int(m),
        "day" => int(d),
        "hour" => int(hh),
        "minute" => int(mm),
        "second" => int(ss),
        "dayofweek" | "day_of_week" | "weekday" => int(dow),
        "dayofyear" | "day_of_year" => int(doy),
        "quarter" => int((m - 1) / 3 + 1),
        other => {
            return Err(HelperError::Value(format!(
                "Unsupported datetime grouping `{other}`"
            )));
        }
    })
}

/// Days since 1970-01-01 to (year, month, day) (Howard Hinnant's algorithm).
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `sum_next_n(array, over, N)`: at each position, the sum of the `N` values from that
/// position on (fewer at the end; no wrap-around), NaN if all are NaN.
pub fn sum_next_n(array: &LArray<f64>, over: &str, n: f64) -> Result<LArray<f64>> {
    let ax = array.axis(over)?;
    let len = array.dims()[ax].len();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let n = if n <= 0.0 { 0 } else { n.trunc() as usize };
    let mut rows: BTreeMap<Vec<u32>, Vec<f64>> = BTreeMap::new();
    for (codes, v) in array.cells() {
        let mut key = codes.clone();
        key.remove(ax);
        rows.entry(key).or_insert_with(|| vec![f64::NAN; len])[codes[ax] as usize] = *v;
    }
    let mut cells = Vec::new();
    for (key, row) in rows {
        for i in 0..len {
            let window = &row[i..(i + n).min(len)];
            let vals: Vec<f64> = window.iter().copied().filter(|v| !v.is_nan()).collect();
            if !vals.is_empty() {
                let mut codes = key.clone();
                codes.insert(ax, u32::try_from(i).expect("fits"));
                cells.push((codes, vals.iter().sum()));
            }
        }
    }
    Ok(LArray::from_cells(array.dims().to_vec(), cells)?)
}

/// `(year, month, day, hour, minute)` of a datetime in nanoseconds since the epoch.
pub fn datetime_parts(ns: i64) -> (i64, i64, i64, i64, i64) {
    const NS_PER_DAY: i64 = 86_400_000_000_000;
    let days = ns.div_euclid(NS_PER_DAY);
    let in_day = ns.rem_euclid(NS_PER_DAY) / 1_000_000_000;
    let (y, m, d) = civil_from_days(days);
    (y, m, d, in_day / 3600, in_day / 60 % 60)
}
