//! Port of `tests/preprocess/data_sources_test.py`.
//!
//! Python builds most tables in memory (`data_table_dfs`); the port reads CSV only, so each
//! table is written as the CSV that `DataFrame.to_csv` produces for it.

use calliope_preprocess::ModelError;
use calliope_preprocess::data_tables::DataTable;
use calliope_preprocess::xr::Dataset;
use calliope_schema::{CalliopeDataTable, CalliopeTechs, Validated};
use calliope_yaml::{AttrTree, read_yaml_str};

const DUMMY_INT: i64 = 0xdead;

fn yaml(text: &str) -> AttrTree {
    read_yaml_str(text, false).unwrap()
}

fn table_def(text: &str) -> Result<Validated<CalliopeDataTable>, String> {
    Validated::from_tree(yaml(text)).map_err(|e| e.to_string())
}

/// Writes `csv` to a temporary file and loads it with the table definition `def` (YAML,
/// without `table`).
fn load(csv: &str, def: &str) -> Result<DataTable, ModelError> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.csv");
    std::fs::write(&path, csv).unwrap();
    let def = format!("table: {}\n{def}", path.display());
    let t = table_def(&def).unwrap();
    DataTable::new("ds_name", &t, None)
}

/// Non-missing cells of a variable as `(labels, value)`, sorted by labels.
fn cells(ds: &Dataset, var: &str) -> Vec<(Vec<String>, String)> {
    let da = ds.get(var).unwrap();
    let shape = da.shape();
    let mut out = Vec::new();
    for i in 0..da.data.len() {
        if da.data.is_null(i) {
            continue;
        }
        let mut rem = i;
        let mut idx = vec![0; shape.len()];
        for k in (0..shape.len()).rev() {
            idx[k] = rem % shape[k];
            rem /= shape[k];
        }
        let labels = idx
            .iter()
            .enumerate()
            .map(|(k, j)| da.coords[k].labels()[*j].to_string())
            .collect();
        let v = da.data.get(i);
        // Values compare as numbers where they are numbers (Python: `check_dtype=False`).
        let v = v.as_f64().map_or_else(|| v.to_py_str(), |f| f.to_string());
        out.push((labels, v));
    }
    out.sort();
    out
}

fn c(labels: &[&str], v: &str) -> (Vec<String>, String) {
    (
        labels.iter().map(|s| (*s).to_owned()).collect(),
        v.to_owned(),
    )
}

fn dims(ds: &Dataset, var: &str) -> Vec<String> {
    let mut d: Vec<String> = ds.vars[var].dims.clone();
    d.sort();
    d
}

#[test]
fn utils_name_and_error() {
    let t = load(
        "test_row,0\nbar,0\nbaz,1\n",
        "rows: test_row\nadd_dims: {inputs: test_param}",
    )
    .unwrap();
    assert_eq!(t.name, "ds_name");
    assert_eq!(t.error("bar").to_string(), "(data_tables, ds_name) | bar.");
}

#[test]
fn compare_axis_names() {
    // Loaded names that are None/int pass; others must match.
    let csv = "test_row1,test_row2,foo\nbar1,bar2,0\nbaz1,baz2,7\n";
    let ok = load(
        csv,
        "rows: [test_row1, test_row2]\ncolumns: test_col\nadd_dims: {inputs: test_param}",
    );
    assert!(ok.is_ok());
    let err = load(
        csv,
        "rows: [foobar, test_row2]\ncolumns: [test_col]\nadd_dims: {inputs: test_param}",
    )
    .unwrap_err();
    assert!(
        err.to_string().contains(
            "Trying to set names for index but names in the file do no match names provided | \
             in file: ['test_row1', 'test_row2'] | defined: ['foobar', 'test_row2']."
        ),
        "{err}"
    );
}

#[test]
fn init_one_level() {
    let def_row = "rows: test_row\nadd_dims: {inputs: test_param}";
    let t = load("test_row,0\nbar,0\nbaz,1\n", def_row).unwrap();
    assert_eq!(dims(&t.dataset, "test_param"), ["test_row"]);
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [c(&["bar"], "0"), c(&["baz"], "1")]
    );
    let def = "rows: test_row\ncolumns: test_col\nadd_dims: {inputs: test_param}";
    for (csv, expected) in [
        (
            "test_row,foo\nbar,0\nbaz,1\n",
            vec![c(&["bar", "foo"], "0"), c(&["baz", "foo"], "1")],
        ),
        (
            "test_row,foo,foobar\nbar,0,1\n",
            vec![c(&["bar", "foo"], "0"), c(&["bar", "foobar"], "1")],
        ),
        (
            "test_row,foo,foobar\nbar,0,0\nbaz,10,20\n",
            vec![
                c(&["bar", "foo"], "0"),
                c(&["bar", "foobar"], "0"),
                c(&["baz", "foo"], "10"),
                c(&["baz", "foobar"], "20"),
            ],
        ),
    ] {
        let t = load(csv, def).unwrap();
        assert_eq!(dims(&t.dataset, "test_param"), ["test_col", "test_row"]);
        assert_eq!(cells(&t.dataset, "test_param"), expected);
    }
}

