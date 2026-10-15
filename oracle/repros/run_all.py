"""Run every ``bugNN_*.py`` repro and print its ``RESULT:`` line.

../calliope/.pixi/envs/dev/bin/python oracle/repros/run_all.py
"""

import subprocess
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent

for script in sorted(HERE.glob("bug*.py")):
    p = subprocess.run(
        [sys.executable, script.name], cwd=HERE, capture_output=True, text=True
    )
    lines = [ln for ln in p.stdout.splitlines() if ln.startswith("RESULT:")]
    line = lines[-1] if lines else f"RESULT: error: {p.stderr.strip()[-300:]}"
    print(f"{script.stem}: {line.removeprefix('RESULT: ')}", flush=True)
