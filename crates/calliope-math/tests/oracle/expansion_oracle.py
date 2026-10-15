"""Equation expansion oracle for crates/calliope-math/tests/expansion.rs.

For every component with equations in the pre-defined math (each mode layered on `base`),
the math gallery and the urban-scale example math, records the names Calliope gives to
the expanded equations (`ParsedBackendComponent.parse_equations`) and how many `where`
strings each carries. The parts after the equation name follow Python `set` order, so
they are written sorted.

    ../calliope/.pixi/envs/dev/bin/python -I crates/calliope-math/tests/oracle/expansion_oracle.py \
        ../calliope crates/calliope-math/tests/fixtures/expansion.json
"""

import json
import sys
from copy import deepcopy
from pathlib import Path

from calliope.backend import parsing
from calliope.io import read_rich_yaml
from calliope.schemas.math_schema import CalliopeBuildMath


def build(files):
    math = CalliopeBuildMath()
    for f in files:
        math = math.update(read_rich_yaml(f))
    return math


def normalise(name: str) -> str:
    head, *rest = name.split("-")
    return "-".join([head, *sorted(rest)])


def main(ref: Path, out: Path) -> None:
    math_dir = ref / "src/calliope/math"
    sets = {"base": [math_dir / "base.yaml"]}
    for mode in ["milp", "operate", "spores", "storage_inter_cluster"]:
        sets[mode] = [math_dir / "base.yaml", math_dir / f"{mode}.yaml"]
    sets["urban_scale"] = [
        math_dir / "base.yaml",
        ref / "src/calliope/example_models/urban_scale/additional_math.yaml",
    ]
    for f in sorted((ref / "docs/examples/math_gallery").glob("*.yaml")):
        sets[f"gallery_{f.stem}"] = [math_dir / "base.yaml", math_dir / "milp.yaml", f]

    result = {}
    for set_name, files in sets.items():
        math = build(files)
        components = []
        for group in ["global_expressions", "constraints", "objectives", "postprocessed"]:
            pc = math.parsing_components
            if group == "postprocessed":
                pc = deepcopy(pc)
                pc["where"]["postprocessed"] = set(math.postprocessed.root)
                pc["expression"]["postprocessed"] = set(math.postprocessed._active)
            for name, definition in getattr(math, group).root.items():
                if not definition.active:
                    continue
                parsed = parsing.ParsedBackendComponent(group, name, definition, pc)
                eqs = parsed.parse_equations()
                components.append(
                    {
                        "group": group,
                        "name": name,
                        "equations": sorted(
                            [normalise(e.name), len(e.where)] for e in eqs
                        ),
                    }
                )
        result[set_name] = {"files": [str(f.relative_to(ref)) for f in files], "components": components}
    out.write_text(json.dumps(result, indent=0) + "\n")


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]))
