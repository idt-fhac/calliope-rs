"""Pytest plugin: capture golden fixtures while upstream ``tests/math`` runs.

Usage (from ``../calliope``, see ``oracle/gen_math_lp.sh``)::

    GOLDEN_DIR=.../golden/math_lp PYTHONPATH=.../oracle \
        pytest -p pytest_golden_lp -n0 --no-cov tests/math/math_test.py

For every LP file the math tests build, this writes ``<GOLDEN_DIR>/<lp stem>/``:

- ``inputs.nc``: the model (inputs + attrs, incl. the applied math) as built;
- ``pyomo.lp``: the LP written by the test (Pyomo backend, verbose strings,
  trailing whitespace stripped), i.e. what upstream compares;
- ``reference.lp``: the committed upstream reference ``tests/math/lp_files/*.lp``;
- ``highs.lp``: the same problem built with ``backend: highs`` and ``to_lp``
  (absent if the HiGHS backend cannot build it, e.g. piecewise/SOS2);
- ``meta.json``: test node id, scenario, override dict, custom math dict,
  objective, init kwargs, and errors if any.
"""

from __future__ import annotations

import json
import os
import shutil
import traceback
from pathlib import Path

import pytest

GOLDEN_DIR = Path(os.environ.get("GOLDEN_DIR", "golden/math_lp")).absolute()
_last_build_args: dict = {}
_current_test: dict = {"nodeid": None}


def _jsonable(obj):
    if isinstance(obj, dict):
        return {str(k): _jsonable(v) for k, v in obj.items()}
    if isinstance(obj, list | tuple | set):
        return [_jsonable(v) for v in obj]
    if isinstance(obj, Path):
        return str(obj)
    if isinstance(obj, float | int | str | bool) or obj is None:
        return obj
    return repr(obj)


def _exc(err) -> str:
    return "".join(traceback.format_exception_only(type(err), err)).strip()


def pytest_runtest_setup(item):
    _current_test["nodeid"] = item.nodeid


def pytest_configure(config):
    from tests.common import util
    from tests.math import math_test

    orig_build_test_model = util.build_test_model
    orig_build_lp_file = math_test.build_lp_file

    def build_test_model(override_dict=None, scenario=None, *args, **kwargs):
        _last_build_args.clear()
        _last_build_args.update(
            {
                "override_dict": override_dict,
                "scenario": scenario,
                "args": list(args),
                "kwargs": {k: v for k, v in kwargs.items() if k != "math_dict"},
                "math_dict": kwargs.get("math_dict"),
            }
        )
        return orig_build_test_model(override_dict, scenario, *args, **kwargs)

    def build_lp_file(model, outfile, objective: str = "dummy_obj"):
        orig_build_lp_file(model, outfile, objective)
        outfile = Path(outfile)
        dest = GOLDEN_DIR / outfile.stem
        dest.mkdir(parents=True, exist_ok=True)
        shutil.copy(outfile, dest / "pyomo.lp")
        ref = Path(math_test.__file__).parent / "lp_files" / outfile.name
        if ref.exists():
            shutil.copy(ref, dest / "reference.lp")
        meta = {
            "test": _current_test["nodeid"],
            "lp_file": outfile.name,
            "objective": objective,
            **_jsonable(_last_build_args),
        }
        # Pyomo build mutated nothing in inputs; save them with attrs (math, config)
        model.to_netcdf(dest / "inputs.nc")
        meta["highs_lp_names"] = "verbose"
        try:
            model.build(force=True, backend="highs", objective=objective)
            try:
                model.backend.verbose_strings()
            except Exception as err:  # noqa: BLE001
                # upstream bug (bench/BUGS.md #13): HiGHS verbose_strings fails on
                # integer-typed dimensions. Rebuild and keep HiGHS default names.
                meta["highs_verbose_error"] = _exc(err)
                meta["highs_lp_names"] = "default"
                model.build(force=True, backend="highs", objective=objective)
            model.backend.to_lp(dest / "highs.lp")
        except Exception as err:  # noqa: BLE001
            meta["highs_error"] = _exc(err)
            meta.pop("highs_lp_names")
            (dest / "highs.lp").unlink(missing_ok=True)
        (dest / "meta.json").write_text(json.dumps(meta, indent=2, sort_keys=True))

    util.build_test_model = build_test_model
    math_test.build_lp_file = build_lp_file


@pytest.hookimpl(trylast=True)
def pytest_sessionfinish(session, exitstatus):
    print(f"\n[golden] math LP fixtures written to {GOLDEN_DIR}")
