//! Developer tasks: `cargo run -p xtask -- <task>`.
//!
//! - `lp-diff [--rtol X] [--max N] <a.lp> <b.lp>` / `lp-diff --canon <file.lp>`: canonicalise and
//!   diff CPLEX LP files (Pyomo or HiGHS flavour).
//! - `parity [--fixture NAME]... [--self-check] [--strict]`: compare calliope-rs against the golden
//!   files (inputs NetCDF, LP, results). `--self-check` feeds the golden files back as the output.
//! - `schema-diff [--update]`: compares the `schemars` JSON schema of `CalliopeAttrs` with the
//!   pydantic schema produced by the Calliope pixi env (`$CALLIOPE_REFERENCE`, default
//!   `../calliope`). `--update` rewrites the fixture used by the `calliope-schema` tests.

mod known;
mod ncdiff;
mod parity;
mod runner;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn reference() -> PathBuf {
    std::env::var("CALLIOPE_REFERENCE")
        .map_or_else(|_| workspace_root().join("../calliope"), PathBuf::from)
}

fn pydantic_schema() -> Result<String, String> {
    let python = reference().join(".pixi/envs/dev/bin/python");
    let script = workspace_root().join("crates/xtask/scripts/pydantic_schema.py");
    let out = Command::new(&python)
        .arg("-I")
        .arg(&script)
        .output()
        .map_err(|e| format!("cannot run {}: {e}", python.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).into_owned());
    }
    String::from_utf8(out.stdout).map_err(|e| e.to_string())
}

fn schema_diff(update: bool) -> Result<bool, String> {
    let text = pydantic_schema()?;
    let pydantic: serde_json::Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let ours = calliope_schema::json_schema::<calliope_schema::CalliopeAttrs>();
    let diffs = calliope_schema::schema_diff::diff(&pydantic, &ours);
    if update {
        let fixture = workspace_root()
            .join("crates/calliope-schema/tests/fixtures/pydantic_calliope_attrs.json");
        std::fs::write(&fixture, text).map_err(|e| e.to_string())?;
        println!("wrote {}", fixture.display());
    }
    for d in &diffs {
        println!("{d}");
    }
    println!(
        "schema-diff: {} difference(s) (left: pydantic, right: Rust)",
        diffs.len()
    );
    Ok(diffs.is_empty())
}

fn lp_diff(args: &[String]) -> Result<bool, String> {
    use calliope_lp::lp_format::{CanonOptions, DiffOptions, canonicalise, diff_lp, parse_lp};
    let mut opts = DiffOptions::default();
    let mut max = 50usize;
    let mut canon = false;
    let mut files = Vec::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--rtol" => {
                let v = it.next().ok_or("--rtol needs a value")?;
                opts.rtol = v.parse().map_err(|e| format!("--rtol: {e}"))?;
            }
            "--max" => {
                let v = it.next().ok_or("--max needs a value")?;
                max = v.parse().map_err(|e| format!("--max: {e}"))?;
            }
            "--no-sort-index" => opts.canon = CanonOptions { sort_index: false },
            "--canon" => canon = true,
            f => files.push(f.to_string()),
        }
    }
    let read = |p: &str| std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}"));
    if canon {
        let [f] = files.as_slice() else {
            return Err("usage: lp-diff --canon <file.lp>".into());
        };
        let c = canonicalise(&parse_lp(&read(f)?).map_err(|e| e.to_string())?, opts.canon);
        print!("{}", c.to_text());
        return Ok(true);
    }
    let [fa, fb] = files.as_slice() else {
        return Err(
            "usage: lp-diff [--rtol X] [--max N] [--no-sort-index] <a.lp> <b.lp> | --canon <file.lp>"
                .into(),
        );
    };
    let d = diff_lp(&read(fa)?, &read(fb)?, &opts).map_err(|e| e.to_string())?;
    for line in d.differences.iter().take(max) {
        println!("{line}");
    }
    if d.differences.len() > max {
        println!("... {} more", d.differences.len() - max);
    }
    println!(
        "lp-diff ({:?} comparison): {}",
        d.mode,
        if d.is_equal() {
            "equal".to_string()
        } else {
            format!("{} difference(s)", d.differences.len())
        }
    );
    Ok(d.is_equal())
}

fn parity_task(args: &[String]) -> Result<bool, String> {
    use parity::{Comparer, Env, GoldenRunner, Runner};
    let mut filters = Vec::new();
    let (mut self_check, mut strict) = (false, false);
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--fixture" => filters.push(it.next().ok_or("--fixture needs a name")?.clone()),
            "--self-check" => self_check = true,
            "--strict" => strict = true,
            other => return Err(format!("parity: unknown argument {other}")),
        }
    }
    let root = workspace_root();
    let env = Env {
        python: reference().join(".pixi/envs/dev/bin/python"),
        script: root.join("oracle/nc_dump.py"),
    };
    let mut fixtures = parity::load_manifest(&root.join("golden"))?;
    if !filters.is_empty() {
        fixtures.retain(|f| filters.contains(&f.name));
    }
    let runner: Box<dyn Runner> = if self_check {
        Box::new(GoldenRunner)
    } else {
        Box::new(runner::RustRunner {
            reference: reference(),
        })
    };
    let work = root.join("target/parity");
    let mut cmp = Comparer::new(&env);
    let reports: Vec<_> = fixtures
        .iter()
        .map(|f| cmp.compare(f, runner.as_ref(), &work))
        .collect();
    print!("{}", parity::render(&reports, runner.name()));
    let t = parity::tally(&reports);
    println!(
        "\nparity: {} passed, {} classified, {} failed, {} not run",
        t.pass, t.classified, t.fail, t.not_run
    );
    Ok(t.fail == 0 && (!strict || t.not_run == 0))
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("schema-diff") => schema_diff(args.iter().any(|a| a == "--update")),
        Some("parity") => parity_task(&args[1..]),
        Some("lp-diff") => lp_diff(&args[1..]),
        other => Err(format!(
            "unknown task {other:?}; available: schema-diff [--update], lp-diff, parity"
        )),
    };
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("xtask: {e}");
            ExitCode::from(2)
        }
    }
}
