//! `ModelDataBuilder`, `ModelDataCleaner` and `ModelDTypeUpdater`
//! (`preprocess/model_data.py`).

use std::collections::{BTreeSet, HashMap};

use calliope_array::{Coord, Label};
use calliope_schema::{
    BaseTech, CalliopeBuildMath, CalliopeModelDef, CalliopeRuntime, CalliopeTechs, ComponentRef,
    DataOrList, DataValue, DimensionDtype, DistanceUnit, IndexValue, IndexedData, Init,
    LookupDtype, Numeric, TechDef, Validated,
};
use calliope_yaml::{AttrTree, Node, Value};
use geographiclib_rs::{Geodesic, InverseGeodesic};
use indexmap::IndexMap;

use crate::data_tables::DataTable;
use crate::error::ModelError;
use crate::time;
use crate::xr::{Compat, DType, Data, DataArray, Dataset, Join, Obj};

/// Calliope's dtype names for math components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MathDtype {
    String,
    Float,
    Bool,
    Datetime,
    Date,
    Integer,
}

fn math_dtype(math: &CalliopeBuildMath, name: &str) -> Option<MathDtype> {
    match math.find(name, Some(&["lookups", "parameters", "dimensions"])) {
        Ok(ComponentRef::Parameter(_)) => Some(MathDtype::Float),
        Ok(ComponentRef::Lookup(l)) => Some(match l.dtype {
            LookupDtype::Float => MathDtype::Float,
            LookupDtype::String => MathDtype::String,
            LookupDtype::Bool => MathDtype::Bool,
            LookupDtype::Datetime => MathDtype::Datetime,
            LookupDtype::Date => MathDtype::Date,
        }),
        Ok(ComponentRef::Dimension(d)) => Some(match d.dtype {
            DimensionDtype::String => MathDtype::String,
            DimensionDtype::Float => MathDtype::Float,
            DimensionDtype::Integer => MathDtype::Integer,
            DimensionDtype::Datetime => MathDtype::Datetime,
            DimensionDtype::Date => MathDtype::Date,
        }),
        _ => None,
    }
}

fn update_values(data: &Data, dtype: MathDtype, init: &Init) -> Result<Data, ModelError> {
    let err = |e: String| ModelError::new(e);
    Ok(match dtype {
        MathDtype::String => Data::Object(
            data.objs()
                .into_iter()
                .map(|o| {
                    if o.is_null() {
                        Obj::Nan
                    } else {
                        Obj::str(&o.to_py_str())
                    }
                })
                .collect(),
        ),
        MathDtype::Float => data.cast(DType::Float).map_err(err)?,
        MathDtype::Integer => data.cast(DType::Int).map_err(err)?,
        MathDtype::Bool => Data::Bool(
            data.objs()
                .iter()
                .map(|o| !o.is_null() && o.truthy())
                .collect(),
        ),
        MathDtype::Datetime | MathDtype::Date => {
            let fmt = if dtype == MathDtype::Date {
                &init.date_format
            } else {
                &init.datetime_format
            };
            Data::Datetime(time::datetime_index(&data.objs(), fmt)?)
        }
    })
}

/// `_update_dtypes` on data variables.
fn update_var_dtypes(
    ds: &mut Dataset,
    math: &CalliopeBuildMath,
    init: &Init,
) -> Result<(), ModelError> {
    for name in ds.names() {
        let Some(dt) = math_dtype(math, &name) else {
            tracing::info!(
                "input data `{name}` not defined in model math; it will not be available in the optimisation problem."
            );
            continue;
        };
        let var = ds.vars.get_mut(&name).expect("exists");
        var.data = update_values(&var.data, dt, init)?;
    }
    Ok(())
}

/// `_update_dtypes` on dimension coordinates.
fn update_coord_dtypes(
    ds: &mut Dataset,
    math: &CalliopeBuildMath,
    init: &Init,
) -> Result<(), ModelError> {
    let dims: Vec<String> = ds.coords.keys().cloned().collect();
    for dim in dims {
        let Some(dt) = math_dtype(math, &dim) else {
            continue;
        };
        let c = ds.coords[&dim].clone();
        let data = Data::Object(c.labels().iter().map(Obj::from_label).collect());
        let new = update_values(&data, dt, init)?;
        let labels: Vec<Label> = (0..new.len()).map(|i| new.get(i).to_label()).collect();
        let coord = Coord::new(&dim, labels).map_err(|e| ModelError::new(e.to_string()))?;
        ds.assign_coord(coord);
    }
    Ok(())
}

fn index_label(v: &IndexValue) -> Label {
    match v {
        IndexValue::Str(s) => Label::str(s),
        IndexValue::Number(Numeric::Int(i)) => Label::Int(*i),
        IndexValue::Number(Numeric::Float(f)) => Label::Float(*f),
    }
}

fn data_obj(v: &DataValue) -> Obj {
    match v {
        DataValue::Null => Obj::Nan,
        DataValue::Bool(b) => Obj::Bool(*b),
        DataValue::Number(Numeric::Int(i)) => Obj::Int(*i),
        DataValue::Number(Numeric::Float(f)) => Obj::float(*f),
        DataValue::Str(s) => Obj::str(s),
    }
}

/// `IndexedData` in plain form.
#[derive(Debug, Clone)]
struct Indexed {
    data: Vec<Obj>,
    is_list: bool,
    dims: Vec<String>,
    index: Vec<Vec<Label>>,
}

