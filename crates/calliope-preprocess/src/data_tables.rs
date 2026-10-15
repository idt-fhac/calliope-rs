//! `DataTable` (`preprocess/data_tables.py`): tabular data loaded from CSV.

use std::collections::BTreeSet;
use std::path::Path;

use calliope_array::Label;
use calliope_schema::{CalliopeDataTable, CalliopeTechs, Validated};
use calliope_yaml::{AttrTree, Node, Value, relative_path};

use crate::error::ModelError;
use crate::table::{Frame, Long, read_csv};
use crate::xr::{Dataset, Obj};

/// Parameters that may not be loaded from a table (`config/data_table_exclusions.yaml`).
pub const PROTECTED_PARAMS: [(&str, &str); 2] = [
    (
        "template",
        "Template calls (`template`) can only be used in the YAML model definition.",
    ),
    (
        "templates",
        "Template definitions (`templates`) can only be used in the YAML model definition.",
    ),
];

/// Table parameters that also initialise the YAML-style tech/node definitions.
const PARAMS_TO_INITIALISE_YAML: [&str; 5] =
    ["base_tech", "link_to", "link_from", "one_way", "active"];

/// A value as a YAML node.
pub fn obj_to_node(o: &Obj) -> Node {
    Node::new(match o {
        Obj::Nan => Value::Float(f64::NAN),
        Obj::Bool(b) => Value::Bool(*b),
        Obj::Int(i) => Value::Int(*i),
        Obj::Float(f) => Value::Float(*f),
        Obj::Str(s) => Value::Str(s.to_string()),
        Obj::Datetime(_) => Value::Str(o.to_py_str()),
    })
}

/// A loaded data table.
#[derive(Debug, Clone)]
pub struct DataTable {
    /// The table name.
    pub name: String,
    /// The definition.
    pub input: CalliopeDataTable,
    /// The data: `inputs` are variables, all other dimensions coordinates.
    pub dataset: Dataset,
}

impl DataTable {
    /// Loads a table from file (CSV), relative to the model definition file.
    pub fn new(
        name: &str,
        input: &CalliopeDataTable,
        model_definition_path: Option<&Path>,
    ) -> Result<Self, ModelError> {
        let mut t = Self {
            name: name.to_owned(),
            input: input.clone(),
            dataset: Dataset::new(),
        };
        let path = Path::new(&input.table);
        let is_csv = path
            .file_name()
            .and_then(|f| f.to_str())
            .is_some_and(|f| f.split('.').skip(1).any(|s| s == "csv"));
        if !is_csv {
            return Err(t.error(&format!(
                "Only CSV data tables are supported (in-memory DataFrames are not); found `{}`",
                input.table
            )));
        }
        let columns = t.columns();
        let rows = t.rows();
        let n_header = columns.as_ref().map_or(1, Vec::len);
        let file = relative_path(model_definition_path, path);
        let frame =
            read_csv(&file, n_header, rows.as_ref().map(Vec::len)).map_err(|e| t.error(&e))?;
        t.dataset = t.frame_to_ds(frame)?;
        Ok(t)
    }

    /// Builds a table from an already-read frame (for tests).
    pub fn from_frame(
        name: &str,
        input: &CalliopeDataTable,
        frame: Frame,
    ) -> Result<Self, ModelError> {
        let mut t = Self {
            name: name.to_owned(),
            input: input.clone(),
            dataset: Dataset::new(),
        };
        t.dataset = t.frame_to_ds(frame)?;
        Ok(t)
    }

    fn columns(&self) -> Option<Vec<String>> {
        self.input
            .columns
            .as_ref()
            .map(|c| c.to_vec().iter().map(|s| s.to_string()).collect())
    }

    fn rows(&self) -> Option<Vec<String>> {
        self.input
            .rows
            .as_ref()
            .map(|c| c.to_vec().iter().map(|s| s.to_string()).collect())
    }

    /// `_raise_error`.
    pub fn error(&self, message: &str) -> ModelError {
        ModelError::new(format!("(data_tables, {}) | {message}.", self.name))
    }

