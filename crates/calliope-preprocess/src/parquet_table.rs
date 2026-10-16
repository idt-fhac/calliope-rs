//! Parquet data tables, read like `pandas.read_parquet` reads a file that
//! `DataFrame.to_parquet` (pyarrow) wrote.
//!
//! Calliope itself only reads CSV files; Parquet tables are an extension. The `pandas`
//! schema metadata restores the frame: index columns become index levels (a stored
//! `RangeIndex` becomes `0..n`), column level names come from `column_indexes` and
//! MultiIndex column labels from their stringified tuples (`"('a', 'b')"`). Without that
//! metadata every column is data and the index is `0..n`. Column dtypes follow pandas:
//! integers with missing values become floats, booleans with missing values objects.
//!
//! Only snappy-compressed or uncompressed files are supported (pandas' default is snappy).

use std::fs::File;
use std::path::Path;

use calliope_array::Label;
use parquet::basic::{LogicalType, TimeUnit};
use parquet::file::reader::{FileReader, SerializedFileReader};
use parquet::record::Field;

use crate::table::{Frame, TableResult};
use crate::xr::{Data, Obj};

/// A cell value, typed.
#[derive(Debug, Clone, PartialEq)]
enum Cell {
    Null,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Datetime(i64),
}

fn cell(f: &Field, nanos: bool) -> Cell {
    match f {
        Field::Null => Cell::Null,
        Field::Bool(b) => Cell::Bool(*b),
        Field::Byte(i) => Cell::Int(i64::from(*i)),
        Field::Short(i) => Cell::Int(i64::from(*i)),
        Field::Int(i) => Cell::Int(i64::from(*i)),
        Field::Long(i) if nanos => Cell::Datetime(*i),
        Field::Long(i) => Cell::Int(*i),
        Field::UByte(i) => Cell::Int(i64::from(*i)),
        Field::UShort(i) => Cell::Int(i64::from(*i)),
        Field::UInt(i) => Cell::Int(i64::from(*i)),
        Field::ULong(i) => i64::try_from(*i).map_or(Cell::Float(*i as f64), Cell::Int),
        Field::Float16(f) => Cell::Float(f64::from(*f)),
        Field::Float(f) => Cell::Float(f64::from(*f)),
        Field::Double(f) => Cell::Float(*f),
        Field::Str(s) => Cell::Str(s.clone()),
        Field::Bytes(b) => Cell::Str(String::from_utf8_lossy(b.data()).into_owned()),
        Field::Date(d) => Cell::Datetime(i64::from(*d) * 86_400_000_000_000),
        Field::TimestampMillis(t) => Cell::Datetime(t * 1_000_000),
        Field::TimestampMicros(t) => Cell::Datetime(t * 1_000),
        other => Cell::Str(other.to_string()),
    }
}

/// A column as pandas would type it.
fn column_data(cells: &[Cell]) -> Data {
    let present = || cells.iter().filter(|c| **c != Cell::Null);
    let any_null = cells.contains(&Cell::Null);
    if present().all(|c| matches!(c, Cell::Int(_))) && !any_null && !cells.is_empty() {
        return Data::Int(
            cells
                .iter()
                .map(|c| if let Cell::Int(i) = c { *i } else { 0 })
                .collect(),
        );
    }
    if present().all(|c| matches!(c, Cell::Int(_) | Cell::Float(_))) {
        #[allow(clippy::cast_precision_loss)]
        return Data::Float(
            cells
                .iter()
                .map(|c| match c {
                    Cell::Int(i) => *i as f64,
                    Cell::Float(f) => *f,
                    _ => f64::NAN,
                })
                .collect(),
        );
    }
    if present().all(|c| matches!(c, Cell::Datetime(_))) {
        return Data::Datetime(
            cells
                .iter()
                .map(|c| {
                    if let Cell::Datetime(t) = c {
                        *t
                    } else {
                        i64::MIN
                    }
                })
                .collect(),
        );
    }
    if present().all(|c| matches!(c, Cell::Bool(_))) && !any_null {
        return Data::Bool(cells.iter().map(|c| *c == Cell::Bool(true)).collect());
    }
    Data::Object(
        cells
            .iter()
            .map(|c| match c {
                Cell::Null => Obj::Nan,
                Cell::Bool(b) => Obj::Bool(*b),
                Cell::Int(i) => Obj::Int(*i),
                Cell::Float(f) if f.is_nan() => Obj::Nan,
                Cell::Float(f) => Obj::Float(*f),
                Cell::Str(s) => Obj::str(s),
                Cell::Datetime(t) => Obj::Datetime(*t),
            })
            .collect(),
    )
}

fn label(c: &Cell) -> Label {
    match c {
        Cell::Null => Label::Float(f64::NAN),
        Cell::Bool(b) => Label::str(if *b { "True" } else { "False" }),
        Cell::Int(i) => Label::Int(*i),
        Cell::Float(f) => Label::Float(*f),
        Cell::Str(s) => Label::str(s),
        Cell::Datetime(t) => Label::Datetime(*t),
    }
}

/// Parses a Python tuple of string/number literals, as pandas stores MultiIndex column
/// labels (`"('a', 'b')"`). `None` if `s` is not one.
fn parse_tuple(s: &str) -> Option<Vec<String>> {
    let inner = s.strip_prefix('(')?.strip_suffix(')')?;
    let mut out = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        let Some(&c) = chars.peek() else { break };
        if c == '\'' || c == '"' {
            chars.next();
            let mut item = String::new();
            loop {
                match chars.next()? {
                    '\\' => match chars.next()? {
                        'n' => item.push('\n'),
                        't' => item.push('\t'),
                        other => item.push(other),
                    },
                    q if q == c => break,
                    other => item.push(other),
                }
            }
            out.push(item);
        } else {
            let mut item = String::new();
            while let Some(&c) = chars.peek() {
                if c == ',' {
                    break;
                }
                item.push(c);
                chars.next();
            }
            out.push(item.trim().to_owned());
        }
        while chars.peek().is_some_and(|c| c.is_whitespace()) {
            chars.next();
        }
        match chars.next() {
            Some(',') | None => {}
            Some(_) => return None,
        }
    }
    Some(out)
}

