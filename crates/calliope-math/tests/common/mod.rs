//! Shared test helpers: loading math from the Calliope reference checkout.
#![allow(dead_code)]

use std::path::PathBuf;

use calliope_schema::{CalliopeBuildMath, Validated};
use calliope_yaml::read_yaml_file;

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

/// Math files (paths relative to the checkout) layered in order, as `build_math` does.
pub fn layered(files: &[String]) -> Option<CalliopeBuildMath> {
    let root = reference()?;
    let mut math: Option<Validated<CalliopeBuildMath>> = None;
    for f in files {
        let tree = read_yaml_file(root.join(f), false).unwrap();
        math = Some(match math {
            None => Validated::from_tree(tree).unwrap(),
            Some(m) => m.update(&tree, true).unwrap_or_else(|e| panic!("{f}: {e}")),
        });
    }
    math.map(Validated::into_inner)
}

/// The math sets used by the oracles: name and files.
pub fn math_sets() -> Vec<(String, Vec<String>)> {
    let Some(root) = reference() else {
        return Vec::new();
    };
    let base = "src/calliope/math/base.yaml".to_owned();
    let mut sets = vec![("base".to_owned(), vec![base.clone()])];
    for mode in ["milp", "operate", "spores", "storage_inter_cluster"] {
        sets.push((
            mode.to_owned(),
            vec![base.clone(), format!("src/calliope/math/{mode}.yaml")],
        ));
    }
    sets.push((
        "urban_scale".to_owned(),
        vec![
            base.clone(),
            "src/calliope/example_models/urban_scale/additional_math.yaml".to_owned(),
        ],
    ));
    let mut gallery: Vec<String> = std::fs::read_dir(root.join("docs/examples/math_gallery"))
        .unwrap()
        .filter_map(|e| {
            let p = e.unwrap().path();
            (p.extension()? == "yaml").then(|| p.file_stem().unwrap().to_str().unwrap().to_owned())
        })
        .collect();
    gallery.sort();
    for g in gallery {
        sets.push((
            format!("gallery_{g}"),
            vec![
                base.clone(),
                "src/calliope/math/milp.yaml".to_owned(),
                format!("docs/examples/math_gallery/{g}.yaml"),
            ],
        ));
    }
    sets
}