fn node_to_obj(n: &Node) -> Option<Obj> {
    Some(match &n.value {
        Value::Null => Obj::Nan,
        Value::Bool(b) => Obj::Bool(*b),
        Value::Int(i) => Obj::Int(*i),
        Value::Float(f) => Obj::float(*f),
        Value::Str(s) => Obj::str(s),
        _ => return None,
    })
}

/// Turns a validated model's full dump into `(name, value)` pairs.
fn dump_pairs<T: serde::Serialize>(v: &T) -> AttrTree {
    calliope_yaml::to_tree(v).unwrap_or_default()
}

/// One `(nodes|techs)` entry name and its full dump.
type DefEntries = Vec<(String, AttrTree)>;

fn tech_entries(techs: &CalliopeTechs) -> DefEntries {
    techs
        .0
        .iter()
        .map(|(k, v)| (k.to_string(), dump_pairs(v)))
        .collect()
}

/// The xarray-equivalent model data builder.
#[derive(Debug)]
pub struct ModelDataBuilder<'a> {
    config: &'a Init,
    math: &'a CalliopeBuildMath,
    /// The model definition, updated with what the data tables imply.
    pub model_definition: Validated<CalliopeModelDef>,
    tech_data_from_tables: Validated<CalliopeTechs>,
    /// The dataset being built.
    pub dataset: Dataset,
    /// Warnings raised (`ModelWarning`s in Python).
    pub warnings: Vec<String>,
}

impl<'a> ModelDataBuilder<'a> {
    /// `ModelDataBuilder(init_config, model_definition, math, tables)`.
    pub fn new(
        config: &'a Init,
        model_definition: Validated<CalliopeModelDef>,
        math: &'a CalliopeBuildMath,
        tables: &mut [DataTable],
    ) -> Result<Self, ModelError> {
        let mut b = Self {
            config,
            math,
            model_definition,
            tech_data_from_tables: Validated::default_model()?,
            dataset: Dataset::new(),
            warnings: Vec::new(),
        };
        if !tables.is_empty() {
            b.init_from_data_tables(tables)?;
        }
        Ok(b)
    }

    /// `build`.
    pub fn build(&mut self) -> Result<(), ModelError> {
        self.add_node_tech_data()?;
        self.add_top_level_data_definitions()?;
        Ok(())
    }

    fn raw_techs(&self) -> AttrTree {
        self.model_definition
            .raw()
            .get("techs")
            .and_then(Node::as_map)
            .cloned()
            .unwrap_or_default()
    }

    fn raw_nodes(&self) -> AttrTree {
        self.model_definition
            .raw()
            .get("nodes")
            .and_then(Node::as_map)
            .cloned()
            .unwrap_or_default()
    }

    /// `init_from_data_tables`.
    fn init_from_data_tables(&mut self, tables: &mut [DataTable]) -> Result<(), ModelError> {
        for t in tables.iter() {
            let (tech_def, base) = t.tech_def()?;
            let mut upd = AttrTree::new();
            upd.insert("techs", Node::from(tech_def));
            self.model_definition = self.model_definition.update(&upd, false)?;
            self.tech_data_from_tables = self.tech_data_from_tables.update(&base, true)?;
        }
        for t in tables.iter_mut() {
            for (param, cfg) in &self.math.lookups {
                if let Some(dim) = &cfg.pivot_values_to_dim {
                    let lookup = t.lookup_def_from_param(param, dim)?;
                    self.tech_data_from_tables =
                        self.tech_data_from_tables.update(&lookup, true)?;
                    t.drop(param);
                }
            }
        }
        let techs_incl = self.inherit_techs(None, None)?;
        let mut node_data = AttrTree::new();
        let mut node_v: Validated<calliope_schema::CalliopeNodes> = Validated::default_model()?;
        for t in tables.iter() {
            let nd = t.node_def(&techs_incl)?;
            node_v = node_v.update(&nd, true)?;
        }
        node_data.insert("nodes", Node::from(node_v.raw().clone()));
        self.model_definition = self.model_definition.update(&node_data, false)?;

        let nodes = self.raw_nodes();
        let links = self.links_to_node_format(&nodes)?;
        let mut all_techs: BTreeSet<String> = BTreeSet::new();
        for (_, n) in nodes.iter().chain(links.iter()) {
            if let Some(t) = n
                .as_map()
                .and_then(|m| m.get("techs"))
                .and_then(Node::as_map)
            {
                all_techs.extend(t.keys().cloned());
            }
        }
        let mut node_names: Vec<String> = nodes.keys().cloned().collect();
        node_names.sort();
        self.dataset.assign_coord(
            Coord::new("nodes", node_names.iter().map(|s| Label::str(s)).collect())
                .map_err(|e| ModelError::new(e.to_string()))?,
        );
        self.dataset.assign_coord(
            Coord::new("techs", all_techs.iter().map(|s| Label::str(s)).collect())
                .map_err(|e| ModelError::new(e.to_string()))?,
        );
        for t in tables.iter() {
            let id = format!("(data_tables, {})", t.name);
            self.add_to_dataset(t.dataset.clone(), &id)?;
        }
        Ok(())
    }

