//! Attribute values and Calliope's NetCDF attribute (de)serialisation
//! (`calliope.io._serialise` / `_deserialise`).
//!
//! NetCDF attributes cannot hold dicts, bools, `None` or sets, and a one-element list comes
//! back as a plain string. Calliope stores dicts as YAML strings, bools as ints, `None` as
//! `"None"`, sets as lists, and records the affected keys in the attributes
//! `serialised_{single_element_list,dicts,bools,nones,sets}` so they can be restored.

use calliope_yaml::{AttrTree, read_yaml_str, to_yaml};
use indexmap::IndexMap;
use netcdf::AttributeValue;

use crate::IoError;

/// An attribute value as Calliope uses it.
#[derive(Debug, Clone)]
pub enum AttrValue {
    /// A string.
    Str(String),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// A bool (stored as int).
    Bool(bool),
    /// `None` (stored as `"None"`).
    None,
    /// A list of strings.
    List(Vec<String>),
    /// A set of strings (stored as a list).
    Set(Vec<String>),
    /// A dict (stored as a YAML string).
    Dict(AttrTree),
    /// A numeric array (as numpy arrays come back from NetCDF; an empty list is stored as
    /// an empty float array by netCDF4-python).
    Floats(Vec<f64>),
    /// An integer array.
    Ints(Vec<i64>),
}

impl PartialEq for AttrValue {
    fn eq(&self, other: &Self) -> bool {
        use AttrValue as A;
        match (self, other) {
            (A::Str(a), A::Str(b)) => a == b,
            (A::Int(a), A::Int(b)) => a == b,
            (A::Float(a), A::Float(b)) => a == b || (a.is_nan() && b.is_nan()),
            (A::Bool(a), A::Bool(b)) => a == b,
            (A::None, A::None) => true,
            (A::List(a), A::List(b)) => a == b,
            (A::Set(a), A::Set(b)) => {
                let (mut a, mut b) = (a.clone(), b.clone());
                a.sort();
                b.sort();
                a == b
            }
            (A::Dict(a), A::Dict(b)) => to_yaml(a) == to_yaml(b),
            (A::Floats(a), A::Floats(b)) => a == b,
            (A::Ints(a), A::Ints(b)) => a == b,
            _ => false,
        }
    }
}

/// Ordered attributes.
pub type Attrs = IndexMap<String, AttrValue>;

const LISTS: [&str; 5] = [
    "serialised_single_element_list",
    "serialised_dicts",
    "serialised_bools",
    "serialised_nones",
    "serialised_sets",
];

fn str_list(v: Vec<String>) -> AttributeValue {
    if v.is_empty() {
        // netCDF4-python stores `[]` as an empty float64 array
        AttributeValue::Doubles(Vec::new())
    } else if v.len() == 1 {
        AttributeValue::Str(v.into_iter().next().unwrap_or_default())
    } else {
        AttributeValue::Strs(v)
    }
}

/// `_serialise`: attribute values ready to be written, in Python's key order (original
/// keys, then the five `serialised_*` lists).
pub fn serialise(attrs: &Attrs) -> Vec<(String, AttributeValue)> {
    let keys_where = |f: &dyn Fn(&AttrValue) -> bool| -> Vec<String> {
        attrs
            .iter()
            .filter(|(_, v)| f(v))
            .map(|(k, _)| k.clone())
            .collect()
    };
    let mut single = keys_where(&|v| matches!(v, AttrValue::List(l) if l.len() == 1));
    let dicts = keys_where(&|v| matches!(v, AttrValue::Dict(_)));
    let bools = keys_where(&|v| matches!(v, AttrValue::Bool(_)));
    let nones = keys_where(&|v| matches!(v, AttrValue::None));
    let sets = keys_where(&|v| matches!(v, AttrValue::Set(_)));
    single.extend(keys_where(
        &|v| matches!(v, AttrValue::Set(s) if s.len() == 1),
    ));
    let mut out: Vec<(String, AttributeValue)> = attrs
        .iter()
        .filter(|(k, _)| !LISTS.contains(&k.as_str()))
        .map(|(k, v)| {
            let a = match v {
                AttrValue::Str(s) => AttributeValue::Str(s.clone()),
                AttrValue::Int(i) => AttributeValue::Longlong(*i),
                AttrValue::Float(f) => AttributeValue::Double(*f),
                AttrValue::Bool(b) => AttributeValue::Longlong(i64::from(*b)),
                AttrValue::None => AttributeValue::Str("None".into()),
                AttrValue::List(l) | AttrValue::Set(l) => str_list(l.clone()),
                AttrValue::Dict(d) => AttributeValue::Str(to_yaml(d)),
                AttrValue::Floats(f) => AttributeValue::Doubles(f.clone()),
                AttrValue::Ints(i) => AttributeValue::Longlongs(i.clone()),
            };
            (k.clone(), a)
        })
        .collect();
    for (name, list) in LISTS.iter().zip([single, dicts, bools, nones, sets]) {
        out.push(((*name).to_string(), str_list(list)));
    }
    out
}

