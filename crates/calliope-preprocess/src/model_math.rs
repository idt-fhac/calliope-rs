//! Math loading (`preprocess/model_math.py`). The pre-defined math files are vendored in
//! `math/` (copied from Calliope 0.7.0 @ 088eb7e4) and compiled into the crate.

use std::path::Path;

use calliope_schema::{CalliopeBuildMath, Init, MathPaths, Mode, Validated};
use calliope_yaml::{AttrTree, read_yaml_file, read_yaml_str_at, relative_path};
use indexmap::IndexMap;

use crate::error::ModelError;

/// The pre-defined math files, by name, in alphabetical order.
pub const PRE_DEFINED_MATH: [(&str, &str); 5] = [
    ("base", include_str!("../math/base.yaml")),
    ("milp", include_str!("../math/milp.yaml")),
    ("operate", include_str!("../math/operate.yaml")),
    ("spores", include_str!("../math/spores.yaml")),
    (
        "storage_inter_cluster",
        include_str!("../math/storage_inter_cluster.yaml"),
    ),
];

/// `initialise_math`: loads the pre-defined math and the user's `math_paths` (relative to
/// the model definition). User files may replace pre-defined ones (with a warning).
pub fn initialise_math(
    extra_math: &MathPaths,
    model_def_path: Option<&Path>,
) -> Result<IndexMap<String, AttrTree>, ModelError> {
    let mut out = IndexMap::new();
    for (name, text) in PRE_DEFINED_MATH {
        let tree = if extra_math.contains_key(name) {
            AttrTree::new() // replaced below, keeping its position
        } else {
            read_yaml_str_at(text, format!("<calliope math>/{name}.yaml"), false)
                .map_err(|e| ModelError::new(e.to_string()))?
        };
        out.insert(name.to_owned(), tree);
    }
    for (name, path) in extra_math {
        if out.contains_key(name.as_str()) {
            tracing::warn!(
                "Math init | Overwriting pre-defined '{name}' math with {}.",
                path.display()
            );
        }
        let full = relative_path(model_def_path, path);
        let tree = read_yaml_file(&full, false).map_err(|e| ModelError::new(e.to_string()))?;
        out.insert(name.to_string(), tree);
    }
    Ok(out)
}

/// `get_math_priority`: base, then the mode (if not `base`), then `extra_math`.
pub fn get_math_priority(init: &Init) -> Vec<String> {
    let mut names = vec!["base".to_owned()];
    match init.mode {
        Mode::Base => {}
        Mode::Operate => names.push("operate".to_owned()),
        Mode::Spores => names.push("spores".to_owned()),
    }
    names.extend(init.extra_math.iter().cloned());
    names
}

/// `build_math`: applies the named math in priority order, then `overwrite`. Math string
/// validation (`validate=True`) needs the parser in `calliope-math` and is done there.
pub fn build_math(
    priority: &[String],
    math_init: &IndexMap<String, AttrTree>,
    overwrite: Option<&AttrTree>,
) -> Result<Validated<CalliopeBuildMath>, ModelError> {
    tracing::info!("Math build | building applied math with {priority:?}.");
    let mut math = Validated::<CalliopeBuildMath>::default_model()?;
    for name in priority {
        let Some(m) = math_init.get(name) else {
            return Err(ModelError::new(format!(
                "Requested math '{name}' was not initialised."
            )));
        };
        math = math.update(m, true)?;
    }
    if let Some(o) = overwrite {
        tracing::info!("Math build | appending additional math.");
        math = math.update(o, true)?;
    }
    Ok(math)
}
