//! Shared test helpers.
#![allow(dead_code)]

use std::path::PathBuf;

use calliope_yaml::{AttrTree, read_yaml_file, read_yaml_str};

/// The Calliope reference checkout (`$CALLIOPE_REFERENCE`, default `../calliope`), if present.
pub fn reference() -> Option<PathBuf> {
    let root = std::env::var("CALLIOPE_REFERENCE").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        PathBuf::from,
    );
    if root.join("src/calliope").is_dir() {
        Some(root)
    } else {
        eprintln!(
            "skipping: no Calliope reference checkout at {}",
            root.display()
        );
        None
    }
}

/// A math file from the reference checkout (`base`, `operate`, ...).
pub fn math_file(name: &str) -> Option<AttrTree> {
    let path = reference()?.join(format!("src/calliope/math/{name}.yaml"));
    Some(read_yaml_file(path, false).unwrap())
}

/// Parses YAML text.
pub fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}
