//! Validation errors, worded like pydantic's (`N validation error(s) for <title>`).

use std::fmt;

use calliope_yaml::{PathError, Span};

/// One failed check.
#[derive(Debug, Clone, PartialEq)]
pub struct FieldError {
    /// Dotted path of the failing value (empty for the model itself).
    pub path: String,
    /// What is wrong.
    pub message: String,
    /// Where the failing value was defined, if known.
    pub span: Option<Span>,
}

impl FieldError {
    /// A model-level error at `path`.
    pub fn new(path: &str, message: impl Into<String>) -> Self {
        Self {
            path: path.to_owned(),
            message: message.into(),
            span: None,
        }
    }
}

/// Validation of a schema failed.
#[derive(Debug, Clone, PartialEq)]
pub struct SchemaError {
    /// The title of the model being validated.
    pub title: &'static str,
    /// The individual errors.
    pub errors: Vec<FieldError>,
}

impl SchemaError {
    pub(crate) fn from_path_error(title: &'static str, e: PathError) -> Self {
        Self {
            title,
            errors: vec![FieldError {
                path: e.path,
                message: e.message,
                span: e.span,
            }],
        }
    }

    pub(crate) fn single(title: &'static str, path: &str, message: impl Into<String>) -> Self {
        Self {
            title,
            errors: vec![FieldError::new(path, message)],
        }
    }
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let n = self.errors.len();
        write!(
            f,
            "{n} validation error{} for {}",
            if n == 1 { "" } else { "s" },
            self.title
        )?;
        for e in &self.errors {
            if !e.path.is_empty() {
                write!(f, "\n{}", e.path)?;
            }
            write!(f, "\n  {}", e.message)?;
            if let Some(span) = &e.span {
                write!(f, " (at {span})")?;
            }
        }
        Ok(())
    }
}

impl std::error::Error for SchemaError {}
