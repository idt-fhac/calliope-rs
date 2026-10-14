# Porting Calliope to Rust

Reference checkout: `../calliope` (upstream `calliope-project/calliope`, `main` @ `088eb7e4`,
`v0.7.0-6`). Technical design: `SPEC.md`. All `file:line` references below are into `../calliope`.

**Decision:** the port is built, with full feature parity, regardless of the benchmark
outcome. Part of the goal is to show what a typed rewrite does and does not buy. Phase 0
therefore sets the baseline that the port is measured against; it is not a go/no-go gate.

**Target:** the same model YAML, data tables, math YAML, config keys, CLI and NetCDF/CSV
outputs as Calliope 0.7.0. The solver is HiGHS, called from Rust. There is no Pyomo or Gurobi
backend.

## Starting point (facts that shape the plan)

- **Size.** `src/calliope` has 14.2k lines of Python. About 2.3k of that will not be ported:
  the Pyomo, Gurobi and LaTeX backends and the docs generator. On top come 2.2k lines of math
  YAML (`math/*.yaml`) and 1.3k lines of math-test YAML. The reference LP files are 126 small
  ones (192 KB in total) in `tests/math/lp_files`.
- **Upstream already has a HiGHS backend without Pyomo**
  (`backend/highs_backend_model.py`, via `highspy`). It has known gaps:
  - no piecewise constraints (`highs_backend_model.py:470`);
  - no `unfix_variable` (`:449`);
  - no `get_constraint(eval_body=True)` (`:182`);
  - `update_input` always rebuilds every component that references the input
    (`mutable=False`, `:380`), followed by an O(N) Python fix-up of stale row and column
    indices (`:339-375`).

  `origin/feat/highs-speedups` batches the variable and constraint adds, but expressions are
  still one Python object per array cell.
- **Where the build cost comes from.** Every array cell is a Python object:
  - `np.frompyfunc` over object arrays (`backend_model.py:794`);
  - one `addVariable` or `addConstr` call per element (`highs_backend_model.py:96,115`);
  - arithmetic on object-dtype `xarray` arrays (`expression_parser.py:247-265`);
  - `sum_next_n` concatenates T arrays (`helper_functions.py:867-873`), and `group_sum` loops
    over groups in Python (`:704-708`).
- **The Python parser rebuilds its grammar for every `where` string** (`parsing.py:602`). It
  turns on pyparsing packrat as global state when the module is imported
  (`where_parser.py:19`, `expression_parser.py:49`), and it depends on pinning
  `pyparsing != 3.3.3` to avoid upstream `infix_notation` bugs (`pyproject.toml:127-129`).
- **The defaults are not ours to choose.** `build.backend` defaults to `pyomo` and
  `solve.solver` to `cbc` (`schemas/config_schema.py:164,231`). Every existing model
  therefore names a backend we don't ship. The port must accept these values, warn, and use
  HiGHS. Rejecting them would break parity (SPEC §3.3).

## Phases

Each phase lists its scope, its exit criteria and a size estimate. Sizes assume one
experienced Rust developer: Rust LOC including unit tests, and calendar weeks.

### Phase 0: baseline and benchmarks (no Rust; 1–2 wk)

Setup:
- Set up the pixi env in `../calliope`. Run the full test suite and record the versions of
  Python, xarray, pandas, pyparsing and highspy, and the HiGHS version inside highspy.
- Pin HiGHS: the Rust `highs`/`highs-sys` build must use the same HiGHS version as the oracle.

Benchmark matrix:
- Models: `national_scale` and `urban_scale` (8760 h), plus synthetic scale-ups of
  national_scale (nodes × timesteps = {5, 50, 250} × {168, 2190, 8760}).
- Backends: `highs` on `main` and on `feat/highs-speedups`, plus `pyomo`+HiGHS as a
  reference.
- For each stage (preprocess, build, solve, postprocess, NetCDF write), record wall time and
  peak RSS. The stage timings are already in `model.runtime.timings`; RSS is measured with
  `/usr/bin/time -v` or `memray`.
- Operate mode, measured per window, including the rebuild cost of `update_input`.
- SPORES, measured per iteration.

