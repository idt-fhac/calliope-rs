//! Backend behaviour: options, logs, duals, in-place updates, fix/unfix, (de)activation,
//! deletion and SOS2 handling.

use calliope_highs::{HighsBackend, OptionValue, SolveConfig, Target};
use calliope_lp::{ComponentKind, Label, LinExpr, LpModel, ObjSense, RowSense};

fn quiet() -> SolveConfig {
    SolveConfig {
        solver_options: vec![("output_flag".into(), OptionValue::Bool(false))],
        ..SolveConfig::default()
    }
}

fn var(name: &str, idx: &[&str]) -> Label {
    Label::from_strs(ComponentKind::Variable, name, idx)
}

fn con(name: &str, idx: &[&str]) -> Label {
    Label::from_strs(ComponentKind::Constraint, name, idx)
}

/// min 2a + 3b  s.t.  a + b >= 4 (demand), a <= 3 (cap);  a, b >= 0  → a = 3, b = 1, obj 9.
fn model() -> LpModel {
    let mut m = LpModel::new();
    let a = m.add_col(var("flow", &["a"]), 0.0, f64::INFINITY, false);
    let b = m.add_col(var("flow", &["b"]), 0.0, f64::INFINITY, false);
    m.add_row_sense(con("demand", &[]), [(a, 1.0), (b, 1.0)], RowSense::Ge, 4.0);
    m.add_row_sense(con("cap", &["a"]), [(a, 1.0)], RowSense::Le, 3.0);
    m.set_objective(
        Label::from_strs(ComponentKind::Objective, "cost", &[]),
        ObjSense::Minimize,
        LinExpr {
            terms: vec![(a, 2.0), (b, 3.0)],
            constant: 0.5,
        },
    );
    m
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-7
}

#[test]
fn solve_values_duals_and_logs() {
    let dir = tempfile::tempdir().unwrap();
    let mut be = HighsBackend::new(model());
    let cfg = SolveConfig {
        save_logs: Some(dir.path().join("logs")),
        ..quiet()
    };
    let out = be.solve(&cfg).unwrap();
    assert_eq!(out.termination_condition, "optimal");
    assert!(close(out.objective.unwrap(), 9.5));
    let x = out.col_value.as_ref().unwrap();
    assert!(close(x[0], 3.0) && close(x[1], 1.0));
    let d = out.row_dual.as_ref().unwrap();
    assert!(close(d[0], 3.0) && close(d[1], -1.0), "{d:?}");
    assert!(dir.path().join("logs/highs.log").exists());
    assert!(!be.has_integer_or_binary_variables());
}

#[test]
fn infeasible_and_bad_option() {
    let mut m = model();
    m.add_row_sense(con("never", &[]), [(0, 1.0)], RowSense::Ge, 10.0);
    let mut be = HighsBackend::new(m);
    let out = be.solve(&quiet()).unwrap().clone();
    assert_eq!(out.termination_condition, "infeasible");
    assert!(out.objective.is_none() && out.col_value.is_none());
    let mut cfg = quiet();
    cfg.solver_options
        .push(("not_an_option".into(), OptionValue::Int(3)));
    assert!(be.solve(&cfg).is_err());
}

#[test]
fn in_place_updates() {
    let mut be = HighsBackend::new(model());
    be.solve(&quiet()).unwrap();
    // cap 3 -> 1 and cost of b 3 -> 1.5 (as a re-evaluated parameter would)
    be.deps.register("flow_cap_max", Target::RowUpper(1));
    assert_eq!(be.deps.rows("flow_cap_max"), vec![1]);
    be.apply(&[(Target::RowUpper(1), 1.0), (Target::ObjCoeff(1), 1.5)])
        .unwrap();
    let out = be.solve(&quiet()).unwrap();
    // b is now cheaper: a = 0, b = 4 → 6 + 0.5
    assert!(close(out.objective.unwrap(), 6.5));
    // coefficient change: demand row a-coefficient 2 → a counts double
    be.apply(&[(Target::ObjCoeff(1), 3.0), (Target::Coeff(0, 0), 2.0)])
        .unwrap();
    let out = be.solve(&quiet()).unwrap();
    // a <= 1 gives 2 units, b = 2 → 2 + 6 + 0.5
    assert!(close(out.objective.unwrap(), 8.5), "{out:?}");
    assert_eq!(be.model().row(0).1, &[2.0, 1.0]);
}

