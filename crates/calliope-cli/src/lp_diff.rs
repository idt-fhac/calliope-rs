//! `calliope lp-diff`: canonicalise and diff CPLEX LP files (Pyomo or HiGHS flavour).

use calliope_lp::lp_format::{CanonOptions, DiffOptions, canonicalise, diff_lp, parse_lp};

use crate::error::{CliError, CliResult};

/// Arguments of `lp-diff`.
#[derive(Debug, Clone)]
pub struct LpDiffArgs {
    /// Files: two to diff, one with `canon`.
    pub files: Vec<String>,
    /// Relative tolerance on coefficients.
    pub rtol: Option<f64>,
    /// Maximum differences to print.
    pub max: usize,
    /// Do not sort index names.
    pub no_sort_index: bool,
    /// Print the canonical form of one file.
    pub canon: bool,
}

fn read(p: &str) -> CliResult<String> {
    std::fs::read_to_string(p).map_err(|e| CliError::new(format!("{p}: {e}")))
}

/// Runs the command, printing to stdout. Returns true when the files are equal.
pub fn lp_diff(a: &LpDiffArgs) -> CliResult<bool> {
    let mut opts = DiffOptions::default();
    if let Some(r) = a.rtol {
        opts.rtol = r;
    }
    if a.no_sort_index {
        opts.canon = CanonOptions { sort_index: false };
    }
    if a.canon {
        let [f] = a.files.as_slice() else {
            return Err(CliError::new("usage: calliope lp-diff --canon <file.lp>"));
        };
        let c = canonicalise(
            &parse_lp(&read(f)?).map_err(|e| CliError::new(e.to_string()))?,
            opts.canon,
        );
        print!("{}", c.to_text());
        return Ok(true);
    }
    let [fa, fb] = a.files.as_slice() else {
        return Err(CliError::new("usage: calliope lp-diff <a.lp> <b.lp>"));
    };
    let d = diff_lp(&read(fa)?, &read(fb)?, &opts).map_err(|e| CliError::new(e.to_string()))?;
    for line in d.differences.iter().take(a.max) {
        println!("{line}");
    }
    if d.differences.len() > a.max {
        println!("... {} more", d.differences.len() - a.max);
    }
    println!(
        "lp-diff ({:?} comparison): {}",
        d.mode,
        if d.is_equal() {
            "equal".to_owned()
        } else {
            format!("{} difference(s)", d.differences.len())
        }
    );
    Ok(d.is_equal())
}
