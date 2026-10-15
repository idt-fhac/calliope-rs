//! Time handling (`preprocess/time.py`): datetime parsing as pandas `to_datetime` does,
//! partial-date slices, resampling with pandas bin edges, inferred time parameters and
//! time clustering.

use std::collections::{BTreeMap, HashMap};
use std::sync::LazyLock;

use calliope_array::{Coord, Label};
use calliope_schema::{CalliopeBuildMath, ComponentRef, ResampleMethod};
use chrono::{Datelike, NaiveDate, NaiveDateTime, NaiveTime};
use regex::Regex;

use crate::error::ModelError;
use crate::xr::{DType, Data, DataArray, Dataset, NAT, Obj, format_datetime};

const NS_PER_SEC: i64 = 1_000_000_000;
const NS_PER_HOUR: i64 = 3_600 * NS_PER_SEC;
const NS_PER_DAY: i64 = 24 * NS_PER_HOUR;

/// Nanoseconds since the epoch of a naive datetime.
pub fn to_ns(dt: NaiveDateTime) -> Option<i64> {
    dt.and_utc().timestamp_nanos_opt()
}

/// The naive datetime of nanoseconds since the epoch.
pub fn from_ns(ns: i64) -> NaiveDateTime {
    chrono::DateTime::from_timestamp(
        ns.div_euclid(NS_PER_SEC),
        u32::try_from(ns.rem_euclid(NS_PER_SEC)).unwrap_or(0),
    )
    .map(|d| d.naive_utc())
    .unwrap_or_default()
}

/// Midnight of the day of `ns`.
pub fn floor_day(ns: i64) -> i64 {
    ns - ns.rem_euclid(NS_PER_DAY)
}

static ISO_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^\s*(\d{4})(?:-(\d{1,2})(?:-(\d{1,2})(?:[T ](\d{1,2})(?::(\d{1,2})(?::(\d{1,2})(?:[.,](\d{1,9}))?)?)?)?)?)?\s*$",
    )
    .expect("valid regex")
});

/// Resolution of a partially given ISO 8601 date.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Resolution {
    Year,
    Month,
    Day,
    Hour,
    Minute,
    Second,
    Nano,
}

fn parse_iso_parts(s: &str) -> Option<(NaiveDateTime, Resolution)> {
    let c = ISO_RE.captures(s)?;
    let num =
        |i: usize, d: u32| -> Option<u32> { c.get(i).map_or(Some(d), |m| m.as_str().parse().ok()) };
    let year: i32 = c.get(1)?.as_str().parse().ok()?;
    let res = (2..=7).take_while(|i| c.get(*i).is_some()).count();
    let resolution = match res {
        0 => Resolution::Year,
        1 => Resolution::Month,
        2 => Resolution::Day,
        3 => Resolution::Hour,
        4 => Resolution::Minute,
        5 => Resolution::Second,
        _ => Resolution::Nano,
    };
    let nanos = match c.get(7) {
        Some(m) => {
            let digits = m.as_str();
            let mut v: u32 = digits.parse().ok()?;
            for _ in digits.len()..9 {
                v *= 10;
            }
            v
        }
        None => 0,
    };
    let date = NaiveDate::from_ymd_opt(year, num(2, 1)?, num(3, 1)?)?;
    let time = NaiveTime::from_hms_nano_opt(num(4, 0)?, num(5, 0)?, num(6, 0)?, nanos)?;
    Some((NaiveDateTime::new(date, time), resolution))
}

/// Parses an ISO 8601 date or datetime, as pandas `to_datetime(format="ISO8601")` does.
pub fn parse_iso8601(s: &str) -> Option<i64> {
    parse_iso_parts(s).and_then(|(dt, _)| to_ns(dt))
}

/// The inclusive nanosecond range a partial ISO date string covers (pandas partial-string
/// indexing): `2005-01` covers all of January 2005.
pub fn partial_bounds(s: &str) -> Option<(i64, i64)> {
    let (start, res) = parse_iso_parts(s)?;
    let end = match res {
        Resolution::Year => {
            NaiveDate::from_ymd_opt(start.year() + 1, 1, 1)?.and_hms_opt(0, 0, 0)?
        }
        Resolution::Month => {
            let (y, m) = if start.month() == 12 {
                (start.year() + 1, 1)
            } else {
                (start.year(), start.month() + 1)
            };
            NaiveDate::from_ymd_opt(y, m, 1)?.and_hms_opt(0, 0, 0)?
        }
        Resolution::Day => start + chrono::Duration::days(1),
        Resolution::Hour => start + chrono::Duration::hours(1),
        Resolution::Minute => start + chrono::Duration::minutes(1),
        Resolution::Second => start + chrono::Duration::seconds(1),
        Resolution::Nano => start + chrono::Duration::nanoseconds(1),
    };
    Some((to_ns(start)?, to_ns(end)? - 1))
}

