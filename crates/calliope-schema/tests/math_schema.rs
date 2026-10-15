//! Port of `tests/schemas/math_schema_test.py`, plus layering of every pre-defined math file.

mod common;

use calliope_schema::{
    CalliopeBuildMath, ComponentRef, FindError, Lookup, MathEquationComponent, Parameter,
    Validated, Variable,
};
use calliope_yaml::{AttrTree, Node, Value, to_node};
use common::{math_file, reference, yaml};

#[test]
fn global_expression_order() {
    let math = yaml(
        "
global_expressions:
  expr0: {equations: [{expression: x * y}]}
  expr1: {equations: [{expression: x + y}], order: 2}
  expr2: {equations: [{expression: x - y}], order: 1}
  expr3: {equations: [{expression: x / y}], order: 3}
",
    );
    let m = Validated::<CalliopeBuildMath>::from_tree(math).unwrap();
    let order: Vec<i64> = m.global_expressions.values().map(|g| g.order).collect();
    assert_eq!(order, [0, 2, 1, 3]);
}

fn dummy() -> AttrTree {
    yaml(
        r#"
title: foo
description: A dummy equation
active: true
equations:
  - where: "True"
    expression: something == $sub_bar
sub_expressions:
  bar:
    - where: "True"
      expression: something_else + 3
slices:
  cake:
    - where: "True"
      expression: has_cake
"#,
    )
}

#[test]
fn equation_component_correctness() {
    let m = Validated::<MathEquationComponent>::from_tree(dummy()).unwrap();
    assert_eq!(m.dump(), dummy());
}

#[test]
fn equation_component_active_fail_if_empty() {
    let mut d = dummy();
    d.insert("equations", Node::new(Value::Seq(vec![])));
    let err = Validated::<MathEquationComponent>::from_tree(d).unwrap_err();
    assert!(
        err.to_string()
            .contains("Must have equations defined if component is active."),
        "{err}"
    );
}

#[test]
fn equation_component_inactive_empty_pass() {
    let mut d = dummy();
    d.insert("equations", Node::new(Value::Seq(vec![])));
    d.insert("active", false);
    Validated::<MathEquationComponent>::from_tree(d).unwrap();
}

fn default_dump<T: serde::Serialize + Default>() -> Node {
    to_node(&T::default()).unwrap()
}

/// The base math with the edge cases of the Python `base_math_validated` fixture.
fn base_math_validated() -> Option<Validated<CalliopeBuildMath>> {
    let mut raw = math_file("base")?;
    raw.set_key("variables.foo", default_dump::<Variable>())
        .unwrap();
    raw.set_key("parameters.foo", default_dump::<Parameter>())
        .unwrap();
    raw.set_key("lookups.foo", default_dump::<Lookup>())
        .unwrap();
    raw.set_key("parameters.foo.active", false.into()).unwrap();
    raw.set_key("lookups.foo.active", false.into()).unwrap();
    raw.set_key("lookups.bar", default_dump::<Lookup>())
        .unwrap();
    raw.set_key("lookups.bar.active", false.into()).unwrap();
    raw.set_key("parameters.flow_cap", default_dump::<Parameter>())
        .unwrap();
    raw.set_key("variables.flow_cap.active", false.into())
        .unwrap();
    raw.set_key("global_expressions.flow_in_inc_eff.active", false.into())
        .unwrap();
    raw.set_key("dimensions.datesteps.active", false.into())
        .unwrap();
    Some(Validated::from_tree(raw).unwrap())
}

#[test]
fn base_math_pass() {
    let Some(raw) = math_file("base") else { return };
    Validated::<CalliopeBuildMath>::from_tree(raw).unwrap();
}

#[test]
fn duplicate_component_error() {
    let Some(mut raw) = math_file("base") else {
        return;
    };
    raw.set_key("variables.techs", default_dump::<Variable>())
        .unwrap();
    raw.set_key(
        "dimensions.flow_cap",
        default_dump::<calliope_schema::Dimension>(),
    )
    .unwrap();
    let err = Validated::<CalliopeBuildMath>::from_tree(raw).unwrap_err();
    assert!(
        err.to_string()
            .contains("Non-unique names in math components: ['flow_cap', 'techs']"),
        "{err}"
    );
}

const SEARCH_GROUP_ARGS: [(&str, &str); 9] = [
    ("foo", "variables"),
    ("flow_out", "variables"),
    ("flow_cap", "parameters"),
    ("bigM", "parameters"),
    ("timesteps", "dimensions"),
    ("base_tech", "lookups"),
    ("force_zero_area_use", "constraints"),
    ("min_cost_optimisation", "objectives"),
    ("cost_operation_variable", "global_expressions"),
];

