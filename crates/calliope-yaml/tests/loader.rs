//! YAML loading parity with `ruamel.yaml` `YAML(typ="safe")`. Expected values were produced
//! with ruamel.yaml 0.19.1 from the Calliope pixi environment.

use calliope_yaml::{LoadError, Value, parse_tree, read_yaml_str, resolve_plain};

#[test]
fn plain_scalar_resolution_matches_ruamel() {
    use Value::*;
    let s = |x: &str| Str(x.to_owned());
    let cases: Vec<(&str, Value)> = vec![
        ("012", Int(12)),
        ("0o12", Int(10)),
        ("0x1F", Int(31)),
        ("0b11", Int(3)),
        ("1_000", Int(1000)),
        ("1.5", Float(1.5)),
        ("1e3", Float(1000.0)),
        ("1E3", Float(1000.0)),
        ("1.e3", Float(1000.0)),
        (".5", Float(0.5)),
        ("-.inf", Float(f64::NEG_INFINITY)),
        (".Inf", Float(f64::INFINITY)),
        ("+1", Int(1)),
        ("-0", Int(0)),
        ("yes", s("yes")),
        ("on", s("on")),
        ("Yes", s("Yes")),
        ("True", Bool(true)),
        ("TRUE", Bool(true)),
        ("tRue", s("tRue")),
        ("null", Null),
        ("Null", Null),
        ("~", Null),
        ("", Null),
        ("1:30", s("1:30")),
        ("2005-01-01", Date("2005-01-01".into())),
        ("2005-01-01 00:00", s("2005-01-01 00:00")),
        (
            "2005-01-01 00:00:00",
            DateTime("2005-01-01 00:00:00".into()),
        ),
        ("1e+3", Float(1000.0)),
        (".5e3", s(".5e3")),
        ("5.", Float(5.0)),
        ("+.inf", Float(f64::INFINITY)),
        ("inf", s("inf")),
        ("nan", s("nan")),
        ("0.", Float(0.0)),
        ("00", Int(0)),
        ("08", Int(8)),
        ("1__2", Int(12)),
        ("_1", s("_1")),
        ("0x_1", Int(1)),
        ("1.0e", s("1.0e")),
        ("1e3.5", s("1e3.5")),
        ("12:30:00", s("12:30:00")),
    ];
    for (text, expected) in cases {
        let got = resolve_plain(text);
        assert_eq!(got, expected, "{text:?}");
        assert_eq!(got.type_name(), expected.type_name(), "{text:?}");
    }
    assert!(matches!(resolve_plain(".NaN"), Float(f) if f.is_nan()));
}

#[test]
fn quoted_and_tagged_scalars() {
    let d = read_yaml_str(
        "a: '1'\nb: \"true\"\nc: !!str 12\nd: !!float 1\ne: |\n  12\n",
        false,
    )
    .unwrap();
    assert_eq!(d.get("a").unwrap().value, Value::Str("1".into()));
    assert_eq!(d.get("b").unwrap().value, Value::Str("true".into()));
    assert_eq!(d.get("c").unwrap().value, Value::Str("12".into()));
    assert!(matches!(d.get("d").unwrap().value, Value::Float(f) if f == 1.0));
    assert_eq!(d.get("e").unwrap().value, Value::Str("12\n".into()));
    assert!(read_yaml_str("a: !custom 1", false).is_err());
}

#[test]
fn anchors_and_merge_keys() {
    let d = read_yaml_str("a: &x {b: 1, c: 0}\nc:\n  b: 3\n  <<: *x\n  d: 2", false).unwrap();
    let expected = read_yaml_str("{a: {b: 1, c: 0}, c: {b: 3, c: 0, d: 2}}", false).unwrap();
    assert_eq!(d, expected);
    let keys: Vec<_> = d
        .get("c")
        .unwrap()
        .as_map()
        .unwrap()
        .keys()
        .cloned()
        .collect();
    assert_eq!(keys, ["b", "c", "d"]);

    let d = read_yaml_str(
        "a: &x {b: 1}\nz: &y {b: 2, e: 1}\nc:\n  <<: [*x, *y]\n  d: 2",
        false,
    )
    .unwrap();
    let c = d.get("c").unwrap().as_map().unwrap();
    assert_eq!(c.keys().cloned().collect::<Vec<_>>(), ["b", "e", "d"]);
    assert_eq!(c.get("b").unwrap().value, Value::Int(1));

    let err = read_yaml_str("a: &x {b: 1}\nc:\n  <<: *x\n  <<: *x", false).unwrap_err();
    assert!(matches!(err, LoadError::DuplicateKey { .. }), "{err}");
}

