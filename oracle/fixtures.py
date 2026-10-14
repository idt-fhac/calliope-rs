"""Golden fixture definitions.

Each fixture is a model YAML + scenario + override_dict + init kwargs, run in a
given mode with the HiGHS backend (``build.backend: highs``) and fixed solver
options. Paths are relative to the Calliope checkout (``CALLIOPE_DIR``).

Exceptions to "HiGHS backend everywhere":
- ``docs_piecewise``: the upstream HiGHS backend has no piecewise constraints
  (``highs_backend_model.py:470``), so it is built with the Pyomo backend and
  solved with CBC (SOS2 support). Only its LP and inputs are HiGHS-independent.
"""

from __future__ import annotations

from dataclasses import dataclass, field

NATIONAL = "src/calliope/example_models/national_scale/model.yaml"
URBAN = "src/calliope/example_models/urban_scale/model.yaml"
TEST_MODEL = "tests/common/test_model/model.yaml"

#: Applied to every HiGHS solve. ``solve.solver_options`` is replaced wholesale.
HIGHS_OPTIONS = {"threads": 1, "random_seed": 0}

PIECEWISE_DATA = """
data_definitions:
  capacity_steps:
    data: [0, 2500, 5000, 7500, 10000]
    index: [0, 1, 2, 3, 4]
    dims: "breakpoints"
  cost_steps:
    data: [0, 3.75e6, 6e6, 7.5e6, 8e6]
    index: [0, 1, 2, 3, 4]
    dims: "breakpoints"
"""

# Verbatim from docs/examples/piecewise_constraints.py
PIECEWISE_MATH = """
dimensions:
  breakpoints:
    dtype: integer
    ordered: true
    iterator: breakpoint
parameters:
  capacity_steps:
    description: Capacity at each piecewise curve breakpoint
    unit: power
  cost_steps:
    description: Investment cost at each piecewise curve breakpoint
    unit: cost
variables:
  piecewise_cost_investment:
    description: "Investment cost that increases monotonically"
    foreach: ["nodes", "techs", "carriers", "costs"]
    where: "[csp] in techs"
    bounds:
      min: 0
      max: .inf
    default: 0
    unit: cost
global_expressions:
  cost_investment_flow_cap:
    where: (flow_cap AND (cost_flow_cap OR cost_flow_cap_per_distance)) OR piecewise_cost_investment
    equations:
      - expression: "$cost_sum * flow_cap"
        where: "NOT [csp] in techs"
      - expression: "piecewise_cost_investment"
        where: "[csp] in techs"
piecewise_constraints:
  csp_piecewise_costs:
    description: "Set investment costs values along a piecewise curve using special ordered sets of type 2 (SOS2)."
    foreach: ["nodes", "techs", "carriers", "costs"]
    where: "piecewise_cost_investment"
    x_expression: "flow_cap"
    x_values: "capacity_steps"
    y_expression: "piecewise_cost_investment"
    y_values: "cost_steps"
"""


@dataclass
class Fixture:
    name: str
    model: str
    scenario: str | None = None
    override_dict: dict = field(default_factory=dict)
    init_kwargs: dict = field(default_factory=dict)
    #: extra ``solve.solver_options`` on top of HIGHS_OPTIONS
    solver_options: dict = field(default_factory=dict)
    #: extra ``solve(**kwargs)``
    solve_kwargs: dict = field(default_factory=dict)
    backend: str = "highs"
    #: solver for the pyomo backend (only used when backend == "pyomo")
    solver: str = "highs"
    #: also write a Pyomo LP (for parity with tests/math/lp_files naming)
    pyomo_lp: bool = True
    #: seed for np.random (SPORES `random`)
    numpy_seed: int | None = 0
    #: piecewise example: yaml snippets
    yaml_override: str | None = None
    yaml_math: str | None = None
    notes: str = ""
    #: upstream raises during preprocess; the golden is the error message
    expect_error: bool = False


