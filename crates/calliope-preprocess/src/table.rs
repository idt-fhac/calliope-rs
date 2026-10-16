//! pandas `read_csv` + `stack`/`unstack`/`to_xarray`, as `DataTable` uses them.
//!
//! A loaded table is turned into a long series ([`Long`]): one row per non-missing cell,
//! labelled by one label per index level. Level label order follows pandas: a single index
//! keeps file order, a MultiIndex (several header rows or index columns) has sorted levels,
//! `add_dims` levels keep the given order.

use std::collections::HashSet;
use std::path::Path;

use calliope_array::{Coord, Label};

use crate::xr::{DType, Data, DataArray, Dataset, Obj, comparable, parse_py_float};

/// pandas' default NA strings.
const NA_VALUES: [&str; 19] = [
    "", "#N/A", "#N/A N/A", "#NA", "-1.#IND", "-1.#QNAN", "-NaN", "-nan", "1.#IND", "1.#QNAN",
    "<NA>", "N/A", "NA", "NULL", "NaN", "None", "n/a", "nan", "null",
];

fn is_na(s: &str) -> bool {
    NA_VALUES.contains(&s)
}

fn parse_int(s: &str) -> Option<i64> {
    let t = s.trim();
    let t = t.strip_prefix('+').unwrap_or(t);
    if t.is_empty()
        || !t
            .trim_start_matches('-')
            .chars()
            .all(|c| c.is_ascii_digit())
    {
        return None;
    }
    t.parse().ok()
}

fn parse_bool(s: &str) -> Option<bool> {
    match s {
        "True" | "TRUE" | "true" => Some(true),
        "False" | "FALSE" | "false" => Some(false),
        _ => None,
    }
}

/// Exact powers of ten as C double literals (`1e0` .. `1e308`), as in pandas' tokenizer.
fn pow10(n: usize) -> f64 {
    static TABLE: std::sync::OnceLock<Vec<f64>> = std::sync::OnceLock::new();
    TABLE.get_or_init(|| {
        (0..=308)
            .map(|i| format!("1e{i}").parse().unwrap_or(f64::INFINITY))
            .collect()
    })[n]
}

/// pandas' `precise_xstrtod` (`read_csv`'s default float converter): at most 17
/// significant digits are accumulated (leading zeros count, further digits are dropped),
/// then scaled by a power of ten. Not always correctly rounded: `0.17191199999999998`
/// reads as `0.1719119999999999`. `None` if `s` is not a plain decimal number.
fn precise_xstrtod(s: &str) -> Option<f64> {
    const MAX_DIGITS: i32 = 17;
    let b = s.trim().as_bytes();
    let mut p = 0;
    let negative = match b.first() {
        Some(b'-') => {
            p += 1;
            true
        }
        Some(b'+') => {
            p += 1;
            false
        }
        _ => false,
    };
    let (mut number, mut exponent, mut num_digits) = (0.0f64, 0i64, 0i32);
    while p < b.len() && b[p].is_ascii_digit() {
        if num_digits < MAX_DIGITS {
            number = number * 10.0 + f64::from(b[p] - b'0');
            num_digits += 1;
        } else {
            exponent += 1;
        }
        p += 1;
    }
    if p < b.len() && b[p] == b'.' {
        p += 1;
        let mut num_decimals = 0;
        while p < b.len() && num_digits < MAX_DIGITS && b[p].is_ascii_digit() {
            number = number * 10.0 + f64::from(b[p] - b'0');
            p += 1;
            num_digits += 1;
            num_decimals += 1;
        }
        while p < b.len() && b[p].is_ascii_digit() {
            p += 1;
        }
        exponent -= num_decimals;
    }
    if num_digits == 0 {
        return None;
    }
    if negative {
        number = -number;
    }
    if p < b.len() && (b[p] == b'e' || b[p] == b'E') {
        p += 1;
        let neg_exp = match b.get(p) {
            Some(b'-') => {
                p += 1;
                true
            }
            Some(b'+') => {
                p += 1;
                false
            }
            _ => false,
        };
        let start = p;
        let mut n: i64 = 0;
        while p < b.len() && b[p].is_ascii_digit() {
            n = n.saturating_mul(10).saturating_add(i64::from(b[p] - b'0'));
            p += 1;
        }
        if p == start {
            return None;
        }
        exponent += if neg_exp { -n } else { n };
    }
    if p != b.len() {
        return None;
    }
    #[allow(clippy::cast_sign_loss, clippy::cast_possible_truncation)]
    let v = if exponent > 308 {
        if number == 0.0 {
            0.0
        } else {
            number.signum() * f64::INFINITY
        }
    } else if exponent > 0 {
        number * pow10(exponent as usize)
    } else if exponent < -308 {
        if exponent < -616 {
            0.0
        } else {
            number / pow10((-308 - exponent) as usize) / pow10(308)
        }
    } else {
        number / pow10((-exponent) as usize)
    };
    Some(v)
}

