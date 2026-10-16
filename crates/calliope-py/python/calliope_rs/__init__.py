"""calliope-rs from Python, with the `calliope` model API.

```python
import calliope_rs

model = calliope_rs.read_yaml("model.yaml", scenario="milp", subset={...})
model.build()
model.solve()
model.results.flow_cap  # an xarray.DataArray
```

`read_yaml`, `read_dict`, `read_netcdf` and `Model(inputs, attrs, results, **kwargs)`
mirror `calliope`. Inputs and results are `xarray.Dataset`s; in-memory data tables are
pandas DataFrames given as `data_table_dfs`. The optimisation problem is always built
for and solved with HiGHS.

The backend interface (`model.backend`) works on values, not on solver objects: the
`as_backend_objs` arguments of `calliope` default to `False` here and `True` is not
supported.
"""

from __future__ import annotations

import math
from collections.abc import Mapping
from pathlib import Path
from typing import Any

import numpy as np
import pandas as pd
import xarray as xr

from calliope_rs import _core
from calliope_rs._core import BackendError, ModelError, __version__

__all__ = [
    "AttrDict",
    "Backend",
    "BackendError",
    "Model",
    "ModelAttrs",
    "ModelError",
    "__version__",
    "read_dict",
    "read_netcdf",
    "read_yaml",
]


class AttrDict(dict):
    """A nested `dict` with attribute access (`model.config.init.name`)."""

    def __getattr__(self, key: str) -> Any:
        try:
            return self[key]
        except KeyError:
            raise AttributeError(key) from None

    @classmethod
    def wrap(cls, value: Any) -> Any:
        """Wraps nested dicts (in dicts and lists) as `AttrDict`s."""
        if isinstance(value, dict):
            return cls({k: cls.wrap(v) for k, v in value.items()})
        if isinstance(value, list):
            return [cls.wrap(v) for v in value]
        return value


# ---- conversion: Rust -> xarray ----------------------------------------------------------


def _coord_values(kind: str, values: list) -> np.ndarray:
    if kind == "datetime":
        return np.array(values, dtype="int64").view("datetime64[ns]")
    if values and all(isinstance(v, int) and not isinstance(v, bool) for v in values):
        return np.array(values, dtype="int64")
    if values and all(isinstance(v, float) for v in values):
        return np.array(values, dtype="float64")
    out = np.empty(len(values), dtype=object)
    out[:] = values
    return out


def _values(dtype: str, payload: Any, shape: tuple[int, ...]) -> np.ndarray:
    if dtype == "O":
        out = np.empty(len(payload), dtype=object)
        out[:] = payload
        return out.reshape(shape)
    return (
        np.frombuffer(payload, dtype=np.dtype(dtype).newbyteorder("<"))
        .reshape(shape)
        .astype(np.dtype(dtype), copy=True)
    )


def _to_dataset(raw: dict) -> xr.Dataset:
    coords = {k: _coord_values(*v) for k, v in raw["coords"].items()}
    data_vars = {}
    for name, dims, dtype, payload in raw["vars"]:
        shape = tuple(len(coords[d]) for d in dims)
        data_vars[name] = (tuple(dims), _values(dtype, payload, shape))
    return xr.Dataset(data_vars, coords=coords)


def _to_dataarray(raw: dict, name: str | None = None) -> xr.DataArray:
    coords = {k: _coord_values(*v) for k, v in raw["coords"].items()}
    dims = tuple(raw["dims"])
    shape = tuple(len(coords[d]) for d in dims)
    return xr.DataArray(
        _values(raw["dtype"], raw["data"], shape),
        coords={d: coords[d] for d in dims},
        dims=dims,
        name=name,
    )


# ---- conversion: xarray / pandas -> Rust ----------------------------------------------------


def _isna(v: Any) -> bool:
    return (
        v is None
        or v is pd.NA
        or v is pd.NaT
        or (isinstance(v, float) and math.isnan(v))
    )


def _labels_raw(index: pd.Index) -> tuple[str, list]:
    if pd.api.types.is_datetime64_dtype(index.dtype):
        return "datetime", index.values.astype("datetime64[ns]").view("int64").tolist()
    return "object", index.tolist()


def _data_raw(values: np.ndarray) -> tuple[str, Any]:
    values = np.asarray(values)
    kind = values.dtype.kind
    if kind == "f":
        return "f8", np.ascontiguousarray(values, dtype="<f8").tobytes()
    if kind in "iu":
        return "i8", np.ascontiguousarray(values, dtype="<i8").tobytes()
    if kind == "b":
        return "?", np.ascontiguousarray(values, dtype=bool).tobytes()
    if kind == "M":
        return "M8[ns]", np.ascontiguousarray(
            values.astype("datetime64[ns]").view("int64"), dtype="<i8"
        ).tobytes()
    return "O", [None if _isna(v) else v for v in values.ravel().tolist()]