fn raw_to_value(a: AttributeValue) -> AttrValue {
    use AttributeValue as R;
    let ints = |v: Vec<i64>| {
        if v.len() == 1 {
            AttrValue::Int(v[0])
        } else {
            AttrValue::Ints(v)
        }
    };
    let floats = |v: Vec<f64>| {
        if v.len() == 1 {
            AttrValue::Float(v[0])
        } else {
            AttrValue::Floats(v)
        }
    };
    match a {
        R::Str(s) => AttrValue::Str(s),
        R::Strs(v) => AttrValue::List(v),
        R::Uchar(x) => AttrValue::Int(x.into()),
        R::Schar(x) => AttrValue::Int(x.into()),
        R::Ushort(x) => AttrValue::Int(x.into()),
        R::Short(x) => AttrValue::Int(x.into()),
        R::Uint(x) => AttrValue::Int(x.into()),
        R::Int(x) => AttrValue::Int(x.into()),
        R::Longlong(x) => AttrValue::Int(x),
        R::Ulonglong(x) => AttrValue::Int(i64::try_from(x).unwrap_or(i64::MAX)),
        R::Uchars(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Schars(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Ushorts(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Shorts(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Uints(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Ints(v) => ints(v.into_iter().map(i64::from).collect()),
        R::Longlongs(v) => ints(v),
        R::Ulonglongs(v) => ints(
            v.into_iter()
                .map(|x| i64::try_from(x).unwrap_or(i64::MAX))
                .collect(),
        ),
        R::Float(x) => AttrValue::Float(x.into()),
        R::Double(x) => AttrValue::Float(x),
        R::Floats(v) => floats(v.into_iter().map(f64::from).collect()),
        R::Doubles(v) => floats(v),
    }
}

fn as_list(v: Option<AttrValue>) -> Vec<String> {
    match v {
        Some(AttrValue::Str(s)) => vec![s],
        Some(AttrValue::List(l) | AttrValue::Set(l)) => l,
        _ => Vec::new(),
    }
}

/// `_deserialise`: restore Calliope values from raw NetCDF attributes.
pub fn deserialise(raw: Vec<(String, AttributeValue)>) -> Result<Attrs, IoError> {
    let mut attrs: Attrs = raw.into_iter().map(|(k, v)| (k, raw_to_value(v))).collect();
    let mut lists: Vec<Vec<String>> = LISTS
        .iter()
        .map(|k| as_list(attrs.shift_remove(*k)))
        .collect();
    let sets = lists.pop().unwrap_or_default();
    let nones = lists.pop().unwrap_or_default();
    let bools = lists.pop().unwrap_or_default();
    let dicts = lists.pop().unwrap_or_default();
    let single = lists.pop().unwrap_or_default();
    for k in dicts {
        if let Some(v) = attrs.get_mut(&k) {
            let AttrValue::Str(text) = v else {
                return Err(IoError::Format(format!(
                    "serialised dict `{k}` is not a string"
                )));
            };
            let tree = read_yaml_str(text, false)
                .map_err(|e| IoError::Format(format!("attribute `{k}`: {e}")))?;
            *v = AttrValue::Dict(tree);
        }
    }
    for k in bools {
        if let Some(v) = attrs.get_mut(&k) {
            *v = AttrValue::Bool(match v {
                AttrValue::Int(i) => *i != 0,
                AttrValue::Float(f) => *f != 0.0,
                _ => true,
            });
        }
    }
    for k in nones {
        if let Some(v) = attrs.get_mut(&k) {
            *v = AttrValue::None;
        }
    }
    for k in single {
        if let Some(v) = attrs.get_mut(&k)
            && let AttrValue::Str(s) = v
        {
            *v = AttrValue::List(vec![std::mem::take(s)]);
        }
    }
    for k in sets {
        if let Some(v) = attrs.get_mut(&k) {
            *v = AttrValue::Set(match std::mem::replace(v, AttrValue::None) {
                AttrValue::List(l) | AttrValue::Set(l) => l,
                AttrValue::Str(s) => vec![s],
                _ => Vec::new(),
            });
        }
    }
    Ok(attrs)
}

/// Attributes of a NetCDF group, deserialised.
pub fn group_attrs(g: &netcdf::Group<'_>) -> Result<Attrs, IoError> {
    let mut raw = Vec::new();
    for a in g.attributes() {
        raw.push((a.name().to_owned(), a.value()?));
    }
    deserialise(raw)
}

/// Attributes of a NetCDF variable, deserialised; `_FillValue`, `units`, `calendar` (CF
/// encoding) are dropped as xarray does when decoding.
pub fn var_attrs(v: &netcdf::Variable<'_>) -> Result<Attrs, IoError> {
    let mut raw = Vec::new();
    for a in v.attributes() {
        let name = a.name().to_owned();
        if matches!(
            name.as_str(),
            "_FillValue" | "units" | "calendar" | "dtype" | "_Netcdf4Dimid"
        ) {
            continue;
        }
        raw.push((name, a.value()?));
    }
    deserialise(raw)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut d = AttrTree::new();
        d.insert("a", calliope_yaml::Node::new(calliope_yaml::Value::Int(1)));
        let attrs: Attrs = [
            ("s", AttrValue::Str("x".into())),
            ("b", AttrValue::Bool(true)),
            ("n", AttrValue::None),
            ("one", AttrValue::List(vec!["x".into()])),
            ("two", AttrValue::List(vec!["x".into(), "y".into()])),
            ("set", AttrValue::Set(vec!["q".into()])),
            ("d", AttrValue::Dict(d)),
            ("f", AttrValue::Float(1.5)),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect();
        let raw = serialise(&attrs);
        assert_eq!(raw.len(), attrs.len() + 5);
        assert!(matches!(raw[1].1, AttributeValue::Longlong(1)));
        let back = deserialise(raw).unwrap();
        assert_eq!(back, attrs);
    }
}