#[test]
fn merged_mapping_replaces_rather_than_merges() {
    // ruamel builds a plain dict first, so an explicit mapping replaces a merged one.
    let d = read_yaml_str("a: &x {m: {p: 1}}\nc:\n  <<: *x\n  m: {q: 2}", false).unwrap();
    let m = d.get_key("c.m").unwrap().as_map().unwrap();
    assert_eq!(m.keys().cloned().collect::<Vec<_>>(), ["q"]);
}

#[test]
fn anchors_in_math_like_yaml() {
    let text = "
constraints:
  a:
    sub_expressions:
      scaler: &scaler
        - where: x
          expression: '1'
  b:
    sub_expressions:
      scaler: *scaler
";
    let d = read_yaml_str(text, false).unwrap();
    assert_eq!(
        d.get_key("constraints.a.sub_expressions.scaler").unwrap(),
        d.get_key("constraints.b.sub_expressions.scaler").unwrap()
    );
}

#[test]
fn keys_are_strings() {
    let d = read_yaml_str("1: a\ntrue: b", false).unwrap();
    assert_eq!(d.keys().cloned().collect::<Vec<_>>(), ["1", "True"]);
    assert!(read_yaml_str("1.5: a", false).is_err());
    assert!(read_yaml_str("null: a", false).is_err());
}

#[test]
fn dotted_and_nested_definitions_merge_in_order() {
    let d = read_yaml_str("a.b: 1\na: {c: 2}\na.b: 3", false);
    // `a.b` twice is a literal duplicate key in the YAML mapping.
    assert!(d.is_err());
    let d = read_yaml_str("a.b: 1\na: {c: 2}\nx.y: 0", false).unwrap();
    assert_eq!(d.keys_nested(), ["a.b", "a.c", "x.y"]);
    let d = read_yaml_str("a: {c: 2}\na.c: 3", false).unwrap();
    assert_eq!(d.get_key("a.c").unwrap().value, Value::Int(3));
}

#[test]
fn spans_are_one_based() {
    let src = calliope_yaml::Source::new("model.yaml");
    let d = parse_tree("top:\n  key: [1, 2]\n  other: x\n", &src).unwrap();
    let key = d.get_key("top.key").unwrap();
    let span = key.span.as_ref().unwrap();
    assert_eq!((span.line, span.col), (2, 8));
    let other = d.get_key("top.other").unwrap().span.as_ref().unwrap();
    assert_eq!(other.to_string(), "model.yaml:3:10");
    let item = &key.as_seq().unwrap()[1];
    assert_eq!(item.span.as_ref().unwrap().col, 12);
}

#[test]
fn syntax_error_has_location() {
    let err = read_yaml_str("a: [1, 2\nb: 3", false).unwrap_err();
    assert!(
        err.to_string()
            .starts_with("Parser error when reading YAML from <yaml string>:"),
        "{err}"
    );
}

/// Every YAML file of the reference checkout loads. Byte-level parity with Python is checked
/// with `examples/dump_yaml.rs` and `examples/parity_dump.py`.
#[test]
fn reference_yaml_files_load() {
    let root = std::env::var("CALLIOPE_REFERENCE").map_or_else(
        |_| std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        std::path::PathBuf::from,
    );
    if !root.join("src/calliope").is_dir() {
        eprintln!(
            "skipping: no Calliope reference checkout at {}",
            root.display()
        );
        return;
    }
    let mut stack = vec![
        root.join("src/calliope"),
        root.join("tests"),
        root.join("docs"),
    ];
    let mut n = 0;
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "yaml") {
                calliope_yaml::read_yaml_file(&path, false)
                    .unwrap_or_else(|e| panic!("{}: {e}", path.display()));
                n += 1;
            }
        }
    }
    assert!(n >= 30, "only {n} YAML files found");
}
