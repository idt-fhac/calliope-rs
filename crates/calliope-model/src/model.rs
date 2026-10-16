//! `calliope.Model`: init, build, solve, results, save.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use calliope_io::{AttrValue, Attrs, ModelNetcdf, NcGroup};
use calliope_math::eval::run_checks;
use calliope_preprocess::backend::{backend_dataset, math_inputs};
use calliope_preprocess::model_data::ModelDataCleaner;
use calliope_preprocess::xr::Dataset;
use calliope_preprocess::{ModelInputs, ReadOptions, read_dict, read_yaml};
use calliope_schema::{
    CalliopeBuildMath, CalliopeConfig, CalliopeModelDef, CalliopeRuntime, Mode, Validated,
};
use calliope_yaml::{AttrTree, Node, Value as YValue};
use indexmap::IndexMap;

use crate::backend::Backend;
use crate::results::Results;
use crate::rng::Mt19937;
use crate::{Error, operate, spores};

/// Options of [`Model::build`] (`Model.build(force, **kwargs)`).
#[derive(Debug, Clone, Default)]
pub struct BuildOptions {
    /// Overwrite an existing built problem.
    pub force: bool,
    /// `config.build` overrides (dotted keys allowed).
    pub overrides: AttrTree,
}

/// Options of [`Model::solve`] (`Model.solve(force, warmstart, **kwargs)`).
#[derive(Debug, Clone, Default)]
pub struct SolveOptions {
    /// Overwrite existing results.
    pub force: bool,
    /// Start from the previous solution (base mode).
    pub warmstart: bool,
    /// `config.solve` overrides (dotted keys allowed).
    pub overrides: AttrTree,
}

/// A Calliope model.
#[derive(Debug)]
pub struct Model {
    /// Input data (`model.inputs`).
    pub inputs: Dataset,
    /// Results (`model.results`; empty before a successful solve).
    pub results: Results,
    /// `model.config`.
    pub config: Validated<CalliopeConfig>,
    /// `model.definition`.
    pub definition: Validated<CalliopeModelDef>,
    /// `model.math.init`.
    pub math_init: IndexMap<String, AttrTree>,
    /// `model.math.build`.
    pub math: Validated<CalliopeBuildMath>,
    /// `model.runtime`.
    pub runtime: CalliopeRuntime,
    /// Warnings raised while reading the model.
    pub warnings: Vec<String>,
    pub(crate) backend: Option<Backend>,
    pub(crate) start_window_idx: usize,
    pub(crate) rng: Mt19937,
}

fn now() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |d| d.as_secs_f64())
}

/// An override tree `{group: overrides}` with dotted keys of `overrides` expanded.
fn nested(group: &str, overrides: &AttrTree) -> Result<AttrTree, Error> {
    let mut inner = AttrTree::new();
    for (k, v) in overrides.iter() {
        inner
            .set_key(k, v.clone())
            .map_err(|e| Error::Model(e.to_string()))?;
    }
    let mut t = AttrTree::new();
    t.insert(group, Node::new(YValue::Map(inner)));
    Ok(t)
}

impl Model {
    /// `calliope.read_yaml(path, scenario, override_dict, math_dict, **kwargs)`.
    pub fn from_yaml(path: &Path, opts: &ReadOptions) -> Result<Self, Error> {
        let start = now();
        let m = read_yaml(path, opts)?;
        Ok(Self::from_inputs(m, start))
    }

    /// `calliope.read_dict(model_definition, ...)`.
    pub fn from_dict(
        definition: &AttrTree,
        definition_path: Option<&Path>,
        opts: &ReadOptions,
    ) -> Result<Self, Error> {
        let start = now();
        let m = read_dict(definition, definition_path, opts)?;
        Ok(Self::from_inputs(m, start))
    }

    /// A model from preprocessed inputs.
    pub fn from_inputs(m: ModelInputs, preprocess_start: f64) -> Self {
        let mut runtime = m.runtime;
        runtime
            .timings
            .insert("preprocess_start".to_owned(), preprocess_start);
        runtime.timings.insert("init_complete".to_owned(), now());
        Self {
            inputs: m.inputs,
            results: Results::default(),
            config: m.config,
            definition: m.definition,
            math_init: m.math_init,
            math: m.math,
            runtime,
            warnings: m.warnings,
            backend: None,
            start_window_idx: 0,
            rng: Mt19937::new(0),
        }
    }

