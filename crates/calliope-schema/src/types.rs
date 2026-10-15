//! Constrained value types from `schemas/general.py` and the value unions used by the schemas.

use std::borrow::{Borrow, Cow};
use std::fmt;
use std::ops::Deref;
use std::sync::LazyLock;

use calliope_yaml::{Node, Value};
use regex::Regex;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

/// The `AttrStr` pattern: a single snake_case word not starting with `_` or a digit.
pub const ATTR_STR_PATTERN: &str = r"^[^_^\d][\w]*$";

static ATTR_STR_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(ATTR_STR_PATTERN).expect("valid regex"));

/// Python-style equality, used for uniqueness checks (`1 == 1.0 == True`, `nan != nan`).
pub trait PyEq {
    /// True if Python would consider the two values equal.
    fn py_eq(&self, other: &Self) -> bool;
}

impl PyEq for String {
    fn py_eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl PyEq for Node {
    fn py_eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<T: PyEq> PyEq for Vec<T> {
    fn py_eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.py_eq(b))
    }
}

/// `AttrStr`: a single word in snake_case (e.g. `wind_offshore`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(transparent)]
pub struct AttrStr(String);

impl AttrStr {
    /// Validates a string.
    pub fn new(s: impl Into<String>) -> Result<Self, String> {
        let s = s.into();
        if ATTR_STR_RE.is_match(&s) {
            Ok(Self(s))
        } else {
            Err(format!(
                "String should match pattern '{ATTR_STR_PATTERN}', found `{s}`"
            ))
        }
    }

    /// The string.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for AttrStr {
    type Target = str;
    fn deref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for AttrStr {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AttrStr {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl PyEq for AttrStr {
    fn py_eq(&self, other: &Self) -> bool {
        self == other
    }
}

impl<'de> Deserialize<'de> for AttrStr {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        AttrStr::new(s).map_err(de::Error::custom)
    }
}

impl JsonSchema for AttrStr {
    fn schema_name() -> Cow<'static, str> {
        "AttrStr".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"type": "string", "pattern": ATTR_STR_PATTERN})
    }
}

fn check_unique<T: PyEq>(items: &[T]) -> Result<(), String> {
    for (i, a) in items.iter().enumerate() {
        if items[..i].iter().any(|b| a.py_eq(b)) {
            return Err("List must be unique".to_owned());
        }
    }
    Ok(())
}

macro_rules! list_newtype {
    ($(#[$m:meta])* $name:ident, unique = $unique:expr, min = $min:expr) => {
        $(#[$m])*
        #[derive(Debug, Clone, PartialEq, Serialize)]
        #[serde(transparent)]
        pub struct $name<T>(pub Vec<T>);

        impl<T> Default for $name<T> {
            fn default() -> Self {
                Self(Vec::new())
            }
        }

        impl<T> Deref for $name<T> {
            type Target = Vec<T>;
            fn deref(&self) -> &Vec<T> {
                &self.0
            }
        }

        impl<'de, T: Deserialize<'de> + PyEq> Deserialize<'de> for $name<T> {
            fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
                let items = Vec::<T>::deserialize(d)?;
                let min: usize = $min;
                if items.len() < min {
                    return Err(de::Error::custom(format!(
                        "List should have at least {} item after validation, not {}",
                        min,
                        items.len()
                    )));
                }
                if $unique {
                    check_unique(&items).map_err(de::Error::custom)?;
                }
                Ok(Self(items))
            }
        }

        impl<T: PyEq> PyEq for $name<T> {
            fn py_eq(&self, other: &Self) -> bool {
                self.0.py_eq(&other.0)
            }
        }

        impl<T: JsonSchema> JsonSchema for $name<T> {
            fn schema_name() -> Cow<'static, str> {
                format!("{}_{}", stringify!($name), T::schema_name()).into()
            }
            fn inline_schema() -> bool {
                true
            }
            fn json_schema(g: &mut SchemaGenerator) -> Schema {
                let mut s = json_schema!({"type": "array", "items": g.subschema_for::<T>()});
                if $unique {
                    s.insert("uniqueItems".into(), true.into());
                }
                let min: usize = $min;
                if min > 0 {
                    s.insert("minItems".into(), min.into());
                }
                s
            }
        }
    };
}

