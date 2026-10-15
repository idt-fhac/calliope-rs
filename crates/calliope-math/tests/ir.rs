//! Lowering to the evaluation IR: Calliope's implicit evaluation rules as explicit nodes.

mod common;

use calliope_math::ast::{ArithOp, CmpOp};
use calliope_math::helpers::Helper;
use calliope_math::ir::{Body, Ir, lower};
use calliope_math::{ParseMode, parse_math};

fn math(yaml: &str) -> calliope_schema::CalliopeBuildMath {
    let tree = calliope_yaml::read_yaml_str(yaml, false).unwrap();
    calliope_schema::Validated::<calliope_schema::CalliopeBuildMath>::from_tree(tree)
        .unwrap()
        .into_inner()
}

const MATH: &str = r#"
dimensions:
  techs: {iterator: tech}
  timesteps: {iterator: timestep, dtype: datetime}
parameters:
  p: {default: 5}
  q: {}
lookups:
  flag: {dtype: bool}
variables:
  x: {foreach: [techs, timesteps]}
constraints:
  c:
    foreach: [techs, timesteps]
    where: "p AND flag AND q>1"
    equations:
      - where: x
        expression: x * p + roll(x * q, timesteps=1) <= -q
"#;

fn mask(ir: Ir) -> Ir {
    Ir::Mask(Box::new(ir))
}
fn b(ir: Ir) -> Box<Ir> {
    Box::new(ir)
}

#[test]
fn defaults_presence_and_masking() {
    let m = math(MATH);
    let (comps, diags) = parse_math(&m, ParseMode::Build);
    assert!(diags.is_empty(), "{diags:?}");
    let c = comps.iter().find(|c| c.name == "c").unwrap();
    let low = lower(c, &m);
    let p = Ir::Input {
        name: "p".into(),
        default: Some(5.0),
    };
    let q = Ir::Input {
        name: "q".into(),
        default: None,
    };
    let x = Ir::Result("x".into());
    // `where`: presence tests outside comparisons, filled values inside.
    assert_eq!(
        low.mask.where_,
        Ir::And(
            b(Ir::And(
                b(Ir::InputPresence("p".into())),
                b(Ir::InputPresence("flag".into()))
            )),
            b(Ir::Compare(CmpOp::Gt, b(q.clone()), b(Ir::Const(1.0)))),
        )
    );
    let eq = &low.equations[0];
    assert_eq!(eq.wheres, [Ir::ResultPresence("x".into())]);
    // Operands are masked, except inside `roll` (ignore_where); unary minus is not.
    let roll = Ir::Helper {
        helper: Helper::Roll,
        params: vec![("array", Ir::Arith(ArithOp::Mul, b(x.clone()), b(q.clone())))],
        dim_kwargs: vec![("timesteps".into(), Ir::Const(1.0))],
    };
    let lhs = Ir::Arith(
        ArithOp::Add,
        b(mask(Ir::Arith(ArithOp::Mul, b(mask(x)), b(mask(p))))),
        b(mask(roll)),
    );
    assert_eq!(
        eq.body,
        Body::Constraint {
            lhs: mask(lhs),
            op: CmpOp::Le,
            rhs: mask(Ir::Neg(b(q))),
        }
    );
}

#[test]
fn lowers_all_shipped_math() {
    let mut n = 0;
    for (_, files) in common::math_sets() {
        let m = common::layered(&files).unwrap();
        let (comps, _) = parse_math(&m, ParseMode::Build);
        for c in &comps {
            let low = lower(c, &m);
            assert_eq!(low.equations.len(), c.equations.len());
            n += low.equations.len();
        }
    }
    assert!(n > 1000 || common::reference().is_none(), "{n}");
}
