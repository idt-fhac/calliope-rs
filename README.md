# calliope-rs

A Rust rewrite of [Calliope](https://github.com/calliope-project/calliope) 0.7.0, the
energy-systems modelling framework. The same model YAML, data tables, math YAML, config
keys and CLI give the same optimisation problem and the same results as Calliope 0.7.0;
the solver is [HiGHS](https://github.com/ERGO-Code/HiGHS), called directly from Rust.
There is no Pyomo or Gurobi backend.

Parity is measured, not claimed: `cargo run -p xtask -- parity` compares inputs, LP files
and results of all 37 golden fixtures against Calliope and reports
**90 pass / 17 classified / 0 fail**, where every classified difference is documented and
re-verified on each run (e.g. alternative optima are checked to be feasible and optimal
in both LPs). All 122 math-LP and 34 example LP files match Calliope's byte for byte
after canonicalisation. See `bench/REPORT.md` for the full evaluation.

## What it does better

- **Speed:** LP build 5–76× faster than `main` (national_scale 8760 h: 35 s → 2 s);
  operate runs 56–63× faster end to end. Runs where the solve dominates gain 1.1–2×.
- **Memory:** about half of Python's peak RSS on large models; sparse preprocessing
  brought 250 nodes × 2190 h from 11.8 GB to 206 MB.
- **Correctness tooling:** `calliope check` runs a schema check, a math parser and a
  static typecheck (37 diagnostic codes with source spans) before the build;
  `lp-diff` compares CPLEX LP files.
- **Bugs found in Calliope:** 14 runtime bugs with repros (`bench/BUGS.md`) and 9
  math-layer findings (`docs/findings.md`), e.g. equation names that depend on
  `PYTHONHASHSEED` and `foreach` typos that silently drop a component.

## Quick start

Build the CLI (needs a Rust stable toolchain, cmake, libclang and the system
NetCDF/HDF5 development libraries):

```bash
cargo build --release -p calliope-cli
target/release/calliope run ../calliope/src/calliope/example_models/national_scale/model.yaml \
    --save_netcdf results.nc
target/release/calliope check <model.yaml>      # static checks only
target/release/calliope inspect <model.yaml> --component <name>
```

The subcommands match Python Calliope's (`new`, `run`, `generate_runs`,
`generate_scenarios`, `build`, `solve`) plus `check`, `inspect`, `bench` and `lp-diff`.

### Python

`crates/calliope-py/build_wheel.sh` builds a `calliope-rs` wheel containing the PyO3
model API (`import calliope_rs`, an xarray-based clone of the Python model API) and the
CLI binary exposed as the `calliope-rs` command, so it can be installed next to the
Python package.

## Repository layout

| Path | Contents |
|---|---|
| `crates/calliope-array` | labelled N-d arrays, coordinates, alignment (the xarray subset) |
| `crates/calliope-yaml` | YAML with `import:`, overrides, dotted keys (ruamel parity) |
| `crates/calliope-schema` | config, model-definition and math schemas (pydantic parity) |
| `crates/calliope-preprocess` | model definition → input dataset: templates, scenarios, data tables, time |
| `crates/calliope-math` | `where`/expression parser, static typecheck and evaluation IR |
| `crates/calliope-lp` | LP/MILP model, CPLEX-LP writer/parser/diff |
| `crates/calliope-highs` | HiGHS FFI (`highs-sys`), results, in-place updates |
| `crates/calliope-model` | model lifecycle: build, solve, plan/operate/SPORES, postprocess |
| `crates/calliope-io` | NetCDF and CSV I/O, attribute (de)serialisation |
| `crates/calliope-cli` | the `calliope` binary |
| `crates/calliope-py` | PyO3 bindings, wheel, `calliope_rs` package |
| `crates/xtask` | parity harness, `lp-diff`, `schema-diff` |
| `golden/` | golden files generated from Calliope 0.7.0 (37 fixtures + 122 math LPs) |
| `oracle/` | the Python side that generates and compares the golden files |
| `bench/` | baselines, benchmarks, `BUGS.md`, `REPORT.md` |
| `docs/` | ADRs and findings |

## Development

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -D warnings
cargo test --workspace
```

Tests that need Calliope itself read the reference checkout at `../calliope`
(override with `CALLIOPE_REFERENCE`) and skip when it is absent; the golden-file tests
run everywhere. `cargo run -p xtask -- parity` needs the golden files and Python with
`netCDF4`. See `CONTRIBUTING.md`.

## Deviations from Python Calliope

Deliberate deviations are listed in `docs/findings.md` (e.g. `--save_logs` works,
`horizon < window` is rejected, SPORES `random` is seeded). Where Calliope is buggy,
parity either matches the bug or classifies the difference with a reference to
`bench/BUGS.md`.

## Licence

Apache-2.0, see `LICENSE`. The math formulation is reused from Calliope
(Apache-2.0, (c) Calliope developers); HiGHS is MIT-licensed.
