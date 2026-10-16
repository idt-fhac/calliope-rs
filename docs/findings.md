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