/// A float cell as pandas' `read_csv` reads it ([`precise_xstrtod`]; other spellings such
/// as `inf` and `nan` as Python does).
fn csv_float(s: &str) -> Option<f64> {
    precise_xstrtod(s).or_else(|| parse_py_float(s))
}

/// pandas C-parser type inference for one column of raw cells: `int64` (no missing),
/// `float64`, `bool` (no missing), else `object` with the raw strings (NA → NaN).
pub fn infer_column(cells: &[&str]) -> Data {
    let na: Vec<bool> = cells.iter().map(|c| is_na(c)).collect();
    let any_na = na.iter().any(|x| *x);
    let present = || cells.iter().zip(&na).filter(|(_, n)| !**n).map(|(c, _)| *c);
    if present().all(|c| parse_int(c).is_some()) {
        if !any_na {
            return Data::Int(cells.iter().map(|c| parse_int(c).unwrap_or(0)).collect());
        }
        #[allow(clippy::cast_precision_loss)]
        return Data::Float(
            cells
                .iter()
                .zip(&na)
                .map(|(c, n)| {
                    if *n {
                        f64::NAN
                    } else {
                        parse_int(c).unwrap_or(0) as f64
                    }
                })
                .collect(),
        );
    }
    if present().all(|c| csv_float(c).is_some()) {
        return Data::Float(
            cells
                .iter()
                .zip(&na)
                .map(|(c, n)| {
                    if *n {
                        f64::NAN
                    } else {
                        csv_float(c).unwrap_or(f64::NAN)
                    }
                })
                .collect(),
        );
    }
    if present().all(|c| parse_bool(c).is_some()) {
        if !any_na {
            return Data::Bool(
                cells
                    .iter()
                    .map(|c| parse_bool(c).unwrap_or(false))
                    .collect(),
            );
        }
        return Data::Object(
            cells
                .iter()
                .zip(&na)
                .map(|(c, n)| {
                    if *n {
                        Obj::Nan
                    } else {
                        Obj::Bool(parse_bool(c).unwrap_or(false))
                    }
                })
                .collect(),
        );
    }
    Data::Object(
        cells
            .iter()
            .zip(&na)
            .map(|(c, n)| if *n { Obj::Nan } else { Obj::str(c) })
            .collect(),
    )
}

/// `pandas.api.types.find_common_type` of column dtypes (used by `stack`).
fn common_dtype(dtypes: &[DType]) -> DType {
    let mut it = dtypes.iter().copied();
    let Some(first) = it.next() else {
        return DType::Object;
    };
    it.fold(first, |a, b| match (a, b) {
        (a, b) if a == b => a,
        (DType::Int, DType::Float) | (DType::Float, DType::Int) => DType::Float,
        _ => DType::Object,
    })
}

/// A table as read by `pd.read_csv(header=..., index_col=...)`.
#[derive(Debug, Clone)]
pub struct Frame {
    /// Index level names (`None` = unnamed).
    pub index_names: Vec<Option<String>>,
    /// Index labels per row, one per index level.
    pub index: Vec<Vec<Label>>,
    /// Column level names.
    pub column_names: Vec<Option<String>>,
    /// Column labels, one per column level.
    pub columns: Vec<Vec<Label>>,
    /// Column data.
    pub data: Vec<Data>,
}

/// Errors reading a table.
pub type TableResult<T> = Result<T, String>;

fn label_of(o: &Obj) -> Label {
    o.to_label()
}

