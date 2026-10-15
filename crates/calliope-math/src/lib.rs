//! Parsing, type-checking and compiling Calliope math strings.
//!
//! - [`parse`]: a pest grammar for `where` strings and expressions with the same syntax,
//!   precedence and accepted strings as Calliope's pyparsing grammar.
//! - [`ast`]: the parse tree; its `Display` is the pyparsing `repr`.
//! - [`component`]: whole components, with equations expanded by sub-expressions and slices
//!   and named as Calliope names them.
//! - [`diagnostic`]: findings with the math string, its location and a caret (miette).

pub mod ast;
pub mod component;
pub mod diagnostic;
pub mod helpers;
pub mod names;
pub mod parse;

pub use ast::{Lang, Node, NodeKind, Role, Span};
pub use component::{Equation, Group, ParseMode, ParsedComponent, parse_math};
pub use diagnostic::{MathDiagnostic, StringPath};
pub use names::{NameContext, Names};
pub use parse::{ParseError, StringKind, parse};
