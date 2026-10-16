//! Argument parsing and command dispatch.

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use clap::{Args, Parser, Subcommand};

use crate::check::{CheckKind, check_file, render};
use crate::error::{CliError, CliResult};
use crate::facade::{LoadOptions, SolveOptions, load_model};
use crate::generate::{Kind, RunsArgs, generate, scenarios_yaml};
use crate::lp_diff::{LpDiffArgs, lp_diff};
use crate::new_cmd::new_model;

const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Calliope: a multi-scale energy systems modelling framework.
#[derive(Debug, Parser)]
#[command(name = "calliope", disable_version_flag = true)]
pub struct Cli {
    /// Display version.
    #[arg(long)]
    version: bool,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Args, Clone)]
struct Common {
    /// Print debug information when encountering errors.
    #[arg(long)]
    debug: bool,
    /// Be less verbose about what is happening, including hiding solver output.
    #[arg(long)]
    quiet: bool,
    /// Accepted for compatibility; Python-specific, ignored.
    #[arg(long)]
    pdb: bool,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create a new model based on a built-in example.
    New {
        path: PathBuf,
        #[arg(long)]
        template: Option<String>,
        /// Print debug information when encountering errors.
        #[arg(long)]
        debug: bool,
    },
    /// Build and solve a model.
    Run(Box<RunArgs>),
    /// Generate a script to run multiple models.
    #[command(name = "generate_runs")]
    GenerateRuns {
        model_file: String,
        out_file: String,
        /// One of: "bash", "bsub", "sbatch", or "windows".
        #[arg(long)]
        kind: Option<String>,
        #[arg(long)]
        scenarios: Option<String>,
        #[arg(long = "cluster_threads", default_value_t = 1)]
        cluster_threads: i64,
        #[arg(long = "cluster_mem")]
        cluster_mem: Option<String>,
        #[arg(long = "cluster_time")]
        cluster_time: Option<String>,
        /// Any additional arguments to pass directly on to `calliope run`.
        #[arg(
            long = "additional_args",
            default_value = "",
            allow_hyphen_values = true
        )]
        additional_args: String,
        #[arg(long = "override_dict")]
        override_dict: Option<String>,
        #[command(flatten)]
        common: Common,
    },
    /// Generate scenario definitions from given combinations of overrides.
    #[command(name = "generate_scenarios")]
    GenerateScenarios {
        model_file: String,
        out_file: String,
        overrides: Vec<String>,
        #[arg(long = "scenario_name_prefix")]
        scenario_name_prefix: Option<String>,
        #[command(flatten)]
        common: Common,
    },
    /// Validate model definitions and math files (schema, parser, static typecheck).
    Check {
        #[arg(required = true)]
        files: Vec<PathBuf>,
        /// `auto` (default), `model` or `math`.
        #[arg(long, default_value = "auto")]
        kind: String,
        #[arg(long)]
        scenario: Option<String>,
        #[arg(long = "override_dict")]
        override_dict: Option<String>,
        /// Exit with an error on warnings too.
        #[arg(long)]
        deny_warnings: bool,
    },
    /// Canonicalise and diff CPLEX LP files.
    #[command(name = "lp-diff")]
    LpDiff {
        files: Vec<String>,
        #[arg(long)]
        rtol: Option<f64>,
        #[arg(long, default_value_t = 50)]
        max: usize,
        #[arg(long)]
        no_sort_index: bool,
        /// Print the canonical form of one file.
        #[arg(long)]
        canon: bool,
    },
}

#[derive(Debug, Args)]
struct RunArgs {
    model_file: String,
    #[arg(long)]
    scenario: Option<String>,
    #[arg(long = "model_format")]
    model_format: Option<String>,
    #[arg(long = "override_dict")]
    override_dict: Option<String>,
    #[arg(long = "save_netcdf")]
    save_netcdf: Option<String>,
    #[arg(long = "save_csv")]
    save_csv: Option<String>,
    #[arg(long = "save_logs")]
    save_logs: Option<String>,
    /// Build and save model to the given LP file. When this is set, the model is not sent
    /// to a solver, and all other save options are ignored.
    #[arg(long = "save_lp")]
    save_lp: Option<String>,
    #[command(flatten)]
    common: Common,
    /// Accepted for compatibility; Python-specific, ignored.
    #[arg(long)]
    profile: bool,
    /// Accepted for compatibility; Python-specific, ignored.
    #[arg(long = "profile_filename")]
    profile_filename: Option<String>,
    /// Return fail on command line when problem is infeasible (default).
    #[arg(
        long = "fail_when_infeasible",
        overrides_with = "no_fail_when_infeasible"
    )]
    fail_when_infeasible: bool,
    #[arg(
        long = "no_fail_when_infeasible",
        overrides_with = "fail_when_infeasible"
    )]
    no_fail_when_infeasible: bool,
}

