#!/usr/bin/env bash
# Build the calliope-rs wheel: the PyO3 extension plus the release CLI binary,
# so that `pip install calliope_rs-*.whl` provides the `calliope` command.
#
# Uses the toolchain of the LEDGER notes: rustup cargo in ~/.cargo, cmake 4.4 in
# ~/.local/bin and pip-installed libclang, all needed by highs-sys.
#
# The result is in target/wheels/. Install it with e.g.
#   uv venv --system-site-packages -p ../calliope/.pixi/envs/dev/bin/python
#   uv pip install ../../target/wheels/calliope_rs-*.whl
set -euo pipefail
cd "$(dirname "$0")"

export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$PATH"
export LIBCLANG_PATH="${LIBCLANG_PATH:-$HOME/.local/opt/libclang/clang/native}"

cargo build --release -p calliope-cli
mkdir -p python/calliope_rs/bin
cp ../../target/release/calliope python/calliope_rs/bin/calliope

if command -v maturin >/dev/null; then
  maturin build --release "$@"
else
  uv tool run --from 'maturin>=1.9,<2' maturin build --release "$@"
fi
