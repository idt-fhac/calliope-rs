//! `prepare_model_definition` (`preprocess/model_definition.py`), without scenarios and
//! overrides (Phase 3) and without math loading.

use calliope_schema::{CalliopeConfig, CalliopeModelDef, Validated};
use calliope_yaml::{AttrTree, Node};

use crate::error::ModelError;
use crate::templates::TemplateSolver;

/// A validated model definition and configuration.
#[derive(Debug, Clone)]
pub struct PreparedModel {
    /// `config`.
    pub config: Validated<CalliopeConfig>,
    /// Everything else (`nodes`, `techs`, `data_tables`, `data_definitions`).
    pub definition: Validated<CalliopeModelDef>,
}

/// Applies templates and `init` keyword overrides, then validates config and definition.
///
/// `overrides`/`scenarios` are removed (scenario selection is not ported yet), and the
/// deprecated `locations` key is renamed to `nodes`, as in Python.
pub fn prepare_model_definition(
    model_definition: &AttrTree,
    init_kwargs: &AttrTree,
) -> Result<PreparedModel, ModelError> {
    let mut def = model_definition.clone();
    def.remove("overrides");
    def.remove("scenarios");
    if let Some(locations) = def.remove("locations") {
        tracing::warn!(
            "`locations` has been renamed to `nodes` and will stop working in v0.7.1. \
             Please update your model configuration accordingly."
        );
        def.insert("nodes", locations);
    }
    let mut def = TemplateSolver::new(&def)?.resolved_data;
    let mut kwargs = AttrTree::new();
    kwargs.set_key("config.init", Node::from(init_kwargs.clone()))?;
    def.union(&kwargs, true, false)?;
    let config = match def.remove("config").map(|n| n.value) {
        Some(calliope_yaml::Value::Map(m)) => m,
        Some(_) => return Err(ModelError::new("`config` must be a mapping.")),
        None => AttrTree::new(),
    };
    Ok(PreparedModel {
        config: Validated::from_tree(config)?,
        definition: Validated::from_tree(def)?,
    })
}