/// Reads a CSV file like `pd.read_csv(path, header=list(range(n_header)),
/// index_col=list(range(n_index)) or None)`.
pub fn read_csv(path: &Path, n_header: usize, n_index: Option<usize>) -> TableResult<Frame> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    read_csv_str(text, n_header, n_index)
}

/// [`read_csv`] on a string.
pub fn read_csv_str(text: &str, n_header: usize, n_index: Option<usize>) -> TableResult<Frame> {
    let mut rdr = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_reader(text.as_bytes());
    let mut rows: Vec<Vec<String>> = Vec::new();
    for rec in rdr.records() {
        let rec = rec.map_err(|e| e.to_string())?;
        // pandas skips blank lines.
        if rec.len() == 1 && rec.get(0).is_some_and(|f| f.trim().is_empty()) {
            continue;
        }
        rows.push(rec.iter().map(str::to_owned).collect());
    }
    if rows.len() < n_header {
        return Err("Passed header names mismatches usecols".to_owned());
    }
    let header: Vec<Vec<String>> = rows.drain(..n_header).collect();
    let ic = n_index.unwrap_or(0);
    let width = header.iter().map(Vec::len).max().unwrap_or(0);
    // Index names.
    let mut index_names: Vec<Option<String>> = vec![None; ic];
    let none_if_empty = |s: &str| {
        if s.is_empty() {
            None
        } else {
            Some(s.to_owned())
        }
    };
    if ic > 0 {
        if n_header > 1 {
            // An extra row after multi-row headers holds index names if it has only them.
            if let Some(next) = rows.first()
                && next.len() <= width
                && next.iter().skip(ic).all(String::is_empty)
                && next.iter().take(ic).any(|s| !s.is_empty())
            {
                index_names = (0..ic)
                    .map(|i| next.get(i).and_then(|s| none_if_empty(s)))
                    .collect();
                rows.remove(0);
            }
        } else if let Some(h) = header.first() {
            index_names = (0..ic)
                .map(|i| h.get(i).and_then(|s| none_if_empty(s)))
                .collect();
        }
    }
    // Columns.
    let n_cols = width.saturating_sub(ic);
    let mut column_names: Vec<Option<String>> = vec![None; n_header];
    if ic > 0 && n_header > 1 {
        for (k, h) in header.iter().enumerate() {
            column_names[k] = h.get(ic - 1).and_then(|s| none_if_empty(s));
        }
    }
    let mut columns: Vec<Vec<Label>> = Vec::with_capacity(n_cols);
    for j in 0..n_cols {
        let pos = j + ic;
        let labels = header
            .iter()
            .enumerate()
            .map(|(k, h)| {
                let cell = h.get(pos).map_or("", String::as_str);
                if cell.is_empty() {
                    if n_header > 1 {
                        Label::str(&format!("Unnamed: {pos}_level_{k}"))
                    } else {
                        Label::str(&format!("Unnamed: {pos}"))
                    }
                } else {
                    Label::str(cell)
                }
            })
            .collect();
        columns.push(labels);
    }
    if n_header == 1 {
        // pandas mangles duplicate column names (`a`, `a.1`, ...).
        let mut seen: HashSet<String> = HashSet::new();
        for col in &mut columns {
            let base = col[0].to_string();
            let mut name = base.clone();
            let mut k = 1;
            while !seen.insert(name.clone()) {
                name = format!("{base}.{k}");
                k += 1;
            }
            col[0] = Label::str(&name);
        }
    }
    // Index columns and data columns.
    let cell = |r: &Vec<String>, i: usize| r.get(i).map_or("", String::as_str).to_owned();
    let mut index: Vec<Vec<Label>> = vec![Vec::with_capacity(ic); rows.len()];
    for i in 0..ic {
        let cells: Vec<String> = rows.iter().map(|r| cell(r, i)).collect();
        let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
        let d = infer_column(&refs);
        for (r, row) in index.iter_mut().enumerate() {
            row.push(label_of(&d.get(r)));
        }
    }
    if ic == 0 {
        for (r, row) in index.iter_mut().enumerate() {
            row.push(Label::Int(i64::try_from(r).unwrap_or(0)));
        }
        index_names = vec![None];
    }
    let mut data = Vec::with_capacity(n_cols);
    for j in 0..n_cols {
        let cells: Vec<String> = rows.iter().map(|r| cell(r, j + ic)).collect();
        let refs: Vec<&str> = cells.iter().map(String::as_str).collect();
        data.push(infer_column(&refs));
    }
    Ok(Frame {
        index_names,
        index,
        column_names,
        columns,
        data,
    })
}

