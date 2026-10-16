//! `xtask parity`: run calliope-rs on every golden fixture and compare with Python's output.
//!
//! Three comparisons per fixture (PLAN.md Phase 1):
//! - `inputs.nc`: NetCDF data/attrs, relative tolerance 1e-9 (see [`crate::ncdiff`]);
//! - LP: canonical comparison against the Pyomo (or HiGHS) LP ([`calliope_lp::lp_format`]);
//! - results: objective relative 1e-6, `results.nc` arrays absolute 1e-6 scaled by magnitude.
//!   Operate and SPORES runs depend on the solver path: arrays are reported, not failed.
//!
//! [`crate::runner::RustRunner`] runs calliope-rs in-process. `--self-check` runs the comparison
//! machinery with the golden files as the "actual" output, which must pass everywhere.
//! Differences with a recorded reason ([`crate::known`]) are reported as classified and do not
//! fail the run.

use crate::known::Allow;
use crate::ncdiff::{self, INPUT_TOL, Part, RESULT_TOL};
use calliope_lp::lp_format::{DiffOptions, diff_lp};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::fmt;
use std::path::{Path, PathBuf};

/// A golden fixture from `golden/MANIFEST.json`.
#[derive(Debug, Clone)]
pub struct Fixture {
    /// Fixture name (directory under `golden/`).
    pub name: String,
    /// Directory with the golden files.
    pub dir: PathBuf,
    /// Expected build/solve error message from Python, when the fixture is an error case.
    pub expected_error: Option<String>,
    /// Objective values per objective name and iteration (operate windows, SPORES).
    pub objectives: BTreeMap<String, Vec<f64>>,
    /// Termination condition reported by Python.
    pub termination: Option<String>,
}

impl Fixture {
    /// Path of a golden file.
    pub fn file(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Operate and SPORES results depend on the path the solver takes.
    pub fn path_dependent(&self) -> bool {
        self.name.contains("operate") || self.name.contains("spores")
    }
}

/// Files written by a run of calliope-rs.
#[derive(Debug, Clone, Default)]
pub struct RunOutput {
    /// Inputs NetCDF.
    pub inputs_nc: Option<PathBuf>,
    /// LP file (Pyomo or HiGHS flavour).
    pub lp: Option<PathBuf>,
    /// Results NetCDF.
    pub results_nc: Option<PathBuf>,
    /// Objective values per objective name and iteration.
    pub objectives: BTreeMap<String, Vec<f64>>,
    /// Termination condition.
    pub termination: Option<String>,
}

/// Why a run produced no output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RunError {
    /// The runner cannot run this fixture.
    #[cfg_attr(not(test), allow(dead_code))]
    NotImplemented,
    /// calliope-rs ran and failed with this message (compared against `expected_error`).
    Failed(String),
}

/// Runs calliope-rs on a fixture.
pub trait Runner {
    /// Runner name for the report.
    fn name(&self) -> &str;
    /// Build, solve and write the outputs of `fixture` below `work`.
    fn run(&self, fixture: &Fixture, work: &Path) -> Result<RunOutput, RunError>;
}

/// A runner that runs nothing (tests the "not run" path).
#[cfg(test)]
#[derive(Debug, Clone, Copy)]
pub struct Unimplemented;

#[cfg(test)]
impl Runner for Unimplemented {
    fn name(&self) -> &str {
        "calliope-rs (not implemented)"
    }
    fn run(&self, _: &Fixture, _: &Path) -> Result<RunOutput, RunError> {
        Err(RunError::NotImplemented)
    }
}

/// Returns the golden files as if calliope-rs had produced them (tests the comparison code).
#[derive(Debug, Clone, Copy)]
pub struct GoldenRunner;

