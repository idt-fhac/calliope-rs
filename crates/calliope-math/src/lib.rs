//! Parsing, type-checking and compiling Calliope math strings.
//!
//! - [`parse`]: a pest grammar for `where` strings and expressions with the same syntax,
//!   precedence and accepted strings as Calliope's pyparsing grammar.
//! - [`ast`]: the parse tree; its `Display` is the pyparsing `repr`.

pub mod ast;
pub mod names;
pub mod parse;

pub use ast::{Lang, Node, NodeKind, Role, Span};
pub use names::{NameContext, Names};
pub use parse::{ParseError, StringKind, parse};
