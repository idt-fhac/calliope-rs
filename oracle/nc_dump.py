"""Dump a Calliope NetCDF file (groups ``inputs``, ``results``, ``attrs``) to canonical JSON.

Used by ``xtask parity`` so the Rust side can compare NetCDF files without linking libnetcdf.

Canonical form (so that dimension order and coordinate order do not matter):
every variable is transposed to alphabetically sorted dimensions and every dimension is
sorted by its coordinate values.  NaN becomes ``null`` in data arrays (``"nan"`` inside attributes); +-inf become the strings ``"inf"``/``"-inf"``; datetimes become ISO strings.
The volatile ``runtime`` attribute (timings) is dropped; YAML-valued attributes of the
``attrs`` group (``config``, ``definition``, ``math``) are parsed into JSON values.

Usage: python -I nc_dump.py <file.nc> [<out.json>]   (stdout when no output path)
"""

import json
import sys

import numpy as np
import xarray as xr

DROP_ATTRS = {"runtime"}
YAML_ATTRS = {"config", "definition", "math"}


def jsonable(x):
    if isinstance(x, (np.floating, float)):
        return None if np.isnan(x) else float(x)
    if isinstance(x, (np.integer,)):
        return int(x)
    if isinstance(x, (np.bool_,)):
        return bool(x)
    if isinstance(x, np.datetime64):
        return str(x)
    if isinstance(x, np.ndarray):
        return [jsonable(i) for i in x.tolist()]
    if isinstance(x, (list, tuple)):
        return [jsonable(i) for i in x]
    if isinstance(x, bytes):
        return x.decode()
    return x


def clean(x):
    """Replace non-finite floats by the strings ``"inf"``, ``"-inf"`` and ``"nan"``."""
    if isinstance(x, float):
        if np.isnan(x):
            return "nan"
        if np.isinf(x):
            return "inf" if x > 0 else "-inf"
        return x
    if isinstance(x, dict):
        return {k: clean(v) for k, v in x.items()}
    if isinstance(x, list):
        return [clean(v) for v in x]
    return x


def flat(values):
    arr = np.asarray(values)
    if arr.dtype.kind in "OUS":
        return [None if (isinstance(v, float) and np.isnan(v)) else str(v) for v in arr.ravel().tolist()]
    if arr.dtype.kind == "M":
        return [str(v) for v in arr.ravel()]
    if arr.dtype.kind == "f":
        return [None if np.isnan(v) else float(v) for v in arr.ravel().tolist()]
    return arr.ravel().tolist()


def kind(arr):
    k = np.asarray(arr).dtype.kind
    return {"f": "float", "i": "int", "u": "int", "b": "bool", "M": "time"}.get(k, "str")


def canonical(da, ds):
    for d in da.dims:
        if d in ds.coords:
            order = np.argsort(np.asarray(ds[d].values).astype(str), kind="stable")
            da = da.isel({d: order})
    return da.transpose(*sorted(da.dims))


def dump_group(ds):
    out = {"dims": {k: int(v) for k, v in ds.sizes.items()}, "coords": {}, "vars": {}, "attrs": {}}
    for name, c in ds.coords.items():
        c = canonical(c, ds)
        out["coords"][name] = {"dims": list(c.dims), "kind": kind(c.values), "data": flat(c.values)}
    for name, v in ds.data_vars.items():
        v = canonical(v, ds)
        out["vars"][name] = {
            "dims": list(v.dims),
            "shape": [int(s) for s in v.shape],
            "kind": kind(v.values),
            "attrs": {k: jsonable(a) for k, a in v.attrs.items() if not k.startswith("serialised")},
            "data": flat(v.values),
        }
    out["attrs"] = {k: jsonable(a) for k, a in ds.attrs.items() if not k.startswith("serialised")}
    return out


def dump_attrs(path):
    import netCDF4
    import yaml

    out = {}
    with netCDF4.Dataset(path) as d:
        if "attrs" not in d.groups:
            return out
        g = d.groups["attrs"]
        for k in g.ncattrs():
            if k.startswith("serialised") or k in DROP_ATTRS:
                continue
            v = g.getncattr(k)
            if k in YAML_ATTRS and isinstance(v, str):
                v = yaml.safe_load(v)
            out[k] = jsonable(v)
    return out


def main(path, out_path=None):
    result = {"groups": {}, "attrs": dump_attrs(path)}
    for g in ("inputs", "results"):
        try:
            with xr.open_dataset(path, group=g, decode_times=False, decode_timedelta=False) as ds:
                result["groups"][g] = dump_group(ds)
        except OSError:
            continue
    text = json.dumps(clean(result), sort_keys=True, allow_nan=False)
    if out_path:
        with open(out_path, "w") as f:
            f.write(text)
    else:
        sys.stdout.write(text)


if __name__ == "__main__":
    main(*sys.argv[1:3])
