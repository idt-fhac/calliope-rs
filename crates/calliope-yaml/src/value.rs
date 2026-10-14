//! YAML values with source locations.

use std::fmt;
use std::sync::Arc;

use crate::tree::AttrTree;

/// Where a YAML document came from: a file path or a description such as `<yaml string>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Source {
    name: Arc<str>,
}

impl Source {
    /// A source named after a file path or a description.
    pub fn new(name: impl Into<Arc<str>>) -> Self {
        Self { name: name.into() }
    }

    /// The name of the source (a file path, or `<yaml string>`).
    pub fn name(&self) -> &str {
        &self.name
    }
}

/// A position in a YAML source. Lines and columns are 1-based.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Span {
    /// The document the node was read from.
    pub source: Source,
    /// 1-based line.
    pub line: usize,
    /// 1-based column.
    pub col: usize,
}

impl fmt::Display for Span {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}:{}", self.source.name(), self.line, self.col)
    }
}

/// A YAML value. Mappings are [`AttrTree`]s, so every mapping in a loaded document follows
/// `AttrDict` rules (string keys, dotted keys expanded).
#[derive(Debug, Clone)]
pub enum Value {
    /// `null`, `~` or an empty scalar.
    Null,
    /// `true`/`false` (YAML 1.2 spellings only; `yes`/`on` are strings).
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float, including `.inf` and `.nan`.
    Float(f64),
    /// A string.
    Str(String),
    /// An unquoted `YYYY-MM-DD` scalar. ruamel loads these as `datetime.date`.
    Date(String),
    /// An unquoted YAML 1.1 timestamp with a time part. ruamel loads these as `datetime`.
    DateTime(String),
    /// A sequence.
    Seq(Vec<Node>),
    /// A mapping.
    Map(AttrTree),
}

/// A value plus the location it was read from (if any).
#[derive(Debug, Clone)]
pub struct Node {
    /// The value.
    pub value: Value,
    /// Where the value starts in its source; `None` for values built in code.
    pub span: Option<Span>,
}

impl Node {
    /// A node without a source location.
    pub fn new(value: Value) -> Self {
        Self { value, span: None }
    }

    /// A node with a source location.
    pub fn with_span(value: Value, span: Option<Span>) -> Self {
        Self { value, span }
    }

    /// An empty mapping.
    pub fn empty_map() -> Self {
        Self::new(Value::Map(AttrTree::new()))
    }

