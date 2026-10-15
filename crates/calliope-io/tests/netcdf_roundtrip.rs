//! Model NetCDF round trips: Python-written files read in Rust, rewritten by Rust, and the
//! Rust files read by Python `calliope.read_netcdf` with equal attrs and data.
//!
//! The Python side needs the reference checkout with its pixi env (`CALLIOPE_PY_DIR`,
//! default `../calliope` next to this workspace); it is skipped with a note when absent.

use calliope_io::{AttrValue, ModelNetcdf, compare_datasets, read_netcdf, write_netcdf};
use std::path::{Path, PathBuf};
use std::process::Command;

/// libnetcdf/HDF5 calls from parallel test threads on the same files fail (-101); run the
/// tests in this file one at a time.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

const FIXTURES: &[&str] = &[
    "test_model__simple_supply__two_hours__investment_costs__shadow_prices",
    "national_scale_spores_integer",
    "national_scale_operate",
    "test_model__supply_purchase__two_hours",
    "urban_scale_milp",
    "national_scale_time_clustering",
];

fn golden(f: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../golden")
        .join(f)
        .join("results.nc")
}

/// `pixi run python` in the reference checkout, if available.
pub fn python() -> Option<(PathBuf, Command)> {
    let dir = std::env::var_os("CALLIOPE_PY_DIR").map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        PathBuf::from,
    );
    if !dir.join("pixi.toml").exists() && !dir.join("pyproject.toml").exists() {
        return None;
    }
    let mut c = Command::new("pixi");
    c.current_dir(&dir).args(["run", "python", "-I"]);
    Some((dir, c))
}

fn py_compare(expected: &Path, actual: &Path, rtol: f64) -> Option<String> {
    let (_, mut cmd) = python()?;
    let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/py/nc_compare.py");
    let out = cmd
        .arg(script)
        .arg(expected)
        .arg(actual)
        .arg(rtol.to_string())
        .output()
        .ok()?;
    Some(format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    ))
}

fn assert_same(a: &ModelNetcdf, b: &ModelNetcdf, what: &str) {
    for (g, x, y) in [
        ("inputs", &a.inputs, &b.inputs),
        ("results", &a.results, &b.results),
    ] {
        let d = compare_datasets(&x.ds, &y.ds, 0.0);
        assert!(d.is_empty(), "{what} {g}: {d:?}");
        assert_eq!(x.attrs, y.attrs, "{what} {g} attrs");
        assert_eq!(x.var_attrs, y.var_attrs, "{what} {g} var attrs");
    }
    assert_eq!(a.attrs, b.attrs, "{what} model attrs");
}

#[test]
fn python_files_round_trip_through_rust() {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let mut python_ran = false;
    for f in FIXTURES {
        let src = golden(f);
        let m = read_netcdf(&src).unwrap();
        for k in ["config", "math", "runtime", "definition"] {
            assert!(
                matches!(m.attrs.get(k), Some(AttrValue::Dict(_))),
                "{f}: attrs.{k} is {:?}",
                m.attrs.get(k).map(|_| "not a dict")
            );
        }
        let out = tmp.path().join(format!("{f}.nc"));
        write_netcdf(&out, &m).unwrap();
        let back = read_netcdf(&out).unwrap();
        assert_same(&m, &back, f);
        if let Some(report) = py_compare(&src, &out, 0.0) {
            python_ran = true;
            assert!(report.trim_end().ends_with("OK"), "{f}: {report}");
        }
    }
    if !python_ran {
        eprintln!("note: Python reference env not found; Python side of the round trip skipped");
    }
}

fn read_dir_sorted(dir: &Path) -> Vec<(String, String)> {
    let mut v: Vec<(String, String)> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| {
            let p = e.unwrap().path();
            (
                p.file_name().unwrap().to_string_lossy().into_owned(),
                std::fs::read_to_string(&p).unwrap(),
            )
        })
        .collect();
    v.sort();
    v
}

#[test]
fn csv_export_matches_python() {
    let _g = LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    for f in [FIXTURES[0], FIXTURES[1]] {
        let src = golden(f);
        let rust_dir = tmp.path().join(format!("{f}_rs"));
        let m = read_netcdf(&src).unwrap();
        calliope_io::model_to_csv(&m, &rust_dir, true, false).unwrap();
        assert!(calliope_io::model_to_csv(&m, &rust_dir, true, false).is_err());
        let Some((_, mut cmd)) = python() else {
            eprintln!("note: Python reference env not found; CSV comparison skipped");
            return;
        };
        let py_dir = tmp.path().join(format!("{f}_py"));
        let script = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/py/to_csv.py");
        let out = cmd.arg(script).arg(&src).arg(&py_dir).output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let (mut rs, py) = (read_dir_sorted(&rust_dir), read_dir_sorted(&py_dir));
        // Python's Model init drops the `__active` helper that preprocessing left in the file.
        rs.retain(|(n, _)| !n.starts_with("inputs___"));
        let names = |v: &[(String, String)]| v.iter().map(|x| x.0.clone()).collect::<Vec<_>>();
        assert_eq!(names(&rs), names(&py), "{f}");
        for ((name, a), (_, b)) in rs.iter().zip(&py) {
            if name == "attrs.yaml" {
                // Python's Model re-orders the top-level keys and re-stamps
                // `runtime.timings.init_complete` when it loads the file.
                let norm = |t: &str| {
                    let mut tree = calliope_yaml::read_yaml_str(t, false).unwrap();
                    let _ = tree.del_key("runtime.timings.init_complete");
                    let mut parts: Vec<String> = tree
                        .iter()
                        .map(|(k, v)| {
                            let mut one = calliope_yaml::AttrTree::new();
                            one.insert(k.clone(), v.clone());
                            calliope_yaml::to_yaml(&one)
                        })
                        .collect();
                    parts.sort();
                    parts
                };
                assert_eq!(norm(a), norm(b), "{f}: attrs.yaml");
            } else {
                assert_eq!(a, b, "{f}: {name}");
            }
        }
    }
}
