//! `TemplateSolver` (`preprocess/model_definition.py:206-281`).

use calliope_yaml::{AttrTree, Node, Value};
use indexmap::IndexMap;

use crate::error::ModelError;

const TEMPLATES_SECTION: &str = "templates";
const TEMPLATE_CALL: &str = "template";

/// Resolves `templates:` and `template:` references.
///
/// - Templates may inherit from other templates (`template:` inside a template); cycles are
///   an error.
/// - Any mapping in the data may call a template; its own keys win over the template's.
/// - `templates:` may only appear at the top level, and is removed from the result.
///
/// Python collects a template's own keys in a `set`, so their order is not deterministic; here
/// they keep their definition order.
#[derive(Debug, Clone)]
pub struct TemplateSolver {
    /// Templates with inheritance applied, in definition order.
    pub resolved_templates: IndexMap<String, AttrTree>,
    /// The data with templates applied and template definitions removed.
    pub resolved_data: AttrTree,
}

impl TemplateSolver {
    /// Resolves the templates in `data`.
    pub fn new(data: &AttrTree) -> Result<Self, ModelError> {
        let raw_templates: AttrTree = match data.get(TEMPLATES_SECTION) {
            None => AttrTree::new(),
            Some(n) => match &n.value {
                Value::Map(m) => m.clone(),
                // `data.get_key("templates", AttrDict())` returns whatever is there.
                _ => {
                    return Err(ModelError::at(
                        "Template definitions must be YAML blocks.",
                        n.span.clone(),
                    ));
                }
            },
        };
        let mut solver = Self {
            resolved_templates: IndexMap::new(),
            resolved_data: AttrTree::new(),
        };
        for (key, value) in &raw_templates {
            if !value.is_map() {
                return Err(ModelError::at(
                    "Template definitions must be YAML blocks.",
                    value.span.clone(),
                ));
            }
            let resolved = solver.resolve_template(&raw_templates, key, &mut Vec::new())?;
            solver.resolved_templates.insert(key.clone(), resolved);
        }
        solver.resolved_data = match solver.resolve_data(&Node::from(data.clone()), 0)?.value {
            Value::Map(m) => m,
            _ => unreachable!("a map resolves to a map"),
        };
        Ok(solver)
    }

    fn resolve_template(
        &self,
        raw_templates: &AttrTree,
        name: &str,
        stack: &mut Vec<String>,
    ) -> Result<AttrTree, ModelError> {
        if stack.iter().any(|s| s == name) {
            return Err(ModelError::new(format!(
                "Circular template reference detected for '{name}'."
            )));
        }
        stack.push(name.to_owned());
        let raw = raw_templates
            .get(name)
            .and_then(Node::as_map)
            .ok_or_else(|| ModelError::new(format!("Template `{name}` is not defined.")))?;
        let mut result = AttrTree::new();
        if let Some(call) = raw.get(TEMPLATE_CALL) {
            let inherited_name = template_name(call)?;
            let inherited = match self.resolved_templates.get(inherited_name) {
                Some(t) => t.clone(),
                None => self.resolve_template(raw_templates, inherited_name, stack)?,
            };
            result.union(&inherited, false, false)?;
        }
        let mut local = AttrTree::new();
        for (k, v) in raw {
            if k != TEMPLATE_CALL {
                local.insert(k.clone(), v.clone());
            }
        }
        result.union(&local, true, false)?;
        stack.pop();
        Ok(result)
    }

    fn resolve_data(&self, section: &Node, level: usize) -> Result<Node, ModelError> {
        let Value::Map(map) = &section.value else {
            return Ok(section.clone());
        };
        if let Some(t) = map.get(TEMPLATES_SECTION)
            && level != 0
        {
            return Err(ModelError::at(
                "Template definitions must be placed at the top level of the YAML file.",
                t.span.clone(),
            ));
        }
        let mut template = match map.get(TEMPLATE_CALL) {
            Some(call) => {
                let name = template_name(call)?;
                self.resolved_templates.get(name).cloned().ok_or_else(|| {
                    ModelError::at(
                        format!("Template `{name}` is not defined."),
                        call.span.clone(),
                    )
                })?
            }
            None => AttrTree::new(),
        };
        let mut local = AttrTree::new();
        for (k, v) in map {
            if k != TEMPLATE_CALL && k != TEMPLATES_SECTION {
                local.insert(k.clone(), self.resolve_data(v, level + 1)?);
            }
        }
        template.union(&local, true, false)?;
        Ok(Node::with_span(Value::Map(template), section.span.clone()))
    }
}

fn template_name(call: &Node) -> Result<&str, ModelError> {
    call.as_str().ok_or_else(|| {
        ModelError::at(
            format!(
                "A template reference must be a name, found `{}`.",
                call.value
            ),
            call.span.clone(),
        )
    })
}
