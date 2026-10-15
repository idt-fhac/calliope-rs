"""xarray oracle for crates/calliope-array/tests/xarray_oracle.rs.

Reads generated cases (JSON) from argv[1], applies each operation with xarray and writes the
cases with their expected results to argv[2]. Run with the Calliope pixi env's Python.
"""

import json
import sys

import numpy as np
import pandas as pd
import xarray as xr


def to_xr(spec):
    coords = {}
    for d in spec["dims"]:
        c = spec["coords"][d]
        if c["dtype"] == "datetime":
            coords[d] = pd.to_datetime(np.array(c["labels"], dtype="int64"), unit="ns")
        else:
            coords[d] = c["labels"]
    shape = [len(spec["coords"][d]["labels"]) for d in spec["dims"]]
    if spec["dtype"] == "bool":
        data = np.array(spec["values"], dtype=bool).reshape(shape)
    elif spec["dtype"] == "str":
        data = np.array([np.nan if v is None else v for v in spec["values"]], dtype=object).reshape(shape)
    else:
        data = np.array([np.nan if v is None else v for v in spec["values"]], dtype=float).reshape(shape)
    return xr.DataArray(data, coords=coords, dims=spec["dims"])


def from_xr(da):
    out = {"dims": list(da.dims), "coords": {}, "values": []}
    for d in da.dims:
        idx = da.indexes[d]
        if isinstance(idx, pd.DatetimeIndex):
            out["coords"][d] = {"dtype": "datetime", "labels": [int(x) for x in idx.asi8]}
        elif idx.dtype.kind in "iu":
            out["coords"][d] = {"dtype": "int", "labels": [int(x) for x in idx]}
        else:
            out["coords"][d] = {"dtype": "str", "labels": [str(x) for x in idx]}
    vals = da.values.ravel()
    if da.dtype == bool:
        out["dtype"] = "bool"
        out["values"] = [bool(v) for v in vals]
    else:
        out["dtype"] = "float"
        out["values"] = [None if pd.isna(v) else float(v) for v in vals]
    return out


def conv(case, v):
    """Datetime labels arrive as integer nanoseconds."""
    dim = case["params"].get("dim")
    if v is None or dim is None:
        return v
    if case["inputs"][0]["coords"][dim]["dtype"] == "datetime":
        return pd.Timestamp(v, unit="ns")
    return v


def apply(case):
    p = case["params"]
    xs = [to_xr(a) for a in case["inputs"]]
    op = case["op"]
    if op == "add":
        return xs[0] + xs[1]
    if op == "mul_outer":
        a, b = xr.align(xs[0], xs[1], join="outer")
        return a * b
    if op == "fillna_scalar":
        return xs[0].fillna(p["value"])
    if op == "fillna_array":
        return xs[0].fillna(xs[1])
    if op == "where":
        return xs[0].where(xs[1])
    if op == "where_other":
        return xs[0].where(xs[1], p["value"])
    if op == "sum":
        return xs[0].sum(p["dims"], min_count=p["min_count"], skipna=True)
    if op == "mean":
        return xs[0].mean(p["dims"], skipna=True)
    if op == "any":
        return xs[0].any(p["dims"])
    if op == "roll":
        return xs[0].roll({p["dim"]: p["shift"]})
    if op == "transpose":
        return xs[0].transpose(*p["order"])
    if op == "sel_list":
        return xs[0].sel({p["dim"]: [conv(case, v) for v in p["labels"]]})
    if op == "sel_slice":
        return xs[0].sel({p["dim"]: slice(conv(case, p["start"]), conv(case, p["end"]))})
    if op == "isin":
        labels = p["labels"]
        if case["inputs"][0]["coords"][p["dim"]]["dtype"] == "datetime":
            # A list of Timestamps never matches in `isin`; a DatetimeIndex does.
            labels = pd.to_datetime(labels, unit="ns")
        return xs[0][p["dim"]].isin(labels)
    if op == "group_sum":
        array, groupby = xs[0], xs[1]
        grouped = {}
        gdims = groupby.dims
        groups = array.stack(_stacked=gdims).groupby(groupby.stack(_stacked=gdims))
        for name, sub in groups:
            grouped[name] = sub.sum("_stacked", min_count=1, skipna=True)
        return xr.concat(grouped.values(), dim=pd.Index(grouped.keys(), name="groups"))
    if op == "resample":
        r = xs[0].resample({p["dim"]: f"{p['hours']}h"}, skipna=True)
        if p["method"] == "sum":
            return r.sum(min_count=1)
        return getattr(r, p["method"])()
    if op == "concat":
        return xr.concat(xs, dim=pd.Index(p["labels"], name="new"), join="outer")
    raise ValueError(op)


cases = json.load(open(sys.argv[1]))
for case in cases:
    try:
        case["expected"] = from_xr(apply(case))
    except Exception as e:  # noqa: BLE001
        case["expected"] = {"error": f"{type(e).__name__}: {e}"}
json.dump(cases, open(sys.argv[2], "w"))
