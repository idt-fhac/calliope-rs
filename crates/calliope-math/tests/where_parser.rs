//! Port of the parsing cases of `tests/backend/where_parser_test.py`.
//!
//! Python tests the pyparsing elements one by one; here each case goes through the full
//! `where` grammar. Evaluation cases (masks over model data, config lookups) are covered
//! with the evaluation step; LaTeX output is not ported.

use calliope_math::{Names, StringKind, parse};

fn names() -> Names {
    Names::from_lists(
        &[
            "techs",
            "nodes",
            "carriers",
            "costs",
            "timesteps",
            "dummy_dim",
        ],
        &[
            "foo_bar",
            "with_inf",
            "all_inf",
            "all_nan",
            "only_techs",
            "no_dims",
            "all_true",
            "all_ones",
            "only_techs_as_bool",
        ],
        &["multi_dim_var", "multi_dim_expr", "no_dim_var"],
    )
}

fn repr(s: &str) -> String {
    match parse(StringKind::Where, s, &names()) {
        Ok(n) => n.to_string(),
        Err(e) => panic!("{s:?} failed: {e}"),
    }
}

fn fails(s: &str) {
    assert!(
        parse(StringKind::Where, s, &names()).is_err(),
        "{s:?} should not parse"
    );
}

#[test]
fn dimensions() {
    for d in ["nodes", "techs", "carriers", "dummy_dim"] {
        assert_eq!(repr(&format!("{d}==foo")), format!("DIM:{d}==STRING:foo"));
    }
    // Not dimensions, or malformed.
    for s in ["_foo", "__type__", "1foo", "[techs]", "tech"] {
        fails(&format!("any(x, over={s}) and {s}"));
    }
    // A dimension is not a `where` condition on its own.
    fails("techs");
}

#[test]
fn inputs() {
    for s in ["with_inf", "all_inf", "all_nan", "foo_bar"] {
        assert_eq!(repr(s), format!("INPUT:{s}"));
    }
    for s in ["_foo", "__type__", "1foo", "with _ inf", "bar", "nodes"] {
        fails(s);
    }
}

#[test]
fn results() {
    assert_eq!(repr("multi_dim_var"), "RESULT:multi_dim_var");
    assert_eq!(repr("multi_dim_expr"), "RESULT:multi_dim_expr");
    // Results cannot be compared, except in postprocessed math.
    fails("multi_dim_var>0");
    let mut pp = names();
    pp.postprocessed.insert("total".to_owned());
    let n = parse(StringKind::Where, "multi_dim_var>0 and total", &pp).unwrap();
    assert_eq!(
        n.to_string(),
        "(RESULT:multi_dim_var>NUM:0 and RESULT:total)"
    );
}

#[test]
fn config_options() {
    for (s, key) in [
        ("config.foo", "foo"),
        ("config.FOO", "FOO"),
        ("config.foo1", "foo1"),
        ("config.a_b", "a_b"),
        ("config.bar", "bar"),
    ] {
        assert_eq!(
            repr(&format!("{s}==True")),
            format!("CONFIG:{key}==BOOL:true")
        );
    }
    for s in [
        "config.",
        "config.bar.foobar",
        "RUN",
        "r un.foo",
        "model,a_b",
        "scenarios",
        "CONFIG.bar",
        "foo.bar",
        "all_inf.is_result",
    ] {
        fails(&format!("{s}==1"));
    }
}

#[test]
fn booleans() {
    for (s, b) in [
        ("true", true),
        ("TRUE", true),
        ("True", true),
        ("false", false),
        ("FALSE", false),
        ("False", false),
    ] {
        assert_eq!(repr(s), format!("BOOL:{b}"));
    }
    for s in ["tru e", "_TRUE", "True_", "false1", "1false", "1", "foo"] {
        fails(s);
    }
}

#[test]
fn generic_strings() {
    for s in ["foo", "FOO", "foo10", "foo_10"] {
        assert_eq!(
            repr(&format!("with_inf=={s}")),
            format!("INPUT:with_inf==STRING:{s}")
        );
    }
    for s in ["_foo", "1foo", ".foo", "$foo", "__foo__"] {
        fails(&format!("with_inf=={s}"));
    }
    // Inside a helper call, names are not generic strings.
    assert_eq!(
        repr("any(techs, over=foo_bar)"),
        "any(args=[DIM:techs], kwargs={over=INPUT:foo_bar})"
    );
    assert_eq!(repr("with_inf==inf"), "INPUT:with_inf==NUM:inf");
}

#[test]
fn comparisons() {
    for (s, expected) in [
        ("all_inf==.inf", "INPUT:all_inf==NUM:inf"),
        ("all_inf==1", "INPUT:all_inf==NUM:1"),
        ("all_inf==bar", "INPUT:all_inf==STRING:bar"),
        ("with_inf + 2==3", "(INPUT:with_inf + NUM:2)==NUM:3"),
        ("with_inf<3", "INPUT:with_inf<NUM:3"),
        ("with_inf<=3", "INPUT:with_inf<=NUM:3"),
        ("with_inf>1", "INPUT:with_inf>NUM:1"),
        ("with_inf>=1", "INPUT:with_inf>=NUM:1"),
        ("with_inf==-1", "INPUT:with_inf==NUM:-1"),
        (
            "with_inf + no_dims == 3",
            "(INPUT:with_inf + INPUT:no_dims)==NUM:3",
        ),
        (
            "all_ones + only_techs - 5 > 0",
            "(INPUT:all_ones + INPUT:only_techs - NUM:5)>NUM:0",
        ),
        (
            "sum(with_inf, over=techs) + sum(all_ones, over=nodes) <= 200",
            "(sum(args=[INPUT:with_inf], kwargs={over=DIM:techs}) + sum(args=[INPUT:all_ones], kwargs={over=DIM:nodes}))<=NUM:200",
        ),
        ("config.a_b==0.0", "CONFIG:a_b==NUM:0.0"),
        ("config.a_b==False", "CONFIG:a_b==BOOL:false"),
        (
            "timesteps<=get_val_at_index(timesteps=-24)",
            "DIM:timesteps<=get_val_at_index(args=[], kwargs={timesteps=NUM:-24})",
        ),
    ] {
        assert_eq!(repr(s), expected, "{s}");
    }
    for s in [
        "config.foo=bar",
        "all_inf==__type__",
        "$foo==bar",
        "foo==$bar",
        "foo==config.bar",
        "config.foo==_bar",
        // Arithmetic on the right-hand side is not allowed.
        "with_inf==1 + 1",
    ] {
        fails(s);
    }
}

