//! Error types.

use std::fmt;
use std::path::PathBuf;

use crate::value::Span;

fn at(span: &Option<Span>) -> String {
    span.as_ref()
        .map_or_else(String::new, |s| format!(" (at {s})"))
}

fn twice(first: &Option<Span>, second: &Option<Span>) -> String {
    match (first, second) {
        (Some(a), Some(b)) => format!(" (first defined at {a}, again at {b})"),
        (Some(a), None) => format!(" (first defined at {a})"),
        (None, Some(b)) => format!(" (defined again at {b})"),
        (None, None) => String::new(),
    }
}

/// Errors from [`crate::AttrTree`] operations.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum TreeError {
    /// A key (or an intermediate key of a dotted key) does not exist.
    #[error("Key not found: `{key}`")]
    KeyNotFound {
        /// The full dotted key.
        key: String,
    },
    /// A nested key was set or read below a value that is not a mapping.
    #[error("Cannot set nested key on non-dict key: `{key}`{}", at(span))]
    NestedOnNonMap {
        /// The full dotted key.
        key: String,
        /// Location of the non-mapping value.
        span: Option<Span>,
    },
    /// `union` without `allow_override` found a key in both trees.
    #[error("Key defined twice: {key}{}", twice(first, second))]
    DefinedTwice {
        /// The full dotted key.
        key: String,
        /// Where the existing value was defined.
        first: Option<Span>,
        /// Where the overriding value was defined.
        second: Option<Span>,
    },
    /// `key._REPLACE_` was given for a key that does not exist.
    #[error(
        "Cannot replace `{key}` with `_REPLACE_`: the key does not exist{}",
        at(span)
    )]
    ReplaceMissing {
        /// The key that should have been replaced.
        key: String,
        /// Location of the replacement value.
        span: Option<Span>,
    },
}

/// Errors from loading YAML text or files.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    /// The file could not be read.
    #[error("Cannot read YAML file `{path}`: {source}")]
    Io {
        /// The file.
        path: PathBuf,
        /// The underlying error.
        source: std::io::Error,
    },
    /// The YAML could not be parsed.
    #[error("Parser error when reading YAML from {location}: {message}")]
    Syntax {
        /// `file:line:col` of the error.
        location: String,
        /// The parser's message.
        message: String,
    },
    /// The document is not a mapping.
    #[error("Could not parse {source_name} as YAML")]
    NotAMapping {
        /// The file or `<yaml string>`.
        source_name: String,
    },
    /// A mapping key appears twice in the same mapping.
    #[error("found duplicate key \"{key}\" at {second} (original at {first})")]
    DuplicateKey {
        /// The key.
        key: String,
        /// Where it was first defined.
        first: Span,
        /// Where it was defined again.
        second: Span,
    },
    /// A mapping key cannot be turned into a string key.
    #[error("Unsupported mapping key {key} at {span}: keys must be strings or integers")]
    BadKey {
        /// The key as text.
        key: String,
        /// Where the key is.
        span: Span,
    },
    /// An alias refers to an unknown anchor, or a merge key has an invalid value.
    #[error("{message} at {span}")]
    Invalid {
        /// What is wrong.
        message: String,
        /// Where.
        span: Span,
    },
    /// An `AttrDict` rule was violated while building the tree.
    #[error(transparent)]
    Tree(#[from] TreeError),
    /// `import` is not a list.
    #[error("`import` must be a list{}", at(span))]
    ImportNotList {
        /// Location of the `import` value.
        span: Option<Span>,
    },
    /// `import` was used in YAML that was not read from a file.
    #[error("Imports are not possible for non-file yaml inputs.")]
    ImportWithoutFile,
    /// A file imports itself, directly or indirectly.
    #[error("Circular YAML import: {}", DisplayChain(chain))]
    ImportCycle {
        /// The chain of files, ending with the file that closes the cycle.
        chain: Vec<PathBuf>,
    },
    /// An imported file could not be loaded.
    #[error("In file imported from {}: {source}", importer.display())]
    InImport {
        /// The importing file.
        importer: PathBuf,
        /// The error in the imported file.
        source: Box<LoadError>,
    },
}

struct DisplayChain<'a>(&'a [PathBuf]);

impl fmt::Display for DisplayChain<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, p) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str(" -> ")?;
            }
            write!(f, "{}", p.display())?;
        }
        Ok(())
    }
}
