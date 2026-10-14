//! YAML text to [`Node`] trees, matching `ruamel.yaml` `YAML(typ="safe")` (YAML 1.2)
//! followed by `AttrDict(...)`.

use std::collections::HashMap;
use std::sync::LazyLock;

use indexmap::IndexMap;
use regex::Regex;
use yaml_rust2::parser::{Event, MarkedEventReceiver, Parser, Tag};
use yaml_rust2::scanner::{Marker, TScalarStyle};

use crate::error::LoadError;
use crate::tree::AttrTree;
use crate::value::{Node, Source, Span, Value};

static FLOAT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^(?:[-+]?(?:[0-9][0-9_]*)\.[0-9_]*(?:[eE][-+]?[0-9]+)?|[-+]?(?:[0-9][0-9_]*)(?:[eE][-+]?[0-9]+)|[-+]?\.[0-9_]+(?:[eE][-+][0-9]+)?|[-+]?\.(?:inf|Inf|INF)|\.(?:nan|NaN|NAN))$",
    )
    .expect("valid regex")
});
static INT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:[-+]?0b[0-1_]+|[-+]?0o?[0-7_]+|[-+]?[0-9_]+|[-+]?0x[0-9a-fA-F_]+)$")
        .expect("valid regex")
});
static DATE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[0-9]{4}-[0-9]{2}-[0-9]{2}$").expect("valid regex"));
static DATETIME_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"^[0-9]{4}-[0-9]{1,2}-[0-9]{1,2}(?:[Tt]|[ \t]+)[0-9]{1,2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]*)?(?:[ \t]*(?:Z|[-+][0-9]{1,2}(?::[0-9]{2})?))?$",
    )
    .expect("valid regex")
});

/// Resolves a plain (unquoted, untagged) scalar with the YAML 1.2 implicit resolvers of
/// `ruamel.yaml` (bool, float, int, null, timestamp; otherwise string).
pub fn resolve_plain(s: &str) -> Value {
    match s {
        "true" | "True" | "TRUE" => return Value::Bool(true),
        "false" | "False" | "FALSE" => return Value::Bool(false),
        "" | "~" | "null" | "Null" | "NULL" => return Value::Null,
        _ => {}
    }
    // ruamel only tries a resolver if the first character is one it registered for.
    let first = s.chars().next().unwrap_or(' ');
    if (first.is_ascii_digit() || matches!(first, '-' | '+' | '.'))
        && FLOAT_RE.is_match(s)
        && let Some(f) = construct_float(s)
    {
        return Value::Float(f);
    }
    if (first.is_ascii_digit() || matches!(first, '-' | '+'))
        && INT_RE.is_match(s)
        && let Some(v) = construct_int(s)
    {
        return v;
    }
    if DATE_RE.is_match(s) {
        return Value::Date(s.to_owned());
    }
    if DATETIME_RE.is_match(s) {
        return Value::DateTime(s.to_owned());
    }
    Value::Str(s.to_owned())
}

fn construct_float(s: &str) -> Option<f64> {
    let clean: String = s.chars().filter(|c| *c != '_').collect();
    let lower = clean.to_ascii_lowercase();
    match lower.as_str() {
        ".inf" | "+.inf" => Some(f64::INFINITY),
        "-.inf" => Some(f64::NEG_INFINITY),
        ".nan" => Some(f64::NAN),
        _ => clean.parse().ok(),
    }
}

fn construct_int(s: &str) -> Option<Value> {
    let clean: String = s.chars().filter(|c| *c != '_').collect();
    let (neg, digits) = match clean.as_bytes().first() {
        Some(b'-') => (true, &clean[1..]),
        Some(b'+') => (false, &clean[1..]),
        _ => (false, clean.as_str()),
    };
    if digits.is_empty() {
        return None;
    }
    let (radix, body) = if let Some(b) = digits.strip_prefix("0b") {
        (2, b)
    } else if let Some(b) = digits.strip_prefix("0x") {
        (16, b)
    } else if let Some(b) = digits.strip_prefix("0o") {
        (8, b)
    } else {
        (10, digits)
    };
    if body.is_empty() {
        return Some(Value::Int(0));
    }
    match i128::from_str_radix(body, radix) {
        Ok(v) => {
            let v = if neg { -v } else { v };
            Some(i64::try_from(v).map_or_else(
                // Python ints are unbounded; beyond i64 we keep the magnitude as a float.
                #[allow(clippy::cast_precision_loss)]
                |_| Value::Float(v as f64),
                Value::Int,
            ))
        }
        Err(_) => None,
    }
}