impl Runner for GoldenRunner {
    fn name(&self) -> &str {
        "golden self-check"
    }
    fn run(&self, f: &Fixture, _: &Path) -> Result<RunOutput, RunError> {
        if let Some(m) = &f.expected_error {
            return Err(RunError::Failed(m.clone()));
        }
        let exists = |n: &str| Some(f.file(n)).filter(|p| p.exists());
        // Prefer the HiGHS file to exercise the structural comparison.
        let lp = exists("highs.lp").or_else(|| exists("pyomo.lp"));
        Ok(RunOutput {
            inputs_nc: exists("inputs.nc"),
            lp,
            results_nc: exists("results.nc"),
            objectives: f.objectives.clone(),
            termination: f.termination.clone(),
        })
    }
}

/// Outcome of one comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    /// Equal under the comparison rules.
    Pass,
    /// Differences found (first few lines kept).
    Fail(Vec<String>),
    /// Could not be compared (runner not implemented, or file missing).
    NotRun(String),
    /// Not applicable for this fixture.
    Skipped(String),
    /// The comparison itself failed (unreadable file, parse error, Python failure).
    Error(String),
    /// Differences with a recorded reason ([`crate::known`]); not failing.
    Classified(&'static str, Vec<String>),
}

impl fmt::Display for Status {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Pass => f.write_str("pass"),
            Self::Fail(d) => write!(f, "FAIL({})", d.len()),
            Self::NotRun(_) => f.write_str("not implemented"),
            Self::Skipped(_) => f.write_str("n/a"),
            Self::Error(_) => f.write_str("ERROR"),
            Self::Classified(c, _) => write!(f, "classified:{c}"),
        }
    }
}

/// Result for one fixture.
#[derive(Debug, Clone)]
pub struct FixtureReport {
    /// Fixture name.
    pub name: String,
    /// Inputs comparison.
    pub inputs: Status,
    /// LP comparison.
    pub lp: Status,
    /// Results comparison.
    pub results: Status,
    /// Informational notes (non-failing differences).
    pub notes: Vec<String>,
}

impl FixtureReport {
    /// Statuses in report order.
    pub fn statuses(&self) -> [&Status; 3] {
        [&self.inputs, &self.lp, &self.results]
    }
}

/// Load fixtures from `golden/MANIFEST.json` (math LP fixtures are not part of this list).
pub fn load_manifest(golden: &Path) -> Result<Vec<Fixture>, String> {
    let path = golden.join("MANIFEST.json");
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let v: Value = serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let fixtures = v["fixtures"]
        .as_object()
        .ok_or("MANIFEST.json: no fixtures object")?;
    let mut out = Vec::new();
    for (name, f) in fixtures {
        let mut objectives = BTreeMap::new();
        if let Some(o) = f["objectives"].as_object() {
            for (k, v) in o {
                let vals = match v {
                    Value::Array(a) => a.iter().filter_map(Value::as_f64).collect(),
                    other => other.as_f64().into_iter().collect(),
                };
                objectives.insert(k.clone(), vals);
            }
        }
        out.push(Fixture {
            name: name.clone(),
            dir: golden.join(name),
            expected_error: f["expected_error"]["message"].as_str().map(str::to_string),
            objectives,
            termination: f["termination_condition"].as_str().map(str::to_string),
        });
    }
    Ok(out)
}

/// Where to find Python and the dump script.
#[derive(Debug, Clone)]
pub struct Env {
    /// Interpreter with xarray, netCDF4 and yaml.
    pub python: PathBuf,
    /// `oracle/nc_dump.py`.
    pub script: PathBuf,
}

/// Runs the comparisons, caching NetCDF dumps.
#[allow(missing_debug_implementations)]
pub struct Comparer<'a> {
    env: &'a Env,
    cache: HashMap<PathBuf, Value>,
}

