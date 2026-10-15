//! Port of `tests/schemas/general_test.py`.
//!
//! The dynamic `pydantic.create_model` fixtures become small test structs with the same
//! fields, defaults and update shapes. `__repr__`/`__rich_repr__`/`__setitem__` tests are not
//! ported (Python-only); `model_no_ref_schema` tests are replaced by JSON-schema checks.

use calliope_schema::{
    AttrStr, DictShape, FieldError, ModelShape, NonEmptyList, NonEmptyUniqueList, Numeric, Schema,
    Shape, UniqueList, Validated,
};
use calliope_yaml::{AttrTree, Node, from_node, read_yaml_str, to_tree};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}

fn node(text: &str) -> Node {
    yaml(&format!("v: {text}")).get("v").unwrap().clone()
}

// ---------------------------------------------------------------- list and string types

#[test]
fn unique_list_valid() {
    for text in [
        "[1, 2, 3]",
        "[1.0, 1.1, 1.2]",
        "['1', '2', '3']",
        "['1', 1, 'foo']",
        "[null, .nan]",
    ] {
        let parsed: Result<UniqueList<Node>, _> = from_node(&node(text));
        assert!(parsed.is_ok(), "{text}");
    }
    for text in ["[1, 2, 3]", "['1', '2', '3']", "['foo', 'bar']"] {
        assert!(from_node::<UniqueList<Node>>(&node(text)).is_ok(), "{text}");
    }
}

#[test]
fn unique_list_invalid() {
    for text in [
        "[1, 1, 2]",
        "[1, 1.0]",
        "['1', 'foo', 'foo']",
        "[null, null]",
        "[1, true]",
        "[0, false]",
        "[1, 1.0, 2, '2']",
        "['foo', 1, 'foo']",
        "[[1, 1, 2], [1, 1, 2], ['1', 'foo', 3]]",
    ] {
        let err = from_node::<UniqueList<Node>>(&node(text)).unwrap_err();
        assert!(err.message.contains("List must be unique"), "{text}: {err}");
    }
}

#[test]
fn non_empty_list() {
    assert!(from_node::<NonEmptyList<Node>>(&node("[]")).is_err());
    assert!(from_node::<NonEmptyList<Node>>(&node("[1, 1]")).is_ok());
}

#[test]
fn non_empty_unique_list() {
    for text in [
        "[]",
        "[1, 1]",
        "['1', 'foo', 'foo']",
        "[[1, 2], [1, 2], [1]]",
    ] {
        assert!(
            from_node::<NonEmptyUniqueList<Node>>(&node(text)).is_err(),
            "{text}"
        );
    }
}

#[test]
fn attr_str() {
    for text in [
        "1",
        "'1thing'",
        "'with spaces'",
        "'111992'",
        "null",
        "true",
        "'_x'",
    ] {
        assert!(from_node::<AttrStr>(&node(text)).is_err(), "{text}");
    }
    for text in ["wind_offshore", "ccgt", "élan", "a1"] {
        assert!(from_node::<AttrStr>(&node(text)).is_ok(), "{text}");
    }
}

#[test]
fn numeric_val() {
    for text in ["[1, 2]", "foobar", "null"] {
        assert!(from_node::<Numeric>(&node(text)).is_err(), "{text}");
    }
    assert_eq!(from_node::<Numeric>(&node("1")).unwrap(), Numeric::Int(1));
    assert!(
        matches!(from_node::<Numeric>(&node(".inf")).unwrap(), Numeric::Float(f) if f.is_infinite())
    );
    assert!(
        matches!(from_node::<Numeric>(&node(".nan")).unwrap(), Numeric::Float(f) if f.is_nan())
    );
    // pydantic lax mode
    assert_eq!(
        from_node::<Numeric>(&node("true")).unwrap(),
        Numeric::Int(1)
    );
    assert_eq!(
        from_node::<Numeric>(&node("'1.5'")).unwrap(),
        Numeric::Float(1.5)
    );
}

// ---------------------------------------------------------------- test models

fn d_value1() -> String {
    "value1".into()
}
fn d_bar() -> String {
    "bar".into()
}
fn d_one() -> i64 {
    1
}
fn d_ten() -> i64 {
    10
}
fn d_default_value() -> String {
    "default_value".into()
}

