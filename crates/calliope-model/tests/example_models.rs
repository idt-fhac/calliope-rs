//! Ports of `tests/example_models/example_models_test.py`: the example models are solved
//! and checked against the values Calliope's test suite asserts (`pytest.approx`, relative
//! 1e-6, unless stated). The data-table variant of the national-scale test passes its
//! in-memory DataFrame as a [`Frame`] read from the same CSV file.

use std::path::PathBuf;

use calliope_array::{LArray, Label};
use calliope_model::{BuildOptions, Model, SolveOptions};
use calliope_preprocess::ReadOptions;
use calliope_preprocess::table::{Frame, read_csv};
use calliope_yaml::{AttrTree, Node, Value};

mod common;

fn example(name: &str) -> Option<PathBuf> {
    let p = common::reference()?.join(format!("src/calliope/example_models/{name}/model.yaml"));
    p.is_file().then_some(p)
}

fn subset(from: &str, to: &str) -> AttrTree {
    let mut t = AttrTree::new();
    t.set_key(
        "subset.timesteps",
        Node::new(Value::Seq(vec![
            Node::new(Value::Str(from.into())),
            Node::new(Value::Str(to.into())),
        ])),
    )
    .unwrap();
    t
}

fn run(
    name: &str,
    scenario: Option<&str>,
    overrides: Option<AttrTree>,
    init: AttrTree,
    solver_options: &[(&str, Value)],
) -> Option<Model> {
    let opts = ReadOptions {
        scenario: scenario.map(str::to_owned),
        override_dict: overrides,
        init_kwargs: init,
        ..ReadOptions::default()
    };
    let mut m = Model::from_yaml(&example(name)?, &opts).unwrap();
    m.build(&BuildOptions::default()).unwrap();
    let mut o = AttrTree::new();
    o.set_key("solver_options.output_flag", Node::new(Value::Bool(false)))
        .unwrap();
    for (k, v) in solver_options {
        o.set_key(&format!("solver_options.{k}"), Node::new(v.clone()))
            .unwrap();
    }
    m.solve(&SolveOptions {
        overrides: o,
        ..SolveOptions::default()
    })
    .unwrap();
    assert_eq!(m.runtime.termination_condition.as_deref(), Some("optimal"));
    Some(m)
}

/// `results[name].sel(dim=label, ...)` summed over the remaining dims (`.sum()` of a
/// selection, or `.item()` of a single cell).
fn sel_sum(m: &Model, name: &str, sel: &[(&str, &str)]) -> f64 {
    let mut a: LArray<f64> = m.results.get(name).unwrap().clone();
    for (d, l) in sel {
        a = a.sel_one(d, &Label::str(l)).unwrap();
    }
    a.cells().map(|(_, v)| *v).filter(|v| !v.is_nan()).sum()
}

#[track_caller]
fn approx(got: f64, want: f64, rel: f64, abs: f64) {
    assert!(
        (got - want).abs() <= (rel * want.abs()).max(abs),
        "{got} != {want} (rel {rel}, abs {abs})"
    );
}

fn timesteps_hourly(m: &Model, first: i64, hours: usize) {
    let ts = m
        .results
        .get("flow_out")
        .unwrap()
        .coord("timesteps")
        .unwrap()
        .labels()
        .to_vec();
    let want: Vec<Label> = (0..hours)
        .map(|h| Label::Datetime(first + h as i64 * 3_600_000_000_000))
        .collect();
    assert_eq!(ts, want);
}

const JAN_1: i64 = 1_104_537_600_000_000_000;
const JUL_1: i64 = 1_120_176_000_000_000_000;

#[test]
fn national_scale_sense_checks() {
    let Some(m) = run(
        "national_scale",
        None,
        None,
        subset("2005-01-01", "2005-01-01"),
        &[],
    ) else {
        return;
    };
    national_scale_checks(&m);
}

