//! `calliope check`: validate a model definition or a math file without building.
//!
//! Model files run the whole preprocessing (YAML, schema, templates, data tables, math
//! strings), then the static typecheck of the applied math against the input dimensions.
//! Math files are layered on the built-in base math and typechecked.

use std::path::Path;

use calliope_math::{InputSchema, MathDiagnostic, typecheck};
use calliope_preprocess::model_math::{build_math, initialise_math};
use calliope_preprocess::{ReadOptions, read_yaml};
use calliope_yaml::read_yaml_file;
use miette::{GraphicalReportHandler, GraphicalTheme};

use crate::error::{CliError, CliResult};
use crate::generate::read_rich_yaml;

/// What a file holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckKind {
    /// Decide from the top-level keys.
    Auto,
    /// A model definition.
    Model,
    /// A math file.
    Math,
}

const MATH_KEYS: &[&str] = &[
    "variables",
    "constraints",
    "global_expressions",
    "objectives",
    "dimensions",
    "parameters",
    "lookups",
    "piecewise_constraints",
];

/// Guess the kind from top-level keys.
pub fn detect(path: &Path) -> CliResult<CheckKind> {
    let tree = read_yaml_file(path, false).map_err(|e| CliError::from_error(&e))?;
    Ok(if tree.keys().any(|k| MATH_KEYS.contains(&k.as_str())) {
        CheckKind::Math
    } else {
        CheckKind::Model
    })
}

/// Checks one file; returns the typecheck diagnostics. Load/schema errors are `Err`.
pub fn check_file(
    path: &Path,
    kind: CheckKind,
    scenario: Option<&str>,
    override_dict: Option<&str>,
) -> CliResult<Vec<MathDiagnostic>> {
    let kind = if kind == CheckKind::Auto {
        detect(path)?
    } else {
        kind
    };
    match kind {
        CheckKind::Math => {
            let tree = read_yaml_file(path, false).map_err(|e| CliError::from_error(&e))?;
            let init =
                initialise_math(&Default::default(), None).map_err(|e| CliError::from_error(&e))?;
            // User math is layered on base and MILP math (the math gallery needs both);
            // the pre-defined files are checked on top of base only.
            let predefined = path
                .file_stem()
                .is_some_and(|s| init.contains_key(s.to_string_lossy().as_ref()));
            let mut layers = vec!["base".to_owned()];
            if !predefined {
                layers.push("milp".to_owned());
            }
            let math =
                build_math(&layers, &init, Some(&tree)).map_err(|e| CliError::from_error(&e))?;
            Ok(typecheck(&math, None).diagnostics)
        }
        _ => {
            let opts = ReadOptions {
                scenario: scenario.map(str::to_owned),
                override_dict: override_dict.map(read_rich_yaml).transpose()?,
                ..ReadOptions::default()
            };
            let m = read_yaml(path, &opts).map_err(|e| CliError::from_error(&e))?;
            for w in &m.warnings {
                tracing::warn!("{w}");
            }
            let schema = InputSchema {
                dims: m
                    .inputs
                    .vars
                    .iter()
                    .map(|(k, v)| (k.clone(), v.dims.clone()))
                    .collect(),
            };
            Ok(typecheck(&m.math, Some(&schema)).diagnostics)
        }
    }
}

/// Renders a diagnostic with miette.
pub fn render(d: &MathDiagnostic) -> String {
    let mut out = String::new();
    let handler = GraphicalReportHandler::new_themed(GraphicalTheme::unicode_nocolor());
    if handler.render_report(&mut out, d).is_err() {
        return d.short();
    }
    out
}
