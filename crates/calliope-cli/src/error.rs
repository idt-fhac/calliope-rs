//! The error type of the command-line interface.

use std::fmt;

/// An error that ends a command with exit code 1. The message is printed to stdout, as the
/// Python CLI does; `--debug` also prints the chain of causes to stderr.
#[derive(Debug)]
pub struct CliError {
    /// What went wrong.
    pub message: String,
    /// Underlying causes, outermost first.
    pub causes: Vec<String>,
}

impl CliError {
    /// An error without causes.
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            causes: Vec::new(),
        }
    }

    /// Wraps any error, keeping its `source` chain.
    pub fn from_error(e: &(dyn std::error::Error + 'static)) -> Self {
        let mut causes = Vec::new();
        let mut cur = e.source();
        while let Some(c) = cur {
            causes.push(c.to_string());
            cur = c.source();
        }
        Self {
            message: e.to_string(),
            causes,
        }
    }
}

impl fmt::Display for CliError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for CliError {}

impl From<std::io::Error> for CliError {
    fn from(e: std::io::Error) -> Self {
        Self::from_error(&e)
    }
}

/// Result alias.
pub type CliResult<T> = Result<T, CliError>;