    /// `_inherit_techs`: the full definition of each tech in `base_def` (default: all techs),
    /// with node-level definitions over tech definitions over table definitions.
    fn inherit_techs(
        &self,
        base_def: Option<&AttrTree>,
        at_node: Option<&str>,
    ) -> Result<Validated<CalliopeTechs>, ModelError> {
        let prefix = at_node.map_or_else(String::new, |n| format!("(nodes, {n})"));
        let techs = self.raw_techs();
        let base = base_def.cloned().unwrap_or_else(|| techs.clone());
        let tables_raw = self.tech_data_from_tables.raw();
        let mut updated: Validated<CalliopeTechs> = Validated::default_model()?;
        for (name, item) in &base {
            let Some(tech) = techs.get(name) else {
                return Err(ModelError::new(format!(
                    "{prefix}(techs, {name}) | Reference to item not defined in base techs definition."
                )));
            };
            let tech_raw = tech.as_map().cloned().unwrap_or_default();
            let item_raw = item.as_map().cloned().unwrap_or_default();
            let mut item_base =
                Validated::<TechDef>::from_tree(tech_raw)?.update(&item_raw, true)?;
            if let Some(t) = tables_raw.get(name) {
                let t = t.as_map().cloned().unwrap_or_default();
                item_base = Validated::<TechDef>::from_tree(t)?.update(item_base.raw(), true)?;
            }
            let mut upd = AttrTree::new();
            upd.insert(name.clone(), Node::from(item_base.raw().clone()));
            updated = updated.update(&upd, true)?;
        }
        Ok(Validated::from_tree(updated.raw().clone())?)
    }

    /// `_links_to_node_format`: transmission techs assigned to their `link_from`/`link_to`
    /// nodes (`{node: {techs: {link: definition}}}`).
    fn links_to_node_format(&self, _nodes: &AttrTree) -> Result<AttrTree, ModelError> {
        let active = self.inherit_techs(None, None)?;
        let mut out = AttrTree::new();
        for (name, def) in &active.0 {
            let TechDef::Transmission(t) = def else {
                continue;
            };
            let raw = active
                .raw()
                .get(name.as_str())
                .cloned()
                .unwrap_or_else(Node::empty_map);
            for node in [t.link_from.as_str(), t.link_to.as_str()] {
                if !out.contains_key(node) {
                    let mut n = AttrTree::new();
                    n.insert("techs", Node::empty_map());
                    out.insert(node, Node::from(n));
                }
                let techs = out
                    .get_mut(node)
                    .and_then(Node::as_map_mut)
                    .and_then(|m| m.get_mut("techs"))
                    .and_then(Node::as_map_mut)
                    .expect("inserted");
                techs.insert(name.to_string(), raw.clone());
            }
        }
        if out.is_empty() {
            tracing::debug!("links | No links between nodes defined.");
        }
        Ok(out)
    }

