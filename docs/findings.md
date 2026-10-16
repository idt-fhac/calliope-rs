# Findings in Calliope 0.7.0 from the Rust port

Runtime bugs found with Python repros are in `bench/BUGS.md`. This file collects what the
port's parser and static typecheck (`crates/calliope-math`) found in the math layer.

| ID | Status | Finding |
|---|---|---|
| M1 | likely bug | `total_curtailment` divides by `sum(source_use_max * …, over=timesteps)`. When `source_use_max` has no `timesteps` dimension, the sum does nothing and the denominator is off by a factor of T. The example models don't show it because their `source_use_max` is `inf`. |
| M2 | confirmed | A sub-expression reference under a unary sign (`-$s`) is never expanded, so evaluation fails with a `KeyError`. |
| M3 | confirmed | Equation names depend on `PYTHONHASHSEED`, so they are not reproducible between runs. |
| M4 | grammar quirk | A unary sign binds tighter than `**`, so `-x**2` evaluates as `(-x)**2`. |
| M5 | grammar quirk | `and` and `or` have equal precedence and are evaluated left to right. |
| M6 | silent | Typo'd dimension names in `over=` are ignored without any error. |
| M7 | quirk | Arguments to the comparison helpers are handled inconsistently. |
| M8 | silent | A typo in `foreach` silently drops the whole component. |
| M9 | silent | Comparisons between incompatible dtypes are silently false. |

How they were found: the pest grammar mirrors pyparsing rule by rule, and agrees with it
on 358 shipped math strings and 1600 seeded mutants (`parse_corpus.json`). The typecheck
infers the dimensions, linearity and dtype of every expression. On the shipped math it
reports only M1, and only when the example models' input dimensions are supplied.

## Deviations from Python behaviour

| Where | Python | calliope-rs |
|---|---|---|
| `calliope run --save_logs DIR` | Silently ignored (bug03 in `bench/BUGS.md`): the CLI passes `solve.save_logs` as a literal key, which lands under `solve.solve` and is dropped. | The option reaches the solver through `SolveOptions::save_logs`, so logs are written to `DIR`. |
| `calliope run --save_netcdf` with `save_per_spore_path` set | The redirect is passed as a one-element tuple under the same dropped key (bug03). | The per-SPORE directory is the parent of the `--save_netcdf` file, as intended. |
| `--pdb`, `--profile`, `--profile_filename` | Debugger and cProfile. | Accepted and ignored with a warning. |
| Error output | `Error in <function>, <file>:<line>` plus the message on stdout; traceback on stderr with `--debug`. | `Error: <message>` on stdout; `--debug` prints the chain of causes on stderr. |

## Math evaluation and LP parity (Phase 4b)

`crates/calliope-model/tests/lp_parity.rs` builds every golden fixture and compares the
written LP with Calliope's after canonicalisation: all 122 `math_lp` fixtures and all 34
example/test-model fixtures that have an LP are equal (3 fixtures fail in preprocessing, as
expected). No mismatch was left, so none is classified as a Calliope bug; these are the
decisions and deviations behind that result.