#[test]
fn init_multi_level() {
    let t = load(
        "test_row1,test_row2,0\nbar1,bar2,0\nbaz1,baz2,1\n",
        "rows: [test_row1, test_row2]\nadd_dims: {inputs: test_param}",
    )
    .unwrap();
    assert_eq!(dims(&t.dataset, "test_param"), ["test_row1", "test_row2"]);
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [c(&["bar1", "bar2"], "0"), c(&["baz1", "baz2"], "1")]
    );
    let t = load(
        ",foo1,foobar1\n,foo2,foobar2\ntest_row,,\nbar,0,1\n",
        "rows: [test_row]\ncolumns: [test_col1, test_col2]\nadd_dims: {inputs: test_param}",
    )
    .unwrap();
    assert_eq!(
        dims(&t.dataset, "test_param"),
        ["test_col1", "test_col2", "test_row"]
    );
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [
            c(&["bar", "foo1", "foo2"], "0"),
            c(&["bar", "foobar1", "foobar2"], "1")
        ]
    );
    let t = load(
        ",,foo1,foobar1\n,,foo2,foobar2\ntest_row1,test_row2,,\nbar1,bar2,0,0\nbaz1,baz2,10,20\n",
        "rows: [test_row1, test_row2]\ncolumns: [test_col1, test_col2]\nadd_dims: {inputs: test_param}",
    )
    .unwrap();
    assert_eq!(cells(&t.dataset, "test_param").len(), 4);
    assert!(
        cells(&t.dataset, "test_param").contains(&c(&["baz1", "baz2", "foobar1", "foobar2"], "20"))
    );
}

const SEL_CSV: &str = ",,test_param\nbar1,baz1,0\nbar2,baz2,1\nbar3,baz3,2\nbar1,baz4,3\n";

fn sel_table(extra: &str) -> Result<DataTable, ModelError> {
    load(
        SEL_CSV,
        &format!("rows: [test_row1, test_row2]\ncolumns: inputs\n{extra}"),
    )
}

#[test]
fn select_keep_one() {
    let t = sel_table("select: {test_row1: bar1}").unwrap();
    assert_eq!(
        t.dataset.coord("test_row1").unwrap().labels(),
        [calliope_array::Label::str("bar1")]
    );
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [c(&["bar1", "baz1"], "0"), c(&["bar1", "baz4"], "3")]
    );
}

#[test]
fn select_keep_two() {
    let t = sel_table("select: {test_row1: [bar1, bar2]}").unwrap();
    assert_eq!(t.dataset.coord("test_row1").unwrap().len(), 2);
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [
            c(&["bar1", "baz1"], "0"),
            c(&["bar1", "baz4"], "3"),
            c(&["bar2", "baz2"], "1")
        ]
    );
}

#[test]
fn select_drop_one() {
    let t = sel_table("select: {test_row1: bar2, test_row2: baz2}\ndrop: [test_row1, test_row2]")
        .unwrap();
    assert!(t.dataset.coords.is_empty());
    assert_eq!(t.dataset.get("test_param").unwrap().item().to_py_str(), "1");
}

#[test]
fn select_drop_two() {
    let t = sel_table("select: {test_row1: bar1}\ndrop: test_row1").unwrap();
    assert!(t.dataset.coord("test_row1").is_none());
    assert_eq!(
        cells(&t.dataset, "test_param"),
        [c(&["baz1"], "0"), c(&["baz4"], "3")]
    );
}

#[test]
fn drop_one() {
    let t = sel_table("drop: test_row1").unwrap();
    assert!(t.dataset.coord("test_row1").is_none());
    assert_eq!(cells(&t.dataset, "test_param").len(), 4);
}

#[test]
fn rename_dims() {
    let csv = format!("test_row1,test_row2,foo\nbar1,bar2,0\nbaz1,baz2,{DUMMY_INT}\n");
    for (mapping, idx, col) in [
        ("{test_row1: foobar}", "[foobar, test_row2]", "[test_col]"),
        (
            "{test_row1: foobar, test_col: foobaz}",
            "[foobar, test_row2]",
            "[foobaz]",
        ),
    ] {
        let t = load(
            &csv,
            &format!("rows: {idx}\ncolumns: {col}\nadd_dims: {{inputs: test_param}}\nrename_dims: {mapping}"),
        )
        .unwrap();
        assert!(t.dataset.coord("foobar").is_some());
        assert!(t.dataset.coord("test_row1").is_none());
        assert!(
            cells(&t.dataset, "test_param")
                .iter()
                .any(|(l, v)| l[0] == "baz1" && l[1] == "baz2" && *v == DUMMY_INT.to_string())
        );
    }
}