/// The parts of the `pandas` schema metadata used here.
#[derive(Debug, Default)]
struct PandasMeta {
    /// Stored index columns (field names), or `None` for a `RangeIndex`.
    index_columns: Vec<Option<String>>,
    /// Index level names, by stored field name.
    names: Vec<(String, Option<String>)>,
    /// Column level names.
    column_levels: Vec<Option<String>>,
}

fn pandas_meta(json: &str) -> TableResult<PandasMeta> {
    let v: serde_json::Value =
        serde_json::from_str(json).map_err(|e| format!("pandas metadata: {e}"))?;
    let name = |x: &serde_json::Value| x.as_str().map(str::to_owned);
    let index_columns = v["index_columns"]
        .as_array()
        .map(|a| a.iter().map(name).collect())
        .unwrap_or_default();
    let names = v["columns"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|c| Some((c["field_name"].as_str()?.to_owned(), name(&c["name"]))))
                .collect()
        })
        .unwrap_or_default();
    let column_levels = v["column_indexes"]
        .as_array()
        .map(|a| a.iter().map(|c| name(&c["name"])).collect())
        .unwrap_or_default();
    Ok(PandasMeta {
        index_columns,
        names,
        column_levels,
    })
}

/// Reads a Parquet file as `pd.read_parquet(path)` does.
pub fn read_parquet(path: &Path) -> TableResult<Frame> {
    let err = |e: parquet::errors::ParquetError| format!("{}: {e}", path.display());
    let file = File::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let reader = SerializedFileReader::new(file).map_err(err)?;
    let meta = reader.metadata().file_metadata();
    let pandas = meta
        .key_value_metadata()
        .and_then(|kv| kv.iter().find(|k| k.key == "pandas"))
        .and_then(|k| k.value.as_deref())
        .map(pandas_meta)
        .transpose()?;
    let schema = meta.schema_descr();
    let fields: Vec<String> = schema.columns().iter().map(|c| c.path().string()).collect();
    let nanos: Vec<bool> = schema
        .columns()
        .iter()
        .map(|c| {
            matches!(c.logical_type_ref(), Some(LogicalType::Timestamp(t)) if t.unit == TimeUnit::NANOS)
        })
        .collect();
    let mut cols: Vec<Vec<Cell>> = vec![Vec::new(); fields.len()];
    for row in reader.get_row_iter(None).map_err(err)? {
        let row = row.map_err(err)?;
        for (i, (_, f)) in row.get_column_iter().enumerate() {
            if let Some(col) = cols.get_mut(i) {
                col.push(cell(f, nanos[i]));
            }
        }
    }
    let n_rows = cols.first().map_or(0, Vec::len);
    let pandas = pandas.unwrap_or_default();
    let index_fields: Vec<&str> = pandas
        .index_columns
        .iter()
        .flatten()
        .map(String::as_str)
        .collect();

    // Index levels.
    let mut index_names = Vec::new();
    let mut index_levels: Vec<Vec<Label>> = Vec::new();
    for f in &index_fields {
        let i = fields
            .iter()
            .position(|x| x == f)
            .ok_or_else(|| format!("index column `{f}` not found"))?;
        index_names.push(
            pandas
                .names
                .iter()
                .find(|(field, _)| field == f)
                .and_then(|(_, n)| n.clone()),
        );
        index_levels.push(cols[i].iter().map(label).collect());
    }
    if index_levels.is_empty() {
        index_names.push(None);
        index_levels.push(
            (0..n_rows)
                .map(|r| Label::Int(i64::try_from(r).unwrap_or(0)))
                .collect(),
        );
    }
    let index: Vec<Vec<Label>> = (0..n_rows)
        .map(|r| index_levels.iter().map(|l| l[r].clone()).collect())
        .collect();

    // Data columns.
    let n_levels = pandas.column_levels.len().max(1);
    let column_names = if pandas.column_levels.is_empty() {
        vec![None]
    } else {
        pandas.column_levels.clone()
    };
    let mut columns = Vec::new();
    let mut data = Vec::new();
    for (i, f) in fields.iter().enumerate() {
        if index_fields.contains(&f.as_str()) {
            continue;
        }
        let labels = if n_levels > 1 {
            let parts = parse_tuple(f)
                .filter(|p| p.len() == n_levels)
                .ok_or_else(|| format!("column `{f}` is not a {n_levels}-level label"))?;
            parts.iter().map(|p| Label::str(p)).collect()
        } else {
            vec![Label::str(f)]
        };
        columns.push(labels);
        data.push(column_data(&cols[i]));
    }
    Ok(Frame {
        index_names,
        index,
        column_names,
        columns,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tuples() {
        assert_eq!(
            parse_tuple("('a', \"b's\", 'c\\'d')").unwrap(),
            vec!["a", "b's", "c'd"]
        );
        assert_eq!(parse_tuple("('a',)").unwrap(), vec!["a"]);
        assert_eq!(parse_tuple("(1, 'x')").unwrap(), vec!["1", "x"]);
        assert!(parse_tuple("a").is_none());
    }
}