#[test]
fn comparison_helper_arguments() {
    // A non-component identifier must be the whole argument (`unique_evaluatable_string`
    // comes first in the argument grammar), so no nested helpers or config options.
    assert_eq!(
        repr("sum(with_inf + 1, over=techs)>0"),
        "sum(args=[(INPUT:with_inf + NUM:1)], kwargs={over=DIM:techs})>NUM:0"
    );
    fails("sum(foo + 1, over=techs)>0");
    fails("sum(config.foo, over=techs)>0");
    fails("sum(get_val_at_index(timesteps=0), over=techs)>0");
    // A leading number commits to the number.
    fails("sum(1 + with_inf, over=techs)>0");
    assert_eq!(
        repr("sum((1 + with_inf), over=techs)>0"),
        "sum(args=[(NUM:1 + INPUT:with_inf)], kwargs={over=DIM:techs})>NUM:0"
    );
}

#[test]
fn subsets() {
    for (s, items) in [
        ("[bar]", "STRING:bar"),
        ("[foobar, bar]", "STRING:foobar, STRING:bar"),
        ("[ 1 ]", "NUM:1"),
        ("[1., 2e2]", "NUM:1.0, NUM:200.0"),
        ("[1, bar]", "NUM:1, STRING:bar"),
        ("[bar, 1]", "STRING:bar, NUM:1"),
    ] {
        assert_eq!(
            repr(&format!("{s} in nodes")),
            format!("SUBSET:DIM:nodes[{items}]")
        );
    }
    for s in [
        "[bar] infoo",
        "[bar] in",
        "bar in [nodes]",
        "[foo==bar] in nodes",
        "[any(techs=[tech1, tech2], within=nodes, how=any)] in nodes",
        "(bar) in nodes",
        "[bar]in nodes",
        "[bar] in foo_bar",
    ] {
        fails(s);
    }
}

#[test]
fn logic() {
    for (s, expected) in [
        (
            "config.foo==True and config.a_b==0",
            "(CONFIG:foo==BOOL:true and CONFIG:a_b==NUM:0)",
        ),
        (
            "config.foo==False Or config.a_b==0",
            "(CONFIG:foo==BOOL:false or CONFIG:a_b==NUM:0)",
        ),
        ("not config.foo==True", "(not)CONFIG:foo==BOOL:true"),
        (
            "Not config.foo==False and config.a_b==0",
            "((not)CONFIG:foo==BOOL:false and CONFIG:a_b==NUM:0)",
        ),
        (
            "config.foo==1  and  config.a_b==0",
            "(CONFIG:foo==NUM:1 and CONFIG:a_b==NUM:0)",
        ),
        (
            "not all_inf and not all_nan",
            "((not)INPUT:all_inf and (not)INPUT:all_nan)",
        ),
        (
            "with_inf and [bar] in nodes",
            "(INPUT:with_inf and SUBSET:DIM:nodes[STRING:bar])",
        ),
        (
            "with_inf or ([bar] in nodes)",
            "(INPUT:with_inf or SUBSET:DIM:nodes[STRING:bar])",
        ),
        // `and` and `or` share a precedence level and associate to the left.
        (
            "all_inf and all_nan or config.foo==True",
            "(INPUT:all_inf and INPUT:all_nan or CONFIG:foo==BOOL:true)",
        ),
        (
            "all_inf and (all_nan or config.foo==True)",
            "(INPUT:all_inf and (INPUT:all_nan or CONFIG:foo==BOOL:true))",
        ),
        (
            "(all_inf==inf and with_inf) or (config.foo==True and all_nan)",
            "((INPUT:all_inf==NUM:inf and INPUT:with_inf) or (CONFIG:foo==BOOL:true and INPUT:all_nan))",
        ),
        (
            "get_val_at_index(nodes=0)",
            "get_val_at_index(args=[], kwargs={nodes=NUM:0})",
        ),
    ] {
        assert_eq!(repr(s), expected, "{s}");
    }
}

#[test]
fn malformed() {
    for s in [
        "and",
        "or",
        "not",
        "not and",
        "and or",
        "all_inf and",
        "and all_inf",
        "with_inf not and all_inf",
        "with_inf and or all_inf",
        "config.foo==True and and config.foo==True",
        "config.foo==True andnot all_inf",
        // keywords need a non-identifier character before them
        "with_inf==0and all_inf",
    ] {
        fails(s);
    }
}

#[test]
fn repr_cases() {
    for (s, expected) in [
        ("foo_bar", "INPUT:foo_bar"),
        ("multi_dim_var", "RESULT:multi_dim_var"),
        ("TRUE", "BOOL:true"),
        ("config.bar==True", "CONFIG:bar==BOOL:true"),
        ("[bar, 1] in nodes", "SUBSET:DIM:nodes[STRING:bar, NUM:1]"),
    ] {
        assert_eq!(repr(s), expected);
    }
}
