//! Calliope-flavoured YAML.
//!
//! - [`AttrTree`]: an ordered nested map with the semantics of Calliope's `AttrDict`
//!   (dotted keys, `union` with `allow_override` and `_REPLACE_`).
//! - [`read_yaml_file`] / [`read_yaml_str`]: `calliope.io.read_rich_yaml`, with `import:`
//!   resolution and cycle detection (Python recurses until it overflows the stack).
//! - Every node keeps the file, line and column it was read from, so errors can point at the
//!   YAML that caused them, even after merging.
//! - [`from_node`]: `serde` deserialisation of typed values from nodes, with error paths.

mod de;
mod emit;
mod error;
mod load;
mod read;
mod tree;
mod value;

pub use de::{DeError, PathError, from_node, from_tree, span_at};
pub use emit::to_yaml;
pub use error::{LoadError, TreeError};
pub use load::{parse_node, parse_tree, resolve_plain};
pub use read::{STRING_SOURCE, read_yaml_file, read_yaml_str, read_yaml_str_at, relative_path};
pub use tree::{AttrTree, NestedKey, REPLACE_KEY};
pub use value::{Node, Source, Span, Value, python_float};