    /// `_prepare_input_data`.
    fn prepare_input_data(&self, name: &str, raw: &Node) -> Result<Indexed, ModelError> {
        if raw.is_map() {
            let d: IndexedData = calliope_yaml::from_node(raw)
                .map_err(|e| ModelError::new(format!("{name} | {e}")))?;
            let (data, is_list) = match &d.data {
                DataOrList::One(v) => (vec![data_obj(v)], false),
                DataOrList::Many(v) => (v.iter().map(data_obj).collect(), true),
            };
            if !self.config.broadcast_input_data && data.len() != d.index.len() {
                let show_data = if is_list {
                    format!(
                        "[{}]",
                        data.iter()
                            .map(Obj::to_py_str)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                } else {
                    data[0].to_py_str()
                };
                return Err(ModelError::new(format!(
                    "{name} | Length mismatch between data ({show_data}) and index ({}) in input definition. \
                     Check lengths of arrays or set `config.broadcast_input_data` to True to allow single \
                     data entries to be broadcast across all parameter index items.",
                    show_index(&d.index)
                )));
            }
            return Ok(Indexed {
                data,
                is_list,
                dims: d.dims.iter().map(|s| s.to_string()).collect(),
                index: d
                    .index
                    .iter()
                    .map(|i| i.iter().map(index_label).collect())
                    .collect(),
            });
        }
        if let Some(l) = self.math.lookups.get(name)
            && let Some(dim) = &l.pivot_values_to_dim
            && !raw.is_null()
        {
            let items: Vec<Node> = match &raw.value {
                Value::Seq(s) => s.clone(),
                _ => vec![raw.clone()],
            };
            let index = items
                .iter()
                .map(|n| {
                    vec![match &n.value {
                        Value::Int(i) => Label::Int(*i),
                        Value::Float(f) => Label::Float(*f),
                        other => Label::str(&match other {
                            Value::Str(s) => s.clone(),
                            o => o.to_string(),
                        }),
                    }]
                })
                .collect();
            return Ok(Indexed {
                data: vec![Obj::Bool(true)],
                is_list: false,
                dims: vec![dim.clone()],
                index,
            });
        }
        if raw.as_seq().is_some() {
            return Err(ModelError::new(format!(
                "{name} | Cannot pass un-indexed list input data. Received: {}.",
                raw.value
            )));
        }
        let v =
            node_to_obj(raw).ok_or_else(|| ModelError::new(format!("{name} | invalid data")))?;
        Ok(Indexed {
            data: vec![v],
            is_list: false,
            dims: Vec::new(),
            index: vec![Vec::new()],
        })
    }

    /// `_input_data_to_array`: `pd.Series(data, MultiIndex(index)).to_xarray()`.
    fn input_data_to_array(name: &str, d: &Indexed) -> Result<DataArray, ModelError> {
        if d.dims.is_empty() {
            let v = d.data.first().cloned().unwrap_or(Obj::Nan);
            let data = match v {
                Obj::Nan => Data::Object(vec![Obj::Nan]),
                v => Data::infer(vec![v]),
            };
            return Ok(DataArray::new(Vec::new(), data));
        }
        let n = d.index.len();
        let values: Vec<Obj> = if d.is_list {
            if d.data.len() != n {
                return Err(ModelError::new(format!(
                    "{name} | Length of values ({}) does not match length of index ({n})",
                    d.data.len()
                )));
            }
            d.data.clone()
        } else {
            vec![d.data[0].clone(); n]
        };
        let series = Data::from_objs_infer_series(values);
        // MultiIndex.from_tuples: sorted levels.
        let mut coords = Vec::new();
        for (k, dim) in d.dims.iter().enumerate() {
            let mut labels: Vec<Label> = Vec::new();
            for row in &d.index {
                let l = row.get(k).cloned().ok_or_else(|| {
                    ModelError::new(format!(
                        "{name} | index items must match the number of dims"
                    ))
                })?;
                if !labels.contains(&l) {
                    labels.push(l);
                }
            }
            if crate::xr::comparable(&labels) {
                labels.sort();
            }
            coords.push(Coord::new(dim, labels).map_err(|e| ModelError::new(e.to_string()))?);
        }
        let shape: Vec<usize> = coords.iter().map(Coord::len).collect();
        let total: usize = shape.iter().product();
        let mut flat_of_row = Vec::with_capacity(n);
        let mut seen = std::collections::HashSet::new();
        for row in &d.index {
            let mut flat = 0usize;
            for (k, c) in coords.iter().enumerate() {
                flat = flat * shape[k] + c.code(&row[k]).expect("label in level") as usize;
            }
            if !seen.insert(flat) {
                return Err(ModelError::new(
                    "cannot convert a DataFrame with a non-unique MultiIndex into xarray",
                ));
            }
            flat_of_row.push(flat);
        }
        let mut idx: Vec<Option<usize>> = vec![None; total];
        for (r, f) in flat_of_row.iter().enumerate() {
            idx[*f] = Some(r);
        }
        let fill = series.dtype().maybe_promote();
        let data = series.gather(&idx, fill);
        Ok(DataArray::new(coords, data))
    }

    /// `_definition_to_ds`.
    fn definition_to_ds(
        &self,
        entries: &DefEntries,
        dim: &str,
        exclude: &[&str],
    ) -> Result<Dataset, ModelError> {
        let mut ds = Dataset::new();
        for (idx_name, dump) in entries {
            let mut piece = Dataset::new();
            let mut arrays: Vec<(String, DataArray)> = Vec::new();
            for (name, val) in dump {
                if exclude.contains(&name.as_str()) {
                    continue;
                }
                let mut d = self.prepare_input_data(name, val)?;
                d.index = d
                    .index
                    .into_iter()
                    .map(|i| {
                        let mut v = vec![Label::str(idx_name)];
                        v.extend(i);
                        v
                    })
                    .collect();
                d.dims.insert(0, dim.to_owned());
                arrays.push((name.clone(), Self::input_data_to_array(name, &d)?));
            }
            // combine_by_coords(join="outer") of differently named arrays: a merge.
            let singles: Vec<Dataset> = arrays
                .into_iter()
                .map(|(n, a)| {
                    let mut s = Dataset::new();
                    s.set(&n, a);
                    s
                })
                .collect();
            if !singles.is_empty() {
                let refs: Vec<&Dataset> = singles.iter().collect();
                piece = Dataset::merge(&refs, Join::Outer, Compat::NoConflicts)
                    .map_err(ModelError::new)?;
            }
            ds = Dataset::merge(&[&ds, &piece], Join::Outer, Compat::NoConflicts)
                .map_err(ModelError::new)?;
        }
        Ok(ds)
    }

    /// `add_node_tech_data`.
    fn add_node_tech_data(&mut self) -> Result<(), ModelError> {
        let nodes = self.raw_nodes();
        let nodes_v: Validated<calliope_schema::CalliopeNodes> =
            Validated::from_tree(nodes.clone())?;
        let links = self.links_to_node_format(&nodes)?;
        let mut pieces: Vec<Dataset> = Vec::new();
        let mut node_names: Vec<String> = Vec::new();
        for (node_name, node_raw) in &nodes {
            let techs_raw: AttrTree = node_raw
                .as_map()
                .and_then(|m| m.get("techs"))
                .and_then(Node::as_map)
                .cloned()
                .unwrap_or_default();
            let mut refs: BTreeSet<String> = BTreeSet::new();
            for (tech, def) in &techs_raw {
                let def = def.as_map().cloned().unwrap_or_default();
                if def.contains_key("base_tech") {
                    return Err(ModelError::new(format!(
                        "(nodes, {node_name}), (techs, {tech}) | Defining a technology `base_tech` at a node is not supported; \
                         limit yourself to defining this lookup within `techs` or `templates`"
                    )));
                }
                refs.extend(def.keys().cloned());
            }
            let mut techs_incl = self.inherit_techs(Some(&techs_raw), Some(node_name))?;
            let transmission: Vec<String> = techs_incl
                .0
                .iter()
                .filter(|(_, d)| d.base_tech() == Some(BaseTech::Transmission))
                .map(|(k, _)| format!("'{k}'"))
                .collect();
            if !transmission.is_empty() {
                return Err(ModelError::new(format!(
                    "(nodes, {node_name}) | Transmission techs cannot be directly defined at nodes; \
                     they will be automatically assigned to nodes based on `link_to` and `link_from` for: [{}].",
                    transmission.join(", ")
                )));
            }
            if let Some(l) = links
                .get(node_name)
                .and_then(Node::as_map)
                .and_then(|m| m.get("techs"))
                .and_then(Node::as_map)
            {
                techs_incl = techs_incl.update(l, true)?;
            }
            let tech_ds = self.definition_to_ds(&tech_entries(&techs_incl), "techs", &[])?;
            let mut piece = tech_ds;
            let node_coord = Coord::new("nodes", vec![Label::str(node_name)]).expect("one label");
            let expand: Vec<String> = piece
                .names()
                .into_iter()
                .filter(|v| refs.contains(v) || v == "carrier_in" || v == "carrier_out")
                .collect();
            for v in expand {
                let da = piece
                    .get(&v)
                    .expect("exists")
                    .expand_dims(node_coord.clone());
                piece.set(&v, da);
            }
            piece.coords.insert("nodes".to_owned(), node_coord);
            pieces.push(piece);
            node_names.push(node_name.clone());
        }
        let mut node_tech_ds = concat_nodes(&pieces, &node_names)?;
        let node_entries: DefEntries = nodes_v
            .0
            .iter()
            .map(|(k, v)| (k.to_string(), dump_pairs(v)))
            .collect();
        let mut node_ds = self.definition_to_ds(&node_entries, "nodes", &["techs"])?;
        if let (Some(a), Some(b)) = (node_tech_ds.get("active"), node_ds.get("active")) {
            let prod = a.zip_with(&b, mul, mul_dtype(a.dtype(), b.dtype()));
            node_tech_ds.set("active", prod);
        }
        node_ds.remove("active");
        let ds = Dataset::merge(&[&node_tech_ds, &node_ds], Join::Outer, Compat::NoConflicts)
            .map_err(ModelError::new)?;
        self.add_to_dataset(ds, "YAML definition")
    }

    /// `add_top_level_data_definitions`.
    fn add_top_level_data_definitions(&mut self) -> Result<(), ModelError> {
        let defs = self
            .model_definition
            .raw()
            .get("data_definitions")
            .and_then(Node::as_map)
            .cloned()
            .unwrap_or_default();
        for (name, raw) in &defs {
            if self.dataset.contains(name) {
                let msg = format!(
                    "(Model inputs, {name}) | Model input data with this name has already been defined in a data table or at a node/tech level. Non-NaN data defined here will override existing data for it."
                );
                tracing::warn!("{msg}");
                self.warnings.push(msg);
            }
            let d = self.prepare_input_data(name, raw)?;
            let da = Self::input_data_to_array(name, &d)?;
            if da.has_dim("techs") && da.has_dim("nodes") {
                let msg = format!(
                    "(Model inputs, {name}) | This input data will only take effect if you have already defined the following combinations of techs at nodes in your model definition: ..."
                );
                tracing::warn!("{msg}");
                self.warnings.push(msg);
            }
            let mut ds = Dataset::new();
            ds.set(name, da);
            self.add_to_dataset(ds, &format!("(Model inputs, {name})"))?;
        }
        Ok(())
    }

    /// `_add_to_dataset`: coordinate dtypes per the math, then new data over old.
    fn add_to_dataset(&mut self, mut to_add: Dataset, id: &str) -> Result<(), ModelError> {
        tracing::debug!("{id} | adding to dataset");
        update_coord_dtypes(&mut to_add, self.math, self.config)?;
        let merged = Dataset::merge(&[&to_add, &self.dataset], Join::Outer, Compat::Override)
            .map_err(ModelError::new)?;
        self.dataset = merged.fillna(&self.dataset);
        Ok(())
    }
}

fn show_index(index: &[calliope_schema::UniqueList<IndexValue>]) -> String {
    let rows: Vec<String> = index
        .iter()
        .map(|r| {
            let items: Vec<String> = r
                .iter()
                .map(|v| match v {
                    IndexValue::Str(s) => format!("'{s}'"),
                    IndexValue::Number(n) => n.to_string(),
                })
                .collect();
            format!("[{}]", items.join(", "))
        })
        .collect();
    format!("[{}]", rows.join(", "))
}

fn mul(a: Obj, b: Obj) -> Obj {
    match (a, b) {
        (Obj::Bool(x), Obj::Bool(y)) => Obj::Bool(x && y),
        (x, y) => match (x.as_f64(), y.as_f64()) {
            (Some(p), Some(q)) => Obj::float(p * q),
            _ => Obj::Nan,
        },
    }
}

fn mul_dtype(a: DType, b: DType) -> DType {
    match (a, b) {
        (DType::Bool, DType::Bool) => DType::Bool,
        (DType::Object, _) | (_, DType::Object) => DType::Object,
        _ => DType::Float,
    }
}

/// `xr.combine_nested(pieces, concat_dim="nodes", data_vars=["active"], coords="minimal",
/// join="outer", compat="no_conflicts")`.
fn concat_nodes(pieces: &[Dataset], node_names: &[String]) -> Result<Dataset, ModelError> {
    let nodes = Coord::new("nodes", node_names.iter().map(|s| Label::str(s)).collect())
        .map_err(|e| ModelError::new(e.to_string()))?;
    // Outer-join all other dims.
    let mut coords: IndexMap<String, Coord> = IndexMap::new();
    for p in pieces {
        for (name, c) in &p.coords {
            if name == "nodes" {
                continue;
            }
            let j = match coords.get(name) {
                Some(prev) => crate::xr::join_coords(prev, c, Join::Outer),
                None => c.clone(),
            };
            coords.insert(name.clone(), j);
        }
    }
    let mut order: Vec<String> = Vec::new();
    for p in pieces {
        for n in p.vars.keys() {
            if !order.contains(n) {
                order.push(n.clone());
            }
        }
    }
    let concat_over: BTreeSet<String> = order
        .iter()
        .filter(|n| {
            *n == "active"
                || pieces.iter().any(|p| {
                    p.vars
                        .get(*n)
                        .is_some_and(|v| v.dims.iter().any(|d| d == "nodes"))
                })
        })
        .cloned()
        .collect();
    let mut out = Dataset::new();
    out.coords.insert("nodes".to_owned(), nodes.clone());
    for (k, c) in &coords {
        out.coords.insert(k.clone(), c.clone());
    }
    for name in &order {
        if concat_over.contains(name) {
            // common dims: ordered union of the pieces' dims, `nodes` first if absent.
            let mut dims: Vec<String> = Vec::new();
            for p in pieces {
                if let Some(v) = p.vars.get(name) {
                    for d in &v.dims {
                        if !dims.contains(d) {
                            dims.push(d.clone());
                        }
                    }
                }
            }
            if !dims.iter().any(|d| d == "nodes") {
                dims.insert(0, "nodes".to_owned());
            }
            let full: Vec<Coord> = dims
                .iter()
                .map(|d| {
                    if d == "nodes" {
                        nodes.clone()
                    } else {
                        coords[d].clone()
                    }
                })
                .collect();
            // Fill each node slice.
            let mut acc = DataArray::new(
                full.clone(),
                Data::missing(DType::Float, full.iter().map(Coord::len).product()),
            );
            let mut dtype: Option<DType> = None;
            let mut any_missing_piece = false;
            for (i, p) in pieces.iter().enumerate() {
                let one = Coord::new("nodes", vec![Label::str(&node_names[i])]).expect("one");
                let slice_coords: Vec<Coord> = full
                    .iter()
                    .map(|c| {
                        if c.name() == "nodes" {
                            one.clone()
                        } else {
                            c.clone()
                        }
                    })
                    .collect();
                let Some(mut da) = p.get(name) else {
                    any_missing_piece = true;
                    continue;
                };
                if !da.has_dim("nodes") {
                    da = da.expand_dims(one.clone());
                }
                let da = da.conform(&slice_coords, DType::Float);
                dtype = Some(match dtype {
                    None => da.dtype(),
                    Some(d) => concat_dtype(d, da.dtype()),
                });
                acc = acc.fillna(&da.conform(&full, DType::Float));
            }
            let mut dt = dtype.unwrap_or(DType::Float);
            if any_missing_piece {
                dt = dt.with_missing();
            }
            let data = if acc.dtype() == dt {
                acc.data
            } else {
                acc.data.cast(dt).unwrap_or(acc.data)
            };
            out.set(name, DataArray::new(full, data));
        } else {
            let mut acc: Option<DataArray> = None;
            for p in pieces {
                let Some(da) = p.get(name) else { continue };
                let dst: Vec<Coord> = da.coords.iter().map(|c| coords[c.name()].clone()).collect();
                let da = da.conform(&dst, DType::Float);
                acc = Some(match acc {
                    None => da,
                    Some(prev) => prev.fillna(&da),
                });
            }
            if let Some(a) = acc {
                out.set(name, a);
            }
        }
    }
    Ok(out)
}

fn concat_dtype(a: DType, b: DType) -> DType {
    if a == b {
        return a;
    }
    a.result_type(b)
}

/// `ModelDataCleaner`.
#[derive(Debug)]
pub struct ModelDataCleaner<'a> {
    config: &'a Init,
    math: &'a CalliopeBuildMath,
    /// The dataset being cleaned.
    pub dataset: Dataset,
    /// Runtime attributes, updated by cleaning.
    pub runtime: CalliopeRuntime,
    /// Warnings raised.
    pub warnings: Vec<String>,
}