/// `nat_model_from_data_tables`: the model of `tests/example_models/
/// national_scale_from_data_tables`, with the time series given as an in-memory table
/// (`pd.read_csv(..., index_col=0, header=[0, 1, 2, 3])`).
#[test]
fn national_scale_from_data_tables_sense_checks() {
    let Some(root) = common::reference() else {
        return;
    };
    let path = root.join("tests/example_models/national_scale_from_data_tables/model.yaml");
    let csv =
        root.join("src/calliope/example_models/national_scale/data_tables/time_varying_params.csv");
    let df: Frame = read_csv(&csv, 4, Some(1)).unwrap();
    let mut opts = ReadOptions {
        init_kwargs: subset("2005-01-01", "2005-01-01"),
        ..ReadOptions::default()
    };
    opts.data_table_dfs.insert("time_varying_df".into(), df);
    let mut m = Model::from_yaml(&path, &opts).unwrap();
    m.build(&BuildOptions::default()).unwrap();
    let mut o = AttrTree::new();
    o.set_key("solver_options.output_flag", Node::new(Value::Bool(false)))
        .unwrap();
    m.solve(&SolveOptions {
        overrides: o,
        ..SolveOptions::default()
    })
    .unwrap();
    national_scale_checks(&m);
}

/// The data-table model with every table as a Parquet file (`fixtures/parquet_model`,
/// written by pandas): same inputs as with CSV files and an in-memory table, also when the
/// time series has a `DatetimeIndex`; then the sense checks.
#[test]
fn parquet_tables_match_csv_tables() {
    let Some(root) = common::reference() else {
        return;
    };
    let csv_model = root.join("tests/example_models/national_scale_from_data_tables/model.yaml");
    let csv =
        root.join("src/calliope/example_models/national_scale/data_tables/time_varying_params.csv");
    let mut opts = ReadOptions {
        init_kwargs: subset("2005-01-01", "2005-01-01"),
        ..ReadOptions::default()
    };
    opts.data_table_dfs.insert(
        "time_varying_df".into(),
        read_csv(&csv, 4, Some(1)).unwrap(),
    );
    let want = Model::from_yaml(&csv_model, &opts).unwrap();

    let parquet_model =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/parquet_model/model.yaml");
    let opts = ReadOptions {
        init_kwargs: subset("2005-01-01", "2005-01-01"),
        ..ReadOptions::default()
    };
    let mut got = Model::from_yaml(&parquet_model, &opts).unwrap();
    let diffs = calliope_io::compare_datasets(&want.inputs, &got.inputs, 0.0);
    assert!(diffs.is_empty(), "{diffs:#?}");

    let mut over = AttrTree::new();
    over.set_key(
        "data_tables.time_varying_data_from_df.table",
        Node::new(Value::Str(
            "data_tables/time_varying_datetime.parquet".into(),
        )),
    )
    .unwrap();
    let opts = ReadOptions {
        override_dict: Some(over),
        init_kwargs: subset("2005-01-01", "2005-01-01"),
        ..ReadOptions::default()
    };
    let dt = Model::from_yaml(&parquet_model, &opts).unwrap();
    let diffs = calliope_io::compare_datasets(&want.inputs, &dt.inputs, 0.0);
    assert!(diffs.is_empty(), "{diffs:#?}");

    got.build(&BuildOptions::default()).unwrap();
    let mut o = AttrTree::new();
    o.set_key("solver_options.output_flag", Node::new(Value::Bool(false)))
        .unwrap();
    got.solve(&SolveOptions {
        overrides: o,
        ..SolveOptions::default()
    })
    .unwrap();
    national_scale_checks(&got);
}

#[test]
fn missing_in_memory_table_is_an_error() {
    let Some(root) = common::reference() else {
        return;
    };
    let path = root.join("tests/example_models/national_scale_from_data_tables/model.yaml");
    let err = Model::from_yaml(&path, &ReadOptions::default()).unwrap_err();
    assert!(err.to_string().contains("time_varying_df"), "{err}");
}