/// A long (tidy) series with a MultiIndex.
#[derive(Debug, Clone)]
pub struct Long {
    /// Level names.
    pub names: Vec<Option<String>>,
    /// Labels of each level, in level order (may contain unused labels).
    pub levels: Vec<Vec<Label>>,
    /// Per row: the label code of each level.
    pub codes: Vec<Vec<u32>>,
    /// Per row: the value.
    pub values: Vec<Obj>,
    /// The series dtype.
    pub dtype: DType,
}

/// Unique labels in order of appearance (pandas `factorize`), or sorted (MultiIndex levels).
fn factorize(labels: impl Iterator<Item = Label>, sort: bool) -> (Vec<Label>, Vec<u32>) {
    let all: Vec<Label> = labels.collect();
    let mut uniq: Vec<Label> = Vec::new();
    let mut seen: std::collections::HashMap<Label, u32> = Default::default();
    for l in &all {
        if !seen.contains_key(l) {
            seen.insert(l.clone(), u32::try_from(uniq.len()).unwrap_or(0));
            uniq.push(l.clone());
        }
    }
    if sort && comparable(&uniq) {
        uniq.sort();
    }
    let pos: std::collections::HashMap<&Label, u32> = uniq
        .iter()
        .enumerate()
        .map(|(i, l)| (l, u32::try_from(i).unwrap_or(0)))
        .collect();
    let codes = all.iter().map(|l| pos[l]).collect();
    (uniq, codes)
}

impl Frame {
    /// The data as a long series: `df.stack(columns).dropna()`, or the squeezed column if
    /// `squeeze_columns` and there is a single one (no `dropna` then, as in Python).
    pub fn to_long(&self, squeeze_columns: bool) -> Long {
        let n_rows = self.index.len();
        let multi_index = self.index_names.len() > 1;
        let mut levels = Vec::new();
        let mut row_codes: Vec<Vec<u32>> = vec![Vec::new(); n_rows];
        for k in 0..self.index_names.len() {
            let (lv, codes) = factorize(self.index.iter().map(|r| r[k].clone()), multi_index);
            levels.push(lv);
            for (r, c) in codes.into_iter().enumerate() {
                row_codes[r].push(c);
            }
        }
        if squeeze_columns && self.data.len() == 1 {
            let d = &self.data[0];
            return Long {
                names: self.index_names.clone(),
                levels,
                codes: row_codes,
                values: d.objs(),
                dtype: d.dtype(),
            };
        }
        let multi_cols = self.column_names.len() > 1;
        let mut names = self.index_names.clone();
        let mut col_codes: Vec<Vec<u32>> = vec![Vec::new(); self.columns.len()];
        for k in 0..self.column_names.len() {
            let (lv, codes) = factorize(self.columns.iter().map(|c| c[k].clone()), multi_cols);
            levels.push(lv);
            names.push(self.column_names[k].clone());
            for (j, c) in codes.into_iter().enumerate() {
                col_codes[j].push(c);
            }
        }
        let dtype = common_dtype(&self.data.iter().map(Data::dtype).collect::<Vec<_>>());
        let mut codes = Vec::new();
        let mut values = Vec::new();
        for (r, rc) in row_codes.iter().enumerate() {
            for (j, cc) in col_codes.iter().enumerate() {
                let v = self.data[j].get(r);
                if v.is_null() {
                    continue;
                }
                let mut key = rc.clone();
                key.extend(cc);
                codes.push(key);
                values.push(v);
            }
        }
        Long {
            names,
            levels,
            codes,
            values,
            dtype,
        }
    }
}

impl Long {
    /// Position of a named level.
    pub fn level(&self, name: &str) -> Option<usize> {
        self.names.iter().position(|n| n.as_deref() == Some(name))
    }

