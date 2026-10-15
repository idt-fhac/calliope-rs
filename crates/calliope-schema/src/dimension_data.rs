//! `schemas/dimension_data_schema.py`: node, tech and indexed data definitions.

use calliope_yaml::{Node, Value};
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{self, DeserializeOwned, Deserializer};
use serde::{Deserialize, Serialize, Serializer};

use crate::error::FieldError;
use crate::types::{
    AttrStr, DataValue, IndexValue, NonEmptyList, Numeric, UniqueList, from_node_msg,
};
use crate::update::{DictShape, ModelShape, Schema, Shape};

/// `DataValue | NonEmptyList[DataValue]`.
#[derive(Debug, Clone, PartialEq)]
pub enum DataOrList {
    /// One value, applied to all index items.
    One(DataValue),
    /// One value per index item.
    Many(NonEmptyList<DataValue>),
}

impl Serialize for DataOrList {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DataOrList::One(v) => v.serialize(s),
            DataOrList::Many(v) => v.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for DataOrList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        if node.as_seq().is_some() {
            from_node_msg(&node)
                .map(DataOrList::Many)
                .map_err(de::Error::custom)
        } else {
            DataValue::from_value(&node.value)
                .map(DataOrList::One)
                .map_err(de::Error::custom)
        }
    }
}

impl JsonSchema for DataOrList {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DataOrList".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<DataValue>(), g.subschema_for::<NonEmptyList<DataValue>>()
        ]})
    }
}

fn listify_dims<'de, D: Deserializer<'de>>(d: D) -> Result<UniqueList<AttrStr>, D::Error> {
    let node = Node::deserialize(d)?;
    let node = if node.as_seq().is_some() {
        node
    } else {
        Node::new(Value::Seq(vec![node]))
    };
    from_node_msg(&node).map_err(de::Error::custom)
}

fn listify_index<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<UniqueList<IndexValue>>, D::Error> {
    let node = Node::deserialize(d)?;
    let wrap = |n: Node| Node::new(Value::Seq(vec![n]));
    let node = match node.value {
        Value::Seq(items) if items.iter().any(|i| i.as_seq().is_some()) => {
            Node::new(Value::Seq(items))
        }
        Value::Seq(items) => Node::new(Value::Seq(items.into_iter().map(wrap).collect())),
        _ => wrap(wrap(node)),
    };
    from_node_msg(&node).map_err(de::Error::custom)
}

fn default_index() -> Vec<UniqueList<IndexValue>> {
    vec![UniqueList::default()]
}

/// `IndexedData`: `{data, dims, index}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Indexed data definition")]
pub struct IndexedData {
    /// One value for all index items, or one per index item.
    pub data: DataOrList,
    /// Dimensions over which the data is indexed (a single name is listified).
    #[serde(default, deserialize_with = "listify_dims")]
    pub dims: UniqueList<AttrStr>,
    /// Index items; each entry has one item per dimension (scalars are listified).
    #[serde(default = "default_index", deserialize_with = "listify_index")]
    #[schemars(transform = crate::types::without_default)]
    pub index: Vec<UniqueList<IndexValue>>,
}

/// `IndexedTechNodeParam`: indexed data on a tech or node, which must not be indexed over
/// `techs` or `nodes`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(try_from = "IndexedData")]
#[schemars(title = "Indexed `techs` data definition")]
pub struct IndexedTechNodeParam {
    /// The data.
    #[serde(flatten)]
    pub inner: IndexedData,
}

impl TryFrom<IndexedData> for IndexedTechNodeParam {
    type Error = String;
    fn try_from(inner: IndexedData) -> Result<Self, String> {
        let forbidden = ["techs", "nodes"];
        if inner.dims.iter().any(|d| forbidden.contains(&d.as_str())) {
            let dims: Vec<String> = inner.dims.iter().map(|d| format!("'{d}'")).collect();
            return Err(format!(
                "Value error, `dims` must not contain '['techs', 'nodes']', found '[{}]'.",
                dims.join(", ")
            ));
        }
        Ok(Self { inner })
    }
}

