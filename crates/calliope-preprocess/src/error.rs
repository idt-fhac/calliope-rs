//! `calliope.exceptions.ModelError`.

use calliope_yaml::Span;

/// An error in the user's model definition.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}{}", span.as_ref().map_or_else(String::new, |s| format!(" (at {s})")))]
pub struct ModelError {
    /// What is wrong, worded as in Calliope.
    pub message: String,
    /// Where in the YAML, if known.
    pub span: Option<Span>,
}

impl ModelError {
    /// An error without location.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            span: None,
        }
    }

    /// An error at a location.
    pub fn at(message: impl Into<String>, span: Option<Span>) -> Self {
        Self {
            message: message.into(),
            span,
        }
    }
}

impl From<calliope_yaml::TreeError> for ModelError {
    fn from(e: calliope_yaml::TreeError) -> Self {
        ModelError::new(e.to_string())
    }
}

impl From<calliope_schema::SchemaError> for ModelError {
    fn from(e: calliope_schema::SchemaError) -> Self {
        ModelError::new(e.to_string())
    }
}