def _dim_labels(obj: xr.Dataset | xr.DataArray, dim: str) -> tuple[str, list]:
    if dim in obj.indexes:
        return _labels_raw(obj.indexes[dim])
    return "object", list(range(obj.sizes[dim]))


def _dataset_raw(ds: xr.Dataset) -> dict:
    return {
        "coords": {str(d): _dim_labels(ds, d) for d in ds.dims},
        "vars": [
            (str(name), [str(d) for d in da.dims], *_data_raw(da.values))
            for name, da in ds.data_vars.items()
        ],
    }


def _array_raw(value: Any) -> dict:
    da = value if isinstance(value, xr.DataArray) else xr.DataArray(value)
    dtype, data = _data_raw(da.values)
    return {
        "coords": {str(d): _dim_labels(da, d) for d in da.dims},
        "dims": [str(d) for d in da.dims],
        "dtype": dtype,
        "data": data,
    }


def _frame_raw(name: str, df: Any) -> dict:
    if not isinstance(df, pd.DataFrame):
        raise ModelError(
            f"(data_table_dfs, {name}) | Data table must be a pandas DataFrame. "
            "If you are providing an in-memory object, ensure it is not a pandas Series "
            "by calling the method `to_frame()`."
        )

    def levels(index: pd.Index) -> list:
        return [_labels_raw(index.get_level_values(i)) for i in range(index.nlevels)]

    return {
        "index_names": [None if n is None else str(n) for n in df.index.names],
        "index": levels(df.index),
        "column_names": [None if n is None else str(n) for n in df.columns.names],
        "columns": levels(df.columns),
        "data": [_data_raw(df.iloc[:, j].to_numpy()) for j in range(df.shape[1])],
    }


def _dfs_raw(data_table_dfs: Mapping[str, pd.DataFrame] | None) -> dict | None:
    if data_table_dfs is None:
        return None
    return {str(k): _frame_raw(k, v) for k, v in data_table_dfs.items()}


def _kwargs(kwargs: dict) -> dict | None:
    return dict(kwargs) if kwargs else None


# ---- the model ---------------------------------------------------------------------------


class ModelAttrs:
    """`model.all_attrs()`: the definition, config, math and runtime of a model, to pass
    as `attrs` to `Model(inputs, attrs, ...)`."""

    def __init__(self, core: _core.RsModel):
        self._core = core

    def model_dump(self) -> dict:
        """The attributes as a nested dict."""
        return self._core.dump_all_attrs()


