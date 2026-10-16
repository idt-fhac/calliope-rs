//! `generate_runs` and `generate_scenarios` (`calliope.util.generate_runs`, `cli.py`).
//!
//! The scripts are byte-identical to Python's for the same arguments.

use std::path::Path;

use calliope_yaml::{AttrTree, Node, Value, read_yaml_file, read_yaml_str, to_yaml};

use crate::error::{CliError, CliResult};

/// The script kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `bash`.
    Bash,
    /// `bsub` (IBM Spectrum LSF).
    Bsub,
    /// `sbatch` (SLURM).
    Sbatch,
    /// `windows`.
    Windows,
}

impl Kind {
    /// Parses `--kind`; `None` and unknown names are errors (Python raises `KeyError`).
    pub fn parse(kind: Option<&str>) -> CliResult<Self> {
        match kind {
            Some("bash") => Ok(Self::Bash),
            Some("bsub") => Ok(Self::Bsub),
            Some("sbatch") => Ok(Self::Sbatch),
            Some("windows") => Ok(Self::Windows),
            Some(other) => Err(CliError::new(format!("'{other}'"))),
            None => Err(CliError::new("None")),
        }
    }
}

/// The arguments of `generate_runs`.
#[derive(Debug, Clone, Default)]
pub struct RunsArgs {
    /// Model file.
    pub model_file: String,
    /// Output script.
    pub out_file: String,
    /// `;`-separated scenarios.
    pub scenarios: Option<String>,
    /// Extra arguments for `calliope run`.
    pub additional_args: String,
    /// Override dict (YAML text or file).
    pub override_dict: Option<String>,
    /// Cluster memory.
    pub cluster_mem: Option<String>,
    /// Cluster time.
    pub cluster_time: Option<String>,
    /// Cluster threads.
    pub cluster_threads: i64,
}

/// `read_rich_yaml(string | path)`: a path if it names a file, YAML text otherwise.
pub fn read_rich_yaml(text_or_path: &str) -> CliResult<AttrTree> {
    let r = if Path::new(text_or_path).is_file() {
        read_yaml_file(text_or_path, false)
    } else {
        read_yaml_str(text_or_path, false)
    };
    r.map_err(|e| CliError::from_error(&e))
}

/// Python's `str.strip()` on the command (whitespace only at the ends).
fn pad(i: usize, width: usize) -> String {
    format!("{i:0>width$}")
}

/// `generate_runs`: the `calliope run` invocations.
pub fn generate_runs(
    model_file: &str,
    scenarios: Option<&str>,
    additional_args: &str,
    override_dict: Option<&str>,
) -> CliResult<Vec<String>> {
    let runs: Vec<String> = if let Some(s) = scenarios {
        s.split(';').map(str::to_owned).collect()
    } else {
        let mut config = read_yaml_file(model_file, false).map_err(|e| CliError::from_error(&e))?;
        if let Some(o) = override_dict.filter(|o| !o.is_empty()) {
            let over = read_rich_yaml(o)?;
            config
                .union(&over, true, true)
                .map_err(|e| CliError::from_error(&e))?;
        }
        let section = config.get("scenarios").or_else(|| config.get("overrides"));
        match section.and_then(Node::as_map) {
            Some(m) => m.keys().cloned().collect(),
            None => {
                return Err(CliError::new(
                    "the model has neither `scenarios` nor `overrides` to generate runs from",
                ));
            }
        }
    };
    let width = runs.len().to_string().len();
    let mut commands = Vec::new();
    for (i, run) in runs.iter().enumerate() {
        let mut cmd = format!(
            "calliope run {model_file} --scenario {run} --save_netcdf out_{}_{run}.nc ",
            pad(i + 1, width)
        )
        .trim()
        .to_owned();
        if let Some(o) = override_dict.filter(|o| !o.is_empty()) {
            cmd = format!("{cmd} --override_dict=\"{o}\"");
        }
        if !additional_args.is_empty() {
            cmd = format!("{cmd} {additional_args}");
        }
        commands.push(cmd);
    }
    Ok(commands)
}

#[cfg(unix)]
fn make_executable(path: &Path) -> CliResult<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> CliResult<()> {
    Ok(())
}

fn write_script(path: &str, text: &str, executable: bool) -> CliResult<()> {
    std::fs::write(path, text.as_bytes()).map_err(|e| CliError::new(format!("{path}: {e}")))?;
    if executable {
        make_executable(Path::new(path))?;
    }
    Ok(())
}

/// The bash script text for the commands.
pub fn bash_text(commands: &[String]) -> String {
    let mut lines: Vec<String> = vec![
        "#!/bin/sh".into(),
        String::new(),
        "function process_case () {".into(),
        "    case \"$1\" in".into(),
        String::new(),
    ];
    // Python joins `"    {i}) {cmd} ;;\n"` with "\n", which doubles the newlines.
    lines.extend(
        commands
            .iter()
            .enumerate()
            .map(|(i, c)| format!("    {}) {c} ;;\n", i + 1)),
    );
    lines.extend([
        "    esac".to_owned(),
        "}".into(),
        String::new(),
        "if [[ $# -eq 0 ]] ; then".into(),
        "    echo No parameter given, running all runs sequentially...".into(),
        format!(
            "    for i in $(seq 1 {}); do process_case $i; done",
            commands.len()
        ),
        "else".into(),
        "    echo Running run $1".into(),
        "    process_case $1".into(),
        "fi".into(),
        String::new(),
    ]);
    lines.join("\n")
}

