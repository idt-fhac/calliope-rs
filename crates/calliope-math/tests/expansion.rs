//! Equation expansion and naming against Calliope (`tests/oracle/expansion_oracle.py`), and
//! `pre_validate_math_strings` on all math.

mod common;

use std::collections::BTreeMap;

use calliope_math::{Group, ParseMode, parse_math};
use serde::Deserialize;

#[derive(Deserialize)]
struct Set {
    files: Vec<String>,
    components: Vec<Component>,
}

#[derive(Deserialize)]
struct Component {
    group: String,
    name: String,
    equations: Vec<(String, usize)>,
}

fn normalise(name: &str) -> String {
    let mut parts = name.split('-');
    let head = parts.next().unwrap().to_owned();
    let mut rest: Vec<&str> = parts.collect();
    rest.sort_unstable();
    std::iter::once(head.as_str())
        .chain(rest)
        .collect::<Vec<_>>()
        .join("-")
}

#[test]
fn expansion_matches_calliope() {
    let fixture: BTreeMap<String, Set> =
        serde_json::from_str(include_str!("fixtures/expansion.json")).unwrap();
    let mut checked = 0;
    for (set_name, set) in &fixture {
        let Some(math) = common::layered(&set.files) else {
            return;
        };
        let (components, diagnostics) = parse_math(&math, ParseMode::Build);
        assert!(
            diagnostics.is_empty(),
            "{set_name}: {:?}",
            diagnostics.iter().map(|d| d.short()).collect::<Vec<_>>()
        );
        for expected in &set.components {
            let comp = components
                .iter()
                .find(|c| c.group.as_str() == expected.group && c.name == expected.name)
                .unwrap_or_else(|| {
                    panic!("{set_name}: {}.{} missing", expected.group, expected.name)
                });
            let mut got: Vec<(String, usize)> = comp
                .equations
                .iter()
                .map(|e| (normalise(&e.name), e.wheres.len()))
                .collect();
            got.sort();
            assert_eq!(
                got, expected.equations,
                "{set_name}: {}.{}",
                expected.group, expected.name
            );
            checked += got.len();
        }
        let n_with_equations = components
            .iter()
            .filter(|c| {
                matches!(
                    c.group,
                    Group::GlobalExpressions
                        | Group::Constraints
                        | Group::Objectives
                        | Group::Postprocessed
                )
            })
            .count();
        assert_eq!(n_with_equations, set.components.len(), "{set_name}");
    }
    assert!(checked > 1000, "only {checked} equations checked");
}

#[test]
fn pre_validate_all_math() {
    for (name, files) in common::math_sets() {
        let math = common::layered(&files).unwrap();
        let (_, diagnostics) = parse_math(&math, ParseMode::PreValidate);
        assert!(
            diagnostics.is_empty(),
            "{name}: {:?}",
            diagnostics.iter().map(|d| d.short()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn global_expressions_in_build_order() {
    let Some(math) = common::layered(&["src/calliope/math/base.yaml".to_owned()]) else {
        return;
    };
    let (components, _) = parse_math(&math, ParseMode::Build);
    let groups: Vec<Group> = components.iter().map(|c| c.group).collect();
    let mut sorted = groups.clone();
    sorted.sort();
    assert_eq!(groups, sorted, "components come in build order");
    let mut last = i64::MIN;
    for c in components
        .iter()
        .filter(|c| c.group == Group::GlobalExpressions)
    {
        let order = math.global_expressions[c.name.as_str()].order;
        assert!(order >= last);
        last = order;
    }
}

fn math_from(yaml: &str) -> calliope_schema::CalliopeBuildMath {
    let tree = calliope_yaml::read_yaml_str(yaml, false).unwrap();
    calliope_schema::Validated::<calliope_schema::CalliopeBuildMath>::from_tree(tree)
        .unwrap()
        .into_inner()
}

const SMALL: &str = r#"
dimensions:
  techs: {iterator: tech}
parameters:
  p: {}
variables:
  x: {foreach: [techs]}
constraints:
  c:
    foreach: [techs]
    equations:
      - expression: $a + $b * x[techs=$s] <= p
      - where: p
        expression: x == 0
    sub_expressions:
      a:
        - expression: x
        - where: p>0
          expression: 2 * x
      b:
        - expression: p
    slices:
      s:
        - expression: foo
        - expression: bar
"#;

#[test]
fn expansion_naming_and_wheres() {
    let (components, diagnostics) = parse_math(&math_from(SMALL), ParseMode::Build);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    let c = components.iter().find(|c| c.name == "c").unwrap();
    let names: Vec<&str> = c.equations.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "constraints:c:0-a:0-b:0-s:0",
            "constraints:c:0-a:0-b:0-s:1",
            "constraints:c:0-a:1-b:0-s:0",
            "constraints:c:0-a:1-b:0-s:1",
            "constraints:c:1",
        ]
    );
    let e = &c.equations[2];
    assert_eq!(e.wheres.len(), 4);
    assert_eq!(e.wheres[1].source, "p>0");
    assert_eq!(
        e.sub_expressions["a"].node.to_string(),
        "(NUM:2 * COMPONENT:x)"
    );
    assert_eq!(e.slices["s"].node.to_string(), "STRING:foo");
}

#[test]
fn undefined_reference() {
    let yaml = SMALL.replace("$a + ", "$a + $nope + ");
    let (components, diagnostics) = parse_math(&math_from(&yaml), ParseMode::Build);
    assert!(components.iter().all(|c| c.name != "c"));
    assert_eq!(diagnostics.len(), 1);
    let d = &diagnostics[0];
    assert_eq!(d.code, "math::undefined-reference");
    assert!(
        d.message
            .contains("constraints:c: Undefined sub_expressions found in equation: {'nope'}"),
        "{}",
        d.message
    );
    assert_eq!(&d.source[d.span.clone()], "$nope");
}

#[test]
fn parse_errors_are_located() {
    let yaml = SMALL.replace("x == 0", "x === 0");
    let (_, diagnostics) = parse_math(&math_from(&yaml), ParseMode::Build);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].path.to_string(),
        "constraints.c.equations[1].expression"
    );
    assert_eq!(diagnostics[0].span.start, 4);
}