const MALFORMED_CSV: &str = ",,foo\nbar1,baz1,0\nbar2,baz2,1\nbar3,baz3,2\nbar1,baz4,3\n";

fn malformed(extra: &str) -> Result<DataTable, String> {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("table.csv");
    std::fs::write(&path, MALFORMED_CSV).unwrap();
    let rows = if extra.contains("rows:") {
        ""
    } else {
        "rows: [test_row1, test_row2]\n"
    };
    let def = format!("table: {}\n{rows}{extra}", path.display());
    let t = table_def(&def)?;
    DataTable::new("ds_name", &t, None).map_err(|e| e.to_string())
}

#[test]
fn malformed_tables() {
    let e = malformed("").unwrap_err();
    assert!(e.contains("The `inputs` dimension must exist"), "{e}");
    let e = malformed("drop: test_row2\nadd_dims: {inputs: test_param}").unwrap_err();
    assert!(e.contains("Duplicate index items found:"), "{e}");
    let e = malformed("add_dims: {test_row2: foo, inputs: test_param}").unwrap_err();
    assert!(
        e.contains("Added dimensions must not be in columns or rows."),
        "{e}"
    );
    // Python checks an in-memory frame ("Expected 2 columns levels in loaded data."); read
    // from CSV, the second header row is data, so the level names do not match.
    let e = malformed("columns: [foo, bar]").unwrap_err();
    assert!(e.contains("Trying to set names for columns"), "{e}");
    let e = malformed("rows: null\nadd_dims: {inputs: test_param}").unwrap_err();
    assert!(e.contains("Either row or columns must be defined"), "{e}");
    let e = malformed("add_dims: {inputs: template}").unwrap_err();
    assert!(
        e.contains("Template calls (`template`) can only be used in the YAML model definition."),
        "{e}"
    );
}

fn lookup_table() -> DataTable {
    load(
        ",,FOO,BAR\nfoo1,bar1,1.0,1.0\nfoo1,bar2,1.0,\nfoo2,bar2,,1.0\n",
        "rows: [techs, carriers]\ncolumns: inputs",
    )
    .unwrap()
}

#[test]
fn lookup_dict_from_param() {
    let t = lookup_table();
    assert_eq!(
        t.lookup_def_from_param("FOO", "carriers").unwrap(),
        yaml("foo1: {FOO: [bar1, bar2]}")
    );
    assert_eq!(
        t.lookup_def_from_param("BAR", "carriers").unwrap(),
        yaml("foo1: {BAR: bar1}\nfoo2: {BAR: bar2}")
    );
    let e = t.lookup_def_from_param("FOO", "foobar").unwrap_err();
    assert!(
        e.to_string().contains(
            "Loading FOO with missing dimension(s). Must contain `techs` and `foobar`, received: ('techs', 'carriers')"
        ),
        "{e}"
    );
}

fn techs_table(csv: &str, rows: &str) -> DataTable {
    load(csv, &format!("rows: {rows}\ncolumns: inputs")).unwrap()
}

#[test]
fn tech_dicts() {
    let (t, b) = techs_table(",test_param\nfoo1,1\nfoo2,2\n", "techs")
        .tech_def()
        .unwrap();
    assert_eq!(t, yaml("{foo1: {}, foo2: {}}"));
    assert!(b.is_empty());

    let (t, b) = techs_table(
        ",foo,bar\nfoo1,1.0,\nfoo2,2.0,\nbar1,,1.0\nbar2,,2.0\n",
        "techs",
    )
    .tech_def()
    .unwrap();
    assert_eq!(t, yaml("{foo1: {}, foo2: {}, bar1: {}, bar2: {}}"));
    assert!(b.is_empty());

    let (t, b) = techs_table(",base_tech\nfoo1,transmission\nfoo2,supply\n", "techs")
        .tech_def()
        .unwrap();
    assert_eq!(t, yaml("{foo1: {}, foo2: {}}"));
    assert_eq!(
        b,
        yaml("{foo1: {base_tech: transmission}, foo2: {base_tech: supply}}")
    );

    let (t, b) = techs_table(
        ",base_tech,other_param\nfoo1,transmission,\nbar1,,1.0\n",
        "techs",
    )
    .tech_def()
    .unwrap();
    assert_eq!(t, yaml("{foo1: {}, bar1: {}}"));
    assert_eq!(b, yaml("{foo1: {base_tech: transmission}}"));

    let (t, b) = techs_table(
        ",link_from,link_to\nfoo1,bar1,bar2\nfoo2,bar2,\nfoo3,,bar1\n",
        "techs",
    )
    .tech_def()
    .unwrap();
    assert_eq!(t, yaml("{foo1: {}, foo2: {}, foo3: {}}"));
    assert_eq!(
        b,
        yaml(
            "{foo1: {link_from: bar1, link_to: bar2}, foo2: {link_from: bar2}, foo3: {link_to: bar1}}"
        )
    );

    let (_, b) = techs_table(",active\nfoo1,True\nfoo2,False\n", "techs")
        .tech_def()
        .unwrap();
    assert_eq!(b, yaml("{foo1: {active: true}, foo2: {active: false}}"));

    let (t, b) = techs_table(",available_area\nfoo1,1\n", "nodes")
        .tech_def()
        .unwrap();
    assert!(t.is_empty() && b.is_empty());
}