/// A strftime format compiled to a regex (pandas `to_datetime(format=..., exact=False)`
/// searches for the format anywhere in the string).
struct StrfFormat {
    re: Regex,
    fields: Vec<char>,
}

fn compile_strftime(fmt: &str, exact: bool) -> Result<StrfFormat, String> {
    let mut pat = String::new();
    let mut fields = Vec::new();
    let mut chars = fmt.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            if ch.is_whitespace() {
                pat.push_str(r"\s+");
            } else {
                pat.push_str(&regex::escape(&ch.to_string()));
            }
            continue;
        }
        let Some(d) = chars.next() else {
            return Err(format!("stray % in format '{fmt}'"));
        };
        let p = match d {
            'Y' => r"(\d{4})",
            'y' => r"(\d{2})",
            'm' | 'd' | 'H' | 'I' | 'M' | 'S' => r"(\d{1,2})",
            'f' => r"(\d{1,9})",
            'j' => r"(\d{1,3})",
            'b' | 'B' => r"([A-Za-z]+)",
            'p' => r"([AaPp][Mm])",
            '%' => {
                pat.push('%');
                continue;
            }
            other => return Err(format!("'{other}' is a bad directive in format '%{other}'")),
        };
        pat.push_str(p);
        fields.push(d);
    }
    let pat = if exact { format!("^{pat}$") } else { pat };
    Ok(StrfFormat {
        re: Regex::new(&pat).map_err(|e| e.to_string())?,
        fields,
    })
}

const MONTHS: [&str; 12] = [
    "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
];

fn parse_strftime(f: &StrfFormat, s: &str) -> Option<NaiveDateTime> {
    let c = f.re.captures(s)?;
    let (mut y, mut mo, mut d, mut h, mut mi, mut sec, mut ns) =
        (1900i32, 1u32, 1u32, 0u32, 0u32, 0u32, 0u32);
    let mut pm: Option<bool> = None;
    let mut doy: Option<u32> = None;
    for (i, field) in f.fields.iter().enumerate() {
        let txt = c.get(i + 1)?.as_str();
        match field {
            'Y' => y = txt.parse().ok()?,
            'y' => {
                let v: i32 = txt.parse().ok()?;
                y = if v < 69 { 2000 + v } else { 1900 + v };
            }
            'm' => mo = txt.parse().ok()?,
            'd' => d = txt.parse().ok()?,
            'H' | 'I' => h = txt.parse().ok()?,
            'M' => mi = txt.parse().ok()?,
            'S' => sec = txt.parse().ok()?,
            'f' => {
                let mut v: u32 = txt.parse().ok()?;
                for _ in txt.len()..9 {
                    v *= 10;
                }
                ns = v;
            }
            'j' => doy = txt.parse().ok(),
            'b' | 'B' => {
                let low = txt.to_ascii_lowercase();
                mo = u32::try_from(MONTHS.iter().position(|m| low.starts_with(m))? + 1).ok()?;
            }
            'p' => pm = Some(txt.eq_ignore_ascii_case("pm")),
            _ => {}
        }
    }
    if let Some(pm) = pm {
        h = (h % 12) + if pm { 12 } else { 0 };
    }
    let date = match doy {
        Some(j) => NaiveDate::from_yo_opt(y, j)?,
        None => NaiveDate::from_ymd_opt(y, mo, d)?,
    };
    Some(NaiveDateTime::new(
        date,
        NaiveTime::from_hms_nano_opt(h, mi, sec, ns)?,
    ))
}

