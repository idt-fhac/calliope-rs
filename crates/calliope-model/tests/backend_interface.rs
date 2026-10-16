//! The backend interface: ports of the API-agnostic cases of
//! `tests/backend/backend_model_test.py` and `tests/backend/highs_backend_model_test.py`
//! (assertions on values, bounds and the problem, not on Python object types).

use std::path::PathBuf;

use calliope_array::{LArray, Label};
use calliope_lp::ComponentKind;
use calliope_math::Group;
use calliope_math::eval::Value;
use calliope_model::{BuildOptions, Model, SolveOptions};
use calliope_preprocess::ReadOptions;
use calliope_yaml::{AttrTree, Node, Value as YValue};

mod common;

const DUMMY_INT: f64 = 3_735_928_559.0; // 0xDEADBEEF

fn tree(pairs: &[(&str, YValue)]) -> AttrTree {
    let mut t = AttrTree::new();
    for (k, v) in pairs {
        t.set_key(k, Node::new(v.clone())).unwrap();
    }
    t
}

fn test_model() -> Option<PathBuf> {
    let p = common::reference()?.join("tests/common/test_model/model.yaml");
    p.is_file().then_some(p)
}

fn built(scenario: &str) -> Option<Model> {
    let opts = ReadOptions {
        scenario: Some(scenario.to_owned()),
        ..ReadOptions::default()
    };
    let mut m = Model::from_yaml(&test_model()?, &opts).unwrap();
    m.build(&BuildOptions::default()).unwrap();
    Some(m)
}

fn solve(m: &mut Model, extra: &[(&str, YValue)]) {
    let mut o = tree(&[("solver_options.output_flag", YValue::Bool(false))]);
    for (k, v) in extra {
        o.set_key(k, Node::new(v.clone())).unwrap();
    }
    m.solve(&SolveOptions {
        force: true,
        overrides: o,
        ..SolveOptions::default()
    })
    .unwrap();
}

/// `simple_supply,two_hours,investment_costs`, built (and solved).
fn simple_supply(solved: bool) -> Option<Model> {
    let mut m = built("simple_supply,two_hours,investment_costs")?;
    if solved {
        solve(&mut m, &[]);
    }
    Some(m)
}

fn values(a: &LArray<f64>) -> Vec<f64> {
    a.cells().map(|(_, v)| *v).collect()
}

fn num(v: &Value) -> LArray<f64> {
    v.to_num().unwrap()
}

#[test]
fn get_variable_values_and_bounds() {
    let Some(m) = simple_supply(true) else { return };
    let b = m.backend().unwrap();
    let flow_cap = b.get_variable("flow_cap").unwrap();
    assert!(flow_cap.count() > 0);
    assert!(values(&flow_cap).iter().all(|v| v.is_finite()));
    // `test_get_variable_bounds`: flow_out is bounded below by 0 only.
    let (lb, ub) = b.get_variable_bounds("flow_out").unwrap();
    assert!(lb.count() > 0);
    assert!(values(&lb).iter().all(|v| *v == 0.0));
    assert!(values(&ub).iter().all(|v| *v == f64::INFINITY));
    // Unknown names.
    assert!(
        b.get_variable("foo")
            .unwrap_err()
            .to_string()
            .contains("Unknown variable: foo")
    );
    assert!(b.get_parameter("foo").is_err());
}

#[test]
fn global_expressions_constraints_objectives() {
    let Some(m) = simple_supply(true) else { return };
    let b = m.backend().unwrap();
    let cost = b.get_global_expression("cost").unwrap();
    let total: f64 = values(&cost).iter().sum();
    let obj = b.get_objective("min_cost_optimisation").unwrap();
    assert!((total - obj).abs() <= 1e-9 * obj.abs().max(1.0));
    assert_eq!(
        obj,
        m.results.get("min_cost_optimisation").unwrap().get(&[])
    );
    // `get_constraint(as_backend_objs=False)`: system_balance is an equality, satisfied.
    let c = b.get_constraint("system_balance").unwrap();
    let body = c.body.unwrap();
    for ((lb, ub), x) in values(&c.lb).iter().zip(values(&c.ub)).zip(values(&body)) {
        assert_eq!(*lb, ub);
        assert!((x - lb).abs() < 1e-6);
    }
}

