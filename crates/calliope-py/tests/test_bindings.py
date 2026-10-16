"""calliope_rs through its Python API.

- the example-model test of `tests/example_models/example_models_test.py` that needs
  in-memory DataFrames (`national_scale_from_data_tables`);
- golden fixtures (`golden/`, written by Python Calliope): inputs and results compared
  with the tolerances of `xtask parity`;
- NetCDF round trips (also read by Python Calliope, when importable), CSV export, the
  backend interface and `Model(inputs, attrs, results)`.

Paths: the Calliope checkout is `$CALLIOPE_REF` (default `../calliope` next to this
repository); fixtures are skipped if it is missing.
"""

from __future__ import annotations

import json
import math
import os
from pathlib import Path

import numpy as np
import pandas as pd
import pytest
import xarray as xr

import calliope_rs

ROOT = Path(__file__).resolve().parents[3]
REF = Path(os.environ.get("CALLIOPE_REF", ROOT.parent / "calliope"))
GOLDEN = ROOT / "golden"
EXAMPLES = REF / "src" / "calliope" / "example_models"

needs_ref = pytest.mark.skipif(
    not EXAMPLES.is_dir(), reason="Calliope checkout not found"
)

QUIET = {"solver_options": {"output_flag": False}}

# Fixtures that `xtask parity` passes without classification (crates/xtask/src/known.rs)
# and with every result array equal.
GOLDEN_FIXTURES = [
    "national_scale",
    "national_scale_operate",
    "national_scale_time_clustering",
    "national_scale_time_resampling",
    "urban_scale_operate",
    "test_model__conversion_milp__two_hours",
    "test_model__simple_chp__two_hours",
    "test_model__simple_storage__two_hours__investment_costs",
    "test_model__supply_purchase__two_hours",
]
# Fixtures whose objectives match but which end on another optimal vertex (degenerate
# LPs; `xtask parity` reports these as notes): only inputs and objectives are compared.
OBJECTIVE_ONLY = ["national_scale_spores_integer"]


def approx(a: float, b: float, rel: float = 1e-6, abs_: float = 1e-12) -> bool:
    return math.isclose(a, b, rel_tol=rel, abs_tol=abs_)


# ---- national_scale_from_data_tables -------------------------------------------------------


@pytest.fixture(scope="module")
def nat_model_from_data_tables() -> calliope_rs.Model:
    df = pd.read_csv(
        EXAMPLES / "national_scale" / "data_tables" / "time_varying_params.csv",
        index_col=0,
        header=[0, 1, 2, 3],
    )
    model = calliope_rs.read_yaml(
        REF
        / "tests"
        / "example_models"
        / "national_scale_from_data_tables"
        / "model.yaml",
        data_table_dfs={"time_varying_df": df},
        subset={"timesteps": ["2005-01-01", "2005-01-01"]},
    )
    model.build()
    return model


@pytest.fixture(scope="module")
def nat_model() -> calliope_rs.Model:
    model = calliope_rs.read_yaml(
        EXAMPLES / "national_scale" / "model.yaml",
        subset={"timesteps": ["2005-01-01", "2005-01-01"]},
    )
    model.build()
    return model


@needs_ref
@pytest.mark.parametrize("which", ["nat_model", "nat_model_from_data_tables"])
def test_nationalscale_example_results(which, request):
    """`TestNationalScaleExampleModelSenseChecks` (with HiGHS instead of CBC)."""
    model = request.getfixturevalue(which)
    model.solve(force=True, **QUIET)
    r = model.results
    assert approx(r.storage_cap.sel(nodes="region1_1", techs="csp").item(), 45129.950)
    assert approx(r.storage_cap.sel(nodes="region2", techs="battery").item(), 6675.173)
    assert approx(r.flow_cap.sel(nodes="region1_1", techs="csp").item(), 4626.588)
    assert approx(r.flow_cap.sel(nodes="region2", techs="battery").item(), 1000)
    assert approx(r.flow_cap.sel(nodes="region1", techs="ccgt").item(), 30000)
    assert approx(float(r.cost.sum()), 38988.7442)
    assert approx(
        r.systemwide_levelised_cost.sel(carriers="power", techs="battery").item(),
        0.063543,
        rel=0,
        abs_=1e-6,
    )
    assert approx(
        r.systemwide_capacity_factor.sel(carriers="power", techs="battery").item(),
        0.2642256,
        rel=0,
        abs_=1e-6,
    )


@needs_ref
def test_data_tables_model_has_the_same_inputs(nat_model, nat_model_from_data_tables):
    a, b = nat_model.inputs, nat_model_from_data_tables.inputs
    for name in ["sink_use_equals", "source_use_max", "flow_cap_max", "cost_flow_cap"]:
        xr.testing.assert_allclose(a[name], b[name].reindex_like(a[name]))


@needs_ref
def test_data_table_must_be_a_dataframe():
    with pytest.raises(calliope_rs.ModelError, match="must be a pandas DataFrame"):
        calliope_rs.read_yaml(
            REF
            / "tests"
            / "example_models"
            / "national_scale_from_data_tables"
            / "model.yaml",
            data_table_dfs={"time_varying_df": pd.Series([1.0])},
        )


