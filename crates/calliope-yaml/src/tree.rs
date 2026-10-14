//! `AttrTree`: an ordered, nested string-keyed map with the semantics of Calliope's
//! `AttrDict` (`src/calliope/attrdict.py`).

use indexmap::IndexMap;
use indexmap::map::Entry;

use crate::error::TreeError;
use crate::value::{Node, Value};

/// The key that wipes a sub-tree in [`AttrTree::union`] when replacement is allowed.
pub const REPLACE_KEY: &str = "_REPLACE_";

/// An ordered nested map. Keys never contain dots: dotted keys passed to [`AttrTree::set_key`]
/// are split into nested maps, as in `AttrDict.set_key`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct AttrTree {
    entries: IndexMap<String, Node>,
}

impl AttrTree {
    /// An empty tree.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a tree from `(key, node)` pairs, applying `set_key` to each in order
    /// (`AttrDict.init_from_dict`): dotted keys are expanded and a mapping given for a key that
    /// already holds a mapping is merged into it.
    pub fn from_pairs<I, K>(pairs: I) -> Result<Self, TreeError>
    where
        I: IntoIterator<Item = (K, Node)>,
        K: AsRef<str>,
    {
        let mut tree = Self::new();
        for (k, v) in pairs {
            tree.set_key(k.as_ref(), v)?;
        }
        Ok(tree)
    }