| ID | Kind | Finding |
|---|---|---|
| E1 | deviation | A NaN operand meeting a decision variable inside a component's `where` (e.g. `flow_out * p` with `p` NaN and no default): numpy hands the NaN to Pyomo, which builds `nan*x` and writes `nan` into the LP. The port treats the cell as missing, so a global expression reports "Missing a linear expression for some coordinates selected by 'where'" and a constraint reports the same instead of writing a NaN row. No fixture reaches this. |
| E2 | as Python | Math `checks` run when the problem is built (`BackendModel._check_inputs`), not when the model is read: models Calliope can read but not build (e.g. clustering with cyclic storage) still preprocess. `pre_validate_math_strings` runs at read time, as in `build_math`. |
| E3 | as Pyomo | A constraint whose bound side is ±inf (`x <= inf`) is not added, as Pyomo's `to_bounded_expression` drops it; terms with coefficient 0 are kept while evaluating (Pyomo's `is_potentially_variable`), which decides orientation and the "simple boolean" error. |
| E4 | deviation | Piecewise constraints follow Pyomo's `piecewise_sos2` (`_v(i)` columns, rows `_c(0..2)`, SOS2 `_s` with weights 1..n) and check breakpoint/value counts and monotonic breakpoints, but not Pyomo's input-domain coverage; the input may be any affine expression, not only a variable. |
| E5 | bug (known) | `national_scale` `check_feasibility` names an objective no math defines (bench/BUGS.md #14). The port builds without an objective, i.e. a zero objective, as the HiGHS backend does; Pyomo's `to_lp` fails, so the fixture is compared with `highs.lp`. |
| E6 | oracle | Pyomo adds `c_e_ONE_VAR_CONSTANT` when no row has variables (a CBC workaround); the comparison drops it. The operate fixtures' `pyomo.lp` is written after the solve (last window), so they are compared with the first-window `highs.lp`. |

Build time (release, one thread, `examples/build_timing`): national_scale 8760 h 1.9–2.2 s
and 331 MB peak RSS against Python's 36.0 s build (535 MB); urban_scale 8760 h 3.9 s
against 61 s.

## Modes, results and backend interface (Phase 6)

`cargo run -p xtask -- parity` runs calliope-rs on all 37 golden fixtures (read, build,
solve, write `inputs.nc`, LP and `results.nc`) and compares them with Calliope's. Result:
104 comparisons pass, 7 fixture parts are classified (`crates/xtask/src/known.rs`), none
fail. `cargo test -p calliope-model --test golden_results` checks every objective value
(per operate timestep and per SPORE) and termination condition the same way. All operate
fixtures and the `integer`, `random` (numpy's MT19937, seed 0) and `relative_deployment`
SPORES fixtures match to 1e-6 in every objective value.

| ID | Class | Fixture | Finding |
|---|---|---|---|
| R1 | degenerate | `urban_scale`, `national_scale_spores_evolving_average` | Same LP (canonical diff equal; HiGHS LP written in the same column/row order), same objective, another optimal vertex. The model HiGHS receives differs only in rounding and row signs: highspy merges duplicate terms with a cumsum difference (`unique_elements`), so some coefficients differ in the last bits (e.g. `0.0249999999999999` vs `0.025`), and orients rows by its own "most terms first" rules (`area_use == flow_cap * k` becomes `k*flow_cap - area_use = 0`). With HiGHS 1.15.0 vs Calliope's 1.15.1 that is enough to end on another vertex. In SPORES, SPORE 1 has equal objectives but `flow_cap` differs by 0.1% in one cell; `evolving_average` scores depend on capacities, so SPORE 2's `min_spores` differs by 1.4e-5. |
| R2 | solver | `docs_piecewise` | The golden run is Pyomo + CBC (SOS2); the port solves with HiGHS, SOS2 as binaries. Same objective, 8 cells differ. |
| R3 | Calliope bug | `national_scale_check_feasibility` | bench/BUGS.md #14: the objective does not exist, so both solve a zero objective and any feasible point is optimal; the reported `min_cost_optimisation` (254838 vs 287661) depends on the point. The port logs a warning. |
| R4 | MIP gap | `urban_scale_milp` | `mip_rel_gap = 0.05`: 901.51 (Calliope) vs 901.67, both within the gap. |
| R5 | message | `national_scale_cold_fusion`, `..._maximize_utility`, `..._minimize_emissions` | Rejected in preprocessing like Calliope (bench/BUGS.md #12), with the port's message. |

Deviations from Calliope's behaviour, each covered by a test in
`crates/calliope-model/tests/modes.rs` or `backend_interface.rs`:

| Where | Calliope | calliope-rs |
|---|---|---|
| Operate, one window ≥ the time span | `UnboundLocalError` (bench/BUGS.md #1) | Solved and returned. |
| Operate, `horizon < window` | Accepted; uncovered timesteps silently dropped (#8) | `ModelError`. |
| Operate windows | One backend; each changed time series input is updated with `update_input` (NaN keeps the previous window's value), rebuild for the shorter last horizon; LP names keep the first window's timestamps | Each window is built from the sliced inputs (same problem; NaN means "use the default"). `storage_initial` carry-over follows Calliope's update/rebuild rule. |
| SPORES `tracking_parameter` | `.notnull()`: techs set to `false` are tracked (#2) | Only techs whose value is true. |
| SPORES `random` | numpy's global generator, unseeded unless the caller seeds it | numpy's legacy MT19937 with seed 0 (`Model::set_random_seed`); draws equal `np.random.seed(s)`. |
| Non-optimal MILP with an incumbent | Empty results (#7) | Results of the incumbent, with the termination condition. |
| Unknown `build.objective` | Silently a zero objective (#14) | Same, with a warning. |
| `unfix_variable` (HiGHS) | `BackendError` | Restores the bounds from before `fix_variable`. |
| `update_input` | Rebuilds the components that recorded a reference while built; a component whose `where` was empty is never added | Re-evaluates the problem and changes only the rows/columns/objective of components that refer to the input (static dependency index, `eval::Dependencies`) in HiGHS in place; a changed sparsity pattern re-passes the model, so a component the update enables is added. Manual bound changes on unaffected components survive. |
| `delete_component` | Removes the component; a later rebuild of a referencing component fails | Removed, and stays removed across updates. |
| `to_lp` | HiGHS writer (anonymous names unless `verbose_strings`) | The port's writer (Pyomo-style names with `verbose_strings`, `c<N>`/`r<N>` otherwise); `write_solver_model` gives HiGHS' own file. |
| NetCDF | — | Datetimes encoded like xarray (`<unit> since <first>`), so raw values equal Python's files. |

Run time and peak RSS, national_scale (release `calliope run`, HiGHS `threads=1`), against
bench/BASELINE.md (Python, `main`):

| case | Python | calliope-rs |
|---|---|---|
| operate, 10 days | 43.3 s, 214 MB | 0.8 s, 61 MB |
| operate, 31 days | 119.1 s, 220 MB | 2.0 s, 61 MB |
| SPORES, 10 days | 14.5 s, 249 MB | 4.0 s, 72 MB |
| SPORES, 31 days | 46.6 s, 327 MB | 33.5 s, 152 MB |

Operate is no longer bound by `update_input` and result loading. SPORES 31 days is
bound by HiGHS (34 s of solve in Python too).
