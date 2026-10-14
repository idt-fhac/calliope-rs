"""#8 ``build.operate.horizon >= window`` is documented but not validated.

config_schema.py:141. With horizon < window, part of each window is never
optimised. Check that the config is accepted and what the results cover.
"""

from _common import QUIET_HIGHS, quiet, result, test_model

quiet()
m = test_model(
    "simple_supply,operate,var_costs,investment_costs",
    {"config.build.operate": {"window": "12h", "horizon": "6h"}},
)
detail = f"config accepted: window={m.config.build.operate.window}, horizon={m.config.build.operate.horizon}"
try:
    m.build()
    m.solve(solver_options=QUIET_HIGHS)
    n_in = m.inputs.sizes["timesteps"]
    n_out = m.results.sizes.get("timesteps", 0)
    detail += f"; solved: {n_in} input timesteps, {n_out} result timesteps"
    if "flow_out" in m.results:
        nan_ts = int(
            m.results.flow_out.isnull()
            .all([d for d in m.results.flow_out.dims if d != "timesteps"])
            .sum()
        )
        detail += f", {nan_ts} timesteps with all-NaN flow_out"
except Exception as err:  # noqa: BLE001
    detail += f"; build/solve raised {type(err).__name__}: {str(err)[:200]}"
result("confirmed", detail)
