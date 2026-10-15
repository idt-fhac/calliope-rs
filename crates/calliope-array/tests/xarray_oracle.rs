//! Array operations against an xarray oracle on generated cases.
//!
//! Cases are generated deterministically with proptest strategies. `fixtures/xarray_oracle.json`
//! holds the cases with xarray's results; regenerate it with
//! `cargo test -p calliope-array --test xarray_oracle -- --ignored` (needs the Calliope pixi
//! env, or `CALLIOPE_PYTHON` pointing at a Python with xarray).

use std::path::PathBuf;

use calliope_array::{Coord, Join, LArray, Label, Resample, Selection};
use proptest::prelude::*;
use proptest::strategy::ValueTree;
use proptest::test_runner::TestRunner;
use serde_json::{Value, json};

const HOUR: i64 = 3_600_000_000_000;
/// 2005-01-01 05:00 UTC.
const T0: i64 = 1_104_555_600_000_000_000;
const N_CASES: usize = 400;

// ------------------------------------------------------------------ generation

fn coord_strategy(dim: &'static str) -> BoxedStrategy<Value> {
    match dim {
        "a" => proptest::sample::subsequence(vec!["x", "y", "z", "w"], 1..=4)
            .prop_shuffle()
            .prop_map(|ls| json!({"dtype": "str", "labels": ls}))
            .boxed(),
        "b" => proptest::sample::subsequence(vec![0i64, 1, 2, 3, 4], 1..=5)
            .prop_shuffle()
            .prop_map(|ls| json!({"dtype": "int", "labels": ls}))
            .boxed(),
        _ => proptest::sample::subsequence((0..10i64).collect::<Vec<_>>(), 1..=8)
            .prop_map(|hs| json!({"dtype": "datetime", "labels": hs.iter().map(|h| T0 + h * HOUR).collect::<Vec<_>>()}))
            .boxed(),
    }
}

fn dims_strategy(min: usize) -> BoxedStrategy<Vec<&'static str>> {
    proptest::sample::subsequence(vec!["a", "b", "t"], min..=3)
        .prop_shuffle()
        .boxed()
}

fn values_strategy(n: usize, dtype: &'static str) -> BoxedStrategy<Vec<Value>> {
    match dtype {
        "bool" => proptest::collection::vec(any::<bool>().prop_map(Value::from), n).boxed(),
        _ => proptest::collection::vec(
            prop_oneof![1 => Just(Value::Null), 3 => (-4i32..5).prop_map(|v| json!(f64::from(v)))],
            n,
        )
        .boxed(),
    }
}

fn array_with(
    dims: Vec<&'static str>,
    coords: Vec<Value>,
    dtype: &'static str,
) -> BoxedStrategy<Value> {
    let n: usize = coords
        .iter()
        .map(|c| c["labels"].as_array().map_or(0, Vec::len))
        .product();
    values_strategy(n, dtype)
        .prop_map(move |values| {
            let mut cmap = serde_json::Map::new();
            for (d, c) in dims.iter().zip(&coords) {
                cmap.insert((*d).to_owned(), c.clone());
            }
            json!({"dims": dims.clone(), "coords": cmap, "values": values, "dtype": dtype})
        })
        .boxed()
}

fn array_strategy(min_dims: usize, dtype: &'static str) -> BoxedStrategy<Value> {
    dims_strategy(min_dims)
        .prop_flat_map(move |dims| {
            let coords: Vec<BoxedStrategy<Value>> =
                dims.iter().map(|d| coord_strategy(d)).collect();
            (Just(dims), coords)
        })
        .prop_flat_map(move |(dims, coords)| array_with(dims, coords, dtype))
        .boxed()
}

/// An array over a subset of `base`'s dims with exactly `base`'s coordinates.
fn same_coords(base: &Value, dtype: &'static str, min: usize) -> BoxedStrategy<Value> {
    let dims: Vec<&'static str> = base["dims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| match d.as_str().unwrap() {
            "a" => "a",
            "b" => "b",
            _ => "t",
        })
        .collect();
    let base = base.clone();
    proptest::sample::subsequence(dims.clone(), min.min(dims.len())..=dims.len())
        .prop_flat_map(move |ds| {
            let coords: Vec<Value> = ds.iter().map(|d| base["coords"][*d].clone()).collect();
            array_with(ds, coords, dtype)
        })
        .boxed()
}