/// `_datetime_index`: parses values to datetimes with `format` (`ISO8601`, or a strftime
/// format matched anywhere in the string, as pandas does with `exact=False`).
pub fn datetime_index(values: &[Obj], format: &str) -> Result<Vec<i64>, ModelError> {
    let wrap = |e: String| {
        ModelError::new(format!(
            "Error in parsing dates in timeseries data from using datetime format `{format}`. \
             Full error: {e}"
        ))
    };
    let compiled = if format == "ISO8601" {
        None
    } else {
        Some(compile_strftime(format, false).map_err(wrap)?)
    };
    let mut out = Vec::with_capacity(values.len());
    for (pos, v) in values.iter().enumerate() {
        let ns = match v {
            Obj::Nan => NAT,
            Obj::Datetime(d) => *d,
            Obj::Str(s) => {
                let parsed = match &compiled {
                    None => parse_iso8601(s),
                    Some(f) => parse_strftime(f, s).and_then(to_ns),
                };
                match parsed {
                    Some(ns) => ns,
                    None => {
                        return Err(wrap(match &compiled {
                            None => format!(
                                "Time data {s} is not ISO8601 format, at position {pos}. \
                                 You might want to try:\n    - passing `format` if your strings have a consistent format;\n    - passing `format='ISO8601'` if your strings are all ISO8601 but not necessarily in exactly the same format;\n    - passing `format='mixed'`, and the format will be inferred for each element individually. You might want to use `dayfirst` alongside this."
                            ),
                            Some(_) => format!(
                                "time data \"{s}\" doesn't match format \"{format}\", at position {pos}. \
                                 You might want to try:\n    - passing `format` if your strings have a consistent format;\n    - passing `format='ISO8601'` if your strings are all ISO8601 but not necessarily in exactly the same format;\n    - passing `format='mixed'`, and the format will be inferred for each element individually. You might want to use `dayfirst` alongside this."
                            ),
                        }));
                    }
                }
            }
            other => {
                // Numbers are parsed from their string form, as pandas does for objects.
                let s = other.to_py_str();
                match &compiled {
                    None => parse_iso8601(&s),
                    Some(f) => parse_strftime(f, &s).and_then(to_ns),
                }
                .ok_or_else(|| wrap(format!("Given date string {s} not likely a datetime")))?
            }
        };
        out.push(ns);
    }
    Ok(out)
}

/// Converts the labels of a coordinate to datetimes.
pub fn datetime_coord(coord: &Coord, format: &str) -> Result<Coord, ModelError> {
    let objs: Vec<Obj> = coord.labels().iter().map(Obj::from_label).collect();
    let ns = datetime_index(&objs, format)?;
    let labels: Vec<Label> = ns.into_iter().map(Label::Datetime).collect();
    Coord::new(coord.name(), labels).map_err(|e| ModelError::new(e.to_string()))
}

/// `check_time_subset`.
pub fn check_time_subset(ts_index: &[i64], time_subset: &[String]) -> Result<(), ModelError> {
    if time_subset.len() != 2 {
        return Err(ModelError::new(format!(
            "Timeseries subset must be a list of two timestamps. Received: {}",
            py_list(time_subset)
        )));
    }
    let parsed: Option<Vec<i64>> = time_subset.iter().map(|s| parse_iso8601(s)).collect();
    let Some(parsed) = parsed else {
        return Err(ModelError::new(format!(
            "Timeseries subset must be in ISO format (anything up to the  detail of \
             `%Y-%m-%d %H:%M:%S`).\n User time subset: {}\n Error caused: unparseable date",
            py_list(time_subset)
        )));
    };
    let (Some(first), Some(last)) = (ts_index.first(), ts_index.last()) else {
        return Ok(());
    };
    if floor_day(parsed[1]) < floor_day(*first) || floor_day(parsed[0]) > floor_day(*last) {
        return Err(ModelError::new(format!(
            "subset time range {} is outside the input data time range [{}, {}]",
            py_list(time_subset),
            format_datetime(*first),
            format_datetime(*last)
        )));
    }
    Ok(())
}

fn py_list(items: &[String]) -> String {
    let inner: Vec<String> = items.iter().map(|s| format!("'{s}'")).collect();
    format!("[{}]", inner.join(", "))
}

/// A resampling frequency.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freq {
    /// A fixed duration in nanoseconds (`h`, `min`, `s`, `ms`, `us`, `ns`, `D`).
    Tick(i64),
    /// Weeks ending on a weekday (0 = Monday), `W` = `W-SUN`.
    Week(u32),
    /// Calendar month (start or end).
    Month { start: bool },
    /// Calendar quarter (start or end).
    Quarter { start: bool },
    /// Calendar year (start or end).
    Year { start: bool },
}

