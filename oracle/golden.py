"""Generate golden files for one or all fixtures (see ``fixtures.py``).

Run with the Calliope pixi env from the calliope-rust repo root::

    ../calliope/.pixi/envs/dev/bin/python oracle/golden.py --all
    ../calliope/.pixi/envs/dev/bin/python oracle/golden.py national_scale urban_scale

Per fixture, ``golden/<name>/`` gets:

- ``inputs.nc``  model after preprocess (``Model.to_netcdf`` before build)
- ``highs.lp``   ``backend: highs`` + ``verbose_strings`` + ``to_lp`` (HiGHS writer)
- ``pyomo.lp``   ``backend: pyomo`` + ``verbose_strings`` + ``to_lp`` (as tests/math)
- ``results.nc`` model after solve (inputs + results + attrs)
- ``meta.json``  fixture definition, objective values, termination, stage timings,
                 peak RSS, per-iteration events, package versions

Each fixture runs in its own subprocess (clean RSS, timeout).
"""

from __future__ import annotations

import argparse
import copy
import json
import os
import subprocess
import sys
import time
import traceback
from pathlib import Path

HERE = Path(__file__).resolve().parent
REPO = HERE.parent
CALLIOPE_DIR = Path(os.environ.get("CALLIOPE_DIR", REPO.parent / "calliope")).resolve()
DEFAULT_OUT = REPO / "golden"
sys.path.insert(0, str(HERE))
# Solver binaries (cbc) live next to the env's python; pixi run would add this.
os.environ["PATH"] = f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"


def versions() -> dict:
    import calliope
    import highspy
    import numpy
    import pandas
    import pyomo
    import pyparsing
    import xarray

    h = highspy.Highs()
    return {
        "python": sys.version.split()[0],
        "calliope": calliope.__version__,
        "calliope_git": subprocess.run(
            ["git", "-C", str(CALLIOPE_DIR), "rev-parse", "--short", "HEAD"],
            capture_output=True,
            text=True,
        ).stdout.strip(),
        "xarray": xarray.__version__,
        "pandas": pandas.__version__,
        "numpy": numpy.__version__,
        "pyparsing": pyparsing.__version__,
        "pyomo": pyomo.__version__,
        "highs": h.version(),
    }


def _scalar_or_list(da):
    vals = da.values
    if vals.ndim == 0:
        return float(vals)
    return [float(v) for v in vals.ravel()]


