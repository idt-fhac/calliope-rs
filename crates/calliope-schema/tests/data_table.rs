//! Port of `tests/schemas/data_table_schema_test.py`.

mod common;

use calliope_schema::{CalliopeDataTable, Validated};
use calliope_yaml::{AttrTree, Node};
use common::yaml;

fn full_data_table_config() -> AttrTree {
    yaml(
        "
    table: time_varying_df
    rows: timesteps
    columns: [comment, nodes, techs]
    select:
        nodes: [node1, node2]
        techs: pv
    drop: comment
    add_dims:
        inputs: something
        costs: monetary
    rename_dims:
        location: nodes
    ",
    )
}

fn validate(t: AttrTree) -> Result<Validated<CalliopeDataTable>, String> {
    Validated::from_tree(t).map_err(|e| e.to_string())
}

#[test]
fn path_not_provided() {
    for t in [
        "{rows: timesteps}",
        "{rows: timesteps, columns: [techs, nodes]}",
    ] {
        let err = validate(yaml(t)).unwrap_err();
        assert!(err.contains("validation error for Data table"), "{err}");
        assert!(err.contains("missing field `table`"), "{err}");
    }
}

#[test]
fn incomplete_column_or_row() {
    let err = validate(yaml("{table: foo}")).unwrap_err();
    assert!(
        err.contains("Either row or columns must be defined (and not dropped)."),
        "{err}"
    );
}

#[test]
fn row_column_overlap() {
    for (rows, columns) in [
        ("nodes", "nodes"),
        ("[nodes, techs]", "techs"),
        ("[nodes, techs, params]", "[params, costs]"),
    ] {
        let err = validate(yaml(&format!(
            "{{table: foobar, rows: {rows}, columns: {columns}}}"
        )))
        .unwrap_err();
        assert!(err.contains("Rows and columns must not overlap."), "{err}");
    }
}

#[test]
fn add_dims_overlap() {
    let err = validate(yaml("{table: foo, rows: nodes, add_dims: {nodes: MEX}}")).unwrap_err();
    assert!(
        err.contains("Added dimensions must not be in columns or rows."),
        "{err}"
    );
}

#[test]
fn full_table_config() {
    let t = validate(full_data_table_config()).unwrap();
    assert_eq!(t.table, "time_varying_df");
    // The given form (single name vs list) is kept.
    assert_eq!(t.dump().get("rows").unwrap().as_str(), Some("timesteps"));
}

#[test]
fn invalid_rename() {
    for (rename, drop) in [("{bar: foo}", "null"), ("{locations: nodes}", "nodes")] {
        let mut t = full_data_table_config();
        t.insert(
            "drop",
            yaml(&format!("v: {drop}")).get("v").unwrap().clone(),
        );
        t.insert("rename_dims", Node::from(yaml(rename)));
        let err = validate(t).unwrap_err();
        assert!(
            err.contains("Renamed dimensions must be in either rows or columns."),
            "{err}"
        );
    }
}

#[test]
fn unknown_key_rejected() {
    let err = validate(yaml("{table: foo, rows: nodes, template: x}")).unwrap_err();
    assert!(err.contains("unknown field `template`"), "{err}");
}
