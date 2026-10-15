//! Port of the parsing cases of `tests/backend/expression_parser_test.py`.
//!
//! Python tests individual pyparsing elements; here each case goes through the full grammar
//! of the context it belongs to. Numeric evaluation cases use a small constant folder.
//! Evaluation against model data, LaTeX output, default filling and `where` masking are
//! covered with the evaluation step.

use calliope_math::ast::{ArithOp, CmpOp, NodeKind, Sign};
use calliope_math::{Names, Node, StringKind, parse};

const COMPONENTS: [&str; 10] = [
    "foo",
    "with_inf",
    "only_techs",
    "no_dims",
    "multi_dim_var",
    "no_dim_var",
    "all_true",
    "all_ones",
    "only_techs_as_bool",
    "with_inf_as_bool",
];

fn names() -> Names {
    Names::from_lists(&[], &COMPONENTS, &[])
}

fn arith(s: &str) -> Result<Node, calliope_math::ParseError> {
    parse(StringKind::Arithmetic, s, &names())
}

fn repr(kind: StringKind, s: &str) -> String {
    match parse(kind, s, &names()) {
        Ok(n) => n.to_string(),
        Err(e) => panic!("{s:?} failed: {e}"),
    }
}

fn fails(kind: StringKind, s: &str) {
    assert!(
        parse(kind, s, &names()).is_err(),
        "{s:?} should not parse as {kind:?}"
    );
}

/// Folds numeric expressions (Python's float semantics).
fn eval(n: &Node) -> f64 {
    match &n.kind {
        NodeKind::Number(x) => x.value,
        NodeKind::Sign(Sign::Minus, x) => -eval(x),
        NodeKind::Sign(Sign::Plus, x) => eval(x),
        NodeKind::Arith(first, rest) => rest.iter().fold(eval(first), |acc, (op, x)| {
            let v = eval(x);
            match op {
                ArithOp::Add => acc + v,
                ArithOp::Sub => acc - v,
                ArithOp::Mul => acc * v,
                ArithOp::Div => acc * (1.0 / v),
                ArithOp::Pow => acc.powf(v),
            }
        }),
        other => panic!("not numeric: {other:?}"),
    }
}

fn eval_cmp(n: &Node) -> bool {
    let NodeKind::Compare { lhs, op, rhs, .. } = &n.kind else {
        panic!("not a comparison")
    };
    let (l, r) = (eval(lhs), eval(rhs));
    match op {
        CmpOp::Eq => l == r,
        CmpOp::Le => l <= r,
        CmpOp::Ge => l >= r,
        CmpOp::Lt => l < r,
        CmpOp::Gt => l > r,
    }
}

#[test]
fn numbers() {
    for (s, v) in [
        ("0", 0.0),
        ("0.11", 0.11),
        ("1.0", 1.0),
        ("20.1", 20.1),
        ("1e2", 100.0),
        ("1e-2", 0.01),
        ("inf", f64::INFINITY),
        ("INF", f64::INFINITY),
        (".inf", f64::INFINITY),
    ] {
        assert_eq!(eval(&arith(s).unwrap()), v, "{s}");
    }
    // A leading sign is a sign operator in arithmetic, and part of the number in slices.
    assert_eq!(repr(StringKind::Arithmetic, "-1"), "(-)NUM:1");
    assert_eq!(repr(StringKind::Arithmetic, "-1e2"), "(-)NUM:100.0");
    assert_eq!(repr(StringKind::Slice, "-1"), "NUM:-1");
    assert_eq!(repr(StringKind::Slice, "-1e2"), "NUM:-100.0");
}

#[test]
fn fail_numbers() {
    for s in [
        "1 2", "1'200", "1,200", "fo1o", "1foo", "1ee2", "infinity", "pi", "e", "one",
    ] {
        fails(StringKind::Arithmetic, s);
    }
}

#[test]
fn identifiers() {
    let names = Names::from_lists(&[], &["foo", "foo_bar", "Foo1", "FOO_1_BAR", "FOO__"], &[]);
    for s in ["foo", "foo_bar", "Foo1", "FOO_1_BAR", "FOO__"] {
        let n = parse(StringKind::Arithmetic, s, &names).unwrap();
        assert_eq!(n.to_string(), format!("COMPONENT:{s}"));
    }
    for s in [
        // `foo(bar)` is a helper call in the full grammar.
        "foo bar", "foo.bar", "foo-bar", "foo[bar]", "1foo", "_foo", "__type__",
    ] {
        assert!(parse(StringKind::Slice, s, &names).is_err(), "{s}");
    }
    // `inf` is a number, not an identifier.
    for s in ["inf", "INF", ".inf"] {
        assert_eq!(repr(StringKind::Slice, s), "NUM:inf");
    }
}