const DEFAULT_PALETTE: [&str; 10] = [
    "#19122b", "#17344c", "#185b48", "#3c7632", "#7e7a36", "#bc7967", "#d486af", "#caa9e7",
    "#c2d2f3", "#d6f0ef",
];

impl<'a> ModelDataCleaner<'a> {
    /// `ModelDataCleaner(init_config, dataset, math, runtime)`.
    pub fn new(
        config: &'a Init,
        dataset: Dataset,
        math: &'a CalliopeBuildMath,
        runtime: CalliopeRuntime,
    ) -> Self {
        Self {
            config,
            math,
            dataset,
            runtime,
            warnings: Vec::new(),
        }
    }

    /// `clean`.
    pub fn clean(&mut self) -> Result<(), ModelError> {
        update_var_dtypes(&mut self.dataset, self.math, self.config)?;
        if !self.config.retain_inactive {
            self.clean_data_from_undefined_members()?;
        }
        self.add_colors();
        self.add_link_distances();
        self.update_and_resample_dimensions()?;
        update_coord_dtypes(&mut self.dataset, self.math, self.config)?;
        update_var_dtypes(&mut self.dataset, self.math, self.config)?;
        self.runtime.instantiated = true;
        Ok(())
    }

    fn get(&self, name: &str) -> Result<DataArray, ModelError> {
        self.dataset
            .get(name)
            .ok_or_else(|| ModelError::new(format!("'Dataset' object has no attribute '{name}'")))
    }

