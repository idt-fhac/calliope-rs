//! Phase 4 exit check: the problem the port builds equals Calliope's LP for every golden
//! fixture, after canonicalisation (`calliope_lp::lp_format`).
//!
//! - `golden/math_lp/*` (the 122 `tests/math` fixtures): compared by name with `pyomo.lp`.
//! - `golden/*` (37 example/test-model fixtures): compared by name with `pyomo.lp`, or by
//!   structure with `highs.lp` where there is no Pyomo file. Fixtures whose preprocessing
//!   is expected to fail have no LP and are skipped.
//!
//! The models are read from the Calliope reference checkout (`../calliope`, or
//! `CALLIOPE_REFERENCE`); without it the test is skipped. `GOLDEN_FILTER=<substring>` limits
//! the fixtures; `GOLDEN_VERBOSE=1` prints all differences; `GOLDEN_WRITE=<dir>` writes the
//! port's LP files there.

use std::path::Path;
use std::time::Instant;

use calliope_lp::lp_format::{CanonOptions, DiffOptions, canonicalise, diff_canonical, parse_lp};
use calliope_lp::{LpNames, write_lp_string};
use calliope_preprocess::read_yaml;

mod common;
use common::{fixture, fixture_dirs, read_meta, reference, repo};

/// Pyomo adds `c_e_ONE_VAR_CONSTANT: ONE_VAR_CONSTANT = 1` when no row has variables
/// (a CBC workaround), which is not part of the problem.
fn drop_dummy_row(mut raw: calliope_lp::lp_format::RawLp) -> calliope_lp::lp_format::RawLp {
    raw.rows.retain(|r| r.name != "c_e_ONE_VAR_CONSTANT");
    raw
}

struct Outcome {
    differences: Vec<String>,
    build_s: f64,
}

fn run(root: &Path, dir: &Path) -> Result<Option<Outcome>, String> {
    let tmp = tempfile::tempdir().unwrap();
    let Some((model, opts, objective)) = fixture(root, dir, tmp.path())? else {
        return Ok(None);
    };
    let meta = read_meta(dir);
    // `pyomo.lp` of the example fixtures is written after the solve: in operate mode that
    // is the last window, so the first window (what `build` makes) is in `highs.lp`. It is
    // missing where Pyomo's `to_lp` failed (bench/BUGS.md #14).
    let pyomo_ok = meta["errors"].get("pyomo_lp").is_none()
        && !(meta.get("fixture").is_some()
            && meta["fixture"]["name"]
                .as_str()
                .is_some_and(|n| n.contains("operate")));
    let reference = ["pyomo.lp", "highs.lp"]
        .iter()
        .filter(|f| pyomo_ok || **f != "pyomo.lp")
        .map(|f| dir.join(f))
        .find(|p| p.is_file())
        .ok_or("no LP file")?;
    let m = read_yaml(&model, &opts).map_err(|e| format!("preprocess: {e}"))?;
    let mut build = m.config.build.clone();
    if let Some(o) = objective {
        build.objective = o;
    }
    let t = Instant::now();
    let built = calliope_model::build_with(&m, &build).map_err(|e| format!("build: {e}"))?;
    let build_s = t.elapsed().as_secs_f64();
    let ours = write_lp_string(&built.lp, LpNames::Verbose);
    if let Ok(out) = std::env::var("GOLDEN_WRITE") {
        let name = dir.file_name().unwrap().to_string_lossy().to_string();
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(Path::new(&out).join(format!("{name}.lp")), &ours).unwrap();
    }
    let theirs = std::fs::read_to_string(&reference).unwrap();
    let a = canonicalise(
        &drop_dummy_row(parse_lp(&theirs).map_err(|e| e.0)?),
        CanonOptions::default(),
    );
    let b = canonicalise(&parse_lp(&ours).map_err(|e| e.0)?, CanonOptions::default());
    let d = diff_canonical(&a, &b, &DiffOptions::default());
    Ok(Some(Outcome {
        differences: d.differences,
        build_s,
    }))
}

#[test]
fn built_lp_matches_calliope() {
    let Some(root) = reference() else {
        eprintln!("skipping: no Calliope reference checkout");
        return;
    };
    let verbose = std::env::var("GOLDEN_VERBOSE").is_ok();
    let (mut passed, mut skipped) = (0, 0);
    let mut failed = Vec::new();
    for dir in fixture_dirs(true) {
        let name = dir
            .strip_prefix(repo().join("golden"))
            .unwrap()
            .display()
            .to_string();
        match run(&root, &dir) {
            Ok(None) => skipped += 1,
            Ok(Some(o)) if o.differences.is_empty() => {
                passed += 1;
                println!("ok    {name} ({:.3} s)", o.build_s);
            }
            Ok(Some(o)) => {
                println!("DIFF  {name}: {} differences", o.differences.len());
                for d in o
                    .differences
                    .iter()
                    .take(if verbose { usize::MAX } else { 6 })
                {
                    let d: String = d.chars().take(300).collect();
                    println!("        {d}");
                }
                failed.push(name);
            }
            Err(e) => {
                let e: String = e.chars().take(600).collect();
                println!("FAIL  {name}: {e}");
                failed.push(name);
            }
        }
    }
    println!(
        "{passed} equal, {} differ, {skipped} skipped (expected errors)",
        failed.len()
    );
    assert!(failed.is_empty(), "{} differ: {failed:?}", failed.len());
}
