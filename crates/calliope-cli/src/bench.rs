//! `calliope bench`: wall time and peak memory of each stage of a run (preprocess, build,
//! solve incl. postprocessing, write), in the layout of the oracle's `timings.stages`.
//!
//! Peak memory is the process' resident-set high-water mark (`VmHWM`), reset before each
//! stage through `/proc/self/clear_refs`, so each stage reports its own peak. Off Linux the
//! memory columns are empty.

use std::fmt::Write as _;
use std::time::Instant;

use calliope_model::Model;

use crate::error::CliResult;
use crate::facade::{self, LoadOptions, SolveOptions};

/// One measured stage.
#[derive(Debug, Clone)]
pub struct Stage {
    /// `preprocess`, `build`, `solve` or `write`.
    pub name: &'static str,
    /// Wall time in seconds.
    pub wall_s: f64,
    /// Resident set size before the stage, in MB.
    pub start_rss_mb: Option<f64>,
    /// Peak resident set size during the stage, in MB.
    pub peak_rss_mb: Option<f64>,
}

fn status_mb(key: &str) -> Option<f64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let line = s.lines().find(|l| l.starts_with(key))?;
    let kb: f64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb / 1024.0)
}

fn reset_peak() {
    // Linux >= 4.0: "5" resets the peak RSS of the process.
    let _ = std::fs::write("/proc/self/clear_refs", "5");
}

fn measure<T>(
    stages: &mut Vec<Stage>,
    name: &'static str,
    f: impl FnOnce() -> CliResult<T>,
) -> CliResult<T> {
    reset_peak();
    let start_rss_mb = status_mb("VmRSS:");
    let t = Instant::now();
    let out = f()?;
    stages.push(Stage {
        name,
        wall_s: t.elapsed().as_secs_f64(),
        start_rss_mb,
        peak_rss_mb: status_mb("VmHWM:"),
    });
    Ok(out)
}

/// Runs the model once, stage by stage. The NetCDF of the write stage goes to a temporary
/// file that is removed afterwards.
pub fn bench(load: &LoadOptions) -> CliResult<(Model, Vec<Stage>)> {
    let mut stages = Vec::new();
    let mut model = measure(&mut stages, "preprocess", || facade::load_model(load))?;
    measure(&mut stages, "build", || facade::build(&mut model))?;
    measure(&mut stages, "solve", || {
        facade::solve(&mut model, &SolveOptions::default())
    })?;
    let path = std::env::temp_dir().join(format!("calliope-bench-{}.nc", std::process::id()));
    let written = measure(&mut stages, "write", || facade::to_netcdf(&model, &path));
    let _ = std::fs::remove_file(&path);
    written?;
    Ok((model, stages))
}

fn mb(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_owned(), |v| format!("{v:.1}"))
}

/// The stages as a table, with the total.
pub fn table(stages: &[Stage]) -> String {
    let total: f64 = stages.iter().map(|s| s.wall_s).sum();
    let mut out = String::new();
    let _ = writeln!(
        out,
        "{:<12} {:>10} {:>7} {:>14} {:>14}",
        "stage", "wall_s", "share", "start_rss_mb", "peak_rss_mb"
    );
    for s in stages {
        let share = if total > 0.0 { s.wall_s / total } else { 0.0 };
        let _ = writeln!(
            out,
            "{:<12} {:>10.3} {:>6.1}% {:>14} {:>14}",
            s.name,
            s.wall_s,
            share * 100.0,
            mb(s.start_rss_mb),
            mb(s.peak_rss_mb)
        );
    }
    let peak = stages
        .iter()
        .filter_map(|s| s.peak_rss_mb)
        .fold(None, |m: Option<f64>, v| Some(m.map_or(v, |m| m.max(v))));
    let _ = writeln!(
        out,
        "{:<12} {total:>10.3} {:>7} {:>14} {:>14}",
        "total",
        "",
        "",
        mb(peak)
    );
    out
}

/// The stages as JSON (`{"stages": {name: {...}}, "total_wall_s": ..}`).
pub fn json(stages: &[Stage]) -> String {
    let num = |v: Option<f64>| v.map_or_else(|| "null".to_owned(), |v| format!("{v}"));
    let total: f64 = stages.iter().map(|s| s.wall_s).sum();
    let items: Vec<String> = stages
        .iter()
        .map(|s| {
            let share = if total > 0.0 { s.wall_s / total } else { 0.0 };
            format!(
                "\"{}\": {{\"wall_s\": {}, \"start_rss_mb\": {}, \"peak_rss_mb\": {}, \"share\": {share}}}",
                s.name,
                s.wall_s,
                num(s.start_rss_mb),
                num(s.peak_rss_mb)
            )
        })
        .collect();
    format!(
        "{{\"stages\": {{{}}}, \"total_wall_s\": {total}}}\n",
        items.join(", ")
    )
}