    fn clean_data_from_undefined_members(&mut self) -> Result<(), ModelError> {
        let ci = self.get("carrier_in")?;
        let co = self.get("carrier_out")?;
        let active = self.get("active")?;
        let or = ci.zip_with(
            &co,
            |a, b| Obj::Bool(a.truthy() && !a.is_null() || b.truthy() && !b.is_null()),
            DType::Bool,
        );
        let def = or.zip_with(
            &active,
            |a, b| Obj::Bool(a.truthy() && b.truthy()),
            DType::Bool,
        );
        let def_dims: Vec<String> = def.dims().iter().map(|s| (*s).to_owned()).collect();
        for name in self.dataset.names() {
            let var = self.dataset.get(&name).expect("exists");
            let non_dims: Vec<&str> = def_dims
                .iter()
                .filter(|d| !var.has_dim(d))
                .map(String::as_str)
                .collect();
            let cond = def.any(&non_dims);
            let updated = var.where_(&cond);
            let updated = if var.dtype() == DType::Bool {
                let objs = updated.data.objs();
                DataArray::new(
                    updated.coords.clone(),
                    Data::Bool(objs.iter().map(|o| !o.is_null() && o.truthy()).collect()),
                )
            } else {
                updated
            };
            self.dataset.set(&name, updated);
        }
        self.dataset = drop_undefined(self.dataset.clone(), &def);
        Ok(())
    }