class Model:
    """A calliope-rs model (`calliope.Model`)."""

    def __init__(
        self,
        inputs: xr.Dataset,
        attrs: ModelAttrs,
        results: xr.Dataset | None = None,
        **kwargs: Any,
    ) -> None:
        """A model with the given inputs, the attributes of another model
        (`other.all_attrs()`) and optional results; `kwargs` override `config.init`."""
        if not isinstance(attrs, ModelAttrs):
            raise TypeError(
                "`attrs` must be `Model.all_attrs()` of a calliope_rs model."
            )
        raw_results = (
            None if results is None or not results.data_vars else _dataset_raw(results)
        )
        core = attrs._core.with_inputs(
            _dataset_raw(inputs), raw_results, _kwargs(kwargs)
        )
        self._set_core(core)

    @classmethod
    def _from_core(cls, core: _core.RsModel) -> Model:
        self = cls.__new__(cls)
        self._set_core(core)
        return self

    def _set_core(self, core: _core.RsModel) -> None:
        self._core = core
        self._invalidate()

    def _invalidate(self) -> None:
        self._inputs: xr.Dataset | None = None
        self._results: xr.Dataset | None = None
        self._attrs: AttrDict | None = None

    # -- data and attributes

    @property
    def inputs(self) -> xr.Dataset:
        """Input data."""
        if self._inputs is None:
            self._inputs = _to_dataset(self._core.inputs())
        return self._inputs

    @property
    def results(self) -> xr.Dataset:
        """Results (an empty dataset before a successful solve)."""
        if self._results is None:
            self._results = _to_dataset(self._core.results())
        return self._results

    def _attr(self, key: str) -> AttrDict:
        if self._attrs is None:
            self._attrs = AttrDict.wrap(self._core.dump_all_attrs())
        return self._attrs[key]

    @property
    def config(self) -> AttrDict:
        """`config` (as set, not including defaults)."""
        return self._attr("config")

    @property
    def definition(self) -> AttrDict:
        """The model definition."""
        return self._attr("definition")

    @property
    def math(self) -> AttrDict:
        """`math.init` and `math.build`."""
        return self._attr("math")

    @property
    def runtime(self) -> AttrDict:
        """Runtime attributes (timings, applied overrides, termination condition, ...)."""
        return self._attr("runtime")

    @property
    def name(self) -> str | None:
        """`config.init.name`."""
        return self._core.name

    @property
    def is_built(self) -> bool:
        """True once `build` has run."""
        return self._core.is_built

    @property
    def is_solved(self) -> bool:
        """True if the model has results."""
        return self._core.is_solved

    @property
    def warnings(self) -> list[str]:
        """Warnings raised while reading the model."""
        return self._core.warnings

    def all_attrs(self) -> ModelAttrs:
        """The model attributes, for `Model(inputs, attrs, ...)`."""
        return ModelAttrs(self._core)

    def dump_all_attrs(self) -> dict:
        """The model attributes as a dict."""
        return self._core.dump_all_attrs()

    def info(self) -> str:
        """The model name and size."""
        return self._core.info()

    # -- lifecycle

    def build(self, force: bool = False, **kwargs: Any) -> None:
        """Builds the optimisation problem; `kwargs` override `config.build`."""
        self._core.build(force, _kwargs(kwargs))
        self._invalidate()

    def solve(
        self, force: bool = False, warmstart: bool = False, **kwargs: Any
    ) -> None:
        """Solves the built problem with HiGHS; `kwargs` override `config.solve`."""
        self._core.solve(force, warmstart, _kwargs(kwargs))
        self._invalidate()

    def to_netcdf(self, path: str | Path) -> None:
        """Saves inputs, results and attributes to a NetCDF file."""
        self._core.to_netcdf(Path(path))

    def to_csv(
        self, path: str | Path, dropna: bool = True, allow_overwrite: bool = False
    ) -> None:
        """Saves inputs and results as CSV files (and `attrs.yaml`) in a directory."""
        self._core.to_csv(Path(path), dropna, allow_overwrite)

    def set_random_seed(self, seed: int) -> None:
        """Seeds the random generator SPORES uses (`np.random.seed` in Calliope)."""
        self._core.set_random_seed(seed)

    @property
    def backend(self) -> Backend:
        """The backend interface of the built problem."""
        if not self._core.is_built:
            raise AttributeError("The model has not been built yet (call `build()`).")
        return Backend(self)


def _no_backend_objs(as_backend_objs: bool) -> None:
    if as_backend_objs:
        raise NotImplementedError(
            "calliope_rs returns values only; solver objects are not exposed "
            "(use `as_backend_objs=False`)."
        )


