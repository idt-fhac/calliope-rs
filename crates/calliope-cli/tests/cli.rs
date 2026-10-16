//! Ports of Python's `tests/cli_test.py` (build, solve and save included), and byte-for-byte
//! comparisons of the generated scripts with Python's.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const BIN: &str = env!("CARGO_BIN_EXE_calliope");

fn reference() -> PathBuf {
    std::env::var("CALLIOPE_REFERENCE").map_or_else(
        |_| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../calliope"),
        PathBuf::from,
    )
}

fn national() -> Option<PathBuf> {
    let p = reference().join("src/calliope/example_models/national_scale/model.yaml");
    p.is_file().then_some(p)
}

fn run(args: &[&str]) -> Output {
    Command::new(BIN).args(args).output().unwrap()
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn new_copies_example() {
    let d = tempfile::tempdir().unwrap();
    let t = d.path().join("a/new_model");
    let o = run(&["new", t.to_str().unwrap()]);
    assert!(o.status.success(), "{}", stdout(&o));
    assert!(t.join("model.yaml").is_file());
    assert!(t.join("data_tables/costs.csv").is_file());
    // the target must not exist yet
    assert_eq!(run(&["new", t.to_str().unwrap()]).status.code(), Some(1));
    let u = d.path().join("urban");
    assert!(
        run(&["new", u.to_str().unwrap(), "--template", "urban_scale"])
            .status
            .success()
    );
    assert!(u.join("additional_math.yaml").is_file());
}

#[test]
fn incorrect_file_format() {
    let o = run(&["run", "test_model.txt", "--save_netcdf=output.nc"]);
    assert!(stdout(&o).contains("Cannot determine model file format"));
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn incorrect_model_format() {
    let o = run(&[
        "run",
        "test_model.txt",
        "--model_format=yml",
        "--save_netcdf=output.nc",
    ]);
    assert!(stdout(&o).contains("Invalid model format"));
    assert_eq!(o.status.code(), Some(1));
}

#[test]
fn unavailable_arguments() {
    for arg in [
        "--scenario=test",
        "--override_dict={'config.init.name': 'test'}",
    ] {
        let o = run(&["run", "test_model.nc", arg, "--save_netcdf=output.nc"]);
        assert!(
            stdout(&o).contains("the --scenario and --override_dict options are not available")
        );
        assert_eq!(o.status.code(), Some(1));
    }
}

#[test]
fn fail_no_debug_no_stderr_and_debug_stderr() {
    let o = run(&["run", "nonexistent_model.yaml"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let o = run(&["run", "nonexistent_model.yaml", "--debug"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(!o.stderr.is_empty());
}

fn test_model() -> PathBuf {
    reference().join("tests/common/test_model/model.yaml")
}

#[test]
fn run_from_yaml_saves_netcdf_csv_and_logs() {
    let Some(m) = national() else { return };
    let d = tempfile::tempdir().unwrap();
    let nc = d.path().join("o.nc");
    let logs = d.path().join("logs");
    let o = run(&[
        "run",
        m.to_str().unwrap(),
        "--override_dict={'config.solve.solver': 'glpk'}",
        &format!("--save_netcdf={}", nc.display()),
        &format!("--save_csv={}", d.path().join("csv").display()),
        &format!("--save_logs={}", logs.display()),
        "--quiet",
        "--pdb",
        "--profile",
    ]);
    let out = stdout(&o);
    assert_eq!(o.status.code(), Some(0), "{out}");
    assert!(out.contains("Model name:"), "{out}");
    assert!(nc.is_file());
    assert!(d.path().join("csv/results_flow_cap.csv").is_file());
    assert!(d.path().join("csv/attrs.yaml").is_file());
    // Python drops `--save_logs` (bench/BUGS.md #3); the port passes it to HiGHS.
    assert!(logs.join("highs.log").is_file());
}

#[test]
fn run_from_netcdf() {
    let Some(m) = national() else { return };
    let d = tempfile::tempdir().unwrap();
    // A model file written by calliope-rs before solving, and one written by Python.
    let ours = d.path().join("model.nc");
    calliope_model::Model::from_yaml(&m, &calliope_preprocess::ReadOptions::default())
        .unwrap()
        .to_netcdf(&ours)
        .unwrap();
    let python =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../golden/national_scale/inputs.nc");
    for (i, model_file) in [ours, python].iter().enumerate() {
        let out = d.path().join(format!("output{i}.nc"));
        let o = run(&[
            "run",
            model_file.to_str().unwrap(),
            &format!("--save_netcdf={}", out.display()),
        ]);
        assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
        assert!(out.is_file());
    }
}

#[test]
fn run_save_lp() {
    let Some(m) = national() else { return };
    let d = tempfile::tempdir().unwrap();
    let lp = d.path().join("output.lp");
    let o = run(&[
        "run",
        m.to_str().unwrap(),
        &format!("--save_lp={}", lp.display()),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
    let text = std::fs::read_to_string(&lp).unwrap();
    assert!(text.contains("min"));
}

#[test]
fn save_per_spore() {
    let Some(m) = national() else { return };
    let d = tempfile::tempdir().unwrap();
    let o = run(&[
        "run",
        m.to_str().unwrap(),
        &format!("--save_netcdf={}", d.path().join("output.nc").display()),
        "--scenario=spores",
        &format!(
            "--override_dict={{config.solve.spores.save_per_spore_path: '{}', config.init.subset.timesteps: ['2005-01-01', '2005-01-02']}}",
            d.path().join("elsewhere").display()
        ),
    ]);
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
    // With --save_netcdf the SPOREs go next to it (Python drops the redirect, bug03).
    for i in 0..4 {
        assert!(
            d.path().join(format!("spore_{i}.nc")).is_file(),
            "spore {i}"
        );
    }
    assert!(d.path().join("output.nc").is_file());
}

#[test]
fn exit_code_when_infeasible() {
    let m = test_model();
    if !m.is_file() {
        return;
    }
    let args = [
        "run",
        m.to_str().unwrap(),
        "--scenario=simple_supply",
        "--override_dict={techs.test_supply_elec.flow_cap_max: 1}",
    ];
    let o = run(&args);
    assert_ne!(o.status.code(), Some(0), "{}", stdout(&o));
    let mut with_flag = args.to_vec();
    with_flag.push("--no_fail_when_infeasible");
    let o = run(&with_flag);
    assert_eq!(o.status.code(), Some(0), "{}", stdout(&o));
}

#[test]
fn generate_scenarios() {
    let d = tempfile::tempdir().unwrap();
    let out = d.path().join("scenarios.yaml");
    let o = run(&[
        "generate_scenarios",
        "model.yaml",
        out.to_str().unwrap(),
        "cold_fusion",
        "run1;run2",
        "cold_fusion_cap_share;cold_fusion_prod_share",
    ]);
    assert!(o.status.success());
    let text = std::fs::read_to_string(&out).unwrap();
    assert!(!text.contains("scenario_0:"));
    assert!(text.contains("scenario_1:\n  - cold_fusion\n  - run1\n  - cold_fusion_cap_share\n"));
    assert!(text.contains("scenario_4:"));
}

/// Python's output for the same arguments, via the reference environment.
fn python_generate(kind: &str, out: &Path, extra: &[(&str, &str)]) -> bool {
    let py = reference().join(".pixi/envs/dev/bin/python");
    if !py.is_file() {
        eprintln!("skipping: no Python reference environment");
        return false;
    }
    let script = r#"
import sys, json
from calliope.util.generate_runs import generate
a = json.loads(sys.argv[1])
generate(a.pop("kind"), **a)
"#;
    let mut args = serde_like(kind, out, extra);
    let o = Command::new(py)
        .args(["-c", script, &args])
        .output()
        .unwrap();
    args.clear();
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    true
}

fn serde_like(kind: &str, out: &Path, extra: &[(&str, &str)]) -> String {
    let mut s = format!(
        "{{\"kind\": \"{kind}\", \"model_file\": \"model.yaml\", \"out_file\": \"{}\", \"scenarios\": \"run1;run2;run3;run4;run5;run6;run7;run8;run9;run10\", \"additional_args\": \"--quiet\", \"override_dict\": null, \"cluster_threads\": 4, \"cluster_mem\": null, \"cluster_time\": null",
        out.display()
    );
    for (k, v) in extra {
        s.push_str(&format!(", \"{k}\": \"{v}\""));
    }
    s.push('}');
    s
}

#[test]
fn generated_scripts_are_byte_identical_to_python() {
    let cases: [(&str, &[(&str, &str)]); 7] = [
        ("bash", &[]),
        ("windows", &[]),
        ("bsub", &[("cluster_mem", "1G"), ("cluster_time", "100")]),
        ("bsub", &[]),
        ("sbatch", &[("cluster_mem", "1G"), ("cluster_time", "100")]),
        (
            "sbatch",
            &[("cluster_mem", "2G"), ("cluster_time", "1500.5")],
        ),
        (
            "sbatch",
            &[("cluster_mem", "2G"), ("cluster_time", "01:30:00")],
        ),
    ];
    for (kind, extra) in cases {
        let d = tempfile::tempdir().unwrap();
        let py = d.path().join("py");
        let rs = d.path().join("rs");
        std::fs::create_dir_all(&py).unwrap();
        std::fs::create_dir_all(&rs).unwrap();
        if !python_generate(kind, &py.join("test.sh"), extra) {
            return;
        }
        let mut args = vec![
            "generate_runs".to_owned(),
            "model.yaml".to_owned(),
            rs.join("test.sh").display().to_string(),
            format!("--kind={kind}"),
            "--scenarios=run1;run2;run3;run4;run5;run6;run7;run8;run9;run10".to_owned(),
            "--additional_args=--quiet".to_owned(),
            "--cluster_threads=4".to_owned(),
        ];
        for (k, v) in extra {
            args.push(format!("--{k}={v}"));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let o = run(&refs);
        assert!(o.status.success(), "{kind}: {}", stdout(&o));
        let mut names: Vec<_> = std::fs::read_dir(&py)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        names.sort();
        assert!(!names.is_empty());
        for n in names {
            assert_eq!(
                std::fs::read(py.join(&n)).unwrap(),
                std::fs::read(rs.join(&n)).unwrap(),
                "{kind} {n:?} {extra:?}"
            );
        }
    }
}

#[test]
fn check_passes_on_example_models() {
    let Some(m) = national() else { return };
    let o = run(&["check", m.to_str().unwrap()]);
    assert!(o.status.success(), "{}", stdout(&o));
    let bad = tempfile::tempdir().unwrap();
    let f = bad.path().join("model.yaml");
    std::fs::write(&f, "config:\n  init:\n    nope: 1\n").unwrap();
    assert_eq!(run(&["check", f.to_str().unwrap()]).status.code(), Some(1));
}

#[test]
fn lp_diff_equal_files() {
    let d = tempfile::tempdir().unwrap();
    let f = d.path().join("a.lp");
    std::fs::write(
        &f,
        "min\nobj:\n+1 x\n\ns.t.\n\nc1:\n+1 x\n>= 1\n\nbounds\n   0 <= x <= 5\nend\n",
    )
    .unwrap();
    let o = run(&["lp-diff", f.to_str().unwrap(), f.to_str().unwrap()]);
    assert!(o.status.success(), "{}", stdout(&o));
}

const ONE_DAY: &str =
    "--override_dict={'config.init.subset.timesteps': ['2005-01-01', '2005-01-01']}";

#[test]
fn run_shows_solver_output_unless_quiet() {
    let Some(m) = national() else { return };
    let o = run(&["run", m.to_str().unwrap(), ONE_DAY]);
    assert!(o.status.success());
    assert!(stdout(&o).contains("HiGHS"), "{}", stdout(&o));
    let o = run(&["run", m.to_str().unwrap(), ONE_DAY, "--quiet"]);
    assert!(o.status.success());
    assert!(!stdout(&o).contains("HiGHS"), "{}", stdout(&o));
}

#[test]
fn inspect_shows_equations_index_set_and_rows() {
    let Some(m) = national() else { return };
    let o = run(&[
        "inspect",
        m.to_str().unwrap(),
        ONE_DAY,
        "--component",
        "balance_supply_with_storage",
        "--max",
        "2",
    ]);
    let out = stdout(&o);
    assert!(o.status.success(), "{out}");
    assert!(
        out.starts_with("constraints.balance_supply_with_storage\n"),
        "{out}"
    );
    assert!(out.contains("expanded equations (3):"), "{out}");
    assert!(
        out.contains("constraints:balance_supply_with_storage:0-storage_previous_step:1"),
        "{out}"
    );
    // csp at three nodes for 24 hours.
    assert!(
        out.contains("active index set: 72 of 960 foreach combinations"),
        "{out}"
    );
    assert!(
        out.contains("  [region1_1, csp, power, 2005-01-01 00:00]\n"),
        "{out}"
    );
    assert!(out.contains("== 0\n"), "{out}");
    assert!(out.contains("... 70 more (--max)"), "{out}");

    let o = run(&[
        "inspect",
        m.to_str().unwrap(),
        ONE_DAY,
        "--component",
        "flow_cap",
    ]);
    let out = stdout(&o);
    assert!(
        out.contains("0 <= variables(flow_cap)(region1__ccgt__power) <= 30000"),
        "{out}"
    );

    let o = run(&[
        "inspect",
        m.to_str().unwrap(),
        ONE_DAY,
        "--component",
        "nope",
    ]);
    assert_eq!(o.status.code(), Some(1));
    assert!(
        stdout(&o).contains("`nope` is not an active"),
        "{}",
        stdout(&o)
    );
}

#[test]
fn bench_reports_each_stage() {
    let Some(m) = national() else { return };
    let o = run(&["bench", m.to_str().unwrap(), ONE_DAY, "--json"]);
    let out = stdout(&o);
    assert!(o.status.success(), "{out}");
    for stage in ["preprocess", "build", "solve", "write"] {
        assert!(
            out.contains(&format!("\"{stage}\": {{\"wall_s\": ")),
            "{out}"
        );
    }
    assert!(out.contains("\"total_wall_s\": "), "{out}");
    let o = run(&["bench", m.to_str().unwrap(), ONE_DAY]);
    let out = stdout(&o);
    assert!(out.contains("Model name:"), "{out}");
    assert!(out.lines().any(|l| l.starts_with("total ")), "{out}");
}