Write a minimal repro for every suspected bug in the table at the end of this plan, and
record the result for each: confirmed, not a bug, or already fixed upstream.

**Exit:**
- `bench/BASELINE.md`, with numbers and machine specs.
- The share of time and peak memory for each stage, for each model size. This sets the
  realistic ceiling for speedup.
- A triage table of the suspected bugs.

### Phase 1: test oracle and harness (≈1.5k LOC Rust + Python scripts; 2–3 wk)

**Golden files** (`xtask golden`, which drives the pixi env):
- `inputs.nc` for:
  - every `tests/common/test_model` scenario used by `tests/math/*_math_tests.yaml`;
  - the seven `calliope.examples` variants (national, clustering, resampling, urban, milp,
    operate, operate_milp);
  - the doc examples in `docs/examples/*`.
- LP files:
  - the 126 existing references (written by Pyomo, using its naming);
  - new LP files for each example model, written the same way.
- `results.nc` for each example model and mode, with `backend: highs` and solver options
  fixed (`threads=1`, `random_seed=0`, same presolve). Python-HiGHS against Rust-HiGHS on an
  identical column and row order is the strongest oracle (see the results comparison below).
- SPORES golden runs pin `np.random.seed`, because the `random` algorithm uses the global
  NumPy RNG (`model.py:801`).

**LP canonicaliser** (also shipped as `calliope lp-diff`):
- Parse the CPLEX LP that Pyomo writes. It has to handle:
  - `c_l_`, `c_u_`, `c_e_` and `r_l_`/`r_u_` row prefixes;
  - `ONE_VAR_CONSTANT` for constant objectives;
  - `bounds`, `general`/`binary` and `SOS` sections.
- Map each row to `(sense, rhs, {var: coef})`, with names in the form
  `component(name)(idx__idx)`.
- Compare without regard to order, with a relative tolerance of 1e-9. Calliope evaluates
  `a / b` as `a * (1/b)` (`expression_parser.py:257-260`), so coefficients carry rounding
  noise.

**Results comparison:**
- Objective within relative 1e-6.
- Arrays (variables, global expressions, postprocessed, shadow prices) within absolute
  1e-6, scaled by magnitude.
- When the column and row order matches Python's, HiGHS normally lands on the same vertex
  and the arrays can be compared strictly. Otherwise, fall back to the objective, the
  postprocessed aggregates and the `example_models_test.py` assertions.
- Operate and SPORES depend on the path taken: compare iteration 0 strictly; for later
  iterations compare the objective, and report vertex differences rather than failing on
  them.

**Parser corpus:**
- Extract every `where`/`expression` string from:
  - `math/*.yaml`;
  - `tests/math/*.yaml`;
  - the docs math gallery;
  - the table-driven cases in `where_parser_test.py`, `expression_parser_test.py` and
    `helper_functions_test.py`.
- Store each string with its Python parse tree (`repr`) and its evaluation result where one
  exists.

**Exit:**
- `xtask golden` is reproducible: two runs give byte-identical NetCDF data variables.
- `lp-diff` reports that Python LP files are equal to themselves, and catches injected
  perturbations (coefficient, sense, missing row, renamed index).

### Phase 2: foundations (≈6k LOC; 5–6 wk)

**Storage spike (first 1–2 wk, decides SPEC §3.1).** Build both candidates, dense
`ndarray` (A) and Polars long format (B), behind the same minimal `LArray` API, and use
them to build one heavy constraint end to end into CSC triples: `system_balance` of
`national_scale` at 8760 h and on the largest Phase 0 synthetic size.
- Measure: build time, peak RSS, lines of code for `sum`, `where`, `roll` and
  `group_sum`, and how many xarray missing-value cases need special handling
  (`null` vs `NaN`, outer-join fill).
- Exit: a decision note (`docs/adr/0001-array-storage.md`) choosing A, B or a hybrid
  (dense for node/tech parameters, long format for anything with timesteps), with
  the numbers. Option B cuts Phase 2/4 effort if Polars covers group-by, shift and
  resampling. Option A keeps the dependency light.

