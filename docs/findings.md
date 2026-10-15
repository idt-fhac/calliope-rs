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