fn resolve_tagged(s: &str, tag: &Tag, span: &Span) -> Result<Value, LoadError> {
    let invalid = |what: &str| LoadError::Invalid {
        message: format!("could not construct {what} from `{s}`"),
        span: span.clone(),
    };
    if tag.handle != "!!" && tag.handle != "tag:yaml.org,2002:" {
        return Err(LoadError::Invalid {
            message: format!(
                "could not determine a constructor for the tag '{}{}'",
                tag.handle, tag.suffix
            ),
            span: span.clone(),
        });
    }
    match tag.suffix.as_str() {
        "str" => Ok(Value::Str(s.to_owned())),
        "null" => Ok(Value::Null),
        "bool" => match resolve_plain(s) {
            v @ Value::Bool(_) => Ok(v),
            _ => Err(invalid("a bool")),
        },
        "int" => construct_int(s).ok_or_else(|| invalid("an int")),
        "float" => construct_float(s)
            .map(Value::Float)
            .ok_or_else(|| invalid("a float")),
        other => Err(LoadError::Invalid {
            message: format!("unsupported tag '!!{other}'"),
            span: span.clone(),
        }),
    }
}

enum Frame {
    Seq {
        items: Vec<Node>,
        anchor: usize,
        span: Span,
    },
    Map {
        /// Explicit `(key, value)` pairs, keys as scalar nodes.
        pairs: Vec<(Node, Node)>,
        /// Values of `<<` merge keys, in order.
        merges: Vec<(Node, Span)>,
        pending_key: Option<PendingKey>,
        anchor: usize,
        span: Span,
    },
}

enum PendingKey {
    Key(Node),
    /// A plain `<<` scalar in key position.
    Merge(Span),
}

struct Builder {
    source: Source,
    stack: Vec<Frame>,
    anchors: HashMap<usize, Node>,
    root: Option<Node>,
    error: Option<LoadError>,
}

impl Builder {
    fn span(&self, mark: Marker) -> Span {
        Span {
            source: self.source.clone(),
            line: mark.line(),
            col: mark.col() + 1,
        }
    }

    fn push_value(&mut self, node: Node, anchor: usize) -> Result<(), LoadError> {
        if anchor != 0 {
            self.anchors.insert(anchor, node.clone());
        }
        match self.stack.last_mut() {
            None => self.root = Some(node),
            Some(Frame::Seq { items, .. }) => items.push(node),
            Some(Frame::Map {
                pairs,
                merges,
                pending_key,
                ..
            }) => match pending_key.take() {
                None => *pending_key = Some(PendingKey::Key(node)),
                Some(PendingKey::Key(key)) => pairs.push((key, node)),
                Some(PendingKey::Merge(span)) => merges.push((node, span)),
            },
        }
        Ok(())
    }

    fn handle(&mut self, ev: Event, mark: Marker) -> Result<(), LoadError> {
        match ev {
            Event::Scalar(text, style, anchor, tag) => {
                let span = self.span(mark);
                if let Some(Frame::Map { pending_key, .. }) = self.stack.last_mut()
                    && pending_key.is_none()
                    && text == "<<"
                    && tag.is_none()
                    && matches!(style, TScalarStyle::Plain)
                {
                    *pending_key = Some(PendingKey::Merge(span));
                    return Ok(());
                }
                let value = match (&style, &tag) {
                    (_, Some(tag)) => resolve_tagged(&text, tag, &span)?,
                    (TScalarStyle::Plain, None) => resolve_plain(&text),
                    _ => Value::Str(text),
                };
                self.push_value(Node::with_span(value, Some(span)), anchor)
            }
            Event::SequenceStart(anchor, _tag) => {
                let span = self.span(mark);
                self.stack.push(Frame::Seq {
                    items: Vec::new(),
                    anchor,
                    span,
                });
                Ok(())
            }
            Event::MappingStart(anchor, _tag) => {
                let span = self.span(mark);
                self.stack.push(Frame::Map {
                    pairs: Vec::new(),
                    merges: Vec::new(),
                    pending_key: None,
                    anchor,
                    span,
                });
                Ok(())
            }
            Event::SequenceEnd => match self.stack.pop() {
                Some(Frame::Seq {
                    items,
                    anchor,
                    span,
                }) => self.push_value(Node::with_span(Value::Seq(items), Some(span)), anchor),
                _ => Err(LoadError::Invalid {
                    message: "unexpected end of sequence".to_owned(),
                    span: self.span(mark),
                }),
            },
            Event::MappingEnd => match self.stack.pop() {
                Some(Frame::Map {
                    pairs,
                    merges,
                    anchor,
                    span,
                    ..
                }) => {
                    let tree = build_mapping(pairs, merges)?;
                    self.push_value(Node::with_span(Value::Map(tree), Some(span)), anchor)
                }
                _ => Err(LoadError::Invalid {
                    message: "unexpected end of mapping".to_owned(),
                    span: self.span(mark),
                }),
            },
            Event::Alias(id) => {
                let span = self.span(mark);
                let node = self.anchors.get(&id).cloned().ok_or(LoadError::Invalid {
                    message: "found undefined alias".to_owned(),
                    span,
                })?;
                self.push_value(node, 0)
            }
            Event::Nothing
            | Event::StreamStart
            | Event::StreamEnd
            | Event::DocumentStart
            | Event::DocumentEnd => Ok(()),
        }
    }
}