list_newtype!(
    /// `UniqueList`: a list with no repeated values (Python equality).
    UniqueList, unique = true, min = 0usize
);
list_newtype!(
    /// `NonEmptyList`: a list with at least one value.
    NonEmptyList, unique = false, min = 1usize
);
list_newtype!(
    /// `NonEmptyUniqueList`: a non-empty list with no repeated values.
    NonEmptyUniqueList, unique = true, min = 1usize
);

/// `NumericVal`: an integer or a float (`nan` and `±inf` allowed).
///
/// Equality treats `nan` as equal to itself, so that schema values compare as "the same
/// configuration"; it is also Python-numeric (`Int(1) == Float(1.0)`).
#[derive(Debug, Clone, Copy)]
pub enum Numeric {
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
}

impl Numeric {
    /// The value as a float.
    #[allow(clippy::cast_precision_loss)]
    pub fn as_f64(self) -> f64 {
        match self {
            Numeric::Int(i) => i as f64,
            Numeric::Float(f) => f,
        }
    }

    /// True for the `Float` variant (Python `type(x) is float`).
    pub fn is_float(self) -> bool {
        matches!(self, Numeric::Float(_))
    }
}

impl PartialEq for Numeric {
    fn eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_f64(), other.as_f64());
        a == b || (a.is_nan() && b.is_nan())
    }
}

impl PyEq for Numeric {
    fn py_eq(&self, other: &Self) -> bool {
        let (a, b) = (self.as_f64(), other.as_f64());
        a == b
    }
}

impl fmt::Display for Numeric {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Numeric::Int(i) => write!(f, "{i}"),
            Numeric::Float(x) => f.write_str(&calliope_yaml::python_float(*x)),
        }
    }
}

impl From<f64> for Numeric {
    fn from(f: f64) -> Self {
        Numeric::Float(f)
    }
}

impl From<i64> for Numeric {
    fn from(i: i64) -> Self {
        Numeric::Int(i)
    }
}

impl Serialize for Numeric {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            Numeric::Int(i) => s.serialize_i64(*i),
            Numeric::Float(f) => s.serialize_f64(*f),
        }
    }
}

fn numeric_from_value(v: &Value) -> Option<Numeric> {
    match v {
        Value::Int(i) => Some(Numeric::Int(*i)),
        Value::Float(f) => Some(Numeric::Float(*f)),
        // pydantic lax mode: bools are ints, numeric strings are parsed.
        Value::Bool(b) => Some(Numeric::Int(i64::from(*b))),
        Value::Str(s) => s
            .trim()
            .parse::<i64>()
            .map(Numeric::Int)
            .ok()
            .or_else(|| s.trim().parse::<f64>().ok().map(Numeric::Float)),
        _ => None,
    }
}

impl<'de> Deserialize<'de> for Numeric {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        numeric_from_value(&node.value).ok_or_else(|| {
            de::Error::custom(format!(
                "Input should be a valid number, got {} `{}`",
                node.value.type_name(),
                node.value
            ))
        })
    }
}

impl JsonSchema for Numeric {
    fn schema_name() -> Cow<'static, str> {
        "NumericVal".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"anyOf": [{"type": "integer"}, {"type": "number"}]})
    }
}

/// `DataValue = str | bool | NumericVal | None`: a scalar data value, typed as loaded.
#[derive(Debug, Clone, PartialEq)]
pub enum DataValue {
    /// `null`.
    Null,
    /// A bool.
    Bool(bool),
    /// A number.
    Number(Numeric),
    /// A string.
    Str(String),
}

impl PyEq for DataValue {
    fn py_eq(&self, other: &Self) -> bool {
        use DataValue::*;
        match (self, other) {
            (Null, Null) => true,
            (Str(a), Str(b)) => a == b,
            (Bool(a), Bool(b)) => a == b,
            (Bool(a), Number(n)) | (Number(n), Bool(a)) => n.as_f64() == f64::from(u8::from(*a)),
            (Number(a), Number(b)) => a.py_eq(b),
            _ => false,
        }
    }
}

