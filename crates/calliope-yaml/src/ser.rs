//! `serde` serialisation of typed values into [`Node`]s (`model_dump`).

use serde::Serialize;
use serde::ser::{self, Impossible};

use crate::de::DeError;
use crate::tree::AttrTree;
use crate::value::{Node, Value};

/// Serialises a value into a node tree. Struct fields and map entries keep their order.
pub fn to_node<T: Serialize + ?Sized>(value: &T) -> Result<Node, DeError> {
    value.serialize(NodeSerializer)
}

/// Serialises a value that must become a mapping.
pub fn to_tree<T: Serialize + ?Sized>(value: &T) -> Result<AttrTree, DeError> {
    match to_node(value)?.value {
        Value::Map(m) => Ok(m),
        other => Err(DeError(format!(
            "expected a mapping, got {}",
            other.type_name()
        ))),
    }
}

impl ser::Error for DeError {
    fn custom<T: std::fmt::Display>(msg: T) -> Self {
        DeError(msg.to_string())
    }
}

struct NodeSerializer;

fn node(v: Value) -> Result<Node, DeError> {
    Ok(Node::new(v))
}

impl ser::Serializer for NodeSerializer {
    type Ok = Node;
    type Error = DeError;
    type SerializeSeq = SeqSer;
    type SerializeTuple = SeqSer;
    type SerializeTupleStruct = SeqSer;
    type SerializeTupleVariant = Impossible<Node, DeError>;
    type SerializeMap = MapSer;
    type SerializeStruct = MapSer;
    type SerializeStructVariant = Impossible<Node, DeError>;

    fn serialize_bool(self, v: bool) -> Result<Node, DeError> {
        node(Value::Bool(v))
    }
    fn serialize_i8(self, v: i8) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_i16(self, v: i16) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_i32(self, v: i32) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_i64(self, v: i64) -> Result<Node, DeError> {
        node(Value::Int(v))
    }
    fn serialize_u8(self, v: u8) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_u16(self, v: u16) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_u32(self, v: u32) -> Result<Node, DeError> {
        node(Value::Int(v.into()))
    }
    fn serialize_u64(self, v: u64) -> Result<Node, DeError> {
        i64::try_from(v)
            .map(|i| Node::new(Value::Int(i)))
            .map_err(|_| DeError("integer out of range".to_owned()))
    }
    fn serialize_f32(self, v: f32) -> Result<Node, DeError> {
        node(Value::Float(v.into()))
    }
    fn serialize_f64(self, v: f64) -> Result<Node, DeError> {
        node(Value::Float(v))
    }
    fn serialize_char(self, v: char) -> Result<Node, DeError> {
        node(Value::Str(v.to_string()))
    }
    fn serialize_str(self, v: &str) -> Result<Node, DeError> {
        node(Value::Str(v.to_owned()))
    }
    fn serialize_bytes(self, _v: &[u8]) -> Result<Node, DeError> {
        Err(DeError("bytes cannot be represented in YAML".to_owned()))
    }
    fn serialize_none(self) -> Result<Node, DeError> {
        node(Value::Null)
    }
    fn serialize_some<T: Serialize + ?Sized>(self, value: &T) -> Result<Node, DeError> {
        value.serialize(self)
    }
    fn serialize_unit(self) -> Result<Node, DeError> {
        node(Value::Null)
    }
    fn serialize_unit_struct(self, _name: &'static str) -> Result<Node, DeError> {
        node(Value::Null)
    }
    fn serialize_unit_variant(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
    ) -> Result<Node, DeError> {
        node(Value::Str(variant.to_owned()))
    }
    fn serialize_newtype_struct<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        value: &T,
    ) -> Result<Node, DeError> {
        value.serialize(self)
    }
    fn serialize_newtype_variant<T: Serialize + ?Sized>(
        self,
        _name: &'static str,
        _index: u32,
        variant: &'static str,
        value: &T,
    ) -> Result<Node, DeError> {
        let mut tree = AttrTree::new();
        tree.insert(variant, value.serialize(NodeSerializer)?);
        node(Value::Map(tree))
    }
    fn serialize_seq(self, len: Option<usize>) -> Result<SeqSer, DeError> {
        Ok(SeqSer(Vec::with_capacity(len.unwrap_or(0))))
    }
    fn serialize_tuple(self, len: usize) -> Result<SeqSer, DeError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_struct(self, _name: &'static str, len: usize) -> Result<SeqSer, DeError> {
        self.serialize_seq(Some(len))
    }
    fn serialize_tuple_variant(
        self,
        name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeTupleVariant, DeError> {
        Err(DeError(format!(
            "tuple variants are not supported ({name})"
        )))
    }
    fn serialize_map(self, _len: Option<usize>) -> Result<MapSer, DeError> {
        Ok(MapSer {
            tree: AttrTree::new(),
            key: None,
        })
    }
    fn serialize_struct(self, _name: &'static str, _len: usize) -> Result<MapSer, DeError> {
        self.serialize_map(None)
    }
    fn serialize_struct_variant(
        self,
        name: &'static str,
        _index: u32,
        _variant: &'static str,
        _len: usize,
    ) -> Result<Self::SerializeStructVariant, DeError> {
        Err(DeError(format!(
            "struct variants are not supported ({name})"
        )))
    }
}

struct SeqSer(Vec<Node>);

impl ser::SerializeSeq for SeqSer {
    type Ok = Node;
    type Error = DeError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), DeError> {
        self.0.push(value.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, DeError> {
        node(Value::Seq(self.0))
    }
}

impl ser::SerializeTuple for SeqSer {
    type Ok = Node;
    type Error = DeError;
    fn serialize_element<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), DeError> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Node, DeError> {
        ser::SerializeSeq::end(self)
    }
}

impl ser::SerializeTupleStruct for SeqSer {
    type Ok = Node;
    type Error = DeError;
    fn serialize_field<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), DeError> {
        ser::SerializeSeq::serialize_element(self, value)
    }
    fn end(self) -> Result<Node, DeError> {
        ser::SerializeSeq::end(self)
    }
}

struct MapSer {
    tree: AttrTree,
    key: Option<String>,
}

impl ser::SerializeMap for MapSer {
    type Ok = Node;
    type Error = DeError;
    fn serialize_key<T: Serialize + ?Sized>(&mut self, key: &T) -> Result<(), DeError> {
        let k = match key.serialize(NodeSerializer)?.value {
            Value::Str(s) => s,
            Value::Int(i) => i.to_string(),
            other => return Err(DeError(format!("map keys must be strings, got {other}"))),
        };
        self.key = Some(k);
        Ok(())
    }
    fn serialize_value<T: Serialize + ?Sized>(&mut self, value: &T) -> Result<(), DeError> {
        let key = self
            .key
            .take()
            .ok_or_else(|| DeError("value without key".to_owned()))?;
        self.tree.insert(key, value.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, DeError> {
        node(Value::Map(self.tree))
    }
}

impl ser::SerializeStruct for MapSer {
    type Ok = Node;
    type Error = DeError;
    fn serialize_field<T: Serialize + ?Sized>(
        &mut self,
        key: &'static str,
        value: &T,
    ) -> Result<(), DeError> {
        self.tree.insert(key, value.serialize(NodeSerializer)?);
        Ok(())
    }
    fn end(self) -> Result<Node, DeError> {
        node(Value::Map(self.tree))
    }
}
