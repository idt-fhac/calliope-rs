//! Validated models with pydantic's "explicitly set" tracking and Calliope's `update`
//! semantics (`CalliopeBaseModel.update`, `CalliopeDictModel.update`,
//! `CalliopeListModel.update` in `schemas/general.py`).
//!
//! A [`Validated<T>`] keeps the typed value and the tree of explicitly set values, which is
//! pydantic's `model_dump(exclude_unset=True)`. Python's `model_fields_set` is the set of
//! top-level keys of that tree. Updates are tree merges guided by a [`Shape`], followed by
//! re-validation, which is what the Python implementation does too.

use calliope_yaml::{AttrTree, Node, Value, to_tree};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::error::{FieldError, SchemaError};

/// How a field (or dictionary entry) takes part in an update.
#[derive(Debug, Clone, Copy)]
pub enum Shape {
    /// A plain value: replaced by the update.
    Leaf,
    /// A `CalliopeListModel`: replaced as a whole; skipped when `overwrite=False`.
    List,
    /// A `CalliopeBaseModel`: updated field by field.
    Model(&'static ModelShape),
    /// A `CalliopeDictModel`: updated entry by entry.
    Dict(&'static DictShape),
    /// A union of a model and plain values (tech parameters): updated as the model if the
    /// current value is a mapping, otherwise replaced.
    MapOr(&'static ModelShape),
}

/// The update shape of a `CalliopeBaseModel`.
#[derive(Debug)]
pub struct ModelShape {
    /// The Python class name, used in log messages.
    pub name: &'static str,
    /// Fields that are not plain values.
    pub fields: &'static [(&'static str, Shape)],
    /// The shape of extra (undeclared) keys, for models with `extra="allow"`.
    pub extra: Shape,
}

/// The update shape of a `CalliopeDictModel`.
#[derive(Debug)]
pub struct DictShape {
    /// The Python class name, used in log messages.
    pub name: &'static str,
    /// The shape of the entries. Only `Model`, `MapOr` and `List` entries are updated in
    /// place; anything else is replaced.
    pub entry: Shape,
}

impl ModelShape {
    fn field(&self, key: &str) -> Shape {
        self.fields
            .iter()
            .find(|(k, _)| *k == key)
            .map_or(self.extra, |(_, s)| *s)
    }
}

/// A schema type: deserialisable, serialisable, with a title for error messages, an update
/// shape and model-level validators.
pub trait Schema: DeserializeOwned + Serialize {
    /// The pydantic title (`model_config["title"]`, else the class name).
    const TITLE: &'static str;

    /// How updates are applied.
    fn shape() -> Shape;

    /// Model-level validators (pydantic `model_validator(mode="after")`), run after
    /// deserialisation. `path` is the dotted path of `self`.
    fn validate(&self, _path: &str, _errors: &mut Vec<FieldError>) {}
}

/// A validated model plus the tree of values that were explicitly set.
#[derive(Debug, Clone)]
pub struct Validated<T> {
    raw: AttrTree,
    value: T,
}

impl<T: PartialEq> PartialEq for Validated<T> {
    fn eq(&self, other: &Self) -> bool {
        self.value == other.value
    }
}

impl<T> std::ops::Deref for Validated<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

/// Collected `LOGGER.debug` messages of an update, in Python's wording.
pub type UpdateLog = Vec<String>;

impl<T: Schema> Validated<T> {
    /// `T.model_validate(tree)`.
    pub fn from_tree(tree: AttrTree) -> Result<Self, SchemaError> {
        let node = Node::from(tree);
        let value: T = calliope_yaml::from_node(&node)
            .map_err(|e| SchemaError::from_path_error(T::TITLE, e))?;
        let mut errors = Vec::new();
        value.validate("", &mut errors);
        if !errors.is_empty() {
            for e in &mut errors {
                if e.span.is_none() {
                    e.span = calliope_yaml::span_at(&node, &e.path);
                }
            }
            return Err(SchemaError {
                title: T::TITLE,
                errors,
            });
        }
        let Value::Map(raw) = node.value else {
            unreachable!("built from a tree")
        };
        Ok(Self { raw, value })
    }

    /// `T()`: the model with all defaults and nothing set.
    pub fn default_model() -> Result<Self, SchemaError> {
        Self::from_tree(AttrTree::new())
    }

    /// The typed value.
    pub fn get(&self) -> &T {
        &self.value
    }

    /// Consumes the wrapper, returning the typed value.
    pub fn into_inner(self) -> T {
        self.value
    }