impl DataValue {
    pub(crate) fn from_value(v: &Value) -> Result<Self, String> {
        match v {
            Value::Null => Ok(DataValue::Null),
            Value::Bool(b) => Ok(DataValue::Bool(*b)),
            Value::Int(i) => Ok(DataValue::Number(Numeric::Int(*i))),
            Value::Float(f) => Ok(DataValue::Number(Numeric::Float(*f))),
            Value::Str(s) => Ok(DataValue::Str(s.clone())),
            other => Err(format!(
                "Input should be a string, bool, number or null, got {} `{other}`",
                other.type_name()
            )),
        }
    }
}

impl Serialize for DataValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DataValue::Null => s.serialize_none(),
            DataValue::Bool(b) => s.serialize_bool(*b),
            DataValue::Number(n) => n.serialize(s),
            DataValue::Str(v) => s.serialize_str(v),
        }
    }
}

impl<'de> Deserialize<'de> for DataValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        DataValue::from_value(&node.value).map_err(de::Error::custom)
    }
}

impl JsonSchema for DataValue {
    fn schema_name() -> Cow<'static, str> {
        "DataValue".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"anyOf": [
            {"type": "string"}, {"type": "boolean"}, {"type": "integer"}, {"type": "number"},
            {"type": "null"}
        ]})
    }
}

/// `IndexValue = str | NumericVal`: an index item.
#[derive(Debug, Clone, PartialEq)]
pub enum IndexValue {
    /// A string.
    Str(String),
    /// A number.
    Number(Numeric),
}

impl PyEq for IndexValue {
    fn py_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (IndexValue::Str(a), IndexValue::Str(b)) => a == b,
            (IndexValue::Number(a), IndexValue::Number(b)) => a.py_eq(b),
            _ => false,
        }
    }
}

impl Serialize for IndexValue {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            IndexValue::Str(v) => s.serialize_str(v),
            IndexValue::Number(n) => n.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for IndexValue {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        match &node.value {
            Value::Str(s) => Ok(IndexValue::Str(s.clone())),
            Value::Int(i) => Ok(IndexValue::Number(Numeric::Int(*i))),
            Value::Float(f) => Ok(IndexValue::Number(Numeric::Float(*f))),
            Value::Bool(b) => Ok(IndexValue::Number(Numeric::Int(i64::from(*b)))),
            other => Err(de::Error::custom(format!(
                "Input should be a string or a number, got {} `{other}`",
                other.type_name()
            ))),
        }
    }
}

impl JsonSchema for IndexValue {
    fn schema_name() -> Cow<'static, str> {
        "IndexValue".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({"anyOf": [{"type": "string"}, {"type": "integer"}, {"type": "number"}]})
    }
}

/// A value given either as one item or as a list (`None | AttrStr | UniqueList[AttrStr]`).
/// The original form is kept, as pydantic does.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum OneOrMany<T> {
    /// A single item.
    One(T),
    /// A list.
    Many(UniqueList<T>),
}

impl<T: Clone> OneOrMany<T> {
    /// `listify`: the items as a list.
    pub fn to_vec(&self) -> Vec<T> {
        match self {
            OneOrMany::One(x) => vec![x.clone()],
            OneOrMany::Many(xs) => xs.0.clone(),
        }
    }
}

impl<'de, T: DeserializeOwned + PyEq> Deserialize<'de> for OneOrMany<T> {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        if node.as_seq().is_some() {
            calliope_yaml::from_node::<UniqueList<T>>(&node)
                .map(OneOrMany::Many)
                .map_err(|e| de::Error::custom(e.message))
        } else {
            calliope_yaml::from_node::<T>(&node)
                .map(OneOrMany::One)
                .map_err(|e| de::Error::custom(e.message))
        }
    }
}

/// Re-deserialises a node with the crate's YAML deserialiser, keeping the error message.
pub(crate) fn from_node_msg<T: DeserializeOwned>(node: &Node) -> Result<T, String> {
    calliope_yaml::from_node(node).map_err(|e| {
        if e.path.is_empty() {
            e.message
        } else {
            format!("{}: {}", e.path, e.message)
        }
    })
}

/// JSON-schema stand-in for untyped values (`Any`).
#[derive(Debug)]
pub struct AnyValue;

impl JsonSchema for AnyValue {
    fn schema_name() -> Cow<'static, str> {
        "AnyValue".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({})
    }
}

/// `schemars` transform that drops `default` (pydantic omits `default_factory` defaults).
pub(crate) fn without_default(schema: &mut Schema) {
    schema.remove("default");
}