    /// `calliope.read_netcdf(path)`: a model saved by [`Model::to_netcdf`] or by Python
    /// Calliope, with its results (if any).
    pub fn from_netcdf(path: &Path) -> Result<Self, Error> {
        let nc = calliope_io::read_netcdf(path)?;
        let dict = |key: &str| -> AttrTree {
            match nc.attrs.get(key) {
                Some(AttrValue::Dict(t)) => t.clone(),
                _ => AttrTree::new(),
            }
        };
        let config = Validated::<CalliopeConfig>::from_tree(dict("config"))?;
        let definition = Validated::<CalliopeModelDef>::from_tree(dict("definition"))?;
        let math_attr = dict("math");
        let math_init: IndexMap<String, AttrTree> = math_attr
            .get("init")
            .and_then(Node::as_map)
            .map(|t| {
                t.iter()
                    .filter_map(|(k, v)| v.as_map().map(|m| (k.clone(), m.clone())))
                    .collect()
            })
            .unwrap_or_default();
        let math = match math_attr.get("build").and_then(Node::as_map) {
            Some(b) => Validated::<CalliopeBuildMath>::from_tree(b.clone())?,
            None => {
                let priority = calliope_preprocess::model_math::get_math_priority(&config.init);
                calliope_preprocess::model_math::build_math(&priority, &math_init, None)?
            }
        };
        let mut runtime_tree = dict("runtime");
        // Python stores timings etc. as plain values; drop what the schema cannot take.
        runtime_tree.remove("calliope_version_defined");
        let runtime: CalliopeRuntime = calliope_yaml::from_tree(&runtime_tree)
            .map_err(|e| Error::Model(format!("runtime attributes: {e}")))?;
        let inputs = if nc.inputs.ds.vars.is_empty() {
            nc.inputs.ds
        } else {
            let mut c = ModelDataCleaner::new(&config.init, nc.inputs.ds, &math, runtime.clone());
            c.clean()?;
            c.dataset
        };
        let termination = runtime.termination_condition.clone().unwrap_or_default();
        let results = Results::from_dataset(&nc.results.ds, termination)?;
        let mut runtime = runtime;
        runtime.timings.insert("init_complete".to_owned(), now());
        Ok(Self {
            inputs,
            results,
            config,
            definition,
            math_init,
            math,
            runtime,
            warnings: Vec::new(),
            backend: None,
            start_window_idx: 0,
            rng: Mt19937::new(0),
        })
    }

    /// `model.name`.
    pub fn name(&self) -> Option<&str> {
        self.config.init.name.as_deref()
    }

    /// True once [`Model::build`] has run.
    pub fn is_built(&self) -> bool {
        self.backend.is_some()
    }

    /// True if there are results.
    pub fn is_solved(&self) -> bool {
        !self.results.is_empty()
    }

    /// `model.backend`.
    pub fn backend(&self) -> Option<&Backend> {
        self.backend.as_ref()
    }

    /// `model.backend` (mutable, for the backend interface).
    pub fn backend_mut(&mut self) -> Option<&mut Backend> {
        self.backend.as_mut()
    }

    /// Seeds the generator of SPORES `random` scoring (`np.random.seed`); the default
    /// seed is 0. Python uses numpy's global, unseeded generator.
    pub fn set_random_seed(&mut self, seed: u32) {
        self.rng = Mt19937::new(seed);
    }

    pub(crate) fn log_time(&mut self, key: &str) {
        self.runtime.timings.insert(key.to_owned(), now());
    }

    /// `Model.info()`.
    pub fn info(&self) -> String {
        let dims = self
            .inputs
            .coords
            .iter()
            .map(|(k, c)| format!("'{k}': {}", c.len()))
            .collect::<Vec<_>>()
            .join(", ");
        let n_active = self.inputs.vars.get("active").map_or(0, |v| {
            (0..v.data.len())
                .filter(|&i| v.data.get(i).truthy())
                .count()
        });
        format!(
            "Model name:   {}\nModel size:   {{{dims}}} ({n_active} valid node:tech:carrier combinations)",
            self.name().unwrap_or("None")
        )
    }

    /// The backend for the operate window starting at `windowsteps[idx]` (all timesteps
    /// in other modes): inputs sliced and cleaned of undefined members, math `checks` run.
    pub(crate) fn make_backend(
        &self,
        ds: &Dataset,
        storage_initial: Option<&calliope_math::eval::Value>,
    ) -> Result<Backend, Error> {
        let mut inputs = math_inputs(ds, &self.math)?;
        let out =
            run_checks(&inputs, &self.math, &self.config.build).map_err(|e| Error::Model(e.0))?;
        if let Some(e) = out.error_message() {
            return Err(Error::Model(e));
        }
        if let Some(w) = out.warning_message() {
            tracing::warn!("{w}");
        }
        if let Some(si) = storage_initial {
            inputs
                .arrays
                .insert("storage_initial".to_owned(), si.clone());
        }
        Backend::new(inputs, (*self.math).clone(), self.config.build.clone())
    }

