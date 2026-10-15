# Triage of suspected Calliope bugs (Phase 0)

Reference: Calliope 0.7.0, `main` @ `088eb7e4` (no newer upstream commits on
`origin/main` as of 2026-10-14). The `feat/highs-speedups` branch does not touch any
of the code below. Environment: see `BASELINE.md`.

Each row has a minimal repro in `oracle/repros/`. Run them all with:

```
../calliope/.pixi/envs/dev/bin/python oracle/repros/run_all.py
```

Status values: **confirmed**, **not a bug**, **fixed upstream**. None of the 14 is
fixed upstream.

| # | Location | Status | Evidence (from the repro) |
|---|---|---|---|
| 1 | `model.py:513-557` operate, one window | **confirmed** | `bug01`: test model (48 h), `window = horizon = 48h` → `UnboundLocalError: cannot access local variable 'windowstep'`. Any window ≥ the time span crashes after the first solve. |
| 2 | `model.py:848-853` SPORES `tracking_parameter` | **confirmed** | `bug02`: `spores_tracker: {test_supply_elec: true, test_demand_elec: false}`. The undeclared parameter is stored as float (`1.0`/`0.0`), `.notnull()` is true for `0.0`, and the `false` tech gets `spores_score_cumulative = 2000` after SPORE 1. The docs say it "will not be tracked". |
| 3 | `cli.py:284,300` `--save_logs`, `save_per_spore_path` | **confirmed** | `bug03`: `calliope run --save_logs DIR` exits 0, writes nothing to `DIR`, and the saved model has `config.solve.save_logs = None`. `Model.solve(**{"solve.save_logs": x})` is silently dropped (it lands under `solve.solve`). The same nesting drops the CLI's `save_per_spore_path` redirect, so the `(Path,)` tuple never reaches validation: per-SPORE files go to the override path, not next to `--save_netcdf`. |
| 4 | `backend_model.py:1251-1252` missing comma | **confirmed** | `bug04`: `update_input("cost_flow_cap", <one dim fewer>)` logs at DEBUG with message ending `...dimension(s).info`. |
| 5 | `model_definition.py:176` missing `f` | **confirmed** | `bug05`: message is `(scenarios, {scenario}) \| Scenario definition must be a list of override or other scenario names.` |
| 6 | `model_definition.py:172,178` set order, self-reference | **confirmed** | `bug06`: (a) 8 values of `PYTHONHASHSEED` give 8 different `runtime.applied_overrides` strings for one 6-override scenario. The merged model is the same (overrides are combined with `allow_override=False`, and the `techs` coordinate order did not change), so the effect is limited to the attribute and to which override an "already specified" error names. (b) `scenarios: {loop: [loop]}` → `RecursionError`. |
| 7 | `highs_backend_model.py:268-274` non-optimal → empty | **confirmed** | `bug07`: urban_scale `milp` with `mip_max_improving_sols=1`: termination `solution limit reached`, HiGHS has a feasible incumbent (`primal_solution_status=2`, objective 1.51e9), `results` has 0 data variables. |
| 8 | `config_schema.py:141` horizon ≥ window | **confirmed** | `bug08`: `window: 12h, horizon: 6h` is accepted, builds and solves; 48 input timesteps give 24 result timesteps, with no warning. Half the period is silently dropped. |
| 9 | `config_schema.py:209-214` vs `model.py:757-759` | **confirmed** (docs/code mismatch) | `bug09`: national_scale SPORES, factor 0.1. The threshold is `0.1 × max(flow_cap over nodes, techs) = 3000`; `battery` (1000) and `csp@region1_3` (73.7) were deployed but get score 0. By the documented rule (relative to the tech's own previous capacity) both would be scored. |
| 10 | docs drift | **confirmed** | `bug10`: the backend has no `update_param` (the docs use it); `build.operate_horizon` and `solve.postprocess_active` are rejected by the schema (ValidationError) but still documented; `sum_next_n(foo, timesteps, 2)` sums 2 terms, while its LaTeX reads `\sum_{t=timestep}^{timestep+2}` (3 terms). |
| 11 | `backend_model.py:673-675` `_clean_arrays` | **confirmed** (no-op) | `bug11`: after `_clean_arrays(big)` the array is still alive in the caller; it is freed only when the caller's frame exits. No memory is saved. |
| 12 | *new*: `example_models/national_scale/scenarios.yaml` | **confirmed** | `bug12`: 7 of 17 shipped overrides/scenarios fail to load: `cold_fusion` (`KeyError: 'cost_dim_setter'`, template removed), `minimize_emissions_costs` and `maximize_utility_costs` (dict-valued `data` rejected by the schema), `run2`/`run4` (date `2005-02-31`), and the two `cold_fusion_with_*` scenarios (reference commented-out overrides). Golden fixtures record these errors (`expect_error`). |
| 13 | *new*: `highs_backend_model.py:295` `verbose_strings` | **confirmed** | `bug13`: with an integer dimension (`clusters`, `pieces`, `breakpoints`) `coords.str.replace(" ", "_")` raises `ValueError: invalid literal for int() with base 10: '_'`. 12 of the 122 upstream math-test LP builds hit it with the HiGHS backend (inter-cluster storage, piecewise, `monthly_peak_flow_charge`). The golden HiGHS LP for those uses default names. |
| 14 | *new*: `build.objective` not validated | **confirmed** | `bug14`: national_scale `check_feasibility` sets `build.objective: check_feasibility`, which no 0.7 math defines. HiGHS never sets an objective and solves a zero objective (solver objective 0.0, termination `optimal`); the Pyomo backend has no active objective and `to_lp` raises `IndexError`. |

Other findings while building the oracle:

- 4 of the 126 reference LP files (`area_use.lp`, `flow_cap.lp`, `source_cap.lp`,
  `storage_cap.lp`) are not produced by any test: they are orphans.
- Pyomo's own HiGHS interfaces (`highs`, `appsi_highs`) cannot solve the
  `pyomo.kernel` model Calliope builds (`'parameter_dict' object has no attribute
  'mutable'`, or "unavailable solver"). Upstream tests use CBC or GLPK for Pyomo.
- `cbc` is found only when the pixi env is activated (`pixi run`); calling the env's
  python directly gives `ApplicationError: No executable found for solver 'cbc'` in
  202 tests. The scripts in `oracle/` and `bench/` therefore prepend the env's
  `bin/` to `PATH`.

## Consequences for the port

- 1, 7, 8, 14 are behaviour the port should not copy: validate `horizon ≥ window`
  and the objective name, handle the single-window case, and return results for
  feasible non-optimal MILPs (with the termination condition). Each is a parity
  divergence to record as "Calliope bug", not a Rust bug.
- 2 and 9 need a decision on intended semantics (docs or code); until then the port
  follows the code and flags the difference.
- 3, 4, 5, 6, 10, 11, 12, 13 have no effect on the optimisation problem; 6 is why the
  port uses definition order for scenarios.
