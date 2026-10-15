//! The component names a math string is parsed against.

use std::collections::BTreeSet;

use calliope_schema::{CalliopeBuildMath, NameGroups};

use crate::ast::Role;

/// Names by role, as Calliope's `parsing_components` groups them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Names {
    /// Dimension names.
    pub dimensions: BTreeSet<String>,
    /// Parameter and lookup names.
    pub inputs: BTreeSet<String>,
    /// Variable and global expression names.
    pub results: BTreeSet<String>,
    /// Postprocessed expression names.
    pub postprocessed: BTreeSet<String>,
}

impl Names {
    /// Names from string slices (for tests and tools).
    pub fn from_lists(dimensions: &[&str], inputs: &[&str], results: &[&str]) -> Self {
        let set = |l: &[&str]| l.iter().map(|s| (*s).to_owned()).collect();
        Self {
            dimensions: set(dimensions),
            inputs: set(inputs),
            results: set(results),
            postprocessed: BTreeSet::new(),
        }
    }

    fn from_groups(g: &NameGroups) -> Self {
        Self {
            dimensions: g.dimensions.clone(),
            inputs: g.inputs.clone(),
            results: g.results.clone(),
            postprocessed: BTreeSet::new(),
        }
    }

    /// The role of a name, in Calliope's matching order: dimension, input, result,
    /// postprocessed.
    pub fn role(&self, name: &str) -> Option<Role> {
        if self.dimensions.contains(name) {
            Some(Role::Dimension)
        } else if self.inputs.contains(name) {
            Some(Role::Input)
        } else if self.results.contains(name) {
            Some(Role::Result)
        } else if self.postprocessed.contains(name) {
            Some(Role::Postprocessed)
        } else {
            None
        }
    }

    /// True if `name` is any known component.
    pub fn contains(&self, name: &str) -> bool {
        self.role(name).is_some()
    }
}

/// The names for `where` strings and for expressions, as `CalliopeBuildMath.parsing_components`
/// defines them, plus the postprocessed names `_add_postprocessed` adds.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NameContext {
    /// Names valid in `where` strings (inactive inputs and results included).
    pub where_: Names,
    /// Names valid in expressions (active components only).
    pub expression: Names,
    /// Postprocessed names valid in postprocessed `where` strings (all).
    pub postprocessed_where: BTreeSet<String>,
    /// Postprocessed names valid in postprocessed expressions (active only).
    pub postprocessed_expression: BTreeSet<String>,
}

impl NameContext {
    /// The names Calliope uses to build `math`.
    pub fn from_math(math: &CalliopeBuildMath) -> Self {
        let pc = math.parsing_components();
        Self {
            where_: Names::from_groups(&pc.where_),
            expression: Names::from_groups(&pc.expression),
            postprocessed_where: math
                .all_names("postprocessed")
                .into_iter()
                .map(str::to_owned)
                .collect(),
            postprocessed_expression: math
                .active_names("postprocessed")
                .into_iter()
                .map(str::to_owned)
                .collect(),
        }
    }

    /// The names `pre_validate_math_strings` uses: all names, active or not, for both
    /// languages (`model_math._validate_math_string_parsing`).
    pub fn all_from_math(math: &CalliopeBuildMath) -> Self {
        let set = |groups: &[&str]| -> BTreeSet<String> {
            groups
                .iter()
                .flat_map(|g| math.all_names(g))
                .map(str::to_owned)
                .collect()
        };
        let names = Names {
            dimensions: set(&["dimensions"]),
            inputs: set(&["parameters", "lookups"]),
            results: set(&["variables", "global_expressions"]),
            postprocessed: BTreeSet::new(),
        };
        Self {
            where_: names.clone(),
            expression: names,
            postprocessed_where: BTreeSet::new(),
            postprocessed_expression: BTreeSet::new(),
        }
    }

    /// The names for one string.
    pub fn names(&self, lang: crate::ast::Lang, postprocessed: bool) -> Names {
        let mut n = match lang {
            crate::ast::Lang::Where => self.where_.clone(),
            crate::ast::Lang::Expression => self.expression.clone(),
        };
        if postprocessed {
            n.postprocessed = match lang {
                crate::ast::Lang::Where => self.postprocessed_where.clone(),
                crate::ast::Lang::Expression => self.postprocessed_expression.clone(),
            };
        }
        n
    }
}