static FREQ_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^(\d*)([A-Za-z]+)(?:-([A-Za-z]{3}))?$").expect("valid regex"));

/// Parses a pandas frequency string (pandas 3 aliases).
pub fn parse_freq(s: &str) -> Option<Freq> {
    let c = FREQ_RE.captures(s.trim())?;
    let n: i64 = match c.get(1).map(|m| m.as_str()).unwrap_or("") {
        "" => 1,
        t => t.parse().ok()?,
    };
    if n <= 0 {
        return None;
    }
    let unit = c.get(2)?.as_str();
    let anchor = c.get(3).map(|m| m.as_str().to_ascii_uppercase());
    let tick = |ns: i64| Some(Freq::Tick(n * ns));
    let calendar = |f: Freq| if n == 1 { Some(f) } else { None };
    match (unit, anchor.as_deref()) {
        ("D", None) => tick(NS_PER_DAY),
        ("h", None) => tick(NS_PER_HOUR),
        ("min", None) => tick(60 * NS_PER_SEC),
        ("s", None) => tick(NS_PER_SEC),
        ("ms", None) => tick(1_000_000),
        ("us", None) => tick(1_000),
        ("ns", None) => tick(1),
        ("W", a) => {
            let days = ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"];
            let wd = a.map_or(Some(6), |a| days.iter().position(|d| *d == a))?;
            calendar(Freq::Week(u32::try_from(wd).ok()?))
        }
        ("MS", None) => calendar(Freq::Month { start: true }),
        ("ME", None) => calendar(Freq::Month { start: false }),
        ("QS", None | Some("JAN")) => calendar(Freq::Quarter { start: true }),
        ("QE", None | Some("DEC")) => calendar(Freq::Quarter { start: false }),
        ("YS", None | Some("JAN")) => calendar(Freq::Year { start: true }),
        ("YE", None | Some("DEC")) => calendar(Freq::Year { start: false }),
        _ => None,
    }
}

fn ymd(y: i32, m: u32, d: u32) -> i64 {
    NaiveDate::from_ymd_opt(y, m, d)
        .and_then(|d| d.and_hms_opt(0, 0, 0))
        .and_then(to_ns)
        .unwrap_or(NAT)
}

fn month_add(y: i32, m: u32, k: i32) -> (i32, u32) {
    let total = y * 12 + i32::try_from(m).unwrap_or(1) - 1 + k;
    (
        total.div_euclid(12),
        u32::try_from(total.rem_euclid(12) + 1).unwrap_or(1),
    )
}

fn month_end(y: i32, m: u32) -> i64 {
    let (ny, nm) = month_add(y, m, 1);
    ymd(ny, nm, 1) - NS_PER_DAY
}

impl Freq {
    /// The bin label of a timestamp, following pandas' default `closed`/`label` per
    /// frequency (left for ticks and starts; right for weeks and ends), with fixed
    /// frequencies anchored at midnight of `origin`'s day.
    fn label(self, ns: i64, origin_day: i64) -> i64 {
        match self {
            Freq::Tick(step) => origin_day + (ns - origin_day).div_euclid(step) * step,
            Freq::Week(wd) => {
                let day = from_ns(floor_day(ns)).date();
                let cur = day.weekday().num_days_from_monday();
                let ahead = (wd + 7 - cur) % 7;
                floor_day(ns) + i64::from(ahead) * NS_PER_DAY
            }
            Freq::Month { start } => {
                let d = from_ns(ns).date();
                if start {
                    ymd(d.year(), d.month(), 1)
                } else {
                    month_end(d.year(), d.month())
                }
            }
            Freq::Quarter { start } => {
                let d = from_ns(ns).date();
                let q0 = (d.month() - 1) / 3 * 3 + 1;
                if start {
                    ymd(d.year(), q0, 1)
                } else {
                    month_end(d.year(), q0 + 2)
                }
            }
            Freq::Year { start } => {
                let d = from_ns(ns).date();
                if start {
                    ymd(d.year(), 1, 1)
                } else {
                    ymd(d.year(), 12, 31)
                }
            }
        }
    }