fn group_of(c: ComponentRef<'_>) -> &'static str {
    match c {
        ComponentRef::Dimension(_) => "dimensions",
        ComponentRef::Parameter(_) => "parameters",
        ComponentRef::Lookup(_) => "lookups",
        ComponentRef::Variable(_) => "variables",
        ComponentRef::GlobalExpression(_) => "global_expressions",
        ComponentRef::Constraint(_) => "constraints",
        ComponentRef::PiecewiseConstraint(_) => "piecewise_constraints",
        ComponentRef::Objective(_) => "objectives",
        ComponentRef::Postprocessed(_) => "postprocessed",
        ComponentRef::Check(_) => "checks",
    }
}

#[test]
fn find() {
    let Some(m) = base_math_validated() else {
        return;
    };
    for (name, group) in SEARCH_GROUP_ARGS {
        let found = m.find(name, None).unwrap();
        assert_eq!(group_of(found), group, "{name}");
        match found {
            ComponentRef::Variable(v) => assert_eq!(v, &m.variables[name]),
            ComponentRef::Parameter(p) => assert_eq!(p, &m.parameters[name]),
            _ => {}
        }
    }
}

#[test]
fn find_duplicate_error() {
    let Some(m) = base_math_validated() else {
        return;
    };
    // Python injects a duplicate into the cached `_active` dict; here we activate the lookup.
    let mut raw = m.raw().clone();
    raw.set_key("lookups.foo.active", true.into()).unwrap();
    let mut b = m.get().clone();
    b.lookups = calliope_yaml::from_tree::<CalliopeBuildMath>(&raw)
        .unwrap()
        .lookups;
    assert!(matches!(
        b.find("foo", None),
        Err(FindError::Multiple { .. })
    ));
    assert!(
        b.find("foo", None)
            .unwrap_err()
            .to_string()
            .contains("found in multiple")
    );
}

#[test]
fn find_not_found_error() {
    let Some(m) = base_math_validated() else {
        return;
    };
    let cases: [(&str, Option<&[&str]>); 6] = [
        ("flow_in_inc_eff", None),
        ("datesteps", None),
        ("non_existent", None),
        ("3", None),
        (
            "foo",
            Some(&["parameters", "lookups", "global_expressions"]),
        ),
        ("flow_cap", Some(&["variables", "objectives"])),
    ];
    for (name, subset) in cases {
        let err = m.find(name, subset).unwrap_err();
        assert_eq!(
            err.to_string(),
            format!("Component name `{name}` not found in math schema."),
        );
    }
}

#[test]
fn in_active_dict() {
    let Some(m) = base_math_validated() else {
        return;
    };
    for (group, is_active) in [
        ("variables", true),
        ("parameters", false),
        ("lookups", false),
    ] {
        assert_eq!(m.active_names(group).contains(&"foo"), is_active, "{group}");
    }
}

#[test]
fn where_components() {
    let Some(m) = base_math_validated() else {
        return;
    };
    let pc = m.parsing_components();
    let w = &pc.where_;
    assert!(w.dimensions.contains("techs"));
    assert!(!w.dimensions.contains("datesteps"));
    assert!(w.inputs.contains("flow_cap"));
    assert!(w.inputs.contains("bar"));
    assert!(!w.inputs.contains("foo"));
    assert!(w.results.contains("foo"));
    assert!(w.results.contains("flow_in_inc_eff"));
    assert!(!w.results.contains("flow_cap"));
    let e = &pc.expression;
    let all: Vec<&String> = e
        .dimensions
        .iter()
        .chain(&e.inputs)
        .chain(&e.results)
        .collect();
    for c in ["techs", "foo", "flow_cap"] {
        assert!(all.iter().any(|n| *n == c), "{c}");
    }
    for c in ["flow_in_inc_eff", "datesteps"] {
        assert!(!all.iter().any(|n| *n == c), "{c}");
    }
}

/// `build_math`: every pre-defined math file layered on top of `base` validates.
#[test]
fn layered_math_validates() {
    let Some(root) = reference() else { return };
    let base = Validated::<CalliopeBuildMath>::from_tree(math_file("base").unwrap()).unwrap();
    let mut n = 0;
    for entry in std::fs::read_dir(root.join("src/calliope/math")).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_stem().unwrap().to_str().unwrap().to_owned();
        if name == "base" {
            continue;
        }
        let extra = math_file(&name).unwrap();
        let layered = base
            .update(&extra, true)
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        n += 1;
        // Layering replaces equation lists wholesale and keeps untouched components.
        assert!(
            layered.constraints.len() >= base.constraints.len() || name == "operate",
            "{name}"
        );
    }
    assert!(n >= 4, "only {n} math files");
}

#[test]
fn operate_math_turns_variables_into_parameters() {
    let (Some(base), Some(operate)) = (math_file("base"), math_file("operate")) else {
        return;
    };
    let base = Validated::<CalliopeBuildMath>::from_tree(base).unwrap();
    let layered = base.update(&operate, true).unwrap();
    assert!(!layered.variables["flow_cap"].active);
    assert!(layered.parameters.contains_key("flow_cap"));
    // Only what `operate.yaml` sets is changed on the variable.
    assert_eq!(
        layered.variables["flow_cap"].foreach,
        base.variables["flow_cap"].foreach
    );
}