/// `TestBaseModel`: `foo: str = "value1"`, `bar: int` (required).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct BaseM {
    #[serde(default = "d_value1")]
    foo: String,
    bar: i64,
}
static BASE_M: ModelShape = ModelShape {
    name: "TestBaseModel",
    fields: &[],
    extra: Shape::Leaf,
};
impl Schema for BaseM {
    const TITLE: &'static str = "TestBaseModel";
    fn shape() -> Shape {
        Shape::Model(&BASE_M)
    }
}

/// `dict[str, str]` dict model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct DictStr(IndexMap<String, String>);
static DICT_STR: DictShape = DictShape {
    name: "TestModel",
    entry: Shape::Leaf,
};
impl Schema for DictStr {
    const TITLE: &'static str = "TestModel";
    fn shape() -> Shape {
        Shape::Dict(&DICT_STR)
    }
}

/// `dict[str, TestBaseModel]` dict model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct DictBase(IndexMap<String, BaseM>);
static DICT_BASE: DictShape = DictShape {
    name: "TestModel",
    entry: Shape::Model(&BASE_M),
};
impl Schema for DictBase {
    const TITLE: &'static str = "TestModel";
    fn shape() -> Shape {
        Shape::Dict(&DICT_BASE)
    }
}

/// `dict[str, TestListModel]` dict model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(transparent)]
struct DictList(IndexMap<String, Vec<BaseM>>);
static DICT_LIST: DictShape = DictShape {
    name: "TestModel",
    entry: Shape::List,
};
impl Schema for DictList {
    const TITLE: &'static str = "TestModel";
    fn shape() -> Shape {
        Shape::Dict(&DICT_LIST)
    }
}

/// `TestModel1`: `foo: str = "bar"`, `foobar: int = 1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Flat {
    #[serde(default = "d_bar")]
    foo: String,
    #[serde(default = "d_one")]
    foobar: i64,
}
impl Default for Flat {
    fn default() -> Self {
        Self {
            foo: d_bar(),
            foobar: 1,
        }
    }
}
static FLAT: ModelShape = ModelShape {
    name: "TestModel1",
    fields: &[],
    extra: Shape::Leaf,
};
impl Schema for Flat {
    const TITLE: &'static str = "TITLE";
    fn shape() -> Shape {
        Shape::Model(&FLAT)
    }
}

/// `TestModel2`: `nested: TestModel1`, `top_level_foobar: int = 10`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Nested {
    #[serde(default)]
    nested: Flat,
    #[serde(default = "d_ten")]
    top_level_foobar: i64,
}
impl Default for Nested {
    fn default() -> Self {
        Self {
            nested: Flat::default(),
            top_level_foobar: 10,
        }
    }
}
static NESTED: ModelShape = ModelShape {
    name: "TestModel2",
    fields: &[("nested", Shape::Model(&FLAT))],
    extra: Shape::Leaf,
};
impl Schema for Nested {
    const TITLE: &'static str = "TITLE 2";
    fn shape() -> Shape {
        Shape::Model(&NESTED)
    }
}

/// `TestModel3`: `extra_nested: TestModel2`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct DoubleNested {
    extra_nested: Nested,
}
static DOUBLE_NESTED: ModelShape = ModelShape {
    name: "TestModel3",
    fields: &[("extra_nested", Shape::Model(&NESTED))],
    extra: Shape::Leaf,
};
impl Schema for DoubleNested {
    const TITLE: &'static str = "TITLE 3";
    fn shape() -> Shape {
        Shape::Model(&DOUBLE_NESTED)
    }
}

/// `config_model_nested_with_dict_and_list_models`: the inner `TestModel1`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Inner {
    #[serde(default)]
    nested_list_field: Vec<Flat>,
    #[serde(default)]
    nested_config: Flat,
    #[serde(default = "d_default_value")]
    other_field: String,
}
static INNER: ModelShape = ModelShape {
    name: "TestModel1",
    fields: &[
        ("nested_list_field", Shape::List),
        ("nested_config", Shape::Model(&FLAT)),
    ],
    extra: Shape::Leaf,
};
static INNER_DICT: DictShape = DictShape {
    name: "TestDictModel",
    entry: Shape::Model(&INNER),
};