#[test]
fn evaluatable_identifier() {
    assert_eq!(repr(StringKind::Slice, "hello"), "STRING:hello");
    assert_eq!(repr(StringKind::Slice, "foo"), "COMPONENT:foo");
}

#[test]
fn id_lists() {
    assert_eq!(
        repr(StringKind::Arithmetic, "dummy_func_1([hello, there])"),
        "dummy_func_1(args=[[STRING:hello, STRING:there]], kwargs={})"
    );
    assert_eq!(
        repr(
            StringKind::Arithmetic,
            "dummy_func_1([hello, 1, 1.0, there])"
        ),
        "dummy_func_1(args=[[STRING:hello, NUM:1, NUM:1.0, STRING:there]], kwargs={})"
    );
    assert_eq!(
        repr(StringKind::Arithmetic, "dummy_func_1([hello, with_inf])"),
        "dummy_func_1(args=[[STRING:hello, COMPONENT:with_inf]], kwargs={})"
    );
    for s in ["$foo", "dummy_func_1(1)"] {
        fails(
            StringKind::Arithmetic,
            &format!("dummy_func_1([{s}, there])"),
        );
    }
}

#[test]
fn unsliced() {
    assert_eq!(
        repr(StringKind::Arithmetic, "with_inf"),
        "COMPONENT:with_inf"
    );
    for s in ["Foo", "foobar"] {
        fails(StringKind::Arithmetic, s);
    }
}

#[test]
fn sliced() {
    for (s, expected) in [
        ("foo[techs=tech]", "SLICED_COMPONENT:foo[techs=STRING:tech]"),
        (
            "foo[techs=$tech]",
            "SLICED_COMPONENT:foo[techs=REFERENCE:tech]",
        ),
        (
            "foo[techs=tech,bars=$bar]",
            "SLICED_COMPONENT:foo[techs=STRING:tech, bars=REFERENCE:bar]",
        ),
        (
            "foo[ bars=$bar, techs=tech ]",
            "SLICED_COMPONENT:foo[bars=REFERENCE:bar, techs=STRING:tech]",
        ),
        (
            "with_inf[techs=[tech1, tech2], nodes=node]",
            "SLICED_COMPONENT:with_inf[techs=[STRING:tech1, STRING:tech2], nodes=STRING:node]",
        ),
    ] {
        assert_eq!(repr(StringKind::Arithmetic, s), expected);
    }
    for s in [
        "foobar[bars=bar]",
        "foo [bars=bar]",
        "foo[techs=tech bars=bar]",
        "foo[]",
        "foo[techs]",
        "foo[techs=]",
        "foo[=techs]",
        "[bars=bar]",
        // `foo(bars=bar)` is a helper call in the full grammar.
        // components are not slicers
        "foo[techs=with_inf]",
        "foo[techs=[a, with_inf]]",
    ] {
        fails(StringKind::Arithmetic, s);
    }
}

#[test]
fn sub_expressions() {
    for (s, name) in [("$foo", "foo"), ("$Foo_Bar_1", "Foo_Bar_1")] {
        assert_eq!(
            repr(StringKind::Arithmetic, s),
            format!("SUB_EXPRESSION:{name}")
        );
    }
    for s in [
        "$ foo",
        "foo$",
        "f$oo",
        "$",
        "$foo(1)",
        "$foo[bars=bar1]",
        "$1",
    ] {
        fails(StringKind::Arithmetic, s);
    }
    // No sub-expressions inside sub-expressions.
    fails(StringKind::SubExpression, "$foo");
}

