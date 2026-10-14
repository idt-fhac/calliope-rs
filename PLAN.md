# Porting Calliope to Rust

Reference checkout: `../calliope` (upstream `calliope-project/calliope`, `main` @ `088eb7e4`).

## Is it worth it?

Not yet proven. Phase 0 exists to settle that before any real porting starts.

Points that weaken the case:

- Calliope already solves through HiGHS without Pyomo
  (`src/calliope/backend/highs_backend_model.py`, via `highspy`), and upstream has a
  `feat/highs-speedups` branch in progress. "Drop Pyomo, use HiGHS" is already done upstream.
- For large models the solver usually dominates the run time, and HiGHS is C++ in both
  cases. A Rust front end only speeds up preprocessing and the model build.
- Users work in Python: custom math in YAML, results as xarray in notebooks, and
  integrations such as euro-calliope. A Rust tool that can't do this loses most of them.
- Upstream is still changing fast (0.7 development). A full port has to keep up with the
  math schema, the YAML format and the data-table features.

Points in favour:

- A single static binary with no Python environment, which is easier to deploy on
  HPC and in containers.
- Lower memory use and faster model builds for very large models (many nodes and
  hourly resolution), where the xarray/pandas build can take minutes and many GB of memory.
- The math is already declarative YAML (`math/*.yaml`, about 2,200 lines) with its own
  `where` and expression language. That can be ported cleanly and checked against
  reference LP files.

**Decision rule:** port only if Phase 0 shows that the Python model build (not the solve)
is a substantial share of wall time, or of peak memory, on realistic models. If it isn't,
the better path is to contribute speedups upstream. A middle ground is a Rust core
(math engine → sparse matrix) exposed to Python through PyO3.

## Phases

### Phase 0: measure (go/no-go)
- Run the upstream test suite here; record versions.
- Profile `national_scale`, `urban_scale` and a large model (e.g. euro-calliope at
  hourly resolution): time and peak memory for preprocess, build, solve and postprocess.
- Compare `main` with `feat/highs-speedups`.
- Output: a short report and a go/no-go decision. Choose a standalone CLI or a PyO3 core.

### Phase 1: test oracle
- Generate golden files from Python Calliope: results NetCDF for the example models,
  and the 126 reference LP files in `tests/math/lp_files`.
- Write an LP normaliser that canonicalises the ordering and naming of variables and
  constraints, so the Rust and Python output can be diffed.
- Extract the table-driven cases from `where_parser_test.py` and
  `expression_parser_test.py`.

### Phase 2: foundations
- `calliope-core`: labelled N-d arrays (named dimensions and coordinates over `ndarray`,
  with broadcasting and masks). Keep the API minimal and driven by what the math needs.
- `calliope-config`: YAML loading with `import:`, overrides, scenarios and schema
  validation (`schemas/`).

### Phase 3: preprocess
- Expand nodes, techs and carriers; apply inheritance and templates; load data tables
  (CSV and Parquet via Polars or Arrow); handle time subsetting and resampling.
- Check the result against the dataset built by Python before the build step
  (exported to NetCDF).

### Phase 4: math engine
- Port the `where` and expression parsers (`pest` or `nom`) and `helper_functions.py`.
- An evaluator that produces sparse linear expressions over the variable index sets.
- Run order: `base.yaml` → `milp.yaml` → `storage_inter_cluster.yaml` → `operate.yaml` →
  `spores.yaml`.
- Exit criterion: all reference LP files match after normalisation.

### Phase 5: HiGHS backend and results
- Pass the model to HiGHS as column-wise sparse matrices (the `highs` crate); read
  primal values and duals back into labelled arrays.
- Write NetCDF results compatible with Python Calliope's `read_netcdf`, plus CSV.
- Exit criterion: the assertions in `example_models_test.py` hold within tolerance.

### Phase 6: CLI and modes
- `calliope run model.yaml --scenario ... --save_netcdf ...` in parity with `cli.py`.
- Operate mode (rolling horizon) and SPORES (iterative re-solve).

## Out of scope for now
- The Gurobi and Pyomo backends, LaTeX math documentation, and plotting.
- Python bindings, unless Phase 0 chooses the PyO3 option.

## Risks
- **Moving upstream:** pin to one upstream commit and track schema changes deliberately.
- **Matching xarray semantics:** broadcasting and NaN/mask behaviour must match exactly,
  or the LP output will differ subtly.
- **Workload:** about 10k lines of Python core plus about 2k lines of math YAML. Expect
  months of work, not weeks, for one person to reach parity.
