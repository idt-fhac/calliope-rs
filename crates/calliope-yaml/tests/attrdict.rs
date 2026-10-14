//! Port of `tests/core/attrdict_test.py`.
//!
//! Not ported, because they test Python-only mechanics:
//! - `test_missing_nonzero` (the `_MISSING` sentinel; Rust uses `Option`);
//! - `test_init_from_nondict` (the type system rules it out; see `top_level_must_be_mapping`);
//! - `test_dot_access_*` (attribute access; covered by `get_key`);
//! - `test_get_subkey_from_nested_non_attrdict` (assigning a plain `dict` bypasses `AttrDict`).

use calliope_yaml::{AttrTree, NestedKey, Node, TreeError, Value, read_yaml_str};

fn attr_dict() -> AttrTree {
    read_yaml_str(
        "a: 1\nb: 2\nd: null\nc: {x: foo, y: bar, z: {I: 1, II: 2}}\n",
        false,
    )
    .unwrap()
}

fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}

fn get<'a>(t: &'a AttrTree, key: &str) -> &'a Value {
    &t.get_key(key).unwrap().value
}

#[test]
fn init_from_dict() {
    assert_eq!(*get(&attr_dict(), "a"), Value::Int(1));
}

#[test]
fn init_from_dict_with_nested_keys() {
    let d = yaml("foo.bar.baz: 1");
    assert_eq!(*get(&d, "foo.bar.baz"), Value::Int(1));
    assert!(d.get("foo").unwrap().is_map());
}

#[test]
fn top_level_must_be_mapping() {
    let err = read_yaml_str("foo", false).unwrap_err();
    assert!(
        err.to_string()
            .contains("Could not parse <yaml string> as YAML")
    );
}

#[test]
fn dot_access_list() {
    let d = yaml("a: [{x: 1}, {y: 2}]");
    let items = d.get("a").unwrap().as_seq().unwrap();
    assert_eq!(*get(items[0].as_map().unwrap(), "x"), Value::Int(1));
}

#[test]
fn set_key_first() {
    let mut d = attr_dict();
    d.set_key("a", 2i64.into()).unwrap();
    assert_eq!(*get(&d, "a"), Value::Int(2));
}

#[test]
fn set_key_second() {
    let mut d = attr_dict();
    d.set_key("c.x", "baz".into()).unwrap();
    assert_eq!(*get(&d, "c.x"), Value::Str("baz".into()));
}

#[test]
fn set_key_multiple_inexisting() {
    let mut d = attr_dict();
    d.set_key("c.l.o.h.a", "foo".into()).unwrap();
    assert_eq!(*get(&d, "c.l.o.h.a"), Value::Str("foo".into()));
}

#[test]
fn set_key_nested_on_string() {
    let mut d = attr_dict();
    let err = d.set_key("a.foo", "bar".into()).unwrap_err();
    assert!(matches!(err, TreeError::NestedOnNonMap { .. }));
}

#[test]
fn set_key_nested_on_none() {
    let mut d = attr_dict();
    assert!(d.get("d").unwrap().is_null());
    d.set_key("d.foo", "bar".into()).unwrap();
    assert_eq!(*get(&d, "d.foo"), Value::Str("bar".into()));
}

#[test]
fn pass_regular_dict_to_set_key() {
    let mut d = attr_dict();
    d.set_key("c.z.newkey", Node::from(yaml("foo: 1\ndoo: 2")))
        .unwrap();
    assert!(d.get_key("c.z.newkey").unwrap().is_map());
    assert_eq!(*get(&d, "c.z.newkey.foo"), Value::Int(1));
}

#[test]
fn set_key_merges_mapping_into_existing_mapping() {
    let mut d = attr_dict();
    d.set_key("c", Node::from(yaml("w: 0"))).unwrap();
    assert_eq!(*get(&d, "c.x"), Value::Str("foo".into()));
    assert_eq!(*get(&d, "c.w"), Value::Int(0));
}

#[test]
fn get_key_first() {
    assert_eq!(*get(&attr_dict(), "a"), Value::Int(1));
}

#[test]
fn get_key_second() {
    assert_eq!(*get(&attr_dict(), "c.x"), Value::Str("foo".into()));
}

#[test]
fn get_key_inexistant() {
    assert!(matches!(
        attr_dict().get_key("foo"),
        Err(TreeError::KeyNotFound { .. })
    ));
}

#[test]
fn get_key_second_inexistant() {
    assert!(matches!(
        attr_dict().get_key("foo.bar"),
        Err(TreeError::KeyNotFound { .. })
    ));
}

#[test]
fn get_key_default() {
    assert_eq!(
        attr_dict().get_key_opt("c.x").unwrap().value,
        Value::Str("foo".into())
    );
}

#[test]
fn get_key_inexistant_default() {
    assert!(attr_dict().get_key_opt("foo").is_none());
}

#[test]
fn get_key_second_inexistant_default() {
    assert!(attr_dict().get_key_opt("foo.bar").is_none());
}

