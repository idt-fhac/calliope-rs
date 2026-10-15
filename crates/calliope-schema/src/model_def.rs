//! `schemas/model_def_schema.py`: the model definition (after templates and overrides).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::data_table::{CalliopeDataTables, DATA_TABLES};
use crate::dimension_data::{
    CalliopeDataDef, CalliopeNodes, CalliopeTechs, DATA_DEF, NODES, TECHS,
};
use crate::error::FieldError;
use crate::join;
use crate::update::{ModelShape, Schema, Shape};

/// `CalliopeModelDef`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Calliope Model Definition")]
pub struct CalliopeModelDef {
    /// YAML-based input data.
    pub data_definitions: CalliopeDataDef,
    /// Tabular input data.
    pub data_tables: CalliopeDataTables,
    /// Node-specific input data.
    pub nodes: CalliopeNodes,
    /// Tech-specific input data.
    pub techs: CalliopeTechs,
}

impl CalliopeModelDef {
    /// Names of all data definitions.
    pub fn data_definition_names(&self) -> impl Iterator<Item = &str> {
        self.data_definitions.keys().map(|k| k.as_str())
    }
}

pub(crate) static MODEL_DEF: ModelShape = ModelShape {
    name: "CalliopeModelDef",
    fields: &[
        ("data_definitions", Shape::Dict(&DATA_DEF)),
        ("data_tables", Shape::Dict(&DATA_TABLES)),
        ("nodes", Shape::Dict(&NODES)),
        ("techs", Shape::Dict(&TECHS)),
    ],
    extra: Shape::Leaf,
};

impl CalliopeModelDef {
    pub(crate) fn validate_at(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.data_tables
            .validate_at(&join(path, "data_tables"), errors);
        self.nodes.validate_at(&join(path, "nodes"), errors);
    }
}

impl Schema for CalliopeModelDef {
    const TITLE: &'static str = "Calliope Model Definition";
    fn shape() -> Shape {
        Shape::Model(&MODEL_DEF)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.validate_at(path, errors);
    }
}
