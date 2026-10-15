"""Parse oracle for crates/calliope-math/tests/parse_corpus.rs.

Collects every math string (`where`, equation, sub-expression, slice, piecewise and check
strings) in Calliope's pre-defined math, the math gallery and the example models, parses
each with the pyparsing grammar Calliope would use to build it, and records the parse tree
`repr` (or that parsing failed). Seeded mutants of each string are added so that the corpus
also covers near-miss strings.

Run with the Calliope pixi env's Python:

    ../calliope/.pixi/envs/dev/bin/python -I crates/calliope-math/tests/oracle/parse_oracle.py \
        ../calliope crates/calliope-math/tests/fixtures/parse_corpus.json
"""

import json
import random
import sys
from pathlib import Path

import pyparsing as pp

from calliope.backend import expression_parser, where_parser
from calliope.io import read_rich_yaml
from calliope.schemas.math_schema import CalliopeBuildMath

MUTANTS_PER_STRING = 6
INSERTS = list(" ()[],=+-*/<>$.") + ["**", "==", " and ", " or ", " not ", "inf", "1"]


def build(priority: list[Path]) -> CalliopeBuildMath:
    math = CalliopeBuildMath()
    for path in priority:
        math = math.update(read_rich_yaml(path))
    return math


def names(math: CalliopeBuildMath) -> dict:
    pc = math.parsing_components
    return {
        "where": {k: sorted(v) for k, v in pc["where"].items()},
        "expression": sorted(set().union(*pc["expression"].values())),
        "postprocessed_where": sorted(math.postprocessed.root),
        "postprocessed_expression": sorted(math.postprocessed._active),
    }


def parsers(n: dict) -> dict:
    w = {k: set(v) for k, v in n["where"].items()}
    expr = set(n["expression"])
    expr_pp = expr | set(n["postprocessed_expression"])
    return {
        "where": where_parser.generate_where_string_parser(**w),
        "where_postprocessed": where_parser.generate_where_string_parser(
            **w, postprocessed=set(n["postprocessed_where"])
        ),
        "equation": expression_parser.generate_equation_parser(expr),
        "arithmetic": expression_parser.generate_arithmetic_parser(expr),
        "arithmetic_postprocessed": expression_parser.generate_arithmetic_parser(
            expr_pp
        ),
        "sub_expression": expression_parser.generate_sub_expression_parser(expr),
        "sub_expression_postprocessed": expression_parser.generate_sub_expression_parser(
            expr_pp
        ),
        "slice": expression_parser.generate_slice_parser(expr),
        "slice_postprocessed": expression_parser.generate_slice_parser(expr_pp),
    }


def strings(math: CalliopeBuildMath):
    """Yield (path, context, string) for every math string."""
    groups = {
        "variables": None,
        "global_expressions": "arithmetic",
        "constraints": "equation",
        "piecewise_constraints": None,
        "objectives": "arithmetic",
        "postprocessed": "arithmetic",
    }
    for group, eq_context in groups.items():
        pp_suffix = "_postprocessed" if group == "postprocessed" else ""
        where_ctx = "where" + pp_suffix
        for name, comp in getattr(math, group).root.items():
            base = f"{group}.{name}"
            if hasattr(comp, "where"):
                yield f"{base}.where", where_ctx, comp.where
            if group == "piecewise_constraints":
                yield f"{base}.x_expression", "arithmetic", comp.x_expression
                yield f"{base}.y_expression", "arithmetic", comp.y_expression
            if eq_context is None:
                continue
            for i, eq in enumerate(comp.equations):
                yield f"{base}.equations[{i}].where", where_ctx, eq.where
                yield f"{base}.equations[{i}].expression", eq_context + pp_suffix, eq.expression
            for key, ctx in [("sub_expressions", "sub_expression"), ("slices", "slice")]:
                for sub_name, items in getattr(comp, key).root.items():
                    for i, item in enumerate(items):
                        p = f"{base}.{key}.{sub_name}[{i}]"
                        yield f"{p}.where", where_ctx, item.where
                        yield f"{p}.expression", ctx + pp_suffix, item.expression
    for name, check in math.checks.root.items():
        yield f"checks.{name}.where", "where", check.where


def parse(parser, string: str) -> dict:
    try:
        return {"ok": True, "repr": str(parser.parse_string(string, parse_all=True)[0])}
    except pp.ParseException as err:
        return {"ok": False, "col": err.col}
    except RecursionError:
        return {"ok": False, "col": 0, "recursion": True}


def mutants(rng: random.Random, s: str) -> list[str]:
    out = []
    for _ in range(MUTANTS_PER_STRING):
        kind = rng.randrange(5)
        pos = rng.randrange(len(s) + 1)
        if kind == 0 and s:
            pos = min(pos, len(s) - 1)
            m = s[:pos] + s[pos + 1 :]
        elif kind == 1:
            m = s[:pos] + rng.choice(INSERTS) + s[pos:]
        elif kind == 2:
            m = s.replace(" ", "", 1) if " " in s else s + " "
        elif kind == 3:
            m = s.swapcase() if rng.random() < 0.3 else s.replace("(", "", 1)
        else:
            words = [w for w in s.replace("(", " ").replace(")", " ").split() if w.isidentifier()]
            m = s.replace(rng.choice(words), "unknown_x", 1) if words else s + ")"
        if m != s:
            out.append(m)
    return out


def main(ref: Path, out: Path) -> None:
    math_dir = ref / "src/calliope/math"
    gallery = ref / "docs/examples/math_gallery"
    sets = {
        "base": [math_dir / "base.yaml"],
        "milp": [math_dir / "base.yaml", math_dir / "milp.yaml"],
        "operate": [math_dir / "base.yaml", math_dir / "operate.yaml"],
        "spores": [math_dir / "base.yaml", math_dir / "spores.yaml"],
        "storage_inter_cluster": [
            math_dir / "base.yaml",
            math_dir / "storage_inter_cluster.yaml",
        ],
        "urban_scale": [
            math_dir / "base.yaml",
            ref / "src/calliope/example_models/urban_scale/additional_math.yaml",
        ],
    }
    for f in sorted(gallery.glob("*.yaml")):
        sets[f"gallery_{f.stem}"] = [math_dir / "base.yaml", math_dir / "milp.yaml", f]

    rng = random.Random(20261015)
    corpus = {"name_sets": {}, "cases": []}
    seen = set()
    for set_name, files in sets.items():
        math = build(files)
        n = names(math)
        corpus["name_sets"][set_name] = n
        ps = parsers(n)
        for path, ctx, s in strings(math):
            if (ctx, s) in seen:
                continue
            for i, variant in enumerate([s, *mutants(rng, s)]):
                key = (ctx, variant)
                if key in seen:
                    continue
                seen.add(key)
                corpus["cases"].append(
                    {
                        "set": set_name,
                        "path": path if i == 0 else f"{path}~{i}",
                        "context": ctx,
                        "string": variant,
                        **parse(ps[ctx], variant),
                    }
                )
    out.write_text(json.dumps(corpus, indent=0, sort_keys=True) + "\n")
    n_ok = sum(c["ok"] for c in corpus["cases"])
    print(f"{len(corpus['cases'])} cases, {n_ok} parse", file=sys.stderr)


if __name__ == "__main__":
    main(Path(sys.argv[1]), Path(sys.argv[2]))
