"""#7 HiGHS backend returns empty results for any non-optimal status.

highs_backend_model.py:268-274 loads results only for ``kOptimal``. A MILP stopped
by a solution/time/node limit with a feasible incumbent gets an empty dataset,
although SPORES saving accepts ``"feasible"`` (model.py:705).
"""

import calliope
from _common import CALLIOPE_DIR, quiet, result

quiet()
m = calliope.read_yaml(
    CALLIOPE_DIR / "src/calliope/example_models/urban_scale/model.yaml",
    scenario="milp",
    override_dict={"config.build.backend": "highs"},
)
m.build()
# stop at the first improving MILP solution: feasible incumbent, not proven optimal
m.solve(
    solver_options={"output_flag": False, "threads": 1, "mip_max_improving_sols": 1}
)
h = m.backend._instance
info = h.getInfo()
has_incumbent = info.primal_solution_status == 2  # kSolutionStatusFeasible
detail = (
    f"termination={m.runtime.termination_condition!r}, "
    f"HiGHS primal_solution_status={info.primal_solution_status} "
    f"(2=feasible), mip objective={info.objective_function_value:.6g}, "
    f"results data_vars={len(m.results.data_vars)}"
)
if has_incumbent and not m.results.data_vars:
    result("confirmed", detail)
else:
    result("not a bug" if m.results.data_vars else "inconclusive", detail)
