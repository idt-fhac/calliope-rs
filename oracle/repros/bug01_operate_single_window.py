"""#1 Operate mode where one window covers the whole time span.

model.py:513-557: the loop over ``windowsteps[1:]`` never runs, so ``windowstep``
is unbound at ``results_list.append(... slice(windowstep, None))``.
"""

from _common import QUIET_HIGHS, quiet, result, test_model

quiet()
# test model operate scenario: 2005-01-01..2005-01-02 (48 h); window = horizon = 48h
m = test_model(
    "simple_supply,operate,var_costs,investment_costs",
    {"config.build.operate": {"window": "48h", "horizon": "48h"}},
)
m.build()
try:
    m.solve(solver_options=QUIET_HIGHS)
except UnboundLocalError as err:
    result("confirmed", f"UnboundLocalError: {err}")
except Exception as err:  # noqa: BLE001
    result("confirmed (other error)", f"{type(err).__name__}: {err}")
else:
    result("not a bug", f"solved, {m.results.sizes['timesteps']} timesteps in results")
