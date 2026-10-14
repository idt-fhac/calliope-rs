//! Port of `TestYaml` in `tests/io_test.py`, plus import-cycle and source-location tests.

use std::fs;

use calliope_yaml::{AttrTree, LoadError, Value, read_yaml_file, read_yaml_str, to_yaml};

const TEST_TEXT: [(&str, &str, &str); 4] = [
    (
        "simple_nested",
        "\nsomekey.nested: 1\nanotherkey: 2\n",
        "{somekey: {nested: 1}, anotherkey: 2}",
    ),
    (
        "triple_nested",
        "\nfoo:\n    bar: 1\n    baz: 2\n    nested:\n        value: 5\n",
        "{foo: {bar: 1, baz: 2, nested: {value: 5}}}",
    ),
    (
        "complex_commented",
        "
# a comment
a: 1
b: 2
# a comment about `c`
c:  # a comment inline with `c`
    x: foo  # a comment on foo

    #
    y: bar  #
    z:
        I: 1
        II: 2
d:
e: False
",
        "{a: 1, b: 2, c: {x: foo, y: bar, z: {I: 1, II: 2}}, d: null, e: false}",
    ),
    (
        "nested_string",
        "a.b.c: 1\na.b.foo: 2\nb.a.c.bar: foo",
        "{a: {b: {c: 1, foo: 2}}, b: {a: {c: {bar: foo}}}}",
    ),
];

fn expected(flow: &str) -> AttrTree {
    // Expected values are written as flow mappings without dotted keys.
    read_yaml_str(flow, false).unwrap()
}

#[test]
fn text_read() {
    for (name, text, exp) in TEST_TEXT {
        assert_eq!(read_yaml_str(text, false).unwrap(), expected(exp), "{name}");
    }
}

#[test]
fn file_read() {
    let dir = tempfile::tempdir().unwrap();
    for (name, text, exp) in TEST_TEXT {
        let file = dir.path().join(format!("{name}.yaml"));
        fs::write(&file, text).unwrap();
        assert_eq!(
            read_yaml_file(&file, false).unwrap(),
            expected(exp),
            "{name}"
        );
    }
}

#[test]
fn text_import_error() {
    for bad in [
        "import: ['somefile.yaml']\n",
        "import: ['somefile.yaml', 'other_file.yaml']\n",
    ] {
        for (_, text, _) in TEST_TEXT {
            let err = read_yaml_str(&format!("{bad}{text}"), false).unwrap_err();
            assert!(matches!(err, LoadError::ImportWithoutFile));
            assert_eq!(
                err.to_string(),
                "Imports are not possible for non-file yaml inputs."
            );
        }
    }
}

fn dummy_imported_file(dir: &std::path::Path) -> std::path::PathBuf {
    let file = dir.join("test_import.yaml");
    fs::write(
        &file,
        "\n# Comment\nimport_key_a.nested: 1\nimport_key_b: 2\nimport_key_c: [1, 2, 3]\n        ",
    )
    .unwrap();
    file
}

#[test]
fn import() {
    let dir = tempfile::tempdir().unwrap();
    let imported = dummy_imported_file(dir.path());
    for (name, text, _) in TEST_TEXT {
        let file = dir.path().join(format!("{name}_relative.yaml"));
        let import_text = format!(
            "\nimport:\n    - {}\n",
            imported.file_name().unwrap().to_str().unwrap()
        );
        fs::write(&file, import_text + text).unwrap();
        let d = read_yaml_file(&file, false).unwrap();
        assert!(d.keys_nested().contains(&"import_key_a.nested".to_owned()));
        assert_eq!(d.get_key("import_key_b").unwrap().value, Value::Int(2));
        assert_eq!(
            d.get("import_key_c").unwrap().value,
            read_yaml_str("v: [1, 2, 3]", false)
                .unwrap()
                .get("v")
                .unwrap()
                .value
        );
        assert!(!d.contains_key("import"));
    }
}