    /// `model_dump(exclude_unset=True)`: the explicitly set values.
    pub fn raw(&self) -> &AttrTree {
        &self.raw
    }

    /// `model_fields_set`: the explicitly set top-level keys.
    pub fn fields_set(&self) -> impl Iterator<Item = &str> {
        self.raw.keys().map(String::as_str)
    }

    /// True if the top-level field was explicitly set.
    pub fn is_set(&self, key: &str) -> bool {
        self.raw.contains_key(key)
    }

    /// `model_dump()`: all values, including defaults.
    pub fn dump(&self) -> AttrTree {
        to_tree(&self.value).expect("schema values serialise to mappings")
    }

    /// `model_dump(exclude_unset=True)` with validated values: the keys that were set, with
    /// the values the schema made of them (e.g. `dims: costs` becomes `dims: [costs]`).
    pub fn dump_set(&self) -> AttrTree {
        intersect(&self.dump(), &self.raw)
    }

    /// `update(update_def, overwrite=...)`: a new model with the update applied.
    ///
    /// The update may use dotted keys. With `overwrite=false`, explicitly set values are
    /// kept and only unset ones are filled from the update.
    pub fn update(&self, update: &AttrTree, overwrite: bool) -> Result<Self, SchemaError> {
        self.update_logged(update, overwrite, &mut Vec::new())
    }

    /// [`Validated::update`], collecting the debug messages Python would log.
    pub fn update_logged(
        &self,
        update: &AttrTree,
        overwrite: bool,
        log: &mut UpdateLog,
    ) -> Result<Self, SchemaError> {
        let current = self.dump();
        let raw = match T::shape() {
            Shape::Model(shape) => update_model(
                shape,
                &self.raw,
                Some(&current),
                update,
                overwrite,
                &mut Ctx {
                    log,
                    suppress: false,
                },
            ),
            Shape::Dict(shape) => update_dict(
                shape,
                &self.raw,
                Some(&current),
                update,
                overwrite,
                &mut Ctx {
                    log,
                    suppress: false,
                },
            ),
            Shape::Leaf | Shape::List | Shape::MapOr(_) => {
                let mut t = self.raw.clone();
                t.union(update, true, false)
                    .map_err(|e| SchemaError::single(T::TITLE, "", e.to_string()))?;
                Ok(t)
            }
        }
        .map_err(|msg| SchemaError::single(T::TITLE, "", msg))?;
        Self::from_tree(raw)
    }

    /// `update` with another model: its explicitly set values are the update.
    pub fn update_with(&self, other: &Self, overwrite: bool) -> Result<Self, SchemaError> {
        self.update(&other.raw, overwrite)
    }
}

/// `full` restricted to the keys of `set`, recursively (values from `full`; keys only in
/// `set` keep its value).
fn intersect(full: &AttrTree, set: &AttrTree) -> AttrTree {
    let mut out = AttrTree::new();
    for (k, v) in set.iter() {
        let node = match full.get(k) {
            Some(f) => intersect_node(f, v),
            None => v.clone(),
        };
        out.insert(k.clone(), node);
    }
    out
}

fn intersect_node(full: &Node, set: &Node) -> Node {
    match (&full.value, &set.value) {
        (Value::Map(fm), Value::Map(sm)) => Node::new(Value::Map(intersect(fm, sm))),
        // `None` validated into a model with nothing set dumps as `{}`.
        (Value::Map(_), Value::Null) => Node::new(Value::Map(AttrTree::new())),
        (Value::Seq(fs), Value::Seq(ss)) if fs.len() == ss.len() => Node::new(Value::Seq(
            fs.iter()
                .zip(ss)
                .map(|(f, s)| intersect_node(f, s))
                .collect(),
        )),
        _ => full.clone(),
    }
}

struct Ctx<'a> {
    log: &'a mut UpdateLog,
    suppress: bool,
}

fn as_update_tree(val: &Node, key: &str) -> Result<AttrTree, String> {
    match &val.value {
        Value::Map(m) => Ok(m.clone()),
        // `AttrDict(None)` is an empty dict.
        Value::Null => Ok(AttrTree::new()),
        other => Err(format!(
            "Cannot update `{key}` with {} `{other}`: must pass a dict",
            other.type_name()
        )),
    }
}

fn sub<'a>(tree: Option<&'a AttrTree>, key: &str) -> Option<&'a AttrTree> {
    tree.and_then(|t| t.get(key)).and_then(Node::as_map)
}

