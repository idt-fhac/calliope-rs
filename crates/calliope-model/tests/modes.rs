//! Operate and SPORES mode: ports of `tests/model_test.py` (`TestOperateMode`,
//! `TestSporesMode`) that assert on results, plus the corrected Calliope bugs
//! (bench/BUGS.md #1, #2, #8). Log-text assertions are not ported.

use std::path::PathBuf;

use calliope_array::Label;
use calliope_model::{BuildOptions, Model, SolveOptions};
use calliope_preprocess::ReadOptions;
use calliope_yaml::{AttrTree, Node, Value};

mod common;

fn test_model() -> Option<PathBuf> {
    let p = common::reference()?.join("tests/common/test_model/model.yaml");
    p.is_file().then_some(p)
}

fn tree(pairs: &[(&str, Value)]) -> AttrTree {
    let mut t = AttrTree::new();
    for (k, v) in pairs {
        t.set_key(k, Node::new(v.clone())).unwrap();
    }
    t
}

fn s(x: &str) -> Value {
    Value::Str(x.to_owned())
}

fn quiet() -> AttrTree {
    tree(&[("solver_options.output_flag", Value::Bool(false))])
}

fn model(scenario: &str, overrides: &[(&str, Value)]) -> Option<Model> {
    let path = test_model()?;
    let opts = ReadOptions {
        scenario: Some(scenario.to_owned()),
        override_dict: (!overrides.is_empty()).then(|| tree(overrides)),
        ..ReadOptions::default()
    };
    Some(Model::from_yaml(&path, &opts).unwrap())
}