    /// `Model.build(force, **kwargs)`.
    pub fn build(&mut self, opts: &BuildOptions) -> Result<(), Error> {
        if self.backend.is_some() && !opts.force {
            return Err(Error::Model(
                "This model object already has a built optimisation problem. Use model.build(force=True) to force the existing optimisation problem to be overwritten with a new one.".into(),
            ));
        }
        self.log_time("build_start");
        if !opts.overrides.is_empty() {
            self.config = self
                .config
                .update(&nested("build", &opts.overrides)?, true)?;
        }
        let backend = if self.config.init.mode == Mode::Operate {
            let ds = operate::window_dataset(self, self.start_window_idx)?;
            self.make_backend(&ds, None)?
        } else {
            let model = self.as_inputs();
            let ds = backend_dataset(&model)?;
            self.make_backend(&ds, None)?
        };
        self.backend = Some(backend);
        self.log_time("build_complete");
        Ok(())
    }

    /// The model as [`ModelInputs`] (for the preprocessing helpers).
    fn as_inputs(&self) -> ModelInputs {
        ModelInputs {
            inputs: self.inputs.clone(),
            config: self.config.clone(),
            definition: self.definition.clone(),
            math_init: self.math_init.clone(),
            math: self.math.clone(),
            runtime: self.runtime.clone(),
            warnings: Vec::new(),
        }
    }

    /// `Model.solve(force, warmstart, **kwargs)`.
    pub fn solve(&mut self, opts: &SolveOptions) -> Result<(), Error> {
        if self.backend.is_none() {
            return Err(Error::Model(
                "You must build the optimisation problem (`.build()`) before you can run it."
                    .into(),
            ));
        }
        if !self.results.is_empty() && !opts.force {
            return Err(Error::Model(
                "This model object already has results. Use model.solve(force=True) to forcethe existing results to be overwritten with a new run.".into(),
            ));
        }
        if !opts.overrides.is_empty() {
            self.config = self
                .config
                .update(&nested("solve", &opts.overrides)?, true)?;
        }
        self.log_time("solve_start");
        let solve = self.config.solve.clone();
        let results = match self.config.init.mode {
            Mode::Operate => operate::solve_operate(self, &solve)?,
            Mode::Spores => spores::solve_spores(self, &solve)?,
            Mode::Base => self
                .backend
                .as_mut()
                .expect("checked")
                .solve(&solve, opts.warmstart)?,
        };
        self.log_time("solver_exit");
        self.runtime.termination_condition = Some(results.termination_condition.clone());
        self.log_time("postprocess_complete");
        self.log_time("solve_complete");
        self.results = results;
        Ok(())
    }

    /// `dump_all_attrs()`: `definition`, `config`, `math` and `runtime` as set.
    pub fn dump_all_attrs(&self) -> Attrs {
        let mut math = AttrTree::new();
        let mut init = AttrTree::new();
        for (k, v) in &self.math_init {
            init.insert(k.clone(), Node::new(YValue::Map(v.clone())));
        }
        math.insert("init", Node::new(YValue::Map(init)));
        math.insert("build", Node::new(YValue::Map(self.math.dump_set())));
        let mut a = Attrs::new();
        a.insert(
            "definition".into(),
            AttrValue::Dict(self.definition.dump_set()),
        );
        a.insert("config".into(), AttrValue::Dict(self.config.dump_set()));
        a.insert("math".into(), AttrValue::Dict(math));
        a.insert(
            "runtime".into(),
            AttrValue::Dict(runtime_tree(&self.runtime)),
        );
        a
    }

    /// The model as NetCDF groups (`inputs`, `results`, `attrs`).
    pub fn to_model_netcdf(&self) -> Result<ModelNetcdf, Error> {
        Ok(ModelNetcdf {
            inputs: NcGroup {
                ds: self.inputs.clone(),
                ..NcGroup::default()
            },
            results: NcGroup {
                ds: self.results.to_dataset()?,
                ..NcGroup::default()
            },
            attrs: self.dump_all_attrs(),
        })
    }

    /// `Model.to_netcdf(path)`.
    pub fn to_netcdf(&self, path: &Path) -> Result<(), Error> {
        calliope_io::write_netcdf(path, &self.to_model_netcdf()?)?;
        Ok(())
    }

    /// `Model.to_csv(path, dropna, allow_overwrite)`.
    pub fn to_csv(&self, dir: &Path, dropna: bool, allow_overwrite: bool) -> Result<(), Error> {
        if self.results.is_empty() {
            tracing::warn!("No results available, saving inputs only.");
        }
        calliope_io::model_to_csv(&self.to_model_netcdf()?, dir, dropna, allow_overwrite)?;
        Ok(())
    }
}

/// `runtime.model_dump(exclude_unset=True)`, approximately: unset optional values and
/// empty mappings are left out.
fn runtime_tree(r: &CalliopeRuntime) -> AttrTree {
    let full = calliope_yaml::to_tree(r).unwrap_or_default();
    let mut out = AttrTree::new();
    for (k, v) in full.iter() {
        let keep = match &v.value {
            YValue::Null => false,
            YValue::Map(m) => !m.is_empty(),
            YValue::Seq(s) => !s.is_empty() || k == "math_priority",
            _ => true,
        };
        if keep {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}
