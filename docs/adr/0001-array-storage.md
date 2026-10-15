# ADR 0001: array storage for `calliope-array`

Status: accepted (Phase 2 spike, 2026-10-15). Decides SPEC §3.1.

## Decision

**Option C, a hybrid: sparse over the non-time dimensions, dense over `timesteps`.**

An `LArray` stores a sorted list of the coordinate tuples that are defined over its non-time
dimensions (for example the `(node, tech, carrier)` triples that exist), and for each tuple
either one value or a dense vector over the time axis. Masks, variables and affine
expressions use the same layout, with expressions held as struct-of-arrays terms
`(key, t, col, coef)`. Missing data has a single representation, as in xarray: an absent key
or a NaN value.

Neither of the two candidates from the plan is taken as is:

- **A, dense `ndarray`**, is rejected. It materialises the full product of dimensions, as
  xarray does, and at 250 nodes × 8760 h it needs 13 GB and 10.6 s for one constraint.
- **B, Polars long format**, is rejected for `calliope-array`. It is 2.2× slower and needs 2×
  more memory than C at the largest size, it costs 15–20 ms per expression at the smallest
  sizes, and it has two kinds of missing value (`null` and NaN) that every operation must
  treat alike. Polars stays the tool for reading and reshaping data tables in
  `calliope-preprocess` (SPEC §3.4), where its CSV and group-by support is what we need.

## Spike

`spike/array-storage/` builds one heavy constraint end to end into CSC triples with each
candidate behind a minimal `LArray` API (`sum`, `where`, `roll`, `group_sum`, `fillna`, `any`,
broadcasting multiply):

```text
foreach: [nodes, carriers, timesteps]
where:   any(carrier_in, over=techs) OR any(carrier_out, over=techs)
sum(flow_out * fillna(ts_factor, 1) / fillna(flow_out_eff, 1), over=techs)
  - sum(flow_in, over=techs) == 0
```

This is `system_balance` with an `flow_out_inc_eff`-style coefficient, so that a
`[nodes, techs, timesteps]` parameter with NaN holes (1 in 97 hours), `fillna`, the `where`
mask and `sum(over)` are all on the hot path. The data is a Rust re-implementation of
`bench/synthetic.py`: copies of the 5-node `national_scale` system joined in a ring, so tech
count grows with the number of links (8 techs at 5 nodes, 254 at 250 nodes). The placement of
techs at nodes approximates `national_scale` (14 `carrier_out` and 13 `carrier_in` entries at
5 nodes, against 13 and 11 in the real model).

All four builds (A, B, C and a hand-written loop as reference) produce the same matrix in every
case (equal checksums in `0001-array-storage-results.jsonl`). Times are wall-clock seconds for
the build from in-memory inputs to CSC; memory is the peak RSS of the process. Release build,
one run each, Intel Xeon Platinum 8268 (16 cores visible, Polars may use them all), 62 GB RAM.

| nodes × hours | full product | non-zeros | A dense s / MB | B Polars s / MB | C hybrid s / MB | loops s / MB |
|---|---|---|---|---|---|---|
| 5 × 168 | 0.0 M | 0.00 M | 0.001 / 7 | 0.018 / 32 | 0.001 / 7 | 0.000 / 7 |
| 5 × 2190 | 0.1 M | 0.06 M | 0.007 / 12 | 0.021 / 40 | 0.004 / 11 | 0.004 / 9 |
| 5 × 8760 | 0.4 M | 0.24 M | 0.024 / 27 | 0.048 / 68 | 0.016 / 21 | 0.014 / 16 |
| 50 × 168 | 0.5 M | 0.05 M | 0.013 / 18 | 0.025 / 38 | 0.004 / 10 | 0.003 / 9 |
| 50 × 2190 | 5.9 M | 0.68 M | 0.144 / 156 | 0.106 / 145 | 0.048 / 52 | 0.045 / 34 |
| 50 × 8760 | 23.7 M | 2.72 M | 0.554 / 607 | 0.323 / 471 | 0.162 / 176 | 0.144 / 107 |
| 250 × 168 | 10.7 M | 0.26 M | 0.198 / 257 | 0.049 / 73 | 0.019 / 24 | 0.016 / 16 |
| 250 × 2190 | 139.1 M | 3.39 M | 2.498 / 3266 | 0.424 / 566 | 0.197 / 222 | 0.227 / 133 |
| 250 × 8760 | 556.3 M | 13.58 M | 10.600 / 13026 | 1.605 / 1708 | 0.719 / 833 | 0.760 / 503 |

"Full product" is nodes × techs × carriers × timesteps, the size of one dense variable array.

For scale: Python Calliope builds the whole `national_scale` model (5 nodes, 8760 h, about 60
components) in 36 s with 535 MB peak (`bench/results/national__main.json`). C builds this one
constraint at that size in 16 ms.

### Other criteria

| | A dense | B Polars | C hybrid |
|---|---|---|---|
| Lines for `sum`, `where`, `roll`, `group_sum` | 68 | about 40, plus a `not_missing` helper used by every reduction | 66 |
| Missing-value rules the code must implement | 2: `sum` with `min_count=1`, `where` → NaN | 5: the two of A, plus `null` vs NaN in every filter and reduction, `sum` of all-`null` = 0 (needs a count), joins that create `null`, and `fillna` for both | 3: the two of A, plus "absent key = NaN" in lookups |
| Cost per operation on tiny arrays | ~0 | 15–20 ms per expression (query planning, thread pool) | ~0 |
| Build cost | `ndarray`: seconds | `polars` with `lazy` + `cross_join`: 80 s wall (12.6 CPU-min) for a clean release build, 662 MB `target/` | none beyond `std` (the spike version uses only `std`) |
| Scales with | full product | active cells | active cells |

## Consequences

- `calliope-array` implements C natively, with no Polars dependency. The xarray behaviour the
  math needs (outer-join broadcasting by name, `where`, `fillna`, `sum(min_count=1)`, `any`,
  `roll`, `isin`, `sel`, `group_sum`, `resample`, `concat`) is written once against this
  layout and checked against an xarray oracle with proptests.
- Broadcasting two arrays is a join on their shared non-time dimensions. The math engine
  mostly evaluates parameters at the cells of a component's `foreach` mask, which is a lookup
  per non-time key, amortised over the time axis. That is why C beats B: there is no general
  join where a lookup suffices.
- Arrays without a time dimension use the same layout with one value per key. They are small
  in every model, so no separate dense path is needed. If profiling later shows a need, a
  dense path can be added behind the same API.
- Coordinate labels are interned per dimension (`u32` codes). The order of coordinates follows
  xarray (sorted union on an outer join, insertion order otherwise), which the column and row
  order of the LP depends on (PLAN Phase 5).
- Risk: time-varying data that is defined on only part of the time axis is stored as full
  vectors with NaN, as xarray does. Calliope's data always covers the whole axis after
  preprocessing, so this costs nothing in practice.
- The spike is kept in `spike/array-storage/` (a separate Cargo project, not a workspace
  member) so the numbers can be reproduced:
  `cargo run --release -- <dense|polars|sparse|reference> <copies> <hours>`.