#[test]
fn function_parse() {
    for (s, expected) in [
        (
            "dummy_func_1(foo[bars=$bar])",
            "dummy_func_1(args=[SLICED_COMPONENT:foo[bars=REFERENCE:bar]], kwargs={})",
        ),
        (
            "dummy_func_1(1, x=foo[bars=bar1])",
            "dummy_func_1(args=[NUM:1], kwargs={x=SLICED_COMPONENT:foo[bars=STRING:bar1]})",
        ),
        (
            "dummy_func_1($foo)",
            "dummy_func_1(args=[SUB_EXPRESSION:foo], kwargs={})",
        ),
        (
            "dummy_func_1($foo, x=$foo)",
            "dummy_func_1(args=[SUB_EXPRESSION:foo], kwargs={x=SUB_EXPRESSION:foo})",
        ),
        (
            "dummy_func_1(dummy_func_2(foo[bars=bar1], y=$foo))",
            "dummy_func_1(args=[dummy_func_2(args=[SLICED_COMPONENT:foo[bars=STRING:bar1]], kwargs={y=SUB_EXPRESSION:foo})], kwargs={})",
        ),
        (
            "dummy_func_1(1, dummy_func_2(1, foo[bars=$bar]), $foo, 1, foo, x=foo[foos=foo1, bars=bar1])",
            "dummy_func_1(args=[NUM:1, dummy_func_2(args=[NUM:1, SLICED_COMPONENT:foo[bars=REFERENCE:bar]], kwargs={}), SUB_EXPRESSION:foo, NUM:1, COMPONENT:foo], kwargs={x=SLICED_COMPONENT:foo[foos=STRING:foo1, bars=STRING:bar1]})",
        ),
        // kwargs separated by whitespace only (`dict_of` is `OneOrMore`)
        ("f(x=1 y=2)", "f(args=[], kwargs={x=NUM:1, y=NUM:2})"),
        ("f()", "f(args=[], kwargs={})"),
    ] {
        assert_eq!(repr(StringKind::Arithmetic, s), expected, "{s}");
    }
}

#[test]
fn function_malformed() {
    for s in [
        "dummy_func_1",
        "dummy_func_1(x=1, 2)",
        "dummy_func_1[]",
        "dummy_func_1{}",
        "dummy_func_1 ()",
        "dummy_func_1()dummy_func_2()",
        "()",
        "()dummy_func_1",
    ] {
        fails(StringKind::Arithmetic, s);
    }
}

#[test]
fn function_args_with_arithmetic() {
    for s in ["1 + 2", "1 * 2", "x=1/2", "foo + 1, x=1", "foo, x=1+1"] {
        parse(StringKind::Arithmetic, &format!("foobar({s})"), &names()).unwrap();
    }
    // Generic strings cannot take part in arithmetic.
    for s in ["hello + 1", "1 + hello", "-hello", "(hello)", "hello[a=b]"] {
        fails(StringKind::Arithmetic, &format!("foobar({s})"));
    }
}

#[test]
fn arithmetic_values() {
    let numbers = ["2", "100", "0.02", "1e2", "2e-2", "inf"];
    for a in numbers {
        for b in numbers {
            let (x, y): (f64, f64) = (a.parse().unwrap(), b.parse().unwrap());
            for (op, expected) in [
                ("+", x + y),
                ("-", x - y),
                ("*", x * y),
                ("/", x * (1.0 / y)),
                ("**", x.powf(y)),
            ] {
                for wrap in ["{}", "dummy_func_1({})"] {
                    let s = format!("{a} {op} {b}");
                    let n = arith(&wrap.replace("{}", &s)).unwrap();
                    let n = match &n.kind {
                        NodeKind::Call { args, .. } => args[0].clone(),
                        _ => n,
                    };
                    let got = eval(&n);
                    assert!(
                        (got.is_nan() && expected.is_nan())
                            || got == expected
                            || (got - expected).abs() <= 1e-12 * expected.abs(),
                        "{s}: {got} != {expected}"
                    );
                }
            }
        }
        for sign in ["+", "-"] {
            let x: f64 = a.parse().unwrap();
            let n = arith(&format!("{sign}{a}")).unwrap();
            assert_eq!(eval(&n), if sign == "-" { -x } else { x });
        }
    }
}

#[test]
fn mashup_precedence() {
    for (s, expected) in [
        ("1+2*2", 5.0),
        ("-1 - 2", -3.0),
        ("-1 + 2**-2", -0.75),
        ("2**2**2-1", 15.0),
        ("2 * 3 + -2 - -3", 7.0),
        ("100 / 10 + 3 * 5 - 2**3 + -5", 12.0),
        (
            "(1e5 * 10 / 1000 - 16**0.5) + (10 + 100) * (10) - 1/2",
            2095.5,
        ),
        // The sign binds tighter than `**`, unlike Python.
        ("-2**2", 4.0),
    ] {
        assert_eq!(eval(&arith(s).unwrap()), expected, "{s}");
    }
}