/// A tech or node parameter: `IndexedTechNodeParam | DataValue | NonEmptyList[DataValue]`.
#[derive(Debug, Clone, PartialEq)]
pub enum TechParam {
    /// Indexed data.
    Indexed(IndexedTechNodeParam),
    /// One value.
    Value(DataValue),
    /// A list of values.
    List(NonEmptyList<DataValue>),
}

impl Serialize for TechParam {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TechParam::Indexed(v) => v.serialize(s),
            TechParam::Value(v) => v.serialize(s),
            TechParam::List(v) => v.serialize(s),
        }
    }
}

impl TechParam {
    fn from_node(node: &Node) -> Result<Self, String> {
        match &node.value {
            Value::Map(_) => from_node_msg(node).map(TechParam::Indexed),
            Value::Seq(_) => from_node_msg(node).map(TechParam::List),
            other => DataValue::from_value(other).map(TechParam::Value),
        }
    }
}

impl<'de> Deserialize<'de> for TechParam {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        TechParam::from_node(&node).map_err(de::Error::custom)
    }
}

impl JsonSchema for TechParam {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TechParam".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<IndexedTechNodeParam>(),
            g.subschema_for::<DataValue>(),
            g.subschema_for::<NonEmptyList<DataValue>>()
        ]})
    }
}

/// `BaseTech`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum BaseTech {
    /// Converts carriers.
    Conversion,
    /// Consumes a carrier.
    Demand,
    /// Stores a carrier.
    Storage,
    /// Supplies a carrier.
    Supply,
    /// Moves a carrier between nodes.
    Transmission,
}

/// Undeclared (extra) keys of a tech or node: its parameters and lookups.
pub type Params = IndexMap<AttrStr, TechParam>;

/// `Literal["transmission"]`, for the JSON schema of transmission techs.
#[derive(JsonSchema)]
#[serde(rename_all = "lowercase")]
#[allow(dead_code)]
enum TransmissionOnly {
    Transmission,
}

/// Splits a mapping into declared fields and extras, validating extra keys as `AttrStr`.
fn split_extras(node: &Node, fields: &[&str]) -> Result<(IndexMap<String, Node>, Params), String> {
    let Some(map) = node.as_map() else {
        return Err(format!(
            "Input should be a valid dictionary, got {} `{}`",
            node.value.type_name(),
            node.value
        ));
    };
    let mut known = IndexMap::new();
    let mut extra = IndexMap::new();
    for (k, v) in map {
        if fields.contains(&k.as_str()) {
            known.insert(k.clone(), v.clone());
        } else {
            let key = AttrStr::new(k.clone()).map_err(|e| format!("{k}: {e}"))?;
            let val = TechParam::from_node(v).map_err(|e| format!("{k}: {e}"))?;
            extra.insert(key, val);
        }
    }
    Ok((known, extra))
}

fn field<T: DeserializeOwned>(
    known: &IndexMap<String, Node>,
    name: &str,
    default: impl FnOnce() -> T,
) -> Result<T, String> {
    match known.get(name) {
        Some(n) => from_node_msg(n).map_err(|e| format!("{name}: {e}")),
        None => Ok(default()),
    }
}

fn required<T: DeserializeOwned>(known: &IndexMap<String, Node>, name: &str) -> Result<T, String> {
    match known.get(name) {
        Some(n) => from_node_msg(n).map_err(|e| format!("{name}: {e}")),
        None => Err(format!("{name}: Field required")),
    }
}

