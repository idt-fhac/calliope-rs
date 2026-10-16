# ADR 0002: dense input dataset in `calliope-preprocess`

Status: accepted (Phase 3, 2026-10-15).

## Decision

Preprocessing builds the input dataset on a thin dense layer (`calliope_preprocess::xr`:
`Data`, `DataArray`, `Dataset`), not on `calliope_array::LArray`. It reuses
`calliope_array::{Coord, Label}` for coordinates. The finished dataset converts to `LArray`s
for the math engine.

## Why not `LArray`

The exit criterion of Phase 3 is that `inputs.nc` matches Python in variables, dtypes, label
order and dimension order. The values that decide this are the side effects of numpy/pandas
on the way:

- dtypes change when values go missing: `bool` becomes `float64` on alignment but `object`
  in `Series.to_xarray`; `int64` becomes `float64`; mixed values are `object` arrays;
- `fillna`/`where` follow numpy `result_type`;
- outer joins sort labels (pandas `Index.union`) except for equal or empty indexes; stacked
  CSV rows keep file order while MultiIndex columns are sorted.

`LArray<T>` is typed per array, has no `object` dtype, and treats a missing `bool` as `false`
(ADR 0001), which is right for the math but loses exactly this information. Adding dtype
tracking to it would burden the hot math path for a step that runs once on small arrays.

## Consequences

- About 700 lines that mirror the xarray behaviour preprocessing needs, nothing more.
- Memory is dense like xarray's during preprocessing (same as Python); the math engine
  gets sparse `LArray`s.
- CSV tables are read with the `csv` crate, not Polars: pandas type inference has to be
  reproduced on the raw strings either way, and Polars costs about 2.3 GB of `target/` and
  1.5 min of build on a 7 GB-free disk. Parquet input can be added later behind a feature.

## Addendum (2026-10-16): sparse data-table arrays

The premise "a step that runs once on small arrays" did not hold at 250 nodes. Preprocessing
250 × 2190 peaked at 11.8 GB, for a 2.2 GB result that was 99.8% NaN (`bench/REPORT.md` §1).
The dense layer stays, with three changes:

- `Data::Sparse` stores only the non-missing cells of a `float64` array. It is created for
  unstacked data tables that are mostly empty. `conform` also builds it when a float array
  is reindexed into a mostly empty one, for example a table's techs joined with all techs.
  `conform`, `reindex`, `where` and `fillna` keep the data sparse. It reads as `float64`, so
  dtype rules are unchanged. The NetCDF writer, results and the Python bindings expand it
  to dense values, and the math conversion builds `LArray`s from the stored cells.
- Dense paths avoid the per-cell `Option<usize>` index and the `Obj` vectors: `conform`
  maps through an iterator, and `where`, `fillna` and `zip_with` work on typed values,
  in place where they can.
- `add_to_dataset` merges in place (`Dataset::merge_over`). Cleaning and `drop_undefined`
  move arrays instead of cloning them.

Inputs still match the golden files exactly (37 fixtures and 122 math LP fixtures). They
also match with every data-table float array forced sparse. Preprocessing peak RSS at
250 × 2190 fell from 11.8 GB to 0.2 GB, and preprocessing time from 70 s to 5 s.