/// `TestNestedModel`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default)]
struct WithDictAndList {
    dict_field: IndexMap<String, Inner>,
    list_field: Vec<Flat>,
}
static WITH_DICT_AND_LIST: ModelShape = ModelShape {
    name: "TestNestedModel",
    fields: &[
        ("dict_field", Shape::Dict(&INNER_DICT)),
        ("list_field", Shape::List),
    ],
    extra: Shape::Leaf,
};
impl Schema for WithDictAndList {
    const TITLE: &'static str = "Nested Model";
    fn shape() -> Shape {
        Shape::Model(&WITH_DICT_AND_LIST)
    }
}

fn model<T: Schema>(text: &str) -> Validated<T> {
    Validated::from_tree(yaml(text)).unwrap()
}

fn dump<T: Schema>(m: &Validated<T>) -> AttrTree {
    m.dump()
}

// ---------------------------------------------------------------- CalliopeDictModel

#[test]
fn dict_update() {
    let m: Validated<DictStr> = model("{key1: value1, key2: value2}");
    let new = m
        .update(&yaml("{key1: new_value1, key3: value3}"), true)
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml("{key1: new_value1, key2: value2, key3: value3}")
    );
    assert_eq!(dump(&m), yaml("{key1: value1, key2: value2}"));
}

#[test]
fn dict_update_invalid() {
    let m: Validated<DictStr> = model("{key1: value1, key2: value2}");
    let err = m.update(&yaml("{key1: 123}"), true).unwrap_err();
    assert!(err.to_string().starts_with("1 validation error"), "{err}");
}

#[test]
fn dict_update_str_no_overwrite() {
    let m: Validated<DictStr> = model("{key1: value1, key2: value2}");
    let new = m
        .update(&yaml("{key1: new_value1, key3: value3}"), false)
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml("{key1: value1, key2: value2, key3: value3}")
    );
}

#[test]
fn dict_check_base_model() {
    let m: Validated<DictBase> = model("{key1: {bar: 2}, key2: {foo: value2, bar: 3}}");
    assert_eq!(
        dump(&m),
        yaml("{key1: {foo: value1, bar: 2}, key2: {foo: value2, bar: 3}}")
    );
}

#[test]
fn dict_update_base_model() {
    let m: Validated<DictBase> = model("{key1: {bar: 2}, key2: {foo: value2, bar: 3}}");
    let new = m
        .update(
            &yaml("{key1: {foo: new_value1}, key2: {foo: new_value2, bar: 6}}"),
            true,
        )
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml("{key1: {foo: new_value1, bar: 2}, key2: {foo: new_value2, bar: 6}}")
    );
    assert_eq!(
        dump(&m),
        yaml("{key1: {foo: value1, bar: 2}, key2: {foo: value2, bar: 3}}")
    );
}

#[test]
fn dict_update_base_model_no_overwrite() {
    let m: Validated<DictBase> = model("{key1: {bar: 2}, key2: {foo: value2, bar: 3}}");
    let new = m
        .update(
            &yaml("{key1: {foo: new_value1, bar: 3735928559}, key3: {foo: new_value3, bar: 4}}"),
            false,
        )
        .unwrap();
    // key1.foo was unset, so it is filled; key1.bar was set and is kept.
    assert_eq!(
        dump(&new),
        yaml(
            "{key1: {foo: new_value1, bar: 2}, key2: {foo: value2, bar: 3}, key3: {foo: new_value3, bar: 4}}"
        )
    );
}

#[test]
fn dict_update_list_model() {
    let m: Validated<DictList> = model("{key1: [{bar: 2}], key2: [{foo: value2, bar: 3}]}");
    let new = m
        .update(
            &yaml("{key1: [{bar: 4}], key2: [{foo: new_value2, bar: 6}]}"),
            true,
        )
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml("{key1: [{foo: value1, bar: 4}], key2: [{foo: new_value2, bar: 6}]}")
    );
    assert_eq!(
        dump(&m),
        yaml("{key1: [{foo: value1, bar: 2}], key2: [{foo: value2, bar: 3}]}")
    );
}

#[test]
fn dict_update_list_model_no_overwrite() {
    let m: Validated<DictList> = model("{key1: [{bar: 2}], key2: [{foo: value2, bar: 3}]}");
    let new = m
        .update(&yaml("{key1: [{bar: 4}], key3: [{bar: 4}]}"), false)
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml(
            "{key1: [{foo: value1, bar: 2}], key2: [{foo: value2, bar: 3}], key3: [{foo: value1, bar: 4}]}"
        )
    );
}