fn serialize_with_extras<S: Serializer>(
    s: S,
    fields: Vec<(&'static str, Node)>,
    extra: &IndexMap<AttrStr, TechParam>,
) -> Result<S::Ok, S::Error> {
    use serde::ser::SerializeMap;
    let mut map = s.serialize_map(Some(fields.len() + extra.len()))?;
    for (k, v) in &fields {
        map.serialize_entry(k, v)?;
    }
    for (k, v) in extra {
        map.serialize_entry(k.as_str(), v)?;
    }
    map.end()
}

fn to_node<T: Serialize>(v: &T) -> Node {
    calliope_yaml::to_node(v).unwrap_or_else(|_| Node::new(Value::Null))
}

/// `CalliopeTech`: a technology definition. Undeclared keys are parameters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalliopeTech {
    /// If false, the tech is deactivated.
    pub active: bool,
    /// The abstract base class.
    pub base_tech: Option<BaseTech>,
    /// Parameters and lookups.
    pub extra: IndexMap<AttrStr, TechParam>,
}

impl CalliopeTech {
    const FIELDS: [&'static str; 2] = ["active", "base_tech"];

    fn from_node(node: &Node) -> Result<Self, String> {
        let (known, extra) = split_extras(node, &Self::FIELDS)?;
        Ok(Self {
            active: field(&known, "active", || true)?,
            base_tech: field(&known, "base_tech", || None)?,
            extra,
        })
    }
}

impl Serialize for CalliopeTech {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_with_extras(
            s,
            vec![
                ("active", to_node(&self.active)),
                ("base_tech", to_node(&self.base_tech)),
            ],
            &self.extra,
        )
    }
}

impl<'de> Deserialize<'de> for CalliopeTech {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        CalliopeTech::from_node(&node).map_err(de::Error::custom)
    }
}

/// `CalliopeTransmissionTech`: a technology with `link_from` and `link_to`.
#[derive(Debug, Clone, PartialEq)]
pub struct CalliopeTransmissionTech {
    /// If false, the tech is deactivated.
    pub active: bool,
    /// `None` or `transmission`.
    pub base_tech: Option<BaseTech>,
    /// Node the link starts at.
    pub link_from: AttrStr,
    /// Node the link ends at.
    pub link_to: AttrStr,
    /// Flow only from `link_from` to `link_to`.
    pub one_way: bool,
    /// Parameters and lookups.
    pub extra: IndexMap<AttrStr, TechParam>,
}

impl CalliopeTransmissionTech {
    const FIELDS: [&'static str; 5] = ["active", "base_tech", "link_from", "link_to", "one_way"];

    fn from_node(node: &Node) -> Result<Self, String> {
        let (known, extra) = split_extras(node, &Self::FIELDS)?;
        let base_tech: Option<BaseTech> = field(&known, "base_tech", || None)?;
        if base_tech.is_some_and(|b| b != BaseTech::Transmission) {
            return Err("base_tech: Input should be 'transmission'".to_owned());
        }
        Ok(Self {
            active: field(&known, "active", || true)?,
            base_tech,
            link_from: required(&known, "link_from")?,
            link_to: required(&known, "link_to")?,
            one_way: field(&known, "one_way", || false)?,
            extra,
        })
    }
}

impl Serialize for CalliopeTransmissionTech {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_with_extras(
            s,
            vec![
                ("active", to_node(&self.active)),
                ("base_tech", to_node(&self.base_tech)),
                ("link_from", to_node(&self.link_from)),
                ("link_to", to_node(&self.link_to)),
                ("one_way", to_node(&self.one_way)),
            ],
            &self.extra,
        )
    }
}

/// A tech definition: `CalliopeTransmissionTech | CalliopeTech`. As in pydantic's smart
/// union, a definition that validates as a transmission tech is one.
#[derive(Debug, Clone, PartialEq)]
pub enum TechDef {
    /// A transmission tech (has `link_from` and `link_to`).
    Transmission(CalliopeTransmissionTech),
    /// Any other tech.
    Tech(CalliopeTech),
}

impl TechDef {
    /// Whether the tech is active.
    pub fn active(&self) -> bool {
        match self {
            TechDef::Transmission(t) => t.active,
            TechDef::Tech(t) => t.active,
        }
    }

    /// The base tech, if given.
    pub fn base_tech(&self) -> Option<BaseTech> {
        match self {
            TechDef::Transmission(t) => t.base_tech,
            TechDef::Tech(t) => t.base_tech,
        }
    }

