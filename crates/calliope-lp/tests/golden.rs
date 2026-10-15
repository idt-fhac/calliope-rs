//! Golden-file tests: Pyomo vs HiGHS LP equivalence and round trips.

use calliope_lp::lp_format::{
    CanonOptions, DiffOptions, Mode, canonicalise, diff_canonical, diff_lp, parse_lp,
};
use std::fs;
use std::path::{Path, PathBuf};

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden")
}

/// All fixture directories (top level and `math_lp/*`).
fn fixture_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    for e in fs::read_dir(golden()).unwrap() {
        let p = e.unwrap().path();
        if !p.is_dir() {
            continue;
        }
        if p.file_name().unwrap() == "math_lp" {
            for m in fs::read_dir(&p).unwrap() {
                dirs.push(m.unwrap().path());
            }
        } else {
            dirs.push(p);
        }
    }
    dirs.sort();
    dirs
}

/// Fixtures whose model build failed (expected error) have empty or comment-only LP files.
fn has_model(p: &Path) -> bool {
    fs::read_to_string(p).unwrap().contains("\nmin")
        || fs::read_to_string(p).unwrap().contains("\nmax")
}

#[test]
fn pyomo_and_highs_lp_are_structurally_equal() {
    let opts = DiffOptions::default();
    let mut checked = 0;
    let mut failures = Vec::new();
    for d in fixture_dirs() {
        let (py, hi) = (d.join("pyomo.lp"), d.join("highs.lp"));
        // Operate-mode LP dumps cover different windows in the two flavours.
        if !py.exists()
            || !hi.exists()
            || !has_model(&py)
            || d.to_string_lossy().contains("operate")
        {
            continue;
        }
        let r = diff_lp(
            &fs::read_to_string(&py).unwrap(),
            &fs::read_to_string(&hi).unwrap(),
            &opts,
        )
        .unwrap_or_else(|e| panic!("{}: {e}", d.display()));
        assert_eq!(r.mode, Mode::Structural, "{}", d.display());
        checked += 1;
        if !r.is_equal() {
            let first: String = r.differences[0].chars().take(200).collect();
            failures.push(format!(
                "{} ({} diffs): {first}",
                d.display(),
                r.differences.len()
            ));
        }
    }
    assert!(checked >= 100, "only {checked} pairs found");
    assert!(
        failures.is_empty(),
        "{} pairs differ:\n{}",
        failures.len(),
        failures.join("\n")
    );
}

#[test]
fn every_golden_lp_round_trips() {
    let mut n = 0;
    for d in fixture_dirs() {
        for f in ["pyomo.lp", "highs.lp", "reference.lp"] {
            let p = d.join(f);
            if !p.exists() || !has_model(&p) {
                continue;
            }
            let canon = canonicalise(
                &parse_lp(&fs::read_to_string(&p).unwrap()).unwrap(),
                CanonOptions::default(),
            );
            let text = canon.to_lp();
            let again = canonicalise(&parse_lp(&text).unwrap(), CanonOptions::default());
            let r = diff_canonical(&canon, &again, &DiffOptions::default());
            assert!(
                r.is_equal(),
                "{}: {:?}",
                p.display(),
                &r.differences[..r.differences.len().min(5)]
            );
            if !canon.anonymous {
                assert_eq!(canon.vars, again.vars, "{}", p.display());
                assert_eq!(canon.rows.len(), again.rows.len(), "{}", p.display());
            }
            n += 1;
        }
    }
    assert!(n >= 122 * 2, "only {n} files");
}

#[test]
fn math_reference_equals_pyomo() {
    for d in fixture_dirs() {
        let (py, re) = (d.join("pyomo.lp"), d.join("reference.lp"));
        if !py.exists() || !re.exists() {
            continue;
        }
        let r = diff_lp(
            &fs::read_to_string(&py).unwrap(),
            &fs::read_to_string(&re).unwrap(),
            &DiffOptions::default(),
        )
        .unwrap();
        assert!(
            r.is_equal(),
            "{}: {:?}",
            d.display(),
            &r.differences[..r.differences.len().min(5)]
        );
    }
}

#[test]
fn pyomo_lp_equals_itself_and_perturbations_are_caught() {
    let text = fs::read_to_string(golden().join("national_scale/pyomo.lp")).unwrap();
    let opts = DiffOptions::default();
    assert!(diff_lp(&text, &text, &opts).unwrap().is_equal());

    let find = |pat: &str| text.lines().position(|l| l.starts_with(pat)).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let edit = |f: &dyn Fn(&mut Vec<String>)| {
        let mut v: Vec<String> = lines.iter().map(|s| s.to_string()).collect();
        f(&mut v);
        v.join("\n")
    };
    let row = find("c_u_constraints(");
    // coefficient perturbation (1e-6 relative)
    let coef = edit(&|v| {
        let l = &v[row + 1];
        let (c, rest) = l.split_once(' ').unwrap();
        let x: f64 = c.parse().unwrap();
        v[row + 1] = format!("{:+} {rest}", x * (1.0 + 1e-6));
    });
    assert!(!diff_lp(&text, &coef, &opts).unwrap().is_equal());
    // noise below tolerance is accepted
    let noise = edit(&|v| {
        let l = &v[row + 1];
        let (c, rest) = l.split_once(' ').unwrap();
        let x: f64 = c.parse().unwrap();
        v[row + 1] = format!("{:+} {rest}", x * (1.0 + 1e-12));
    });
    assert!(diff_lp(&text, &noise, &opts).unwrap().is_equal());
    // sense
    let sense = edit(&|v| {
        let j = v[row..].iter().position(|l| l.starts_with("<=")).unwrap() + row;
        v[j] = v[j].replacen("<=", ">=", 1);
    });
    assert!(!diff_lp(&text, &sense, &opts).unwrap().is_equal());
    // missing row
    let missing = edit(&|v| {
        let end = v[row + 1..].iter().position(|l| l.is_empty()).unwrap() + row + 1;
        v.drain(row..end);
    });
    assert!(!diff_lp(&text, &missing, &opts).unwrap().is_equal());
    // renamed index
    let renamed = edit(&|v| v[row] = v[row].replacen("region1", "region9", 1));
    assert!(!diff_lp(&text, &renamed, &opts).unwrap().is_equal());
}