    /// The mapping held by this node, if it is one.
    pub fn as_map(&self) -> Option<&AttrTree> {
        match &self.value {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    /// The mutable mapping held by this node, if it is one.
    pub fn as_map_mut(&mut self) -> Option<&mut AttrTree> {
        match &mut self.value {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }

    /// The string held by this node, if it is one.
    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The sequence held by this node, if it is one.
    pub fn as_seq(&self) -> Option<&[Node]> {
        match &self.value {
            Value::Seq(s) => Some(s),
            _ => None,
        }
    }

    /// True for `null`.
    pub fn is_null(&self) -> bool {
        matches!(self.value, Value::Null)
    }

    /// True for a mapping.
    pub fn is_map(&self) -> bool {
        matches!(self.value, Value::Map(_))
    }

    /// Python truthiness: `None`, `False`, `0`, `0.0`, `""` and empty containers are falsy.
    pub fn is_truthy(&self) -> bool {
        match &self.value {
            Value::Null => false,
            Value::Bool(b) => *b,
            Value::Int(i) => *i != 0,
            Value::Float(f) => *f != 0.0,
            Value::Str(s) => !s.is_empty(),
            Value::Date(_) | Value::DateTime(_) => true,
            Value::Seq(s) => !s.is_empty(),
            Value::Map(m) => !m.is_empty(),
        }
    }

    /// The source location as text, or `<unknown location>`.
    pub fn location(&self) -> String {
        self.span
            .as_ref()
            .map_or_else(|| "<unknown location>".to_owned(), ToString::to_string)
    }
}

impl From<Value> for Node {
    fn from(value: Value) -> Self {
        Node::new(value)
    }
}

macro_rules! impl_from {
    ($t:ty, $v:ident, $conv:expr) => {
        impl From<$t> for Value {
            fn from(x: $t) -> Self {
                #[allow(clippy::redundant_closure_call)]
                Value::$v(($conv)(x))
            }
        }
        impl From<$t> for Node {
            fn from(x: $t) -> Self {
                Node::new(Value::from(x))
            }
        }
    };
}
impl_from!(bool, Bool, |x| x);
impl_from!(i64, Int, |x| x);
impl_from!(i32, Int, i64::from);
impl_from!(f64, Float, |x| x);
impl_from!(&str, Str, str::to_owned);
impl_from!(String, Str, |x| x);
impl_from!(AttrTree, Map, |x| x);
impl_from!(Vec<Node>, Seq, |x| x);

/// Python equality: `1 == 1.0 == True`, `nan != nan`, mappings compare by content
/// (ignoring order) and locations are ignored.
impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        use Value::*;
        match (self, other) {
            (Null, Null) => true,
            (Str(a), Str(b)) => a == b,
            (Date(a), Date(b)) | (DateTime(a), DateTime(b)) => a == b,
            (Seq(a), Seq(b)) => a == b,
            (Map(a), Map(b)) => a == b,
            (a, b) => match (a.as_number(), b.as_number()) {
                (Some(x), Some(y)) => x == y,
                _ => false,
            },
        }
    }
}

impl PartialEq for Node {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl Value {
    /// The numeric value of a bool, int or float (Python numeric tower).
    pub fn as_number(&self) -> Option<f64> {
        match self {
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            #[allow(clippy::cast_precision_loss)]
            Value::Int(i) => Some(*i as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    /// A short name of the value's type, for error messages.
    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Null => "null",
            Value::Bool(_) => "bool",
            Value::Int(_) => "int",
            Value::Float(_) => "float",
            Value::Str(_) => "string",
            Value::Date(_) => "date",
            Value::DateTime(_) => "datetime",
            Value::Seq(_) => "list",
            Value::Map(_) => "mapping",
        }
    }
}

/// Formats a float the way Python's `str(float)` does for common values.
pub fn python_float(f: f64) -> String {
    if f.is_nan() {
        "nan".to_owned()
    } else if f.is_infinite() {
        if f > 0.0 { "inf" } else { "-inf" }.to_owned()
    } else {
        let sci = format!("{f:e}");
        let (mantissa, exp) = sci.split_once('e').unwrap_or((&sci, "0"));
        let exp: i32 = exp.parse().unwrap_or(0);
        if (-4..16).contains(&exp) {
            let plain = format!("{f}");
            if plain.contains('.') {
                plain
            } else {
                format!("{plain}.0")
            }
        } else {
            let sign = if exp < 0 { '-' } else { '+' };
            format!("{mantissa}e{sign}{:02}", exp.abs())
        }
    }
}

/// Python `str()` formatting, used in log and error messages that mirror Calliope's.
impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Null => f.write_str("None"),
            Value::Bool(true) => f.write_str("True"),
            Value::Bool(false) => f.write_str("False"),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => f.write_str(&python_float(*x)),
            Value::Str(s) | Value::Date(s) | Value::DateTime(s) => f.write_str(s),
            Value::Seq(items) => {
                f.write_str("[")?;
                for (i, item) in items.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    item.value.fmt_repr(f)?;
                }
                f.write_str("]")
            }
            Value::Map(m) => {
                f.write_str("{")?;
                for (i, (k, v)) in m.iter().enumerate() {
                    if i > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "'{k}': ")?;
                    v.value.fmt_repr(f)?;
                }
                f.write_str("}")
            }
        }
    }
}

impl Value {
    fn fmt_repr(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Str(s) => write!(f, "'{s}'"),
            Value::Date(s) => write!(f, "datetime.date({s})"),
            Value::DateTime(s) => write!(f, "datetime.datetime({s})"),
            other => fmt::Display::fmt(other, f),
        }
    }
}

impl fmt::Display for Node {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.value, f)
    }
}