#[test]
fn dict_log_message_on_adding_entry() {
    let m: Validated<DictStr> = model("{key1: value1, key2: value2}");
    let mut log = Vec::new();
    let new = m
        .update_logged(&yaml("{key3: value3}"), true, &mut log)
        .unwrap();
    assert!(
        log.iter()
            .any(|l| l.contains("Adding TestModel entry: `key3`")),
        "{log:?}"
    );
    assert_eq!(new.0["key3"], "value3");
}

// ---------------------------------------------------------------- CalliopeListModel

#[test]
fn list_update_replaces_whole_list() {
    let m: Validated<WithDictAndList> = model("{list_field: [{foo: value1}, {foo: value2}]}");
    let new = m
        .update(
            &yaml("{list_field: [{foo: new_value1, foobar: 1}, {foo: new_value2, foobar: 6}]}"),
            true,
        )
        .unwrap();
    assert_eq!(
        dump(&new).get("list_field").unwrap(),
        yaml("v: [{foo: new_value1, foobar: 1}, {foo: new_value2, foobar: 6}]")
            .get("v")
            .unwrap()
    );
    assert_eq!(m.list_field.len(), 2);
    assert_eq!(m.list_field[0].foo, "value1");
}

// ---------------------------------------------------------------- CalliopeBaseModel

#[test]
fn update_flat() {
    for (upd, exp) in [
        ("{foo: baz}", "{foo: baz, foobar: 1}"),
        ("{foobar: 2}", "{foo: bar, foobar: 2}"),
        ("{foo: baz, foobar: 2}", "{foo: baz, foobar: 2}"),
    ] {
        let m: Validated<Flat> = Validated::default_model().unwrap();
        let before = dump(&m);
        let new = m.update(&yaml(upd), true).unwrap();
        assert_eq!(dump(&new), yaml(exp), "{upd}");
        assert_eq!(dump(&m), before);
    }
}

#[test]
fn update_flat_no_overwrite() {
    for (upd, exp) in [
        ("{foo: baz}", "{foo: bar, foobar: 1}"),
        ("{foobar: 2}", "{foo: bar, foobar: 2}"),
        ("{foo: baz, foobar: 2}", "{foo: bar, foobar: 2}"),
    ] {
        let m: Validated<Flat> = model("{foo: bar}");
        let mut log = Vec::new();
        let new = m.update_logged(&yaml(upd), false, &mut log).unwrap();
        assert_eq!(dump(&new), yaml(exp), "{upd}");
        if upd.contains("foo:") {
            assert!(
                log.iter()
                    .any(|l| l.contains("Skipping existing TestModel1 entry: `foo` as")),
                "{log:?}"
            );
        }
    }
}

#[test]
fn update_nested() {
    for (upd, exp) in [
        (
            "{top_level_foobar: 20}",
            "{top_level_foobar: 20, nested: {foo: bar, foobar: 1}}",
        ),
        (
            "{nested: {foobar: 2}}",
            "{top_level_foobar: 10, nested: {foo: bar, foobar: 2}}",
        ),
        (
            "{top_level_foobar: 20, nested: {foobar: 2}}",
            "{top_level_foobar: 20, nested: {foo: bar, foobar: 2}}",
        ),
        (
            "{top_level_foobar: 20, nested.foobar: 2}",
            "{top_level_foobar: 20, nested: {foo: bar, foobar: 2}}",
        ),
    ] {
        let m: Validated<Nested> = Validated::default_model().unwrap();
        let before = dump(&m);
        let new = m.update(&yaml(upd), true).unwrap();
        assert_eq!(dump(&new), yaml(exp), "{upd}");
        assert_eq!(dump(&m), before);
    }
}

#[test]
fn update_nested_no_overwrite() {
    for (upd, exp) in [
        (
            "{top_level_foobar: 20}",
            "{top_level_foobar: 20, nested: {foo: bar, foobar: 1}}",
        ),
        (
            "{nested: {foo: baz, foobar: 2}}",
            "{top_level_foobar: 10, nested: {foo: bar, foobar: 2}}",
        ),
    ] {
        let m: Validated<Nested> = model("{nested: {foo: bar}}");
        let new = m.update(&yaml(upd), false).unwrap();
        assert_eq!(dump(&new), yaml(exp), "{upd}");
    }
}