#[test]
fn get_key_second_nondict_default() {
    assert!(attr_dict().get_key_opt("c.x.foo").is_none());
}

#[test]
fn as_dict_flat() {
    let d = attr_dict();
    let flat = d.as_flat();
    assert_eq!(flat["c.x"].value, Value::Str("foo".into()));
}

#[test]
fn keys_nested_as_list() {
    assert_eq!(
        attr_dict().keys_nested(),
        ["a", "b", "d", "c.x", "c.y", "c.z.I", "c.z.II"]
    );
}

#[test]
fn keys_nested_as_dict() {
    use NestedKey::{Branch, Leaf};
    let l = |s: &str| Leaf(s.to_owned());
    assert_eq!(
        attr_dict().keys_nested_tree(),
        vec![
            l("a"),
            l("b"),
            l("d"),
            Branch(
                "c".into(),
                vec![l("x"), l("y"), Branch("z".into(), vec![l("I"), l("II")])]
            )
        ]
    );
}

#[test]
fn union() {
    let mut d = attr_dict();
    let mut new = AttrTree::new();
    new.set_key("c.z.III", "foo".into()).unwrap();
    d.union(&new, false, false).unwrap();
    assert_eq!(*get(&d, "c.z.III"), Value::Str("foo".into()));
    assert_eq!(*get(&d, "c.z.I"), Value::Int(1));
}

#[test]
fn union_duplicate_keys() {
    let mut d = attr_dict();
    let mut new = AttrTree::new();
    new.set_key("c.z.II", "foo".into()).unwrap();
    let err = d.union(&new, false, false).unwrap_err();
    assert!(matches!(err, TreeError::DefinedTwice { ref key, .. } if key == "c.z.II"));
    assert!(err.to_string().starts_with("Key defined twice: c.z.II"));
}

#[test]
fn union_duplicate_keys_reports_both_locations() {
    let mut d = attr_dict();
    let new = yaml("x: 0\nc.z.II: foo");
    let err = d.union(&new, false, false).unwrap_err().to_string();
    assert!(err.contains("first defined at <yaml string>:4:"), "{err}");
    assert!(err.contains("again at <yaml string>:2:"), "{err}");
}

#[test]
fn union_replacement() {
    for to_replace in ["foo", "[]", "{}", "1"] {
        let mut d = attr_dict();
        let new = yaml(&format!("c._REPLACE_: {to_replace}"));
        d.union(&new, true, true).unwrap();
        let expected = yaml(&format!("v: {to_replace}"));
        assert_eq!(
            d.get("c").unwrap(),
            expected.get("v").unwrap(),
            "{to_replace}"
        );
    }
}

#[test]
fn union_replacement_of_missing_key_is_an_error() {
    let mut d = attr_dict();
    let err = d.union(&yaml("q._REPLACE_: 1"), true, true).unwrap_err();
    assert!(matches!(err, TreeError::ReplaceMissing { .. }));
}

#[test]
fn union_empty_dicts() {
    let mut d = attr_dict();
    let new = yaml("'1': {foo: {}}\nbaz: {bar: {}}");
    d.union(&new, false, false).unwrap();
    assert!(d.get_key("baz.bar").unwrap().as_map().unwrap().is_empty());
}

#[test]
fn union_order_retained() {
    let mut d = attr_dict();
    let new = yaml("a: 10\ne: {b: 1, a: 2}\nA: -1\nc.z.II: 20");
    d.union(&new, true, false).unwrap();
    let expected = yaml(
        "a: 10\nb: 2\nd: null\nc: {x: foo, y: bar, z: {I: 1, II: 20}}\ne: {b: 1, a: 2}\nA: -1",
    );
    assert_eq!(d, expected);
    let keys: Vec<_> = d.keys().cloned().collect();
    assert_eq!(keys, ["a", "b", "d", "c", "e", "A"]);
    let e_keys: Vec<_> = d
        .get("e")
        .unwrap()
        .as_map()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(e_keys, ["b", "a"]);
}

#[test]
fn union_null_does_not_wipe_subtree() {
    let mut d = attr_dict();
    d.union(&yaml("c: null"), true, false).unwrap();
    assert_eq!(*get(&d, "c.x"), Value::Str("foo".into()));
}

#[test]
fn del_key_single() {
    let mut d = attr_dict();
    d.del_key("c").unwrap();
    assert!(!d.contains_key("c"));
}

#[test]
fn del_key_nested() {
    let mut d = attr_dict();
    d.del_key("c.z.I").unwrap();
    assert!(
        !d.get_key("c.z")
            .unwrap()
            .as_map()
            .unwrap()
            .contains_key("I")
    );
}

#[test]
fn del_key_removes_emptied_parents() {
    let mut d = yaml("a.b.c: 1\nz: 0");
    d.del_key("a.b.c").unwrap();
    assert!(!d.contains_key("a"));
}
