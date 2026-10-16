"""Run the CLI binary bundled in this package.

The console script `calliope-rs` executes the Rust `calliope` binary that
`build_wheel.sh` bundles into `calliope_rs/bin/`, side by side with the
`calliope` command of the Python package.
"""

from __future__ import annotations

import os
import sys
from pathlib import Path

BIN = Path(__file__).resolve().parent / "bin" / "calliope"


def main() -> None:
    if not BIN.exists():
        sys.exit(
            f"{BIN} is missing.\n"
            "Build the wheel with crates/calliope-py/build_wheel.sh, which bundles "
            "the release CLI binary built by `cargo build --release -p calliope-cli`."
        )
    os.execv(str(BIN), [str(BIN), *sys.argv[1:]])
