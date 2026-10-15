//! NetCDF comparison.
//!
//! NetCDF files are read through `oracle/nc_dump.py` (Python, in the Calliope pixi env), which
//! writes a canonical JSON form (sorted dimensions and coordinates, NaN as `null`, infinities as
//! strings). This module compares two such dumps. A native `netcdf` crate would need libnetcdf
//! and libhdf5 at link and run time; the dump keeps the Rust build free of C dependencies.

use serde_json::Value;
use std::path::Path;
use std::process::Command;

/// Tolerances for numeric comparison: `|a - b| <= tol * max(1, |a|, |b|)`.
#[derive(Debug, Clone, Copy)]
pub struct Tolerance(pub f64);

/// Inputs are copied data; they must agree (almost) bit for bit.
pub const INPUT_TOL: Tolerance = Tolerance(1e-9);
/// Solver results: PLAN.md "absolute 1e-6, scaled by magnitude".
pub const RESULT_TOL: Tolerance = Tolerance(1e-6);

/// Dump a NetCDF file to canonical JSON using `python` (an interpreter with xarray, netCDF4, yaml).
pub fn dump(python: &Path, script: &Path, nc: &Path) -> Result<Value, String> {
    let out = Command::new(python)
        .arg("-I")
        .arg(script)
        .arg(nc)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", python.display()))?;
    if !out.status.success() {
        return Err(format!(
            "nc_dump failed for {}: {}",
            nc.display(),
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    serde_json::from_slice(&out.stdout).map_err(|e| format!("nc_dump output: {e}"))
}

fn num(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => match s.as_str() {
            "inf" => Some(f64::INFINITY),
            "-inf" => Some(f64::NEG_INFINITY),
            _ => None,
        },
        _ => None,
    }
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    a == b || (a - b).abs() <= tol * 1f64.max(a.abs()).max(b.abs())
}

/// Structural JSON equality with numeric tolerance; `path` is used in the message.
fn value_diff(path: &str, a: &Value, b: &Value, tol: f64, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            for (k, v) in x {
                match y.get(k) {
                    Some(w) => value_diff(&format!("{path}.{k}"), v, w, tol, out),
                    None => out.push(format!("{path}.{k}: only in expected")),
                }
            }
            for k in y.keys().filter(|k| !x.contains_key(*k)) {
                out.push(format!("{path}.{k}: only in actual"));
            }
        }
        (Value::Array(x), Value::Array(y)) => {
            if x.len() != y.len() {
                out.push(format!("{path}: length {} vs {}", x.len(), y.len()));
            } else {
                for (i, (v, w)) in x.iter().zip(y).enumerate() {
                    value_diff(&format!("{path}[{i}]"), v, w, tol, out);
                }
            }
        }
        _ => {
            let same = match (num(a), num(b)) {
                (Some(x), Some(y)) => close(x, y, tol),
                _ => a == b,
            };
            if !same {
                out.push(format!("{path}: {a} vs {b}"));
            }
        }
    }
}

/// Compare the data of one array (flat lists); reports the count and worst element.
fn data_diff(path: &str, a: &Value, b: &Value, tol: f64, out: &mut Vec<String>) {
    let (Some(x), Some(y)) = (a.as_array(), b.as_array()) else {
        out.push(format!("{path}: data is not a list"));
        return;
    };
    if x.len() != y.len() {
        out.push(format!("{path}: {} vs {} elements", x.len(), y.len()));
        return;
    }
    let mut bad = 0usize;
    let mut first: Option<(usize, String)> = None;
    let mut worst = 0.0f64;
    for (i, (v, w)) in x.iter().zip(y).enumerate() {
        let same = match (num(v), num(w)) {
            (Some(p), Some(q)) => {
                let d = (p - q).abs() / 1f64.max(p.abs()).max(q.abs());
                if d.is_finite() && d > worst {
                    worst = d;
                }
                close(p, q, tol)
            }
            _ => v == w,
        };
        if !same {
            bad += 1;
            first.get_or_insert((i, format!("{v} vs {w}")));
        }
    }
    if let Some((i, what)) = first {
        out.push(format!(
            "{path}: {bad}/{} elements differ (first at flat index {i}: {what}; worst scaled diff {worst:.3e})",
            x.len()
        ));
    }
}

