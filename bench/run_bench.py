"""Phase 0 benchmark runner for Python Calliope.

Every run is a separate subprocess (clean RSS). The child writes its JSON result
after each stage, so a run killed by the timeout or the RSS cap still reports
the stages it finished.

Variants:
  main      upstream main (``../calliope``), ``build.backend: highs``
  speedups  ``origin/feat/highs-speedups`` worktree (``../calliope-speedups``) via
            PYTHONPATH, ``build.backend: highs``
  pyomo     upstream main, ``build.backend: pyomo``, solver ``highs`` (Pyomo's
            HiGHS interface), as the reference

Cases:
  national          national_scale, full year (8760 h, 5 nodes)
  urban             urban_scale, full year (8760 h)
  syn:N:T           synthetic national_scale scale-up, N nodes x T hours
  operate:D         national_scale operate scenario over D days (window 12h,
                    horizon 24h), per-window events
  spores:D          national_scale spores scenario (integer, 3 SPORES) over D days

Usage::

    ../calliope/.pixi/envs/dev/bin/python bench/run_bench.py matrix
    ../calliope/.pixi/envs/dev/bin/python bench/run_bench.py one national main
"""

from __future__ import annotations

import argparse
import json
import os
import signal
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CALLIOPE = (REPO.parent / "calliope").resolve()
SPEEDUPS = (REPO.parent / "calliope-speedups").resolve()
WORK = HERE / ".work"
RESULTS = HERE / "results"
sys.path.insert(0, str(REPO / "oracle"))
sys.path.insert(0, str(HERE))

NATIONAL = CALLIOPE / "src/calliope/example_models/national_scale/model.yaml"
URBAN = CALLIOPE / "src/calliope/example_models/urban_scale/model.yaml"
SOLVER_OPTIONS = {"threads": 1, "random_seed": 0}
RSS_CAP_GB = float(os.environ.get("BENCH_RSS_CAP_GB", 28))


def _days(d: int) -> list[str]:
    import pandas as pd

    end = pd.Timestamp("2005-01-01") + pd.Timedelta(days=d - 1)
    return ["2005-01-01", end.strftime("%Y-%m-%d")]


def child(case: str, variant: str, out: Path):
    import calliope
    from instrument import Recorder, install

    calliope.set_log_verbosity("WARNING", include_solver_output=False)
    rec = Recorder()
    install(rec)
    result = {
        "case": case,
        "variant": variant,
        "calliope_file": calliope.__file__,
        "status": "running",
    }

    def dump():
        result["timings"] = rec.summary()
        out.write_text(json.dumps(result, indent=1, default=str))

    backend = "pyomo" if variant == "pyomo" else "highs"
    override = {"config.build.backend": backend}
    scenario = None
    if case == "national":
        path = NATIONAL
        override["config.init.subset"] = {"timesteps": None}
    elif case == "urban":
        path = URBAN
        override["config.init.subset"] = {"timesteps": None}
    elif case.startswith("syn:"):
        from synthetic import make_model

        _, n, t = case.split(":")
        path = make_model(int(n), int(t), WORK / f"syn_{n}_{t}", CALLIOPE)
    elif case.startswith("operate:"):
        path = NATIONAL
        scenario = "operate"
        override["config.init.subset"] = {"timesteps": _days(int(case.split(":")[1]))}
    elif case.startswith("spores:"):
        path = NATIONAL
        scenario = "spores"
        override["config.init.subset"] = {"timesteps": _days(int(case.split(":")[1]))}
    else:
        raise ValueError(case)

    import numpy as np

    np.random.seed(0)
    with rec.stage("preprocess"):
        model = calliope.read_yaml(path, scenario=scenario, override_dict=override)
    result["sizes"] = {k: int(v) for k, v in model.inputs.sizes.items()}
    dump()
    with rec.stage("build"):
        model.build()
    dump()
    if os.environ.get("BENCH_BUILD_ONLY"):
        result["status"] = "ok-build-only"
        dump()
        return
    if variant == "pyomo":
        # Pyomo's HiGHS interfaces (`highs`, `appsi_highs`) do not work on the
        # pyomo.kernel model Calliope builds, so the reference is: Pyomo LP
        # writer -> highspy readModel -> run. No results loading/postprocess.
        import highspy

        lp = WORK / f"pyomo_{os.getpid()}.lp"
        with rec.stage("lp_write"):
            model.backend.to_lp(lp)
        with rec.stage("solve"):
            h = highspy.Highs()
            h.setOptionValue("output_flag", False)
            for k, v in SOLVER_OPTIONS.items():
                h.setOptionValue(k, v)
            h.readModel(str(lp))
            h.run()
        result["termination_condition"] = h.modelStatusToString(h.getModelStatus())
        result["objective"] = [h.getInfo().objective_function_value]
        result["lp_mb"] = lp.stat().st_size / 2**20
        lp.unlink()
        result["status"] = "ok"
        dump()
        return
    with rec.stage("solve"):
        model.solve(solver_options=SOLVER_OPTIONS)
    result["termination_condition"] = model.runtime.termination_condition
    obj = model.config.build.objective
    if obj in model.results:
        result["objective"] = model.results[obj].values.ravel().tolist()
    if "min_spores" in model.results:
        result["objective_min_spores"] = (
            model.results["min_spores"].values.ravel().tolist()
        )
    dump()
    nc = WORK / f"out_{os.getpid()}.nc"
    with rec.stage("write"):
        model.to_netcdf(nc)
    result["netcdf_mb"] = nc.stat().st_size / 2**20
    nc.unlink()
    result["status"] = "ok"
    dump()


