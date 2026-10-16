//! `calliope new`: copies an embedded example model.

use std::path::Path;

use include_dir::{Dir, include_dir};

use crate::error::{CliError, CliResult};

static EXAMPLES: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/examples");

/// Names of the embedded example models.
pub fn templates() -> Vec<String> {
    EXAMPLES
        .dirs()
        .filter_map(|d| d.path().file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .collect()
}

/// `shutil.copytree(example, path)`: the target must not exist.
pub fn new_model(template: &str, path: &Path) -> CliResult<()> {
    let dir = EXAMPLES.get_dir(template).ok_or_else(|| {
        CliError::new(format!(
            "no example model named '{template}' (available: {})",
            templates().join(", ")
        ))
    })?;
    if path.exists() {
        return Err(CliError::new(format!(
            "[Errno 17] File exists: '{}'",
            path.display()
        )));
    }
    write_dir(dir, path)
}

fn write_dir(dir: &Dir<'_>, target: &Path) -> CliResult<()> {
    std::fs::create_dir_all(target)?;
    for f in dir.files() {
        let name = f.path().file_name().unwrap_or_default();
        std::fs::write(target.join(name), f.contents())?;
    }
    for d in dir.dirs() {
        let name = d.path().file_name().unwrap_or_default();
        write_dir(d, &target.join(name))?;
    }
    Ok(())
}
