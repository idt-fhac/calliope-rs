//! `schemas/__init__.py`: `CalliopeAttrs`, all model attributes.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{CONFIG, CalliopeConfig};
use crate::error::FieldError;
use crate::join;
use crate::math::{CalliopeMath, MATH};
use crate::model_def::{CalliopeModelDef, MODEL_DEF};
use crate::runtime::{CalliopeRuntime, RUNTIME};
use crate::update::{ModelShape, Schema, Shape};

/// `CalliopeAttrs`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
pub struct CalliopeAttrs {
    /// The model definition.
    pub definition: CalliopeModelDef,
    /// The configuration.
    pub config: CalliopeConfig,
    /// The math.
    pub math: CalliopeMath,
    /// Runtime attributes.
    pub runtime: CalliopeRuntime,
}

static ATTRS: ModelShape = ModelShape {
    name: "CalliopeAttrs",
    fields: &[
        ("definition", Shape::Model(&MODEL_DEF)),
        ("config", Shape::Model(&CONFIG)),
        ("math", Shape::Model(&MATH)),
        ("runtime", Shape::Model(&RUNTIME)),
    ],
    extra: Shape::Leaf,
};

impl Schema for CalliopeAttrs {
    const TITLE: &'static str = "CalliopeAttrs";
    fn shape() -> Shape {
        Shape::Model(&ATTRS)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.definition
            .validate_at(&join(path, "definition"), errors);
        self.config.validate(&join(path, "config"), errors);
        self.math.validate(&join(path, "math"), errors);
    }
}