/// The Windows script text for the commands.
pub fn windows_text(commands: &[String]) -> String {
    let mut lines = vec!["@echo off".to_owned(), String::new()];
    lines.extend(
        commands
            .iter()
            .enumerate()
            .map(|(i, c)| format!("echo \"Run {}\"\r\n{c}\r\n", i + 1)),
    );
    lines.join("\r\n")
}

fn none_or(v: Option<&str>) -> String {
    v.map_or_else(|| "None".to_owned(), str::to_owned)
}

/// `pd.to_datetime(float(minutes), unit="m").strftime("%H:%M:%S")`.
pub fn minutes_to_hms(minutes: &str) -> CliResult<String> {
    let m: f64 = minutes
        .trim()
        .parse()
        .map_err(|_| CliError::new(format!("could not convert string to float: '{minutes}'")))?;
    if !m.is_finite() {
        return Err(CliError::new("cannot convert a non-finite time"));
    }
    // pandas works in integer nanoseconds and `%S` truncates.
    let secs = (m * 60.0 * 1e9).round().div_euclid(1e9) as i64;
    let day = secs.rem_euclid(86_400);
    Ok(format!(
        "{:02}:{:02}:{:02}",
        day / 3600,
        (day / 60) % 60,
        day % 60
    ))
}

fn bsub_text(n: usize, a: &RunsArgs, array_name: &str) -> String {
    [
        "#!/bin/sh".to_owned(),
        format!("#BSUB -J calliope[1-{n}]"),
        format!("#BSUB -n {}", a.cluster_threads),
        format!(
            "#BSUB -R \"rusage[mem={}]\"",
            none_or(a.cluster_mem.as_deref())
        ),
        format!("#BSUB -W {}", none_or(a.cluster_time.as_deref())),
        "#BSUB -r".into(),
        "#BSUB -o log_%I.log".into(),
        String::new(),
        format!("./{array_name} ${{LSB_JOBINDEX}}"),
        String::new(),
    ]
    .join("\n")
}

fn sbatch_text(n: usize, a: &RunsArgs, array_name: &str) -> CliResult<String> {
    let time = a
        .cluster_time
        .as_deref()
        .ok_or_else(|| CliError::new("argument of type 'NoneType' is not iterable"))?;
    let time = if time.contains(':') {
        time.to_owned()
    } else {
        minutes_to_hms(time)?
    };
    Ok([
        "#!/bin/bash".to_owned(),
        "#SBATCH -J calliope".into(),
        format!("#SBATCH --array=1-{n}"),
        format!("#SBATCH --ntasks={}", a.cluster_threads),
        format!("#SBATCH --mem={}", none_or(a.cluster_mem.as_deref())),
        format!("#SBATCH --time={time}"),
        "#SBATCH -o log_%a.log".into(),
        String::new(),
        "#! Optional add-ins for SBATCH (uncomment and add info as necessary):".into(),
        "##SBATCH -A project_name".into(),
        "##SBATCH --nodes=X".into(),
        "##SBATCH -p partition_name".into(),
        String::new(),
        "#! Insert module load commands after this line, if needed:".into(),
        "#! (Note: you can load this in ~.bashrc if you want them loaded every time you log in)"
            .into(),
        String::new(),
        "#! module load gurobi (or glpk/cplex)".into(),
        "#! module load miniconda3".into(),
        "#! module load /path/to/miniconda3/envs/your_env_name/".into(),
        String::new(),
        "cd $SLURM_SUBMIT_DIR".into(),
        String::new(),
        format!("./{array_name} ${{SLURM_ARRAY_TASK_ID}}"),
    ]
    .join("\n"))
}

/// `generate(kind, **kwargs)`: writes the script(s).
pub fn generate(kind: Kind, a: &RunsArgs) -> CliResult<()> {
    let commands = generate_runs(
        &a.model_file,
        a.scenarios.as_deref(),
        &a.additional_args,
        a.override_dict.as_deref(),
    )?;
    match kind {
        Kind::Bash => write_script(&a.out_file, &bash_text(&commands), true),
        Kind::Windows => write_script(&a.out_file, &windows_text(&commands), true),
        Kind::Bsub | Kind::Sbatch => {
            let array_file = format!("{}.array.sh", a.out_file);
            let array_name = Path::new(&array_file)
                .file_name()
                .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
            write_script(&array_file, &bash_text(&commands), true)?;
            let text = if kind == Kind::Bsub {
                bsub_text(commands.len(), a, &array_name)
            } else {
                sbatch_text(commands.len(), a, &array_name)?
            };
            write_script(&a.out_file, &text, false)
        }
    }
}

/// The YAML of `generate_scenarios`: the cartesian product of `;`-separated override lists.
pub fn scenarios_yaml(overrides: &[String], prefix: Option<&str>) -> CliResult<String> {
    let prefix = prefix.filter(|p| !p.is_empty()).unwrap_or("scenario_");
    let options: Vec<Vec<&str>> = overrides.iter().map(|o| o.split(';').collect()).collect();
    let mut combos: Vec<Vec<&str>> = vec![Vec::new()];
    for opts in &options {
        combos = combos
            .iter()
            .flat_map(|c| {
                opts.iter().map(move |o| {
                    let mut n = c.clone();
                    n.push(o);
                    n
                })
            })
            .collect();
    }
    let width = combos.len().to_string().len();
    let mut inner = AttrTree::new();
    for (i, c) in combos.iter().enumerate() {
        let seq = c
            .iter()
            .map(|s| Node::new(Value::Str((*s).to_owned())))
            .collect();
        inner.insert(
            format!("{prefix}{}", pad(i + 1, width)),
            Node::new(Value::Seq(seq)),
        );
    }
    let mut root = AttrTree::new();
    root.insert("scenarios", Node::new(Value::Map(inner)));
    Ok(to_yaml(&root))
}