    /// `tdf.loc[selector]`: keeps rows whose label in `name` is one of `labels`.
    pub fn select(&mut self, name: &str, labels: &[String]) {
        let Some(k) = self.level(name) else { return };
        let keep: HashSet<u32> = self.levels[k]
            .iter()
            .enumerate()
            .filter(|(_, l)| labels.iter().any(|s| l.to_string() == *s))
            .map(|(i, _)| u32::try_from(i).unwrap_or(0))
            .collect();
        let mut codes = Vec::new();
        let mut values = Vec::new();
        for (c, v) in self.codes.iter().zip(&self.values) {
            if keep.contains(&c[k]) {
                codes.push(c.clone());
                values.push(v.clone());
            }
        }
        self.codes = codes;
        self.values = values;
    }

    /// `droplevel(name)`.
    pub fn drop_level(&mut self, name: &str) -> Result<(), String> {
        let k = self.level(name).ok_or_else(|| {
            format!(
                "'Requested level ({name}) does not match index name ({})'",
                self.names_str()
            )
        })?;
        self.names.remove(k);
        self.levels.remove(k);
        for c in &mut self.codes {
            c.remove(k);
        }
        Ok(())
    }

    /// `pd.concat([tdf for _ in items], keys=items, names=[name])`: a new first level.
    pub fn add_dim(&mut self, name: &str, items: &[String]) {
        let n = self.values.len();
        let mut codes = Vec::with_capacity(n * items.len());
        let mut values = Vec::with_capacity(n * items.len());
        for (i, _) in items.iter().enumerate() {
            for (c, v) in self.codes.iter().zip(&self.values) {
                let mut key = vec![u32::try_from(i).unwrap_or(0)];
                key.extend(c);
                codes.push(key);
                values.push(v.clone());
            }
        }
        self.names.insert(0, Some(name.to_owned()));
        self.levels
            .insert(0, items.iter().map(|s| Label::str(s)).collect());
        self.codes = codes;
        self.values = values;
    }

    /// Level names as pandas prints them (`['a', None]`).
    pub fn names_str(&self) -> String {
        let parts: Vec<String> = self
            .names
            .iter()
            .map(|n| {
                n.as_ref()
                    .map_or_else(|| "None".to_owned(), |s| format!("'{s}'"))
            })
            .collect();
        format!("[{}]", parts.join(", "))
    }

    /// Rows whose index occurs more than once.
    pub fn duplicated(&self) -> Vec<usize> {
        let mut seen: HashSet<&Vec<u32>> = HashSet::new();
        self.codes
            .iter()
            .enumerate()
            .filter(|(_, c)| !seen.insert(c))
            .map(|(i, _)| i)
            .collect()
    }

    /// The label tuple of a row, as pandas prints it.
    pub fn row_label(&self, r: usize) -> String {
        let parts: Vec<String> = self.codes[r]
            .iter()
            .enumerate()
            .map(|(k, c)| format!("'{}'", self.levels[k][*c as usize]))
            .collect();
        format!("({})", parts.join(", "))
    }