    fn add_colors(&mut self) {
        let Some(techs) = self.dataset.coord("techs").cloned() else {
            return;
        };
        let palette: Vec<Obj> = (0..techs.len())
            .map(|i| Obj::str(DEFAULT_PALETTE[i % DEFAULT_PALETTE.len()]))
            .collect();
        let new = DataArray::new(vec![techs], Data::Object(palette));
        match self.dataset.get("color") {
            None => self.dataset.set("color", new),
            Some(c) if (0..c.data.len()).any(|i| c.data.is_null(i)) => {
                self.dataset.set("color", c.fillna(&new));
            }
            Some(_) => {}
        }
    }

    fn add_link_distances(&mut self) {
        let (Some(lat), Some(lon), Some(base)) = (
            self.dataset.get("latitude"),
            self.dataset.get("longitude"),
            self.dataset.get("base_tech"),
        ) else {
            tracing::debug!(
                "Link distances will not be computed automatically since lat/lon coordinates are not defined."
            );
            return;
        };
        let techs = base.coords[0].clone();
        let is_trans: Vec<bool> = (0..base.data.len())
            .map(|i| base.data.get(i).as_str() == Some("transmission"))
            .collect();
        if !is_trans.iter().any(|b| *b) {
            return;
        }
        let (Some(from), Some(to)) = (self.dataset.get("link_from"), self.dataset.get("link_to"))
        else {
            return;
        };
        let latlon = |node: &Obj| -> (f64, f64) {
            let l = Label::str(&node.to_py_str());
            let la = lat
                .sel("nodes", std::slice::from_ref(&l))
                .map(|a| a.item().as_f64().unwrap_or(f64::NAN));
            let lo = lon
                .sel("nodes", std::slice::from_ref(&l))
                .map(|a| a.item().as_f64().unwrap_or(f64::NAN));
            (la.unwrap_or(f64::NAN), lo.unwrap_or(f64::NAN))
        };
        let geod = Geodesic::wgs84();
        let mut cache: HashMap<(String, String), f64> = HashMap::new();
        let mut labels = Vec::new();
        let mut values = Vec::new();
        for (i, t) in techs.labels().iter().enumerate() {
            if !is_trans[i] {
                continue;
            }
            let (f, to_) = (from.data.get(i), to.data.get(i));
            let key = (f.to_py_str(), to_.to_py_str());
            let d = *cache.entry(key).or_insert_with(|| {
                let (la1, lo1) = latlon(&f);
                let (la2, lo2) = latlon(&to_);
                geod.inverse(la1, lo1, la2, lo2)
            });
            labels.push(t.clone());
            values.push(if self.config.distance_unit == DistanceUnit::Km {
                d / 1000.0
            } else {
                d
            });
        }
        let arr = DataArray::new(
            vec![Coord::new("techs", labels).expect("unique techs")],
            Data::Float(values),
        );
        match self.dataset.get("distance") {
            None => self.dataset.set("distance", arr),
            Some(d) => self.dataset.set("distance", d.fillna(&arr)),
        }
    }

    fn update_and_resample_dimensions(&mut self) -> Result<(), ModelError> {
        if !self.dataset.coords.values().any(Coord::is_datetime) {
            return Err(ModelError::new(
                "Must define at least one timeseries data input in a Calliope model.",
            ));
        }
        if self.config.subset != self.runtime.subset {
            self.subset_dims()?;
            self.runtime.subset = self.config.subset.clone();
        }
        if self.config.resample != self.runtime.resample {
            self.resample_dims()?;
            self.runtime.resample = self.config.resample.clone();
        }
        if !self.runtime.instantiated {
            self.warnings
                .extend(time::add_inferred_time_params(&mut self.dataset));
        }
        match (&self.runtime.time_cluster, &self.config.time_cluster) {
            (None, Some(tc)) => {
                self.dataset = time::cluster(&self.dataset, tc, &self.config.datetime_format)?;
                self.runtime.time_cluster = Some(tc.clone());
            }
            (a, b) if a != b => {
                return Err(ModelError::new(
                    "Cannot change time clustering configuration at this stage.",
                ));
            }
            _ => {}
        }
        Ok(())
    }

