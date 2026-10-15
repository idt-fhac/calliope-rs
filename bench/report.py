"""Summarise ``bench/results/*.json`` as Markdown tables (pasted into BASELINE.md).

python3 bench/report.py
"""

from __future__ import annotations

import json
from pathlib import Path

RESULTS = Path(__file__).resolve().parent / "results"
STAGES = ["preprocess", "build", "lp_write", "solve", "postprocess", "write"]


def load() -> list[dict]:
    return [json.loads(p.read_text()) for p in sorted(RESULTS.glob("*.json"))]


def _size(r: dict) -> str:
    s = r.get("sizes", {})
    return f"{s.get('nodes', '?')}×{s.get('techs', '?')}×{s.get('timesteps', '?')}"


def stage_table(rows: list[dict]) -> str:
    head = (
        "| case | variant | nodes×techs×timesteps | status | "
        + " | ".join(f"{s} s (share)" for s in STAGES)
        + " | total s | peak RSS MB |"
    )
    sep = "|" + "---|" * (len(STAGES) + 6)
    out = [head, sep]
    for r in rows:
        st = r.get("timings", {}).get("stages", {})
        total = r.get("timings", {}).get("total_wall_s") or 0
        cells = []
        for s in STAGES:
            if s in st and st[s].get("wall_s") is not None:
                w = st[s]["wall_s"]
                share = f" ({100 * w / total:.0f}%)" if total else ""
                cells.append(f"{w:.1f}{share}")
            else:
                cells.append("–")
        out.append(
            f"| {r['case']} | {r['variant']} | {_size(r)} | {r.get('status', '?')} | "
            + " | ".join(cells)
            + f" | {total:.1f} | {r.get('observed_peak_rss_mb', 0):.0f} |"
        )
    return "\n".join(out)


def rss_table(rows: list[dict]) -> str:
    out = [
        "| case | variant | " + " | ".join(f"{s} peak MB" for s in STAGES) + " |",
        "|" + "---|" * (len(STAGES) + 2),
    ]
    for r in rows:
        st = r.get("timings", {}).get("stages", {})
        cells = [
            f"{st[s]['peak_rss_mb']:.0f}" if s in st and "peak_rss_mb" in st[s] else "–"
            for s in STAGES
        ]
        out.append(f"| {r['case']} | {r['variant']} | " + " | ".join(cells) + " |")
    return "\n".join(out)


def events_table(rows: list[dict]) -> str:
    out = [
        "| case | variant | solves | per-solve s (mean / max) | update_input calls (total s) | rebuilds (total s) | load_results total s |",
        "|---|---|---|---|---|---|---|",
    ]
    for r in rows:
        ev = r.get("timings", {}).get("events", [])
        if not ev:
            continue
        solves = [e["wall_s"] for e in ev if e["kind"] == "backend_solve"]
        upd = [e["wall_s"] for e in ev if e["kind"] == "update_input"]
        reb = [e["wall_s"] for e in ev if e["kind"] == "model_build"][1:]
        lr = sum(e["wall_s"] for e in ev if e["kind"] == "load_results")
        if not solves:
            continue
        out.append(
            f"| {r['case']} | {r['variant']} | {len(solves)} | "
            f"{sum(solves) / len(solves):.2f} / {max(solves):.2f} | "
            f"{len(upd)} ({sum(upd):.1f}) | {len(reb)} ({sum(reb):.1f}) | {lr:.1f} |"
        )
    return "\n".join(out)


if __name__ == "__main__":
    rows = load()
    order = {"main": 0, "speedups": 1, "pyomo": 2}

    def key(r):
        c = r["case"]
        parts = c.split(":")
        num = tuple(int(p) for p in parts[1:]) if len(parts) > 1 else ()
        return (parts[0], num, order.get(r["variant"], 9))

    rows.sort(key=key)
    print("## Stage wall time\n")
    print(stage_table(rows))
    print("\n## Peak RSS per stage\n")
    print(rss_table(rows))
    print("\n## Per-iteration events (operate / SPORES)\n")
    print(
        events_table(
            [r for r in rows if r["case"].split(":")[0] in ("operate", "spores")]
        )
    )
