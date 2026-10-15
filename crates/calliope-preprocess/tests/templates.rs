//! Port of `TestTemplateSolver` in `tests/preprocess/model_definition_test.py`.

use calliope_preprocess::TemplateSolver;
use calliope_yaml::{AttrTree, read_yaml_str};

fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}

fn dummy_solved_template() -> TemplateSolver {
    TemplateSolver::new(&yaml(
        r#"
        templates:
            T1:
                A: ["foo", "bar"]
                B: 1
            T2:
                C: bar
                template: T1
            T3:
                template: T1
                B: 11
            T4:
                template: T3
                A: ["bar", "foobar"]
                B: "1"
                C: {"foo": "bar"}
                D: true
        a:
            template: T1
            a1: 1
        b:
            template: T3
        c:
            template: T4
            D: false
        "#,
    ))
    .unwrap()
}

#[test]
fn inheritance_templates() {
    let t = dummy_solved_template().resolved_templates;
    assert_eq!(t["T1"], yaml("{A: [foo, bar], B: 1}"));
    assert_eq!(t["T2"], yaml("{A: [foo, bar], B: 1, C: bar}"));
    assert_eq!(t["T3"], yaml("{A: [foo, bar], B: 11}"));
    assert_eq!(
        t["T4"],
        yaml("{A: [bar, foobar], B: '1', C: {foo: bar}, D: true}")
    );
}

#[test]
fn template_inheritance_data() {
    let d = dummy_solved_template().resolved_data;
    let get = |k: &str| d.get(k).unwrap().as_map().unwrap().clone();
    assert_eq!(get("a"), yaml("{A: [foo, bar], B: 1, a1: 1}"));
    assert_eq!(get("b"), yaml("{A: [foo, bar], B: 11}"));
    assert_eq!(
        get("c"),
        yaml("{A: [bar, foobar], B: '1', C: {foo: bar}, D: false}")
    );
    assert!(!d.contains_key("templates"));
}

#[test]
fn invalid_template_error() {
    let err = TemplateSolver::new(&yaml(
        "
            templates:
                T1: \"not_a_yaml_block\"
                T2:
                    foo: bar
            a:
                template: T2
            ",
    ))
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Template definitions must be YAML blocks.")
    );
    assert!(err.span.is_some());
}

#[test]
fn circular_template_error() {
    let err = TemplateSolver::new(&yaml(
        "
            templates:
                T1:
                    template: T2
                    bar: foo
                T2:
                    template: T1
                    foo: bar
            a:
                template: T2
            ",
    ))
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Circular template reference detected")
    );
}

#[test]
fn incorrect_template_placement_error() {
    let err = TemplateSolver::new(&yaml(
        "
            templates:
                T1:
                    stuff: null
                T2:
                    foo: bar
            a:
                template: T2
            b:
                templates:
                    T3:
                        this: \"should not be here\"
            ",
    ))
    .unwrap_err();
    assert!(
        err.to_string()
            .contains("Template definitions must be placed at the top level of the YAML file.")
    );
}

#[test]
fn templates_apply_in_nested_sections_and_lists_are_untouched() {
    let d = TemplateSolver::new(&yaml(
        "
templates:
  cost: {cost_flow_cap: {data: 1, index: monetary, dims: costs}}
techs:
  pv: {template: cost, base_tech: supply}
  other: {items: [{template: cost}]}
",
    ))
    .unwrap()
    .resolved_data;
    assert!(d.get_key("techs.pv.cost_flow_cap.data").is_ok());
    let items = d.get_key("techs.other.items").unwrap().as_seq().unwrap();
    assert!(items[0].as_map().unwrap().contains_key("template"));
}

#[test]
fn undefined_template_is_an_error() {
    let err = TemplateSolver::new(&yaml("a: {template: nope}")).unwrap_err();
    assert!(err.to_string().contains("`nope`"), "{err}");
}