def _spores(alg: str) -> Fixture:
    return Fixture(
        name=f"national_scale_spores_{alg}",
        model=NATIONAL,
        scenario="spores",
        init_kwargs={"subset": {"timesteps": ["2005-01-01", "2005-01-10"]}},
        solve_kwargs={"spores.scoring_algorithm": alg},
        notes="docs/examples/modes.py SPORES loop, one algorithm per fixture",
    )


FIXTURES: list[Fixture] = [
    # --- calliope.examples variants ---------------------------------------
    Fixture("national_scale", NATIONAL),
    Fixture("national_scale_time_clustering", NATIONAL, "time_clustering"),
    Fixture("national_scale_time_resampling", NATIONAL, "time_resampling"),
    Fixture("urban_scale", URBAN),
    Fixture(
        "urban_scale_milp",
        URBAN,
        "milp",
        solver_options={"mip_rel_gap": 0.05},
        notes="scenario sets mipgap=0.05 (CBC name); mapped to HiGHS mip_rel_gap",
    ),
    Fixture("urban_scale_operate", URBAN, "operate"),
    Fixture(
        "urban_scale_operate_milp",
        URBAN,
        "operate,milp",
        solver_options={"mip_rel_gap": 0.05},
    ),
    # --- national_scale scenarios -----------------------------------------
    Fixture("national_scale_operate", NATIONAL, "operate"),
    Fixture("national_scale_check_feasibility", NATIONAL, "check_feasibility"),
    Fixture("national_scale_reserve_margin", NATIONAL, "reserve_margin"),
    Fixture(
        "national_scale_minimize_emissions",
        NATIONAL,
        "minimize_emissions_costs",
        expect_error=True,
        notes="stale upstream override; fails in preprocess (see bench/BUGS.md #12)",
    ),
    Fixture(
        "national_scale_maximize_utility",
        NATIONAL,
        "maximize_utility_costs",
        expect_error=True,
        notes="stale upstream override; fails in preprocess (see bench/BUGS.md #12)",
    ),
    Fixture("national_scale_capacity_factor", NATIONAL, "capacity_factor"),
    Fixture(
        "national_scale_cold_fusion",
        NATIONAL,
        "cold_fusion",
        expect_error=True,
        notes="stale upstream override; fails in preprocess (see bench/BUGS.md #12)",
    ),
    # --- docs/examples ------------------------------------------------------
    Fixture(
        "docs_modes_base",
        NATIONAL,
        init_kwargs={"subset": {"timesteps": ["2005-01-01", "2005-01-10"]}},
        notes="docs/examples/modes.py base run",
    ),
    *[
        _spores(a)
        for a in ("integer", "evolving_average", "random", "relative_deployment")
    ],
    Fixture(
        "docs_piecewise",
        NATIONAL,
        yaml_override=PIECEWISE_DATA,
        yaml_math=PIECEWISE_MATH,
        init_kwargs={"extra_math": ["piecewise_math"]},
        backend="pyomo",
        solver="cbc",
        notes="HiGHS backend lacks piecewise constraints; Pyomo backend + CBC",
    ),
    # --- tests/common/test_model scenarios (most used by the test suite) ---
    *[
        Fixture(f"test_model__{s.replace(',', '__')}", TEST_MODEL, s)
        for s in (
            "simple_supply,two_hours,investment_costs",
            "simple_supply,two_hours",
            "simple_storage,two_hours,investment_costs",
            "supply_milp,two_hours,investment_costs",
            "simple_supply_and_supply_plus,two_hours",
            "simple_supply,one_day",
            "simple_chp,two_hours",
            "fuel_distribution,two_hours",
            "supply_purchase,two_hours",
            "simple_supply,two_hours,investment_costs,shadow_prices",
            "conversion_and_conversion_plus,two_hours",
            "simple_supply_locations,one_day,investment_costs",
            "simple_supply,two_hours,weighted_costs",
            "simple_conversion,two_hours,investment_costs",
            "conversion_milp,two_hours",
            "simple_supply,operate,var_costs,investment_costs",
            "spores,var_costs,two_hours,simple_supply_spores_ready,investment_costs",
        )
    ],
]

BY_NAME = {f.name: f for f in FIXTURES}