fn init_logging(debug: bool, quiet: bool) {
    let level = if debug {
        tracing::Level::DEBUG
    } else if quiet {
        tracing::Level::WARN
    } else {
        tracing::Level::INFO
    };
    let _ = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .with_ansi(std::io::IsTerminal::is_terminal(&std::io::stderr()))
        .without_time()
        .with_target(false)
        .try_init();
}

fn timestamp() -> String {
    // UTC, "%Y-%m-%d %H:%M:%S".
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let (days, rem) = (secs / 86_400, secs % 86_400);
    // Civil-from-days (Howard Hinnant).
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem / 60) % 60,
        rem % 60
    )
}

fn print_end_time(start: Instant, msg: &str) {
    let secs = (start.elapsed().as_secs_f64() * 10.0).round() / 10.0;
    println!(
        "\nCalliope run {msg}. Elapsed: {secs} seconds (time at exit: {})",
        timestamp()
    );
}

/// Prints the error as the Python CLI does (stdout, `--debug` adds the causes on stderr).
fn report(e: &CliError, debug: bool, start: Option<Instant>) {
    if debug {
        eprintln!("Error (most recent cause last):");
        for c in e.causes.iter().rev() {
            eprintln!("  caused by: {c}");
        }
        eprintln!("  {}", e.message);
    } else {
        println!("\nError: {}", e.message);
        if let Some(s) = start {
            print_end_time(s, "aborted due to an error");
        }
    }
}

fn warn_python_only(c: &Common, profile: bool) {
    if c.pdb {
        tracing::warn!("--pdb is Python-specific and is ignored");
    }
    if profile {
        tracing::warn!("--profile is Python-specific and is ignored");
    }
}

fn run(a: &RunArgs) -> CliResult<()> {
    let start = Instant::now();
    println!("Calliope {VERSION} starting at {}\n", timestamp());
    warn_python_only(&a.common, a.profile || a.profile_filename.is_some());
    let result = (|| {
        let mut model = load_model(&LoadOptions {
            model_file: a.model_file.clone(),
            scenario: a.scenario.clone(),
            model_format: a.model_format.clone(),
            override_dict: a.override_dict.clone(),
        })?;
        println!("{}\n", model.info());
        if let Some(lp) = &a.save_lp {
            println!("Saving model to LP file...");
            if a.save_csv.is_some() || a.save_netcdf.is_some() {
                println!("WARNING: Model will not be solved - ignoring other save options!");
            }
            model.build()?;
            model.to_lp(Path::new(lp))?;
            print_end_time(start, "complete");
            return Ok(());
        }
        println!("Starting model run...");
        if a.save_csv.is_none() && a.save_netcdf.is_none() {
            println!(
                "\n!!!\nWARNING: No options to save results have been specified.\n\
                 Model will run without saving results!\n!!!\n"
            );
        }
        // Unlike Python (bug03: both options are silently dropped), `--save_logs` and the
        // per-SPORE redirect reach the solver.
        let mut opts = SolveOptions {
            save_logs: a.save_logs.as_ref().map(PathBuf::from),
            ..SolveOptions::default()
        };
        if let (Some(nc), Some(_)) = (&a.save_netcdf, model.spores_save_path()) {
            opts.save_per_spore_path = Some(
                Path::new(nc)
                    .parent()
                    .map_or_else(PathBuf::new, Path::to_path_buf),
            );
        }
        model.build()?;
        let termination = model.solve(&opts)?;
        if let Some(d) = &a.save_csv {
            println!("Saving CSV results to directory: {d}");
            model.to_csv(Path::new(d))?;
        }
        if let Some(f) = &a.save_netcdf {
            println!("Saving NetCDF results to file: {f}");
            model.to_netcdf(Path::new(f))?;
        }
        print_end_time(start, "complete");
        if !a.no_fail_when_infeasible && termination != "optimal" {
            return Err(CliError::new("Problem is infeasible."));
        }
        Ok(())
    })();
    result.inspect_err(|e| report(e, a.common.debug, Some(start)))
}

