//! `calliope.io.to_yaml`: block-style YAML with 2-space indentation and sequences at the
//! indentation of their key, in key order.
//!
//! The output re-loads to an equal tree; it is not byte-identical to ruamel's output
//! (for example `null` is written out, and quoting is chosen independently).

use std::fmt::Write;

use crate::load::resolve_plain;
use crate::tree::AttrTree;
use crate::value::{Node, Value, python_float};

/// Serialises a tree to YAML text.
pub fn to_yaml(tree: &AttrTree) -> String {
    let mut out = String::new();
    if tree.is_empty() {
        out.push_str("{}\n");
    } else {
        write_map(&mut out, tree, 0);
    }
    out
}

fn indent(out: &mut String, n: usize) {
    out.extend(std::iter::repeat_n(' ', n));
}

fn write_map(out: &mut String, tree: &AttrTree, level: usize) {
    for (k, v) in tree {
        indent(out, level);
        out.push_str(&scalar_str(k));
        out.push(':');
        write_value_after_key(out, v, level);
    }
}

fn write_value_after_key(out: &mut String, node: &Node, level: usize) {
    match &node.value {
        Value::Map(m) if !m.is_empty() => {
            out.push('\n');
            write_map(out, m, level + 2);
        }
        Value::Seq(items) if !items.is_empty() => {
            out.push('\n');
            write_seq(out, items, level);
        }
        other => {
            out.push(' ');
            out.push_str(&inline(other));
            out.push('\n');
        }
    }
}

fn write_seq(out: &mut String, items: &[Node], level: usize) {
    for item in items {
        indent(out, level);
        out.push_str("- ");
        match &item.value {
            Value::Map(m) if !m.is_empty() => {
                // First entry on the dash line, the rest aligned under it.
                let mut first = true;
                for (k, v) in m {
                    if !first {
                        indent(out, level + 2);
                    }
                    first = false;
                    out.push_str(&scalar_str(k));
                    out.push(':');
                    write_value_after_key(out, v, level + 2);
                }
            }
            Value::Seq(inner) if !inner.is_empty() => {
                // Nested sequence: start it on the dash line.
                let mut sub = String::new();
                write_seq(&mut sub, inner, level + 2);
                out.push_str(sub.trim_start());
            }
            other => {
                out.push_str(&inline(other));
                out.push('\n');
            }
        }
    }
}

fn inline(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(true) => "true".to_owned(),
        Value::Bool(false) => "false".to_owned(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.is_nan() => ".nan".to_owned(),
        Value::Float(f) if f.is_infinite() => if *f > 0.0 { ".inf" } else { "-.inf" }.to_owned(),
        Value::Float(f) => python_float(*f),
        Value::Str(s) => scalar_str(s),
        Value::Date(s) | Value::DateTime(s) => s.clone(),
        Value::Seq(_) => "[]".to_owned(),
        Value::Map(_) => "{}".to_owned(),
    }
}

fn needs_quotes(s: &str) -> bool {
    if s.is_empty() || s.trim() != s {
        return true;
    }
    if !matches!(resolve_plain(s), Value::Str(_)) {
        return true;
    }
    let first = s.chars().next().unwrap_or(' ');
    if "-?:,[]{}#&*!|>'\"%@`".contains(first) {
        return true;
    }
    s.contains(": ")
        || s.contains(" #")
        || s.ends_with(':')
        || s.chars()
            .any(|c| c.is_control() || matches!(c, '\u{85}' | '\u{2028}' | '\u{2029}'))
}

fn scalar_str(s: &str) -> String {
    if !needs_quotes(s) {
        return s.to_owned();
    }
    if !s.chars().any(char::is_control) {
        return format!("'{}'", s.replace('\'', "''"));
    }
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