#[test]
fn update_variable_bounds() {
    for (min, max) in [
        (Some(DUMMY_INT), None),
        (None, Some(DUMMY_INT)),
        (Some(DUMMY_INT), Some(DUMMY_INT)),
    ] {
        let Some(mut m) = simple_supply(true) else {
            return;
        };
        let b = m.backend_mut().unwrap();
        let (lb0, ub0) = b.get_variable_bounds("flow_out").unwrap();
        b.update_variable_bounds(
            "flow_out",
            min.map(LArray::scalar).as_ref(),
            max.map(LArray::scalar).as_ref(),
        )
        .unwrap();
        let (lb, ub) = b.get_variable_bounds("flow_out").unwrap();
        match min {
            Some(v) => assert!(values(&lb).iter().all(|x| *x == v)),
            None => assert_eq!(lb, lb0),
        }
        match max {
            Some(v) => assert!(values(&ub).iter().all(|x| *x == v)),
            None => assert_eq!(ub, ub0),
        }
    }
}

#[test]
fn update_variable_bounds_with_an_array() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    // `test_update_variable_single_bound_multi_val`: broadcast over the missing dims.
    let demand = num(b.get_parameter("sink_use_equals").unwrap())
        .sel_one("techs", &Label::str("test_demand_elec"))
        .unwrap();
    b.update_variable_bounds("flow_in", Some(&demand), None)
        .unwrap();
    let (lb, _) = b.get_variable_bounds("flow_in").unwrap();
    let ax: Vec<usize> = demand
        .dims()
        .iter()
        .map(|d| lb.axis(d.name()).unwrap())
        .collect();
    for (codes, v) in lb.cells() {
        let own: Vec<u32> = demand
            .dims()
            .iter()
            .zip(&ax)
            .map(|(d, a)| d.code(lb.dims()[*a].label(codes[*a])).unwrap())
            .collect();
        assert_eq!(*v, demand.get(&own));
    }
}

#[test]
fn update_variable_bounds_set_by_parameter_is_an_error() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    let e = m
        .backend_mut()
        .unwrap()
        .update_variable_bounds("flow_cap", None, Some(&LArray::scalar(1.0)))
        .unwrap_err()
        .to_string();
    assert_eq!(
        e,
        "Cannot update variable bounds that have been set by parameters. Use `update_input('flow_cap_max')` to update the max bound of flow_cap."
    );
}

#[test]
fn fix_and_unfix_variable() {
    let Some(mut m) = simple_supply(false) else {
        return;
    };
    // `test_fix_variable_before_solve`
    let e = m
        .backend_mut()
        .unwrap()
        .fix_variable("flow_cap", None)
        .unwrap_err()
        .to_string();
    assert!(e.contains(
        "Cannot fix variable values without already having solved the model successfully."
    ));
    solve(&mut m, &[]);
    let b = m.backend_mut().unwrap();
    let solved = b.get_variable("flow_cap").unwrap();
    let before = b.get_variable_bounds("flow_cap").unwrap();
    b.fix_variable("flow_cap", None).unwrap();
    let (lb, ub) = b.get_variable_bounds("flow_cap").unwrap();
    assert_eq!(lb, ub);
    assert_eq!(lb, solved);
    // Python's HiGHS backend cannot unfix; the port restores the original bounds.
    b.unfix_variable("flow_cap", None).unwrap();
    assert_eq!(b.get_variable_bounds("flow_cap").unwrap(), before);
}

