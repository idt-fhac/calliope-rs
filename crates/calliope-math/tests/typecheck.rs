//! Typecheck unit cases: each check fires on a small piece of math.

use calliope_math::{InputSchema, typecheck};

fn math(yaml: &str) -> calliope_schema::CalliopeBuildMath {
    let tree = calliope_yaml::read_yaml_str(yaml, false).unwrap();
    calliope_schema::Validated::<calliope_schema::CalliopeBuildMath>::from_tree(tree)
        .unwrap()
        .into_inner()
}

const BASE: &str = r#"
dimensions:
  techs: {iterator: tech}
  nodes: {iterator: node}
  carriers: {iterator: carrier}
  timesteps: {iterator: timestep, dtype: datetime}
parameters:
  p: {}
  q: {}
lookups:
  base_tech: {dtype: string}
  flag: {dtype: bool}
  techmap: {dtype: string}
variables:
  x: {foreach: [nodes, techs]}
  y: {foreach: [nodes, techs, timesteps]}
"#;

/// Diagnostics as `code @ snippet`.
fn check(extra: &str) -> Vec<String> {
    check_with(extra, None)
}

fn check_with(extra: &str, schema: Option<&InputSchema>) -> Vec<String> {
    let m = math(&format!("{BASE}{extra}"));
    typecheck(&m, schema)
        .diagnostics
        .iter()
        .map(|d| format!("{} @ {}", d.code, &d.source[d.span.clone()]))
        .collect()
}

fn constraint(foreach: &str, where_: &str, expr: &str) -> String {
    format!(
        "constraints:\n  c:\n    foreach: {foreach}\n    where: \"{where_}\"\n    equations:\n      - expression: \"{expr}\"\n"
    )
}