fn solve(m: &mut Model, extra: &[(&str, Value)]) {
    let mut o = quiet();
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

fn timesteps(m: &Model) -> Vec<Label> {
    m.results
        .get("flow_out")
        .unwrap()
        .coord("timesteps")
        .unwrap()
        .labels()
        .to_vec()
}

const OPERATE: &str = "simple_supply,operate,var_costs,investment_costs";

/// `base_model` + `operate_model_and_log`: capacities from a base-mode solve fixed as
/// inputs of an operate-mode model.
fn operate_model(window: &str, horizon: &str) -> Option<Model> {
    let mut base = model(OPERATE, &[("config.init.mode", s("base"))])?;
    base.build(&BuildOptions::default()).unwrap();
    solve(&mut base, &[]);
    let inputs = base.inputs_with_results().unwrap();
    let mut m = base
        .with_inputs(inputs, None, &tree(&[("mode", s("operate"))]))
        .unwrap();
    m.build(&BuildOptions {
        overrides: tree(&[
            ("operate.window", s(window)),
            ("operate.horizon", s(horizon)),
        ]),
        ..BuildOptions::default()
    })
    .unwrap();
    solve(&mut m, &[]);
    Some(m)
}

#[test]
fn operate_windows_cover_the_time_series() {
    for (w, h) in [("6h", "12h"), ("12h", "12h"), ("16h", "20h")] {
        let Some(mut m) = operate_model(w, h) else {
            return;
        };
        assert_eq!(m.runtime.termination_condition.as_deref(), Some("optimal"));
        // `test_operate_timeseries`: 2005-01-01 00:00 .. 2005-01-02 23:00, hourly.
        let ts = timesteps(&m);
        assert_eq!(ts.len(), 48, "{w}/{h}");
        let hour = 3_600_000_000_000;
        for (i, l) in ts.iter().enumerate() {
            assert_eq!(
                *l,
                Label::Datetime(1_104_537_600_000_000_000 + i as i64 * hour)
            );
        }
        // Scalar results get the timesteps dimension, as with xr.concat.
        let obj = m.results.get("min_cost_optimisation").unwrap();
        assert_eq!(obj.dim_names(), ["timesteps"]);
        // Solving again starts from the first window and gives the same results.
        let before = m.results.clone();
        solve(&mut m, &[]);
        assert_eq!(m.results, before);
    }
}

#[test]
fn operate_single_window_is_solved() {
    // bench/BUGS.md #1: Python raises UnboundLocalError when one window covers it all.
    let Some(mut m) = model(
        "simple_supply,operate,var_costs,investment_costs",
        &[
            ("config.build.operate.window", s("48h")),
            ("config.build.operate.horizon", s("48h")),
        ],
    ) else {
        return;
    };
    m.build(&BuildOptions::default()).unwrap();
    solve(&mut m, &[]);
    assert_eq!(m.runtime.termination_condition.as_deref(), Some("optimal"));
    assert_eq!(timesteps(&m).len(), 48);
}

#[test]
fn operate_horizon_shorter_than_window_is_rejected() {
    // bench/BUGS.md #8: Python silently drops the timesteps no horizon covers.
    let Some(mut m) = model(
        "simple_supply,operate,var_costs,investment_costs",
        &[
            ("config.build.operate.window", s("12h")),
            ("config.build.operate.horizon", s("6h")),
        ],
    ) else {
        return;
    };
    let e = m.build(&BuildOptions::default()).unwrap_err().to_string();
    assert!(e.contains("must be at least as long as its window"), "{e}");
}

#[test]
fn operate_not_allowed_with_time_clustering() {
    let Some(mut m) = model(
        "simple_supply,two_hours,investment_costs",
        &[("config.init.mode", s("operate"))],
    ) else {
        return;
    };
    m.config = m
        .config
        .update(&tree(&[("init.time_cluster", s("foo"))]), true)
        .unwrap();
    let e = m.build(&BuildOptions::default()).unwrap_err().to_string();
    assert!(e.contains("Unable to run this model in operate"), "{e}");
}

const SPORES: &str = "spores,var_costs,two_hours,simple_supply_spores_ready,investment_costs";

fn spores_model(extra_scenario: &str, solve_overrides: &[(&str, Value)]) -> Option<Model> {
    let mut m = model(&format!("{SPORES}{extra_scenario}"), &[])?;
    m.build(&BuildOptions::default()).unwrap();
    solve(&mut m, solve_overrides);
    Some(m)
}

fn spores_labels(m: &Model) -> Vec<Label> {
    m.results
        .get("flow_cap")
        .unwrap()
        .coord("spores")
        .unwrap()
        .labels()
        .to_vec()
}

/// `flow_cap` / `spores_score_cumulative` at one SPORE, densely over the other dims.
fn at_spore(m: &Model, name: &str, spore: i64) -> Vec<f64> {
    m.results
        .get(name)
        .unwrap()
        .sel_one("spores", &Label::Int(spore))
        .unwrap()
        .to_dense()
}

fn baseline_cost() -> Option<f64> {
    let mut m = model(
        "var_costs,simple_supply_spores_ready,two_hours,investment_costs",
        &[],
    )?;
    m.build(&BuildOptions::default()).unwrap();
    solve(&mut m, &[]);
    Some(m.results.get("min_cost_optimisation").unwrap().get(&[]))
}

#[test]
fn spores_all_algorithms() {
    for alg in [
        "integer",
        "relative_deployment",
        "random",
        "evolving_average",
    ] {
        let Some(m) = spores_model("", &[("spores.scoring_algorithm", s(alg))]) else {
            return;
        };
        assert_eq!(
            m.runtime.termination_condition.as_deref(),
            Some("optimal"),
            "{alg}"
        );
        assert_eq!(m.backend().unwrap().objective(), "min_spores");
        assert_eq!(
            spores_labels(&m),
            [Label::Int(0), Label::Int(1), Label::Int(2)]
        );
        // `test_spores_caps`: capacities change between SPORES.
        let caps: Vec<Vec<f64>> = (0..3).map(|i| at_spore(&m, "flow_cap", i)).collect();
        assert!(caps[0] != caps[1] || caps[1] != caps[2], "{alg}");
    }
}

#[test]
fn spores_constraining_cost_is_baseline_objective() {
    let Some(base) = baseline_cost() else { return };
    let m = spores_model("", &[]).unwrap();
    let b = m
        .backend()
        .unwrap()
        .get_parameter("spores_baseline_cost")
        .unwrap();
    assert_eq!(b.scalar_num(), Some(base));
}

#[test]
fn spores_integer_scores_never_decrease_and_follow_capacity() {
    let Some(m) = spores_model("", &[]) else {
        return;
    };
    for spore in 1..3 {
        let prev = at_spore(&m, "spores_score_cumulative", spore - 1);
        let cur = at_spore(&m, "spores_score_cumulative", spore);
        let cap = at_spore(&m, "flow_cap", spore - 1);
        for ((p, c), k) in prev.iter().zip(&cur).zip(&cap) {
            let (p, c) = (
                if p.is_nan() { 0.0 } else { *p },
                if c.is_nan() { 0.0 } else { *c },
            );
            assert!(c >= p);
            // `test_spores_scores_increasing_with_cap_integer_algo`
            assert_eq!(c > p, *k > 0.0);
        }
    }
}

#[test]
fn spores_tracking_parameter() {
    for alg in [
        "integer",
        "relative_deployment",
        "random",
        "evolving_average",
    ] {
        let Some(m) = spores_model(
            ",spores_tech_tracking",
            &[("spores.scoring_algorithm", s(alg))],
        ) else {
            return;
        };
        let score = m.results.get("spores_score_cumulative").unwrap();
        let techs = score.coord("techs").unwrap().clone();
        let ax = score.axis("techs").unwrap();
        for (codes, v) in score.cells() {
            if techs.label(codes[ax]) != &Label::str("test_supply_elec") {
                assert_eq!(*v, 0.0, "{alg}");
            }
        }
    }
}

#[test]
fn spores_tracking_parameter_false_is_not_tracked() {
    // bench/BUGS.md #2: Python tracks techs set to `false` too (`.notnull()`).
    let Some(m) = spores_model(",spores_tech_tracking", &[]) else {
        return;
    };
    let tracked = m.results.get("spores_score_cumulative").unwrap().clone();
    let mut m2 = model(
        &format!("{SPORES},spores_tech_tracking"),
        &[(
            "data_definitions.spores_tracker",
            Value::Map(tree(&[
                (
                    "data",
                    Value::Seq(vec![
                        Node::new(Value::Bool(true)),
                        Node::new(Value::Bool(false)),
                    ]),
                ),
                (
                    "index",
                    Value::Seq(vec![
                        Node::new(s("test_supply_elec")),
                        Node::new(s("test_supply_elec_ineff")),
                    ]),
                ),
                ("dims", s("techs")),
            ])),
        )],
    )
    .unwrap();
    m2.build(&BuildOptions::default()).unwrap();
    solve(&mut m2, &[]);
    assert_eq!(m2.results.get("spores_score_cumulative").unwrap(), &tracked);
}

#[test]
fn spores_continue_from_baseline_and_from_spores() {
    let Some(all) = spores_model("", &[]) else {
        return;
    };
    // `spores_model_continue_from_plan_and_log`: baseline solved in base mode first.
    let mut base = model(SPORES, &[("config.init.mode", s("base"))]).unwrap();
    base.build(&BuildOptions::default()).unwrap();
    solve(&mut base, &[]);
    let mut m2 = base
        .with_inputs(
            base.inputs.clone(),
            Some(base.results.clone()),
            &tree(&[("mode", s("spores"))]),
        )
        .unwrap();
    m2.build(&BuildOptions::default()).unwrap();
    solve(&mut m2, &[("spores.use_latest_results", Value::Bool(true))]);
    // `test_skip_cost_op_same_result`
    assert_eq!(m2.results.get("flow_cap"), all.results.get("flow_cap"));

    // `spores_model_continue_from_spores_and_log`: two more SPORES.
    let mut more = spores_model("", &[]).unwrap();
    solve(
        &mut more,
        &[
            ("spores.use_latest_results", Value::Bool(true)),
            ("spores.number", Value::Int(4)),
        ],
    );
    assert_eq!(
        spores_labels(&more),
        (0..5).map(Label::Int).collect::<Vec<_>>()
    );
    for i in 0..3 {
        assert_eq!(
            at_spore(&more, "flow_cap", i),
            at_spore(&all, "flow_cap", i)
        );
    }
    // `test_continue_from_spores_number_not_big_enough`
    let mut again = spores_model("", &[]).unwrap();
    let e = again
        .solve(&SolveOptions {
            force: true,
            overrides: tree(&[("spores.use_latest_results", Value::Bool(true))]),
            ..SolveOptions::default()
        })
        .unwrap_err()
        .to_string();
    assert!(e.contains("Cannot restart SPORES from SPORE 2"), "{e}");
}

#[test]
fn spores_fail_without_baseline() {
    let Some(mut m) = model(SPORES, &[]) else {
        return;
    };
    m.build(&BuildOptions::default()).unwrap();
    let e = m
        .solve(&SolveOptions {
            overrides: tree(&[("spores.use_latest_results", Value::Bool(true))]),
            ..SolveOptions::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("Cannot run SPORES without baseline results."),
        "{e}"
    );
}

#[test]
fn spores_relative_deployment_needs_max_param() {
    let Some(mut m) = model(
        &format!("{SPORES},spores_tech_tracking"),
        &[(
            "techs.test_supply_elec.flow_cap_max",
            Value::Float(f64::INFINITY),
        )],
    ) else {
        return;
    };
    m.build(&BuildOptions::default()).unwrap();
    let e = m
        .solve(&SolveOptions {
            overrides: tree(&[("spores.scoring_algorithm", s("relative_deployment"))]),
            ..SolveOptions::default()
        })
        .unwrap_err()
        .to_string();
    assert!(
        e.contains("Cannot score SPORES with `relative_deployment`"),
        "{e}"
    );
}

#[test]
fn spores_save_per_spore_and_netcdf_round_trip() {
    let dir = tempfile::tempdir().unwrap();
    let Some(m) = spores_model(
        "",
        &[(
            "spores.save_per_spore_path",
            s(&dir.path().display().to_string()),
        )],
    ) else {
        return;
    };
    for spore in 0..3_i64 {
        let f = Model::from_netcdf(&dir.path().join(format!("spore_{spore}.nc"))).unwrap();
        let fc = f.results.get("flow_cap").unwrap();
        assert_eq!(fc.coord("spores").unwrap().labels(), [Label::Int(spore)]);
        let bits = |v: Vec<f64>| v.into_iter().map(f64::to_bits).collect::<Vec<_>>();
        assert_eq!(
            bits(fc.sel_one("spores", &Label::Int(spore)).unwrap().to_dense()),
            bits(at_spore(&m, "flow_cap", spore))
        );
        // Inputs are saved with SPORE 0 only.
        assert_eq!(f.inputs.vars.is_empty(), spore != 0);
    }
    // `test_io_load`
    let path = dir.path().join("all.nc");
    m.to_netcdf(&path).unwrap();
    let back = Model::from_netcdf(&path).unwrap();
    let bits = |v: Vec<f64>| v.into_iter().map(f64::to_bits).collect::<Vec<_>>();
    for (k, v) in &m.results.arrays {
        let got = back.results.get(k).unwrap().to_dense();
        assert_eq!(bits(got), bits(v.to_dense()), "{k}");
    }
}

#[test]
fn spores_break_on_infeasible() {
    let Some(path) = test_model() else { return };
    let mut math = AttrTree::new();
    math.set_key(
        "objectives.min_spores.equations",
        Node::new(Value::Seq(vec![Node::new(Value::Map(tree(&[(
            "expression",
            s("sum(flow_cap * -1 * spores_score, over=[nodes, techs, carriers])"),
        )])))])),
    )
    .unwrap();
    let mut md = AttrTree::new();
    md.insert("custom_math", Node::new(Value::Map(math)));
    let opts = ReadOptions {
        scenario: Some(SPORES.to_owned()),
        math_dict: Some(md),
        init_kwargs: tree(&[("extra_math", Value::Seq(vec![Node::new(s("custom_math"))]))]),
        ..ReadOptions::default()
    };
    let mut m = Model::from_yaml(&path, &opts).unwrap();
    m.build(&BuildOptions::default()).unwrap();
    let dir = tempfile::tempdir().unwrap();
    solve(
        &mut m,
        &[(
            "spores.save_per_spore_path",
            s(&dir.path().display().to_string()),
        )],
    );
    assert_eq!(spores_labels(&m), [Label::Int(0)]);
    assert!(dir.path().join("spore_0.nc").is_file());
    assert!(!dir.path().join("spore_1.nc").exists());
}