    /// `unstack("inputs").infer_objects().to_xarray()`: one variable per input, each over
    /// all remaining levels (unused labels dropped).
    pub fn unstack_to_dataset(&self, inputs: &str) -> Dataset {
        let k_in = self.level(inputs).expect("checked");
        let dims: Vec<usize> = (0..self.names.len()).filter(|k| *k != k_in).collect();
        // remove unused levels (keeping level order)
        let mut coords: Vec<Coord> = Vec::new();
        let mut remap: Vec<Vec<Option<u32>>> = Vec::new();
        for &k in &dims {
            let used: HashSet<u32> = self.codes.iter().map(|c| c[k]).collect();
            let mut map = vec![None; self.levels[k].len()];
            let mut labels = Vec::new();
            for (i, l) in self.levels[k].iter().enumerate() {
                if used.contains(&u32::try_from(i).unwrap_or(0)) {
                    map[i] = Some(u32::try_from(labels.len()).unwrap_or(0));
                    labels.push(l.clone());
                }
            }
            let name = self.names[k]
                .clone()
                .unwrap_or_else(|| format!("level_{k}"));
            coords.push(Coord::new(&name, labels).expect("level labels are unique"));
            remap.push(map);
        }
        let shape: Vec<usize> = coords.iter().map(Coord::len).collect();
        let strides = {
            let mut s = vec![1usize; shape.len()];
            for i in (0..shape.len().saturating_sub(1)).rev() {
                s[i] = s[i + 1] * shape[i + 1];
            }
            s
        };
        let total: usize = shape.iter().product();
        // rows of the unstacked frame: distinct remaining-index tuples
        let mut row_set: HashSet<usize> = HashSet::new();
        let mut ds = Dataset::new();
        for c in &coords {
            ds.coords.insert(c.name().to_owned(), c.clone());
        }
        let n_inputs = self.levels[k_in].len();
        let used_inputs: HashSet<u32> = self.codes.iter().map(|c| c[k_in]).collect();
        let mut cells: Vec<Vec<Option<Obj>>> = vec![vec![None; total]; n_inputs];
        for (c, v) in self.codes.iter().zip(&self.values) {
            let flat: usize = dims
                .iter()
                .enumerate()
                .map(|(d, &k)| remap[d][c[k] as usize].unwrap_or(0) as usize * strides[d])
                .sum();
            row_set.insert(flat);
            cells[c[k_in] as usize][flat] = Some(v.clone());
        }
        for (i, input) in self.levels[k_in].iter().enumerate() {
            if !used_inputs.contains(&u32::try_from(i).unwrap_or(0)) {
                continue;
            }
            // unstack: rows are the observed tuples; missing combinations are NaN.
            let col: Vec<Obj> = cells[i]
                .iter()
                .enumerate()
                .filter(|(f, _)| row_set.contains(f))
                .map(|(_, v)| v.clone().unwrap_or(Obj::Nan))
                .collect();
            let unstack_dtype = if col.iter().any(Obj::is_null) {
                match self.dtype {
                    DType::Bool => DType::Object,
                    d => d.with_missing(),
                }
            } else {
                self.dtype
            };
            let col_dtype = if unstack_dtype == DType::Object {
                infer_objects(&col)
            } else {
                unstack_dtype
            };
            // to_xarray: full product, missing cells promoted with `maybe_promote`.
            let full = row_set.len() == total;
            let out_dtype = if full {
                col_dtype
            } else {
                col_dtype.maybe_promote()
            };
            let values: Vec<Obj> = cells[i]
                .iter()
                .map(|v| v.clone().unwrap_or(Obj::Nan))
                .collect();
            let data = Data::from_objs(out_dtype, values);
            ds.set(&input.to_string(), DataArray::new(coords.clone(), data));
        }
        ds
    }

    /// `xr.Dataset(tdf.to_dict())` for a series indexed only by `inputs`: scalars.
    pub fn to_scalars(&self) -> Dataset {
        let mut ds = Dataset::new();
        for (c, v) in self.codes.iter().zip(&self.values) {
            let name = self.levels[0][c[0] as usize].to_string();
            ds.set(&name, DataArray::scalar(v.clone()));
        }
        ds
    }
}

/// `infer_objects` on an object column: the dtype pandas converts it to.
fn infer_objects(col: &[Obj]) -> DType {
    let present: Vec<&Obj> = col.iter().filter(|o| !o.is_null()).collect();
    let any_null = present.len() != col.len();
    if present.is_empty() {
        return DType::Object;
    }
    if present.iter().all(|o| matches!(o, Obj::Bool(_))) {
        return if any_null { DType::Object } else { DType::Bool };
    }
    if present.iter().all(|o| matches!(o, Obj::Int(_))) {
        return if any_null { DType::Float } else { DType::Int };
    }
    if present
        .iter()
        .all(|o| matches!(o, Obj::Int(_) | Obj::Float(_)))
    {
        return DType::Float;
    }
    DType::Object
}

#[cfg(test)]
mod float_tests {
    use super::precise_xstrtod;

    #[test]
    fn pandas_float_parsing() {
        // Values pandas 3.0 `read_csv` returns (float_precision=None).
        assert_eq!(
            precise_xstrtod("0.17191199999999998"),
            Some(0.171_911_999_999_999_9)
        );
        assert_eq!(precise_xstrtod("0.171912"), Some(0.171_912));
        assert_eq!(precise_xstrtod("-1.5e3"), Some(-1500.0));
        assert_eq!(precise_xstrtod("1e-320"), Some(1e-320));
        assert_eq!(precise_xstrtod(" 2 "), Some(2.0));
        assert_eq!(precise_xstrtod("1e"), None);
        assert_eq!(precise_xstrtod("inf"), None);
        assert_eq!(precise_xstrtod("."), None);
    }
}