**`calliope-array`.** Labelled N-d arrays, plus a written spec of the xarray behaviour that
Calliope actually relies on:
- outer-join broadcasting by name;
- `where` (scalar and array);
- `fillna`;
- `sum(min_count=1, skipna)`;
- `any`, `roll`, `isin`, and `sel` with lists and slices (inclusive on both ends);
- `stack`/`groupby` (`group_sum`);
- `resample` (`first`/`mean`/`sum`);
- `concat` along a new dimension.

Missing values use `Option`/masks. The NaN-as-missing behaviour is reproduced only where
results depend on it (for example, `sum` of all-NaN gives NaN).

**`calliope-yaml`:**
- `AttrDict.union` semantics: `allow_override`, and the `_REPLACE_` key that wipes a
  sub-tree (`attrdict.py:233-255`);
- dotted keys;
- `import:` with cycle detection. Python has none (`io.py:282-284`).

**`calliope-schema`:**
- serde with `deny_unknown_fields`.
- Reproduce pydantic's `update(overwrite=…)` semantics, including the
  "explicitly set to the default" tracking through `model_fields_set`
  (`schemas/general.py:200-206`). Tech inheritance precedence depends on it.
- Validators: `AttrStr` regex, unique and non-empty lists, `ge`/`le` bounds, `inf`/`nan` in
  numeric values.

**Exit:**
- The proptests of array operations against an xarray oracle pass on generated cases.
- `tests/core/attrdict_test.py` and `tests/schemas/*` are ported and pass.
- `xtask schema-diff` reports no difference to the pydantic JSON schema.

### Phase 3: preprocess (≈3.5k LOC; 4–5 wk; needs Phase 2)

Steps, in Calliope order:

1. **Overrides.** `override_dict`, then scenarios (recursive, comma-separated), then
   `override_dict` again (`model_definition.py:70-143`).
   - Python collects scenario overrides in a `set`, so the order is not deterministic
     (`:172`). Rust uses definition order.
2. **Templates.** `TemplateSolver` (nested, with cycle detection, top-level only) applies to
   any dict, including data tables.
3. **Math.** `initialise_math` and `math_paths`, then `build_math` in the priority order
   base → mode → `extra_math`, then `math_dict`.
4. **Data tables:**
   - CSV header and index levels;
   - `select`, `drop`, `add_dims`, `rename_dims`;
   - the protected-parameter list (`config/data_table_exclusions.yaml`);
   - `pivot_values_to_dim` for lookups;
   - the precedence order: later tables, then YAML.
   - pandas `read_csv` type inference has to be reproduced, including `True`/`inf`/empty
     cells and mixed columns.
   - In-memory `data_table_dfs` has no Rust equivalent. Accept Parquet/Arrow files instead,
     and later DataFrames through PyO3.
5. **Node and tech data:**
   - `_inherit_techs`;
   - links from `link_from`/`link_to`;
   - `active` and `retain_inactive`;
   - `broadcast_input_data`;
   - `data_definitions` (top-level indexed data);
   - dtype coercion per the math (`ModelDTypeUpdater`; bool means NaN→False).
6. **Cleaning:**
   - drop undefined members (`_drop_undefined`);
   - colours;
   - geodesic distances (WGS84, `distance_unit`).
7. **Time:**
   - datetime parsing with `datetime_format`/`date_format`, and `exact=False` for
     non-ISO formats;
   - subset (inclusive; ordered vs unordered dimensions; drop hanging links);
   - resample, with `resample_method` per parameter;
   - infer `timestep_resolution` and `timestep_weights`;
   - cluster (`time.py:133-175`, plus the `lookup_cluster_*` arrays).
   - Only fixed-duration frequencies are supported natively (`h`, `min`, `D`, `W`).
     Calendar offsets (`MS`, `ME`) give a clear error until they are implemented.
8. **Re-init from NetCDF.** Changing `subset`/`resample` with `init` kwargs re-applies them.
   Changing `time_cluster` is an error (`model_data.py:704-732`).

**Exit:**
- `inputs.nc` matches the golden file for every fixture: the same variables, coordinates,
  dtypes and coordinate order, and values within 1e-12.
- `tests/preprocess/*` cases are ported, including the error and warning messages.

### Phase 4: math engine (≈6k LOC; 6–8 wk)

**4a. Parse and typecheck.** Needs only Phase 2, so it can run in parallel with Phase 3.