class Backend:
    """`model.backend`: inspect and update the built optimisation problem."""

    def __init__(self, model: Model):
        self._model = model

    @property
    def _core(self) -> _core.RsModel:
        return self._model._core

    def _changed(self) -> None:
        self._model._invalidate()

    # -- inputs

    def get_parameter(self, name: str, as_backend_objs: bool = False) -> xr.DataArray:
        """A parameter or lookup as the backend holds it."""
        _no_backend_objs(as_backend_objs)
        raw = self._core.backend_get_parameter(name)
        return (
            _to_dataarray(raw, name)
            if isinstance(raw, dict)
            else xr.DataArray(raw, name=name)
        )

    get_lookup = get_parameter

    def update_input(self, name: str, new_values: xr.DataArray | float) -> None:
        """Updates a parameter or lookup and rebuilds the components that use it."""
        self._core.backend_update_input(name, _array_raw(new_values))
        self._changed()

    # -- variables

    def get_variable(self, name: str, as_backend_objs: bool = False) -> xr.DataArray:
        """A decision variable's values at the last solution."""
        _no_backend_objs(as_backend_objs)
        return _to_dataarray(self._core.backend_get_variable(name), name)

    def get_variable_bounds(self, name: str) -> xr.Dataset:
        """The lower (`lb`) and upper (`ub`) bounds of a decision variable."""
        lb, ub = self._core.backend_get_variable_bounds(name)
        return xr.Dataset({"lb": _to_dataarray(lb), "ub": _to_dataarray(ub)})

    def update_variable_bounds(
        self,
        name: str,
        *,
        min: xr.DataArray | float | None = None,
        max: xr.DataArray | float | None = None,
    ) -> None:
        """Sets new bounds where the given values are not NaN."""
        self._core.backend_update_variable_bounds(
            name,
            None if min is None else _array_raw(min),
            None if max is None else _array_raw(max),
        )

    def fix_variable(self, name: str, where: xr.DataArray | None = None) -> None:
        """Fixes a variable to its last solved values (where `where` is True)."""
        self._core.backend_fix_variable(
            name, None if where is None else _array_raw(where)
        )

    def unfix_variable(self, name: str, where: xr.DataArray | None = None) -> None:
        """Restores the bounds a variable had before `fix_variable`."""
        self._core.backend_unfix_variable(
            name, None if where is None else _array_raw(where)
        )

    # -- expressions, constraints, objectives

    def get_global_expression(
        self, name: str, as_backend_objs: bool = False, eval_body: bool = True
    ) -> xr.DataArray:
        """A global expression's values at the last solution."""
        _no_backend_objs(as_backend_objs)
        return _to_dataarray(self._core.backend_get_global_expression(name), name)

    def get_constraint(
        self, name: str, as_backend_objs: bool = False, eval_body: bool = True
    ) -> xr.Dataset:
        """`lb`, `ub` and (after a solve, if `eval_body`) `body` of every row."""
        _no_backend_objs(as_backend_objs)
        lb, ub, body = self._core.backend_get_constraint(name)
        out = {"lb": _to_dataarray(lb), "ub": _to_dataarray(ub)}
        if eval_body and body is not None:
            out["body"] = _to_dataarray(body)
        return xr.Dataset(out)

    def get_objective(self, name: str) -> float:
        """An objective's value at the last solution."""
        return self._core.backend_get_objective(name)

    def set_objective(self, name: str) -> None:
        """Makes a built objective the one that is optimised."""
        self._core.backend_set_objective(name)

    @property
    def objective(self) -> str:
        """The name of the active objective."""
        return self._core.backend_objective()

    def activate_constraint(self, name: str) -> None:
        """Activates a deactivated constraint."""
        self._core.backend_set_constraint_active(name, True)

    def deactivate_constraint(self, name: str) -> None:
        """Deactivates (frees the rows of) a constraint."""
        self._core.backend_set_constraint_active(name, False)

    def delete_component(self, key: str, component_type: str) -> None:
        """Removes a component (`variables`, `constraints`, ...) from the problem."""
        self._core.backend_delete_component(key, component_type)

    def component_names(self, component_type: str) -> list[str]:
        """Names of the math components of a group (`variables`, `constraints`, ...)."""
        return self._core.backend_component_names(component_type)

    # -- output

    def verbose_strings(self) -> None:
        """Names rows and columns by component and index in LP files."""
        self._core.backend_verbose_strings()

    def to_lp(self, path: str | Path) -> None:
        """Writes the problem as an LP file."""
        self._core.backend_to_lp(Path(path))

    def lp_string(self) -> str:
        """The problem as LP file text."""
        return self._core.backend_lp_string()

    @property
    def has_integer_or_binary_variables(self) -> bool:
        """True if the problem is a MILP."""
        return self._core.backend_has_integer_or_binary_variables()


# ---- readers -----------------------------------------------------------------------------


def read_yaml(
    file: str | Path,
    scenario: str | None = None,
    override_dict: dict | None = None,
    math_dict: dict | None = None,
    data_table_dfs: Mapping[str, pd.DataFrame] | None = None,
    **kwargs: Any,
) -> Model:
    """`calliope.read_yaml`: a model from YAML (and CSV or in-memory data tables)."""
    core = _core.read_yaml(
        Path(file),
        scenario,
        override_dict,
        math_dict,
        _dfs_raw(data_table_dfs),
        _kwargs(kwargs),
    )
    return Model._from_core(core)


def read_dict(
    model_definition: dict,
    scenario: str | None = None,
    override_dict: dict | None = None,
    math_dict: dict | None = None,
    data_table_dfs: Mapping[str, pd.DataFrame] | None = None,
    definition_path: str | Path | None = None,
    **kwargs: Any,
) -> Model:
    """`calliope.read_dict`: a model from a definition dict."""
    core = _core.read_dict(
        dict(model_definition),
        scenario,
        override_dict,
        math_dict,
        _dfs_raw(data_table_dfs),
        None if definition_path is None else Path(definition_path),
        _kwargs(kwargs),
    )
    return Model._from_core(core)


def read_netcdf(path: str | Path, **kwargs: Any) -> Model:
    """`calliope.read_netcdf`: a model saved by `to_netcdf` (Rust or Python Calliope)."""
    return Model._from_core(_core.read_netcdf(Path(path), _kwargs(kwargs)))
