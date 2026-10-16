# Phase 9 evaluation: calliope-rs against Calliope 0.7.0

Measured 2026-10-16 on the Phase 0 machine (Xeon 8268, 16 vCPU, 62 GiB; see
`bench/BASELINE.md`), with nothing else compiling. Raw data is in `bench/results/phase9/`:
one JSON per run, plus `matrix.log`, `tables.md`, `parity.txt`, `cargo_test.txt`,
`loc.json`, `memprobe.txt` and `build_info.txt`. Scripts: `bench/run_phase9.py`,
`bench/report_phase9.py`, `bench/loc.py`, `bench/memprobe/`.

## SPEC §7, filled in

| Claim | Result | Holds? |
|---|---|---|
| **Finds bugs** | 14 confirmed upstream bugs (`bench/BUGS.md`: 11 suspected from reading the code, 3 found while building the oracle). 9 math findings from the parser and typecheck (`docs/findings.md` M1–M9: 1 likely bug, 2 confirmed bugs, 6 quirks or silent failures). Parity triage found no new Calliope bug. The 13 classified differences are 1 known bug (#14), 3 rejected scenarios (#12), 2 degenerate or solver vertices and 1 MIP gap. None filed upstream so far. | **Yes, but mainly from building the oracle and grammar, not from parity divergences** |
| **Less complexity** | 26.7k Rust code lines against 8.5k Python (6.9k for equal features). Math engine plus arrays: 7.4k against 3.2k. | **No** (null result) |
| **Faster** | The build is 5–76× faster than `main` and 5–72× faster than `feat/highs-speedups`. It reaches ≥10× up to 50 nodes × 168 h and at 5 nodes up to 8760 h. It misses it at 50×2190 (7.8×), 50×8760 (9.9×) and 250×168 (5.4×). Operate is 56–63× faster end to end. Models where the solve dominates gain 1.1–1.3× end to end. Peak memory is 1.2–1.9× lower on every large model, against a ≥5× target. 250×2190 still does not fit in 28 GB. | **Build yes, memory no** |
| **Stands out** | `calliope check` (37 static diagnostic codes with source spans), `lp-diff`, `inspect`, `bench`, piecewise constraints on HiGHS, `unfix_variable`, in-place `update_input`, Parquet tables, seeded SPORES `random`, deterministic overrides, PyO3 bindings. Not delivered: static binary, WASM checker, Zarr, cargo-fuzz, native math docs. | **Partly** |

## 1. Speed and memory

The Python solve column is `solve + load_results`; the Rust solve includes loading results.
The small cases are the median of 3 runs, with a spread under 3%: syn 5×168 and 5×2190,
operate 10 d, SPORES 10 and 31 d, and for Rust also operate 31 d. All other cases are
single runs. HiGHS ran with `threads=1, random_seed=0` in every variant. The full table is
in `bench/results/phase9/tables.md`.

| case | build s main / speedups / rust | total s main / rust | peak RSS MB main / rust | rust solve share |
|---|---|---|---|---|
| national 8760 h | 34.9 / 22.7 / **2.02** | 276 / 247 (1.1×) | 1113 / 907 | 99% |
| urban 8760 h | 59.9 / 39.8 / **4.02** | 122 / 59 (2.1×) | 1768 / 1458 | 91% |
| syn 5×168 | 4.3 / 4.0 / 0.06 | 6.6 / 0.29 | 228 / 56 | 59% |
| syn 5×2190 | 10.1 / 7.1 / 0.48 | 27.1 / 14.5 (1.9×) | 415 / 257 | 96% |
| syn 5×8760 | 34.8 / 23.4 / 1.93 | 228 / 195 (1.2×) | 1062 / 908 | 99% |
| syn 50×168 | 13.5 / 10.7 / 1.17 | 127 / 99 (1.3×) | 570 / 325 | 98% |
| syn 50×2190 (build only) | 142 / 104 / 18.2 | – | 2885 / 1695 | – |
| syn 50×8760 (build only) | 707 / 569 / 71.7 | – | 11006 / 6692 | – |
| syn 250×168 (build only) | 196 / 180 / 36.0 | – | 4183 / 2217 | – |
| syn 250×2190 | Python: over 28 GB in Phase 0, not re-run | Rust: **killed, RSS > 28 GB** | | |
| syn 250×8760 | Python: skipped | Rust: **killed, RSS > 28 GB** | | |
| operate 10 d | 3.0 / 2.9 / 0.02 | 42.7 / 0.77 (56×) | 215 / 59 | 70% |
| operate 31 d | 3.1 / 2.9 / 0.02 | 122 / 1.94 (63×) | 220 / 59 | 87% |
| SPORES 10 d | 4.7 / 4.2 / 0.07 | 14.5 / 4.2 (3.4×) | 248 / 71 | 92% |
| SPORES 31 d | 6.0 / 5.0 / 0.18 | 46.3 / 33.4 (1.4×) | 323 / 150 | 99% |

The Python numbers reproduce `BASELINE.md` within about 5%. In operate mode Rust builds
each window inside the solve stage, so the build column means little there; compare the
totals.

What this shows:

- **The build is no longer a cost on models up to 50 nodes.** It falls from 13–49% of the
  run to 1–7%. The run time of national, urban and 5×8760 is now HiGHS: 99%, 91% and 99%
  of it. The Amdahl ceilings in `BASELINE.md` (1.2× national, 2.2× urban) are reached:
  1.1× and 2.1×.
- **Operate is the clear win: 56–63× end to end.** In Python, 95% of that run went to
  `update_input` and `load_results`.
- **The build speedup shrinks as nodes grow**, from 17× at 5 nodes × 8760 h to 5.4× at
  250×168.
  - Rust build time follows the dense nodes × techs × timesteps product, not the LP size.
  - 250×168 has 1.34 M nonzeros and builds in 36 s with 2.2 GB. 50×8760 has 14.0 M
    nonzeros and builds in 72 s (see `lp` in the result JSONs).
  - The build has not been profiled yet.
- **Memory falls only 1.2–1.9× on large models, against a ≥5× target.** Ratios of 3.5–6×
  appear only on small models, where most of Python's RSS is its 160–200 MB interpreter
  and import baseline. In runs where the solve dominates, the peak is HiGHS's own memory.
- **Preprocessing is not always faster.**
  - Slower at 50×8760: 13.6 s in Rust against 10.0 s in Python.
  - At 250×2190, Rust takes 70 s; Python was killed after at least 50 s.
  - Faster at 250×168 (10.8 s against 27.4 s) and on the example models (0.2 s against
    1.1 s).

### Why 250×2190 still does not fit: preprocessing memory

`target/release/calliope check` runs preprocessing and the typecheck only. It peaks at
**11.8 GB at 250×2190**, about the same as Python's 11.9 GB preprocessing peak in Phase 0.
The `bench/memprobe` breakdown shows where the memory goes
(`bench/results/phase9/memprobe.txt`; two runs agree within 0.1%):

| stage (`calliope_preprocess`) | time | peak RSS | dataset after the stage |
|---|---|---|---|
| definition, math | 0.0 s | 10 MB | – |
| `DataTable::load` (CSV) | 0.9 s | 219 MB | `source_use_max` and `sink_use_equals`: 1.1 M cells each (2190 × 250 nodes × 2 techs), 9 MB each |
| `ModelDataBuilder::new`, via `init_from_data_tables` and `add_to_dataset` | 17.7 s | **11 761 MB** | the two arrays reindexed to 2190 × 250 × 254 = 139 M cells each, `Float`, 1113 MB each |
| `ModelDataBuilder::build` (node and tech YAML data, more `add_to_dataset` calls) | 38.1 s | **11 708 MB** | 35 variables, 2227 MB |
| `ModelDataCleaner::clean` | 12.9 s | 7 722 MB | 38 variables, 2226 MB; the two arrays are still 1113 MB each |

- **The final inputs are 2.2 GB of dense `f64`, and 99.8% of it is NaN.** Calliope's own
  `model.inputs` at 250×168 has 25 200 non-null of 10.7 M cells in `source_use_max` and
  16 800 in `sink_use_equals`. These two are the only variables over
  timesteps × nodes × techs.
- **The peak is about 5× the final size**, from transient copies made while these two
  arrays are merged:
  - `add_to_dataset` (`crates/calliope-preprocess/src/model_data.rs:684-690`) calls
    `Dataset::merge(.., Join::Outer, ..)` and then `merged.fillna(&self.dataset)`. Each
    call therefore holds the old, the merged and the filled dataset at once. It runs once
    per table and again for every YAML data addition in `build`.
  - The generic array (`xr`) code paths go through `Obj` cells. `Obj` is an enum holding an
    `Arc<str>`, so each cell takes 24 bytes instead of 8 for an `f64`.
    - The affected paths include `DataArray::zip_with` (`xr/array.rs:420-431`),
      `Data::objs()`, `where_else`, and `Data::from_objs` followed by `cast`.
    - `where_` (`xr/array.rs:263-275`) also builds a `Vec<Option<usize>>` (16 bytes per
      cell) and a `Vec<bool>` mask.
  - `clean_data_from_undefined_members` uses these `where_` and `zip_with` paths, then
    calls `drop_undefined(self.dataset.clone(), ..)`, which copies the whole dataset. This
    explains the 7.7 GB peak in `clean`.
- **ADR 0002 (`docs/adr/0002-preprocess-dataset.md`) chose this on purpose.** It says
  memory is "dense like xarray's during preprocessing (same as Python)". Its premise, "a
  step that runs once on small arrays", does not hold at 250 nodes.
- **The build would probably not fit after a sparse preprocess either.** Build RSS at
  250×168 is 2.2 GB. Scaling with timesteps gives about 29 GB at 2190 h (not measured).

Follow-up, in order of effect:

1. Keep data-table variables in their long, sparse form until they are converted to
   `LArray` for the math. Expand them to dense arrays only when writing `inputs.nc`.
2. Add typed `Float` and `Bool` fast paths to `merge`, `fillna`, `where_` and `zip_with`
   that avoid `Obj` and the `Option<usize>` index.
3. Merge in place in `add_to_dataset`, and remove the clone in `drop_undefined`.
4. Profile the build at 250×168 to find why its cost follows the dense product rather than
   the nonzeros.

## 2. Complexity

`bench/loc.py` counts code lines only: blank lines, comments, docstrings and doc comments
are excluded. Rust `#[cfg(test)]` modules and `tests/` directories count as tests.

| subsystem | Rust code | Rust tests | Python code (raw lines) |
|---|---|---|---|
| YAML + config schemas (yaml, schema) | 5047 | 2299 | 1026 (1650) |
| Preprocess | 4887 | 1489 | 1243 (1993) |
| Arrays (xarray subset) | 1545 | 629 | – (xarray) |
| Math engine | 5810 | 1961 | 2672 (5329) |
| LP model + LP format/diff | 1880 | 370 | – |
| HiGHS backend | 1293 | 449 | 487 (656) |
| Model, modes | 1951 | 1911 | 560 (861) |
| I/O | 993 | 478 | 210 (333) |
| CLI | 1561 | 402 | 491 (680) |
| Misc / Pyomo, Gurobi, LaTeX backends, math docs | – | – | 173 + 1638 |
| Python bindings / xtask | 808 / 927 | 0 / 125 | – |
| **total** | **26 702** | **10 113** | **8 500 (14 201)** |

- **For the same features, the Rust core is 3.6× the size of the Python.** Without the
  bindings and xtask, Rust has 24 967 code lines. Python has 6 862 without the Pyomo,
  Gurobi and LaTeX backends and the math docs.
- **The SPEC's complexity target is missed in substance.** Math engine plus arrays is
  7 355 lines. That is below the 8.2k raw lines of Python's `backend/*`, but those include
  three backends the port does not have. Against the Python code with the same features
  (parsers, `backend_model`, HiGHS backend: 3 159 lines) it is 2.3×.
- **Part of the extra code is new functionality**: the typecheck (1 225 lines), LP parsing
  and diffing (1 224), `check`, `inspect`, `bench` and Parquet input.
- **The rest re-implements what Python gets from its dependencies**:
  - pydantic schemas and validation semantics (3 218 lines in `calliope-schema`);
  - a subset of xarray (`calliope-array`, plus about 700 lines of array code in
    preprocess);
  - pandas CSV type inference and float parsing;
  - ruamel YAML behaviour.
- **Python's dependency weight**: xarray 107k lines, pandas 250k, numpy 112k, pydantic 46k
  plus a 4.7 MB `pydantic_core` binary, pyparsing 12.9k, Pyomo 260k. The pixi `default`
  environment is 794 MB with 81 conda packages, Gurobi included.
- **Rust's dependencies**: 146 third-party crates in `calliope-cli`'s tree and 249
  packages in `Cargo.lock`.
- **Release binary**: 84 MB with line tables, 17 MB stripped. It is **not static**: it
  links 47 shared libraries, including the system NetCDF, HDF5 and curl
  (`build_info.txt`).
- **Clean release build of `calliope-cli`**: 59 s on 16 vCPU, 178 compile units including
  HiGHS (C++).

Runtime and static checks (these are `grep` counts, so only proxies):

- **Python** has 91 `isinstance`, 40 `hasattr`/`getattr`, 10 pydantic validators, 111
  `fillna`/`notnull`/`isnull`/`.where(` and 18 `type: ignore`.
- **In the math, Rust replaces Python's convention-based handling of missing values with
  typed `LArray`s (ADR 0001).** The dynamic checks did not disappear; they moved to the
  YAML and config boundary:
  - 401 matches on `Value::*`;
  - 218 dynamic `as_*` accessors;
  - also 173 `unwrap`/`expect` and 16 `panic!`/`unreachable!` in non-test code.
- **`unsafe` is confined to `calliope-highs/src/ffi.rs`** (26 blocks).
- **Static checks are new**: 37 typecheck diagnostic codes (dimensions, linearity, dtype,
  `where`, helpers) run before the build. On the shipped math they report only M1.

## 3. Correctness value

Re-run 2026-10-16:

- **`cargo run -p xtask -- parity`: 94 pass, 13 classified, 0 fail** (`parity.txt`).
- **`cargo test --workspace`: 384 pass, 0 fail, 1 ignored** (`cargo_test.txt`).
- **The Python bindings test suite was not re-run.** It needs a maturin build in a fresh
  virtual environment, and the disk had 1.1 GB free. The last recorded result is 30 tests
  passing (`docs/findings.md`, Phase 8).

Caveats on "pass":

- **Operate and SPORES result arrays do not count toward pass or fail**
  (`crates/xtask/src/parity.rs:44,364-368`). For those fixtures, "pass" means the inputs,
  LP, objectives and termination status match.
- **All 4 SPORES fixtures currently end on different optimal solutions**, giving 40
  "vertex/array (not failing)" notes. This includes
  `national_scale_spores_evolving_average`, where 1–61 cells differ per variable (for
  example `capacity_factor` in 61 of 38 400 cells).
- **This contradicts `docs/findings.md` P1**, which says evolving_average "reaches
  Calliope's vertex". Only its objectives match; that entry should be corrected.
- **The LP is identical to Calliope's in every case that has one.** In `lp_parity`,
  122/122 math LP fixtures and 34/34 example LPs are equal after canonicalisation.

Bug tally:

| Source | Count | Notes |
|---|---|---|
| Upstream runtime bugs with repros (`bench/BUGS.md`) | 14 confirmed | 4 crash or change results: #1 single-window operate, #7 non-optimal MILP gives empty results, #8 `horizon < window` drops data, #14 unknown objective gives a zero objective. 2 are docs/code mismatches in semantics (#2, #9). 8 concern messages, the CLI, docs or code that does nothing. |
| Math layer (`docs/findings.md` M1–M9) | 1 likely bug (M1, the `total_curtailment` denominator), 2 confirmed (M2: `-$sub` raises a KeyError; M3: names depend on the hash seed), 6 quirks or silent failures | Found by mirroring the pyparsing grammar (1958 corpus and mutant cases) and by the typecheck |
| Parity triage (R1–R5, P1, P2) | 0 new Calliope bugs | 1 known bug (#14), 1 pandas float-parsing quirk (P1), 1 HiGHS scheduler pitfall (P2) |
| Intentional deviations | 13 | e.g. a single operate window is solved, `horizon < window` is rejected, `tracking_parameter: false` is honoured, non-optimal MILPs return the incumbent's results, `--save_logs` works, SPORES `random` is seeded |
| Filed upstream | 0 | The SPEC asks for this; not done |

## 4. Where it does not help (limits)

- **Models where the solve dominates.** HiGHS is 91–99% of the Rust run on national, urban
  and every synthetic size that was solved, so the end-to-end gain there is 1.1–1.3×.
  SPORES 31 d gains 1.4×.
- **HiGHS version gap.** `highs-sys` is pinned to 1.15.0
  (`crates/calliope-highs/Cargo.toml`), while the oracle uses 1.15.1. Together with
  highspy's term merging and row orientation (R1), this is why urban and the SPORES
  fixtures end on different optimal solutions. Solve times differ by a few percent either
  way (national 245 s against 240 s, single runs).
- **Memory at scale.** See §1. The Phase 0 conclusion, that memory decides which sizes are
  feasible, still holds for the port.
- **Math documentation** still comes from Python: `calliope_rs.math_documentation` calls
  Calliope's `MathDocumentation` on the model's NetCDF. There is no native port.
- **Math-formulation bugs and docs drift.** The port reuses `math/*.yaml` unchanged. It
  catches drift only where the typecheck or the schema does.
- **Upstream velocity.** Every math or schema change has to be ported twice, and the Rust
  code is 3.6× the size.
- **Python users.** The bindings exist but return values only (`as_backend_objs=True`
  raises an error).

## Answer

- **Does it find bugs?** Yes: 14 runtime bugs and 3 math bugs in Calliope, plus 6 silent
  failures or quirks. They came from building the oracle and mirroring the grammar and
  schema. The parity diffs themselves found nothing new, and nothing has been filed
  upstream yet.
- **Does it reduce complexity?** No. It is 3.6× the Python code for the same features,
  because Rust has to re-implement what Python gets from pydantic, xarray and pandas. It
  does give stronger guarantees: typed missing values and static math checks.
- **Is it faster?** The build is 10–20× faster up to 50 nodes × 168 h and at 5 nodes up to
  8760 h, but only 5–10× at 50 nodes × 2190/8760 h and at 250 nodes. Operate runs are
  56–63× faster end to end. Runs where the solve dominates gain 1.1–2×. Memory falls only
  1.2–1.9×, and the largest size still does not fit, because preprocessing stays dense
  (ADR 0002).
- **Does it stand out?** Partly. It has `check`, `lp-diff`, `inspect`, `bench`, piecewise
  constraints and `unfix_variable` on HiGHS, and deterministic runs. A single static
  binary, a WASM build and native math docs are not delivered.

## Follow-up (2026-10-16)

Follow-ups 1–3 of §1 are done, and the parity caveat of §3 is resolved. Raw data:
`bench/results/followup/` (`memprobe.txt`, result JSONs, `parity.txt`, `cargo_test.txt`).
All runs are release builds on the same machine, single runs.

### Preprocessing memory

Mostly empty data-table arrays are now stored sparsely (`Data::Sparse`). They stay sparse
through alignment, `where` and `fillna`. They are expanded only when `inputs.nc` is
written, and the math engine gets `LArray`s built from the stored cells. The dense paths
no longer build `Obj` or `Option<usize>` vectors per cell. `add_to_dataset` merges in
place, and `drop_undefined` no longer clones the dataset. See the ADR 0002 addendum.

| `bench/memprobe` | Phase 9 peak RSS | now | Phase 9 time | now |
|---|---|---|---|---|
| 250 × 168 | 922 MB | 28 MB | 8.3 s | 1.6 s |
| 250 × 2190 | 11 761 MB | 206 MB (CSV loading) | 70 s | 5.2 s |
| 250 × 8760 | not measured | 786 MB (CSV loading) | – | 22 s |

At 250 × 2190 the two time series take 5 MB and 4 MB instead of 1113 MB each. All 37
golden fixtures and all 122 math LP fixtures still give inputs equal to Calliope's. They
also do with every data-table float array forced sparse.

### Build (`examples/build_timing`, build-only cases)

| case | preprocess s (Phase 9 → now) | build s | peak RSS MB | Python `main` build s / peak MB |
|---|---|---|---|---|
| syn 250 × 168 | 10.8 → **1.5** | 36.0 → 32.3 | 2217 → **1893** | 196 / 4183 |
| syn 50 × 8760 | 13.6 → **1.3** | 71.7 → 75.2 | 6692 → **6005** | 707 / 11006 |
| syn 250 × 2190 | 70 → **5.5** | killed → 366 | > 28 GB → **24 595** | over 28 GB (Phase 0) |

- **Preprocessing is now faster than Python at the measured sizes**: 8× at 50 × 8760 and
  18× at 250 × 168, against 0.7× and 2.5× in Phase 9.
- **250 × 2190 now builds within the 28 GB cap**: 5.0 M columns, 6.4 M rows, 17.5 M
  nonzeros, built in 6.2 min with a 24.6 GB peak. It was not solved.
- **Peak memory is now set by the build**, not by preprocessing. Against Python, it is
  2.2× lower at 250 × 168 (1.9× in Phase 9) and 1.8× lower at 50 × 8760 (1.6×). That is
  still short of the ≥5× target.
- **Build times are unchanged within run-to-run noise**: single runs, and follow-up 4
  (profiling the build) is not done.

### Correctness, re-run

- **`cargo run -p xtask -- parity`: 90 pass, 17 classified, 0 fail.**
  - Result arrays of operate and SPORES runs now count. Every array difference fails
    unless the fixture is classified.
  - The 4 SPORES fixtures and `urban_scale` are classified `AlternativeOptimum`. Parity
    verifies this classification on every run (`crates/xtask/src/altopt.rs`): it rebuilds
    each solve, for SPORES with Calliope's recorded scores and cost budget, and puts
    Calliope's solution into the port's LP.
  - In all 17 solves, that solution is feasible: the worst row violation is 1.5e-9 and no
    bound is violated. Its objective equals the port's optimum to at most 2.4e-16
    relative.
  - The scores of the port's own SPORES runs match Calliope's to 2e-11.
  - So the array differences are alternative optima of the same LP. `docs/findings.md` P1
    is corrected: `evolving_average` matches Calliope's objectives, not its vertex.
  - The other 12 classified items are unchanged:
    - 9 come from the expected-error fixtures (3 parts each);
    - `docs_piecewise` was solved by CBC in the golden run (SOS2);
    - `check_feasibility` hits bug #14;
    - `urban_scale_milp` differs within its MIP gap.
- **`cargo test --workspace`: 391 pass, 0 fail, 1 ignored.** The ignored test regenerates
  the xarray oracle fixture and needs Python.
- **Python bindings (`crates/calliope-py/tests`): 30 pass.** They were rebuilt with
  maturin in a scratch virtual environment, which was then removed.
- **`cargo fmt --check` and `cargo clippy --workspace --all-targets -D warnings` are
  clean.**
