"""Print the pydantic JSON schema of calliope.schemas.CalliopeAttrs.

Non-finite floats are written as the strings "nan", "inf" and "-inf" so the output is
valid JSON. Run with the Python of the Calliope pixi env.
"""

import json
import math

from calliope.schemas import CalliopeAttrs


def finite(obj):
    if isinstance(obj, float) and not math.isfinite(obj):
        return "nan" if math.isnan(obj) else ("inf" if obj > 0 else "-inf")
    if isinstance(obj, dict):
        return {k: finite(v) for k, v in obj.items()}
    if isinstance(obj, list):
        return [finite(v) for v in obj]
    return obj


print(json.dumps(finite(CalliopeAttrs.model_json_schema()), indent=1, sort_keys=True))
