# calliope-rs: technical specification

Parity target: Calliope **0.7.0** (`../calliope`, `v0.7.0-6-g088eb7e4`).
Solver: HiGHS only. No Pyomo or Gurobi backend.
For phasing and order of work see `PLAN.md`.

## 1. Goals

1. **Feature parity:** the same model YAML, data tables, math YAML, config keys, CLI and
   result files give the same optimisation problem and results as Calliope 0.7.0.
2. **Measurable added value** on at least one of:
   - **Correctness:** finds bugs in Calliope or in user math.
   - **Complexity:** a smaller and simpler core.
   - **Performance:** build time and peak memory.
   - **Distinctive features:** things Calliope can't offer at all.

   Section 7 defines how each is measured.

Non-goals: the Pyomo and Gurobi backends, Jupyter integration, plotting, and
generating the documentation site. LaTeX math export is optional and comes last.

## 2. Architecture

A Cargo workspace. Each crate maps to a Python module, so parity can be tracked file by file.

| Crate | Responsibility | Python counterpart |
|---|---|---|
| `calliope-array` | Labelled N-d arrays, coordinates, alignment, masks | xarray (implicit) |
| `calliope-yaml` | YAML with `import:`, overrides, dotted keys | `attrdict.py`, `io.py` (YAML) |
| `calliope-schema` | Typed config, model definition, math and data-table schemas, with validation | `schemas/*` |
| `calliope-preprocess` | Model definition → input dataset: templates, scenarios, data tables, time | `preprocess/*` |
| `calliope-math` | Parsing, type-checking and compiling `where` strings, expressions and helper functions into an IR | `backend/{where,expression}_parser.py`, `parsing.py`, `helper_functions.py` |
| `calliope-lp` | IR + inputs → sparse LP/MILP; HiGHS interface; LP writer | `backend/backend_model.py`, `highs_backend_model.py` |
| `calliope-model` | `Model` lifecycle: init, build, solve, the plan/operate/SPORES modes, results and postprocessing | `model.py` |
| `calliope-io` | NetCDF and CSV read/write, attribute (de)serialisation | `io.py` |
| `calliope-cli` | The `calliope` binary | `cli.py`, `util/generate_runs.py` |
| `calliope-py` (optional, later) | PyO3 bindings that return xarray-compatible results | public Python API |
| `xtask` | Parity harness, golden files, benchmarks | none (new) |

Dependency direction:
`array ← yaml ← schema ← preprocess ← math ← lp ← model ← io/cli`.

## 3. Component specifications

### 3.1 `calliope-array`
The storage backend is an **open decision**, made in Phase 2 by a spike (`PLAN.md`). The
public API below is the same either way, so `calliope-math` doesn't depend on the choice.

- `Coord`: a named dimension with ordered labels of type `string`, `datetime`, `date`,
  `int` or `float` (the `Dimension.dtype` options in `math_schema.py`).
- `LArray<T>`: an array labelled by `Vec<Coord>`. The element types are `f64` (NaN =
  missing), `bool`, `i64`, string IDs (interned `u32`) and datetimes (`i64` ns).
- Candidate storage:

  | | A: dense `ndarray` | B: Polars long format |
  |---|---|---|
  | Layout | `ndarray::ArrayD<T>` over the full Cartesian product of coordinates | `DataFrame` with one categorical column per dimension and a value column; only active cells are stored |
  | Broadcasting | stride tricks, no copy | join on shared dimension columns |
  | `where` | elementwise mask | filter |
  | `sum(over)` | axis reduction | `group_by` on the remaining dimensions |
  | `roll`/`shift` | index arithmetic | `shift` over the time column within groups |
  | Strengths | fast on small, fully populated arrays; simple; light dependency | memory scales with the active cells; feeds the COO/CSC build directly; vectorised and parallel; same model as Pyoframe |
  | Risks | the full product of dimensions for nodes × techs × carriers × timesteps; needs a separate sparse layer | `null` vs `NaN` and join fill rules differ from xarray outer joins; joins cost more on small arrays; Rust API churn; compile time and binary size |

  Decision criteria: build time and peak RSS on the spike, the effort needed to match
  xarray missing-value semantics, and the code size of the helper functions.
  A hybrid is allowed: dense for small parameters indexed by nodes and techs, long format
  for anything with a time dimension.
- Operations, matching xarray semantics exactly:
  - outer-join broadcasting by dimension name;
  - `where`, `sel`, `isel`, `fillna`, `sum(over)`, `any(over)`, `shift`/`roll`;
  - group-by sum and mean, and resampling (`mean`, `sum` with `min_count=1`, `first`).