    /// The parameters.
    pub fn extra(&self) -> &IndexMap<AttrStr, TechParam> {
        match self {
            TechDef::Transmission(t) => &t.extra,
            TechDef::Tech(t) => &t.extra,
        }
    }

    fn from_node(node: &Node) -> Result<Self, String> {
        let transmission = CalliopeTransmissionTech::from_node(node);
        match transmission {
            Ok(t) => Ok(TechDef::Transmission(t)),
            Err(t_err) => CalliopeTech::from_node(node)
                .map(TechDef::Tech)
                .map_err(|e| {
                    let says_transmission = node
                        .as_map()
                        .and_then(|m| m.get("base_tech"))
                        .and_then(Node::as_str)
                        == Some("transmission");
                    if says_transmission { t_err } else { e }
                }),
        }
    }
}

impl Serialize for TechDef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            TechDef::Transmission(t) => t.serialize(s),
            TechDef::Tech(t) => t.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for TechDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        TechDef::from_node(&node).map_err(de::Error::custom)
    }
}

/// `CalliopeTechs`: techs by name. `null` (for the whole map or an entry) means `{}`.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[serde(transparent)]
pub struct CalliopeTechs(pub IndexMap<AttrStr, TechDef>);

impl<'de> Deserialize<'de> for CalliopeTechs {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        let mut out = IndexMap::new();
        match &node.value {
            Value::Null => {}
            Value::Map(m) => {
                for (k, v) in m {
                    let key = AttrStr::new(k.clone())
                        .map_err(|e| de::Error::custom(format!("{k}: {e}")))?;
                    let v = if v.is_null() {
                        Node::empty_map()
                    } else {
                        v.clone()
                    };
                    let tech = TechDef::from_node(&v)
                        .map_err(|e| de::Error::custom(format!("{k}: {e}")))?;
                    out.insert(key, tech);
                }
            }
            other => {
                return Err(de::Error::custom(format!(
                    "Input should be a valid dictionary, got {} `{other}`",
                    other.type_name()
                )));
            }
        }
        Ok(CalliopeTechs(out))
    }
}

/// `CalliopeNode`: a node definition. Undeclared keys are parameters.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CalliopeNode {
    /// If false, the node is deactivated.
    pub active: bool,
    /// Latitude (WGS84), in [-90, 90].
    pub latitude: Option<Numeric>,
    /// Longitude (WGS84), in [-180, 180].
    pub longitude: Option<Numeric>,
    /// Techs at this node, with node-specific overrides.
    pub techs: CalliopeTechs,
    /// Parameters and lookups.
    pub extra: IndexMap<AttrStr, TechParam>,
}

fn bounded(v: Option<Numeric>, name: &str, lo: f64, hi: f64) -> Result<Option<Numeric>, String> {
    if let Some(n) = v {
        let x = n.as_f64();
        if x.is_nan() || x < lo {
            return Err(format!(
                "{name}: Input should be greater than or equal to {lo}"
            ));
        }
        if x > hi {
            return Err(format!(
                "{name}: Input should be less than or equal to {hi}"
            ));
        }
    }
    Ok(v)
}

