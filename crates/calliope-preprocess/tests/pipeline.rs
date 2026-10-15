//! Ports of the model-level tests in `tests/preprocess/` (`model_definition_test.py`,
//! `model_data_test.py`, `time_test.py`): the test model (vendored in
//! `tests/fixtures/test_model`) is built with overrides and the inputs are checked.

use std::path::PathBuf;

use calliope_array::Label;
use calliope_preprocess::data_tables::DataTable;
use calliope_preprocess::model_data::ModelDataBuilder;
use calliope_preprocess::model_math::{build_math, get_math_priority};
use calliope_preprocess::xr::{Dataset, Obj};
use calliope_preprocess::{
    ModelError, ModelInputs, ReadOptions, prepare_model_definition_with, read_yaml,
};
use calliope_yaml::{AttrTree, Node, Value, read_yaml_file, read_yaml_str};

const DUMMY_INT: i64 = 0xDEAD_BEEF;

fn fixtures() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn model_path() -> PathBuf {
    fixtures().join("test_model/model.yaml")
}

fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}

fn build(overrides: &str, scenario: &str, kwargs: &str) -> Result<ModelInputs, ModelError> {
    let opts = ReadOptions {
        scenario: (!scenario.is_empty()).then(|| scenario.to_owned()),
        override_dict: (!overrides.is_empty()).then(|| yaml(overrides)),
        math_dict: None,
        init_kwargs: if kwargs.is_empty() {
            AttrTree::new()
        } else {
            yaml(kwargs)
        },
    };
    read_yaml(&model_path(), &opts)
}

/// The value of `var` at the given labels (all its dims must be given).
fn val(ds: &Dataset, var: &str, at: &[(&str, &str)]) -> Obj {
    let mut da = ds.get(var).unwrap_or_else(|| panic!("no `{var}`"));
    for (dim, label) in at {
        let Some(coord) = da.coord(dim).cloned() else {
            continue;
        };
        let l = coord
            .labels()
            .iter()
            .find(|l| l.to_string() == *label)
            .unwrap_or_else(|| panic!("no {label} in {dim}"))
            .clone();
        da = da.sel(dim, &[l]).unwrap();
    }
    // Remaining dims (e.g. `carriers`): the first non-missing value, as `.sel(...) == x`
    // compares where data exists.
    (0..da.data.len())
        .map(|i| da.data.get(i))
        .find(|o| !o.is_null())
        .unwrap_or(Obj::Nan)
}

fn labels(ds: &Dataset, dim: &str) -> Vec<String> {
    ds.coord(dim)
        .map(|c| c.labels().iter().map(ToString::to_string).collect())
        .unwrap_or_default()
}

// --- model_definition_test.py: TestScenarioOverrides --------------------------------

#[test]
fn valid_scenarios() {
    let m = build(
        &format!(
            "scenarios:\n  scenario_1: ['one', 'two']\noverrides:\n  one:\n    techs.test_supply_gas.flow_cap_max: {DUMMY_INT}\n  two:\n    techs.test_supply_elec.flow_cap_max: {}\nnodes:\n  a:\n    techs:\n      test_supply_gas:\n      test_supply_elec:\n      test_demand_elec:\n",
            DUMMY_INT as f64 / 2.0
        ),
        "scenario_1",
        "",
    )
    .unwrap();
    let ds = &m.inputs;
    assert!(
        val(
            ds,
            "flow_cap_max",
            &[("techs", "test_supply_gas"), ("nodes", "a")]
        )
        .py_eq(&Obj::Int(DUMMY_INT))
    );
    assert!(
        val(
            ds,
            "flow_cap_max",
            &[("techs", "test_supply_elec"), ("nodes", "a")]
        )
        .py_eq(&Obj::Float(DUMMY_INT as f64 / 2.0))
    );
}

