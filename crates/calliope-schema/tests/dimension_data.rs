//! Port of `tests/schemas/dimension_data_schema_test.py`, plus tech/node update semantics.

mod common;

use calliope_schema::{
    CalliopeNode, CalliopeTech, CalliopeTechs, DataOrList, DataValue, IndexValue, IndexedData,
    Numeric, TechDef, TechParam, Validated,
};
use calliope_yaml::AttrTree;
use common::yaml;

fn indexed(data: &str, dims: &str, index: &str) -> Result<Validated<IndexedData>, String> {
    let mut text = format!("data: {data}\n");
    if dims != "~skip" {
        text.push_str(&format!("dims: {dims}\n"));
    }
    if index != "~skip" {
        text.push_str(&format!("index: {index}\n"));
    }
    Validated::from_tree(yaml(&text)).map_err(|e| e.to_string())
}

fn s(x: &str) -> IndexValue {
    IndexValue::Str(x.to_owned())
}

#[test]
fn regular_definition() {
    indexed("100", "costs", "monetary").unwrap();
    indexed(
        "[2, 10]",
        "[cost, carriers]",
        "[[monetary, electricity], [monetary, heat]]",
    )
    .unwrap();
}

#[test]
fn broadcasted_definition() {
    let r = indexed("1", "my_dim", "[i1, i2, i3, i4]").unwrap();
    assert_eq!(r.data, DataOrList::One(DataValue::Number(Numeric::Int(1))));
    assert_eq!(
        r.dims.0.iter().map(|d| d.as_str()).collect::<Vec<_>>(),
        ["my_dim"]
    );
    let idx: Vec<Vec<IndexValue>> = r.index.iter().map(|u| u.0.clone()).collect();
    assert_eq!(
        idx,
        vec![vec![s("i1")], vec![s("i2")], vec![s("i3")], vec![s("i4")]]
    );
}

#[test]
fn listify_dims() {
    for (dims, expected) in [
        ("techs", vec!["techs"]),
        ("nodes", vec!["nodes"]),
        ("[techs]", vec!["techs"]),
        ("[techs, nodes]", vec!["techs", "nodes"]),
    ] {
        let r = indexed("1", dims, "~skip").unwrap();
        assert_eq!(
            r.dims.0.iter().map(|d| d.as_str()).collect::<Vec<_>>(),
            expected
        );
        // Default index is `[[]]`.
        assert_eq!(r.index.len(), 1);
        assert!(r.index[0].is_empty());
    }
}

#[test]
fn listify_index() {
    for (index, expected) in [
        ("i1", vec![vec!["i1"]]),
        ("[i1]", vec![vec!["i1"]]),
        ("[i1, i2]", vec![vec!["i1"], vec!["i2"]]),
        ("[[i1], [i2]]", vec![vec!["i1"], vec!["i2"]]),
        (
            "[[i1, i2], [i2, i3]]",
            vec![vec!["i1", "i2"], vec!["i2", "i3"]],
        ),
    ] {
        let r = indexed("1", "my_dim", index).unwrap();
        let got: Vec<Vec<IndexValue>> = r.index.iter().map(|u| u.0.clone()).collect();
        let exp: Vec<Vec<IndexValue>> = expected
            .iter()
            .map(|v| v.iter().map(|x| s(x)).collect())
            .collect();
        assert_eq!(got, exp, "{index}");
    }
}

#[test]
fn invalid_definition() {
    for (data, dims, index) in [
        ("1", "1", "monetary"),
        ("value", "techs", "null"),
        ("1", "[techs, techs, nodes]", "[i1, i2, i3]"),
        ("[]", "[techs]", "[i1]"),
    ] {
        let err = indexed(data, dims, index).unwrap_err();
        assert!(
            err.contains("validation error for Indexed data definition"),
            "{err}"
        );
    }
}

fn tech(text: &str) -> Result<Validated<CalliopeTech>, String> {
    Validated::from_tree(yaml(text)).map_err(|e| e.to_string())
}

#[test]
fn tech_invalid_dims() {
    for dims in ["techs", "nodes"] {
        let err = tech(&format!(
            "
        name: 'Combined cycle gas turbine'
        color: '#FDC97D'
        base_tech: supply
        carrier_out: power
        foobar:
          data: 0.10
          index: monetary
          dims: {dims}
        "
        ))
        .unwrap_err();
        assert!(err.contains("`dims` must not contain"), "{err}");
    }
}

