"""#9 ``score_threshold_factor``: docs vs code.

config_schema.py:209-214 documents the threshold relative to the tech's own previous
capacity ("previous capacity 100, factor 0.1 -> only capacities above 10 ...").
model.py:757-759 compares the previous capacity against
``factor * previous_cap.max(nodes, techs)``, i.e. the largest capacity in the system.
A deployed tech that is small relative to the largest one is never scored.
"""

import calliope
from _common import NATIONAL, QUIET_HIGHS, quiet, result

quiet()
factor = 0.1
m = calliope.read_yaml(
    NATIONAL,
    scenario="spores",
    override_dict={
        "config.build.backend": "highs",
        "config.init.subset.timesteps": ["2005-01-01", "2005-01-02"],
        "config.solve.spores": {
            "number": 1,
            "scoring_algorithm": "integer",
            "score_threshold_factor": factor,
        },
    },
)
m.build()
m.solve(solver_options=QUIET_HIGHS)
tracked = m.inputs.spores_tracker.notnull() & m.inputs.active
cap0 = m.results.flow_cap.sel(spores=0).where(tracked)
score1 = m.results.spores_score_cumulative.sel(spores=1)
system_max = cap0.max(["nodes", "techs"])
rows = []
for (n, t, c), v in cap0.to_series().dropna().items():
    if v <= 0:
        continue
    s = float(score1.sel(nodes=n, techs=t, carriers=c))
    rows.append((n, t, round(v, 1), s))
unscored = [r for r in rows if r[3] == 0]
detail = (
    f"system max previous cap={float(system_max.max()):.1f}, threshold="
    f"{factor * float(system_max.max()):.1f}; deployed tracked (node, tech, cap, score): "
    f"{rows}; deployed but unscored: {unscored}"
)
result("confirmed (docs/code mismatch)" if unscored else "not observable here", detail)
