//! `serde` support: deserialising typed values from [`Node`]s, with the path and source
//! location of the first error.
//!
//! Scalar coercion follows pydantic's lax mode where a YAML document can trigger it: a float
//! field accepts an int or bool, an int field accepts an integral float or a bool, a bool field
//! accepts `0`/`1`. Strings are never coerced to or from numbers here; schema types decide that.

use std::fmt;

use serde::de::{
    self, DeserializeOwned, DeserializeSeed, Deserializer, IntoDeserializer, MapAccess, SeqAccess,
    Unexpected, Visitor,
};

use crate::tree::AttrTree;
use crate::value::{Node, Span, Value};

/// A deserialisation error message (without location).
#[derive(Debug, Clone, PartialEq)]
pub struct DeError(pub String);

impl fmt::Display for DeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for DeError {}

impl de::Error for DeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        DeError(msg.to_string())
    }
}

/// A deserialisation error with the key path and source location of the failing value.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{}{message}{}", if path.is_empty() { String::new() } else { format!("{path}: ") }, span.as_ref().map_or_else(String::new, |s| format!(" (at {s})")))]
pub struct PathError {
    /// Dotted path to the failing value (`techs.ccgt.base_tech`, `equations.0.where`).
    pub path: String,
    /// The error message.
    pub message: String,
    /// Location of the failing value (or of its closest parent with a location).
    pub span: Option<Span>,
}

/// Deserialises `T` from a node, reporting the path and location of the first error.
pub fn from_node<T: DeserializeOwned>(node: &Node) -> Result<T, PathError> {
    serde_path_to_error::deserialize(node).map_err(|e| {
        let path = path_string(e.path());
        let span = span_at(node, &path);
        PathError {
            path,
            message: e.into_inner().0,
            span,
        }
    })
}

/// Deserialises `T` from a mapping.
pub fn from_tree<T: DeserializeOwned>(tree: &AttrTree) -> Result<T, PathError> {
    from_node(&Node::new(Value::Map(tree.clone())))
}

fn path_string(path: &serde_path_to_error::Path) -> String {
    use serde_path_to_error::Segment;
    let parts: Vec<String> = path
        .iter()
        .filter_map(|seg| match seg {
            Segment::Seq { index } => Some(index.to_string()),
            Segment::Map { key } => Some(key.clone()),
            Segment::Enum { variant } => Some(variant.clone()),
            Segment::Unknown => None,
        })
        .collect();
    parts.join(".")
}

/// The location of the value at a dotted path (sequence indices as numbers), or of the
/// deepest parent that has one.
pub fn span_at(root: &Node, path: &str) -> Option<Span> {
    let mut node = root;
    let mut best = root.span.clone();
    if path.is_empty() {
        return best;
    }
    for part in path.split('.') {
        let next = match &node.value {
            Value::Map(m) => m.get(part),
            Value::Seq(items) => part.parse::<usize>().ok().and_then(|i| items.get(i)),
            _ => None,
        };
        match next {
            Some(n) => {
                node = n;
                if n.span.is_some() {
                    best = n.span.clone();
                }
            }
            None => break,
        }
    }
    best
}

fn unexpected(value: &Value) -> Unexpected<'_> {
    match value {
        Value::Null => Unexpected::Unit,
        Value::Bool(b) => Unexpected::Bool(*b),
        Value::Int(i) => Unexpected::Signed(*i),
        Value::Float(f) => Unexpected::Float(*f),
        Value::Str(s) => Unexpected::Str(s),
        Value::Date(_) => Unexpected::Other("date"),
        Value::DateTime(_) => Unexpected::Other("datetime"),
        Value::Seq(_) => Unexpected::Seq,
        Value::Map(_) => Unexpected::Map,
    }
}

impl<'de> Deserializer<'de> for &'de Node {
    type Error = DeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Null => visitor.visit_unit(),
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Int(i) => visitor.visit_i64(*i),
            Value::Float(f) => visitor.visit_f64(*f),
            Value::Str(s) => visitor.visit_borrowed_str(s),
            Value::Date(_) | Value::DateTime(_) => {
                Err(de::Error::invalid_type(unexpected(&self.value), &visitor))
            }
            Value::Seq(items) => visitor.visit_seq(SeqDe { iter: items.iter() }),
            Value::Map(m) => visitor.visit_map(MapDe {
                iter: m.iter(),
                value: None,
            }),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Bool(b) => visitor.visit_bool(*b),
            Value::Int(0) => visitor.visit_bool(false),
            Value::Int(1) => visitor.visit_bool(true),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Int(i) => visitor.visit_i64(*i),
            Value::Bool(b) => visitor.visit_i64(i64::from(*b)),
            #[allow(clippy::cast_possible_truncation)]
            Value::Float(f) if f.is_finite() && f.fract() == 0.0 => visitor.visit_i64(*f as i64),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match self.value.as_number() {
            Some(f) => visitor.visit_f64(f),
            None => Err(de::Error::invalid_type(unexpected(&self.value), &visitor)),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Str(s) => visitor.visit_borrowed_str(s),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Null => visitor.visit_none(),
            _ => visitor.visit_some(self),
        }
    }

    fn deserialize_unit<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Null => visitor.visit_unit(),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Seq(items) => visitor.visit_seq(SeqDe { iter: items.iter() }),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Map(m) => visitor.visit_map(MapDe {
                iter: m.iter(),
                value: None,
            }),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        self.deserialize_map(visitor)
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, DeError> {
        match &self.value {
            Value::Str(s) => visitor.visit_enum(s.as_str().into_deserializer()),
            other => Err(de::Error::invalid_type(unexpected(other), &visitor)),
        }
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, DeError> {
        visitor.visit_newtype_struct(self)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, DeError> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        i8 i16 i32 i128 u8 u16 u32 u64 u128 f32 char bytes byte_buf unit_struct tuple
        tuple_struct identifier
    }
}

