"""#10 Documentation drift.

(a) docs/how_to/backend_interface.md:23 uses ``model.backend.update_param``.
(b) docs/building_blocks/modes.md:78-79 uses ``operate_horizon``/``operate_window``.
(c) CHANGELOG.md:56 names ``config.solve.postprocess_active``.
(d) ``sum_next_n`` LaTeX shows t..t+N (N+1 terms), the array sums N terms.
"""

from types import SimpleNamespace

import numpy as np
import xarray as xr
from _common import CALLIOPE_DIR, quiet, result, test_model
from calliope.backend.helper_functions import SumNextN

quiet()
findings = []

m = test_model("simple_supply,two_hours,investment_costs")
m.build()
findings.append(f"(a) backend has update_param: {hasattr(m.backend, 'update_param')}")

for label, od in (
    ("(b) build.operate_horizon", {"config.build.operate_horizon": "48h"}),
    ("(c) solve.postprocess_active", {"config.solve.postprocess_active": False}),
):
    try:
        test_model("simple_supply,two_hours", od)
        findings.append(f"{label}: accepted")
    except Exception as err:  # noqa: BLE001
        findings.append(f"{label}: rejected ({type(err).__name__})")

ts = xr.DataArray(
    np.arange(6), dims="timesteps", coords={"timesteps": np.arange(6)}, name="timesteps"
)
arr = xr.DataArray(np.ones(6), dims="timesteps", coords=ts.coords, name="foo")
summed = SumNextN.as_array(None, arr, ts, 2)
fn = object.__new__(SumNextN)
fn._attrs = SimpleNamespace(
    math=SimpleNamespace(dimensions={"timesteps": SimpleNamespace(iterator="timestep")})
)
latex = fn.as_math_string(r"\textbf{foo}_\text{timestep}", "timesteps", 2)
findings.append(
    f"(d) N=2: array sums {int(summed.isel(timesteps=0))} terms at t=0; LaTeX: {latex}"
)

doc_hits = []
for rel, needle in (
    ("docs/how_to/backend_interface.md", "update_param"),
    ("docs/building_blocks/modes.md", "operate_horizon"),
    ("CHANGELOG.md", "postprocess_active"),
):
    text = (CALLIOPE_DIR / rel).read_text()
    doc_hits.append(f"{rel} mentions {needle}: {needle in text}")

result("confirmed", "; ".join(findings + doc_hits))
