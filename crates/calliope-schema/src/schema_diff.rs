//! Structural comparison of JSON schemas (`xtask schema-diff`).
//!
//! Both the pydantic and the `schemars` schema are reduced to a canonical form that keeps what
//! decides validity and defaults, and drops presentation:
//!
//! - `$ref`s are inlined; titles, descriptions and `format`s are dropped;
//! - unions (`anyOf`, `oneOf`, multi-type `type` lists, `null` in `enum`) become a sorted set of
//!   single-type alternatives, each keeping only the keywords that apply to its type;
//! - string-keyed maps become `{"map": {"keys": <pattern>, "values": <schema>}}`, whether they are
//!   written with `patternProperties` (pydantic) or `additionalProperties` + `propertyNames`;
//! - `default` values are kept, with non-finite floats (`"nan"`, `"inf"`, `NaN`) written as
//!   `null`, because `serde_json` cannot represent them;
//! - object properties keep `required` and whether extra keys are allowed.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

/// The canonical form of a schema.
pub fn normalize(schema: &Value) -> Value {
    let defs = schema
        .get("$defs")
        .or_else(|| schema.get("definitions"))
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    norm(schema, &defs, 0)
}

/// Differences between the canonical forms of two schemas, as `path: left != right` lines.
pub fn diff(left: &Value, right: &Value) -> Vec<String> {
    let mut out = Vec::new();
    diff_at("", &normalize(left), &normalize(right), &mut out);
    out
}

fn resolve<'a>(r: &str, defs: &'a Map<String, Value>) -> Option<&'a Value> {
    let name = r.rsplit('/').next()?;
    defs.get(name)
}