def run_fixture(name: str, out_root: Path, skip_pyomo_lp: bool = False) -> dict:
    import calliope
    import numpy as np
    from calliope.io import read_rich_yaml
    from fixtures import BY_NAME, HIGHS_OPTIONS
    from instrument import Recorder, install

    calliope.set_log_verbosity("WARNING", include_solver_output=False)
    fx = BY_NAME[name]
    out = out_root / name
    out.mkdir(parents=True, exist_ok=True)
    for f in ("inputs.nc", "results.nc", "highs.lp", "pyomo.lp", "meta.json"):
        (out / f).unlink(missing_ok=True)

    if fx.numpy_seed is not None:
        np.random.seed(fx.numpy_seed)

    override = copy.deepcopy(fx.override_dict)
    if fx.yaml_override:
        override.update(read_rich_yaml(fx.yaml_override).as_dict())
    override["config.build.backend"] = fx.backend
    if fx.backend == "pyomo":
        override["config.solve.solver"] = fx.solver
    math_dict = (
        {"piecewise_math": read_rich_yaml(fx.yaml_math).as_dict()}
        if fx.yaml_math
        else None
    )
    solver_options = (
        {**HIGHS_OPTIONS, **fx.solver_options}
        if fx.backend == "highs" or fx.solver == "highs"
        else dict(fx.solver_options)
    )

    meta: dict = {
        "fixture": {
            "name": fx.name,
            "model": fx.model,
            "scenario": fx.scenario,
            "override_dict": override,
            "math_dict": math_dict,
            "init_kwargs": fx.init_kwargs,
            "solve_kwargs": {"solver_options": solver_options, **fx.solve_kwargs},
            "backend": fx.backend,
            "solver": fx.solver if fx.backend == "pyomo" else "highs",
            "numpy_seed": fx.numpy_seed,
            "notes": fx.notes,
        },
        "versions": versions(),
        "errors": {},
    }

    rec = Recorder()
    install(rec)
    model_path = CALLIOPE_DIR / fx.model
    try:
        with rec.stage("preprocess"):
            model = calliope.read_yaml(
                model_path,
                scenario=fx.scenario,
                override_dict=override,
                math_dict=math_dict,
                **fx.init_kwargs,
            )
    except Exception as err:
        if not fx.expect_error:
            raise
        meta["expected_error"] = {"type": type(err).__name__, "message": str(err)}
        (out / "meta.json").write_text(json.dumps(meta, indent=2, default=str))
        return meta
    model.to_netcdf(out / "inputs.nc")

    with rec.stage("build"):
        model.build()
    meta["lp_names"] = "verbose"
    try:
        model.backend.verbose_strings()
    except Exception as err:  # noqa: BLE001
        # bench/BUGS.md #13: HiGHS verbose_strings fails on integer dimensions;
        # the names are then partly renamed, so rebuild outside the timed stage.
        meta["errors"]["verbose_strings"] = repr(err)
        meta["lp_names"] = "default"
        model.build(force=True)
    try:
        model.backend.to_lp(out / f"{fx.backend}.lp")
    except Exception as err:  # noqa: BLE001
        meta["errors"]["to_lp"] = repr(err)

    with rec.stage("solve"):
        model.solve(solver_options=solver_options, **fx.solve_kwargs)
    with rec.stage("write"):
        model.to_netcdf(out / "results.nc")

    results = model.results
    objectives = (
        {
            k: _scalar_or_list(results[k])
            for k in model.math.build.objectives.root.keys()
            if k in results
        }
        if hasattr(model.math.build.objectives, "root")
        else {}
    )
    if not objectives:
        objectives = {
            k: _scalar_or_list(results[k])
            for k in results.data_vars
            if results[k].attrs.get("obj_type") == "objectives"
        }
    meta["objective_name"] = model.config.build.objective
    if fx.backend == "highs":
        # value HiGHS optimised (last solve for operate/SPORES); differs from the
        # results' objectives if build.objective is undefined (BUGS.md #14)
        meta["solver_objective_last"] = (
            model.backend._instance.getInfo().objective_function_value
        )
    meta["objectives"] = objectives
    meta["termination_condition"] = model.runtime.termination_condition
    meta["timings"] = rec.summary()
    meta["runtime_timings"] = dict(model.runtime.timings)
    meta["sizes"] = {
        "dims": {k: int(v) for k, v in model.inputs.sizes.items()},
        "n_results_vars": len(results.data_vars),
    }

    if fx.pyomo_lp and fx.backend != "pyomo" and not skip_pyomo_lp:
        try:
            t = time.perf_counter()
            model.build(force=True, backend="pyomo")
            meta["timings"]["pyomo_build_s"] = time.perf_counter() - t
            model.backend.verbose_strings()
            model.backend.to_lp(out / "pyomo.lp")
        except Exception as err:  # noqa: BLE001
            meta["errors"]["pyomo_lp"] = "".join(
                traceback.format_exception_only(type(err), err)
            ).strip()

    (out / "meta.json").write_text(json.dumps(meta, indent=2, default=str))
    return meta


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("fixtures", nargs="*")
    ap.add_argument("--all", action="store_true")
    ap.add_argument("--out", type=Path, default=DEFAULT_OUT)
    ap.add_argument("--timeout", type=int, default=1200)
    ap.add_argument("--inproc", action="store_true", help="run in this process")
    ap.add_argument("--skip-pyomo-lp", action="store_true")
    args = ap.parse_args()

    from fixtures import FIXTURES

    names = [f.name for f in FIXTURES] if args.all else args.fixtures
    if args.inproc:
        for n in names:
            run_fixture(n, args.out, args.skip_pyomo_lp)
        return

    failed = []
    for n in names:
        t = time.perf_counter()
        cmd = [sys.executable, __file__, "--inproc", "--out", str(args.out), n]
        if args.skip_pyomo_lp:
            cmd.append("--skip-pyomo-lp")
        try:
            proc = subprocess.run(
                cmd, capture_output=True, text=True, timeout=args.timeout
            )
            ok = proc.returncode == 0
            err = proc.stderr[-3000:]
        except subprocess.TimeoutExpired:
            ok, err = False, f"timeout after {args.timeout}s"
        dt = time.perf_counter() - t
        print(f"{'OK  ' if ok else 'FAIL'} {n:70s} {dt:7.1f}s", flush=True)
        if not ok:
            failed.append(n)
            (args.out / n).mkdir(parents=True, exist_ok=True)
            (args.out / n / "error.txt").write_text(err)
        else:
            (args.out / n / "error.txt").unlink(missing_ok=True)
    if failed:
        print("failed:", ", ".join(failed))
        sys.exit(1)


if __name__ == "__main__":
    main()