- Sparse masks: everything that is indexed on `foreach` (constraints, variables,
  expressions) is stored as a **coordinate list over the active mask** (COO), not as dense
  arrays. Large hourly models are mostly empty.
- Missing-data semantics are documented and tested as a unit (NaN in arithmetic,
  NaN → False in `where`, defaults filled at evaluation time; see CHANGELOG 0.7.0).

Libraries: `indexmap`, `chrono`, `smallvec`, plus `ndarray` (A) or `polars` (B).
`polars` is used in `calliope-preprocess` whichever option is chosen.

### 3.2 `calliope-yaml`
- A YAML 1.2 loader (`serde_yaml` or `saphyr`) that keeps key order.
- `AttrTree`: nested map with dotted-key get, set and union. `allow_override`
  semantics match `AttrDict.union`.
- `import:` resolved relative to the importing file, recursively, with cycle detection.
- Overrides: `scenarios` (lists, including nested scenario references), `overrides`
  and the CLI `--override_dict`. The merge order must match `_combine_overrides`.
- Errors carry the source file, line and key path. This improves on Calliope, which
  loses line information after merging.

### 3.3 `calliope-schema`
Typed `serde` structs, one per pydantic model:
- `CalliopeConfig { init, build, solve }`:
  - `init`: subset, resample, `time_cluster`, `mode` (`base`/`operate`/`spores`),
    `extra_math`, `math_paths`, `broadcast_input_data`, `retain_inactive`, date formats,
    `distance_unit`.
  - `build`: `operate.window` and `operate.horizon`, `objective`, `ensure_feasibility`.
  - `solve`: the SPORES block (four scoring algorithms: `integer`,
    `relative_deployment`, `random`, `evolving_average`), `shadow_prices`,
    `zero_threshold`, `solver_options`, `save_logs`.
  - `build.backend`: the default is `pyomo`, so existing models set it implicitly.
    `pyomo` and `gurobi` give a warning and fall back to `highs`, and unknown values
    are an error.
- `ModelDef { data_definitions, data_tables, nodes, techs, templates, overrides, scenarios }`:
  - `CalliopeTech` with `base_tech` ∈ {conversion, demand, storage, supply, transmission}.
  - Transmission adds `link_from`, `link_to` and `one_way`.
  - Indexed data is given as `{data, dims, index}`.
- `DataTable { table, rows, columns, select, drop, add_dims, rename_dims, template }`.
- `Math` with the sections `dimensions`, `parameters`, `lookups`, `variables`,
  `global_expressions`, `constraints`, `piecewise_constraints`, `objectives`,
  `postprocessed` and `checks`. These carry `foreach`, `where`, `equations`,
  `sub_expressions`, `slices`, `order`, `bounds`, `domain`, `default`, `resample_method`
  and `unit`.
- `deny_unknown_fields` everywhere. A JSON Schema is generated with `schemars` and
  diffed against the pydantic-generated schema, to show the two schemas match.

### 3.4 `calliope-preprocess`
Pipeline:
1. Model definition: load YAML, apply scenarios and overrides, resolve templates
   (`TemplateSolver`, including nested templates and cycle detection).
2. Math: load `base.yaml` plus `extra_math` and user math, in the priority order of
   `get_math_priority`.
3. Data: `data_definitions`, per-node and per-tech values, and data tables. Tables are
   read with **`polars`** (CSV and Parquet, lazy scan) and reshaped with `select`,
   `drop`, `add_dims`, `rename_dims` and `pivot_values_to_dim` for lookups. CSV type
   inference is set explicitly to match pandas `read_csv` on the fixtures (the
   `data_tables` golden files).
4. Dataset building (`ModelDataBuilder`):
   - assign transmission techs to their nodes from `link_from` and `link_to`
     (`model_data.py:471`);
   - activate and deactivate techs and nodes;
   - expand `carrier_in` and `carrier_out`;
   - compute distances (geodesic, `geographiclib` equivalent: the `geographiclib` crate);
   - set dtypes per `ModelDTypeUpdater`.
5. Time:
   - parse datetimes with `datetime_format`;
   - subset;
   - resample, with the method per parameter (`polars` `group_by_dynamic`, with bin
     edges matched to pandas `resample`);
   - cluster (representative days, `timestep_cluster`, `timestep_weights`,
     `lookup_cluster_*`);
   - infer `timestep_resolution` and `timestep_weights`.
6. Run `checks` from the math (raise or warn).

Output: `InputDataset`, a map of `LArray`s plus attributes. It can be exported to NetCDF
for diffing against the Python `model.inputs`.

### 3.5 `calliope-math`
The core of the port. There are three stages, and each can be tested on its own.

