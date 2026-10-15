//! `CalliopeConfig` defaults, backend fallback and update behaviour.

mod common;

use calliope_schema::{Backend, CalliopeConfig, Init, Mode, SporesScoring, Validated};
use common::yaml;

#[test]
fn defaults_match_calliope() {
    let c = Validated::<CalliopeConfig>::default_model().unwrap();
    assert_eq!(c.build.backend, Backend::Pyomo);
    assert_eq!(c.solve.solver, "cbc");
    assert_eq!(c.init.mode, Mode::Base);
    assert_eq!(c.init.datetime_format, "ISO8601");
    assert_eq!(c.build.operate.window, "24h");
    assert_eq!(c.build.operate.horizon, "48h");
    assert_eq!(c.solve.spores.number, 3);
    assert_eq!(c.solve.spores.scoring_algorithm, SporesScoring::Integer);
    assert!((c.solve.zero_threshold - 1e-10).abs() < 1e-24);
    assert!(c.solve.postprocessing_active);
    assert_eq!(c.fields_set().count(), 0);
}

#[test]
fn backends() {
    for (name, falls_back) in [("pyomo", true), ("gurobi", true), ("highs", false)] {
        let c = Validated::<CalliopeConfig>::from_tree(yaml(&format!("build.backend: {name}")))
            .unwrap();
        assert_eq!(c.build.backend.falls_back_to_highs(), falls_back);
    }
    let err = Validated::<CalliopeConfig>::from_tree(yaml("build.backend: cplex")).unwrap_err();
    assert!(err.to_string().contains("build.backend"), "{err}");
}

#[test]
fn unknown_key_reports_path_and_title() {
    let err = Validated::<CalliopeConfig>::from_tree(yaml("init:\n  foo: 1\n")).unwrap_err();
    let text = err.to_string();
    assert!(
        text.starts_with("1 validation error for Model configuration schema"),
        "{text}"
    );
    assert!(text.contains("unknown field `foo`"), "{text}");
}

#[test]
fn pydantic_lax_scalars() {
    let i = Validated::<Init>::from_tree(yaml("{broadcast_input_data: 'yes', retain_inactive: 0}"))
        .unwrap();
    assert!(i.broadcast_input_data);
    assert!(!i.retain_inactive);
    // Numbers are not coerced to strings.
    assert!(Validated::<Init>::from_tree(yaml("{name: 1}")).is_err());
    // Unquoted dates are not strings (ruamel loads them as dates).
    assert!(
        Validated::<Init>::from_tree(yaml("{subset: {timesteps: [2005-01-01, '2005-01-02']}}"))
            .is_err()
    );
    let i = Validated::<Init>::from_tree(yaml("{subset: {timesteps: ['2005-01-01', 1]}}")).unwrap();
    assert_eq!(i.subset["timesteps"].as_ref().unwrap().len(), 2);
}

#[test]
fn score_threshold_factor_ge_zero() {
    let err =
        Validated::<CalliopeConfig>::from_tree(yaml("solve.spores.score_threshold_factor: -1"))
            .unwrap_err();
    assert!(
        err.to_string().contains("greater than or equal to 0"),
        "{err}"
    );
}

#[test]
fn config_update_like_model_init() {
    // `Model.__init__`: `config.update({"init": kwargs})`.
    let c = Validated::<CalliopeConfig>::from_tree(yaml(
        "init: {name: m, subset: {timesteps: ['2005-01-01', '2005-01-02']}}",
    ))
    .unwrap();
    let c2 = c
        .update(
            &yaml("init: {subset: {nodes: [a]}, resample.timesteps: 2h}"),
            true,
        )
        .unwrap();
    assert_eq!(c2.init.subset.len(), 2);
    assert_eq!(c2.init.resample["timesteps"].as_deref(), Some("2h"));
    assert_eq!(c2.init.name.as_deref(), Some("m"));
    // solver_options is a plain dict: replaced, not merged.
    let c3 = c2
        .update(&yaml("solve.solver_options: {threads: 1}"), true)
        .unwrap();
    let c4 = c3
        .update(&yaml("solve.solver_options: {presolve: 'on'}"), true)
        .unwrap();
    assert_eq!(c4.solve.solver_options.len(), 1);
}