#[test]
fn fix_variable_where() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    // `test_fix_variable_where`: only where flow_cap_max is defined.
    let max = num(b.get_parameter("flow_cap_max").unwrap());
    let where_ = max.map(|v| !v.is_nan());
    b.fix_variable("flow_cap", Some(&where_)).unwrap();
    let (lb, ub) = b.get_variable_bounds("flow_cap").unwrap();
    let techs = lb.axis("techs").unwrap();
    for ((codes, l), (_, u)) in lb.cells().zip(ub.cells()) {
        let tech = lb.dims()[techs].label(codes[techs]).clone();
        if tech == Label::str("test_demand_elec") {
            assert_ne!(l, u);
        }
    }
}

#[test]
fn fix_variable_after_infeasible_solve_is_an_error() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    m.backend_mut()
        .unwrap()
        .update_input("flow_cap_max", &Value::num(0.0))
        .unwrap();
    solve(&mut m, &[]);
    assert_ne!(m.runtime.termination_condition.as_deref(), Some("optimal"));
    let e = m
        .backend_mut()
        .unwrap()
        .fix_variable("flow_cap", None)
        .unwrap_err()
        .to_string();
    assert!(e.contains("Cannot fix variable values"), "{e}");
}

#[test]
fn update_input_values_broadcast_and_dims() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    // `test_update_input`
    let eff = num(b.get_parameter("flow_out_eff").unwrap());
    let scaled = eff.map(|v| v * 1000.0);
    b.update_input("flow_out_eff", &Value::Num(scaled.clone()))
        .unwrap();
    assert_eq!(num(b.get_parameter("flow_out_eff").unwrap()), scaled);
    // `test_update_input_one_val`: a scalar is broadcast over the old dims.
    b.update_input("flow_out_eff", &Value::num(DUMMY_INT))
        .unwrap();
    let p = num(b.get_parameter("flow_out_eff").unwrap());
    assert!(p.count() > 0 && values(&p).iter().all(|v| *v == DUMMY_INT));
    // `test_update_input_no_refs_to_update`
    b.update_input("flow_cap_per_storage_cap_max", &Value::num(1.0))
        .unwrap();
    assert_eq!(
        b.get_parameter("flow_cap_per_storage_cap_max")
            .unwrap()
            .scalar_num(),
        Some(1.0)
    );
}

#[test]
fn update_input_references() {
    let Some(m) = simple_supply(true) else { return };
    let b = m.backend().unwrap();
    let built = |name: &str| {
        b.expression(name).is_some_and(|a| a.count() > 0)
            || b.get_constraint(name).is_ok()
            || b.lp().rows.iter().any(|r| &*r.label.component == name)
    };
    let affected = |input: &str| {
        let mut v: Vec<String> = b
            .dependencies()
            .affected_by_input(input)
            .into_iter()
            .filter(|(g, _)| *g != Group::Postprocessed)
            .map(|(_, n)| n)
            .filter(|n| built(n))
            .collect();
        v.sort();
        v
    };
    // `test_update_input_add_dim` / `test_update_input_replace_undefined`
    assert_eq!(
        affected("flow_out_eff"),
        [
            "balance_supply_no_storage",
            "balance_transmission",
            "flow_out_inc_eff"
        ]
    );
    assert_eq!(affected("source_eff"), ["balance_supply_no_storage"]);
}