#[test]
fn update_extra_nested() {
    for upd in [
        "{extra_nested.nested.foobar: 2}",
        "{extra_nested: {nested: {foobar: 2}}}",
    ] {
        let m: Validated<DoubleNested> = Validated::default_model().unwrap();
        let before = dump(&m);
        let new = m.update(&yaml(upd), true).unwrap();
        assert_eq!(new.extra_nested.nested.foobar, 2);
        assert_eq!(dump(&m), before);
    }
}

#[test]
fn update_extra_nested_validation_error() {
    for upd in [
        "{extra_nested.nested.foobar: foo}",
        "{extra_nested.top_level_foobar: foo}",
    ] {
        let m: Validated<DoubleNested> = Validated::default_model().unwrap();
        let err = m.update(&yaml(upd), true).unwrap_err();
        assert!(err.to_string().starts_with("1 validation error"), "{err}");
    }
}

#[test]
fn update_nested_with_dict_and_list_models() {
    let m: Validated<WithDictAndList> = model(
        "
dict_field:
  key1:
    nested_list_field: [{foo: value1, foobar: 2}, {foo: value2, foobar: 4}]
    nested_config: {foo: new_value1, foobar: 1}
  key2: {other_field: value3}
list_field: [{foo: value3, foobar: 6}]
",
    );
    let orig = dump(&m);
    let new = m
        .update(
            &yaml(
                "
dict_field:
  key1:
    nested_list_field: [{foo: new_value1}]
    nested_config: {foo: new_value1}
  key3: {}
list_field: [{}, {foo: new_value3, foobar: 8}]
",
            ),
            true,
        )
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml(
            "
dict_field:
  key1:
    nested_list_field: [{foo: new_value1, foobar: 1}]
    nested_config: {foo: new_value1, foobar: 1}
    other_field: default_value
  key2:
    nested_list_field: []
    nested_config: {foo: bar, foobar: 1}
    other_field: value3
  key3:
    nested_list_field: []
    nested_config: {foo: bar, foobar: 1}
    other_field: default_value
list_field: [{foo: bar, foobar: 1}, {foo: new_value3, foobar: 8}]
"
        )
    );
    assert_eq!(dump(&m), orig);
}

#[test]
fn update_nested_with_dict_and_list_models_no_overwrite() {
    let m: Validated<WithDictAndList> = model(
        "
dict_field:
  key1:
    nested_list_field: [{foo: value1, foobar: 2}, {foo: value2, foobar: 4}]
    nested_config: {foobar: 1}
  key2: {other_field: value3}
list_field: [{foo: value3, foobar: 6}]
",
    );
    let mut log = Vec::new();
    let new = m
        .update_logged(
            &yaml(
                "
dict_field:
  key1:
    nested_list_field: [{foo: new_value1}]
    nested_config: {foo: new_value1, foobar: 2}
  key2: {other_field: new_value3}
  key3: {}
list_field: [{foo: bar, foobar: 1}, {foo: new_value3, foobar: 8}]
",
            ),
            false,
            &mut log,
        )
        .unwrap();
    assert_eq!(
        dump(&new),
        yaml(
            "
dict_field:
  key1:
    nested_list_field: [{foo: value1, foobar: 2}, {foo: value2, foobar: 4}]
    nested_config: {foo: new_value1, foobar: 1}
    other_field: default_value
  key2:
    nested_list_field: []
    nested_config: {foo: bar, foobar: 1}
    other_field: value3
  key3:
    nested_list_field: []
    nested_config: {foo: bar, foobar: 1}
    other_field: default_value
list_field: [{foo: value3, foobar: 6}]
"
        )
    );
    assert!(
        log.iter()
            .any(|l| l.contains("Skipping existing TestNestedModel entry: `list_field`")),
        "{log:?}"
    );
}

#[test]
fn logging() {
    for (upd, expected) in [
        (
            "{extra_nested.nested.foobar: 2}",
            vec!["Updating TestModel1 `foobar`: 1 -> 2"],
        ),
        (
            "{extra_nested.top_level_foobar: 2}",
            vec!["Updating TestModel2 `top_level_foobar`: 10 -> 2"],
        ),
        (
            "{extra_nested.nested.foobar: 2, extra_nested.top_level_foobar: 3}",
            vec![
                "Updating TestModel1 `foobar`: 1 -> 2",
                "Updating TestModel2 `top_level_foobar`: 10 -> 3",
            ],
        ),
    ] {
        let m: Validated<DoubleNested> = Validated::default_model().unwrap();
        let mut log = Vec::new();
        m.update_logged(&yaml(upd), true, &mut log).unwrap();
        for e in expected {
            assert!(log.iter().any(|l| l.contains(e)), "{upd}: {log:?}");
        }
    }
}