fn node_span(node: &Node) -> Span {
    node.span.clone().unwrap_or_else(|| Span {
        source: Source::new("<unknown>"),
        line: 0,
        col: 0,
    })
}

fn key_to_string(key: &Node) -> Result<String, LoadError> {
    match &key.value {
        Value::Str(s) => Ok(s.clone()),
        Value::Int(i) => Ok(i.to_string()),
        Value::Bool(true) => Ok("True".to_owned()),
        Value::Bool(false) => Ok("False".to_owned()),
        other => Err(LoadError::BadKey {
            key: other.to_string(),
            span: node_span(key),
        }),
    }
}

/// Builds an `AttrDict` from a YAML mapping: merge keys first (later sources lose), then the
/// explicit pairs, with dict semantics (later wins, first position kept); then `AttrDict`
/// key expansion.
fn build_mapping(
    pairs: Vec<(Node, Node)>,
    merges: Vec<(Node, Span)>,
) -> Result<AttrTree, LoadError> {
    let mut plain: IndexMap<String, Node> = IndexMap::new();
    if merges.len() > 1 {
        return Err(LoadError::DuplicateKey {
            key: "<<".to_owned(),
            first: merges[0].1.clone(),
            second: merges[1].1.clone(),
        });
    }
    if let Some((merge, span)) = merges.into_iter().next() {
        let sources: Vec<AttrTree> = match merge.value {
            Value::Map(m) => vec![m],
            Value::Seq(items) => items
                .into_iter()
                .map(|n| match n.value {
                    Value::Map(m) => Ok(m),
                    _ => Err(LoadError::Invalid {
                        message: "expected a mapping for merging".to_owned(),
                        span: span.clone(),
                    }),
                })
                .collect::<Result<_, _>>()?,
            _ => {
                return Err(LoadError::Invalid {
                    message: "expected a mapping or list of mappings for merging".to_owned(),
                    span,
                });
            }
        };
        for src in sources.into_iter().rev() {
            for (k, v) in src {
                plain.insert(k, v);
            }
        }
    }
    let mut seen: HashMap<String, Span> = HashMap::new();
    for (key, value) in pairs {
        let k = key_to_string(&key)?;
        let span = node_span(&key);
        if let Some(first) = seen.get(&k) {
            return Err(LoadError::DuplicateKey {
                key: k,
                first: first.clone(),
                second: span,
            });
        }
        seen.insert(k.clone(), span);
        plain.insert(k, value);
    }
    Ok(AttrTree::from_pairs(plain)?)
}

/// Parses YAML text into a node. `source` names the document in locations and errors.
pub fn parse_node(text: &str, source: &Source) -> Result<Node, LoadError> {
    let mut builder = Builder {
        source: source.clone(),
        stack: Vec::new(),
        anchors: HashMap::new(),
        root: None,
        error: None,
    };
    let mut parser = Parser::new_from_str(text);
    let result = parser.load(&mut builder, false);
    if let Err(e) = result {
        let m = e.marker();
        return Err(LoadError::Syntax {
            location: format!("{}:{}:{}", source.name(), m.line(), m.col() + 1),
            message: e.info().to_owned(),
        });
    }
    if let Some(e) = builder.error {
        return Err(e);
    }
    Ok(builder.root.unwrap_or_else(|| Node::new(Value::Null)))
}

/// Parses YAML text whose top level must be a mapping (`calliope.io._yaml_load`).
pub fn parse_tree(text: &str, source: &Source) -> Result<AttrTree, LoadError> {
    match parse_node(text, source)?.value {
        Value::Map(m) => Ok(m),
        _ => Err(LoadError::NotAMapping {
            source_name: source.name().to_owned(),
        }),
    }
}

impl MarkedEventReceiver for Builder {
    fn on_event(&mut self, ev: Event, mark: Marker) {
        if self.error.is_some() {
            return;
        }
        if let Err(e) = self.handle(ev, mark) {
            self.error = Some(e);
        }
    }
}
