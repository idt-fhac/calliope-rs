//! Parsing, type-checking and compiling Calliope math strings.
//!
//! - [`parse`]: a pest grammar for `where` strings and expressions with the same syntax,
//!   precedence and accepted strings as Calliope's pyparsing grammar.
//! - [`ast`]: the parse tree; its `Display` is the pyparsing `repr`.
//! - [`component`]: whole components, with equations expanded by sub-expressions and slices
//!   and named as Calliope names them.
//! - [`ir`]: components lowered for evaluation, with Calliope's implicit rules (defaults,
//!   presence tests, masking) as explicit nodes.
//! - [`typecheck`]: static checks (dimensions, linearity, dtypes, helper use) before building.
//! - [`helpers`]: the helper functions, with their semantics on `LArray`.
//! - [`diagnostic`]: findings with the math string, its location and a caret (miette).

pub mod ast;
pub mod component;
pub mod diagnostic;
pub mod eval;
pub mod helpers;
pub mod ir;
pub mod names;
pub mod parse;
pub mod typecheck;

pub use ast::{Lang, Node, NodeKind, Role, Span};
pub use component::{Equation, Group, ParseMode, ParsedComponent, parse_math};
pub use diagnostic::{MathDiagnostic, StringPath};
pub use names::{NameContext, Names};
pub use parse::{ParseError, StringKind, parse};
pub use typecheck::{InputSchema, TypecheckReport, typecheck};