/// `CalliopeBaseModel.update`.
fn update_model(
    shape: &ModelShape,
    raw: &AttrTree,
    current: Option<&AttrTree>,
    update: &AttrTree,
    overwrite: bool,
    ctx: &mut Ctx<'_>,
) -> Result<AttrTree, String> {
    let mut new = raw.clone();
    for (key, val) in update {
        let field = match shape.field(key) {
            Shape::MapOr(m) => {
                let is_map = raw.get(key).is_some_and(Node::is_map);
                if is_map { Shape::Model(m) } else { Shape::Leaf }
            }
            s => s,
        };
        match field {
            Shape::Model(m) => {
                let upd = as_update_tree(val, key)?;
                let sub_raw = sub(Some(raw), key).cloned().unwrap_or_default();
                let res = update_model(m, &sub_raw, sub(current, key), &upd, overwrite, ctx)?;
                new.insert(key.clone(), res);
            }
            Shape::Dict(d) => {
                let upd = as_update_tree(val, key)?;
                let sub_raw = sub(Some(raw), key).cloned().unwrap_or_default();
                let res = update_dict(d, &sub_raw, sub(current, key), &upd, overwrite, ctx)?;
                new.insert(key.clone(), res);
            }
            Shape::List => {
                if overwrite {
                    new.insert(key.clone(), val.clone());
                } else {
                    ctx.log.push(format!(
                        "Skipping existing {} entry: `{key}` as overwrite is set to False",
                        shape.name
                    ));
                }
            }
            Shape::Leaf | Shape::MapOr(_) => {
                let is_set = raw.contains_key(key);
                let cur = current.and_then(|c| c.get(key));
                if cur.is_some_and(|c| c == val) {
                    // An explicitly given value equal to the (default) current value is
                    // recorded as set, so that it wins in later updates.
                    if !is_set {
                        new.insert(key.clone(), val.clone());
                    }
                    continue;
                }
                if !ctx.suppress && (!is_set || overwrite) {
                    let cur_txt = cur.map_or_else(|| "None".to_owned(), ToString::to_string);
                    ctx.log.push(format!(
                        "Updating {} `{key}`: {cur_txt} -> {val}",
                        shape.name
                    ));
                }
                if is_set && !overwrite {
                    ctx.log.push(format!(
                        "Skipping existing {} entry: `{key}` as overwrite is set to False",
                        shape.name
                    ));
                    continue;
                }
                new.insert(key.clone(), val.clone());
            }
        }
    }
    if !overwrite {
        // Python re-applies the explicitly set values of the original model on top.
        let saved = std::mem::replace(&mut ctx.suppress, true);
        let res = update_model(shape, &new, None, raw, true, ctx);
        ctx.suppress = saved;
        new = res?;
    }
    Ok(new)
}

/// `CalliopeDictModel.update`.
fn update_dict(
    shape: &DictShape,
    raw: &AttrTree,
    current: Option<&AttrTree>,
    update: &AttrTree,
    overwrite: bool,
    ctx: &mut Ctx<'_>,
) -> Result<AttrTree, String> {
    let mut new = raw.clone();
    for (key, val) in update {
        let existing = raw.get(key);
        let entry_shape = match (existing, shape.entry) {
            (None, _) => Shape::Leaf,
            (Some(n), Shape::MapOr(m)) if n.is_map() => Shape::Model(m),
            (Some(_), Shape::MapOr(_)) => Shape::Leaf,
            (Some(_), s) => s,
        };
        match entry_shape {
            Shape::Model(m) => {
                let upd = as_update_tree(val, key)?;
                let sub_raw = sub(Some(raw), key).cloned().unwrap_or_default();
                let res = update_model(m, &sub_raw, sub(current, key), &upd, overwrite, ctx)?;
                new.insert(key.clone(), res);
            }
            Shape::List => {
                if overwrite {
                    new.insert(key.clone(), val.clone());
                } else {
                    ctx.log.push(format!(
                        "Skipping existing {} entry: `{key}` as overwrite is set to False",
                        shape.name
                    ));
                }
            }
            Shape::Leaf | Shape::Dict(_) | Shape::MapOr(_) => {
                if existing.is_some_and(|e| e == val) {
                    continue;
                }
                if existing.is_none() || overwrite {
                    if !ctx.suppress {
                        ctx.log
                            .push(format!("Adding {} entry: `{key}`", shape.name));
                    }
                    new.insert(key.clone(), val.clone());
                }
            }
        }
    }
    Ok(new)
}
