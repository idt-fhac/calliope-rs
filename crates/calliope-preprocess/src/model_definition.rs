//! `prepare_model_definition` (`preprocess/model_definition.py`): overrides, scenarios,
//! templates, `init` keyword overrides and math loading.

use std::path::Path;

use calliope_schema::{CalliopeConfig, CalliopeModelDef, Validated};
use calliope_yaml::{AttrTree, Node, Value};
use indexmap::IndexMap;

use crate::error::ModelError;
use crate::model_math::initialise_math;
use crate::templates::TemplateSolver;

/// A validated model definition and configuration, with the loaded math files.
#[derive(Debug, Clone)]
pub struct PreparedModel {
    /// `config`.
    pub config: Validated<CalliopeConfig>,
    /// Everything else (`nodes`, `techs`, `data_tables`, `data_definitions`).
    pub definition: Validated<CalliopeModelDef>,
    /// `math.init`: every loaded math file by name.
    pub math_init: IndexMap<String, AttrTree>,
    /// `runtime.applied_overrides`: the applied overrides joined by `;`.
    pub applied_overrides: String,
    /// `runtime.scenario`.
    pub scenario: Option<String>,
}

/// Applies templates and `init` keyword overrides (no scenario, no override dict), then
/// validates config and definition and loads the pre-defined math.
pub fn prepare_model_definition(
    model_definition: &AttrTree,
    init_kwargs: &AttrTree,
) -> Result<PreparedModel, ModelError> {
    prepare_model_definition_with(model_definition, None, None, None, None, init_kwargs)
}

/// `prepare_model_definition`: priority is `init_kwargs` > `override_dict` > scenarios >
/// model definition > templates.
pub fn prepare_model_definition_with(
    model_definition: &AttrTree,
    scenario: Option<&str>,
    override_dict: Option<&AttrTree>,
    math_dict: Option<&AttrTree>,
    definition_path: Option<&Path>,
    init_kwargs: &AttrTree,
) -> Result<PreparedModel, ModelError> {
    let (def, applied) = load_scenario_overrides(model_definition, scenario, override_dict)?;
    let mut def = TemplateSolver::new(&def)?.resolved_data;
    let mut kwargs = AttrTree::new();
    kwargs.set_key("config.init", Node::from(init_kwargs.clone()))?;
    def.union(&kwargs, true, false)?;
    let config = match def.remove("config").map(|n| n.value) {
        Some(Value::Map(m)) => m,
        Some(Value::Null) | None => AttrTree::new(),
        Some(_) => return Err(ModelError::new("`config` must be a mapping.")),
    };
    let config: Validated<CalliopeConfig> = Validated::from_tree(config)?;
    let definition = Validated::from_tree(def)?;
    let mut math_init = initialise_math(&config.init.math_paths, definition_path)?;
    if let Some(extra) = math_dict {
        // `math.union(math_dict, allow_override=True)` on the AttrDict of math files.
        let mut tree = AttrTree::new();
        for (k, v) in &math_init {
            tree.insert(k.clone(), Node::from(v.clone()));
        }
        tree.union(extra, true, false)?;
        math_init = tree
            .iter()
            .map(|(k, v)| (k.clone(), v.as_map().cloned().unwrap_or_default()))
            .collect();
    }
    Ok(PreparedModel {
        config,
        definition,
        math_init,
        applied_overrides: applied.join(";"),
        scenario: scenario.map(str::to_owned),
    })
}

/// `_load_scenario_overrides`: `override_dict`, then the scenario's overrides, then
/// `override_dict` again (so it has the last word).
pub fn load_scenario_overrides(
    model_definition: &AttrTree,
    scenario: Option<&str>,
    override_dict: Option<&AttrTree>,
) -> Result<(AttrTree, Vec<String>), ModelError> {
    let mut def = model_definition.clone();
    let mut overrides = override_dict.cloned().unwrap_or_default();
    def.union(&overrides, true, true)?;
    let scenario_overrides = take_map(&mut def, "overrides");
    let scenarios = take_map(&mut def, "scenarios");
    let applied = match scenario {
        Some(s) => {
            let applied = load_overrides_from_scenario(s, &scenario_overrides, &scenarios)?;
            tracing::info!("(scenarios, {s} ) | Applying the following overrides: {applied:?}.");
            let combined = combine_overrides(&scenario_overrides, &applied)?;
            def.union(&combined, true, true)?;
            applied
        }
        None => Vec::new(),
    };
    if !overrides.is_empty() {
        overrides.remove("scenarios");
        overrides.remove("overrides");
        def.union(&overrides, true, true)?;
    }
    if let Some(locations) = def.remove("locations") {
        tracing::warn!(
            "`locations` has been renamed to `nodes` and will stop working in v0.7.1. \
             Please update your model configuration accordingly."
        );
        def.insert("nodes", locations);
    }
    Ok((def, applied))
}

fn take_map(def: &mut AttrTree, key: &str) -> AttrTree {
    match def.remove(key).map(|n| n.value) {
        Some(Value::Map(m)) => m,
        _ => AttrTree::new(),
    }
}

/// `_combine_overrides`: unions the named overrides; a key set by two of them is an error.
fn combine_overrides(overrides: &AttrTree, names: &[String]) -> Result<AttrTree, ModelError> {
    let mut combined = AttrTree::new();
    for name in names {
        let Some(o) = overrides.get(name) else {
            return Err(ModelError::new(format!(
                "Override `{name}` is not defined."
            )));
        };
        let tree = match &o.value {
            Value::Map(m) => m.clone(),
            _ => AttrTree::new(),
        };
        combined.union(&tree, false, false).map_err(|e| {
            ModelError::new(format!(
                "{e}. Already specified but defined again in override `{name}`."
            ))
        })?;
    }
    Ok(combined)
}

/// `listify` of a YAML node (a mapping gives its keys, as iterating a dict does).
fn listify(node: &Node) -> Vec<Node> {
    match &node.value {
        Value::Null => Vec::new(),
        Value::Seq(items) => items.clone(),
        Value::Map(m) => m.keys().map(|k| Node::new(Value::Str(k.clone()))).collect(),
        _ => vec![node.clone()],
    }
}

/// `_load_overrides_from_scenario`: expands a scenario (or comma-separated override names)
/// into override names, recursively. Python collects them in a set; here they keep the
/// order in which they are first named.
fn load_overrides_from_scenario(
    scenario: &str,
    overrides: &AttrTree,
    scenarios: &AttrTree,
) -> Result<Vec<String>, ModelError> {
    let list: Vec<Node> = match scenarios.get(scenario) {
        Some(n) => {
            tracing::info!("Loading overrides from scenario: {scenario} ");
            listify(n)
        }
        None => scenario
            .split(',')
            .map(|s| Node::new(Value::Str(s.to_owned())))
            .collect(),
    };
    let mut out: Vec<String> = Vec::new();
    for item in list {
        if item.is_map() {
            return Err(ModelError::new(
                "(scenarios, {scenario}) | Scenario definition must be a list of override or other scenario names.",
            ));
        }
        let name = match &item.value {
            Value::Str(s) => s.clone(),
            other => other.to_string(),
        };
        if scenarios.contains_key(&name) {
            for o in load_overrides_from_scenario(&name, overrides, scenarios)? {
                if !out.contains(&o) {
                    out.push(o);
                }
            }
        } else if !overrides.contains_key(&name) {
            return Err(ModelError::new(format!(
                "(scenarios, {scenario}) | Unrecognised override name: {name}."
            )));
        } else if !out.contains(&name) {
            out.push(name);
        }
    }
    Ok(out)
}
