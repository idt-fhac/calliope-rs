//! `schemas/runtime_attrs_schema.py`: runtime attributes.

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::config::{Resamples, Subsets};
use crate::types::AttrStr;
use crate::update::{DictShape, ModelShape, Schema, Shape};

/// The Calliope version the port reproduces (`calliope._version.__version__`).
pub const CALLIOPE_VERSION: &str = "0.7.0";

/// `CalliopeRuntime`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Calliope Model Attributes")]
pub struct CalliopeRuntime {
    /// Overrides applied at initialisation (`;`-separated).
    pub applied_overrides: Option<String>,
    /// The Calliope version the model was initialised with.
    pub calliope_version_initialised: String,
    /// Scenario applied at initialisation.
    pub scenario: Option<String>,
    /// Stage timings.
    pub timings: IndexMap<String, f64>,
    /// Termination condition of the last solve.
    pub termination_condition: Option<String>,
    /// Whether the model has been instantiated.
    pub instantiated: bool,
    /// Applied subsets.
    pub subset: Subsets,
    /// Applied resampling.
    pub resample: Resamples,
    /// Applied time clustering.
    pub time_cluster: Option<AttrStr>,
    /// Order in which math entries were applied.
    pub math_priority: Vec<AttrStr>,
}

impl Default for CalliopeRuntime {
    fn default() -> Self {
        Self {
            applied_overrides: None,
            calliope_version_initialised: CALLIOPE_VERSION.to_owned(),
            scenario: None,
            timings: IndexMap::new(),
            termination_condition: None,
            instantiated: false,
            subset: IndexMap::new(),
            resample: IndexMap::new(),
            time_cluster: None,
            math_priority: Vec::new(),
        }
    }
}

static TIMINGS: DictShape = DictShape {
    name: "CalliopeTimings",
    entry: Shape::Leaf,
};
static SUBSETS: DictShape = DictShape {
    name: "Subsets",
    entry: Shape::Leaf,
};
static RESAMPLES: DictShape = DictShape {
    name: "Resamples",
    entry: Shape::Leaf,
};
pub(crate) static RUNTIME: ModelShape = ModelShape {
    name: "CalliopeRuntime",
    fields: &[
        ("timings", Shape::Dict(&TIMINGS)),
        ("subset", Shape::Dict(&SUBSETS)),
        ("resample", Shape::Dict(&RESAMPLES)),
    ],
    extra: Shape::Leaf,
};

impl Schema for CalliopeRuntime {
    const TITLE: &'static str = "Calliope Model Attributes";
    fn shape() -> Shape {
        Shape::Model(&RUNTIME)
    }
}