#[test]
fn invalid_import_type_error() {
    let dir = tempfile::tempdir().unwrap();
    let imported = dummy_imported_file(dir.path());
    for (name, text, _) in TEST_TEXT {
        let file = dir.path().join(format!("{name}_invalid_import_type.yaml"));
        let import_text = format!(
            "import: {}\n",
            imported.file_name().unwrap().to_str().unwrap()
        );
        fs::write(&file, import_text + text).unwrap();
        let err = read_yaml_file(&file, false).unwrap_err();
        assert!(err.to_string().contains("`import` must be a list"), "{err}");
    }
}

#[test]
fn duplicate_dot_string_error() {
    let err = read_yaml_str("a.b.c: 1\na.b.c: 2", false).unwrap_err();
    assert!(matches!(err, LoadError::DuplicateKey { .. }));
    assert!(err.to_string().contains("<yaml string>:2:1"), "{err}");
}

#[test]
fn simple_invalid_yaml() {
    let err = read_yaml_str("1 this is not valid yaml", false).unwrap_err();
    assert!(
        err.to_string()
            .contains("Could not parse <yaml string> as YAML")
    );
}

#[test]
fn parser_error() {
    let err = read_yaml_str(
        "\n            foo: bar\n            baz: 1\n                - foobar\n                bar: baz\n\n            ",
        false,
    )
    .unwrap_err();
    assert!(matches!(err, LoadError::Syntax { .. }), "{err}");
}

#[test]
fn as_dict_with_sublists() {
    let d = read_yaml_str("a: [{x: 1}, {y: 2}]", false).unwrap();
    let first = &d.get("a").unwrap().as_seq().unwrap()[0];
    assert_eq!(
        first.as_map().unwrap().get("x").unwrap().value,
        Value::Int(1)
    );
}

#[test]
fn replacement_null_from_file() {
    let mut d = read_yaml_str(
        "\n            A.B.C: 10\n            A.B:\n                E: 20\n            C: \"foobar\"\n        ",
        false,
    )
    .unwrap();
    let replacement = read_yaml_str("C._REPLACE_: null", false).unwrap();
    d.union(&replacement, true, true).unwrap();
    assert!(d.get("C").unwrap().is_null());
    assert_eq!(d.get_key("A.B.C").unwrap().value, Value::Int(10));
    assert_eq!(d.get_key("A.B.E").unwrap().value, Value::Int(20));
}

#[test]
fn to_yaml_roundtrip() {
    for (name, _, exp) in TEST_TEXT {
        let tree = expected(exp);
        let text = to_yaml(&tree);
        assert_eq!(
            read_yaml_str(&text, false).unwrap(),
            tree,
            "{name}:\n{text}"
        );
    }
}

#[test]
fn to_yaml_complex() {
    for (_, text, _) in TEST_TEXT {
        let mut d = read_yaml_str(text, false).unwrap();
        d.set_key("numpy.some_int", 10i64.into()).unwrap();
        d.set_key("numpy.some_float", 0.5f64.into()).unwrap();
        d.set_key("a_list", vec![0i64.into(), 1i64.into(), 2i64.into()].into())
            .unwrap();
        let out = to_yaml(&d);
        assert!(out.contains("some_int: 10"), "{out}");
        assert!(out.contains("some_float: 0.5"), "{out}");
        assert!(out.contains("a_list:\n- 0\n- 1\n- 2"), "{out}");
    }
}

#[test]
fn to_yaml_roundtrip_tricky_scalars() {
    let text = "
a: 'yes'
b: '1.5'
c: 'null'
d: ''
e: ' padded '
f: 'x: y'
g: \"line\\nbreak\"
h: .inf
i: -.inf
j: .nan
k: 1e-10
l: 2005-01-01
m: [[1, 2], [{x: 1, y: [a, b]}]]
n: {}
o: []
p: '#hash'
q: it's
";
    let tree = read_yaml_str(text, false).unwrap();
    let out = to_yaml(&tree);
    let back = read_yaml_str(&out, false).unwrap();
    // NaN != NaN, so compare everything else and check `j` separately.
    let mut t1 = tree.clone();
    let mut t2 = back.clone();
    t1.remove("j");
    t2.remove("j");
    assert_eq!(t1, t2, "{out}");
    assert!(matches!(back.get("j").unwrap().value, Value::Float(f) if f.is_nan()));
}