    /// Number of top-level keys.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// True if there are no keys.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Iterates over top-level entries in order.
    pub fn iter(&self) -> indexmap::map::Iter<'_, String, Node> {
        self.entries.iter()
    }

    /// Iterates mutably over top-level entries in order.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&String, &mut Node)> {
        self.entries.iter_mut()
    }

    /// Top-level keys in order.
    pub fn keys(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// Top-level lookup (no dotted-key handling).
    pub fn get(&self, key: &str) -> Option<&Node> {
        self.entries.get(key)
    }

    /// Mutable top-level lookup (no dotted-key handling).
    pub fn get_mut(&mut self, key: &str) -> Option<&mut Node> {
        self.entries.get_mut(key)
    }

    /// True if the top-level key exists.
    pub fn contains_key(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    /// Inserts a top-level key verbatim (no dotted-key handling, no merging), keeping the
    /// position of an existing key like a Python `dict`.
    pub fn insert(&mut self, key: impl Into<String>, node: impl Into<Node>) -> Option<Node> {
        self.entries.insert(key.into(), node.into())
    }

    /// Removes a top-level key, keeping the order of the others (`dict.pop`).
    pub fn remove(&mut self, key: &str) -> Option<Node> {
        self.entries.shift_remove(key)
    }

    /// `AttrDict.set_key`: sets a possibly dotted key.
    ///
    /// - Missing intermediate maps are created and `null` intermediates are replaced by maps.
    /// - A mapping set on a key that already holds a mapping is merged into it, key by key.
    /// - Setting a nested key below a non-mapping value is an error.
    pub fn set_key(&mut self, key: &str, value: Node) -> Result<(), TreeError> {
        self.set_key_inner(key, value, key)
    }

    fn set_key_inner(&mut self, key: &str, value: Node, full: &str) -> Result<(), TreeError> {
        if let Some((head, rest)) = key.split_once('.') {
            let child = match self.entries.entry(head.to_owned()) {
                Entry::Vacant(e) => e.insert(Node::empty_map()),
                Entry::Occupied(e) => {
                    let node = e.into_mut();
                    if node.is_null() {
                        *node = Node::with_span(Value::Map(AttrTree::new()), node.span.clone());
                    }
                    node
                }
            };
            match &mut child.value {
                Value::Map(m) => m.set_key_inner(rest, value, full),
                _ => Err(TreeError::NestedOnNonMap {
                    key: full.to_owned(),
                    span: child.span.clone(),
                }),
            }
        } else {
            match (self.entries.get_mut(key), value.value) {
                (Some(existing), Value::Map(incoming)) => match &mut existing.value {
                    Value::Map(m) => {
                        for (k, v) in incoming.entries {
                            m.set_key_inner(&k, v, full)?;
                        }
                        Ok(())
                    }
                    // Python iterates over the (empty) incoming dict and does nothing.
                    _ if incoming.is_empty() => Ok(()),
                    _ => Err(TreeError::NestedOnNonMap {
                        key: full.to_owned(),
                        span: existing.span.clone(),
                    }),
                },
                (_, v) => {
                    self.entries
                        .insert(key.to_owned(), Node::with_span(v, value.span));
                    Ok(())
                }
            }
        }
    }

    /// `AttrDict.get_key` without a default: errors if the key or an intermediate map is missing.
    pub fn get_key(&self, key: &str) -> Result<&Node, TreeError> {
        let mut tree = self;
        let mut parts = key.split('.').peekable();
        while let Some(part) = parts.next() {
            let node = tree
                .entries
                .get(part)
                .ok_or_else(|| TreeError::KeyNotFound {
                    key: key.to_owned(),
                })?;
            if parts.peek().is_none() {
                return Ok(node);
            }
            tree = node.as_map().ok_or_else(|| TreeError::NestedOnNonMap {
                key: key.to_owned(),
                span: node.span.clone(),
            })?;
        }
        Err(TreeError::KeyNotFound {
            key: key.to_owned(),
        })
    }

    /// `AttrDict.get_key` with a default: `None` if the key is missing or an intermediate
    /// value is not a map.
    pub fn get_key_opt(&self, key: &str) -> Option<&Node> {
        self.get_key(key).ok()
    }

    /// Mutable version of [`AttrTree::get_key_opt`].
    pub fn get_key_mut(&mut self, key: &str) -> Option<&mut Node> {
        match key.split_once('.') {
            None => self.entries.get_mut(key),
            Some((head, rest)) => self.entries.get_mut(head)?.as_map_mut()?.get_key_mut(rest),
        }
    }

    /// `AttrDict.del_key`: deletes a possibly dotted key, then removes any parent map that
    /// became empty.
    pub fn del_key(&mut self, key: &str) -> Result<Node, TreeError> {
        match key.split_once('.') {
            None => self
                .entries
                .shift_remove(key)
                .ok_or_else(|| TreeError::KeyNotFound {
                    key: key.to_owned(),
                }),
            Some((head, rest)) => {
                let child = self
                    .entries
                    .get_mut(head)
                    .ok_or_else(|| TreeError::KeyNotFound {
                        key: key.to_owned(),
                    })?;
                let span = child.span.clone();
                let map = child.as_map_mut().ok_or(TreeError::NestedOnNonMap {
                    key: key.to_owned(),
                    span,
                })?;
                let removed = map.del_key(rest).map_err(|e| match e {
                    TreeError::KeyNotFound { .. } => TreeError::KeyNotFound {
                        key: key.to_owned(),
                    },
                    other => other,
                })?;
                if map.is_empty() {
                    self.entries.shift_remove(head);
                }
                Ok(removed)
            }
        }
    }

    /// `AttrDict.keys_nested()`: all leaf keys in dotted form. Empty maps count as leaves.
    pub fn keys_nested(&self) -> Vec<String> {
        let mut out = Vec::new();
        self.collect_keys(None, &mut out);
        out
    }

    fn collect_keys(&self, prefix: Option<&str>, out: &mut Vec<String>) {
        for (k, v) in &self.entries {
            let full = match prefix {
                Some(p) => format!("{p}.{k}"),
                None => k.clone(),
            };
            match v.as_map() {
                Some(m) if !m.is_empty() => m.collect_keys(Some(&full), out),
                _ => out.push(full),
            }
        }
    }

    /// `AttrDict.keys_nested(subkeys_as="dict")`.
    pub fn keys_nested_tree(&self) -> Vec<NestedKey> {
        self.entries
            .iter()
            .map(|(k, v)| match v.as_map() {
                Some(m) if !m.is_empty() => NestedKey::Branch(k.clone(), m.keys_nested_tree()),
                _ => NestedKey::Leaf(k.clone()),
            })
            .collect()
    }

    /// `AttrDict.as_dict_flat()`: leaf keys in dotted form with their values.
    pub fn as_flat(&self) -> IndexMap<String, &Node> {
        self.keys_nested()
            .into_iter()
            .filter_map(|k| {
                let v = self.get_key_opt(&k)?;
                Some((k, v))
            })
            .collect()
    }

    /// `AttrDict.union`: an in-place merge in which `other` takes precedence.
    ///
    /// - Without `allow_override`, a leaf key defined in both trees is an error.
    /// - A `null` in `other` does not replace a whole sub-tree of `self`.
    /// - With `allow_replacement`, `key._REPLACE_: value` deletes `key` and sets it to `value`.
    ///   Replacements are applied after all other keys.
    pub fn union(
        &mut self,
        other: &AttrTree,
        allow_override: bool,
        allow_replacement: bool,
    ) -> Result<(), TreeError> {
        let self_keys: std::collections::HashSet<String> = self.keys_nested().into_iter().collect();
        let other_keys = other.keys_nested();
        let (override_keys, wipe_keys): (Vec<_>, Vec<_>) = if allow_replacement {
            let (wipe, keep): (Vec<_>, Vec<_>) = other_keys
                .into_iter()
                .partition(|k| k.contains(REPLACE_KEY));
            let wipe = wipe
                .into_iter()
                .map(|k| {
                    let sep = format!(".{REPLACE_KEY}");
                    k.split(sep.as_str()).next().unwrap_or_default().to_owned()
                })
                .collect();
            (keep, wipe)
        } else {
            (other_keys, Vec::new())
        };

        for k in &override_keys {
            let other_value = other.get_key(k)?;
            if !allow_override && self_keys.contains(k) {
                let first = self.get_key_opt(k).and_then(|n| n.span.clone());
                return Err(TreeError::DefinedTwice {
                    key: k.clone(),
                    first,
                    second: other_value.span.clone(),
                });
            }
            let skip = other_value.is_null() && self.get_key_opt(k).is_some_and(Node::is_map);
            if !skip {
                self.set_key(k, other_value.clone())?;
            }
        }
        for k in &wipe_keys {
            let replacement = other.get_key(&format!("{k}.{REPLACE_KEY}"))?.clone();
            self.del_key(k).map_err(|e| match e {
                TreeError::KeyNotFound { key } => TreeError::ReplaceMissing {
                    key,
                    span: replacement.span.clone(),
                },
                other => other,
            })?;
            self.set_key(k, replacement)?;
        }
        Ok(())
    }
}

/// An entry of [`AttrTree::keys_nested_tree`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NestedKey {
    /// A key whose value is not a non-empty map.
    Leaf(String),
    /// A key holding a non-empty map, with that map's keys.
    Branch(String, Vec<NestedKey>),
}

impl<'a> IntoIterator for &'a AttrTree {
    type Item = (&'a String, &'a Node);
    type IntoIter = indexmap::map::Iter<'a, String, Node>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}

impl IntoIterator for AttrTree {
    type Item = (String, Node);
    type IntoIter = indexmap::map::IntoIter<String, Node>;
    fn into_iter(self) -> Self::IntoIter {
        self.entries.into_iter()
    }
}