fn national_scale_checks(m: &Model) {
    let r = 1e-6;
    approx(
        sel_sum(
            m,
            "storage_cap",
            &[("nodes", "region1_1"), ("techs", "csp")],
        ),
        45129.950,
        r,
        1e-12,
    );
    approx(
        sel_sum(
            m,
            "storage_cap",
            &[("nodes", "region2"), ("techs", "battery")],
        ),
        6675.173,
        r,
        1e-12,
    );
    approx(
        sel_sum(m, "flow_cap", &[("nodes", "region1_1"), ("techs", "csp")]),
        4626.588,
        r,
        1e-12,
    );
    approx(
        sel_sum(m, "flow_cap", &[("nodes", "region2"), ("techs", "battery")]),
        1000.0,
        r,
        1e-12,
    );
    approx(
        sel_sum(m, "flow_cap", &[("nodes", "region1"), ("techs", "ccgt")]),
        30000.0,
        r,
        1e-12,
    );
    approx(sel_sum(m, "cost", &[]), 38988.7442, r, 1e-12);
    approx(
        sel_sum(
            m,
            "systemwide_levelised_cost",
            &[("carriers", "power"), ("techs", "battery")],
        ),
        0.063543,
        0.0,
        1e-6,
    );
    approx(
        sel_sum(
            m,
            "systemwide_capacity_factor",
            &[("carriers", "power"), ("techs", "battery")],
        ),
        0.2642256,
        0.0,
        1e-6,
    );
}

#[test]
fn national_scale_operate() {
    let Some(m) = run(
        "national_scale",
        Some("operate"),
        None,
        subset("2005-01-01", "2005-01-03"),
        &[],
    ) else {
        return;
    };
    timesteps_hourly(&m, JAN_1, 72);
}

#[test]
fn national_scale_resampled_sense_checks() {
    let Some(m) = run(
        "national_scale",
        Some("time_resampling"),
        None,
        subset("2005-01-01", "2005-01-01"),
        &[],
    ) else {
        return;
    };
    let r = 1e-6;
    approx(
        sel_sum(
            &m,
            "storage_cap",
            &[("nodes", "region1_1"), ("techs", "csp")],
        ),
        23563.444,
        r,
        1e-12,
    );
    approx(
        sel_sum(
            &m,
            "storage_cap",
            &[("nodes", "region2"), ("techs", "battery")],
        ),
        6315.78947,
        r,
        1e-12,
    );
    approx(
        sel_sum(&m, "flow_cap", &[("nodes", "region1_1"), ("techs", "csp")]),
        1440.8377,
        r,
        1e-12,
    );
    approx(
        sel_sum(
            &m,
            "flow_cap",
            &[("nodes", "region2"), ("techs", "battery")],
        ),
        1000.0,
        r,
        1e-12,
    );
    approx(
        sel_sum(&m, "flow_cap", &[("nodes", "region1"), ("techs", "ccgt")]),
        30000.0,
        r,
        1e-12,
    );
    approx(sel_sum(&m, "cost", &[]), 37344.221869, r, 1e-12);
    approx(
        sel_sum(
            &m,
            "systemwide_levelised_cost",
            &[("carriers", "power"), ("techs", "battery")],
        ),
        0.063543,
        0.0,
        1e-6,
    );
    approx(
        sel_sum(
            &m,
            "systemwide_capacity_factor",
            &[("carriers", "power"), ("techs", "battery")],
        ),
        0.25,
        0.0,
        1e-6,
    );
}

fn source_unit(unit: &str) -> AttrTree {
    let mut t = AttrTree::new();
    t.set_key("techs.pv.source_unit", Node::new(Value::Str(unit.into())))
        .unwrap();
    t.set_key(
        "data_tables.pv_resource.select.scaler",
        Node::new(Value::Str(unit.into())),
    )
    .unwrap();
    t
}