fn check(
    files: &[PathBuf],
    kind: &str,
    scenario: Option<&str>,
    ov: Option<&str>,
    deny: bool,
) -> bool {
    let kind = match kind {
        "auto" => CheckKind::Auto,
        "model" => CheckKind::Model,
        "math" => CheckKind::Math,
        other => {
            println!("Error: unknown --kind '{other}' (auto, model, math)");
            return false;
        }
    };
    let mut ok = true;
    for f in files {
        match check_file(f, kind, scenario, ov) {
            Err(e) => {
                ok = false;
                println!("FAIL {}: {}", f.display(), e.message);
            }
            Ok(diags) => {
                let errors = diags.iter().filter(|d| d.is_error()).count();
                let warnings = diags.len() - errors;
                for d in &diags {
                    println!("{}", render(d));
                }
                let bad = errors > 0 || (deny && warnings > 0);
                ok &= !bad;
                println!(
                    "{} {}: {errors} error(s), {warnings} warning(s)",
                    if bad { "FAIL" } else { "ok" },
                    f.display()
                );
            }
        }
    }
    ok
}

/// Entry point of the binary.
pub fn main() -> ExitCode {
    let cli = Cli::parse();
    let code = |ok: bool| {
        if ok {
            ExitCode::SUCCESS
        } else {
            ExitCode::from(1)
        }
    };
    let Some(cmd) = cli.command else {
        if cli.version {
            println!("Version {VERSION}");
        } else {
            use clap::CommandFactory;
            let _ = Cli::command().print_help();
        }
        return ExitCode::SUCCESS;
    };
    if cli.version {
        println!("Version {VERSION}");
    }
    match cmd {
        Command::New {
            path,
            template,
            debug,
        } => {
            init_logging(debug, false);
            let template = template.unwrap_or_else(|| "national_scale".to_owned());
            println!(
                "Copying {template} template to target directory: {}",
                path.display()
            );
            code(
                new_model(&template, &path)
                    .inspect_err(|e| report(e, debug, None))
                    .is_ok(),
            )
        }
        Command::Run(a) => {
            init_logging(a.common.debug, a.common.quiet);
            code(run(&a).is_ok())
        }
        Command::GenerateRuns {
            model_file,
            out_file,
            kind,
            scenarios,
            cluster_threads,
            cluster_mem,
            cluster_time,
            additional_args,
            override_dict,
            common,
        } => {
            init_logging(common.debug, common.quiet);
            let r = Kind::parse(kind.as_deref()).and_then(|k| {
                generate(
                    k,
                    &RunsArgs {
                        model_file,
                        out_file,
                        scenarios,
                        additional_args,
                        override_dict,
                        cluster_mem,
                        cluster_time,
                        cluster_threads,
                    },
                )
            });
            code(r.inspect_err(|e| report(e, common.debug, None)).is_ok())
        }
        Command::GenerateScenarios {
            out_file,
            overrides,
            scenario_name_prefix,
            common,
            ..
        } => {
            init_logging(common.debug, common.quiet);
            let r = scenarios_yaml(&overrides, scenario_name_prefix.as_deref())
                .and_then(|t| std::fs::write(&out_file, t).map_err(Into::into));
            code(r.inspect_err(|e| report(e, common.debug, None)).is_ok())
        }
        Command::Check {
            files,
            kind,
            scenario,
            override_dict,
            deny_warnings,
        } => {
            init_logging(false, true);
            code(check(
                &files,
                &kind,
                scenario.as_deref(),
                override_dict.as_deref(),
                deny_warnings,
            ))
        }
        Command::LpDiff {
            files,
            rtol,
            max,
            no_sort_index,
            canon,
        } => match lp_diff(&LpDiffArgs {
            files,
            rtol,
            max,
            no_sort_index,
            canon,
        }) {
            Ok(eq) => code(eq),
            Err(e) => {
                println!("Error: {}", e.message);
                ExitCode::from(1)
            }
        },
    }
}