#[test]
fn import_precedence_and_order() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.yaml"), "x: a\nonly_a: 1\n").unwrap();
    fs::write(dir.path().join("b.yaml"), "y: b\nonly_b: 1\n").unwrap();
    let main = dir.path().join("main.yaml");
    fs::write(&main, "import: [a.yaml, b.yaml]\nz: main\n").unwrap();
    let d = read_yaml_file(&main, false).unwrap();
    let keys: Vec<_> = d.keys().cloned().collect();
    assert_eq!(keys, ["y", "only_b", "x", "only_a", "z"]);

    // A key in both an import and the importing file needs allow_override.
    fs::write(&main, "import: [a.yaml]\nx: main\n").unwrap();
    let err = read_yaml_file(&main, false).unwrap_err();
    assert!(err.to_string().contains("Key defined twice: x"), "{err}");
    let d = read_yaml_file(&main, true).unwrap();
    assert_eq!(d.get("x").unwrap().value, Value::Str("main".into()));
}

#[test]
fn nested_imports_resolve_relative_to_importing_file() {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir(dir.path().join("sub")).unwrap();
    fs::write(dir.path().join("sub/inner.yaml"), "inner: 1\n").unwrap();
    fs::write(
        dir.path().join("sub/mid.yaml"),
        "import: [inner.yaml]\nmid: 1\n",
    )
    .unwrap();
    let main = dir.path().join("main.yaml");
    fs::write(&main, "import: [sub/mid.yaml]\nmain: 1\n").unwrap();
    let d = read_yaml_file(&main, false).unwrap();
    assert_eq!(d.keys_nested(), ["inner", "mid", "main"]);
}

#[test]
fn import_cycle_is_detected() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("a.yaml"), "import: [b.yaml]\na: 1\n").unwrap();
    fs::write(dir.path().join("b.yaml"), "import: [a.yaml]\nb: 1\n").unwrap();
    let err = read_yaml_file(dir.path().join("a.yaml"), false).unwrap_err();
    assert!(
        matches!(err, LoadError::ImportCycle { ref chain } if chain.len() == 3),
        "{err}"
    );
    assert!(
        err.to_string().starts_with("Circular YAML import:"),
        "{err}"
    );

    fs::write(dir.path().join("self.yaml"), "import: [self.yaml]\n").unwrap();
    let err = read_yaml_file(dir.path().join("self.yaml"), false).unwrap_err();
    assert!(matches!(err, LoadError::ImportCycle { .. }), "{err}");
}

#[test]
fn same_file_imported_twice_is_not_a_cycle() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("common.yaml"), "c: 1\n").unwrap();
    fs::write(dir.path().join("x.yaml"), "import: [common.yaml]\nx: 1\n").unwrap();
    fs::write(dir.path().join("y.yaml"), "import: [common.yaml]\ny: 1\n").unwrap();
    let main = dir.path().join("main.yaml");
    fs::write(&main, "import: [x.yaml, y.yaml]\n").unwrap();
    // `c` comes from both imports, so it is defined twice (as in Python).
    let err = read_yaml_file(&main, false).unwrap_err();
    assert!(err.to_string().contains("Key defined twice: c"), "{err}");
}

#[test]
fn locations_survive_imports() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("techs.yaml"),
        "techs:\n  ccgt:\n    flow_cap_max: 10\n",
    )
    .unwrap();
    let main = dir.path().join("model.yaml");
    fs::write(&main, "import: [techs.yaml]\nnodes: {}\n").unwrap();
    let d = read_yaml_file(&main, false).unwrap();
    let span = d
        .get_key("techs.ccgt.flow_cap_max")
        .unwrap()
        .span
        .clone()
        .unwrap();
    assert!(span.source.name().ends_with("techs.yaml"));
    assert_eq!((span.line, span.col), (3, 19));
}

#[test]
fn missing_import_names_the_importer() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.yaml");
    fs::write(&main, "import: [nope.yaml]\n").unwrap();
    let err = read_yaml_file(&main, false).unwrap_err().to_string();
    assert!(
        err.contains("main.yaml") && err.contains("nope.yaml"),
        "{err}"
    );
}

#[test]
fn empty_import_list_is_kept() {
    // Python only resolves a truthy `import` and leaves `import: []` in place.
    let d = read_yaml_str("import: []\na: 1", false).unwrap();
    assert!(d.contains_key("import"));
}