    fn rename_axes(
        &self,
        loaded: &mut [Option<String>],
        axis: &str,
        names: &[String],
    ) -> Result<(), ModelError> {
        if loaded.len() != names.len() {
            return Err(self.error(&format!(
                "Expected {} {axis} levels in loaded data",
                names.len()
            )));
        }
        if let Some(mapper) = &self.input.rename_dims {
            for n in loaded.iter_mut() {
                if let Some(s) = n
                    && let Some(new) = mapper.get(s.as_str())
                {
                    *n = Some(new.to_string());
                }
            }
        }
        if loaded
            .iter()
            .enumerate()
            .any(|(i, n)| n.as_ref().is_some_and(|n| *n != names[i]))
        {
            let show = |v: &[Option<String>]| {
                let p: Vec<String> = v
                    .iter()
                    .map(|n| {
                        n.as_ref()
                            .map_or_else(|| "None".to_owned(), |s| format!("'{s}'"))
                    })
                    .collect();
                format!("[{}]", p.join(", "))
            };
            let defined: Vec<Option<String>> = names.iter().cloned().map(Some).collect();
            return Err(self.error(&format!(
                "Trying to set names for {axis} but names in the file do no match names provided | \
                 in file: {} | defined: {}",
                show(loaded),
                show(&defined)
            )));
        }
        for (n, d) in loaded.iter_mut().zip(names) {
            *n = Some(d.clone());
        }
        Ok(())
    }

    /// `_df_to_ds`.
    fn frame_to_ds(&self, mut frame: Frame) -> Result<Dataset, ModelError> {
        let columns = self.columns();
        let rows = self.rows();
        match &columns {
            None if frame.column_names.len() != 1 => {
                return Err(self.error("Expected a single columns level in loaded data"));
            }
            Some(names) => self.rename_axes(&mut frame.column_names, "columns", names)?,
            None => {}
        }
        match &rows {
            None if frame.index_names.len() != 1 => {
                return Err(self.error("Expected a single index level in loaded data"));
            }
            Some(names) => self.rename_axes(&mut frame.index_names, "index", names)?,
            None => {}
        }
        // squeeze(axis="index") with a single row drops the (unnamed) row level.
        let squeeze_index = rows.is_none() && frame.index.len() == 1;
        let mut tdf: Long = frame.to_long(columns.is_none());
        if squeeze_index && tdf.names.len() > 1 {
            tdf.names.remove(0);
            tdf.levels.remove(0);
            for c in &mut tdf.codes {
                c.remove(0);
            }
        }
        if let Some(select) = &self.input.select {
            let names = tdf.names.clone();
            for name in names.iter().flatten() {
                if let Some(items) = select.get(name.as_str()) {
                    let items: Vec<String> = items.to_vec().iter().map(|s| s.to_string()).collect();
                    tdf.select(name, &items);
                }
            }
        }
        if let Some(drop) = &self.input.drop {
            for d in drop.to_vec() {
                tdf.drop_level(&d).map_err(ModelError::new)?;
            }
        }
        if let Some(add) = &self.input.add_dims {
            for (dim, items) in add {
                let items: Vec<String> = items.to_vec().iter().map(|s| s.to_string()).collect();
                tdf.add_dim(dim, &items);
            }
        }
        self.check_processed(&tdf)?;
        self.check_protected(&tdf)?;
        let ds = if tdf.names.len() == 1 && tdf.names[0].as_deref() == Some("inputs") {
            tdf.to_scalars()
        } else {
            tdf.unstack_to_dataset("inputs")
        };
        tracing::debug!(
            "(data_tables, {}) | Loaded arrays: {:?}.",
            self.name,
            ds.names()
        );
        Ok(ds)
    }