    fn next(self, label: i64) -> i64 {
        match self {
            Freq::Tick(step) => label + step,
            Freq::Week(_) => label + 7 * NS_PER_DAY,
            Freq::Month { start } | Freq::Quarter { start } | Freq::Year { start } => {
                let k = match self {
                    Freq::Month { .. } => 1,
                    Freq::Quarter { .. } => 3,
                    _ => 12,
                };
                let d = from_ns(label).date();
                if start {
                    let (y, m) = month_add(d.year(), d.month(), k);
                    ymd(y, m, 1)
                } else {
                    let (y, m) = month_add(d.year(), d.month(), k);
                    month_end(y, m)
                }
            }
        }
    }

    /// Bin labels covering `times` (sorted) and the bin of each timestamp.
    pub fn bins(self, times: &[i64]) -> (Vec<i64>, Vec<usize>) {
        let Some(first) = times.first() else {
            return (Vec::new(), Vec::new());
        };
        let origin = floor_day(*first);
        let last_label = self.label(*times.last().unwrap_or(first), origin);
        let mut labels = Vec::new();
        let mut l = self.label(*first, origin);
        while l <= last_label {
            labels.push(l);
            l = self.next(l);
        }
        let pos: HashMap<i64, usize> = labels.iter().enumerate().map(|(i, l)| (*l, i)).collect();
        let assign = times.iter().map(|t| pos[&self.label(*t, origin)]).collect();
        (labels, assign)
    }
}

/// Resamples one array along `dim` (datetime labels, ascending).
pub fn resample_array(
    da: &DataArray,
    dim: &str,
    labels: &[i64],
    assign: &[usize],
    method: ResampleMethod,
) -> DataArray {
    let axis = da
        .coords
        .iter()
        .position(|c| c.name() == dim)
        .expect("resampled dim exists");
    let new_coord = Coord::new(dim, labels.iter().map(|l| Label::Datetime(*l)).collect())
        .expect("bin labels are unique");
    let shape = da.shape();
    let outer: usize = shape[..axis].iter().product();
    let inner: usize = shape[axis + 1..].iter().product();
    let n_t = shape[axis];
    let n_b = labels.len();
    let mut out: Vec<Obj> = Vec::with_capacity(outer * n_b * inner);
    for o in 0..outer {
        for b in 0..n_b {
            for i in 0..inner {
                let vals = (0..n_t)
                    .filter(|t| assign[*t] == b)
                    .map(|t| da.data.get((o * n_t + t) * inner + i))
                    .filter(|v| !v.is_null());
                out.push(match method {
                    ResampleMethod::First => vals.into_iter().next().unwrap_or(Obj::Nan),
                    ResampleMethod::Mean => {
                        let v: Vec<f64> = vals.filter_map(|v| v.as_f64()).collect();
                        if v.is_empty() {
                            Obj::Nan
                        } else {
                            #[allow(clippy::cast_precision_loss)]
                            Obj::float(v.iter().sum::<f64>() / v.len() as f64)
                        }
                    }
                    ResampleMethod::Sum => {
                        let v: Vec<f64> = vals.filter_map(|v| v.as_f64()).collect();
                        if v.is_empty() {
                            Obj::Nan
                        } else {
                            Obj::float(v.iter().sum())
                        }
                    }
                });
            }
        }
    }
    let in_dtype = da.dtype();
    let dtype = match method {
        ResampleMethod::First => {
            if out.iter().any(Obj::is_null) {
                in_dtype.with_missing()
            } else {
                in_dtype
            }
        }
        _ => DType::Float,
    };
    let mut coords = da.coords.clone();
    coords[axis] = new_coord;
    DataArray::new(coords, Data::from_objs(dtype, out))
}

