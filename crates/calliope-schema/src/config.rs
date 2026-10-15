//! `schemas/config_schema.py`: model configuration (`config.init`, `config.build`,
//! `config.solve`).

use std::path::PathBuf;

use calliope_yaml::Node;
use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::FieldError;
use crate::types::{AttrStr, IndexValue, NonEmptyList, UniqueList};
use crate::update::{DictShape, ModelShape, Schema, Shape};

/// `Mode`: the run mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    /// A single optimisation.
    #[default]
    Base,
    /// Rolling-horizon operation.
    Operate,
    /// SPORES (spatially explicit practically optimal results).
    Spores,
}

/// `build.backend`. Only HiGHS exists in the port; `pyomo` and `gurobi` are accepted with a
/// warning and fall back to HiGHS (SPEC §3.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Backend {
    /// Calliope's default; mapped to HiGHS.
    #[default]
    Pyomo,
    /// Mapped to HiGHS.
    Gurobi,
    /// HiGHS.
    Highs,
}

impl Backend {
    /// True for backends the port does not ship (they fall back to HiGHS with a warning).
    pub fn falls_back_to_highs(self) -> bool {
        !matches!(self, Backend::Highs)
    }
}

/// `SPORES_SCORING_OPTIONS`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SporesScoring {
    /// 1000 where capacity exceeds the threshold.
    #[default]
    Integer,
    /// Relative to the maximum possible deployment.
    RelativeDeployment,
    /// Random scores.
    Random,
    /// No added score.
    EvolvingAverage,
}

/// `distance_unit`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, JsonSchema)]
pub enum DistanceUnit {
    /// Kilometres.
    #[default]
    #[serde(rename = "km")]
    Km,
    /// Metres.
    #[serde(rename = "m")]
    M,
}

/// `Subsets`: dimension subsets.
pub type Subsets = IndexMap<AttrStr, Option<NonEmptyList<IndexValue>>>;
/// `Resamples`: dimension resampling frequencies.
pub type Resamples = IndexMap<AttrStr, Option<String>>;
/// `MathPaths`: named math files.
pub type MathPaths = IndexMap<AttrStr, PathBuf>;

/// `Init`: options used when initialising a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model initialisation configuration")]
pub struct Init {
    /// Model name.
    pub name: Option<String>,
    /// Calliope framework version this model is intended for.
    pub calliope_version: Option<String>,
    /// Broadcast single YAML data entries across all index items.
    pub broadcast_input_data: bool,
    /// Keep deactivated techs and nodes in the input data.
    pub retain_inactive: bool,
    /// Inclusive dimension subsets.
    pub subset: Subsets,
    /// Datetime dimension resampling (e.g. `2h`).
    pub resample: Resamples,
    /// Name of the input array to cluster the timeseries by.
    pub time_cluster: Option<AttrStr>,
    /// Timestamp format of datetime data read from file.
    pub datetime_format: String,
    /// Datestamp format of date data read from file.
    pub date_format: String,
    /// Unit of transmission link `distance`.
    pub distance_unit: DistanceUnit,
    /// Run mode.
    pub mode: Mode,
    /// Extra math entries, applied after base and mode math.
    pub extra_math: UniqueList<String>,
    /// Named math files.
    pub math_paths: MathPaths,
    /// Parse all math strings at initialisation.
    pub pre_validate_math_strings: bool,
}

impl Default for Init {
    fn default() -> Self {
        Self {
            name: None,
            calliope_version: None,
            broadcast_input_data: false,
            retain_inactive: false,
            subset: IndexMap::new(),
            resample: IndexMap::new(),
            time_cluster: None,
            datetime_format: "ISO8601".to_owned(),
            date_format: "ISO8601".to_owned(),
            distance_unit: DistanceUnit::Km,
            mode: Mode::Base,
            extra_math: UniqueList::default(),
            math_paths: IndexMap::new(),
            pre_validate_math_strings: false,
        }
    }
}

/// `BuildOperate`: operate-mode build options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model build operate mode configuration")]
pub struct BuildOperate {
    /// Rolling window, as a pandas frequency string.
    pub window: String,
    /// Rolling horizon, as a pandas frequency string. Must be ≥ `window`.
    pub horizon: String,
}

impl Default for BuildOperate {
    fn default() -> Self {
        Self {
            window: "24h".to_owned(),
            horizon: "48h".to_owned(),
        }
    }
}

/// `Build`: options used when building the optimisation problem.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model build configuration")]
pub struct Build {
    /// Backend; anything but `highs` falls back to HiGHS in the port.
    pub backend: Backend,
    /// Add unmet-demand/unused-supply variables so the problem is always feasible.
    pub ensure_feasibility: bool,
    /// Name of the objective to use.
    pub objective: String,
    /// Operate mode options.
    pub operate: BuildOperate,
}

impl Default for Build {
    fn default() -> Self {
        Self {
            backend: Backend::Pyomo,
            ensure_feasibility: false,
            objective: "min_cost_optimisation".to_owned(),
            operate: BuildOperate::default(),
        }
    }
}