fn dim_of(a: &Value) -> BoxedStrategy<String> {
    let dims: Vec<String> = a["dims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d.as_str().unwrap().to_owned())
        .collect();
    proptest::sample::select(dims).boxed()
}

fn case_strategy() -> BoxedStrategy<Value> {
    let ops = vec![
        "add",
        "mul_outer",
        "fillna_scalar",
        "fillna_array",
        "where",
        "where_other",
        "sum",
        "mean",
        "any",
        "roll",
        "transpose",
        "sel_list",
        "sel_slice",
        "isin",
        "group_sum",
        "resample",
        "concat",
    ];
    proptest::sample::select(ops)
        .prop_flat_map(|op| -> BoxedStrategy<Value> {
            match op {
                "add" | "mul_outer" | "fillna_array" => (array_strategy(0, "float"), array_strategy(0, "float"))
                    .prop_map(move |(a, b)| json!({"op": op, "params": {}, "inputs": [a, b]}))
                    .boxed(),
                "fillna_scalar" => array_strategy(0, "float")
                    .prop_map(|a| json!({"op": "fillna_scalar", "params": {"value": 7.0}, "inputs": [a]}))
                    .boxed(),
                "where" => (array_strategy(0, "float"), array_strategy(0, "bool"))
                    .prop_map(|(a, c)| json!({"op": "where", "params": {}, "inputs": [a, c]}))
                    .boxed(),
                "where_other" => array_strategy(1, "float")
                    .prop_flat_map(|a| (Just(a.clone()), same_coords(&a, "bool", 0)))
                    .prop_map(|(a, c)| json!({"op": "where_other", "params": {"value": -9.0}, "inputs": [a, c]}))
                    .boxed(),
                "sum" | "mean" | "any" => {
                    let dtype = if op == "any" { "bool" } else { "float" };
                    array_strategy(1, dtype)
                        .prop_flat_map(move |a| {
                            let dims: Vec<String> =
                                a["dims"].as_array().unwrap().iter().map(|d| d.as_str().unwrap().to_owned()).collect();
                            (Just(a), proptest::sample::subsequence(dims.clone(), 1..=dims.len()), 0usize..=1)
                        })
                        .prop_map(move |(a, over, mc)| json!({"op": op, "params": {"dims": over, "min_count": mc}, "inputs": [a]}))
                        .boxed()
                }
                "roll" => array_strategy(1, "float")
                    .prop_flat_map(|a| (Just(a.clone()), dim_of(&a), -9i64..10))
                    .prop_map(|(a, d, s)| json!({"op": "roll", "params": {"dim": d, "shift": s}, "inputs": [a]}))
                    .boxed(),
                "transpose" => array_strategy(1, "float")
                    .prop_flat_map(|a| {
                        let dims: Vec<String> =
                            a["dims"].as_array().unwrap().iter().map(|d| d.as_str().unwrap().to_owned()).collect();
                        (Just(a), Just(dims).prop_shuffle())
                    })
                    .prop_map(|(a, order)| json!({"op": "transpose", "params": {"order": order}, "inputs": [a]}))
                    .boxed(),
                "sel_list" | "sel_slice" | "isin" => array_strategy(1, "float")
                    .prop_flat_map(|a| (Just(a.clone()), dim_of(&a)))
                    .prop_flat_map(move |(a, d)| {
                        let dtype = a["coords"][&d]["dtype"].as_str().unwrap().to_owned();
                        let pool: Vec<Value> = match dtype.as_str() {
                            "str" => ["x", "y", "z", "w", "q"].iter().map(|s| json!(s)).collect(),
                            "int" => (0..6).map(|i| json!(i)).collect(),
                            _ => (0..11).map(|h| json!(T0 + h * HOUR)).collect(),
                        };
                        let labels = proptest::sample::subsequence(pool.clone(), 0..=3).prop_shuffle();
                        let one = proptest::option::of(proptest::sample::select(pool));
                        (Just(a), Just(d), labels, one.clone(), one)
                    })
                    .prop_map(move |(a, d, labels, start, end)| {
                        json!({"op": op, "params": {"dim": d, "labels": labels, "start": start, "end": end}, "inputs": [a]})
                    })
                    .boxed(),
                "group_sum" => array_strategy(1, "float")
                    .prop_filter("needs a non-time dim", |a| {
                        a["dims"].as_array().unwrap().iter().any(|d| d != "t")
                    })
                    .prop_flat_map(|a| {
                        let base = a.clone();
                        let dims: Vec<&'static str> = a["dims"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .filter_map(|d| match d.as_str().unwrap() {
                                "a" => Some("a"),
                                "b" => Some("b"),
                                _ => None,
                            })
                            .collect();
                        let g = proptest::sample::subsequence(dims.clone(), 1..=dims.len()).prop_flat_map(move |ds| {
                            let coords: Vec<Value> = ds.iter().map(|d| base["coords"][*d].clone()).collect();
                            let n: usize = coords.iter().map(|c| c["labels"].as_array().unwrap().len()).product();
                            let vals = proptest::collection::vec(
                                prop_oneof![1 => Just(Value::Null), 3 => proptest::sample::select(vec!["g1", "g2", "g3"]).prop_map(Value::from)],
                                n,
                            );
                            (Just(ds), Just(coords), vals)
                        });
                        (Just(a), g)
                    })
                    .prop_map(|(a, (ds, coords, vals))| {
                        let mut cmap = serde_json::Map::new();
                        for (d, c) in ds.iter().zip(&coords) {
                            cmap.insert((*d).to_owned(), c.clone());
                        }
                        let g = json!({"dims": ds, "coords": cmap, "values": vals, "dtype": "str"});
                        json!({"op": "group_sum", "params": {}, "inputs": [a, g]})
                    })
                    .boxed(),
                "resample" => array_strategy(1, "float")
                    .prop_filter("needs time", |a| a["dims"].as_array().unwrap().iter().any(|d| d == "t"))
                    .prop_flat_map(|a| (Just(a), 1i64..=5, proptest::sample::select(vec!["first", "mean", "sum"])))
                    .prop_map(|(a, h, m)| json!({"op": "resample", "params": {"dim": "t", "hours": h, "method": m}, "inputs": [a]}))
                    .boxed(),
                _ => dims_strategy(0)
                    .prop_flat_map(|dims| {
                        let arr = || {
                            let coords: Vec<BoxedStrategy<Value>> = dims.iter().map(|d| coord_strategy(d)).collect();
                            let dims = dims.clone();
                            coords.prop_flat_map(move |c| array_with(dims.clone(), c, "float"))
                        };
                        proptest::collection::vec(arr(), 1..=3)
                    })
                    .prop_map(|xs| {
                        let labels: Vec<String> = (0..xs.len()).map(|i| format!("n{i}")).collect();
                        json!({"op": "concat", "params": {"labels": labels}, "inputs": xs})
                    })
                    .boxed(),
            }
        })
        .boxed()
}

