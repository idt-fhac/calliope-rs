"""#2 SPORES ``tracking_parameter`` treats ``false`` as tracked.

model.py:848-853 filters techs with ``inputs[tracking_parameter].notnull()``; a tech
with ``spores_tracker: false`` is not null, so it is scored. The docs say techs set
to false "will not be tracked" (docs/building_blocks/modes.md:145).
"""

from _common import QUIET_HIGHS, quiet, result, test_model

quiet()
m = test_model(
    "spores,var_costs,two_hours,simple_supply_spores_ready,investment_costs,"
    "spores_tech_tracking",
    {
        "data_definitions.spores_tracker": {
            "data": [True, False],
            "index": ["test_supply_elec", "test_demand_elec"],
            "dims": "techs",
        },
        "config.solve.spores.number": 1,
    },
)
m.build()
m.solve(solver_options=QUIET_HIGHS)
score = m.results.spores_score_cumulative.sel(spores=1)
tracker = m.inputs.spores_tracker
cap0 = m.results.flow_cap.sel(spores=0)
false_techs = [t for t in tracker.techs.values if tracker.sel(techs=t).item() == 0]
scored_false = {t: float(score.sel(techs=t).fillna(0).sum()) for t in false_techs}
deployed_false = {t: float(cap0.sel(techs=t).fillna(0).sum()) for t in false_techs}
detail = (
    f"spores_tracker={tracker.to_series().dropna().to_dict()} (dtype {tracker.dtype}), "
    f"baseline flow_cap of false-tracked techs={deployed_false}, "
    f"spores_score after SPORE 1 for false-tracked techs={scored_false}"
)
if any(v > 0 for v in scored_false.values()):
    result("confirmed", detail)
elif not any(v > 0 for v in deployed_false.values()):
    result("inconclusive (false tech not deployed in baseline)", detail)
else:
    result("not a bug", detail)
