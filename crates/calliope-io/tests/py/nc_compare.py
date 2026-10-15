"""Compare two Calliope NetCDF files as Python Calliope reads them.

Usage: python nc_compare.py EXPECTED ACTUAL [rtol] [subset]

With `subset`, only the result variables present in ACTUAL are compared.

Checks that `calliope.io.read_netcdf` gives equal attrs (deserialised) and equal data
(values, dims, dtypes, data-variable attrs) for every group, and that
`calliope.read_netcdf(ACTUAL)` builds a Model. Prints OK or the differences; exit 1 on
differences.
"""

import sys

import numpy as np

import calliope
from calliope import io


def main(expected, actual, rtol=0.0, subset=False):
    a, b = io.read_netcdf(expected), io.read_netcdf(actual)
    if subset:
        a["results"] = a["results"][list(b["results"].data_vars)]
    errs = []
    for g in ["inputs", "results", "attrs"]:
        da, db = a[g], b[g]
        if dict(da.attrs) != dict(db.attrs):
            keys = set(da.attrs) | set(db.attrs)
            bad = [k for k in keys if da.attrs.get(k) != db.attrs.get(k)]
            errs.append(f"{g}: attrs differ in {sorted(bad)}")
        if set(da.data_vars) != set(db.data_vars):
            errs.append(f"{g}: vars {sorted(set(da.data_vars) ^ set(db.data_vars))}")
        for k in set(da.coords) | set(db.coords):
            if k not in da.coords or k not in db.coords:
                errs.append(f"{g}: coord {k} missing")
            elif not da[k].to_index().equals(db[k].to_index()):
                errs.append(f"{g}: coord {k} labels differ")
        for k in set(da.data_vars) & set(db.data_vars):
            x, y = da[k], db[k]
            if x.dims != y.dims:
                errs.append(f"{g}.{k}: dims {x.dims} != {y.dims}")
                continue
            if x.dtype != y.dtype:
                errs.append(f"{g}.{k}: dtype {x.dtype} != {y.dtype}")
            if dict(x.attrs) != dict(y.attrs):
                errs.append(f"{g}.{k}: attrs {x.attrs} != {y.attrs}")
            y = y.transpose(*x.dims)
            if x.dtype.kind == "f" and rtol:
                ok = np.allclose(x.values, y.values, rtol=rtol, atol=rtol, equal_nan=True)
            else:
                ok = x.equals(y)
            if not ok:
                errs.append(f"{g}.{k}: values differ")
    calliope.read_netcdf(actual)
    if errs:
        print("\n".join(errs))
        return 1
    print("OK")
    return 0


if __name__ == "__main__":
    sys.exit(
        main(
            sys.argv[1],
            sys.argv[2],
            float(sys.argv[3]) if len(sys.argv) > 3 else 0.0,
            len(sys.argv) > 4 and sys.argv[4] == "subset",
        )
    )