#[test]
fn urban_scale_sense_checks() {
    for unit in ["per_area", "per_cap"] {
        let Some(m) = run(
            "urban_scale",
            None,
            Some(source_unit(unit)),
            subset("2005-07-01", "2005-07-01"),
            &[],
        ) else {
            return;
        };
        let r = 1e-6;
        approx(
            sel_sum(
                &m,
                "flow_cap",
                &[
                    ("nodes", "X1"),
                    ("techs", "chp"),
                    ("carriers", "electricity"),
                ],
            ),
            250.090112,
            r,
            1e-12,
        );
        approx(
            sel_sum(
                &m,
                "flow_cap",
                &[("nodes", "X2"), ("techs", "N1_to_X2"), ("carriers", "heat")],
            ),
            182.19260,
            r,
            1e-12,
        );
        approx(
            sel_sum(
                &m,
                "flow_out",
                &[("carriers", "heat"), ("nodes", "X3"), ("techs", "boiler")],
            ),
            0.18720,
            r,
            1e-12,
        );
        approx(
            sel_sum(&m, "area_use", &[("nodes", "X2"), ("techs", "pv")]),
            830.064659,
            r,
            1e-12,
        );
        approx(sel_sum(&m, "flow_export", &[]), 122.7156, r, 1e-12);
        approx(sel_sum(&m, "cost", &[]), 430.082290, r, 1e-12);
    }
}

#[test]
fn urban_scale_milp() {
    let Some(m) = run(
        "urban_scale",
        Some("milp"),
        None,
        subset("2005-01-01", "2005-01-01"),
        // `mipgap: 0.001` (CBC) is HiGHS' `mip_rel_gap`.
        &[("mip_rel_gap", Value::Float(0.001))],
    ) else {
        return;
    };
    let r = 1e-6;
    approx(
        sel_sum(
            &m,
            "flow_cap",
            &[
                ("nodes", "X1"),
                ("techs", "chp"),
                ("carriers", "electricity"),
            ],
        ),
        300.0,
        0.0,
        0.0,
    );
    approx(
        sel_sum(
            &m,
            "flow_cap",
            &[("nodes", "X2"), ("techs", "N1_to_X2"), ("carriers", "heat")],
        ),
        188.363137,
        r,
        1e-12,
    );
    approx(
        sel_sum(
            &m,
            "flow_out",
            &[
                ("carriers", "gas"),
                ("nodes", "X1"),
                ("techs", "supply_gas"),
            ],
        ),
        12363.173036,
        r,
        1e-12,
    );
    approx(sel_sum(&m, "flow_export", &[]), 0.0, r, 1e-12);
    approx(
        sel_sum(
            &m,
            "purchased_units",
            &[("nodes", "X2"), ("techs", "boiler")],
        ),
        1.0,
        0.0,
        0.0,
    );
    approx(
        sel_sum(&m, "purchased_units", &[("nodes", "X1"), ("techs", "chp")]),
        1.0,
        0.0,
        0.0,
    );
    approx(sel_sum(&m, "operating_units", &[]), 24.0, 0.0, 0.0);
    approx(sel_sum(&m, "cost", &[]), 540.780779, r, 1e-12);
}

#[test]
fn urban_scale_operate() {
    let Some(m) = run(
        "urban_scale",
        Some("operate"),
        Some(source_unit("per_cap")),
        subset("2005-07-01", "2005-07-04"),
        &[],
    ) else {
        return;
    };
    timesteps_hourly(&m, JUL_1, 96);
}

#[test]
fn urban_scale_operate_milp() {
    let Some(m) = run(
        "urban_scale",
        Some("operate,milp"),
        None,
        subset("2005-01-01", "2005-01-01"),
        &[],
    ) else {
        return;
    };
    let r = 1e-6;
    approx(
        sel_sum(
            &m,
            "flow_out",
            &[("carriers", "heat"), ("techs", "chp"), ("nodes", "X1")],
        ),
        4234.869616,
        r,
        1e-12,
    );
    approx(sel_sum(&m, "flow_export", &[]), 309.77935138, r, 1e-12);
    approx(sel_sum(&m, "cost", &[]), 8471.15341812, r, 1e-12);
}