fn group_diff(name: &str, a: &Value, b: &Value, tol: f64, out: &mut Vec<String>) {
    value_diff(&format!("{name}.dims"), &a["dims"], &b["dims"], tol, out);
    value_diff(&format!("{name}.attrs"), &a["attrs"], &b["attrs"], tol, out);
    let (ca, cb) = (a["coords"].as_object(), b["coords"].as_object());
    if let (Some(ca), Some(cb)) = (ca, cb) {
        for (k, v) in ca {
            let p = format!("{name}.coords.{k}");
            match cb.get(k) {
                Some(w) => {
                    if v["dims"] != w["dims"] {
                        out.push(format!("{p}: dims {} vs {}", v["dims"], w["dims"]));
                    }
                    data_diff(&p, &v["data"], &w["data"], 0.0, out);
                }
                None => out.push(format!("{p}: missing in actual")),
            }
        }
        for k in cb.keys().filter(|k| !ca.contains_key(*k)) {
            out.push(format!("{name}.coords.{k}: unexpected in actual"));
        }
    }
    let (va, vb) = (a["vars"].as_object(), b["vars"].as_object());
    if let (Some(va), Some(vb)) = (va, vb) {
        for (k, v) in va {
            let p = format!("{name}.vars.{k}");
            let Some(w) = vb.get(k) else {
                out.push(format!("{p}: missing in actual"));
                continue;
            };
            if v["dims"] != w["dims"] || v["shape"] != w["shape"] {
                out.push(format!(
                    "{p}: dims/shape {} {} vs {} {}",
                    v["dims"], v["shape"], w["dims"], w["shape"]
                ));
                continue;
            }
            if v["kind"] != w["kind"] {
                out.push(format!("{p}: dtype kind {} vs {}", v["kind"], w["kind"]));
            }
            value_diff(&format!("{p}.attrs"), &v["attrs"], &w["attrs"], tol, out);
            data_diff(&p, &v["data"], &w["data"], tol, out);
        }
        for k in vb.keys().filter(|k| !va.contains_key(*k)) {
            out.push(format!("{name}.vars.{k}: unexpected in actual"));
        }
    }
}

/// Which groups of a dump to compare.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// The `inputs` group plus the `attrs` group (config, definition, math).
    Inputs,
    /// The `results` group.
    Results,
}

/// Compare two dumps (`expected` first). Returns a list of differences.
pub fn diff_dumps(expected: &Value, actual: &Value, part: Part, tol: Tolerance) -> Vec<String> {
    let mut out = Vec::new();
    match part {
        Part::Inputs => {
            value_diff(
                "attrs",
                &expected["attrs"],
                &actual["attrs"],
                tol.0,
                &mut out,
            );
            let g = |v: &'_ Value| v["groups"]["inputs"].clone();
            let (a, b) = (g(expected), g(actual));
            if a.is_null() || b.is_null() {
                if a != b {
                    out.push("group inputs missing on one side".into());
                }
            } else {
                group_diff("inputs", &a, &b, tol.0, &mut out);
            }
        }
        Part::Results => {
            let g = |v: &'_ Value| v["groups"]["results"].clone();
            let (a, b) = (g(expected), g(actual));
            if a.is_null() || b.is_null() {
                if a != b {
                    out.push("group results missing on one side".into());
                }
            } else {
                group_diff("results", &a, &b, tol.0, &mut out);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn dump() -> Value {
        json!({
            "attrs": {"config": {"a": 1.0}},
            "groups": {"inputs": {
                "dims": {"x": 2}, "attrs": {},
                "coords": {"x": {"dims": ["x"], "kind": "str", "data": ["a", "b"]}},
                "vars": {"v": {"dims": ["x"], "shape": [2], "kind": "float", "attrs": {},
                               "data": [1.0, null]}},
            }}
        })
    }

    #[test]
    fn equal_and_perturbed() {
        let a = dump();
        assert!(diff_dumps(&a, &a, Part::Inputs, INPUT_TOL).is_empty());
        let mut b = dump();
        b["groups"]["inputs"]["vars"]["v"]["data"][0] = json!(1.0 + 1e-12);
        assert!(diff_dumps(&a, &b, Part::Inputs, INPUT_TOL).is_empty());
        b["groups"]["inputs"]["vars"]["v"]["data"][0] = json!(1.001);
        let d = diff_dumps(&a, &b, Part::Inputs, INPUT_TOL);
        assert_eq!(d.len(), 1, "{d:?}");
        assert!(diff_dumps(&a, &b, Part::Inputs, Tolerance(1e-2)).is_empty());
        let mut c = dump();
        c["groups"]["inputs"]["vars"]["v"]["data"][1] = json!(0.0);
        assert!(!diff_dumps(&a, &c, Part::Inputs, INPUT_TOL).is_empty());
        let mut e = dump();
        e["groups"]["inputs"]["coords"]["x"]["data"][0] = json!("z");
        assert!(!diff_dumps(&a, &e, Part::Inputs, INPUT_TOL).is_empty());
        let mut f = dump();
        f["attrs"]["config"]["a"] = json!(2.0);
        assert!(!diff_dumps(&a, &f, Part::Inputs, INPUT_TOL).is_empty());
    }
}
