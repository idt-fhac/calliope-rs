"""#14 (new) ``build.objective`` naming an undefined objective is not validated.

The national_scale ``check_feasibility`` override sets
``build.objective: check_feasibility``, which no 0.7 math defines. The HiGHS backend
never calls ``setObjective`` (highs_backend_model.py:139) and silently solves a
zero-objective feasibility problem; the Pyomo backend ends up with no active
objective and its LP writer raises ``IndexError``.
"""

import calliope
from _common import NATIONAL, QUIET_HIGHS, quiet, result

quiet()
od = {"config.init.subset.timesteps": ["2005-01-01", "2005-01-01"]}
m = calliope.read_yaml(
    NATIONAL,
    scenario="check_feasibility",
    override_dict={**od, "config.build.backend": "highs"},
)
defined = list(m.math.build.objectives.root)
m.build()
m.solve(solver_options=QUIET_HIGHS)
h_obj = m.backend._instance.getInfo().objective_function_value
highs = (
    f"highs: termination={m.runtime.termination_condition}, solver objective={h_obj}, "
    f"results objectives={[k for k in m.results.data_vars if k in defined]}"
)

mp = calliope.read_yaml(
    NATIONAL,
    scenario="check_feasibility",
    override_dict={**od, "config.build.backend": "pyomo"},
)
mp.build()
try:
    mp.backend.to_lp("/dev/null")
    pyomo = "pyomo: LP written"
except Exception as err:  # noqa: BLE001
    pyomo = f"pyomo to_lp: {type(err).__name__}: {err}"
result(
    "confirmed",
    f"build.objective={m.config.build.objective!r}, defined objectives={defined}; "
    f"{highs}; {pyomo}",
)