#[test]
fn valid_scenario_of_scenarios() {
    let m = build(
        &format!(
            "scenarios:\n  scenario_1: ['one', 'two']\n  scenario_2: ['scenario_1', 'new_location']\noverrides:\n  one:\n    techs.test_supply_gas.flow_cap_max: {DUMMY_INT}\n  two:\n    techs.test_supply_elec.flow_cap_max: {}\n  new_location:\n    nodes.b.techs:\n      test_supply_elec:\nnodes:\n  a:\n    techs:\n      test_supply_gas:\n      test_supply_elec:\n      test_demand_elec:\n",
            DUMMY_INT as f64 / 2.0
        ),
        "scenario_2",
        "",
    )
    .unwrap();
    let ds = &m.inputs;
    assert!(
        val(
            ds,
            "flow_cap_max",
            &[("techs", "test_supply_gas"), ("nodes", "a")]
        )
        .py_eq(&Obj::Int(DUMMY_INT))
    );
}

#[test]
fn invalid_scenarios() {
    let e = build(
        "scenarios:\n  scenario_1:\n    techs.foo.bar: 1\n",
        "scenario_1",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("(scenarios, scenario_1) | Unrecognised override name: techs."),
        "{e}"
    );
    let e = build("scenarios:\n  scenario_1: 'foo'\n", "scenario_1", "").unwrap_err();
    assert!(
        e.to_string()
            .contains("(scenarios, scenario_1) | Unrecognised override name: foo."),
        "{e}"
    );
}

#[test]
fn override_dict_beats_scenarios() {
    // override_dict is applied before and after the scenario (it has the last word).
    let m = build(
        "techs.test_supply_elec.flow_cap_max: 3\noverrides:\n  o:\n    techs.test_supply_elec.flow_cap_max: 7\n",
        "simple_supply,two_hours,o",
        "",
    )
    .unwrap();
    assert!(
        val(
            &m.inputs,
            "flow_cap_max",
            &[("techs", "test_supply_elec"), ("nodes", "a")]
        )
        .py_eq(&Obj::Int(3))
    );
    assert_eq!(
        m.runtime.applied_overrides.as_deref(),
        Some("simple_supply;two_hours;o")
    );
}

#[test]
fn conflicting_overrides_in_a_scenario() {
    let e = build(
        "overrides:\n  x:\n    techs.test_supply_elec.flow_cap_max: 1\n  y:\n    techs.test_supply_elec.flow_cap_max: 2\n",
        "simple_supply,two_hours,x,y",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("Already specified but defined again in override `y`."),
        "{e}"
    );
}

// --- model_math_test.py ----------------------------------------------------------------

#[test]
fn math_priority_and_errors() {
    let init: calliope_schema::Init =
        calliope_yaml::from_tree(&yaml("mode: operate\nextra_math: [milp]")).unwrap();
    assert_eq!(get_math_priority(&init), ["base", "operate", "milp"]);
    let math = calliope_preprocess::model_math::initialise_math(&Default::default(), None).unwrap();
    assert_eq!(
        math.keys().collect::<Vec<_>>(),
        ["base", "milp", "operate", "spores", "storage_inter_cluster"]
    );
    let e = build_math(&["foobar_fail".to_owned()], &math, None).unwrap_err();
    assert_eq!(
        e.to_string(),
        "Requested math 'foobar_fail' was not initialised."
    );
    // base -> mode -> extra: later math overrides earlier.
    let mut extra = AttrTree::new();
    extra.insert(
        "user",
        Node::from(yaml(&format!(
            "variables:\n  storage:\n    bounds:\n      min: {DUMMY_INT}"
        ))),
    );
    let mut math2 = math.clone();
    math2.insert(
        "user".to_owned(),
        extra.get("user").unwrap().as_map().unwrap().clone(),
    );
    let built = build_math(&["base".to_owned(), "user".to_owned()], &math2, None).unwrap();
    assert!(
        format!("{:?}", built.variables["storage"].bounds.min).contains(&DUMMY_INT.to_string())
    );
}

#[test]
fn user_math_overwrites_predefined() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("custom-math.yaml"),
        "variables:\n  storage:\n    bounds:\n      min: 1\n",
    )
    .unwrap();
    let mut paths = calliope_schema::MathPaths::new();
    paths.insert(
        calliope_schema::AttrStr::new("base").unwrap(),
        PathBuf::from("custom-math.yaml"),
    );
    let math = calliope_preprocess::model_math::initialise_math(&paths, Some(dir.path())).unwrap();
    assert_eq!(
        math["base"],
        yaml("variables:\n  storage:\n    bounds:\n      min: 1\n")
    );
    assert_eq!(math.get_index_of("base"), Some(0));
}

