"""Phase 9 benchmark matrix: calliope-rs against Python Calliope (main, feat/highs-speedups).

Same cases, solver options (``threads=1, random_seed=0``), caps (20 min, 28 GB) and
build-only rules as ``run_bench.py`` (Phase 0). Results go to ``bench/results/phase9/``
as ``<case>__<variant>__r<i>.json``; small cases are repeated 3 times.

Variants:
  rust      ``target/release/calliope bench --json`` (preprocess, build, solve incl.
            results, NetCDF write); build-only sizes use the release ``build_timing``
            example (preprocess + build).
  main      ``run_bench.run_one(.., "main")``
  speedups  ``run_bench.run_one(.., "speedups")``

Usage (sequentially, nothing else running)::

    ../calliope/.pixi/envs/dev/bin/python bench/run_phase9.py --variants rust,main,speedups
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
sys.path.insert(0, str(HERE))
import run_bench

REPO = HERE.parent
OUT = HERE / "results" / "phase9"
CLI = REPO / "target/release/calliope"
BUILD_TIMING = REPO / "target/release/examples/build_timing"

CASES = (
    ["national", "urban"]
    + [f"syn:{n}:{t}" for n in (5, 50, 250) for t in (168, 2190, 8760)]
    + ["operate:10", "operate:31", "spores:10", "spores:31"]
)
SMALL = {"syn:5:168", "syn:5:2190", "operate:10", "spores:10", "spores:31"}
RUST_SMALL = SMALL | {"operate:31"}
# Python sizes known to exceed 28 GB in Phase 0 (BASELINE.md); not re-run.
PY_SKIP = {"syn:250:2190", "syn:250:8760"}


def build_only(case: str) -> bool:
    if not case.startswith("syn:"):
        return False
    _, n, t = case.split(":")
    return int(n) * int(t) > 5 * 8760 or int(n) >= 250


def rust_args(case: str) -> list[str]:
    so = {"config.solve.solver_options": run_bench.SOLVER_OPTIONS}
    if case in ("national", "urban"):
        path = run_bench.NATIONAL if case == "national" else run_bench.URBAN
        return [
            str(path),
            "--override_dict",
            json.dumps({**so, "config.init.subset.timesteps": None}),
        ]
    if case.startswith("syn:"):
        from synthetic import make_model

        _, n, t = case.split(":")
        path = make_model(
            int(n), int(t), run_bench.WORK / f"syn_{n}_{t}", run_bench.CALLIOPE
        )
        return [str(path), "--override_dict", json.dumps(so)]
    kind, d = case.split(":")
    sub = {"config.init.subset.timesteps": run_bench._days(int(d))}
    return [
        str(run_bench.NATIONAL),
        "--scenario",
        kind,
        "--override_dict",
        json.dumps({**so, **sub}),
    ]


def sample(cmd: list[str], timeout: int) -> tuple[str, str, int, float, float, str]:
    """Run ``cmd``, sampling RSS from outside. Returns stdout, stderr, rc, wall, peak MB, status."""
    t0 = time.perf_counter()
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    peak, status = 0, None
    while proc.poll() is None:
        try:
            with open(f"/proc/{proc.pid}/status") as f:
                for line in f:
                    if line.startswith("VmRSS"):
                        rss = int(line.split()[1]) * 1024
                        peak = max(peak, rss)
                        if rss > run_bench.RSS_CAP_GB * 2**30:
                            status = f"killed: RSS > {run_bench.RSS_CAP_GB} GB"
        except FileNotFoundError:
            pass
        if time.perf_counter() - t0 > timeout:
            status = f"killed: timeout {timeout}s"
        if status:
            os.killpg(proc.pid, signal.SIGKILL)
            break
        time.sleep(0.05)
    out, err = proc.communicate()
    return out, err, proc.returncode, time.perf_counter() - t0, peak / 2**20, status


def run_rust(case: str, timeout: int) -> dict:
    args = rust_args(case)  # generates synthetic models before timing
    res: dict = {"case": case, "variant": "rust"}
    if build_only(case):
        cmd = [str(BUILD_TIMING), args[0]]
        res["status"] = "ok-build-only"
    else:
        cmd = [str(CLI), "bench", "--json", *args]
        res["status"] = "ok"
    res["cmd"] = cmd
    out, err, rc, wall, peak, status = sample(cmd, timeout)
    res["wall_total_s"] = wall
    res["observed_peak_rss_mb"] = peak
    if status:
        res["status"] = status
    elif rc != 0:
        res["status"] = "error"
        res["stderr_tail"] = err[-2000:]
    elif build_only(case):
        # "timesteps T  preprocess P s  build B s  cols C  rows R  nnz Z" + "peak RSS: K kB"
        toks = out.split()
        val = lambda k: toks[toks.index(k) + 1]
        res["timings"] = {
            "stages": {
                "preprocess": {"wall_s": float(val("preprocess"))},
                "build": {"wall_s": float(val("build"))},
            },
            "total_wall_s": float(val("preprocess")) + float(val("build")),
        }
        res["lp"] = {k: int(val(k)) for k in ("cols", "rows", "nnz")}
        res["sizes"] = {"timesteps": int(val("timesteps"))}
        res["process_peak_rss_mb"] = int(out.split("peak RSS:")[1].split()[0]) / 1024
    else:
        res["timings"] = json.loads(out)
    return res


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--variants", default="rust,main,speedups")
    ap.add_argument("--only", help="comma-separated case prefixes")
    ap.add_argument("--timeout", type=int, default=1200)
    a = ap.parse_args()
    OUT.mkdir(parents=True, exist_ok=True)
    run_bench.RESULTS = OUT
    cases = CASES
    if a.only:
        cases = [c for c in cases if c.startswith(tuple(a.only.split(",")))]
    for variant in a.variants.split(","):
        for case in cases:
            stem = f"{case.replace(':', '_')}__{variant}"
            if variant != "rust" and case in PY_SKIP:
                (OUT / f"{stem}__r1.json").write_text(
                    json.dumps(
                        {
                            "case": case,
                            "variant": variant,
                            "status": "skipped: exceeded 28 GB in Phase 0 (BASELINE.md)",
                        }
                    )
                )
                continue
            reps = 3 if case in (RUST_SMALL if variant == "rust" else SMALL) else 1
            for i in range(1, reps + 1):
                if variant == "rust":
                    res = run_rust(case, a.timeout)
                else:
                    res = run_bench.run_one(case, variant, a.timeout, build_only(case))
                    (OUT / f"{stem}.json").unlink(missing_ok=True)
                res["rep"] = i
                (OUT / f"{stem}__r{i}.json").write_text(
                    json.dumps(res, indent=1, default=str)
                )
                st = res.get("timings", {}).get("stages", {})
                brief = " ".join(f"{k}={v['wall_s']:.2f}s" for k, v in st.items())
                print(
                    f"{res.get('status', '?')[:30]:30s} {case:14s} {variant:9s} r{i} "
                    f"{res['wall_total_s']:8.1f}s peak={res['observed_peak_rss_mb']:.0f}MB {brief}",
                    flush=True,
                )


if __name__ == "__main__":
    main()