**Parse.** One `pest` grammar covers both languages:
- `where` strings: `and`, `or`, `not`, comparisons, `config.<key>`, input-presence
  checks, `[a, b] in dim` subsets, and the `where`-context helpers (`any`, `sum`, …).
- Equations: `lhs (<=|>=|==) rhs`, arithmetic with `+ - * / **`, unary sign,
  `$sub_expressions`, `component[dim=$slice]`, and helper calls with args and kwargs.

The grammar is checked against all strings in `math/*.yaml`, the tests'
`*_math_tests.yaml` and the parser unit tests (`where_parser_test.py`,
`expression_parser_test.py`).

**Typecheck.** This stage is new; Calliope has no counterpart.
- Infer each sub-expression's dimension set and kind (constant, parameter, variable,
  affine expression, bool mask).
- Before anything is built, reject:
  - dimensions missing from `foreach`;
  - summing over a dimension that isn't present;
  - non-linear terms such as variable × variable in an LP;
  - comparisons between incompatible dtypes;
  - sub-expression and slice combinations that never match.
- Optionally check units, using the `unit` fields on parameters, variables and expressions.
- Report errors with the YAML location and a caret on the failing span.

**Compile and evaluate.**
- Output is an IR of vectorised operations over `LArray`s, and of affine expressions
  stored as sparse `(row index, var column, coefficient)` triples plus a constant per row.
- Helper functions (`helper_functions.py`), each implemented for the `where` and/or
  `expression` contexts, as `ALLOWED_IN` specifies: `any`, `sum`, `reduce_carrier_dim`,
  `select_from_lookup_arrays`, `get_val_at_index`, `roll`, `where`, `map_dim`,
  `group_sum`, `group_datetime`, `sum_next_n`.
- Expansion order follows `equations` × `sub_expressions` × `slices`, with the same
  component naming as Calliope, so that LP files can be compared.

### 3.6 `calliope-lp`
- `LpModel`:
  - columns: variables with bounds from parameters or constants, `real` or `integer`;
  - rows: constraints;
  - piecewise constraints: SOS2 or an equivalent MILP formulation, matching Calliope's
    output;
  - objective: `sense`, with the objective selected by `build.objective`;
  - global expressions: kept as affine expressions that are evaluated after the solve.
- Updatable in place for the backend interface:
  - `update_input` rebuilds only the affected rows and coefficients, through a
    parameter → row/coefficient dependency index;
  - `update_variable_bounds`, `fix_variable` and `unfix_variable`;
  - activate and deactivate components;
  - `delete_component`.
- HiGHS through the `highs` crate (or `highs-sys` for `passModel` with CSC matrices):
  `solver_options` are passed through, logs go to `save_logs`, and warm start is used
  for operate and SPORES.
- Results: primal values, duals for `shadow_prices`, termination condition and
  objective value.