impl CalliopeNode {
    const FIELDS: [&'static str; 4] = ["active", "latitude", "longitude", "techs"];

    fn from_node(node: &Node) -> Result<Self, String> {
        let (known, extra) = split_extras(node, &Self::FIELDS)?;
        let latitude = bounded(field(&known, "latitude", || None)?, "latitude", -90.0, 90.0)?;
        let longitude = bounded(
            field(&known, "longitude", || None)?,
            "longitude",
            -180.0,
            180.0,
        )?;
        Ok(Self {
            active: field(&known, "active", || true)?,
            latitude,
            longitude,
            techs: field(&known, "techs", CalliopeTechs::default)?,
            extra,
        })
    }

    /// `check_dependent_definitions`: latitude and longitude must have the same type
    /// (both missing, both int or both float).
    fn check(&self) -> Result<(), String> {
        let kind = |v: Option<Numeric>| v.map(Numeric::is_float);
        if kind(self.latitude) != kind(self.longitude) {
            let show = |v: Option<Numeric>| v.map_or_else(|| "None".to_owned(), |n| n.to_string());
            return Err(format!(
                "Value error, Invalid latitude/longitude definition. Types must match, found ({}, {}).",
                show(self.latitude),
                show(self.longitude)
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_at(&self, path: &str, errors: &mut Vec<FieldError>) {
        if let Err(msg) = self.check() {
            errors.push(FieldError::new(path, msg));
        }
    }
}

impl Serialize for CalliopeNode {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        serialize_with_extras(
            s,
            vec![
                ("active", to_node(&self.active)),
                ("latitude", to_node(&self.latitude)),
                ("longitude", to_node(&self.longitude)),
                ("techs", to_node(&self.techs)),
            ],
            &self.extra,
        )
    }
}

impl<'de> Deserialize<'de> for CalliopeNode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        CalliopeNode::from_node(&node).map_err(de::Error::custom)
    }
}

/// `CalliopeNodes`: nodes by name.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CalliopeNodes(pub IndexMap<AttrStr, CalliopeNode>);

impl CalliopeNodes {
    pub(crate) fn validate_at(&self, path: &str, errors: &mut Vec<FieldError>) {
        for (name, node) in &self.0 {
            node.validate_at(&crate::join(path, name), errors);
        }
    }
}

/// A top-level data definition: `DataValue | IndexedData`.
#[derive(Debug, Clone, PartialEq)]
pub enum DataDef {
    /// A single value.
    Value(DataValue),
    /// Indexed data.
    Indexed(IndexedData),
}

impl Serialize for DataDef {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        match self {
            DataDef::Value(v) => v.serialize(s),
            DataDef::Indexed(v) => v.serialize(s),
        }
    }
}

impl<'de> Deserialize<'de> for DataDef {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        match &node.value {
            Value::Map(_) => from_node_msg(&node)
                .map(DataDef::Indexed)
                .map_err(de::Error::custom),
            other => DataValue::from_value(other)
                .map(DataDef::Value)
                .map_err(de::Error::custom),
        }
    }
}

/// `CalliopeDataDef`: top-level data definitions by name.
pub type CalliopeDataDef = IndexMap<AttrStr, DataDef>;

macro_rules! json_schema_object {
    ($t:ty, $name:literal, $title:literal, { $($field:literal : $ft:ty = $default:tt),* }) => {
        impl JsonSchema for $t {
            fn schema_name() -> std::borrow::Cow<'static, str> {
                $name.into()
            }
            fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
                let mut props = serde_json::Map::new();
                $(
                    let mut field = g.subschema_for::<$ft>().to_value();
                    let default: serde_json::Value = serde_json::json!($default);
                    if !default.is_string() || default != serde_json::json!("required") {
                        if let Some(obj) = field.as_object_mut() {
                            obj.insert("default".to_owned(), default);
                        } else {
                            field = serde_json::json!({"allOf": [field], "default": default});
                        }
                    }
                    props.insert($field.to_owned(), field);
                )*
                let required: Vec<&str> = [$($field),*]
                    .into_iter()
                    .zip([$(serde_json::json!($default) == serde_json::json!("required")),*])
                    .filter(|(_, r)| *r)
                    .map(|(f, _)| f)
                    .collect();
                let mut schema = schemars::json_schema!({
                    "title": $title,
                    "type": "object",
                    "properties": props,
                    "additionalProperties": g.subschema_for::<TechParam>(),
                });
                if !required.is_empty() {
                    schema.insert("required".to_owned(), serde_json::json!(required));
                }
                schema
            }
        }
    };
}

