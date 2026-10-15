"""Synthetic scale-ups of the national_scale example model.

``make_model(n_nodes, hours, out_dir)`` writes a model directory with
``n_nodes`` nodes (a multiple of 5) made of ``n_nodes / 5`` copies of the
5-node national_scale system. Copy ``k`` gets node and link names suffixed with
``_c{k}``; the ``region1`` hubs of consecutive copies are joined in a ring by
AC links (same parameters as ``region1_to_region2``). Time series are copied
per node and scaled by a deterministic per-copy factor (0.8..1.2) so that the
copies are not identical. ``flow_cap_max_systemwide`` of ``ccgt`` is scaled by
the number of copies. The time span is the first ``hours`` hours of 2005.
"""

from __future__ import annotations

import copy
from pathlib import Path

import pandas as pd

NATIONAL_DIR = Path("src/calliope/example_models/national_scale")
BASE_NODES = ["region1", "region2", "region1_1", "region1_2", "region1_3"]
BASE_LINKS = [
    "region1_to_region2",
    "region1_to_region1_1",
    "region1_to_region1_2",
    "region1_to_region1_3",
]


def _factor(k: int) -> float:
    return 0.8 + 0.4 * ((k * 7) % 11) / 10


def make_model(n_nodes: int, hours: int, out_dir: Path, calliope_dir: Path) -> Path:
    from calliope.io import read_rich_yaml, to_yaml

    assert n_nodes % 5 == 0, "n_nodes must be a multiple of 5"
    copies = n_nodes // 5
    src = calliope_dir / NATIONAL_DIR
    d = read_rich_yaml(src / "model.yaml").as_dict()
    out_dir.mkdir(parents=True, exist_ok=True)

    # --- nodes -------------------------------------------------------------
    nodes = {}
    for k in range(copies):
        for n in BASE_NODES:
            node = copy.deepcopy(d["nodes"][n])
            node["latitude"] = node["latitude"] + 0.5 * (k % 20)
            node["longitude"] = node["longitude"] + 0.5 * (k // 20)
            nodes[f"{n}_c{k}"] = node
    d["nodes"] = nodes

    # --- techs (links) -----------------------------------------------------
    techs = {k: v for k, v in d["techs"].items() if k not in BASE_LINKS}
    techs["ccgt"]["flow_cap_max_systemwide"] = (
        techs["ccgt"]["flow_cap_max_systemwide"] * copies
    )
    for k in range(copies):
        for link in BASE_LINKS:
            t = copy.deepcopy(d["techs"][link])
            t["link_from"] = f"{t['link_from']}_c{k}"
            t["link_to"] = f"{t['link_to']}_c{k}"
            techs[f"{link}_c{k}"] = t
    ac = {
        key: val
        for key, val in d["techs"]["region1_to_region2"].items()
        if key not in ("link_from", "link_to")
    }
    if copies > 1:
        ring = range(copies) if copies > 2 else range(1)
        for k in ring:
            j = (k + 1) % copies
            techs[f"ring_c{k}_c{j}"] = {
                **copy.deepcopy(ac),
                "link_from": f"region1_c{k}",
                "link_to": f"region1_c{j}",
            }
    d["techs"] = techs

    # --- data tables ---------------------------------------------------------
    ts = pd.read_csv(
        src / "data_tables/time_varying_params.csv",
        header=[0, 1, 2, 3],
        index_col=0,
    )
    ts = ts.droplevel(0, axis=1)  # drop comment level
    ts.columns.names = ["nodes", "techs", "inputs"]
    ts = ts.iloc[:hours]
    parts = []
    for k in range(copies):
        part = ts * _factor(k)
        part.columns = pd.MultiIndex.from_tuples(
            [(f"{n}_c{k}", t, i) for n, t, i in ts.columns],
            names=ts.columns.names,
        )
        parts.append(part)
    pd.concat(parts, axis=1).to_csv(out_dir / "time_varying_params.csv")

    costs = pd.read_csv(src / "data_tables/costs.csv", index_col=0, header=[0, 1])
    costs = costs.droplevel(1, axis=1)
    link_cost = costs.loc["region1_to_region2"]
    costs = costs.drop(index="region1_to_region2")
    extra = {f"region1_to_region2_c{k}": link_cost for k in range(copies)}
    extra.update({t: link_cost for t in techs if t.startswith("ring_")})
    costs = pd.concat([costs, pd.DataFrame(extra).T])
    costs.index.name = "techs"
    costs.columns.name = "inputs"
    costs.to_csv(out_dir / "costs.csv")

    d["data_tables"] = {
        "time_varying_parameters": {
            "table": "time_varying_params.csv",
            "rows": "timesteps",
            "columns": ["nodes", "techs", "inputs"],
        },
        "cost_parameters": {
            "table": "costs.csv",
            "rows": "techs",
            "columns": "inputs",
            "add_dims": {"costs": "monetary"},
        },
    }

    # --- config ----------------------------------------------------------------
    d["config"]["init"]["name"] = f"synthetic national_scale {n_nodes}x{hours}"
    d["config"]["init"]["subset"] = {"timesteps": None}
    d.pop("scenarios", None)
    d.pop("overrides", None)
    to_yaml(d, out_dir / "model.yaml")
    return out_dir / "model.yaml"


if __name__ == "__main__":
    import argparse

    ap = argparse.ArgumentParser()
    ap.add_argument("nodes", type=int)
    ap.add_argument("hours", type=int)
    ap.add_argument("out", type=Path)
    ap.add_argument("--calliope-dir", type=Path, default=Path("../calliope"))
    a = ap.parse_args()
    print(make_model(a.nodes, a.hours, a.out, a.calliope_dir.resolve()))