/// `SolveSpores`: SPORES options.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model solve SPORES mode configuration")]
pub struct SolveSpores {
    /// Scoring algorithm.
    pub scoring_algorithm: SporesScoring,
    /// Number of SPORES after the baseline run.
    pub number: i64,
    /// Directory in which to save each SPORE as soon as it is solved.
    pub save_per_spore_path: Option<PathBuf>,
    /// Continue from results already in the model.
    pub use_latest_results: bool,
    /// Input parameter that filters which techs are scored.
    pub tracking_parameter: Option<String>,
    /// Capacity threshold factor for incrementing the score (`ge=0`).
    #[schemars(range(min = 0))]
    pub score_threshold_factor: f64,
}

impl Default for SolveSpores {
    fn default() -> Self {
        Self {
            scoring_algorithm: SporesScoring::Integer,
            number: 3,
            save_per_spore_path: None,
            use_latest_results: false,
            tracking_parameter: None,
            score_threshold_factor: 0.1,
        }
    }
}

/// `Solve`: options used when solving.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model Solve Configuration")]
pub struct Solve {
    /// Run the active postprocessing after solving.
    pub postprocessing_active: bool,
    /// Directory for solver logs.
    pub save_logs: Option<PathBuf>,
    /// Constraints whose shadow prices are returned.
    pub shadow_prices: UniqueList<String>,
    /// Solver name (the port always uses HiGHS).
    pub solver: String,
    /// Solver interface (unused by the port).
    pub solver_io: Option<String>,
    /// Options passed to the solver.
    #[schemars(with = "IndexMap<String, crate::types::AnyValue>")]
    pub solver_options: IndexMap<String, Node>,
    /// SPORES options.
    pub spores: SolveSpores,
    /// Results smaller than this are set to zero.
    pub zero_threshold: f64,
}

impl Default for Solve {
    fn default() -> Self {
        Self {
            postprocessing_active: true,
            save_logs: None,
            shadow_prices: UniqueList::default(),
            solver: "cbc".to_owned(),
            solver_io: None,
            solver_options: IndexMap::new(),
            spores: SolveSpores::default(),
            zero_threshold: 1e-10,
        }
    }
}

/// `CalliopeConfig`: the `config` section of a model.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields, default)]
#[schemars(title = "Model configuration schema")]
pub struct CalliopeConfig {
    /// Initialisation options.
    pub init: Init,
    /// Build options.
    pub build: Build,
    /// Solve options.
    pub solve: Solve,
}

static SUBSETS: DictShape = DictShape {
    name: "Subsets",
    entry: Shape::Leaf,
};
static RESAMPLES: DictShape = DictShape {
    name: "Resamples",
    entry: Shape::Leaf,
};
static MATH_PATHS: DictShape = DictShape {
    name: "MathPaths",
    entry: Shape::Leaf,
};
pub(crate) static INIT: ModelShape = ModelShape {
    name: "Init",
    fields: &[
        ("subset", Shape::Dict(&SUBSETS)),
        ("resample", Shape::Dict(&RESAMPLES)),
        ("math_paths", Shape::Dict(&MATH_PATHS)),
    ],
    extra: Shape::Leaf,
};
static BUILD_OPERATE: ModelShape = ModelShape {
    name: "BuildOperate",
    fields: &[],
    extra: Shape::Leaf,
};
pub(crate) static BUILD: ModelShape = ModelShape {
    name: "Build",
    fields: &[("operate", Shape::Model(&BUILD_OPERATE))],
    extra: Shape::Leaf,
};
static SOLVE_SPORES: ModelShape = ModelShape {
    name: "SolveSpores",
    fields: &[],
    extra: Shape::Leaf,
};
pub(crate) static SOLVE: ModelShape = ModelShape {
    name: "Solve",
    fields: &[("spores", Shape::Model(&SOLVE_SPORES))],
    extra: Shape::Leaf,
};
pub(crate) static CONFIG: ModelShape = ModelShape {
    name: "CalliopeConfig",
    fields: &[
        ("init", Shape::Model(&INIT)),
        ("build", Shape::Model(&BUILD)),
        ("solve", Shape::Model(&SOLVE)),
    ],
    extra: Shape::Leaf,
};

fn validate_spores(s: &SolveSpores, path: &str, errors: &mut Vec<FieldError>) {
    if s.score_threshold_factor.is_nan() || s.score_threshold_factor < 0.0 {
        errors.push(FieldError::new(
            &crate::join(path, "score_threshold_factor"),
            "Input should be greater than or equal to 0",
        ));
    }
}

impl Schema for CalliopeConfig {
    const TITLE: &'static str = "Model configuration schema";
    fn shape() -> Shape {
        Shape::Model(&CONFIG)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        self.solve.validate(&crate::join(path, "solve"), errors);
    }
}

impl Schema for Init {
    const TITLE: &'static str = "Model initialisation configuration";
    fn shape() -> Shape {
        Shape::Model(&INIT)
    }
}

impl Schema for Build {
    const TITLE: &'static str = "Model build configuration";
    fn shape() -> Shape {
        Shape::Model(&BUILD)
    }
}

impl Schema for Solve {
    const TITLE: &'static str = "Model Solve Configuration";
    fn shape() -> Shape {
        Shape::Model(&SOLVE)
    }
    fn validate(&self, path: &str, errors: &mut Vec<FieldError>) {
        validate_spores(&self.spores, &crate::join(path, "spores"), errors);
    }
}