/// `resample`: resamples every variable over `dim` with its math `resample_method`.
pub fn resample(
    data: &Dataset,
    math: &CalliopeBuildMath,
    dim: &str,
    resolution: &str,
) -> Result<Dataset, ModelError> {
    let Some(freq) = parse_freq(resolution) else {
        return Err(ModelError::new(format!(
            "Unknown `{dim}` resampling frequency: {resolution}"
        )));
    };
    let coord = data.coord(dim).expect("checked by caller");
    let times: Vec<i64> = coord
        .labels()
        .iter()
        .map(|l| if let Label::Datetime(d) = l { *d } else { NAT })
        .collect();
    let (labels, assign) = freq.bins(&times);
    let new_coord = Coord::new(dim, labels.iter().map(|l| Label::Datetime(*l)).collect())
        .expect("bin labels are unique");
    let mut out = Dataset::new();
    for (name, c) in &data.coords {
        if name == dim {
            out.coords.insert(name.clone(), new_coord.clone());
        } else {
            out.coords.insert(name.clone(), c.clone());
        }
    }
    for (name, var) in &data.vars {
        let da = data.get(name).expect("exists");
        if !var.dims.iter().any(|d| d == dim) {
            out.set(name, da);
            continue;
        }
        let method = match math.find(name, Some(&["parameters", "lookups"])) {
            Ok(ComponentRef::Parameter(p)) => p.resample_method,
            Ok(ComponentRef::Lookup(l)) => l.resample_method,
            _ => {
                return Err(ModelError::new(format!(
                    "No resampling method defined in math configuration for timeseries input data `{name}`."
                )));
            }
        };
        tracing::debug!("Time Resampling | {name} | resampling function used: {method}");
        out.set(name, resample_array(&da, dim, &labels, &assign, method));
    }
    // xarray puts the variables without the resampled dim first (merge order).
    let mut ordered = Dataset {
        coords: out.coords.clone(),
        vars: Default::default(),
    };
    for (name, var) in &out.vars {
        if !var.dims.iter().any(|d| d == dim) {
            ordered.vars.insert(name.clone(), var.clone());
        }
    }
    for (name, var) in &out.vars {
        if var.dims.iter().any(|d| d == dim) {
            ordered.vars.insert(name.clone(), var.clone());
        }
    }
    Ok(ordered)
}

/// `add_inferred_time_params`: `timestep_resolution` (hours to the next timestep, the last
/// one repeated) and `timestep_weights` (ones).
pub fn add_inferred_time_params(ds: &mut Dataset) -> Vec<String> {
    let mut warnings = Vec::new();
    let Some(coord) = ds.coord("timesteps").cloned() else {
        return warnings;
    };
    let times: Vec<i64> = coord
        .labels()
        .iter()
        .map(|l| if let Label::Datetime(d) = l { *d } else { NAT })
        .collect();
    let n = times.len();
    #[allow(clippy::cast_precision_loss)]
    let mut res: Vec<f64> = (0..n)
        .map(|i| {
            if i + 1 < n {
                (times[i + 1] - times[i]) as f64 / NS_PER_HOUR as f64
            } else {
                f64::NAN
            }
        })
        .collect();
    if n == 1 {
        let msg = "Only one timestep defined. Inferring timestep resolution to be 1 hour";
        tracing::warn!("{msg}");
        warnings.push(msg.to_owned());
        res[0] = 1.0;
    } else if n > 1 {
        res[n - 1] = res[n - 2];
    }
    ds.set(
        "timestep_resolution",
        DataArray::new(vec![coord.clone()], Data::Float(res)),
    );
    ds.set(
        "timestep_weights",
        DataArray::new(vec![coord], Data::Float(vec![1.0; n])),
    );
    warnings
}