`pest` grammar:
- `where`: `and`/`or`/`not`, comparisons with arithmetic on the left, `config.x.y`,
  `[a, b] in dim`, dimensions as arrays, and helper calls;
- equations: `+ - * / **`, unary sign, `$sub_expression`, `var[dim=$slice]`, helpers with
  args and kwargs, and `inf`.

Python builds keywords from component names when it creates the grammar
(`where_parser.py:363-378,498-576`). The port parses identifiers instead and resolves them
afterwards, with the same precedence: dimension, input, result, generic string.

Note: `defined()` was removed in 0.7.0.dev8. (SPEC §3.5 no longer lists it.)

**4b. Evaluate.** Needs Phase 3.

Semantics to reproduce exactly:
- **Inputs in `where`:** present, not NaN and not ±inf. Bool lookups are taken as they
  are (`where_parser.py:181-189`).
- **Inputs in expressions:** NaN is filled with the math default, at evaluation time.
  Python only does this while the array still carries its original name
  (`expression_parser.py:95-110`). The port makes this an explicit IR node.
- **Masking:** `foreach` × `active` × `where`, with non-`foreach` dimensions collapsed by
  "any" (`parsing.py:262-274`).
- **Equation expansion:** equations × sub_expressions × slices, overlap detection, and the
  "missing linear expression for some coordinates" error.
- **Helpers:** `any`, `sum`, `reduce_carrier_dim`, `select_from_lookup_arrays`,
  `get_val_at_index`, `roll`, `where`, `map_dim`, `group_sum`, `group_datetime`,
  `sum_next_n`.
- **Component order:** variables → global expressions (sorted by `order`) → constraints →
  piecewise → objectives.
- **Checks:** `math.checks`, evaluated as `where` and ANDed with `active`, with raise or
  warn (`backend_model.py:600-623`).
- **`pre_validate_math_strings`.**

**4c. LP writer.** Pyomo naming and formatting, so the reference LP files apply directly.

**Exit:**
- All 126 reference LP files and the example-model LP files match through `lp-diff`.
- The parser corpus parses to equivalent trees.
- The cases in `checks_test.py` and `math_checks_config.yaml` raise and warn identically.
- The typecheck reports zero false positives on the built-in math and the docs gallery.

### Phase 5: HiGHS backend, results, I/O (≈3.5k LOC; 4–5 wk)

**Model to HiGHS:**
- Pass the whole model in one call (`passModel`, CSC).
- The column and row order matches Python's, which is the C-order of the `foreach`
  dimensions after `transpose(*foreach)`.
- `solver_options` are passed through; logs go to `save_logs`.
- `small_matrix_value=1e-12`, as upstream (`highs_backend_model.py:84`).

**Results:**
- primal values, global expressions, objective values (`results[<objective>]`);
- `zero_threshold`;
- postprocessed expressions (with `postprocessing_active`, in `order`), evaluated on
  results + inputs;
- shadow prices for `solve.shadow_prices` (NaN when the duals are invalid or the model is
  a MILP);
- `termination_condition`.

**I/O:**
- NetCDF4 with the groups `inputs`/`results`/`attrs`.
- The `_serialise` conventions: `serialised_{dicts,bools,nones,sets,single_element_list}`
  (`io.py:58-133`).
- Encodings: bool as int8, strings with `_FillValue "<NA>"`, zlib level 4, and CF datetime
  encoding that xarray decodes.
- CSV per array, plus `attrs.yaml`.

**Exit:**
- Every example model matches the results oracle (Phase 1 rules).
- Rust NetCDF loads in Python `calliope.read_netcdf`, and Python NetCDF loads in Rust, with
  equal attrs after a round trip.
- `tests/io_test.py` and the result-related parts of `highs_backend_model_test.py` are
  ported.

### Phase 6: modes and backend interface (≈2.5k LOC; 4 wk)

**Backend interface:**
- `get_parameter`/`variable`/`global_expression`/`constraint`/`objective`/
  `variable_bounds`;
