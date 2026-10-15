//! Results mapping against a golden `results.nc`: Pyomo-named LP → HiGHS → labelled arrays.

use calliope_array::Coord;
use calliope_highs::results::{Layout, ResultLayouts, load_results};
use calliope_highs::{HighsBackend, OptionValue, SolveConfig};
use calliope_lp::lp_format::parse_lp;
use calliope_lp::{ComponentKind, LinExpr, LpModel};
use calliope_preprocess::xr::Dataset;
use std::path::Path;

const FIXTURE: &str = "test_model__simple_supply__two_hours__investment_costs__shadow_prices";

fn coords(ds: &Dataset, dims: &[String]) -> Vec<Coord> {
    dims.iter().map(|d| ds.coords[d].clone()).collect()
}

#[test]
fn results_match_golden() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(FIXTURE);
    let text = std::fs::read_to_string(dir.join("pyomo.lp")).unwrap();
    let model = LpModel::from_raw(&parse_lp(&text).unwrap());
    let golden = calliope_io::read_group(&dir.join("results.nc"), "results").unwrap();

    let var_layout = |kind, name: &str| {
        let dims = &golden.vars[if kind == ComponentKind::Variable {
            name.to_string()
        } else {
            format!("shadow_price_{name}")
        }
        .as_str()]
        .dims;
        Layout::from_labels(&model, kind, name, coords(&golden, dims)).unwrap()
    };
    let layouts = ResultLayouts {
        variables: ["flow_cap", "flow_out", "source_use"]
            .iter()
            .map(|n| (n.to_string(), var_layout(ComponentKind::Variable, n)))
            .collect(),
        shadow_prices: vec![(
            "system_balance".into(),
            var_layout(ComponentKind::Constraint, "system_balance"),
        )],
        objectives: vec![(
            "min_cost_optimisation".into(),
            Layout {
                dims: vec![],
                cells: vec![(vec![], model.objective.expr.clone())],
            },
        )],
        global_expressions: vec![(
            "zero".into(),
            Layout {
                dims: vec![],
                cells: vec![(vec![], LinExpr::default())],
            },
        )],
    };
    let mut be = HighsBackend::new(model.clone());
    let cfg = SolveConfig {
        solver_options: vec![
            ("output_flag".into(), OptionValue::Bool(false)),
            ("threads".into(), OptionValue::Int(1)),
            ("random_seed".into(), OptionValue::Int(0)),
        ],
        ..SolveConfig::default()
    };
    let outcome = be.solve(&cfg).unwrap().clone();
    let res = load_results(&outcome, &layouts, 1e-10, None).unwrap();
    assert_eq!(res.termination_condition, "optimal");

    for (name, arr) in &res.arrays {
        let Some(want) = golden.vars.get(name.as_str()) else {
            continue;
        };
        let shape: Vec<usize> = want.dims.iter().map(|d| golden.coords[d].len()).collect();
        let got = arr.to_dense();
        assert_eq!(got.len(), shape.iter().product::<usize>(), "{name}");
        for (i, g) in got.iter().enumerate() {
            let w = want.data.get(i).as_f64().unwrap_or(f64::NAN);
            let ok = (g.is_nan() && w.is_nan()) || (g - w).abs() <= 1e-6 * w.abs().max(1.0);
            assert!(ok, "{name}[{i}]: {g} != {w}");
        }
    }
    // the zero global expression is all zero (kept), nothing is all-NaN
    assert_eq!(res.arrays["zero"].to_dense(), vec![0.0]);
    assert_eq!(res.arrays.len(), 6);
}