/// `cluster`: keeps the representative days given by `clustering_param` (a `datesteps`
/// array of dates) and adds `timestep_cluster`, `timestep_weights` and the
/// `lookup_cluster_*` arrays.
pub fn cluster(
    data: &Dataset,
    clustering_param: &str,
    time_format: &str,
) -> Result<Dataset, ModelError> {
    let param = data.get(clustering_param).ok_or_else(|| {
        ModelError::new(format!(
            "Time clustering parameter `{clustering_param}` is not defined."
        ))
    })?;
    let steps_dim = param
        .coords
        .first()
        .ok_or_else(|| ModelError::new("Time clustering parameter must be indexed over dates."))?
        .clone();
    // representative day per original date (series index: datesteps).
    let (index, repdays): (Vec<Label>, Vec<i64>) = match &param.data {
        Data::Datetime(v) => (
            steps_dim.labels().to_vec(),
            v.iter().map(|d| floor_day(*d)).collect(),
        ),
        d => {
            let mut idx = Vec::new();
            let mut vals = Vec::new();
            for (i, l) in steps_dim.labels().iter().enumerate() {
                let o = d.get(i);
                if o.is_null() {
                    continue;
                }
                idx.push(l.clone());
                vals.push(Obj::str(&format!("{} 00:00:00", o.to_py_str())));
            }
            let parsed = datetime_index(&vals, time_format)?;
            (idx, parsed.into_iter().map(floor_day).collect())
        }
    };
    // grouper: groups = sorted unique representative days.
    let groups: Vec<i64> = {
        let mut g = repdays.clone();
        g.sort_unstable();
        g.dedup();
        g
    };
    let group_id: BTreeMap<i64, i64> = groups
        .iter()
        .enumerate()
        .map(|(i, d)| (*d, i64::try_from(i).unwrap_or(0)))
        .collect();
    let mut weights: BTreeMap<i64, i64> = BTreeMap::new();
    for d in &repdays {
        *weights.entry(*d).or_default() += 1;
    }
    let ts = data
        .coord("timesteps")
        .ok_or_else(|| ModelError::new("Time clustering needs a `timesteps` dimension."))?
        .clone();
    let keep: Vec<Label> = ts
        .labels()
        .iter()
        .filter(|l| matches!(l, Label::Datetime(d) if weights.contains_key(&floor_day(*d))))
        .cloned()
        .collect();
    let mut out = data.clone();
    out.isel_labels("timesteps", keep);
    let ts = out.coord("timesteps").expect("kept").clone();
    let times: Vec<i64> = ts
        .labels()
        .iter()
        .map(|l| if let Label::Datetime(d) = l { *d } else { NAT })
        .collect();
    let cluster_of: Vec<i64> = times.iter().map(|t| group_id[&floor_day(*t)]).collect();
    out.set(
        "timestep_cluster",
        DataArray::new(vec![ts.clone()], Data::Int(cluster_of.clone())),
    );
    out.set(
        "timestep_weights",
        DataArray::new(
            vec![ts.clone()],
            Data::Int(times.iter().map(|t| weights[&floor_day(*t)]).collect()),
        ),
    );
    // datesteps coordinate: the clustering series' index.
    let datesteps =
        Coord::new("datesteps", index.clone()).map_err(|e| ModelError::new(e.to_string()))?;
    if datesteps.labels() != steps_dim.labels() || steps_dim.name() != "datesteps" {
        out.coords.insert("datesteps".to_owned(), datesteps.clone());
    }
    let mut clusters: Vec<i64> = cluster_of.clone();
    clusters.sort_unstable();
    clusters.dedup();
    out.coords.insert(
        "clusters".to_owned(),
        Coord::new("clusters", clusters.into_iter().map(Label::Int).collect())
            .expect("unique clusters"),
    );
    // _lookup_clusters
    let mut first_of_day: HashMap<i64, i64> = HashMap::new();
    let mut last_of_day: HashMap<i64, i64> = HashMap::new();
    for t in &times {
        first_of_day.entry(floor_day(*t)).or_insert(*t);
        last_of_day.insert(floor_day(*t), *t);
    }
    out.set(
        "cluster_first_timestep",
        DataArray::new(
            vec![ts.clone()],
            Data::Bool(
                times
                    .iter()
                    .map(|t| first_of_day[&floor_day(*t)] == *t)
                    .collect(),
            ),
        ),
    );
    out.set(
        "lookup_cluster_last_timestep",
        DataArray::new(
            vec![ts.clone()],
            Data::Datetime(
                times
                    .iter()
                    .map(|t| {
                        if floor_day(*t) == *t {
                            last_of_day[t]
                        } else {
                            NAT
                        }
                    })
                    .collect(),
            ),
        ),
    );
    let datestep_cluster: Vec<i64> = repdays.iter().map(|d| group_id[d]).collect();
    let mut last_ts_of_cluster: HashMap<i64, i64> = HashMap::new();
    for (t, c) in times.iter().zip(&cluster_of) {
        last_ts_of_cluster.insert(*c, *t);
    }
    out.set(
        "lookup_datestep_cluster",
        DataArray::new(vec![datesteps.clone()], Data::Int(datestep_cluster.clone())),
    );
    out.set(
        "lookup_datestep_last_cluster_timestep",
        DataArray::new(
            vec![datesteps],
            Data::Datetime(
                datestep_cluster
                    .iter()
                    .map(|c| last_ts_of_cluster.get(c).copied().unwrap_or(NAT))
                    .collect(),
            ),
        ),
    );
    Ok(out)
}
