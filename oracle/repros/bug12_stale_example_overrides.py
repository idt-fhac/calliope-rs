"""#12 (new) Stale overrides/scenarios in the national_scale example.

example_models/national_scale/scenarios.yaml: several shipped overrides and
scenarios no longer load with 0.7.0. Untested upstream (the example tests only use
a few of them).
"""

import subprocess
import sys

from _common import NATIONAL, result

CHILD = r"""
import sys, warnings
warnings.filterwarnings("ignore")
import calliope
calliope.set_log_verbosity("ERROR")
try:
    calliope.read_yaml(sys.argv[1], scenario=sys.argv[2],
                       override_dict={"config.build.backend": "highs"})
    print("OK")
except Exception as e:
    print(f"{type(e).__name__}: {str(e).strip().splitlines()[-1][:160]}")
"""
names = [
    "profiling",
    "time_resampling",
    "time_clustering",
    "spores",
    "operate",
    "check_feasibility",
    "reserve_margin",
    "run1",
    "run2",
    "run3",
    "run4",
    "cold_fusion",
    "minimize_emissions_costs",
    "maximize_utility_costs",
    "capacity_factor",
    "cold_fusion_with_production_share",
    "cold_fusion_with_capacity_share",
]
out = {}
for n in names:
    p = subprocess.run(
        [sys.executable, "-c", CHILD, str(NATIONAL), n], capture_output=True, text=True
    )
    out[n] = (p.stdout.strip().splitlines() or [p.stderr[-200:]])[-1]
bad = {k: v for k, v in out.items() if v != "OK"}
result(
    "confirmed" if bad else "not a bug",
    f"{len(bad)}/{len(names)} fail at load: {bad}; "
    "note check_feasibility loads but names an undefined objective (#14)",
)
