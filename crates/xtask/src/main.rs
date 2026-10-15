//! Developer tasks: `cargo run -p xtask -- <task>`.
//!
//! - `schema-diff [--update]`: compares the `schemars` JSON schema of `CalliopeAttrs` with the
//!   pydantic schema produced by the Calliope pixi env (`$CALLIOPE_REFERENCE`, default
//!   `../calliope`). `--update` rewrites the fixture used by the `calliope-schema` tests.

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

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("schema-diff") => schema_diff(args.iter().any(|a| a == "--update")),
        other => Err(format!(
            "unknown task {other:?}; available: schema-diff [--update]"
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