#[test]
fn fix_unfix_bounds_activation_and_delete() {
    let mut be = HighsBackend::new(model());
    assert!(be.fix_variable("flow", None).is_err());
    be.solve(&quiet()).unwrap();
    be.fix_variable("flow", Some(&[true, false])).unwrap();
    assert_eq!((be.model().cols[0].lb, be.model().cols[0].ub), (3.0, 3.0));
    be.unfix_variable("flow", None).unwrap();
    assert_eq!(be.model().cols[0].ub, f64::INFINITY);

    be.update_variable_bounds("flow", None, Some(&[f64::NAN, 0.5]))
        .unwrap();
    assert_eq!(be.model().cols[1].ub, 0.5);
    let out = be.solve(&quiet()).unwrap();
    assert_eq!(out.termination_condition, "infeasible");

    be.set_constraint_active("cap", false).unwrap();
    let out = be.solve(&quiet()).unwrap();
    assert!(close(out.objective.unwrap(), 8.5)); // a = 3.5, b = 0.5
    be.set_constraint_active("cap", true).unwrap();
    assert_eq!(be.model().rows[1].ub, 3.0);

    let map = be
        .delete_component(ComponentKind::Constraint, "cap")
        .unwrap();
    assert_eq!(map, vec![Some(0), None]);
    let out = be.solve(&quiet()).unwrap();
    assert!(close(out.objective.unwrap(), 8.5));
    assert!(be.set_constraint_active("cap", true).is_err());
}

#[test]
fn warmstart_and_objective_switch() {
    let mut be = HighsBackend::new(model());
    be.solve(&quiet()).unwrap();
    be.set_objective(
        Label::from_strs(ComponentKind::Objective, "max_b", &[]),
        ObjSense::Maximize,
        LinExpr {
            terms: vec![(1, 1.0)],
            constant: 0.0,
        },
    )
    .unwrap();
    be.update_variable_bounds("flow", None, Some(&[f64::NAN, 7.0]))
        .unwrap();
    let cfg = SolveConfig {
        warmstart: true,
        ..quiet()
    };
    let out = be.solve(&cfg).unwrap();
    assert!(close(out.objective.unwrap(), 7.0));
    be.set_start(&[0.0, 7.0]).unwrap();
    assert!(close(be.solve(&cfg).unwrap().objective.unwrap(), 7.0));
}

#[test]
fn sos2_piecewise() {
    // Non-convex piecewise curve through (0,0), (1,10), (2,21), (3,30); x in [1, 2.5];
    // minimise y - 10.5x. On the curve the optimum is x = 2.5, y = 25.5 (obj -0.75); the
    // LP relaxation would mix (0,0) and (3,30) (y = 25 at x = 2.5, obj -1.25).
    let pts = [(0.0, 0.0), (1.0, 10.0), (2.0, 21.0), (3.0, 30.0)];
    let mut m = LpModel::new();
    let x = m.add_col(var("x", &[]), 1.0, 3.0, false);
    let y = m.add_col(var("y", &[]), f64::NEG_INFINITY, f64::INFINITY, false);
    let lam: Vec<usize> = (0..4)
        .map(|i| {
            m.add_col(
                Label::from_strs(ComponentKind::Piecewise, "pw", &[])
                    .with_suffix(format!("_v({i})")),
                0.0,
                f64::INFINITY,
                false,
            )
        })
        .collect();
    let pw =
        |s: &str| Label::from_strs(ComponentKind::Piecewise, "pw", &[]).with_suffix(s.to_string());
    let mut tx = vec![(x, -1.0)];
    tx.extend(lam.iter().zip(&pts).map(|(&l, p)| (l, p.0)));
    m.add_row_sense(pw("_c(0)"), tx, RowSense::Eq, 0.0);
    let mut ty = vec![(y, -1.0)];
    ty.extend(lam.iter().zip(&pts).map(|(&l, p)| (l, p.1)));
    m.add_row_sense(pw("_c(1)"), ty, RowSense::Eq, 0.0);
    m.add_row_sense(
        pw("_c(2)"),
        lam.iter().map(|&l| (l, 1.0)),
        RowSense::Eq,
        1.0,
    );
    m.add_sos2(
        Label::from_strs(ComponentKind::Piecewise, "pw", &[]),
        lam.iter()
            .enumerate()
            .map(|(i, &l)| (l, (i + 1) as f64))
            .collect(),
    );
    m.set_objective(
        Label::from_strs(ComponentKind::Objective, "o", &[]),
        ObjSense::Minimize,
        LinExpr {
            terms: vec![(y, 1.0), (x, -10.5)],
            constant: 0.0,
        },
    );
    let mut be = HighsBackend::new(m);
    assert!(be.has_integer_or_binary_variables());
    be.set_col_bounds(x, 1.0, 2.5).unwrap();
    let out = be.solve(&quiet()).unwrap();
    let x_val = out.col_value.as_ref().unwrap()[x];
    assert!(close(x_val, 2.5), "{out:?}");
    assert!(close(out.objective.unwrap(), -0.75), "{out:?}");
    assert!(out.row_dual.is_none(), "MILP has no duals");
}

/// HiGHS refuses to run with a `threads` option other than its running scheduler's; the
/// backend restarts the scheduler instead, so consecutive solves may change `threads`.
#[test]
fn threads_may_change_between_solves() {
    let mut alive = Vec::new();
    for threads in [0, 1, 2, 1, 0] {
        let mut be = HighsBackend::new(model());
        let mut cfg = quiet();
        cfg.solver_options
            .push(("threads".into(), OptionValue::Int(threads)));
        let out = be.solve(&cfg).unwrap();
        assert!(close(out.objective.unwrap(), 9.5), "threads {threads}");
        alive.push(be);
    }
}