#[test]
fn tech_invalid_base_tech_name() {
    for base_tech in ["None", "1", "foobar"] {
        let err = tech(&format!(
            "
        name: Supply tech
        carrier_out: gas
        base_tech: {base_tech}
        flow_cap_max: 10
        source_use_max: .inf
        "
        ))
        .unwrap_err();
        assert!(err.contains("error for Technology dimension data"), "{err}");
        assert!(err.contains("base_tech"), "{err}");
    }
}

#[test]
fn techs_none_top_level() {
    let techs: CalliopeTechs =
        calliope_yaml::from_node(&yaml("v: null").get("v").unwrap().clone()).unwrap();
    assert_eq!(techs, CalliopeTechs::default());
}

#[test]
fn techs_none_sub_level() {
    let a: Validated<CalliopeTechs> =
        Validated::from_tree(yaml("{foo: null, bar: {param1: 1}, baz: {}}")).unwrap();
    let b: Validated<CalliopeTechs> =
        Validated::from_tree(yaml("{foo: {}, bar: {param1: 1}, baz: {}}")).unwrap();
    assert_eq!(a.get(), b.get());
}

#[test]
fn transmission_union_matches_pydantic() {
    let t: Validated<CalliopeTechs> = Validated::from_tree(yaml(
        "
a: {link_from: x, link_to: y}
b: {base_tech: transmission, link_from: x, link_to: y, foo: 1}
c: {base_tech: supply, link_from: x, link_to: y}
d: {base_tech: transmission}
",
    ))
    .unwrap();
    let kinds: Vec<bool> =
        t.0.values()
            .map(|d| matches!(d, TechDef::Transmission(_)))
            .collect();
    assert_eq!(kinds, [true, true, false, false]);
    // Dumps include `one_way` only for transmission techs, as in pydantic.
    let dump = t.dump();
    assert!(dump.get_key("a.one_way").is_ok());
    assert!(dump.get_key("c.one_way").is_err());
    assert!(dump.get_key("c.link_from").is_ok());
}

fn node(text: &str) -> Result<Validated<CalliopeNode>, String> {
    Validated::from_tree(yaml(text)).map_err(|e| e.to_string())
}

#[test]
fn node_dependent_definitions() {
    for (lat, lon) in [("null", "30"), ("30", "null"), ("1", "2.5")] {
        let err = node(&format!(
            "{{techs: {{}}, latitude: {lat}, longitude: {lon}}}"
        ))
        .unwrap_err();
        assert!(
            err.contains("Invalid latitude/longitude definition."),
            "{err}"
        );
    }
    node("{latitude: 1.0, longitude: 2.5}").unwrap();
    node("{latitude: 1, longitude: 2}").unwrap();
}

#[test]
fn node_coordinate_bounds() {
    let err = node("{latitude: 91, longitude: 2}").unwrap_err();
    assert!(
        err.contains("greater than or equal") || err.contains("less than or equal"),
        "{err}"
    );
    let err = node("{latitude: 1.0, longitude: -181.0}").unwrap_err();
    assert!(err.contains("greater than or equal to -180"), "{err}");
}

/// Tech inheritance (`model_data.py:458`): a node's tech definition is the global one
/// updated with node-level values; indexed parameters are updated field by field.
#[test]
fn tech_update_merges_indexed_params() {
    let base = tech(
        "
base_tech: supply
flow_cap_max: 10
cost_flow_cap: {data: 2, index: monetary, dims: costs}
",
    )
    .unwrap();
    let local: AttrTree = yaml("{flow_cap_max: 20, cost_flow_cap: {data: 3}}");
    let merged = base.update(&local, true).unwrap();
    match &merged.extra["cost_flow_cap"] {
        TechParam::Indexed(p) => {
            assert_eq!(
                p.inner.data,
                DataOrList::One(DataValue::Number(Numeric::Int(3)))
            );
            assert_eq!(p.inner.dims.0[0].as_str(), "costs");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(
        merged.extra["flow_cap_max"],
        TechParam::Value(DataValue::Number(Numeric::Int(20)))
    );
}

#[test]
fn tech_param_keys_must_be_attr_str() {
    let err = tech("{base_tech: supply, 1bad: 3}").unwrap_err();
    assert!(err.contains("1bad"), "{err}");
}
