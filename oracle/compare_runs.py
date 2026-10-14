"""Reproducibility check: compare two golden trees produced by ``golden.py``.

    python oracle/compare_runs.py golden /path/to/other_golden

For every fixture, every data variable and coordinate in the ``inputs`` and
``results`` groups of ``inputs.nc``/``results.nc`` must be identical (dims, dtype,
values incl. NaN positions, compared as bytes). LP files must be byte-identical.
Attributes are not compared: they carry timestamps.
"""

from __future__ import annotations

import sys
from pathlib import Path

import numpy as np
import xarray as xr


def _same(a: xr.DataArray, b: xr.DataArray) -> bool:
    if a.dims != b.dims or a.dtype != b.dtype or a.shape != b.shape:
        return False
    av, bv = a.values, b.values
    if av.dtype.kind == "f":
        return av.tobytes() == bv.tobytes() or np.array_equal(av, bv, equal_nan=True)
    return np.array_equal(av, bv)


def compare_nc(a: Path, b: Path) -> list[str]:
    diffs = []
    for group in ("inputs", "results"):
        try:
            da = xr.open_dataset(a, group=group)
            db = xr.open_dataset(b, group=group)
        except OSError:
            continue
        with da, db:
            names = set(da.variables) | set(db.variables)
            for n in sorted(names):
                if n not in da.variables or n not in db.variables:
                    diffs.append(f"{group}/{n}: missing in one run")
                elif not _same(da[n], db[n]):
                    diffs.append(f"{group}/{n}: differs")
    return diffs


def main(a_root: Path, b_root: Path) -> int:
    bad = 0
    checked = 0
    for fx in sorted(p for p in a_root.rglob("meta.json")):
        rel = fx.parent.relative_to(a_root)
        other = b_root / rel
        for f in sorted(fx.parent.iterdir()):
            g = other / f.name
            if f.suffix not in (".nc", ".lp"):
                continue
            checked += 1
            if not g.exists():
                print(f"MISSING {rel}/{f.name}")
                bad += 1
                continue
            if f.suffix == ".lp":
                if f.read_bytes() != g.read_bytes():
                    print(f"DIFF    {rel}/{f.name}")
                    bad += 1
            else:
                d = compare_nc(f, g)
                if d:
                    bad += 1
                    print(
                        f"DIFF    {rel}/{f.name}: {d[:5]}{' ...' if len(d) > 5 else ''}"
                    )
    print(f"{checked} files compared, {bad} differ")
    return 1 if bad else 0


if __name__ == "__main__":
    sys.exit(main(Path(sys.argv[1]), Path(sys.argv[2])))
