"""#4 ``update_input`` broadcast message: missing comma before ``"info"``.

backend_model.py:1251-1252 concatenates ``"info"`` into the message string, so the
record is logged at the default level (debug) and ends with "...dimension(s).info".
"""

import logging

from _common import quiet, result, test_model

quiet()
m = test_model("simple_supply,two_hours,investment_costs")
m.build()

records = []


class H(logging.Handler):
    def emit(self, record):
        records.append(record)


lg = logging.getLogger("calliope.backend.backend_model")
lg.setLevel(logging.DEBUG)
lg.addHandler(H())
# pick a parameter with >= 2 dims and update it with one dim dropped
name = next(
    n
    for n in m.backend.parameters
    if n in m.inputs and m.backend.get_parameter(n).ndim >= 2
)
param = m.inputs[name]
new = param.isel({param.dims[0]: 0}, drop=True)
m.backend.update_input(name, new)
hits = [r for r in records if "broadcast" in r.getMessage()]
if not hits:
    result("inconclusive", "no broadcast message logged")
else:
    r = hits[0]
    msg = r.getMessage()
    status = (
        "confirmed"
        if r.levelno == logging.DEBUG and msg.rstrip().endswith("info")
        else "not a bug"
    )
    result(status, f"param={name}, level={r.levelname}, message tail={msg[-60:]!r}")