    fn check_processed(&self, tdf: &Long) -> Result<(), ModelError> {
        if tdf.level("inputs").is_none() {
            return Err(self.error(
                "The `inputs` dimension must exist in on of `rows`, `columns`, or `add_dims`",
            ));
        }
        let dup = tdf.duplicated();
        if !dup.is_empty() {
            let items: Vec<String> = dup.iter().map(|r| tdf.row_label(*r)).collect();
            return Err(self.error(&format!(
                "Duplicate index items found: MultiIndex([{}])",
                items.join(", ")
            )));
        }
        let unique: BTreeSet<&Option<String>> = tdf.names.iter().collect();
        if unique.len() != tdf.names.len() {
            return Err(self.error(&format!(
                "Duplicate dimension names found: {}",
                tdf.names_str()
            )));
        }
        Ok(())
    }

    fn check_protected(&self, tdf: &Long) -> Result<(), ModelError> {
        let k = tdf.level("inputs").expect("checked");
        let used: BTreeSet<u32> = tdf.codes.iter().map(|c| c[k]).collect();
        let mut errors: Vec<&str> = Vec::new();
        for c in used {
            let name = tdf.levels[k][c as usize].to_string();
            if let Some((_, msg)) = PROTECTED_PARAMS.iter().find(|(p, _)| *p == name)
                && !errors.contains(msg)
            {
                errors.push(msg);
            }
        }
        if !errors.is_empty() {
            let lines: Vec<String> = errors.iter().map(|e| format!(" * {e}")).collect();
            return Err(ModelError::new(format!(
                "Errors during data table loading ({}):\n{}",
                self.name,
                lines.join("\n")
            )));
        }
        Ok(())
    }

    /// Drops a variable.
    pub fn drop(&mut self, name: &str) {
        self.dataset.remove(name);
    }

    /// `tech_def`: a dummy tech definition from the `techs` dimension, and the
    /// YAML-initialising parameters defined over `techs` only.
    pub fn tech_def(&self) -> Result<(AttrTree, AttrTree), ModelError> {
        let mut tech_dict = AttrTree::new();
        if let Some(techs) = self.dataset.coord("techs") {
            for t in techs.labels() {
                tech_dict.insert(t.to_string(), Node::empty_map());
            }
        }
        let init: Vec<String> = self
            .dataset
            .vars_with_dims(&["techs"], true)
            .into_iter()
            .filter(|v| PARAMS_TO_INITIALISE_YAML.contains(&v.as_str()))
            .collect();
        let mut base = AttrTree::new();
        if !init.is_empty() {
            let techs = self.dataset.coord("techs").expect("has techs").clone();
            for (i, t) in techs.labels().iter().enumerate() {
                let mut entry = AttrTree::new();
                for p in &init {
                    let v = self.dataset.vars[p].data.get(i);
                    if !v.is_null() {
                        entry.insert(p.clone(), obj_to_node(&v));
                    }
                }
                if !entry.is_empty() {
                    base.insert(t.to_string(), Node::from(entry));
                }
            }
        }
        Ok((tech_dict, base))
    }

