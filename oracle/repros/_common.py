"""Shared helpers for the bug repros. Run each repro with the Calliope pixi env:

    ../calliope/.pixi/envs/dev/bin/python oracle/repros/bug01_operate_single_window.py

Each script prints one ``RESULT:`` line: ``confirmed``, ``not a bug`` or
``fixed upstream``, followed by the evidence.
"""

from __future__ import annotations

import logging
import os
import sys
import warnings
from pathlib import Path

CALLIOPE_DIR = Path(
    os.environ.get("CALLIOPE_DIR", Path(__file__).resolve().parents[3] / "calliope")
).resolve()
TEST_MODEL = CALLIOPE_DIR / "tests/common/test_model/model.yaml"
NATIONAL = CALLIOPE_DIR / "src/calliope/example_models/national_scale/model.yaml"
os.environ["PATH"] = f"{Path(sys.executable).parent}{os.pathsep}{os.environ['PATH']}"
warnings.filterwarnings("ignore")

QUIET_HIGHS = {"output_flag": False, "threads": 1}


def quiet():
    import calliope

    calliope.set_log_verbosity("ERROR", include_solver_output=False)
    logging.getLogger("py.warnings").setLevel(logging.ERROR)


def test_model(scenario=None, override_dict=None, **kwargs):
    import calliope

    od = {"config.build.backend": "highs"}
    od.update(override_dict or {})
    return calliope.read_yaml(TEST_MODEL, scenario=scenario, override_dict=od, **kwargs)


def result(status: str, detail: str):
    print(f"RESULT: {status}: {detail}")