#[test]
fn update_input_rebuilds_references_into_the_objective() {
    let Some(mut m) = simple_supply(false) else {
        return;
    };
    let demand_cols = |m: &Model| -> Vec<usize> {
        m.backend()
            .unwrap()
            .lp()
            .cols
            .iter()
            .enumerate()
            .filter(|(_, c)| c.label.index.iter().any(|i| &**i == "test_demand_elec"))
            .map(|(i, _)| i)
            .collect()
    };
    let mentions_demand = |m: &Model, name: &str| -> bool {
        let cols = demand_cols(m);
        m.backend()
            .unwrap()
            .expression(name)
            .unwrap()
            .cells()
            .any(|(_, l)| l.terms.iter().any(|(c, _)| cols.contains(&(*c as usize))))
    };
    for e in ["cost_investment_flow_cap", "cost_investment", "cost"] {
        assert!(!mentions_demand(&m, e), "{e}");
    }
    // Calliope keeps zero-coefficient terms in the objective expression (and so does the port).
    let objective = |m: &Model| m.backend().unwrap().lp().objective.expr.terms.clone();
    let cols = demand_cols(&m);
    assert!(!objective(&m).iter().any(|(c, _)| cols.contains(c)));
    // `built_model_func_updated_cost_flow_cap`
    m.backend_mut()
        .unwrap()
        .update_input("cost_flow_cap", &Value::num(DUMMY_INT))
        .unwrap();
    for e in ["cost_investment_flow_cap", "cost_investment", "cost"] {
        assert!(mentions_demand(&m, e), "{e}");
    }
    let cols = demand_cols(&m);
    assert!(objective(&m).iter().any(|(c, _)| cols.contains(c)));
}

#[test]
fn update_input_rebuild_and_resolve() {
    // `test_update_input_rebuild_and_resolve`
    let Some(mut m) = simple_supply(false) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    b.update_input("flow_out_eff", &Value::num(0.5)).unwrap();
    b.update_input("flow_out_eff", &Value::num(0.9)).unwrap();
    solve(&mut m, &[]);
    let mut fresh = simple_supply(false).unwrap();
    fresh
        .backend_mut()
        .unwrap()
        .update_input("flow_out_eff", &Value::num(0.9))
        .unwrap();
    solve(&mut fresh, &[]);
    assert_eq!(m.runtime.termination_condition.as_deref(), Some("optimal"));
    let sum = |m: &Model| values(m.results.get("cost").unwrap()).iter().sum::<f64>();
    assert!((sum(&m) - sum(&fresh)).abs() < 1e-9);
}

#[test]
fn update_input_in_place_matches_a_fresh_build() {
    // After a solve, a value-only change (same sparsity) is applied to HiGHS in place; the
    // problem and its solution must equal a backend built from the updated inputs.
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    m.backend_mut()
        .unwrap()
        .update_input("cost_flow_cap", &Value::num(20.0))
        .unwrap();
    solve(&mut m, &[]);
    let b = m.backend().unwrap();
    let mut fresh =
        calliope_model::Backend::new(b.inputs().clone(), b.math().clone(), b.config().clone())
            .unwrap();
    let mut solve_cfg = m.config.solve.clone();
    solve_cfg
        .solver_options
        .insert("output_flag".into(), Node::new(YValue::Bool(false)));
    let r = fresh.solve(&solve_cfg, false).unwrap();
    let a = m.backend_mut().unwrap();
    a.verbose_strings();
    fresh.verbose_strings();
    assert_eq!(a.lp_string(), fresh.lp_string());
    let obj = |r: &calliope_model::Results| r.get("min_cost_optimisation").unwrap().get(&[]);
    assert!((obj(&m.results) - obj(&r)).abs() <= 1e-9 * obj(&r).abs());
}

#[test]
fn set_objective_and_constraint_activation() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    assert_eq!(b.objective(), "min_cost_optimisation");
    assert!(b.set_objective("foo").is_err());
    // Deactivating the demand balance frees its rows: nothing needs to be produced.
    b.set_constraint_active("balance_demand", false).unwrap();
    solve(&mut m, &[]);
    assert_eq!(
        m.results.get("min_cost_optimisation").unwrap().get(&[]),
        0.0
    );
    m.backend_mut()
        .unwrap()
        .set_constraint_active("balance_demand", true)
        .unwrap();
    solve(&mut m, &[]);
    assert!(m.results.get("min_cost_optimisation").unwrap().get(&[]) > 0.0);
}

