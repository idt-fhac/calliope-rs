"""Write ``golden/MANIFEST.json``: per fixture file sizes, sha256 and key results.

    python oracle/manifest.py [golden_dir]

The manifest is committed even if large NetCDFs are ever git-ignored, so that a
regenerated tree can be checked against it (sha256 of LP files; for NetCDF use
``compare_runs.py``, since attrs carry timestamps).
"""

from __future__ import annotations

import hashlib
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def sha256(p: Path) -> str:
    h = hashlib.sha256()
    with p.open("rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def build(golden: Path) -> dict:
    out: dict = {"fixtures": {}, "math_lp": {}}
    for meta_path in sorted(golden.rglob("meta.json")):
        d = meta_path.parent
        meta = json.loads(meta_path.read_text())
        files = {
            f.name: {"bytes": f.stat().st_size, "sha256": sha256(f)}
            for f in sorted(d.iterdir())
            if f.is_file() and f.name != "meta.json"
        }
        rel = d.relative_to(golden)
        if rel.parts[0] == "math_lp":
            out["math_lp"][rel.name] = {
                "files": files,
                "scenario": meta.get("scenario"),
                "highs_lp_names": meta.get("highs_lp_names"),
                "highs_error": meta.get("highs_error"),
            }
        else:
            out["fixtures"][rel.as_posix()] = {
                "files": files,
                "objectives": meta.get("objectives"),
                "termination_condition": meta.get("termination_condition"),
                "expected_error": meta.get("expected_error"),
                "versions": meta.get("versions"),
            }
    total = sum(
        f["bytes"]
        for group in ("fixtures", "math_lp")
        for fx in out[group].values()
        for f in fx["files"].values()
    )
    out["total_bytes"] = total
    return out


if __name__ == "__main__":
    golden = Path(sys.argv[1]) if len(sys.argv) > 1 else ROOT / "golden"
    m = build(golden)
    (golden / "MANIFEST.json").write_text(json.dumps(m, indent=1, sort_keys=True))
    print(
        f"{len(m['fixtures'])} fixtures, {len(m['math_lp'])} math LP fixtures, "
        f"{m['total_bytes'] / 2**20:.1f} MiB"
    )
