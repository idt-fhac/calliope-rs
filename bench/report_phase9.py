"""Summarise ``bench/results/phase9/*.json`` (median over repetitions) as Markdown tables.

python3 bench/report_phase9.py

Rust ``solve`` includes loading results; Python ``solve`` here is
``solve.wall_s_incl_postprocess`` (solve + ``load_results``), so the columns compare.
"""

from __future__ import annotations

import json
import statistics
from collections import defaultdict
from pathlib import Path

RESULTS = Path(__file__).resolve().parent / "results" / "phase9"
VARIANTS = ["main", "speedups", "rust"]
CASES = (
    ["national", "urban"]
    + [f"syn:{n}:{t}" for n in (5, 50, 250) for t in (168, 2190, 8760)]
    + ["operate:10", "operate:31", "spores:10", "spores:31"]
)


def stage_s(r: dict, stage: str) -> float | None:
    st = r.get("timings", {}).get("stages", {}).get(stage)
    if not st:
        return None
    if stage == "solve" and "wall_s_incl_postprocess" in st:
        return st["wall_s_incl_postprocess"]
    return st.get("wall_s")


def agg(runs: list[dict]) -> dict:
    ok = [r for r in runs if r.get("status", "").startswith("ok")]
    out = {
        "status": runs[0].get("status", "?") if not ok else ok[0]["status"],
        "n": len(ok),
    }
    for s in ("preprocess", "build", "solve", "write"):
        vals = [v for r in ok if (v := stage_s(r, s)) is not None]
        out[s] = statistics.median(vals) if vals else None
    tot = [
        sum(
            v
            for s in ("preprocess", "build", "solve", "write")
            if (v := stage_s(r, s)) is not None
        )
        for r in ok
    ]
    out["total"] = statistics.median(tot) if tot else None
    bp = [
        r["timings"]["stages"]["build"].get("peak_rss_mb")
        or r.get("observed_peak_rss_mb")
        for r in ok
        if "build" in r.get("timings", {}).get("stages", {})
    ]
    out["build_peak"] = statistics.median(bp) if bp else None
    peaks = [
        r.get("observed_peak_rss_mb")
        for r in (ok or runs)
        if r.get("observed_peak_rss_mb")
    ]
    out["peak"] = statistics.median(peaks) if peaks else None
    return out


def f(v, d=1):
    return "–" if v is None else f"{v:.{d}f}"


def ratio(a, b):
    return "–" if not a or not b else f"{a / b:.1f}×"


def main():
    groups = defaultdict(list)
    for p in sorted(RESULTS.glob("*__r*.json")):
        r = json.loads(p.read_text())
        groups[(r["case"], r["variant"])].append(r)
    a = {k: agg(v) for k, v in groups.items()}

    print(
        "| case | variant | n | status | preprocess s | build s | solve+results s | write s | total s | build peak MB | peak RSS MB |"
    )
    print("|---|---|---|---|---|---|---|---|---|---|---|")
    for c in CASES:
        for v in VARIANTS:
            x = a.get((c, v))
            if not x:
                continue
            d = 2 if v == "rust" else 1
            print(
                f"| {c} | {v} | {x['n']} | {x['status'][:40]} | {f(x['preprocess'], d)} | {f(x['build'], d)} | "
                f"{f(x['solve'], d)} | {f(x['write'], d)} | {f(x['total'], d)} | {f(x['build_peak'], 0)} | {f(x['peak'], 0)} |"
            )
    print()
    print(
        "| case | build: main / rust | build: speedups / rust | total: main / rust | build peak: main / rust | peak RSS: main / rust | rust solve share |"
    )
    print("|---|---|---|---|---|---|---|")
    for c in CASES:
        m, s, r = a.get((c, "main")), a.get((c, "speedups")), a.get((c, "rust"))
        if not r:
            continue
        m = m or {}
        s = s or {}
        share = (
            f"{100 * r['solve'] / r['total']:.0f}%"
            if r.get("solve") and r.get("total")
            else "–"
        )
        tot = (
            ratio(m.get("total"), r.get("total"))
            if "build-only" not in r["status"]
            else "(build only)"
        )
        print(
            f"| {c} | {ratio(m.get('build'), r.get('build'))} | {ratio(s.get('build'), r.get('build'))} | "
            f"{tot} | {ratio(m.get('build_peak'), r.get('build_peak'))} | {ratio(m.get('peak'), r.get('peak'))} | {share} |"
        )


if __name__ == "__main__":
    main()