fn techs(text: &str) -> Validated<CalliopeTechs> {
    Validated::from_tree(yaml(text)).unwrap()
}

#[test]
fn node_dicts() {
    let nt = |csv: &str| techs_table(csv, "[nodes, techs]");
    let bar12 = techs("{bar1: {}, bar2: {}}");
    assert_eq!(
        nt(",,available_area\nfoo1,bar1,1\nfoo2,bar2,2\n")
            .node_def(&bar12)
            .unwrap(),
        yaml("{foo1: {techs: {bar1: {}}}, foo2: {techs: {bar2: {}}}}")
    );
    assert_eq!(
        nt(",,available_area,other_param\nfoo1,bar1,1.0,\nfoo1,bar2,2.0,\nfoo2,bar2,,1.0\n")
            .node_def(&bar12)
            .unwrap(),
        yaml("{foo1: {techs: {bar1: {}, bar2: {}}}, foo2: {techs: {bar2: {}}}}")
    );
    assert_eq!(
        techs_table(
            ",,,available_area\nfoo1,bar1,baz1,1\nfoo2,bar2,baz2,2\n",
            "[nodes, techs, carriers]"
        )
        .node_def(&bar12)
        .unwrap(),
        yaml("{foo1: {techs: {bar1: {}}}, foo2: {techs: {bar2: {}}}}")
    );
    assert_eq!(
        nt(",,active\nfoo1,bar1,False\nfoo2,bar1,True\n")
            .node_def(&bar12)
            .unwrap(),
        yaml("{foo1: {techs: {bar1: {active: false}}}, foo2: {techs: {bar1: {active: true}}}}")
    );
    assert_eq!(
        nt(",,active,available_area\nfoo1,bar1,False,\nfoo2,bar1,True,\nfoo1,bar2,,1.0\n")
            .node_def(&bar12)
            .unwrap(),
        yaml(
            "{foo1: {techs: {bar1: {active: false}, bar2: {}}}, foo2: {techs: {bar1: {active: true}}}}"
        )
    );
    assert_eq!(
        techs_table(
            ",,active\nbar1,foo1,False\nbar1,foo2,True\n",
            "[techs, nodes]"
        )
        .node_def(&bar12)
        .unwrap(),
        yaml("{foo1: {techs: {bar1: {active: false}}}, foo2: {techs: {bar1: {active: true}}}}")
    );
    let with_trans = techs("{bar1: {base_tech: transmission}, bar2: {}}");
    assert!(
        techs_table(",param\nfoo1,1\nfoo2,2\n", "techs")
            .node_def(&with_trans)
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        nt(",,available_area\nfoo1,bar1,1\nfoo1,bar2,2\n")
            .node_def(&techs("{bar1: {}}"))
            .unwrap(),
        yaml("{foo1: {techs: {bar1: {}}}}")
    );
    let e = nt(",,param\nfoo1,bar1,1\nfoo2,bar2,2\n")
        .node_def(&with_trans)
        .unwrap_err();
    assert!(
        e.to_string()
            .contains("Cannot define transmission technology data over the `nodes` dimension"),
        "{e}"
    );
}

#[test]
fn csv_dtype_inference() {
    use calliope_preprocess::table::infer_column;
    use calliope_preprocess::xr::DType;
    let dt = |cells: &[&str]| infer_column(cells).dtype();
    assert_eq!(dt(&["1", "2"]), DType::Int);
    assert_eq!(dt(&["1", ""]), DType::Float);
    assert_eq!(dt(&["1", "2.5"]), DType::Float);
    assert_eq!(dt(&["inf", "-inf", "1"]), DType::Float);
    assert_eq!(dt(&["True", "False"]), DType::Bool);
    assert_eq!(dt(&["True", ""]), DType::Object);
    assert_eq!(dt(&["True", "1"]), DType::Object);
    assert_eq!(dt(&["a", "NA"]), DType::Object);
    assert_eq!(dt(&["", "nan"]), DType::Float);
}