impl<'a> Comparer<'a> {
    /// New comparer.
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            cache: HashMap::new(),
        }
    }

    fn dump(&mut self, p: &Path) -> Result<&Value, String> {
        if !self.cache.contains_key(p) {
            let v = ncdiff::dump(&self.env.python, &self.env.script, p)?;
            self.cache.insert(p.to_path_buf(), v);
        }
        Ok(&self.cache[p])
    }

    fn nc(
        &mut self,
        golden: &Path,
        actual: Option<&Path>,
        part: Part,
        tol: ncdiff::Tolerance,
    ) -> Result<Vec<String>, String> {
        let Some(actual) = actual else {
            return Ok(vec!["runner wrote no file".into()]);
        };
        let g = self.dump(golden)?.clone();
        let a = self.dump(actual)?;
        Ok(ncdiff::diff_dumps(&g, a, part, tol))
    }

    fn lp(&self, f: &Fixture, actual: Option<&Path>) -> Result<Vec<String>, String> {
        let Some(actual) = actual else {
            return Ok(vec!["runner wrote no LP file".into()]);
        };
        let text =
            std::fs::read_to_string(actual).map_err(|e| format!("{}: {e}", actual.display()))?;
        let opts = DiffOptions::default();
        let mut first_failure: Option<Vec<String>> = None;
        for g in ["pyomo.lp", "highs.lp"] {
            let gp = f.file(g);
            if !gp.exists() {
                continue;
            }
            let gt = std::fs::read_to_string(&gp).map_err(|e| e.to_string())?;
            if !gt.contains("\nmin") && !gt.contains("\nmax") {
                continue; // empty LP: the Python build failed or wrote nothing
            }
            let d = diff_lp(&gt, &text, &opts).map_err(|e| format!("{g}: {e}"))?;
            if d.is_equal() {
                return Ok(Vec::new());
            }
            first_failure.get_or_insert_with(|| {
                d.differences
                    .iter()
                    .take(20)
                    .map(|l| format!("vs {g}: {l}"))
                    .collect()
            });
        }
        Ok(first_failure.unwrap_or_else(|| vec!["no golden LP file".into()]))
    }

    /// Compare one fixture.
    pub fn compare(&mut self, f: &Fixture, runner: &dyn Runner, work: &Path) -> FixtureReport {
        let mut rep = FixtureReport {
            name: f.name.clone(),
            inputs: Status::Pass,
            lp: Status::Pass,
            results: Status::Pass,
            notes: Vec::new(),
        };
        let out = match runner.run(f, work) {
            Ok(o) => o,
            Err(RunError::NotImplemented) => {
                let s = Status::NotRun("runner not implemented".into());
                (rep.inputs, rep.lp, rep.results) = (s.clone(), s.clone(), s);
                return rep;
            }
            Err(RunError::Failed(msg)) => {
                let known = crate::known::lookup(&f.name);
                let s = match &f.expected_error {
                    Some(e) if msg.contains(e.as_str()) => Status::Pass,
                    Some(e) if known.is_some_and(|k| k.allow == Allow::ErrorMessage) => {
                        let k = known.expect("checked");
                        Status::Classified(
                            k.class,
                            vec![k.reason.to_owned(), format!("error {msg:?} vs {e:?}")],
                        )
                    }
                    Some(e) => Status::Fail(vec![format!("error {msg:?} does not contain {e:?}")]),
                    None => Status::Fail(vec![format!("unexpected failure: {msg}")]),
                };
                (rep.inputs, rep.lp, rep.results) = (s.clone(), s.clone(), s);
                return rep;
            }
        };
        if let Some(e) = &f.expected_error {
            let s = Status::Fail(vec![format!("expected error {e:?}, but the run succeeded")]);
            (rep.inputs, rep.lp, rep.results) = (s.clone(), s.clone(), s);
            return rep;
        }
        let to_status = |r: Result<Vec<String>, String>| match r {
            Ok(d) if d.is_empty() => Status::Pass,
            Ok(d) => Status::Fail(d),
            Err(e) => Status::Error(e),
        };
        let g_inputs = f.file("inputs.nc");
        rep.inputs = if g_inputs.exists() {
            to_status(self.nc(&g_inputs, out.inputs_nc.as_deref(), Part::Inputs, INPUT_TOL))
        } else {
            Status::Skipped("no golden inputs.nc".into())
        };
        rep.lp = if f.name.contains("operate") {
            Status::Skipped("operate LP dumps cover different windows".into())
        } else {
            to_status(self.lp(f, out.lp.as_deref()))
        };
        rep.results = self.results(f, &out, &mut rep.notes);
        rep
    }

    fn results(&mut self, f: &Fixture, out: &RunOutput, notes: &mut Vec<String>) -> Status {
        let known = crate::known::lookup(&f.name);
        let objective_tol = match known.map(|k| k.allow) {
            Some(Allow::Objective(t)) => t,
            _ => 1e-6,
        };
        let mut fails = objective_diff_tol(f, out, objective_tol);
        let mut classified = objective_diff(f, out);
        if let (Some(g), Some(t)) = (&f.termination, &out.termination)
            && g != t
        {
            fails.push(format!("termination condition {g:?} vs {t:?}"));
        }
        let arrays_allowed = matches!(
            known.map(|k| k.allow),
            Some(Allow::Arrays | Allow::Objective(_))
        );
        let g_res = f.file("results.nc");
        if g_res.exists() {
            match self.nc(&g_res, out.results_nc.as_deref(), Part::Results, RESULT_TOL) {
                Ok(d) if f.path_dependent() => {
                    notes.extend(
                        d.into_iter()
                            .take(10)
                            .map(|l| format!("vertex/array (not failing): {l}")),
                    );
                }
                Ok(d) if arrays_allowed => classified.extend(d),
                Ok(d) => fails.extend(d),
                Err(e) => return Status::Error(e),
            }
        }
        if !fails.is_empty() {
            Status::Fail(fails)
        } else if classified.is_empty() {
            Status::Pass
        } else {
            let k = known.expect("only classified fixtures allow differences");
            let mut lines = vec![k.reason.to_owned()];
            lines.extend(classified);
            Status::Classified(k.class, lines)
        }
    }
}