- `update_input`:
  - coefficients and bounds are changed in place (`changeCoeff`/`changeColBounds`/
    `changeRowBounds`) through a dependency index from parameters to rows and coefficients;
  - a rebuild happens only when the sparsity pattern changes, with dependencies
    re-evaluated in definition order (#616);
- `update_variable_bounds`, `fix_variable`, and `unfix_variable` (which keeps the original
  bounds; upstream HiGHS lacks it);
- `set_objective`/`get_objective`, `delete_component`, `verbose_strings`, `to_lp`, and
  `has_integer_or_binary_variables`.

**Operate mode** (`model.py:451-586`):
- `window`/`horizon` as pandas frequency strings, with horizon ≥ window validated (Python
  does not validate this).
- Window and horizon slicing. Python uses an inclusive `sel` with a 1 ns offset
  (`:154,475-481`); the port uses half-open ranges.
- A rebuild for the shorter last horizon.
- Each window updates only the timeseries inputs that changed.
- `storage_initial` is carried over from the end storage divided by `storage_cap`.
- Results are concatenated, with `termination_condition` joined.
- Operate is rejected when time clustering is used.

**SPORES mode** (`model.py:588-861`):
- A baseline run, or `use_latest_results` from base or SPORES results, including
  continuing at SPORE n.
- `spores_baseline_cost` taken from results or inputs, otherwise the baseline objective.
- The objective is switched to `min_spores`.
- Four scoring algorithms:
  - `integer`: 1000 where capacity exceeds `score_threshold_factor` × max over
    (nodes, techs);
  - `relative_deployment`: needs a finite `flow_cap_max`;
  - `random`: a seeded RNG (an explicit `seed` config option is a port extension);
  - `evolving_average`: no added score.
- `tracking_parameter`.
- `save_per_spore_path`: one NetCDF per SPORE, with inputs in `spore_0`.
- The run stops on infeasibility.
- `spores` is an integer dimension.

**Exit:**
- operate, operate_milp and SPORES (all four algorithms, plus continuation) match the oracle
  under the Phase 1 rules.
- The operate and SPORES tests from `tests/model_test.py` are ported, along with the
  backend-interface tests from `tests/backend/backend_model_test.py` and
  `highs_backend_model_test.py`.

### Phase 7: CLI and tooling (≈1.5k LOC; 2 wk)

`calliope` binary in parity with `cli.py`:
- `run`, with `--scenario`, `--override_dict`, `--model_format`, `--save_netcdf`,
  `--save_csv`, `--save_logs`, `--save_lp`, `--debug`, `--quiet` and
  `--fail_when_infeasible`.
  - `--pdb` and `--profile` are Python-specific: accept them and warn.
- `new` (embedded example models).
- `generate_runs` (`bash`/`bsub`/`sbatch`/`windows`) and `generate_scenarios`.

New commands from SPEC §3.9: `check`, `lp-diff`, `inspect`, `bench`.

**Exit:**
- The scenarios in `tests/cli_test.py` and `tests/util/generate_runs_test.py` are ported.
- The generated scripts are byte-identical to Python's.

### Phase 8: remaining parity and extras (3–6 wk)

- **Piecewise constraints.** HiGHS has no SOS, so use a λ-formulation with binaries.
  - The LP will differ from Pyomo's SOS2 reference (`sos2_piecewise_cost_investment.lp`),
    so the oracle is objective and results parity against Pyomo with a MILP solver.
  - This is a feature the upstream HiGHS backend lacks.
- **Math documentation:** `MathDocumentation` to Markdown, LaTeX and RST, from
  `as_math_string` (about 0.9k lines in Python). Optional; it comes last.
- **`calliope-py`:** PyO3 bindings returning xarray-compatible results, and accepting
  DataFrames for `data_table_dfs`. Optional, but needed for notebook users.

### Phase 9: evaluation report (1 wk)

Re-run the Phase 0 matrix against the port. Fill in the SPEC §7 table, including the counts
of confirmed upstream bugs, LOC per subsystem, build speedup and memory ratio for each
model size, and the solve share. Publish the result honestly, including null results.

### Order and dependencies

```
P0 ─► P1 ─► P2 ─┬─► P3 ─┬─► P4b ─► P4c ─► P5 ─► P6 ─► P7 ─► P9
                └─► P4a ┘                         └─► P8 (parallel to P7)
```

A thin `calliope run --save_lp` should exist at the end of P4c so that benchmarks can start
early. Total: about 24k LOC and roughly 7–9 months for one person. The schedule risk lies in
P2/P3 (pandas and xarray semantics), not in the solver.

## Where a Rust port can add value

### 1. Finding bugs (concrete evidence)

The CHANGELOG shows recurring bug classes that a typed core with explicit missing-value
types targets directly:

- **NaN and default handling on masked entries.** `sum(flow_cap)` returned `inf` (#862).
  Defaults are filled only while an array keeps its name (`expression_parser.py:106`).
- **Bool and NaN dtype mixing.** Mixed-dtype NetCDF failures; bool lookups filled with
  `fillna(False)` (`model_data.py:93,631-640`).
- **Floating-point index mismatches** in clustering, which dropped clusters (#826, #854).
- **Order-dependent propagation** of parameter updates through expressions (#616).
- **Attribute serialisation round trips:** single-element lists (#614) and SPORES models
  (#751, #752).
- **Backend-specific objective-update bugs** (#797, #780).

Suspected bugs found while reading the code. Phase 0 must confirm each one; none has been
run yet.

| # | Location | Suspicion |
|---|---|---|
| 1 | `model.py:513-557` | Operate with only one window (the window covers the whole time span): the loop never runs and `windowstep` is unbound at `:557`. This regresses v0.6 fix #337. Untested: `tests/model_test.py:244-256` only covers shorter windows. |
| 2 | `model.py:848-853` | SPORES `tracking_parameter` uses `.notnull()`, so techs set to `false` are tracked. The docs say "will not be tracked" (`docs/building_blocks/modes.md:145`). The tests only use `true` (`tests/common/test_model/scenarios.yaml:417-420`). |
| 3 | `cli.py:284,300` | The CLI passes `{"solve.save_logs": …}` into `solve(**kwargs)`, which nests it under `solve.solve…`. The value is probably dropped silently by `model_copy` and `model_dump`. `save_per_spore_path` is also passed as a tuple `(Path,)`. |
| 4 | `backend_model.py:1251-1252` | A missing comma concatenates `"info"` into the message, so the log level is ignored. |
| 5 | `model_definition.py:176` | The error message is missing its `f` prefix, so `{scenario}` is printed literally. |
| 6 | `model_definition.py:172,178` | Scenario overrides are kept in a `set` (non-deterministic `applied_overrides` order), and self-referencing scenarios recurse with no cycle check. |
| 7 | `highs_backend_model.py:268-274` | Any non-optimal status, including a feasible MILP that hits a time or gap limit, gives empty results, although SPORES saving accepts `"feasible"` (`model.py:705`). |
| 8 | `config_schema.py:141` | `horizon ≥ window` is documented but not validated. |
| 9 | `config_schema.py:209-214` vs `model.py:757-759` | `score_threshold_factor` is documented relative to the previous capacity, but the code compares against the max over (nodes, techs). |
| 10 | Docs drift | `update_param` (`docs/how_to/backend_interface.md:23`); `operate_horizon`/`operate_window` (`modes.md:78-79`) instead of `build.operate.{horizon,window}`; `postprocess_active` (`CHANGELOG.md:56`) instead of `postprocessing_active`; the `sum_next_n` LaTeX shows N+1 terms while the code sums N (`helper_functions.py:822` vs `:870`). |
| 11 | `backend_model.py:673-675` | `_clean_arrays` does `del args` and frees nothing, so the memory "optimisation" is a no-op. |

How the port finds more:
- Every Phase 1 parity divergence is triaged as one of: a Rust bug, a Calliope bug, or
  unspecified behaviour.
- The Phase 4a typecheck flags dimension, linearity and dtype errors in the built-in and
  gallery math before any build happens. Python only catches these during the build, and
  `pre_validate_math_strings` checks syntax only (`model_math.py:101-105`).
- `cargo-fuzz` on the YAML and math grammars.

### 2. Lower complexity

The things that disappear in Rust:
- the metaclass that validates on add (`backend_model.py:50-91`);
- grammar construction for each string;
- the global packrat state;
- object-dtype arrays of solver objects;
- manual HiGHS index shifting after deletes (`highs_backend_model.py:339-375`);
- the 1 ns slicing offset in operate mode;
- the "array name implies default" convention.

Compiling the math to an IR with explicit "fill default", "mask" and "sum" nodes makes the
semantics readable in one place, instead of being spread across `expression_parser.py`,
`where_parser.py`, `parsing.py` and `helper_functions.py`.

The target is a math engine plus array layer smaller than `backend/*` (8.2k lines) at equal
features. This only holds if the array layer stays minimal: re-implementing xarray in
general would erase the gain.

### 3. Speed and memory

The build is where the gain is:
- per-cell Python objects are replaced by sparse triples and one `passModel`;
- `update_input` modifies the model in place instead of rebuilding referencing components,
  which matters most in operate mode, where every window updates all timeseries inputs
  (`model.py:539-546`);
- independent components are evaluated in parallel with `rayon`.

The SPEC target of ≥10× build and ≥5× peak memory is plausible for hourly multi-node
models, but Phase 0 must show what share the build actually has.

### 4. Things Calliope can't offer today

- A single binary, which matters for HPC and containers. Caveat: NetCDF4/HDF5 must be
  statically linked (the `netcdf` crate's `static` feature), or this advantage is lost.
- Static checking of math, with source spans (`calliope check`).
- `lp-diff` and `inspect`.
- Deterministic runs: ordered overrides and a seeded SPORES `random`.
- Features the HiGHS backend lacks upstream: piecewise constraints, `unfix_variable`,
  `eval_body`.
- A WASM build of the math checker.

### Where it will *not* add value (be explicit in the report)

- **Solve time.** It is the same HiGHS. For large models the solve often dominates, so the
  end-to-end gain is bounded by the build share.
- **Math-formulation bugs.** Most recent fixes are in the YAML math or its intent: #643,
  #645, #663, #762, #767, #835. The port reuses `math/*.yaml` verbatim and will reproduce
  them.
- **Documentation drift** (row 10 above). Rust doesn't help here, except where docs are
  generated from the schema.
- **Python users.** The users are notebook and xarray users, and integrations such as
  euro-calliope. Without `calliope-py` the port serves CLI and HPC batch use only.
- **pandas semantics are a cost.** `read_csv` inference, `to_datetime` formats, offset
  aliases and resample bin edges all have to be re-implemented, and they are a source of
  new divergence.
- **Upstream velocity.** Every upstream math or schema change has to be ported twice.

## Risks

| Risk | Mitigation |
|---|---|
| xarray and pandas edge semantics (broadcast order, NaN propagation, resample bins, CSV dtype inference, datetime parsing) | Phase 2 semantics spec and proptests against an xarray oracle; restrict resample to fixed frequencies at first; golden `inputs.nc` per fixture |
| Degenerate LPs make results comparison flaky | Match the column and row order exactly; pin the HiGHS version and options; fall back to objective and aggregates (Phase 1 rules) |
| Operate and SPORES path dependence amplifies tiny differences | Compare iteration 0 strictly and later iterations by objective; seed the RNG |
| LP reference files come from Pyomo (naming, `ONE_VAR_CONSTANT`, SOS) | The canonicaliser handles both; piecewise is checked on results, not on LP |
| Moving upstream (0.7.1.dev already changing) | Pin to `088eb7e4`; track upstream with a `CHANGELOG` diff each month; make the parity harness re-runnable on new tags |
| NetCDF/HDF5 C dependencies and CF time encoding | Static build in CI from day one; cross-language round-trip test in Phase 5 |
| `highs` crate lags HiGHS releases | Use `highs-sys` directly if needed; pin to the oracle's version |
| Polars Rust API breaks between releases, and its compile time and binary size are heavy | Pin the exact version; enable only the needed features (`lazy`, `csv`, `parquet`, `dtype-categorical`, `temporal`); keep Polars behind the `LArray` API so it can be swapped |
| Scope creep into a general xarray clone | The array API is driven only by what the math and preprocess use; anything else gets a design note first |

## Out of scope

- The Pyomo and Gurobi backends: `build.backend: pyomo|gurobi` and `solve.solver` are
  accepted with a warning and mapped to HiGHS.
- Python-only debugging flags: `--pdb`, cProfile.
- Plotting, which Calliope 0.7 dropped as well.