#[test]
fn non_numbers_parse() {
    let numbers = ["2", "100", "0.02", "1e2", "2e-2", "inf"];
    let subs = ["$foo", "$bar1"];
    let unsliced = ["foo", "with_inf"];
    let sliced = ["foo[bars=bar1]", "with_inf[foos=foo1, bars=$bar]"];
    let helpers = ["foo(1)", "bar1(foo, $foo, with_inf[foos=foo1], x=1)"];
    for n in numbers {
        for s in subs {
            for u in unsliced {
                for sl in sliced {
                    for h in helpers {
                        let items = [n, s, u, sl, h];
                        for rot in 0..5 {
                            let it: Vec<&str> = (0..5).map(|i| items[(i + rot) % 5]).collect();
                            let eq = format!(
                                "({} / {}) + {} - {} * {}**-{}",
                                it[0], it[1], it[1], it[2], it[3], it[4]
                            );
                            arith(&eq).unwrap_or_else(|e| panic!("{eq}: {e}"));
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn arithmetic_repr() {
    assert_eq!(
        repr(
            StringKind::Arithmetic,
            "1 + foo - foo[foos=foo1, bars=$bar] + (foo / $foo) ** -2"
        ),
        "(NUM:1 + COMPONENT:foo - SLICED_COMPONENT:foo[foos=STRING:foo1, bars=REFERENCE:bar] + ((COMPONENT:foo / SUB_EXPRESSION:foo) ** (-)NUM:2))"
    );
}

#[test]
fn slice_parser() {
    for s in ["foo", "1", "[FOO, BAR]", "foo[bars=bar1]", "FOO"] {
        for wrap in ["dummy_func_1({})", "{}"] {
            parse(StringKind::Slice, &wrap.replace("{}", s), &names()).unwrap();
        }
    }
    for s in ["foo + 1", "foo == 1", "$foo", "foo[bars=$bar]"] {
        for wrap in ["dummy_func_1({})", "{}"] {
            fails(StringKind::Slice, &wrap.replace("{}", s));
        }
    }
}

#[test]
fn sub_expression_parser() {
    for s in [
        "foo",
        "1",
        "dummy_func_1([FOO, BAR])",
        "dummy_func_1(FOO)",
        "foo[foos=foo1, bars=$bar]",
        "foo + 1",
    ] {
        for wrap in ["dummy_func_1({})", "{}"] {
            parse(StringKind::SubExpression, &wrap.replace("{}", s), &names()).unwrap();
        }
    }
    for s in ["[FOO, BAR]", "foo == 1", "$foo", "[foo]"] {
        fails(StringKind::SubExpression, s);
    }
}

const EQ_TERMS: [(&str, &str); 10] = [
    ("0", "NUM:0"),
    ("-1", "(-)NUM:1"),
    ("100.0", "NUM:100.0"),
    ("1/2", "(NUM:1 / NUM:2)"),
    ("2**2.0", "(NUM:2 ** NUM:2.0)"),
    (".inf", "NUM:inf"),
    ("with_inf", "COMPONENT:with_inf"),
    (
        "foo[foos=foo1, bars=bar1]",
        "SLICED_COMPONENT:foo[foos=STRING:foo1, bars=STRING:bar1]",
    ),
    ("$foo", "SUB_EXPRESSION:foo"),
    (
        "dummy_func_1(1, y=foo[bars=$bar])",
        "dummy_func_1(args=[NUM:1], kwargs={y=SLICED_COMPONENT:foo[bars=REFERENCE:bar]})",
    ),
];

#[test]
fn simple_equations() {
    for (l, le) in EQ_TERMS {
        for (r, re) in EQ_TERMS {
            for op in ["<=", ">=", "=="] {
                let s = format!("{l} {op} {r}");
                assert_eq!(repr(StringKind::Equation, &s), format!("{le} {op} {re}"));
            }
        }
    }
}

#[test]
fn equation_evaluation() {
    for (s, expected) in [
        ("1<=2", true),
        ("1 >= 2", false),
        ("1  ==  2", false),
        ("(1) <= (2)", true),
        ("1 * 3 <= 1e2", true),
        ("-1 >= -0.1 / 2", false),
        ("2**2 == 4 * 1 / 1 * 1**1", true),
        ("(1 + 3) * 2 >= 10 + -1", false),
    ] {
        let n = parse(StringKind::Equation, s, &names()).unwrap();
        assert_eq!(eval_cmp(&n), expected, "{s}");
    }
}

#[test]
fn equation_malformed() {
    for s in [
        "1 + 2 =<",
        "== 1 + 2 ",
        "1 = 2",
        "1 < 2",
        "2 > 1",
        "1 (<= 2)",
        "foo.bar <= 2",
        "1 <= foo <= 2",
    ] {
        fails(StringKind::Equation, s);
    }
}

#[test]
fn equation_repr() {
    assert_eq!(
        repr(
            StringKind::Equation,
            "1 + foo - foo[foos=foo1, bars=$bar] >= (foo / $foo) ** -2"
        ),
        "(NUM:1 + COMPONENT:foo - SLICED_COMPONENT:foo[foos=STRING:foo1, bars=REFERENCE:bar]) >= ((COMPONENT:foo / SUB_EXPRESSION:foo) ** (-)NUM:2)"
    );
}
