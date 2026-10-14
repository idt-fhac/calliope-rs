//! Prints `read_yaml_file(path)` in a canonical JSON form, one file per line, for comparison
//! with `parity_dump.py` (Python `calliope.io.read_rich_yaml`).
//!
//! Usage: `cargo run -p calliope-yaml --example dump_yaml -- FILE...`

use calliope_yaml::{Node, Value, python_float, read_yaml_file};

fn canon(node: &Node, out: &mut String) {
    match &node.value {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Int(i) => out.push_str(&i.to_string()),
        Value::Float(f) => out.push_str(&format!("{{\"f\": \"{}\"}}", python_float(*f))),
        Value::Str(s) => out.push_str(&json_str(s)),
        Value::Date(s) => out.push_str(&format!("{{\"date\": {}}}", json_str(s))),
        Value::DateTime(s) => out.push_str(&format!("{{\"datetime\": {}}}", json_str(s))),
        Value::Seq(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                canon(item, out);
            }
            out.push(']');
        }
        Value::Map(m) => {
            out.push_str("{\"map\": [");
            for (i, (k, v)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push('[');
                out.push_str(&json_str(k));
                out.push_str(", ");
                canon(v, out);
                out.push(']');
            }
            out.push_str("]}");
        }
    }
}

fn json_str(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn main() {
    for path in std::env::args().skip(1) {
        let mut out = String::new();
        match read_yaml_file(&path, false) {
            Ok(tree) => canon(&Node::from(tree), &mut out),
            Err(e) => out = format!("{{\"error\": {}}}", json_str(&e.to_string())),
        }
        println!("{path}\t{out}");
    }
}