json_schema_object!(CalliopeTech, "CalliopeTech", "Technology dimension data", {
    "active": bool = true, "base_tech": Option<BaseTech> = null
});
json_schema_object!(
    CalliopeTransmissionTech,
    "CalliopeTransmissionTech",
    "Transmission technology dimension data",
    {
        "active": bool = true, "base_tech": Option<TransmissionOnly> = null,
        "link_from": AttrStr = "required", "link_to": AttrStr = "required",
        "one_way": bool = false
    }
);
json_schema_object!(CalliopeNode, "CalliopeNode", "Node dimension", {
    "active": bool = true, "latitude": Option<Numeric> = null,
    "longitude": Option<Numeric> = null, "techs": CalliopeTechs = {}
});

impl JsonSchema for TechDef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "TechDef".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<CalliopeTransmissionTech>(), g.subschema_for::<CalliopeTech>()
        ]})
    }
}

impl JsonSchema for CalliopeTechs {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CalliopeTechs".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "propertyNames": g.subschema_for::<AttrStr>(),
            "additionalProperties": g.subschema_for::<TechDef>(),
        })
    }
}

impl JsonSchema for CalliopeNodes {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "CalliopeNodes".into()
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "object",
            "propertyNames": g.subschema_for::<AttrStr>(),
            "additionalProperties": g.subschema_for::<CalliopeNode>(),
        })
    }
}

impl JsonSchema for DataDef {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "DataDef".into()
    }
    fn inline_schema() -> bool {
        true
    }
    fn json_schema(g: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({"anyOf": [
            g.subschema_for::<DataValue>(), g.subschema_for::<IndexedData>()
        ]})
    }
}

pub(crate) static INDEXED: ModelShape = ModelShape {
    name: "IndexedTechNodeParam",
    fields: &[],
    extra: Shape::Leaf,
};
static INDEXED_DATA: ModelShape = ModelShape {
    name: "IndexedData",
    fields: &[],
    extra: Shape::Leaf,
};
pub(crate) static TECH: ModelShape = ModelShape {
    name: "CalliopeTech",
    fields: &[],
    extra: Shape::MapOr(&INDEXED),
};
pub(crate) static TECHS: DictShape = DictShape {
    name: "CalliopeTechs",
    entry: Shape::Model(&TECH),
};
pub(crate) static NODE: ModelShape = ModelShape {
    name: "CalliopeNode",
    fields: &[("techs", Shape::Dict(&TECHS))],
    extra: Shape::MapOr(&INDEXED),
};
pub(crate) static NODES: DictShape = DictShape {
    name: "CalliopeNodes",
    entry: Shape::Model(&NODE),
};
pub(crate) static DATA_DEF: DictShape = DictShape {
    name: "CalliopeDataDef",
    entry: Shape::MapOr(&INDEXED_DATA),
};

impl Schema for IndexedData {
    const TITLE: &'static str = "Indexed data definition";
    fn shape() -> Shape {
        Shape::Model(&INDEXED_DATA)
    }
}

impl Schema for IndexedTechNodeParam {
    const TITLE: &'static str = "Indexed `techs` data definition";
    fn shape() -> Shape {
        Shape::Model(&INDEXED)
    }
}

impl Schema for CalliopeTech {
    const TITLE: &'static str = "Technology dimension data";
    fn shape() -> Shape {
        Shape::Model(&TECH)
    }
}

impl Schema for TechDef {
    const TITLE: &'static str = "Technology dimension data";
    fn shape() -> Shape {
        Shape::Model(&TECH)
    }
}

impl Schema for CalliopeTechs {
    const TITLE: &'static str = "CalliopeTechs";
    fn shape() -> Shape {
        Shape::Dict(&TECHS)
    }
}

impl Schema for CalliopeNode {
    const TITLE: &'static str = "Node dimension";
    fn shape() -> Shape {
        Shape::Model(&NODE)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_at(path, errors);
    }
}

impl Schema for CalliopeNodes {
    const TITLE: &'static str = "CalliopeNodes";
    fn shape() -> Shape {
        Shape::Dict(&NODES)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_at(path, errors);
    }
}
