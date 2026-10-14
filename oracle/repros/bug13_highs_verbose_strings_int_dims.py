"""#13 (new) HiGHS ``verbose_strings`` fails on integer-typed dimensions.

highs_backend_model.py:295 calls ``coords.str.replace(" ", "_")`` on every dim of a
variable/constraint. For an integer dimension (``clusters``, ``pieces``,
``breakpoints`` ...) xarray casts the replacement to int and raises. Hit by 12 of
the upstream math-test LP builds (inter-cluster storage, piecewise, ...).
"""

from _common import QUIET_HIGHS, quiet, result, test_model

quiet()
m = test_model(
    "simple_storage,inter_cluster_storage",
    {"config.init.extra_math": ["storage_inter_cluster"]},
)
m.build()
int_dims = [d for d in m.inputs.dims if m.inputs[d].dtype.kind in "iu"]
try:
    m.backend.verbose_strings()
except ValueError as err:
    result("confirmed", f"integer dims {int_dims}; ValueError: {err}")
else:
    m.solve(solver_options=QUIET_HIGHS)
    result("not a bug", f"integer dims {int_dims}; verbose_strings ran")