@needs_ref
def test_missing_data_table_df():
    with pytest.raises(calliope_rs.ModelError, match="time_varying_df"):
        calliope_rs.read_yaml(
            REF
            / "tests"
            / "example_models"
            / "national_scale_from_data_tables"
            / "model.yaml"
        )


# ---- golden fixtures -------------------------------------------------------------------------


def _meta(name: str) -> dict:
    return json.loads((GOLDEN / name / "meta.json").read_text())


def _run_fixture(name: str) -> calliope_rs.Model:
    fx = _meta(name)["fixture"]
    model = calliope_rs.read_yaml(
        REF / fx["model"],
        scenario=fx["scenario"],
        override_dict=fx["override_dict"],
        math_dict=fx["math_dict"],
        **(fx["init_kwargs"] or {}),
    )
    if fx.get("numpy_seed") is not None:
        model.set_random_seed(fx["numpy_seed"])
    model.build()
    kwargs = dict(fx["solve_kwargs"] or {})
    kwargs.setdefault("solver_options", {})["output_flag"] = False
    model.solve(**kwargs)
    return model


def _assert_close(name: str, got: xr.DataArray, want: xr.DataArray, tol: float) -> None:
    got = got.transpose(*want.dims)
    assert set(got.dims) == set(want.dims), name
    for d in want.dims:
        assert list(got.indexes[d]) == list(want.indexes[d]), f"{name}: {d}"
    g, w = got.values, want.values
    if g.dtype.kind in "fiub" and w.dtype.kind in "fiub":
        g, w = g.astype(float), w.astype(float)
        ok = np.isclose(g, w, rtol=tol, atol=tol) | (np.isnan(g) & np.isnan(w))
        assert ok.all(), f"{name}: {np.count_nonzero(~ok)} cells differ"
    else:
        same = [
            (pd.isna(x) and pd.isna(y)) or str(x) == str(y)
            for x, y in zip(g.ravel(), w.ravel(), strict=True)
        ]
        assert all(same), name


fixture_params = [
    pytest.param(
        n,
        marks=pytest.mark.skipif(not (GOLDEN / n).is_dir(), reason="no golden files"),
    )
    for n in GOLDEN_FIXTURES + OBJECTIVE_ONLY
]


@needs_ref
@pytest.mark.parametrize("name", fixture_params)
def test_golden_fixture(name, tmp_path):
    model = _run_fixture(name)
    meta = _meta(name)
    assert model.runtime.termination_condition == meta["termination_condition"]

    with xr.open_dataset(GOLDEN / name / "inputs.nc", group="inputs") as want:
        got = model.inputs
        assert set(got.data_vars) == set(want.data_vars)
        for v in want.data_vars:
            _assert_close(v, got[v], want[v].load(), 1e-9)

    with xr.open_dataset(GOLDEN / name / "results.nc", group="results") as want:
        got = model.results
        assert set(got.data_vars) == set(want.data_vars)
        if name not in OBJECTIVE_ONLY:
            for v in want.data_vars:
                _assert_close(v, got[v], want[v].load(), 1e-6)

    for obj, value in meta["objectives"].items():
        if obj in model.results:
            got_obj = model.results[obj].values.ravel()
            assert np.allclose(got_obj, np.ravel(value), rtol=1e-6, atol=0), obj


# ---- I/O ---------------------------------------------------------------------------------------


@pytest.fixture(scope="module")
def solved() -> calliope_rs.Model:
    model = calliope_rs.read_yaml(
        EXAMPLES / "national_scale" / "model.yaml",
        subset={"timesteps": ["2005-01-01", "2005-01-02"]},
    )
    model.build()
    model.solve(**QUIET)
    return model


@needs_ref
def test_netcdf_round_trip(solved, tmp_path):
    path = tmp_path / "model.nc"
    solved.to_netcdf(path)
    back = calliope_rs.read_netcdf(path)
    assert back.is_solved
    assert back.name == solved.name
    xr.testing.assert_identical(back.results, solved.results)
    xr.testing.assert_equal(back.inputs, solved.inputs)


@needs_ref
def test_python_calliope_reads_our_netcdf(solved, tmp_path):
    calliope = pytest.importorskip("calliope")
    path = tmp_path / "model.nc"
    solved.to_netcdf(path)
    py = calliope.read_netcdf(path)
    assert py.name == solved.name
    for v in solved.results.data_vars:
        _assert_close(v, py.results[v], solved.results[v], 1e-12)


@needs_ref
def test_to_csv(solved, tmp_path):
    out = tmp_path / "csv"
    solved.to_csv(out)
    assert (out / "attrs.yaml").is_file()
    flow_cap = pd.read_csv(out / "results_flow_cap.csv")
    assert {"nodes", "techs", "carriers", "flow_cap"} <= set(flow_cap.columns)
    with pytest.raises(OSError):
        solved.to_csv(out)
    solved.to_csv(out, allow_overwrite=True)