/// Objective comparison, relative 1e-6.
pub fn objective_diff(f: &Fixture, out: &RunOutput) -> Vec<String> {
    objective_diff_tol(f, out, 1e-6)
}

/// Objective comparison with relative tolerance `tol`.
pub fn objective_diff_tol(f: &Fixture, out: &RunOutput, tol: f64) -> Vec<String> {
    let mut d = Vec::new();
    for (name, want) in &f.objectives {
        let Some(got) = out.objectives.get(name) else {
            d.push(format!("objective {name}: missing"));
            continue;
        };
        if got.len() != want.len() {
            d.push(format!(
                "objective {name}: {} values vs {}",
                got.len(),
                want.len()
            ));
            continue;
        }
        for (i, (w, g)) in want.iter().zip(got).enumerate() {
            if (w - g).abs() > tol * w.abs().max(g.abs()).max(1e-12) {
                d.push(format!("objective {name}[{i}]: {w} vs {g}"));
                break;
            }
        }
    }
    d
}

/// Render the report as a table plus details.
pub fn render(reports: &[FixtureReport], runner: &str) -> String {
    use std::fmt::Write as _;
    let width = reports
        .iter()
        .map(|r| r.name.len())
        .max()
        .unwrap_or(0)
        .max(7);
    let mut s = format!("parity with {runner}\n\n");
    let _ = writeln!(
        s,
        "{:<width$}  {:<16} {:<16} {:<16}",
        "fixture", "inputs", "lp", "results"
    );
    for r in reports {
        let _ = writeln!(
            s,
            "{:<width$}  {:<16} {:<16} {:<16}",
            r.name,
            r.inputs.to_string(),
            r.lp.to_string(),
            r.results.to_string()
        );
    }
    for r in reports {
        for (label, st) in [
            ("inputs", &r.inputs),
            ("lp", &r.lp),
            ("results", &r.results),
        ] {
            if let Status::Error(e) = st {
                let _ = writeln!(s, "\n{} {label}: {e}", r.name);
            }
            if let Status::Fail(d) = st {
                let _ = writeln!(s, "\n{} {label}:", r.name);
                for l in d.iter().take(10) {
                    let _ = writeln!(s, "  {l}");
                }
            }
            if let Status::Classified(c, d) = st {
                let _ = writeln!(s, "\n{} {label} (classified {c}):", r.name);
                for l in d.iter().take(6) {
                    let _ = writeln!(s, "  {l}");
                }
            }
        }
        for n in &r.notes {
            let _ = writeln!(s, "note {}: {n}", r.name);
        }
    }
    s
}

