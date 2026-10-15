//! Port of `tests/schemas/input_model_schema_test.py`: the example and test model definitions
//! validate against the schemas after templates are applied.

use std::path::PathBuf;

use calliope_preprocess::{prepare_model_definition, prepare_model_definition_with};
use calliope_schema::Backend;
use calliope_yaml::{AttrTree, read_yaml_file};

fn reference() -> Option<PathBuf> {
    let root = std::env::var("CALLIOPE_REFERENCE").map_or_else(
        |_| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        PathBuf::from,
    );
    root.join("src/calliope").is_dir().then_some(root)
}

#[test]
fn prepare_example_models() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let mut paths: Vec<PathBuf> = std::fs::read_dir(root.join("src/calliope/example_models"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .map(|p| p.join("model.yaml"))
        .collect();
    paths.sort();
    paths.push(root.join("tests/example_models/national_scale_from_data_tables/model.yaml"));
    paths.push(root.join("tests/common/test_model/model.yaml"));
    assert!(paths.len() >= 4);
    for path in paths {
        let def = read_yaml_file(&path, false).unwrap();
        let prepared =
            prepare_model_definition_with(&def, None, None, None, Some(&path), &AttrTree::new())
                .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let d = &prepared.definition;
        assert!(
            !d.techs.0.is_empty() || !d.data_tables.0.is_empty(),
            "{}",
            path.display()
        );
        // Existing models name the Pyomo backend (implicitly or explicitly).
        assert!(
            prepared.config.build.backend.falls_back_to_highs()
                || prepared.config.build.backend == Backend::Highs
        );
    }
}

#[test]
fn init_kwargs_override_config() {
    let Some(root) = reference() else { return };
    let path = root.join("src/calliope/example_models/national_scale/model.yaml");
    let def = read_yaml_file(&path, false).unwrap();
    let kwargs =
        calliope_yaml::read_yaml_str("{time_cluster: null, mode: operate}", false).unwrap();
    let prepared = prepare_model_definition(&def, &kwargs).unwrap();
    assert_eq!(prepared.config.init.mode, calliope_schema::Mode::Operate);
    assert!(prepared.config.raw().get_key("init.mode").is_ok());
}