@needs_ref
def test_attributes(solved):
    assert solved.config.init.name == "National-scale example model"
    assert solved.config.init.subset.timesteps == ["2005-01-01", "2005-01-02"]
    assert "base" in solved.math.init
    assert "flow_cap" in solved.math.build.variables
    assert solved.runtime.termination_condition == "optimal"
    assert "Model name:   National-scale example model" in solved.info()


# ---- backend interface -------------------------------------------------------------------------


@pytest.fixture
def built() -> calliope_rs.Model:
    model = calliope_rs.read_yaml(
        EXAMPLES / "national_scale" / "model.yaml",
        subset={"timesteps": ["2005-01-01", "2005-01-01"]},
    )
    model.build()
    return model


@needs_ref
def test_backend_parameters_and_update_input(built):
    b = built.backend
    cost = b.get_parameter("cost_flow_cap")
    assert cost.sel(costs="monetary", techs="ccgt").item() == 750
    built.solve(**QUIET)
    before = float(built.results.cost.sum())
    b.update_input("cost_flow_cap", cost * 2)
    assert (
        b.get_parameter("cost_flow_cap").sel(costs="monetary", techs="ccgt").item()
        == 1500
    )
    built.solve(force=True, **QUIET)
    assert float(built.results.cost.sum()) > before


@needs_ref
def test_backend_variables_constraints_objective(built):
    b = built.backend
    built.solve(**QUIET)
    flow_cap = b.get_variable("flow_cap")
    xr.testing.assert_allclose(
        flow_cap.where(built.results.flow_cap.notnull()),
        built.results.flow_cap.transpose(*flow_cap.dims),
    )
    bounds = b.get_variable_bounds("flow_cap")
    assert set(bounds.data_vars) == {"lb", "ub"}
    con = b.get_constraint("system_balance")
    assert {"lb", "ub", "body"} <= set(con.data_vars)
    assert approx(
        b.get_objective(b.objective), float(built.results.min_cost_optimisation)
    )
    assert "flow_cap" in b.component_names("variables")
    with pytest.raises(NotImplementedError):
        b.get_variable("flow_cap", as_backend_objs=True)


@needs_ref
def test_backend_fix_variable_and_bounds(built):
    b = built.backend
    built.solve(**QUIET)
    b.fix_variable("flow_cap")
    bounds = b.get_variable_bounds("flow_cap")
    fixed = (bounds.lb == bounds.ub) | bounds.lb.isnull()
    assert bool(fixed.all())
    b.unfix_variable("flow_cap")
    with pytest.raises(calliope_rs.BackendError, match="set by parameters"):
        b.update_variable_bounds("storage_cap", max=xr.DataArray(1e5))
    b.update_variable_bounds("unmet_demand", max=xr.DataArray(1e9))
    assert float(b.get_variable_bounds("unmet_demand").ub.max()) == 1e9
    built.solve(force=True, **QUIET)
    assert built.runtime.termination_condition == "optimal"


@needs_ref
def test_backend_lp(built, tmp_path):
    b = built.backend
    b.verbose_strings()
    b.to_lp(tmp_path / "m.lp")
    text = (tmp_path / "m.lp").read_text()
    assert "flow_cap" in text
    assert text == b.lp_string()
    assert not b.has_integer_or_binary_variables


# ---- Model(inputs, attrs, results) ---------------------------------------------------------------


@needs_ref
def test_operate_from_base_results(solved):
    """Base-mode capacities fix the operate-mode inputs (`docs/advanced/mode.md`)."""
    caps = ["flow_cap", "storage_cap", "area_use", "source_cap"]
    inputs = solved.inputs.assign(
        solved.results[[c for c in caps if c in solved.results]]
    )
    # Operate mode needs cyclic storage off (as the `operate` scenario sets).
    inputs["cyclic_storage"] = xr.zeros_like(inputs.techs, dtype=bool)
    model = calliope_rs.Model(
        inputs=inputs,
        attrs=solved.all_attrs(),
        mode="operate",
    )
    assert model.config.init.mode == "operate"
    model.build(operate={"window": "12h", "horizon": "24h"})
    model.solve(**QUIET)
    assert model.runtime.termination_condition == "optimal"
    assert model.results.flow_out.sizes["timesteps"] == solved.inputs.sizes["timesteps"]


@needs_ref
def test_model_with_results_is_solved(solved):
    model = calliope_rs.Model(solved.inputs, solved.all_attrs(), solved.results)
    assert model.is_solved
    xr.testing.assert_identical(model.results, solved.results)


@needs_ref
def test_read_dict_and_errors():
    calliope = pytest.importorskip("calliope")
    definition = calliope.io.read_rich_yaml(EXAMPLES / "national_scale" / "model.yaml")
    model = calliope_rs.read_dict(
        dict(definition),
        definition_path=EXAMPLES / "national_scale" / "model.yaml",
        subset={"timesteps": ["2005-01-01", "2005-01-01"]},
    )
    assert model.inputs.sizes["timesteps"] == 24
    with pytest.raises(calliope_rs.ModelError, match="already has a built"):
        model.build()
        model.build()