struct SeqDe<'a> {
    iter: std::slice::Iter<'a, Node>,
}

impl<'de> SeqAccess<'de> for SeqDe<'de> {
    type Error = DeError;
    fn next_element_seed<T: DeserializeSeed<'de>>(
        &mut self,
        seed: T,
    ) -> Result<Option<T::Value>, DeError> {
        self.iter.next().map(|n| seed.deserialize(n)).transpose()
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

struct MapDe<'a> {
    iter: indexmap::map::Iter<'a, String, Node>,
    value: Option<&'a Node>,
}

impl<'de> MapAccess<'de> for MapDe<'de> {
    type Error = DeError;
    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, DeError> {
        match self.iter.next() {
            Some((k, v)) => {
                self.value = Some(v);
                let key: de::value::BorrowedStrDeserializer<'de, DeError> =
                    de::value::BorrowedStrDeserializer::new(k);
                seed.deserialize(key).map(Some)
            }
            None => Ok(None),
        }
    }
    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, DeError> {
        let v = self
            .value
            .take()
            .ok_or_else(|| DeError("value requested before key".to_owned()))?;
        seed.deserialize(v)
    }
    fn size_hint(&self) -> Option<usize> {
        Some(self.iter.len())
    }
}

/// Deserialises a [`Node`] from any `serde` data (`serde_value`-style), so that untyped
/// fields (`solver_options`, `dict` extras) keep their YAML structure.
impl<'de> de::Deserialize<'de> for Node {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct NodeVisitor;
        impl<'de> Visitor<'de> for NodeVisitor {
            type Value = Node;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("any YAML value")
            }
            fn visit_unit<E>(self) -> Result<Node, E> {
                Ok(Node::new(Value::Null))
            }
            fn visit_none<E>(self) -> Result<Node, E> {
                Ok(Node::new(Value::Null))
            }
            fn visit_some<D: Deserializer<'de>>(self, d: D) -> Result<Node, D::Error> {
                de::Deserialize::deserialize(d)
            }
            fn visit_bool<E>(self, v: bool) -> Result<Node, E> {
                Ok(Node::new(Value::Bool(v)))
            }
            fn visit_i64<E>(self, v: i64) -> Result<Node, E> {
                Ok(Node::new(Value::Int(v)))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Node, E> {
                i64::try_from(v)
                    .map(|i| Node::new(Value::Int(i)))
                    .map_err(|_| E::custom("integer out of range"))
            }
            fn visit_f64<E>(self, v: f64) -> Result<Node, E> {
                Ok(Node::new(Value::Float(v)))
            }
            fn visit_str<E>(self, v: &str) -> Result<Node, E> {
                Ok(Node::new(Value::Str(v.to_owned())))
            }
            fn visit_string<E>(self, v: String) -> Result<Node, E> {
                Ok(Node::new(Value::Str(v)))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Node, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Node::new(Value::Seq(items)))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Node, A::Error> {
                let mut tree = AttrTree::new();
                while let Some((k, v)) = map.next_entry::<String, Node>()? {
                    tree.insert(k, v);
                }
                Ok(Node::new(Value::Map(tree)))
            }
        }
        deserializer.deserialize_any(NodeVisitor)
    }
}

impl serde::Serialize for Node {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{SerializeMap, SerializeSeq};
        match &self.value {
            Value::Null => s.serialize_none(),
            Value::Bool(b) => s.serialize_bool(*b),
            Value::Int(i) => s.serialize_i64(*i),
            Value::Float(f) => s.serialize_f64(*f),
            Value::Str(v) | Value::Date(v) | Value::DateTime(v) => s.serialize_str(v),
            Value::Seq(items) => {
                let mut seq = s.serialize_seq(Some(items.len()))?;
                for item in items {
                    seq.serialize_element(item)?;
                }
                seq.end()
            }
            Value::Map(m) => {
                let mut map = s.serialize_map(Some(m.len()))?;
                for (k, v) in m {
                    map.serialize_entry(k, v)?;
                }
                map.end()
            }
        }
    }
}