fn generate_cases() -> Vec<Value> {
    let mut runner = TestRunner::deterministic();
    let strategy = case_strategy();
    (0..N_CASES)
        .map(|i| {
            let mut c = strategy.new_tree(&mut runner).expect("case").current();
            c["id"] = json!(i);
            c
        })
        .collect()
}

// ------------------------------------------------------------------ evaluation

fn label(v: &Value, dtype: &str) -> Label {
    match dtype {
        "str" => Label::str(v.as_str().unwrap()),
        "int" => Label::Int(v.as_i64().unwrap()),
        _ => Label::Datetime(v.as_i64().unwrap()),
    }
}

fn coords_of(spec: &Value) -> Vec<Coord> {
    spec["dims"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| {
            let d = d.as_str().unwrap();
            let c = &spec["coords"][d];
            let dtype = c["dtype"].as_str().unwrap();
            let labels = c["labels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| label(l, dtype))
                .collect();
            Coord::new(d, labels).unwrap()
        })
        .collect()
}

fn float_array(spec: &Value) -> LArray<f64> {
    let vals = spec["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_f64().unwrap_or(f64::NAN))
        .collect();
    LArray::from_dense(coords_of(spec), vals).unwrap()
}

fn bool_array(spec: &Value) -> LArray<bool> {
    let vals = spec["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_bool().unwrap())
        .collect();
    LArray::from_dense(coords_of(spec), vals).unwrap()
}

fn str_array(spec: &Value) -> LArray<Option<Label>> {
    let vals = spec["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().map(Label::str))
        .collect();
    LArray::from_dense(coords_of(spec), vals).unwrap()
}

enum Out {
    F(LArray<f64>),
    B(LArray<bool>),
    Err(String),
}

fn labels_param(case: &Value, key: &str, dtype: &str) -> Vec<Label> {
    case["params"][key]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| label(l, dtype))
        .collect()
}

fn run(case: &Value) -> Out {
    let p = &case["params"];
    let inputs = case["inputs"].as_array().unwrap();
    let dims_param = |k: &str| -> Vec<String> {
        p[k].as_array()
            .unwrap()
            .iter()
            .map(|d| d.as_str().unwrap().to_owned())
            .collect()
    };
    let res = |r: Result<LArray<f64>, calliope_array::ArrayError>| {
        r.map_or_else(|e| Out::Err(e.to_string()), Out::F)
    };
    match case["op"].as_str().unwrap() {
        "add" => Out::F(float_array(&inputs[0]).zip_with(
            &float_array(&inputs[1]),
            Join::Inner,
            |a, b| a + b,
        )),
        "mul_outer" => Out::F(float_array(&inputs[0]).zip_with(
            &float_array(&inputs[1]),
            Join::Outer,
            |a, b| a * b,
        )),
        "fillna_scalar" => Out::F(float_array(&inputs[0]).fillna(p["value"].as_f64().unwrap())),
        "fillna_array" => Out::F(float_array(&inputs[0]).fillna_with(&float_array(&inputs[1]))),
        "where" => Out::F(float_array(&inputs[0]).where_(&bool_array(&inputs[1]))),
        "where_other" => Out::F(
            float_array(&inputs[0]).where_or(&bool_array(&inputs[1]), p["value"].as_f64().unwrap()),
        ),
        "sum" => {
            let d = dims_param("dims");
            let d: Vec<&str> = d.iter().map(String::as_str).collect();
            res(float_array(&inputs[0]).sum(&d, p["min_count"].as_u64().unwrap() as usize))
        }
        "mean" => {
            let d = dims_param("dims");
            let d: Vec<&str> = d.iter().map(String::as_str).collect();
            res(float_array(&inputs[0]).mean(&d))
        }
        "any" => {
            let d = dims_param("dims");
            let d: Vec<&str> = d.iter().map(String::as_str).collect();
            bool_array(&inputs[0])
                .any(&d)
                .map_or_else(|e| Out::Err(e.to_string()), Out::B)
        }
        "roll" => {
            res(float_array(&inputs[0])
                .roll(p["dim"].as_str().unwrap(), p["shift"].as_i64().unwrap()))
        }
        "transpose" => {
            let d = dims_param("order");
            let d: Vec<&str> = d.iter().map(String::as_str).collect();
            res(float_array(&inputs[0]).transpose(&d))
        }
        op @ ("sel_list" | "sel_slice" | "isin") => {
            let a = float_array(&inputs[0]);
            let dim = p["dim"].as_str().unwrap();
            let dtype = inputs[0]["coords"][dim]["dtype"].as_str().unwrap();
            match op {
                "sel_list" => {
                    res(a.sel(dim, &Selection::Labels(labels_param(case, "labels", dtype))))
                }
                "sel_slice" => {
                    let b = |k: &str| (!p[k].is_null()).then(|| label(&p[k], dtype));
                    res(a.sel(dim, &Selection::Slice(b("start"), b("end"))))
                }
                _ => Out::B(
                    LArray::from_coord(a.coord(dim).unwrap())
                        .isin(&labels_param(case, "labels", dtype)),
                ),
            }
        }
        "group_sum" => res(float_array(&inputs[0]).group_sum(&str_array(&inputs[1]), "groups")),
        "resample" => {
            let method = match p["method"].as_str().unwrap() {
                "first" => Resample::First,
                "mean" => Resample::Mean,
                _ => Resample::Sum,
            };
            res(float_array(&inputs[0]).resample("t", p["hours"].as_i64().unwrap() * HOUR, method))
        }
        "concat" => {
            let xs: Vec<LArray<f64>> = inputs.iter().map(float_array).collect();
            let labels: Vec<Label> = p["labels"]
                .as_array()
                .unwrap()
                .iter()
                .map(|l| Label::str(l.as_str().unwrap()))
                .collect();
            res(LArray::concat(&xs, Coord::new("new", labels).unwrap()))
        }
        other => panic!("unknown op {other}"),
    }
}

fn describe(dims: &[Coord]) -> Vec<(String, Vec<String>)> {
    dims.iter()
        .map(|d| {
            (
                d.name().to_owned(),
                d.labels().iter().map(ToString::to_string).collect(),
            )
        })
        .collect()
}

fn check(case: &Value) -> Result<(), String> {
    let expected = &case["expected"];
    let out = run(case);
    if let Some(err) = expected.get("error") {
        return match out {
            Out::Err(_) => Ok(()),
            _ => Err(format!("xarray raised `{err}`, Rust did not")),
        };
    }
    let exp_dims = coords_of(expected);
    let (dims, values): (Vec<Coord>, Vec<Option<f64>>) = match out {
        Out::Err(e) => return Err(format!("Rust raised `{e}`")),
        Out::F(a) => (
            a.dims().to_vec(),
            a.to_dense()
                .into_iter()
                .map(|v| (!v.is_nan()).then_some(v))
                .collect(),
        ),
        Out::B(a) => (
            a.dims().to_vec(),
            a.to_dense()
                .into_iter()
                .map(|b| Some(f64::from(u8::from(b))))
                .collect(),
        ),
    };
    if describe(&dims) != describe(&exp_dims) {
        return Err(format!(
            "dims {:?} != {:?}",
            describe(&dims),
            describe(&exp_dims)
        ));
    }
    let exp: Vec<Option<f64>> = expected["values"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| match v {
            Value::Bool(b) => Some(f64::from(u8::from(*b))),
            other => other.as_f64(),
        })
        .collect();
    let close = |a: &Option<f64>, b: &Option<f64>| match (a, b) {
        (None, None) => true,
        (Some(x), Some(y)) => (x - y).abs() <= 1e-12 * (1.0 + y.abs()),
        _ => false,
    };
    if values.len() != exp.len() || !values.iter().zip(&exp).all(|(a, b)| close(a, b)) {
        return Err(format!("values {values:?} != {exp:?}"));
    }
    Ok(())
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/xarray_oracle.json")
}

#[test]
fn matches_xarray() {
    let text = std::fs::read_to_string(fixture_path()).expect("fixture; regenerate with --ignored");
    let cases: Vec<Value> = serde_json::from_str(&text).unwrap();
    // The fixture was produced from exactly these generated cases.
    let generated = generate_cases();
    assert_eq!(cases.len(), generated.len());
    let mut failures = Vec::new();
    for (case, gen_case) in cases.iter().zip(&generated) {
        let mut stripped = case.clone();
        stripped.as_object_mut().unwrap().remove("expected");
        assert_eq!(
            &stripped, gen_case,
            "fixture out of date; regenerate with --ignored"
        );
        if let Err(e) = check(case) {
            failures.push(format!(
                "case {} ({}): {e}\n  {}",
                case["id"], case["op"], case["inputs"]
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} cases differ:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
#[ignore = "needs Python with xarray; regenerates the fixture"]
fn regenerate_fixture() {
    let python = std::env::var("CALLIOPE_PYTHON").map_or_else(
        |_| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../calliope/.pixi/envs/dev/bin/python")
        },
        PathBuf::from,
    );
    let dir = tempfile_dir();
    let cases_path = dir.join("cases.json");
    std::fs::write(
        &cases_path,
        serde_json::to_string(&generate_cases()).unwrap(),
    )
    .unwrap();
    let script = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/xarray_oracle.py");
    let status = std::process::Command::new(python)
        .arg("-W")
        .arg("ignore")
        .arg(script)
        .arg(&cases_path)
        .arg(fixture_path())
        .status()
        .unwrap();
    assert!(status.success());
}

fn tempfile_dir() -> PathBuf {
    let d = std::env::temp_dir().join(format!("calliope-array-oracle-{}", std::process::id()));
    std::fs::create_dir_all(&d).unwrap();
    d
}
