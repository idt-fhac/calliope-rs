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
