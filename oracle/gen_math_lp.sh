#!/usr/bin/env bash
# Regenerate golden/math_lp/: one fixture per LP file built by upstream tests/math.
# Runs the upstream math tests with the pytest_golden_lp plugin (see its docstring).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
REPO="$(dirname "$HERE")"
CALLIOPE_DIR="${CALLIOPE_DIR:-$(cd "$REPO/../calliope" && pwd)}"
OUT="${1:-$REPO/golden/math_lp}"
PY="$CALLIOPE_DIR/.pixi/envs/dev/bin/python"

rm -rf "$OUT"
mkdir -p "$OUT"
cd "$CALLIOPE_DIR"
GOLDEN_DIR="$OUT" PYTHONPATH="$HERE:$CALLIOPE_DIR" PATH="$(dirname "$PY"):$PATH" \
  "$PY" -m pytest -p pytest_golden_lp -p no:cacheprovider -n0 --no-cov -q \
  tests/math/math_test.py "${@:2}"
