//! `schemas/data_table_schema.py`: tabular data definitions.

use std::collections::BTreeSet;

use calliope_yaml::Node;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize};

use crate::error::FieldError;
use crate::types::{AttrStr, OneOrMany};
use crate::update::{DictShape, ModelShape, Schema, Shape};

/// `AttrStr | list[AttrStr]` (duplicates allowed), keeping the given form.
#[derive(Debug, Clone, PartialEq, Serialize, JsonSchema)]
#[serde(untagged)]
pub enum OneOrList {
    /// A single name.
    One(AttrStr),
    /// A list of names.
    Many(Vec<AttrStr>),
}

impl OneOrList {
    /// `listify`.
    pub fn to_vec(&self) -> Vec<AttrStr> {
        match self {
            OneOrList::One(x) => vec![x.clone()],
            OneOrList::Many(xs) => xs.clone(),
        }
    }
}

impl<'de> Deserialize<'de> for OneOrList {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let node = Node::deserialize(d)?;
        if node.as_seq().is_some() {
            crate::types::from_node_msg(&node)
                .map(OneOrList::Many)
                .map_err(de::Error::custom)
        } else {
            crate::types::from_node_msg(&node)
                .map(OneOrList::One)
                .map_err(de::Error::custom)
        }
    }
}

/// `CalliopeDataTable`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Data table schema")]
pub struct CalliopeDataTable {
    /// Path of the table file, relative to the model definition file.
    pub table: String,
    /// Dimensions defined row-wise (index columns on the left).
    #[serde(default)]
    pub rows: Option<OneOrMany<AttrStr>>,
    /// Dimensions defined column-wise (header rows on top).
    #[serde(default)]
    pub columns: Option<OneOrMany<AttrStr>>,
    /// Index items to select per dimension (before `drop` and `add_dims`).
    #[serde(default)]
    pub select: Option<IndexMap<AttrStr, OneOrMany<AttrStr>>>,
    /// Rows/columns to drop.
    #[serde(default)]
    pub drop: Option<OneOrMany<AttrStr>>,
    /// Dimensions to add after loading.
    #[serde(default)]
    pub add_dims: Option<IndexMap<AttrStr, OneOrList>>,
    /// Map from table dimension names to Calliope dimension names.
    #[serde(default)]
    pub rename_dims: Option<IndexMap<AttrStr, AttrStr>>,
}

fn listify(x: Option<&OneOrMany<AttrStr>>) -> BTreeSet<String> {
    x.map(|v| {
        v.to_vec()
            .into_iter()
            .map(|a| a.as_str().to_owned())
            .collect()
    })
    .unwrap_or_default()
}

impl CalliopeDataTable {
    /// `check_row_and_columns`.
    fn check(&self) -> Result<(), &'static str> {
        let drop = listify(self.drop.as_ref());
        let rows: BTreeSet<String> = listify(self.rows.as_ref())
            .difference(&drop)
            .cloned()
            .collect();
        let columns: BTreeSet<String> = listify(self.columns.as_ref())
            .difference(&drop)
            .cloned()
            .collect();
        if rows.is_empty() && columns.is_empty() {
            return Err("Either row or columns must be defined (and not dropped).");
        }
        if !rows.is_disjoint(&columns) {
            return Err("Rows and columns must not overlap.");
        }
        let dims: BTreeSet<String> = rows.union(&columns).cloned().collect();
        if let Some(add) = &self.add_dims
            && add.keys().any(|k| dims.contains(k.as_str()))
        {
            return Err("Added dimensions must not be in columns or rows.");
        }
        if let Some(rename) = &self.rename_dims
            && rename.values().any(|v| !dims.contains(v.as_str()))
        {
            return Err("Renamed dimensions must be in either rows or columns.");
        }
        Ok(())
    }

    pub(crate) fn validate_at(&self, path: &str, errors: &mut Vec<FieldError>) {
        if let Err(msg) = self.check() {
            errors.push(FieldError::new(path, format!("Value error, {msg}")));
        }
    }
}

pub(crate) static DATA_TABLE: ModelShape = ModelShape {
    name: "CalliopeDataTable",
    fields: &[],
    extra: Shape::Leaf,
};

pub(crate) static DATA_TABLES: DictShape = DictShape {
    name: "CalliopeDataTables",
    entry: Shape::Model(&DATA_TABLE),
};

impl Schema for CalliopeDataTable {
    const TITLE: &'static str = "Data table schema";
    fn shape() -> Shape {
        Shape::Model(&DATA_TABLE)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_at(path, errors);
    }
}

/// `CalliopeDataTables`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
pub struct CalliopeDataTables(pub IndexMap<AttrStr, CalliopeDataTable>);

impl CalliopeDataTables {
    pub(crate) fn validate_at(&self, path: &str, errors: &mut Vec<FieldError>) {
        for (name, table) in &self.0 {
            table.validate_at(&crate::join(path, name), errors);
        }
    }
}

impl Schema for CalliopeDataTables {
    const TITLE: &'static str = "CalliopeDataTables";
    fn shape() -> Shape {
        Shape::Dict(&DATA_TABLES)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_at(path, errors);
    }
}