fn norm_default(v: &Value) -> Value {
    match v {
        Value::String(s) if matches!(s.as_str(), "nan" | "inf" | "-inf") => Value::Null,
        Value::Number(n)
            if n.as_f64()
                .is_some_and(|f| f.fract() == 0.0 && f.abs() < 1e15) =>
        {
            #[allow(clippy::cast_possible_truncation)]
            let i = n.as_f64().unwrap_or_default() as i64;
            json!(i)
        }
        Value::Array(a) => Value::Array(a.iter().map(norm_default).collect()),
        Value::Object(o) => Value::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), norm_default(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn sort_key(v: &Value) -> String {
    v.to_string()
}

/// Splits a schema into canonical single-type alternatives.
fn alternatives(schema: &Value, defs: &Map<String, Value>, depth: usize) -> Vec<Value> {
    if depth > 64 {
        return vec![json!({"recursion": true})];
    }
    let Some(obj) = schema.as_object() else {
        // `true` / `false` schemas.
        return vec![if schema == &Value::Bool(true) {
            json!({"type": "any"})
        } else {
            schema.clone()
        }];
    };
    if let Some(r) = obj.get("$ref").and_then(Value::as_str) {
        let target = resolve(r, defs).cloned().unwrap_or(Value::Null);
        let mut merged = target.as_object().cloned().unwrap_or_default();
        for (k, v) in obj {
            if k != "$ref" {
                merged.insert(k.clone(), v.clone());
            }
        }
        return alternatives(&Value::Object(merged), defs, depth + 1);
    }
    for key in ["anyOf", "oneOf"] {
        if let Some(Value::Array(branches)) = obj.get(key) {
            let alts: Vec<Value> = branches
                .iter()
                .flat_map(|b| alternatives(b, defs, depth + 1))
                .collect();
            return merge_enums(alts);
        }
    }
    if let Some(Value::Array(all)) = obj.get("allOf")
        && all.len() == 1
    {
        let mut merged = all[0].as_object().cloned().unwrap_or_default();
        for (k, v) in obj {
            if k != "allOf" {
                merged.insert(k.clone(), v.clone());
            }
        }
        return alternatives(&Value::Object(merged), defs, depth + 1);
    }

    let mut types: Vec<String> = match obj.get("type") {
        Some(Value::String(t)) => vec![t.clone()],
        Some(Value::Array(ts)) => ts
            .iter()
            .filter_map(|t| t.as_str().map(str::to_owned))
            .collect(),
        _ => Vec::new(),
    };
    let enum_vals: Option<Vec<Value>> = obj
        .get("enum")
        .and_then(Value::as_array)
        .cloned()
        .or_else(|| obj.get("const").map(|c| vec![c.clone()]));
    if types.is_empty() {
        if let Some(vals) = &enum_vals {
            for v in vals {
                let t = match v {
                    Value::Null => "null",
                    Value::String(_) => "string",
                    Value::Bool(_) => "boolean",
                    Value::Number(n) if n.is_i64() => "integer",
                    Value::Number(_) => "number",
                    _ => "any",
                };
                if !types.iter().any(|x| x == t) {
                    types.push(t.to_owned());
                }
            }
        } else if obj.contains_key("properties")
            || obj.contains_key("additionalProperties")
            || obj.contains_key("patternProperties")
        {
            types.push("object".to_owned());
        } else if obj.contains_key("items") {
            types.push("array".to_owned());
        } else if obj.contains_key("pattern") {
            types.push("string".to_owned());
        } else {
            types.push("any".to_owned());
        }
    }
    let mut alts = Vec::new();
    for t in &types {
        let mut alt = Map::new();
        alt.insert("type".into(), json!(t));
        if let Some(vals) = &enum_vals {
            let mut own: Vec<Value> = vals
                .iter()
                .filter(|v| {
                    matches!(
                        (t.as_str(), v),
                        ("null", Value::Null)
                            | ("string", Value::String(_))
                            | ("boolean", Value::Bool(_))
                            | ("integer" | "number", Value::Number(_))
                    )
                })
                .cloned()
                .collect();
            own.sort_by_key(sort_key);
            if t != "null" {
                alt.insert("enum".into(), Value::Array(own));
            }
        }
        match t.as_str() {
            "string" => {
                if let Some(p) = obj.get("pattern") {
                    alt.insert("pattern".into(), p.clone());
                }
            }
            "integer" | "number" => {
                for k in ["minimum", "maximum", "exclusiveMinimum", "exclusiveMaximum"] {
                    if let Some(v) = obj.get(k) {
                        alt.insert(k.into(), norm_default(v));
                    }
                }
            }
            "array" => {
                if let Some(items) = obj.get("items") {
                    alt.insert("items".into(), norm(items, defs, depth + 1));
                }
                if let Some(v) = obj.get("minItems").filter(|v| v.as_u64() != Some(0)) {
                    alt.insert("minItems".into(), v.clone());
                }
                if obj.get("uniqueItems") == Some(&Value::Bool(true)) {
                    alt.insert("uniqueItems".into(), Value::Bool(true));
                }
            }
            "object" => object_keywords(obj, defs, depth, &mut alt),
            _ => {}
        }
        alts.push(Value::Object(alt));
    }
    alts
}

fn object_keywords(
    obj: &Map<String, Value>,
    defs: &Map<String, Value>,
    depth: usize,
    alt: &mut Map<String, Value>,
) {
    let props = obj.get("properties").and_then(Value::as_object);
    let pattern_props = obj.get("patternProperties").and_then(Value::as_object);
    if props.is_none_or(Map::is_empty) {
        // A map: values from `patternProperties` (one pattern) or `additionalProperties`.
        let (keys, values) = if let Some(pp) = pattern_props.filter(|p| p.len() == 1) {
            let (k, v) = pp.iter().next().expect("one entry");
            (json!(k), norm(v, defs, depth + 1))
        } else {
            let keys = obj
                .get("propertyNames")
                .and_then(|p| p.get("pattern"))
                .cloned()
                .unwrap_or(Value::Null);
            let values = match obj.get("additionalProperties") {
                Some(Value::Bool(false)) => json!(false),
                Some(v) => norm(v, defs, depth + 1),
                None => json!({"type": "any"}),
            };
            (keys, values)
        };
        if values != json!(false) {
            alt.insert("map".into(), json!({"keys": keys, "values": values}));
            return;
        }
    }
    let mut properties = BTreeMap::new();
    if let Some(props) = props {
        for (k, v) in props {
            properties.insert(k.clone(), norm(v, defs, depth + 1));
        }
    }
    alt.insert("properties".into(), json!(properties));
    let mut required: Vec<Value> = obj
        .get("required")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    required.sort_by_key(sort_key);
    alt.insert("required".into(), Value::Array(required));
    let extra = match obj.get("additionalProperties") {
        Some(Value::Bool(false)) => json!(false),
        Some(v) => norm(v, defs, depth + 1),
        None => json!({"type": "any"}),
    };
    alt.insert("additionalProperties".into(), extra);
}

/// Merges single-type enum alternatives (`oneOf` of `const`s) into one enum per type, then
/// sorts and deduplicates.
fn merge_enums(alts: Vec<Value>) -> Vec<Value> {
    let mut merged: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    let mut rest = Vec::new();
    for alt in alts {
        let is_enum = alt
            .as_object()
            .is_some_and(|o| o.len() == 2 && o.contains_key("type") && o.contains_key("enum"));
        if is_enum {
            let t = alt["type"].as_str().unwrap_or_default().to_owned();
            let vals = alt["enum"].as_array().cloned().unwrap_or_default();
            merged.entry(t).or_default().extend(vals);
        } else {
            rest.push(alt);
        }
    }
    for (t, mut vals) in merged {
        vals.sort_by_key(sort_key);
        vals.dedup();
        rest.push(json!({"type": t, "enum": vals}));
    }
    rest.sort_by_key(sort_key);
    rest.dedup();
    rest
}

fn norm(schema: &Value, defs: &Map<String, Value>, depth: usize) -> Value {
    let alts = merge_enums(alternatives(schema, defs, depth));
    // pydantic omits defaults made by `default_factory`; empty containers carry no information.
    let default = schema
        .as_object()
        .and_then(|o| o.get("default"))
        .map(norm_default)
        .filter(|d| d != &json!([]) && d != &json!({}));
    let core = if alts.len() == 1 {
        alts.into_iter().next().unwrap_or(Value::Null)
    } else {
        json!({"anyOf": alts})
    };
    match default {
        Some(d) => json!({"schema": core, "default": d}),
        None => core,
    }
}

fn diff_at(path: &str, a: &Value, b: &Value, out: &mut Vec<String>) {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            let keys: std::collections::BTreeSet<&String> = x.keys().chain(y.keys()).collect();
            for k in keys {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                match (x.get(k), y.get(k)) {
                    (Some(l), Some(r)) => diff_at(&p, l, r, out),
                    (Some(l), None) => out.push(format!("{p}: only left: {l}")),
                    (None, Some(r)) => out.push(format!("{p}: only right: {r}")),
                    (None, None) => {}
                }
            }
        }
        (Value::Array(x), Value::Array(y)) if x.len() == y.len() => {
            for (i, (l, r)) in x.iter().zip(y).enumerate() {
                diff_at(&format!("{path}[{i}]"), l, r, out);
            }
        }
        _ if a == b => {}
        _ => out.push(format!("{path}: {a} != {b}")),
    }
}