#[test]
fn clean_math_has_no_diagnostics() {
    let d = check(&constraint(
        "[nodes, techs]",
        "p AND base_tech==supply",
        "x + sum(y, over=timesteps) <= p * 2",
    ));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn nonlinear() {
    let d = check(&constraint("[nodes, techs]", "True", "x * x <= p"));
    assert!(d.contains(&"math::nonlinear @ x * x".to_owned()), "{d:?}");
    let d = check(&constraint("[nodes, techs]", "True", "p / x <= 1"));
    assert!(d.iter().any(|s| s.starts_with("math::nonlinear")), "{d:?}");
    let d = check(&constraint("[nodes, techs]", "True", "x ** 2 <= 1"));
    assert!(d.iter().any(|s| s.starts_with("math::nonlinear")), "{d:?}");
}

#[test]
fn dims_not_in_foreach() {
    let d = check(&constraint("[nodes, techs]", "True", "y <= p"));
    assert_eq!(d, ["math::dims::not-in-foreach @ y"]);
}

#[test]
fn sum_over_unknown_or_missing_dimension() {
    let d = check(&constraint("[nodes]", "True", "sum(x, over=tech) <= p"));
    assert!(
        d.contains(&"math::helper::unknown-dimension @ tech".to_owned()),
        "{d:?}"
    );
    let d = check(&constraint(
        "[nodes]",
        "True",
        "sum(x, over=[techs, carriers]) <= p",
    ));
    assert_eq!(d, ["math::helper::sum-missing-dimension @ carriers"]);
}

#[test]
fn comparison_dtypes() {
    let d = check(&constraint("[nodes, techs]", "base_tech==1", "x <= p"));
    assert_eq!(d, ["math::where::comparison-dtype @ base_tech==1"]);
    let d = check(&constraint("[nodes, techs]", "p==supply", "x <= p"));
    assert_eq!(d, ["math::where::comparison-dtype @ p==supply"]);
    let d = check(&constraint(
        "[nodes, techs]",
        "flag==True AND p>0",
        "x <= p",
    ));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn helper_context_and_arguments() {
    let d = check(&constraint(
        "[nodes, techs]",
        "True",
        "any(x, over=techs) <= p",
    ));
    assert_eq!(d, ["math::helper::unknown @ any"]);
    let d = check(&constraint("[nodes, techs]", "roll(p, techs=1)", "x <= p"));
    assert!(
        d.contains(&"math::helper::unknown @ roll".to_owned()),
        "{d:?}"
    );
    let d = check(&constraint("[nodes]", "True", "sum(x, techs) <= p"));
    assert_eq!(d, ["math::helper::arguments @ sum(x, techs)"]);
    let d = check(&constraint(
        "[nodes, techs]",
        "True",
        "roll(x, timestep=1) <= p",
    ));
    assert!(
        d.contains(&"math::helper::unknown-dimension @ 1".to_owned()),
        "{d:?}"
    );
}

#[test]
fn foreach_unknown_dimension() {
    let d = check(&constraint("[nodes, tech]", "True", "x <= p"));
    assert!(
        d.contains(&"math::foreach::unknown-dimension @ tech".to_owned()),
        "{d:?}"
    );
}

#[test]
fn global_expression_order() {
    let extra = r#"
global_expressions:
  e1:
    foreach: [nodes, techs]
    equations: [{expression: e2 + x}]
    order: 0
  e2:
    foreach: [nodes, techs]
    equations: [{expression: x}]
    order: 1
"#;
    let d = check(extra);
    assert_eq!(d, ["math::order @ e2"]);
}

#[test]
fn global_expression_kinds_propagate() {
    let extra = r#"
global_expressions:
  e1:
    foreach: [nodes, techs]
    equations: [{expression: x * p}]
  e2:
    foreach: [nodes, techs]
    equations: [{expression: p * q}]
    order: 1
"#;
    let d = check(&format!(
        "{extra}{}",
        constraint("[nodes, techs]", "True", "e1 * e2 <= 1")
    ));
    assert!(d.is_empty(), "{d:?}");
    let d = check(&format!(
        "{extra}{}",
        constraint("[nodes, techs]", "True", "e1 * x <= 1")
    ));
    assert_eq!(d, ["math::nonlinear @ e1 * x"]);
}

#[test]
fn never_matching_combination() {
    let extra = r#"
constraints:
  c:
    foreach: [nodes, techs]
    where: base_tech==supply
    equations:
      - expression: x <= $s
    sub_expressions:
      s:
        - where: base_tech==demand
          expression: p
        - where: NOT flag
          expression: q
        - where: flag
          expression: q
"#;
    let d = check(extra);
    assert_eq!(d, ["math::never-matches @ x <= $s"]);
}

#[test]
fn and_or_precedence_lint() {
    let d = check(&constraint("[nodes, techs]", "p AND q OR flag", "x <= p"));
    assert_eq!(d, ["math::where::and-or-precedence @ p AND q OR flag"]);
    let d = check(&constraint("[nodes, techs]", "p AND (q OR flag)", "x <= p"));
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn hidden_reference_under_sign() {
    let extra = r#"
constraints:
  c:
    foreach: [nodes, techs]
    equations:
      - expression: -$s <= x
    sub_expressions:
      s:
        - expression: p
"#;
    let d = check(extra);
    assert_eq!(d, ["math::calliope::hidden-reference @ -$s <= x"]);
}

#[test]
fn unused_sub_expression() {
    let extra = r#"
constraints:
  c:
    foreach: [nodes, techs]
    equations:
      - expression: x <= p
    sub_expressions:
      s:
        - expression: p
"#;
    assert_eq!(check(extra), ["math::unused @ p"]);
}

#[test]
fn slicing() {
    let d = check(&constraint("[nodes]", "True", "x[techs=pv] <= p"));
    assert!(d.is_empty(), "{d:?}");
    let d = check(&constraint("[nodes]", "True", "x[carriers=pv] <= p"));
    assert!(
        d.contains(&"math::slice::missing-dimension @ x[carriers=pv]".to_owned()),
        "{d:?}"
    );
}

#[test]
fn input_schema_dims() {
    let schema = InputSchema {
        dims: [("p".to_owned(), vec!["timesteps".to_owned()])].into(),
    };
    let d = check_with(
        &constraint("[nodes, techs]", "True", "x <= p"),
        Some(&schema),
    );
    assert_eq!(d, ["math::dims::not-in-foreach @ p"]);
    let d = check_with(
        &constraint("[nodes, techs]", "True", "x <= q"),
        Some(&schema),
    );
    assert!(d.is_empty(), "{d:?}");
}

#[test]
fn constraint_without_variables() {
    let d = check(&constraint("[nodes, techs]", "True", "p <= q"));
    assert_eq!(d, ["math::constraint::no-variables @ p <= q"]);
}