    /// `node_def`: a dummy node definition from the node/tech combinations with data.
    pub fn node_def(
        &self,
        techs_incl_inheritance: &Validated<CalliopeTechs>,
    ) -> Result<AttrTree, ModelError> {
        let node_tech_vars = self.dataset.vars_with_dims(&["nodes", "techs"], false);
        if node_tech_vars.is_empty() {
            return Ok(AttrTree::new());
        }
        let nodes = self.dataset.coord("nodes").expect("has nodes").clone();
        let techs = self.dataset.coord("techs").expect("has techs").clone();
        let mut defined: Vec<(Label, Label)> = Vec::new();
        for n in nodes.labels() {
            for t in techs.labels() {
                let any = node_tech_vars.iter().any(|v| {
                    let da = self.dataset.get(v).expect("exists");
                    let sel_n = da
                        .sel("nodes", std::slice::from_ref(n))
                        .expect("label exists");
                    let sel = sel_n
                        .sel("techs", std::slice::from_ref(t))
                        .expect("label exists");
                    !sel.all_null()
                });
                if any
                    && techs_incl_inheritance
                        .0
                        .contains_key(t.to_string().as_str())
                {
                    defined.push((n.clone(), t.clone()));
                }
            }
        }
        let transmission: Vec<String> = defined
            .iter()
            .filter(|(_, t)| {
                matches!(
                    techs_incl_inheritance.0.get(t.to_string().as_str()),
                    Some(d) if d.base_tech() == Some(calliope_schema::BaseTech::Transmission)
                )
            })
            .map(|(n, t)| format!("('{n}', '{t}')"))
            .collect();
        if !transmission.is_empty() {
            return Err(self.error(&format!(
                "Cannot define transmission technology data over the `nodes` dimension. Found: [{}]",
                transmission.join(", ")
            )));
        }
        let mut init: Vec<String> = self
            .dataset
            .vars_with_dims(&["nodes", "techs"], true)
            .into_iter()
            .filter(|v| PARAMS_TO_INITIALISE_YAML.contains(&v.as_str()))
            .collect();
        init.sort();
        let mut out = AttrTree::new();
        for n in nodes.labels() {
            let mut node = AttrTree::new();
            node.insert("techs", Node::empty_map());
            out.insert(n.to_string(), Node::from(node));
        }
        for (n, t) in &defined {
            let mut entry = AttrTree::new();
            for p in &init {
                let da = self
                    .dataset
                    .get(p)
                    .expect("exists")
                    .transpose(&["nodes", "techs"]);
                let ni = nodes.code(n).expect("node") as usize;
                let ti = techs.code(t).expect("tech") as usize;
                let v = da.data.get(ni * techs.len() + ti);
                if !v.is_null() {
                    entry.insert(p.clone(), obj_to_node(&v));
                }
            }
            let node = out
                .get_mut(&n.to_string())
                .and_then(Node::as_map_mut)
                .expect("node inserted");
            let techs_map = node
                .get_mut("techs")
                .and_then(Node::as_map_mut)
                .expect("techs inserted");
            techs_map.insert(t.to_string(), Node::from(entry));
        }
        Ok(out)
    }

    /// `lookup_def_from_param`: tabular lookup data back to the YAML form
    /// (`{tech: {param: item or [items]}}`).
    pub fn lookup_def_from_param(
        &self,
        param: &str,
        lookup_dim: &str,
    ) -> Result<AttrTree, ModelError> {
        let Some(da) = self.dataset.get(param) else {
            return Ok(AttrTree::new());
        };
        if !da.has_dim("techs") || !da.has_dim(lookup_dim) {
            let dims: Vec<String> = da.dims().iter().map(|d| format!("'{d}'")).collect();
            let dims = if dims.len() == 1 {
                format!("({},)", dims[0])
            } else {
                format!("({})", dims.join(", "))
            };
            return Err(self.error(&format!(
                "Loading {param} with missing dimension(s). Must contain `techs` and `{lookup_dim}`, received: {dims}"
            )));
        }
        let techs = da.coord("techs").expect("has").clone();
        let lk = da.coord(lookup_dim).expect("has").clone();
        let others: Vec<&str> = da
            .dims()
            .into_iter()
            .filter(|d| *d != "techs" && *d != lookup_dim)
            .collect();
        let any = da.notnull().any(&others).transpose(&["techs", lookup_dim]);
        let mask = any.truthy();
        // groupby("techs") iterates techs in sorted order.
        let mut tech_order: Vec<(usize, &Label)> = techs.labels().iter().enumerate().collect();
        tech_order.sort_by(|a, b| a.1.cmp(b.1));
        let mut out = AttrTree::new();
        for (ti, t) in tech_order {
            let mut items: Vec<String> = (0..lk.len())
                .filter(|li| mask[ti * lk.len() + li])
                .map(|li| lk.labels()[li].to_string())
                .collect();
            items.sort();
            items.dedup();
            if items.is_empty() {
                continue;
            }
            let val = if items.len() == 1 {
                Node::new(Value::Str(items[0].clone()))
            } else {
                Node::new(Value::Seq(
                    items
                        .into_iter()
                        .map(|s| Node::new(Value::Str(s)))
                        .collect(),
                ))
            };
            let mut entry = AttrTree::new();
            entry.insert(param, val);
            out.insert(t.to_string(), Node::from(entry));
        }
        Ok(out)
    }
}
