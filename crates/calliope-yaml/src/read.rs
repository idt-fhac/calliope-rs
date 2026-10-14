//! `calliope.io.read_rich_yaml`: YAML files and strings with `import:` resolution.

use std::path::{Path, PathBuf};

use crate::error::LoadError;
use crate::load::parse_tree;
use crate::tree::AttrTree;
use crate::value::{Source, Value};

/// Name used for YAML that does not come from a file.
pub const STRING_SOURCE: &str = "<yaml string>";

/// `calliope.util.tools.relative_path`: an absolute `path` is returned as is; a relative one is
/// joined to the directory of `base` (or to `base` itself if it is a directory, or to the
/// current directory if there is no base). The result is not normalised.
pub fn relative_path(base: Option<&Path>, path: &Path) -> PathBuf {
    if path.is_absolute() {
        return path.to_path_buf();
    }
    let base = match base {
        Some(b) => b.to_path_buf(),
        None => std::env::current_dir().unwrap_or_default(),
    };
    let base = if base.is_file() {
        base.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        base
    };
    std::path::absolute(&base).unwrap_or(base).join(path)
}

/// Reads a YAML file and resolves its imports (`read_rich_yaml(path)`).
///
/// `allow_override` applies to the union of this file with its own imports; nested imports
/// are always resolved without overrides, as in Python.
pub fn read_yaml_file(path: impl AsRef<Path>, allow_override: bool) -> Result<AttrTree, LoadError> {
    let mut stack = Vec::new();
    read_file_inner(path.as_ref(), allow_override, &mut stack)
}

/// Reads YAML text (`read_rich_yaml(string)`). Text with an `import:` key is an error,
/// because imports are resolved relative to a file.
pub fn read_yaml_str(text: &str, allow_override: bool) -> Result<AttrTree, LoadError> {
    let tree = parse_tree(text, &Source::new(STRING_SOURCE))?;
    let mut stack = Vec::new();
    resolve_imports(tree, None, allow_override, &mut stack)
}

/// Reads YAML text as if it were the file `path` (imports are resolved relative to it).
pub fn read_yaml_str_at(
    text: &str,
    path: impl AsRef<Path>,
    allow_override: bool,
) -> Result<AttrTree, LoadError> {
    let path = path.as_ref();
    let tree = parse_tree(text, &Source::new(path.display().to_string()))?;
    let mut stack = vec![identity(path)];
    resolve_imports(tree, Some(path), allow_override, &mut stack)
}

fn identity(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

fn read_file_inner(
    path: &Path,
    allow_override: bool,
    stack: &mut Vec<PathBuf>,
) -> Result<AttrTree, LoadError> {
    let id = identity(path);
    if stack.contains(&id) {
        let mut chain = stack.clone();
        chain.push(id);
        return Err(LoadError::ImportCycle { chain });
    }
    let text = std::fs::read_to_string(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let tree = parse_tree(&text, &Source::new(path.display().to_string()))?;
    stack.push(id);
    let result = resolve_imports(tree, Some(path), allow_override, stack);
    stack.pop();
    result
}

/// `calliope.io._resolve_yaml_imports`. With imports `[a, b]` the result is
/// `b ∪ (a ∪ loaded)`: the importing file wins over `a`, which wins over `b`.
fn resolve_imports(
    loaded: AttrTree,
    base: Option<&Path>,
    allow_override: bool,
    stack: &mut Vec<PathBuf>,
) -> Result<AttrTree, LoadError> {
    let Some(imports) = loaded.get("import") else {
        return Ok(loaded);
    };
    if !imports.is_truthy() {
        // Python only acts on a truthy `import` value and leaves an empty one in place.
        return Ok(loaded);
    }
    let Value::Seq(items) = &imports.value else {
        return Err(LoadError::ImportNotList {
            span: imports.span.clone(),
        });
    };
    let Some(base) = base else {
        return Err(LoadError::ImportWithoutFile);
    };
    let paths: Vec<PathBuf> = items
        .iter()
        .map(|item| match &item.value {
            Value::Str(s) => Ok(relative_path(Some(base), Path::new(s))),
            other => Err(LoadError::Invalid {
                message: format!("`import` entries must be paths, found {other}"),
                span: item.span.clone().unwrap_or_else(|| crate::Span {
                    source: Source::new(base.display().to_string()),
                    line: 0,
                    col: 0,
                }),
            }),
        })
        .collect::<Result<_, _>>()?;
    let mut merged = loaded;
    for path in paths {
        let mut imported = read_file_inner(&path, false, stack).map_err(|e| match e {
            e @ LoadError::ImportCycle { .. } => e,
            e => LoadError::InImport {
                importer: base.to_path_buf(),
                source: Box::new(e),
            },
        })?;
        imported.union(&merged, allow_override, false)?;
        merged = imported;
    }
    merged.del_key("import")?;
    Ok(merged)
}