    fn subset_dims(&mut self) -> Result<(), ModelError> {
        let mut ds = self.dataset.clone();
        for (dim, subset) in &self.config.subset {
            let Some(subset) = subset else { continue };
            let Some(coord) = ds.coord(dim).cloned() else {
                tracing::debug!("Skipping subsetting for undefined dimension: {dim}");
                continue;
            };
            let ordered = self
                .math
                .dimensions
                .get(dim.as_str())
                .ok_or_else(|| ModelError::new(format!("'{dim}'")))?
                .ordered;
            let items: Vec<Label> = subset.iter().map(index_label).collect();
            let strs: Vec<String> = items.iter().map(ToString::to_string).collect();
            let keep: Vec<Label> = if coord.is_datetime() {
                let times: Vec<i64> = coord
                    .labels()
                    .iter()
                    .map(|l| if let Label::Datetime(d) = l { *d } else { 0 })
                    .collect();
                time::check_time_subset(&times, &strs)?;
                if ordered {
                    let lo = time::partial_bounds(&strs[0]).map_or(i64::MIN, |b| b.0);
                    let hi = time::partial_bounds(&strs[1]).map_or(i64::MAX, |b| b.1);
                    coord
                        .labels()
                        .iter()
                        .filter(|l| matches!(l, Label::Datetime(d) if *d >= lo && *d <= hi))
                        .cloned()
                        .collect()
                } else {
                    let mut out = Vec::new();
                    for s in &strs {
                        let (lo, hi) = time::partial_bounds(s)
                            .ok_or_else(|| ModelError::new(format!("'{s}'")))?;
                        out.extend(
                            coord
                                .labels()
                                .iter()
                                .filter(|l| matches!(l, Label::Datetime(d) if *d >= lo && *d <= hi))
                                .cloned(),
                        );
                    }
                    out
                }
            } else if ordered {
                if items.len() != 2 {
                    return Err(ModelError::new(format!(
                        "Subset of ordered dimension `{dim}` must be a slice of two items."
                    )));
                }
                let pos = |l: &Label| coord.labels().iter().position(|x| x == l);
                let (a, b) = (pos(&items[0]), pos(&items[1]));
                match (a, b) {
                    (Some(a), Some(b)) => {
                        coord.labels()[a..=b.max(a).min(coord.len() - 1)].to_vec()
                    }
                    _ => coord
                        .labels()
                        .iter()
                        .filter(|l| **l >= items[0] && **l <= items[1])
                        .cloned()
                        .collect(),
                }
            } else {
                for l in &items {
                    if coord.code(l).is_none() {
                        return Err(ModelError::new(format!(
                            "\"not all values found in index '{dim}'\""
                        )));
                    }
                }
                items
            };
            ds.isel_labels(dim, keep);
        }
        // Drop links that are now hanging (only connected to one node).
        if let (Some(active), Some(base)) = (ds.get("active"), ds.get("base_tech")) {
            let techs = base.coords[0].clone();
            let trans: Vec<Label> = techs
                .labels()
                .iter()
                .enumerate()
                .filter(|(i, _)| base.data.get(*i).as_str() == Some("transmission"))
                .map(|(_, l)| l.clone())
                .collect();
            let sums = active
                .sel("techs", &trans)
                .map_err(ModelError::new)?
                .sum(&["nodes"], 0);
            let st = sums.coord("techs").expect("techs").clone();
            let hanging: Vec<&Label> = st
                .labels()
                .iter()
                .enumerate()
                .filter(|(i, _)| sums.data.get(*i).as_f64() == Some(1.0))
                .map(|(_, l)| l)
                .collect();
            if !hanging.is_empty() {
                let keep: Vec<Label> = ds
                    .coord("techs")
                    .expect("techs")
                    .labels()
                    .iter()
                    .filter(|l| !hanging.contains(l))
                    .cloned()
                    .collect();
                ds.isel_labels("techs", keep);
            }
        }
        if self.config.retain_inactive {
            self.dataset = ds;
        } else {
            let active = ds
                .get("active")
                .ok_or_else(|| ModelError::new("no `active`"))?;
            self.dataset = drop_undefined(ds, &active);
        }
        Ok(())
    }

    fn resample_dims(&mut self) -> Result<(), ModelError> {
        let mut ds = self.dataset.clone();
        for (dim, res) in &self.config.resample {
            let Some(res) = res else { continue };
            let Some(c) = ds.coord(dim) else {
                tracing::debug!("Skipping resampling for undefined dimension: {dim}");
                continue;
            };
            if !c.is_datetime() {
                return Err(ModelError::new(format!(
                    "Cannot resample a non-datetime dimension, received `{dim}`"
                )));
            }
            ds = time::resample(&ds, self.math, dim, res)?;
        }
        self.dataset = ds;
        Ok(())
    }
}

/// `_drop_undefined`: drops labels where `def_matrix` is false everywhere and variables that
/// are then all missing.
pub fn drop_undefined(mut ds: Dataset, def_matrix: &DataArray) -> Dataset {
    let active = def_matrix.where_(def_matrix);
    ds.set("__active", active);
    for dim in def_matrix.dims() {
        let before: Vec<Label> = ds
            .coord(dim)
            .map(|c| c.labels().to_vec())
            .unwrap_or_default();
        ds.dropna(dim, "__active");
        let after: Vec<Label> = ds
            .coord(dim)
            .map(|c| c.labels().to_vec())
            .unwrap_or_default();
        if before.len() != after.len() {
            tracing::debug!("Deleting {dim} values as they are not defined anywhere in the model");
        }
    }
    let empty: Vec<String> = ds
        .vars
        .iter()
        .filter(|(_, v)| (0..v.data.len()).all(|i| v.data.is_null(i)))
        .map(|(k, _)| k.clone())
        .collect();
    for k in empty {
        ds.remove(&k);
    }
    ds.remove("__active");
    ds
}