def run_one(case: str, variant: str, timeout: int, build_only: bool = False) -> dict:
    WORK.mkdir(parents=True, exist_ok=True)
    RESULTS.mkdir(parents=True, exist_ok=True)
    out = RESULTS / f"{case.replace(':', '_')}__{variant}.json"
    out.unlink(missing_ok=True)
    env = dict(os.environ)
    env["PATH"] = f"{Path(sys.executable).parent}{os.pathsep}{env['PATH']}"
    if variant == "speedups":
        env["PYTHONPATH"] = str(SPEEDUPS / "src")
    if build_only:
        env["BENCH_BUILD_ONLY"] = "1"
    t0 = time.perf_counter()
    proc = subprocess.Popen(
        [sys.executable, __file__, "child", case, variant, str(out)],
        env=env,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    status, peak = None, 0
    while proc.poll() is None:
        try:
            with open(f"/proc/{proc.pid}/status") as f:
                for line in f:
                    if line.startswith("VmRSS"):
                        rss = int(line.split()[1]) * 1024
                        peak = max(peak, rss)
                        if rss > RSS_CAP_GB * 2**30:
                            status = f"killed: RSS > {RSS_CAP_GB} GB"
        except FileNotFoundError:
            pass
        if time.perf_counter() - t0 > timeout:
            status = f"killed: timeout {timeout}s"
        if status:
            os.killpg(proc.pid, signal.SIGKILL)
            proc.wait()
            break
        time.sleep(0.2)
    stderr = proc.stderr.read() if proc.stderr else ""
    res = (
        json.loads(out.read_text())
        if out.exists()
        else {"case": case, "variant": variant}
    )
    if status:
        res["status"] = status
    elif proc.returncode != 0:
        res["status"] = "error"
        res["stderr_tail"] = stderr[-2000:]
    res["wall_total_s"] = time.perf_counter() - t0
    res["observed_peak_rss_mb"] = peak / 2**20
    out.write_text(json.dumps(res, indent=1, default=str))
    return res


SYN_NODES = [5, 50, 250]
SYN_HOURS = [168, 2190, 8760]


def matrix(variants: list[str], timeout: int, only: str | None = None):
    cases = ["national", "urban"]
    cases += [f"syn:{n}:{t}" for n in SYN_NODES for t in SYN_HOURS]
    cases += ["operate:10", "operate:31", "spores:10", "spores:31"]
    if only:
        cases = [c for c in cases if c.startswith(tuple(only.split(",")))]
    for variant in variants:
        dead: set = set()
        for case in cases:
            if case.startswith("syn:"):
                _, n, t = case.split(":")
                # skip anything at least as large as a size that already failed
                if any(int(n) >= dn and int(t) >= dt for dn, dt in dead):
                    print(f"SKIP {case} {variant} (larger size failed)", flush=True)
                    out = RESULTS / f"{case.replace(':', '_')}__{variant}.json"
                    out.write_text(
                        json.dumps(
                            {
                                "case": case,
                                "variant": variant,
                                "status": "skipped: a smaller size already exceeded the cap",
                            }
                        )
                    )
                    continue
            build_only = False
            if case.startswith("syn:"):
                _, n, t = case.split(":")
                # single-thread HiGHS needs > 20 min beyond this size: build only
                build_only = int(n) * int(t) > 5 * 8760 or int(n) >= 250
            res = run_one(case, variant, timeout, build_only)
            st = res.get("status")
            stages = res.get("timings", {}).get("stages", {})
            brief = " ".join(f"{k}={v['wall_s']:.1f}s" for k, v in stages.items())
            print(
                f"{st:10.40s} {case:14s} {variant:9s} {res['wall_total_s']:8.1f}s "
                f"peak={res['observed_peak_rss_mb']:.0f}MB {brief}",
                flush=True,
            )
            if not st.startswith("ok") and case.startswith("syn:"):
                _, n, t = case.split(":")
                dead.add((int(n), int(t)))


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    c = sub.add_parser("child")
    c.add_argument("case")
    c.add_argument("variant")
    c.add_argument("out", type=Path)
    o = sub.add_parser("one")
    o.add_argument("case")
    o.add_argument("variant")
    o.add_argument("--timeout", type=int, default=1200)
    m = sub.add_parser("matrix")
    m.add_argument("--variants", default="main,speedups,pyomo")
    m.add_argument("--timeout", type=int, default=1200)
    m.add_argument("--only", help="comma-separated case prefixes")
    a = ap.parse_args()
    if a.cmd == "child":
        child(a.case, a.variant, a.out)
    elif a.cmd == "one":
        print(json.dumps(run_one(a.case, a.variant, a.timeout), indent=1)[:3000])
    else:
        matrix(a.variants.split(","), a.timeout, a.only)


if __name__ == "__main__":
    main()