/// Counts over all statuses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Tally {
    /// Equal.
    pub pass: usize,
    /// Different with a recorded reason.
    pub classified: usize,
    /// Different (or the comparison failed).
    pub fail: usize,
    /// Not run.
    pub not_run: usize,
}

/// Count statuses.
pub fn tally(reports: &[FixtureReport]) -> Tally {
    let mut t = Tally::default();
    for r in reports {
        for s in r.statuses() {
            match s {
                Status::Pass => t.pass += 1,
                Status::Classified(..) => t.classified += 1,
                Status::Fail(_) | Status::Error(_) => t.fail += 1,
                Status::NotRun(_) => t.not_run += 1,
                Status::Skipped(_) => {}
            }
        }
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn env() -> Option<Env> {
        let reference = std::env::var("CALLIOPE_REFERENCE")
            .map_or_else(|_| root().join("../calliope"), PathBuf::from);
        let python = reference.join(".pixi/envs/dev/bin/python");
        python.exists().then(|| Env {
            python,
            script: root().join("oracle/nc_dump.py"),
        })
    }

    fn fixture(name: &str) -> Fixture {
        load_manifest(&root().join("golden"))
            .unwrap()
            .into_iter()
            .find(|f| f.name == name)
            .unwrap()
    }

    #[test]
    fn manifest_lists_all_fixtures() {
        let f = load_manifest(&root().join("golden")).unwrap();
        assert!(f.len() >= 37);
        assert!(f.iter().any(|f| f.expected_error.is_some()));
    }

    #[test]
    fn unimplemented_runner_reports_not_run() {
        let e = Env {
            python: "none".into(),
            script: "none".into(),
        };
        let f = fixture("national_scale");
        let rep = Comparer::new(&e).compare(&f, &Unimplemented, Path::new("."));
        assert!(
            rep.statuses()
                .iter()
                .all(|s| matches!(s, Status::NotRun(_)))
        );
    }

    #[test]
    fn objectives_use_relative_tolerance() {
        let f = fixture("national_scale");
        let mut out = GoldenRunner.run(&f, Path::new(".")).unwrap();
        assert!(objective_diff(&f, &out).is_empty());
        out.objectives.get_mut("min_cost_optimisation").unwrap()[0] *= 1.0 + 1e-8;
        assert!(objective_diff(&f, &out).is_empty());
        out.objectives.get_mut("min_cost_optimisation").unwrap()[0] *= 1.0 + 1e-4;
        assert_eq!(objective_diff(&f, &out).len(), 1);
    }

    struct Perturbed(PathBuf);

    impl Runner for Perturbed {
        fn name(&self) -> &str {
            "perturbed"
        }
        fn run(&self, f: &Fixture, w: &Path) -> Result<RunOutput, RunError> {
            let mut o = GoldenRunner.run(f, w)?;
            let lp = std::fs::read_to_string(o.lp.as_ref().unwrap()).unwrap();
            let bad = lp.replacen(" <= ", " >= ", 1);
            std::fs::write(&self.0, bad).unwrap();
            o.lp = Some(self.0.clone());
            Ok(o)
        }
    }

    #[test]
    fn golden_self_check_and_perturbation() {
        let Some(env) = env() else {
            eprintln!("skipping: no Calliope pixi env");
            return;
        };
        let f = fixture("test_model__simple_supply__two_hours");
        let mut cmp = Comparer::new(&env);
        let rep = cmp.compare(&f, &GoldenRunner, Path::new("."));
        assert_eq!(
            rep.statuses().map(|s| s.to_string()),
            ["pass", "pass", "pass"],
            "{rep:?}"
        );
        let tmp = tempfile::tempdir().unwrap();
        let rep = cmp.compare(&f, &Perturbed(tmp.path().join("bad.lp")), Path::new("."));
        assert!(matches!(rep.lp, Status::Fail(_)), "{:?}", rep.lp);
        assert_eq!(rep.inputs, Status::Pass);
    }
}
