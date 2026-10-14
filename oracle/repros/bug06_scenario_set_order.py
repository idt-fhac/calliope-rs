"""#6 Scenario overrides are collected in a ``set``; self-referencing scenarios.

model_definition.py:172,178.
(a) ``applied_overrides`` order (and the union order of the override dicts,
    hence coordinate order) depends on ``PYTHONHASHSEED``.
(b) A scenario that references itself recurses without a cycle check.
"""

import json
import os
import subprocess
import sys

from _common import TEST_MODEL, result

CHILD = r"""
import json, logging, warnings, sys
warnings.filterwarnings("ignore")
import calliope
calliope.set_log_verbosity("ERROR")
from calliope.io import read_rich_yaml
m = calliope.read_yaml(sys.argv[1], scenario=sys.argv[2],
                       override_dict={"config.build.backend": "highs"})
print(json.dumps({"applied": m.runtime.applied_overrides,
                  "techs": [str(t) for t in m.inputs.techs.values]}))
"""

scenario = (
    "simple_supply,two_hours,investment_costs,var_costs,shadow_prices,demand_elec_max"
)
seen = {}
for seed in range(8):
    env = dict(os.environ, PYTHONHASHSEED=str(seed))
    out = subprocess.run(
        [sys.executable, "-c", CHILD, str(TEST_MODEL), scenario],
        env=env,
        capture_output=True,
        text=True,
    )
    if out.returncode != 0:
        print(out.stderr[-1500:])
        continue
    d = json.loads(out.stdout.strip().splitlines()[-1])
    seen.setdefault(d["applied"], set()).add(tuple(d["techs"]))
n_orders = len(seen)
n_tech_orders = len({t for ts in seen.values() for t in ts})
detail_a = (
    f"8 hash seeds -> {n_orders} distinct applied_overrides orders, "
    f"{n_tech_orders} distinct techs coordinate orders; e.g. {list(seen)[:2]}"
)

# (b) self-reference
CHILD_B = r"""
import sys, warnings
warnings.filterwarnings("ignore")
import calliope
from calliope.io import read_rich_yaml
calliope.set_log_verbosity("ERROR")
d = read_rich_yaml('''
config.init.name: repro
scenarios:
  loop: [loop]
overrides: {}
nodes: {}
techs: {}
''')
try:
    calliope.read_dict(d, scenario="loop")
except RecursionError as e:
    print("RecursionError")
except Exception as e:
    print(type(e).__name__, e)
"""
out = subprocess.run([sys.executable, "-c", CHILD_B], capture_output=True, text=True)
detail_b = f"self-referencing scenario -> {out.stdout.strip() or out.stderr[-200:]}"
status = (
    "confirmed"
    if n_orders > 1 and "RecursionError" in out.stdout
    else (
        "partly confirmed"
        if n_orders > 1 or "RecursionError" in out.stdout
        else "not a bug"
    )
)
result(status, f"(a) {detail_a}. (b) {detail_b}")