#[test]
fn delete_component() {
    let Some(mut m) = simple_supply(false) else {
        return;
    };
    let b = m.backend_mut().unwrap();
    b.verbose_strings();
    let rows = b.lp().num_rows();
    // `test_delete_missing_or_mismatched_is_noop`
    b.delete_component("not_in_dataset", Group::Constraints)
        .unwrap();
    b.delete_component("flow_cap", Group::Constraints).unwrap();
    assert_eq!(b.lp().num_rows(), rows);
    // `test_delete_constraint_shifts_row_indices`
    b.delete_component("balance_demand", Group::Constraints)
        .unwrap();
    assert!(b.get_constraint("balance_demand").is_err());
    assert!(
        b.lp()
            .rows_of(ComponentKind::Constraint, "balance_demand")
            .is_empty()
    );
    let c = b.get_constraint("system_balance").unwrap();
    assert!(c.lb.count() > 0);
    // `test_delete_variable_shifts_col_indices`
    b.delete_component("flow_in", Group::Variables).unwrap();
    assert!(b.get_variable_bounds("flow_in").is_err());
    let cols = b.variable_columns("flow_out").unwrap();
    for (_, c) in cols.cells() {
        assert_eq!(&*b.lp().cols[*c as usize].label.component, "flow_out");
    }
    // `test_delete_variable_shifts_expression_indices`: cost_investment keeps flow_cap.
    b.delete_component("flow_out", Group::Variables).unwrap();
    let ci = b.expression("cost_investment").unwrap();
    assert!(ci.cells().any(|(_, l)| {
        l.terms
            .iter()
            .any(|(c, _)| &*b.lp().cols[*c as usize].label.component == "flow_cap")
    }));
    solve(&mut m, &[]);
    assert_eq!(m.runtime.termination_condition.as_deref(), Some("optimal"));
}

#[test]
fn shadow_prices() {
    let Some(mut m) = simple_supply(false) else {
        return;
    };
    let track = YValue::Seq(vec![
        Node::new(YValue::Str("system_balance".into())),
        Node::new(YValue::Str("balance_demand".into())),
    ]);
    solve(&mut m, &[("shadow_prices", track.clone())]);
    let sb = m.results.get("shadow_price_system_balance").unwrap();
    assert!(sb.count() > 0 && values(sb).iter().all(|v| !v.is_nan()));
    // `test_get_shadow_price_some_nan`: defined where the constraint is.
    let bd = m.results.get("shadow_price_balance_demand").unwrap();
    let full: usize = bd.dims().iter().map(calliope_array::Coord::len).product();
    assert!(bd.count() > 0 && bd.count() < full);
    // `test_get_shadow_price_empty_milp`
    let Some(mut milp) = built("supply_milp,two_hours,investment_costs") else {
        return;
    };
    assert!(milp.backend().unwrap().has_integer_or_binary_variables());
    solve(&mut milp, &[("shadow_prices", track)]);
    assert!(milp.results.get("shadow_price_system_balance").is_none());
}

#[test]
fn solver_options_logs_and_lp() {
    let Some(mut m) = simple_supply(true) else {
        return;
    };
    assert!(!m.backend().unwrap().has_integer_or_binary_variables());
    let dir = tempfile::tempdir().unwrap();
    let logs = dir.path().join("logs");
    solve(
        &mut m,
        &[
            ("save_logs", YValue::Str(logs.display().to_string())),
            ("solver_options.time_limit", YValue::Float(100.0)),
        ],
    );
    assert!(logs.join("highs.log").is_file());
    let b = m.backend_mut().unwrap();
    let e = b
        .to_lp(&dir.path().join("out.txt"))
        .unwrap_err()
        .to_string();
    assert_eq!(e, "File extension must be `.lp`");
    b.verbose_strings();
    let lp = dir.path().join("out.lp");
    b.to_lp(&lp).unwrap();
    let text = std::fs::read_to_string(&lp).unwrap();
    assert!(text.contains("min") && text.contains("variables(flow_cap)"));
}
