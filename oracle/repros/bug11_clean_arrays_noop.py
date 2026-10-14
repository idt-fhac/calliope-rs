"""#11 ``BackendModel._clean_arrays`` frees nothing.

backend_model.py:673-675 does ``del args``, which only drops the function's own
tuple; the caller still holds every array, so nothing can be collected.
"""

import gc
import weakref

import numpy as np
import xarray as xr
from _common import result
from calliope.backend.backend_model import BackendModel


def caller():
    big = xr.DataArray(np.ones((1000, 1000)))
    ref = weakref.ref(big)
    BackendModel._clean_arrays(big)
    gc.collect()
    alive_inside = ref() is not None
    return ref, alive_inside


ref, alive_inside = caller()
gc.collect()
result(
    "confirmed (no-op)" if alive_inside else "not a bug",
    f"array alive in caller after _clean_arrays: {alive_inside}; "
    f"freed only when the caller's frame exits: {ref() is None}",
)