- Writers: LP (CPLEX format, with names matching Calliope's `to_lp`) and MPS.
  `verbose_strings` gives human-readable names.

### 3.7 `calliope-model`
- The `Model` type with `new(model_def, scenario, overrides)`, `build()`, `solve()`,
  `inputs()`, `results()`, `backend()` and `info()`.
- Modes:
  - **base:** a single solve.
  - **operate:** rolling horizon with `window` and `horizon`. Fixed capacities come
    from inputs or earlier results, and storage is carried over between windows
    (`_recalculate_storage_initial`). Results are concatenated.
  - **SPORES:** a baseline solve, then a cost-slack constraint and `number` iterations
    with re-scoring through the four scoring algorithms. Supports
    `tracking_parameter`, `score_threshold_factor`, `use_latest_results` and
    `save_per_spore_path`.
- Postprocessing: the `postprocessed` math components, values below `zero_threshold`
  cleaned to zero, and default dims and attrs in the results.
- Runtime attributes: `applied_overrides`, `scenario`, `timings`,
  `termination_condition` and `math_priority`.

### 3.8 `calliope-io`
- NetCDF4 via the `netcdf` crate (libnetcdf). The layout must be readable by Python
  `calliope.read_netcdf`, including:
  - attribute serialisation (`_serialise` and `_deserialise`: bools, None, dicts as
    YAML strings, lists);
  - math and config stored in attributes;
  - SPORES dimensions.
- Round trip: reading a NetCDF written by Python Calliope restores the model.
- CSV export with the layout of `save_csv`.
- Optional: Parquet or Zarr export (not available in Calliope).

### 3.9 `calliope-cli`
Subcommands in parity with `cli.py`, with the same flags:
- `calliope new <path> [--template national_scale|urban_scale]`;
- `calliope run <model.yaml> [--scenario] [--override_dict] [--save_netcdf] [--save_csv] [--save_lp] [--debug] [--quiet]`;
- `calliope generate_runs` (bash, bsub, sbatch and Windows scripts);
- `calliope generate_scenarios`.

New subcommands, which are part of the value case:
- `calliope check <model.yaml>`: validate config, model definition and math without
  building, including the static typecheck from §3.5.
- `calliope lp-diff <a.lp> <b.lp>`: canonicalise and diff two LP files.
- `calliope inspect <model.yaml> --component <name>`: show the active index set, the
  `where` mask and the expanded equations for one component.
- `calliope bench <model.yaml>`: time and peak memory for each stage.

The example models (`national_scale`, `urban_scale`) are embedded with `include_dir`.

## 4. Tools (the `xtask` crate)

| Tool | Purpose |
|---|---|
| `xtask golden` | Runs Python Calliope (pixi env from `../calliope`) to produce reference inputs NetCDF, LP files, results NetCDF and timings for each fixture |
| `xtask parity` | Runs calliope-rs on the same fixtures and compares results (below) |
| `xtask bench` | Runs both implementations on scaled synthetic models (nodes × hours) and reports time and RSS for each stage |
| `xtask schema-diff` | Compares the JSON Schema from `schemars` with Calliope's pydantic schema |
| `xtask fuzz` | `cargo-fuzz` targets for the YAML loader, the `where` parser and the equation parser |

Comparison rules:
- **Inputs:** NetCDF diff. Same variables, coordinates and dtypes; values within 1e-12;
  NaN matches NaN.
- **LP:** canonical form. Rename variables and constraints by
  `(component, sorted index tuple)`, sort rows and terms, and normalise numbers to
  relative 1e-9. Then the files must match exactly.
- **Results:** objective within relative 1e-6. Variable values may differ in degenerate
  LPs, so compare the objective, the postprocessed aggregates and the
  `example_models_test.py` assertions.

## 5. Test corpus

| Source in `../calliope` | Used for |
|---|---|
| `tests/math/lp_files/*.lp` (126) + `tests/math/*_math_tests.yaml` | LP parity per math component |
| `tests/example_models/example_models_test.py`, `src/calliope/example_models/*` | End-to-end results |
| `tests/backend/where_parser_test.py`, `expression_parser_test.py`, `helper_functions_test.py` | Parser and helper unit tests, ported as table-driven tests |
| `tests/preprocess/**` and `tests/common/test_model` | Preprocessing fixtures, overrides, templates, data tables |
| `tests/io_test.py` | NetCDF and CSV round trip |
| `docs/examples/*` (modes, MILP, piecewise) | Operate and SPORES scenarios |

On top of that:
- `proptest` for array alignment (compared with an xarray oracle on generated cases);
- differential fuzzing: generate random small models, then run both implementations and
  compare the LPs.

## 6. Engineering constraints
- Rust stable, edition 2024, `#![forbid(unsafe_code)]` except in FFI crates.
- `thiserror` for library errors and `miette` for user-facing diagnostics with source
  spans.
- `tracing` for logging, with per-stage timings put into runtime attributes.
- `rayon` for parallel component evaluation, since components are independent once
  their dependencies are built.
- CI: `cargo fmt`, `clippy -D warnings`, tests, and nightly parity and benchmark runs.
  Action versions are checked against the latest releases at the time of writing.

## 7. Evaluating the added value

| Claim | How it is measured | What would make it hold |
|---|---|---|
| **Finds bugs** | Every divergence in `xtask parity` is triaged as a Rust bug, a Calliope bug or unspecified behaviour; static typecheck errors on built-in and user math; fuzz findings | Divergences that are Calliope bugs get filed upstream. Count them. |
| **Less complexity** | Lines of code per subsystem against the Python counterparts (about 14k Python lines plus pydantic and pyparsing); number of runtime type checks removed | The math engine plus arrays are smaller than `backend/*` (about 8.2k lines), with stronger guarantees |
| **Faster** | `xtask bench`: time and RSS for preprocess and build (the solve is the same HiGHS) on 1–1000 nodes at 8760 h | Python builds per-element expression objects (`np.frompyfunc`, `backend_model.py:794`; `highs_linear_expression` per cell). Building CSC matrices directly should give large build speedups and memory savings. The target is ≥10× faster build and ≥5× less peak memory. |
| **Stands out** | Features Calliope lacks | A static binary, `calliope check` with source-located errors, `lp-diff` and `inspect`, Parquet/Zarr output, incremental re-solve, and a WASM build of the math checker (e.g. for an online math playground) |

The honest limit: once a model is built, solve time is HiGHS in both cases. Where the
solve dominates, total run time improves only by the build share. The benchmark
report has to show the share for each stage.