#[test]
fn logging_no_update_on_same_val() {
    let m: Validated<WithDictAndList> = model(
        "
dict_field:
  key1: {nested_config: {foo: value1, foobar: 1}}
  key2: {other_field: value3}
list_field: [{foo: value3, foobar: 6}]
",
    );
    let mut log = Vec::new();
    m.update_logged(
        &yaml("{dict_field.key1.nested_config.foo: value1, dict_field.key2.other_field: value3}"),
        true,
        &mut log,
    )
    .unwrap();
    assert!(log.is_empty(), "{log:?}");
}

#[test]
fn logging_no_overwrite() {
    let m: Validated<WithDictAndList> = model(
        "
dict_field:
  key1: {nested_config: {foo: value1}}
  key2: {other_field: value3}
list_field: [{foo: value3, foobar: 6}]
",
    );
    let mut log = Vec::new();
    m.update_logged(
        &yaml(
            "{dict_field.key1.nested_config.foo: value2, dict_field.key1.nested_config.foobar: 2, dict_field.key2.other_field: value3}",
        ),
        false,
        &mut log,
    )
    .unwrap();
    assert!(
        !log.iter().any(|l| l.contains("`foo`: value1 -> value2")),
        "{log:?}"
    );
    assert!(
        log.iter().any(|l| l.contains("`foobar`: 1 -> 2")),
        "{log:?}"
    );
}

/// The "explicitly set to the default" rule (`general.py:203-207`): a value equal to the
/// default becomes set, so it wins in a later `overwrite=False` update.
#[test]
fn value_equal_to_default_is_recorded_as_set() {
    let m: Validated<Flat> = Validated::default_model().unwrap();
    assert!(!m.is_set("foo"));
    let m = m.update(&yaml("{foo: bar}"), true).unwrap();
    assert!(m.is_set("foo"));
    let inherited = m.update(&yaml("{foo: other, foobar: 5}"), false).unwrap();
    assert_eq!(inherited.foo, "bar");
    assert_eq!(inherited.foobar, 5);
    assert_eq!(
        inherited.fields_set().collect::<Vec<_>>(),
        ["foo", "foobar"]
    );
}

#[test]
fn update_with_model_uses_explicitly_set_values() {
    let base: Validated<Nested> = model("{top_level_foobar: 3}");
    let other: Validated<Nested> = model("{nested: {foo: x}}");
    let merged = base.update_with(&other, true).unwrap();
    assert_eq!(merged.top_level_foobar, 3);
    assert_eq!(merged.nested.foo, "x");
    assert_eq!(merged.nested.foobar, 1);
}

#[test]
fn update_submodel_with_scalar_is_an_error() {
    let m: Validated<Nested> = Validated::default_model().unwrap();
    assert!(m.update(&yaml("{nested: 3}"), true).is_err());
    // `None` is an empty update for a sub-model.
    let same = m.update(&yaml("{nested: null}"), true).unwrap();
    assert_eq!(dump(&same), dump(&m));
}

#[test]
fn unknown_keys_are_rejected() {
    let err = Validated::<Flat>::from_tree(yaml("{foo: x, bogus: 1}")).unwrap_err();
    assert!(err.to_string().contains("unknown field `bogus`"), "{err}");
}

#[test]
fn errors_point_at_the_yaml() {
    let err =
        Validated::<DoubleNested>::from_tree(yaml("extra_nested:\n  nested:\n    foobar: x\n"))
            .unwrap_err();
    assert_eq!(err.errors[0].path, "extra_nested.nested.foobar");
    let span = err.errors[0].span.as_ref().unwrap();
    assert_eq!((span.line, span.col), (3, 13));
    let FieldError { message, .. } = &err.errors[0];
    assert!(message.contains("invalid type"), "{message}");
}

#[test]
fn dump_roundtrips_through_tree() {
    let m: Validated<WithDictAndList> = model("{dict_field: {k: {other_field: x}}}");
    let again: Validated<WithDictAndList> =
        Validated::from_tree(to_tree(m.get()).unwrap()).unwrap();
    assert_eq!(again.get(), m.get());
}
