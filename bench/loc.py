"""Lines of code per subsystem: calliope-rs crates against their Python counterparts.

Counts code lines only: no blank lines, no comments, no docstrings / doc comments.
Rust: ``src/`` without ``#[cfg(test)] mod`` blocks (counted as tests), ``tests/`` and
``examples/`` counted separately; ``grammar.pest`` counts as code. Python: the
``src/calliope`` package of ``../calliope`` (tests are in ``tests/`` upstream).

python3 bench/loc.py [--json]
"""

from __future__ import annotations

import ast
import json
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
PY = (REPO.parent / "calliope" / "src" / "calliope").resolve()

# subsystem: (rust crates, python files relative to src/calliope)
SUBSYSTEMS = {
    "YAML + config schemas": (
        ["calliope-yaml", "calliope-schema"],
        ["attrdict.py", "schemas/*.py"],
    ),
    "Preprocess (definition, data tables, time, model data)": (
        ["calliope-preprocess"],
        ["preprocess/*.py"],
    ),
    "Arrays (xarray subset)": (["calliope-array"], []),
    "Math engine (parser, where, helpers, eval)": (
        ["calliope-math"],
        [
            "backend/parsing.py",
            "backend/expression_parser.py",
            "backend/where_parser.py",
            "backend/helper_functions.py",
            "backend/eval_attrs.py",
            "backend/backend_model.py",
            "backend/__init__.py",
        ],
    ),
    "LP model + LP format/diff": (["calliope-lp"], []),
    "HiGHS backend": (["calliope-highs"], ["backend/highs_backend_model.py"]),
    "Model, modes (operate, SPORES)": (["calliope-model"], ["model.py"]),
    "I/O (NetCDF, CSV)": (["calliope-io"], ["io.py"]),
    "CLI + generate_runs + examples": (
        ["calliope-cli"],
        ["cli.py", "util/generate_runs.py", "examples.py"],
    ),
    "Misc (exceptions, logging, tools)": (
        [],
        [
            "exceptions.py",
            "util/logging.py",
            "util/tools.py",
            "util/__init__.py",
            "__init__.py",
            "_version.py",
        ],
    ),
    "Pyomo / Gurobi / LaTeX backends, math docs": (
        [],
        [
            "backend/pyomo_backend_model.py",
            "backend/gurobi_backend_model.py",
            "backend/latex_backend_model.py",
            "postprocess/*.py",
        ],
    ),
    "Python bindings": (["calliope-py"], []),
    "Dev tooling (parity, golden, schema-diff)": (["xtask"], []),
}


def rust_counts(path: Path) -> tuple[int, int]:
    """(code, test) lines of a Rust or pest file."""
    code = test = 0
    in_test = False
    depth = 0
    pending_cfg = False
    in_block_comment = False
    for raw in path.read_text().splitlines():
        line = raw.strip()
        if in_block_comment:
            if "*/" in line:
                in_block_comment = False
            continue
        if line.startswith("/*"):
            in_block_comment = "*/" not in line
            continue
        if not line or line.startswith("//"):
            continue
        if not in_test and line == "#[cfg(test)]":
            pending_cfg = True
            test += 1
            continue
        if pending_cfg:
            pending_cfg = False
            if line.startswith("mod ") and line.endswith("{"):
                in_test, depth = True, 0
            else:
                code += 1
                continue
        if in_test:
            test += 1
            depth += line.count("{") - line.count("}")
            if depth <= 0:
                in_test = False
            continue
        code += 1
    return code, test


def crate_counts(crate: str) -> dict:
    root = REPO / "crates" / crate
    c = {"src": 0, "inline_tests": 0, "tests": 0, "examples": 0}
    for f in sorted((root / "src").rglob("*")):
        if f.suffix in (".rs", ".pest"):
            code, test = rust_counts(f)
            c["src"] += code
            c["inline_tests"] += test
    for sub in ("tests", "examples"):
        for f in sorted((root / sub).rglob("*.rs")) if (root / sub).exists() else []:
            c[sub] += sum(rust_counts(f))
    return c


def py_code_lines(path: Path) -> int:
    src = path.read_text()
    tree = ast.parse(src)
    skip: set[int] = set()
    for node in ast.walk(tree):
        if isinstance(
            node, (ast.Module, ast.ClassDef, ast.FunctionDef, ast.AsyncFunctionDef)
        ):
            body = node.body
            if (
                body
                and isinstance(body[0], ast.Expr)
                and isinstance(getattr(body[0], "value", None), ast.Constant)
                and isinstance(body[0].value.value, str)
            ):
                skip.update(range(body[0].lineno, body[0].end_lineno + 1))
    n = 0
    for i, raw in enumerate(src.splitlines(), 1):
        line = raw.strip()
        if not line or line.startswith("#") or i in skip:
            continue
        n += 1
    return n


def main():
    rows = []
    for name, (crates, pyglobs) in SUBSYSTEMS.items():
        rs = {"src": 0, "inline_tests": 0, "tests": 0, "examples": 0}
        for cr in crates:
            for k, v in crate_counts(cr).items():
                rs[k] += v
        files = sorted({f for g in pyglobs for f in PY.glob(g)})
        py = sum(py_code_lines(f) for f in files)
        py_raw = sum(len(f.read_text().splitlines()) for f in files)
        rows.append(
            {
                "subsystem": name,
                "crates": crates,
                "rust": rs,
                "python_code": py,
                "python_raw": py_raw,
            }
        )
    if "--json" in sys.argv:
        print(json.dumps(rows, indent=1))
        return
    print(
        "| subsystem | Rust crates | Rust code | Rust tests (inline + tests/) | Python code | Python raw lines |"
    )
    print("|---|---|---|---|---|---|")
    tot = {"rs": 0, "rt": 0, "py": 0, "pr": 0}
    for r in rows:
        rs = r["rust"]
        t = rs["inline_tests"] + rs["tests"]
        print(
            f"| {r['subsystem']} | {', '.join(r['crates']) or '–'} | {rs['src']} | {t} | {r['python_code']} | {r['python_raw']} |"
        )
        tot["rs"] += rs["src"]
        tot["rt"] += t
        tot["py"] += r["python_code"]
        tot["pr"] += r["python_raw"]
    print(f"| **total** | | {tot['rs']} | {tot['rt']} | {tot['py']} | {tot['pr']} |")


if __name__ == "__main__":
    main()
