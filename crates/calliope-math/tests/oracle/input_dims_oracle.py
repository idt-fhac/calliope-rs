"""Input dimensions of Calliope's example models, for the typecheck with model data.

Records, for each example model, its math (mode and extra math) and the dimensions of each
input array after `Model` initialisation.

    ../calliope/.pixi/envs/dev/bin/python -I crates/calliope-math/tests/oracle/input_dims_oracle.py \
        crates/calliope-math/tests/fixtures/input_dims.json
"""

import json
import sys

import calliope

MODELS = {
    "national_scale": (calliope.examples.national_scale, {}),
    "national_scale_spores": (calliope.examples.national_scale, {"scenario": "spores"}),
    "urban_scale": (calliope.examples.urban_scale, {}),
    "milp": (calliope.examples.milp, {}),
}


def main(out: str) -> None:
    result = {}
    for name, (fn, kwargs) in MODELS.items():
        model = fn(**kwargs)
        result[name] = {
            "mode": model.config.init.mode,
            "extra_math": list(model.config.init.extra_math),
            "dims": {k: list(v.dims) for k, v in model.inputs.data_vars.items()},
        }
    with open(out, "w") as f:
        json.dump(result, f, indent=0, sort_keys=True)
        f.write("\n")


if __name__ == "__main__":
    main(sys.argv[1])