// --- model_data_test.py: TestTopLevelParams ---------------------------------------------

#[test]
fn parameter_already_exists_warns() {
    let m = build(
        "data_definitions.flow_out_eff: 1",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    assert!(m.warnings.iter().any(|w| w.contains(
        "Model input data with this name has already been defined in a data table or at a node/tech level."
    )));
}

#[test]
fn top_level_param_single_val() {
    for (v, expected) in [
        ("1", Obj::Int(1)),
        ("1.0", Obj::Float(1.0)),
        (".inf", Obj::Float(f64::INFINITY)),
        ("foo", Obj::str("foo")),
    ] {
        let m = build(
            &format!("data_definitions.my_val: {v}"),
            "simple_supply,two_hours",
            "",
        )
        .unwrap();
        assert!(m.inputs.get("my_val").unwrap().dims().is_empty());
        assert!(m.inputs.get("my_val").unwrap().item().py_eq(&expected));
    }
    for v in ["null", ".nan"] {
        let m = build(
            &format!("data_definitions.my_val: {v}"),
            "simple_supply,two_hours",
            "",
        )
        .unwrap();
        assert!(!m.inputs.contains("my_val"));
    }
}

#[test]
fn top_level_param_indexed() {
    let m = build(
        "data_definitions.my_val: {data: [1, foo], index: [test_supply_elec, test_demand_elec], dims: techs}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    assert_eq!(
        val(&m.inputs, "my_val", &[("techs", "test_supply_elec")]),
        Obj::Int(1)
    );
    assert_eq!(
        val(&m.inputs, "my_val", &[("techs", "test_demand_elec")]),
        Obj::str("foo")
    );

    let m = build(
        "data_definitions.my_val: {data: 10, index: [[electricity, test_supply_elec], [electricity, test_demand_elec]], dims: [carriers, techs]}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    assert!(
        val(
            &m.inputs,
            "my_val",
            &[("carriers", "electricity"), ("techs", "test_demand_elec")]
        )
        .py_eq(&Obj::Int(10))
    );

    let m = build(
        "data_definitions.my_val: {data: [10, 20], index: [[a, test_supply_elec], [b, test_demand_elec]], dims: [nodes, techs]}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    assert!(
        m.warnings
            .iter()
            .any(|w| w.contains("This input data will only take effect"))
    );

    let m = build(
        "data_definitions.my_val: {data: 10, index: [[foo, foobar]], dims: [bar, baz]}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    assert_eq!(labels(&m.inputs, "bar"), ["foo"]);
    assert!(val(&m.inputs, "my_val", &[("bar", "foo"), ("baz", "foobar")]).py_eq(&Obj::Int(10)));

    let m = build(
        "data_definitions.my_val: {data: 10, index: ['2005-01-01'], dims: [timesteps]}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    let ts = m.inputs.coord("timesteps").unwrap();
    assert!(ts.is_datetime());
}

#[test]
fn top_level_param_length_mismatch() {
    let e = build(
        "config.init.broadcast_input_data: false\ndata_definitions.my_val: {data: [1, 2], index: [a]}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string().contains("Length mismatch between data"),
        "{e}"
    );
}

// --- model_data_test.py: TestActiveFalse ------------------------------------------------

const STORAGE: &str = "simple_storage,two_hours,investment_costs";

#[test]
fn tech_active_false() {
    let o = "techs.test_storage.active: false";
    let keep = build(o, STORAGE, "retain_inactive: true").unwrap();
    assert!(labels(&keep.inputs, "techs").contains(&"test_storage".to_owned()));
    let drop = build(o, STORAGE, "retain_inactive: false").unwrap();
    assert!(!labels(&drop.inputs, "techs").contains(&"test_storage".to_owned()));
}

#[test]
fn node_active_false() {
    let o = "nodes.b.active: false";
    let keep = build(o, STORAGE, "retain_inactive: true").unwrap();
    assert!(labels(&keep.inputs, "nodes").contains(&"b".to_owned()));
    let drop = build(o, STORAGE, "retain_inactive: false").unwrap();
    assert!(!labels(&drop.inputs, "nodes").contains(&"b".to_owned()));
}

#[test]
fn node_tech_active_false() {
    let o = format!(
        "nodes.b.techs.test_storage:\n  active: false\n  foo: {DUMMY_INT}\n  cost_flow_cap.data: {DUMMY_INT}\n"
    );
    let keep = build(&o, STORAGE, "retain_inactive: true").unwrap();
    assert!(
        val(
            &keep.inputs,
            "foo",
            &[("techs", "test_storage"), ("nodes", "b")]
        )
        .py_eq(&Obj::Int(DUMMY_INT))
    );
    let costs = keep.inputs.get("cost_flow_cap").unwrap();
    let at: Vec<(&str, &str)> = costs
        .dims()
        .iter()
        .map(|d| match *d {
            "techs" => ("techs", "test_storage"),
            "nodes" => ("nodes", "b"),
            "costs" => ("costs", "monetary"),
            other => panic!("{other}"),
        })
        .collect();
    assert!(val(&keep.inputs, "cost_flow_cap", &at).py_eq(&Obj::Int(DUMMY_INT)));
    let drop = build(&o, STORAGE, "retain_inactive: false").unwrap();
    assert!(!drop.inputs.contains("foo"));
    assert!(val(&drop.inputs, "cost_flow_cap", &at).is_null());
}

#[test]
fn link_active_false() {
    let o = "templates.test_transmission.active: false";
    let is_trans = |m: &ModelInputs| {
        let b = m.inputs.get("base_tech").unwrap();
        (0..b.data.len()).any(|i| b.data.get(i).as_str() == Some("transmission"))
    };
    assert!(is_trans(
        &build(o, STORAGE, "retain_inactive: true").unwrap()
    ));
    assert!(!is_trans(
        &build(o, STORAGE, "retain_inactive: false").unwrap()
    ));
}

#[test]
fn transmission_tech_at_node_fails() {
    let e = build(
        "nodes.a.techs.test_link_a_b_elec: {}",
        "simple_supply,two_hours",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("Transmission techs cannot be directly defined at nodes"),
        "{e}"
    );
    let e = build(
        "nodes.a.techs.test_supply_elec.base_tech: supply",
        "simple_supply,two_hours",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("Defining a technology `base_tech` at a node is not supported"),
        "{e}"
    );
}

// --- model_data_test.py: TestBaseAttributesFromDataTables -------------------------------

/// Writes the variant's records as CSV files (indexed over the table's `rows`) and builds
/// the (uncleaned) dataset of the test model.
fn build_dataset(variant: &AttrTree) -> Result<Dataset, ModelError> {
    let dir = tempfile::tempdir().unwrap();
    let mut overrides = variant
        .get("overrides")
        .and_then(Node::as_map)
        .cloned()
        .unwrap_or_default();
    let dfs = variant
        .get("data_table_dfs")
        .and_then(Node::as_map)
        .cloned()
        .unwrap_or_default();
    if let Some(tables) = overrides.get_mut("data_tables").and_then(Node::as_map_mut) {
        for (_, t) in tables.iter_mut() {
            let t = t.as_map_mut().unwrap();
            let name = t.get("table").unwrap().as_str().unwrap().to_owned();
            let rows: Vec<String> = match &t.get("rows").unwrap().value {
                Value::Seq(s) => s.iter().map(|n| n.as_str().unwrap().to_owned()).collect(),
                Value::Str(s) => vec![s.clone()],
                _ => unreachable!(),
            };
            let records = dfs.get(&name).unwrap().as_seq().unwrap();
            let mut cols: Vec<String> = Vec::new();
            for r in records {
                for k in r.as_map().unwrap().keys() {
                    if !rows.contains(k) && !cols.contains(k) {
                        cols.push(k.clone());
                    }
                }
            }
            let mut csv = rows
                .iter()
                .chain(&cols)
                .cloned()
                .collect::<Vec<_>>()
                .join(",")
                + "\n";
            for r in records {
                let m = r.as_map().unwrap();
                let cell = |k: &String| {
                    m.get(k).map_or_else(String::new, |n| match &n.value {
                        Value::Bool(true) => "True".to_owned(),
                        Value::Bool(false) => "False".to_owned(),
                        Value::Str(s) => s.clone(),
                        other => other.to_string(),
                    })
                };
                csv += &rows
                    .iter()
                    .chain(&cols)
                    .map(cell)
                    .collect::<Vec<_>>()
                    .join(",");
                csv += "\n";
            }
            let path = dir.path().join(format!("{name}.csv"));
            std::fs::write(&path, csv).unwrap();
            t.insert("table", Node::new(Value::Str(path.display().to_string())));
        }
    }
    let path = model_path();
    let def = read_yaml_file(&path, false).unwrap();
    let prepared = prepare_model_definition_with(
        &def,
        Some("simple_supply,two_hours"),
        Some(&overrides),
        None,
        Some(&path),
        &AttrTree::new(),
    )?;
    let math = build_math(
        &get_math_priority(&prepared.config.init),
        &prepared.math_init,
        None,
    )?;
    let mut tables: Vec<DataTable> = prepared
        .definition
        .data_tables
        .0
        .iter()
        .map(|(n, t)| DataTable::new(n, t, Some(&path)))
        .collect::<Result<_, _>>()?;
    let mut b = ModelDataBuilder::new(
        &prepared.config.init,
        prepared.definition.clone(),
        &math,
        &mut tables,
    )?;
    b.build()?;
    Ok(b.dataset)
}

fn variants(file: &str) -> Vec<(String, AttrTree)> {
    read_yaml_file(
        fixtures().join("base_attrs_from_data_tables").join(file),
        false,
    )
    .unwrap()
    .iter()
    .map(|(k, v)| (k.clone(), v.as_map().unwrap().clone()))
    .collect()
}

fn node_obj(n: &Node) -> Obj {
    match &n.value {
        Value::Bool(b) => Obj::Bool(*b),
        Value::Int(i) => Obj::Int(*i),
        Value::Float(f) => Obj::float(*f),
        Value::Str(s) => Obj::str(s),
        _ => Obj::Nan,
    }
}

#[test]
fn base_attrs_tech_attrs() {
    for (name, variant) in variants("test_tech_attrs.yaml") {
        let ds = build_dataset(&variant).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = variant.get("expected").unwrap().as_map().unwrap();
        for (k, v) in expected {
            let got = val(&ds, k, &[("techs", "test_link_a_b_elec")]);
            assert!(
                got.py_eq(&node_obj(v)),
                "{name}: {k}: expected {}, got {got}",
                v.value
            );
        }
        let active = ds.get("active").unwrap();
        let sel = active
            .sel("techs", &[Label::str("test_link_a_b_elec")])
            .unwrap();
        assert!(
            !(0..sel.data.len()).any(|i| sel.data.is_null(i)),
            "{name}: active has NaN"
        );
    }
}

#[test]
fn base_attrs_tech_attrs_over_nodes_fails() {
    let v = yaml(
        "data_table_dfs:\n  table_1:\n    - {nodes: a, techs: test_link_a_b_elec, link_from: a}\noverrides:\n  data_tables:\n    table_1: {table: table_1, rows: [nodes, techs], columns: inputs}\n",
    );
    let e = build_dataset(&v).unwrap_err();
    assert!(
        e.to_string()
            .contains("Cannot define transmission technology data over the `nodes` dimension"),
        "{e}"
    );
}

#[test]
fn base_attrs_active() {
    for (name, variant) in variants("test_active.yaml") {
        let ds = build_dataset(&variant).unwrap_or_else(|e| panic!("{name}: {e}"));
        let expected = variant.get("expected").unwrap().as_map().unwrap();
        for (node, v) in expected {
            let got = val(
                &ds,
                "active",
                &[("techs", "test_supply_elec"), ("nodes", node)],
            );
            assert!(
                got.py_eq(&node_obj(v)),
                "{name}: {node}: expected {}, got {got}",
                v.value
            );
        }
    }
}

// --- time_test.py ----------------------------------------------------------------------

#[test]
fn change_date_format() {
    let m = build(
        "config.init.datetime_format: '%d/%m/%Y %H:%M'\ndata_tables:\n  demand_elec.table: data_tables/demand_heat_diff_dateformat.csv\n  demand_heat.table: data_tables/demand_heat_diff_dateformat.csv\n",
        "simple_conversion",
        "",
    )
    .unwrap();
    let ts = labels(&m.inputs, "timesteps");
    assert_eq!(ts.len(), 48);
    let first = m.inputs.coord("timesteps").unwrap().labels()[0].clone();
    assert_eq!(
        first,
        Label::Datetime(calliope_preprocess::time::parse_iso8601("2005-01-01").unwrap())
    );
}

#[test]
fn incorrect_date_formats() {
    assert!(
        build(
            "data_tables.demand_elec.table: data_tables/demand_heat_diff_dateformat.csv",
            "simple_conversion",
            ""
        )
        .is_err()
    );
    assert!(
        build(
            "config.init.datetime_format: '%d/%m/%Y %H:%M'",
            "simple_supply",
            ""
        )
        .is_err()
    );
    let e = build(
        "data_tables.demand_elec.table: data_tables/demand_heat_wrong_dateformat.csv",
        "simple_conversion",
        "",
    )
    .unwrap_err();
    assert!(
        e.to_string()
            .contains("Time data 02/01/2005 00:00 is not ISO8601 format"),
        "{e}"
    );
}

#[test]
fn check_time_subset() {
    use calliope_preprocess::time::{check_time_subset, parse_iso8601};
    let start = parse_iso8601("2005-01-01").unwrap();
    let ts: Vec<i64> = (0..=96).map(|h| start + h * 3_600_000_000_000).collect();
    let s = |a: &[&str]| a.iter().map(|x| (*x).to_owned()).collect::<Vec<_>>();
    for ok in [
        ["2005", "2005"],
        ["2005-01", "2005-01"],
        ["2005-01-02", "2005-01-04"],
        ["2005-01-02 00", "2005-01-04 00"],
        ["2005-01-02 00:00", "2005-01-04 00:00"],
        ["2005-01-02 00:00:00", "2005-01-04 00:00:00"],
    ] {
        check_time_subset(&ts, &s(&ok)).unwrap();
    }
    let e = check_time_subset(&ts, &s(&["2005-01-06", "2005-01-07"])).unwrap_err();
    assert!(e.to_string().contains("subset time range"));
    let e = check_time_subset(&ts, &s(&["2005-01-02", "2005-01-04", "2005-01-05"])).unwrap_err();
    assert!(
        e.to_string()
            .contains("subset must be a list of two timestamps")
    );
    for bad in [
        ["01/01/2005", "03/01/2005"],
        ["01/01/2005 00:00", "03/01/2005 00:00"],
    ] {
        let e = check_time_subset(&ts, &s(&bad)).unwrap_err();
        assert!(
            e.to_string()
                .contains("Timeseries subset must be in ISO format")
        );
    }
}

#[test]
fn datetime_index_formats() {
    use calliope_preprocess::time::{datetime_index, parse_iso8601};
    let expected = parse_iso8601("2005-01-01").unwrap();
    for f in ["ISO8601", "%Y-%m-%d"] {
        assert_eq!(
            datetime_index(&[Obj::str("2005-01-01")], f).unwrap(),
            [expected]
        );
    }
}

fn cluster_override(name: &str) -> String {
    format!(
        "data_tables:\n  clustering:\n    table: data_tables/{name}.csv\n    rows: datesteps\n    add_dims: {{inputs: {name}}}\n"
    )
}

#[test]
fn clustering() {
    for (name, extra, kwargs) in [
        ("cluster_days", "", ""),
        (
            "cluster_days_diff_dateformat",
            "data_tables.demand_elec: {table: data_tables/demand_heat_diff_dateformat.csv}\n",
            "datetime_format: '%d/%m/%Y %H:%M'\ndate_format: '%d/%m/%Y'\n",
        ),
    ] {
        let m = build(
            &format!("{}{extra}", cluster_override(name)),
            "simple_supply",
            &format!("subset: {{timesteps: ['2005-01-01', '2005-01-04']}}\ntime_cluster: {name}\n{kwargs}"),
        )
        .unwrap_or_else(|e| panic!("{name}: {e}"));
        let ds = &m.inputs;
        assert_eq!(labels(ds, "clusters"), ["0", "1"]);
        let tc = ds.get("timestep_cluster").unwrap();
        assert_eq!(tc.data.len(), 48);
        for i in 0..48 {
            assert_eq!(tc.data.get(i), Obj::Int(i64::from(i >= 24)));
        }
        assert_eq!(labels(ds, "datesteps").len(), 5);
        for v in [
            "cluster_first_timestep",
            "lookup_cluster_last_timestep",
            "lookup_datestep_cluster",
            "lookup_datestep_last_cluster_timestep",
        ] {
            assert!(ds.contains(v), "{name}: {v}");
        }
    }
}

#[test]
fn resampling_then_clustering() {
    let m = build(
        "data_tables.cluster_days: {table: data_tables/cluster_days.csv, rows: datesteps, add_dims: {inputs: cluster_days_param}}",
        "simple_supply",
        "subset: {timesteps: ['2005-01-01', '2005-01-04']}\nresample: {timesteps: 6h}\ntime_cluster: cluster_days_param",
    )
    .unwrap();
    assert_eq!(labels(&m.inputs, "timesteps").len(), 8);
}

#[test]
fn resampling() {
    let e = build("", "simple_supply", "resample: {timesteps: unknown}").unwrap_err();
    assert!(
        e.to_string()
            .contains("Unknown `timesteps` resampling frequency: unknown"),
        "{e}"
    );

    let m = build(
        "data_tables.demand_elec.table: data_tables/demand_elec_15mins.csv",
        "simple_supply",
        "resample: {timesteps: 6h}",
    )
    .unwrap();
    assert_eq!(labels(&m.inputs, "timesteps").len(), 8);

    let m = build(
        "data_tables.demand_elec.table: data_tables/demand_elec_15T_to_2h.csv",
        "simple_supply,one_day",
        "resample: {timesteps: 2h}",
    )
    .unwrap();
    assert_eq!(labels(&m.inputs, "timesteps").len(), 12);

    let m = build(
        "data_tables:\n  demand_elec:\n    select:\n      nodes: a\n  demand_elec_15m:\n    table: data_tables/demand_elec_15mins.csv\n    rows: timesteps\n    columns: nodes\n    select:\n      nodes: b\n    add_dims:\n      inputs: sink_use_equals\n      techs: test_demand_elec\n",
        "simple_supply",
        "resample: {timesteps: 6h}",
    )
    .unwrap();
    assert_eq!(labels(&m.inputs, "timesteps").len(), 8);
    // 15-minute data is summed on resampling, so it is 4x the hourly data.
    for t in labels(&m.inputs, "timesteps") {
        let a = val(
            &m.inputs,
            "sink_use_equals",
            &[
                ("techs", "test_demand_elec"),
                ("nodes", "a"),
                ("timesteps", &t),
            ],
        );
        let b = val(
            &m.inputs,
            "sink_use_equals",
            &[
                ("techs", "test_demand_elec"),
                ("nodes", "b"),
                ("timesteps", &t),
            ],
        );
        let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
        assert!(
            (a.max(0.0) - b.max(0.0) / 4.0).abs() < 1e-9 || (a.is_nan() && b.is_nan()),
            "{t}: {a} {b}"
        );
    }
}

#[test]
fn single_timestep_warns() {
    let m = build(
        "",
        "simple_supply",
        "subset: {timesteps: ['2005-01-01 00:00', '2005-01-01 00:00']}",
    )
    .unwrap();
    assert!(
        m.warnings
            .iter()
            .any(|w| w.contains("Only one timestep defined"))
    );
    assert_eq!(
        val(
            &m.inputs,
            "timestep_resolution",
            &[("timesteps", &labels(&m.inputs, "timesteps")[0])]
        ),
        Obj::Float(1.0)
    );
}

#[test]
fn no_timeseries_fails() {
    let e = build("data_tables: {_REPLACE_: {}}", "simple_supply", "").unwrap_err();
    assert!(
        e.to_string()
            .contains("Must define at least one timeseries data input"),
        "{e}"
    );
}

#[test]
fn distances_from_lat_lon() {
    let m = build(
        "nodes.a: {latitude: 0, longitude: 0}\nnodes.b: {latitude: 0, longitude: 1}\ntechs.test_link_a_b_elec.distance: null\n",
        "simple_supply,two_hours",
        "",
    )
    .unwrap();
    let d = val(&m.inputs, "distance", &[("techs", "test_link_a_b_elec")])
        .as_f64()
        .unwrap();
    // One degree of longitude on the WGS84 equator.
    assert!((d - 111.319_490_793_273_57).abs() < 1e-9, "{d}");
}
