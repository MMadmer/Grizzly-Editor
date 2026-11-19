//! The library's files as text: a value written as Lua data.
//!
//! Every file of the library is `return { ... }` of literals, as every asset of the editor is
//! (`docs/ASSETS.md`): a row of a list on one line, a card laid out key by key. The value is
//! built as JSON (the commands answer the same value) and written here; a key keeps the order it
//! was inserted in.

use serde_json::Value;

/// A name that stands as a key without brackets.
fn is_ident(s: &str) -> bool {
    const KEYWORDS: [&str; 22] = [
        "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local",
        "nil", "not", "or", "repeat", "return", "then", "true", "until", "while", "goto",
    ];
    let mut chars = s.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !KEYWORDS.contains(&s)
}

fn key_into(out: &mut String, k: &str) {
    if is_ident(k) {
        out.push_str(k);
    } else {
        out.push('[');
        quote_into(out, k);
        out.push(']');
    }
}

/// A string as a Lua literal: the escapes every reader of the editor takes.
pub fn quote_into(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\0' => out.push_str("\\0"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\{}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

fn number_into(out: &mut String, n: &serde_json::Number) {
    if let Some(i) = n.as_i64() {
        out.push_str(&i.to_string());
    } else if let Some(u) = n.as_u64() {
        out.push_str(&u.to_string());
    } else {
        let f = n.as_f64().unwrap_or(0.0);
        if f.is_finite() {
            // the shortest text that reads back to the value; a number that came from a
            // single-precision one is said as that one (1.35, not 1.350000023841858)
            let single = f as f32;
            if f64::from(single) == f {
                out.push_str(&format!("{single}"));
            } else {
                out.push_str(&format!("{f}"));
            }
        } else {
            out.push('0');
        }
    }
}

/// A value on one line.
pub fn inline_into(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => number_into(out, n),
        Value::String(s) => quote_into(out, s),
        Value::Array(a) => {
            if a.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                inline_into(out, x);
            }
            out.push_str(" }");
        }
        Value::Object(m) => {
            if m.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push_str("{ ");
            for (i, (k, x)) in m.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                key_into(out, k);
                out.push_str(" = ");
                inline_into(out, x);
            }
            out.push_str(" }");
        }
    }
}

/// A value on one line.
#[must_use]
pub fn inline(v: &Value) -> String {
    let mut out = String::new();
    inline_into(&mut out, v);
    out
}

/// The widest line a value is kept on before it is laid out over several.
const WIDTH: usize = 150;

/// A value laid out: on one line when it fits, else a key or an element a line.
pub fn pretty_into(out: &mut String, v: &Value, indent: usize) {
    let line = inline(v);
    if line.len() + indent <= WIDTH || !matches!(v, Value::Array(_) | Value::Object(_)) {
        out.push_str(&line);
        return;
    }
    let pad = "  ".repeat(indent + 1);
    out.push_str("{\n");
    match v {
        Value::Array(a) => {
            for x in a {
                out.push_str(&pad);
                pretty_into(out, x, indent + 1);
                out.push_str(",\n");
            }
        }
        Value::Object(m) => {
            for (k, x) in m {
                out.push_str(&pad);
                key_into(out, k);
                out.push_str(" = ");
                pretty_into(out, x, indent + 1);
                out.push_str(",\n");
            }
        }
        _ => {}
    }
    out.push_str(&"  ".repeat(indent));
    out.push('}');
}

/// A whole file: the comment that says what it is, then `return <value>`.
#[must_use]
pub fn file(comment: &str, v: &Value) -> String {
    let mut out = String::new();
    for l in comment.lines() {
        out.push_str("-- ");
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("return ");
    pretty_into(&mut out, v, 0);
    out.push('\n');
    out
}

/// A file of rows: every key of `top` is a list whose elements stand one a line, so a search
/// finds a whole row and a row can be read without the rest. Other keys are laid out as usual.
#[must_use]
pub fn rows_file(comment: &str, top: &Value) -> String {
    let mut out = String::new();
    for l in comment.lines() {
        out.push_str("-- ");
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("return {\n");
    if let Value::Object(m) = top {
        for (k, v) in m {
            out.push_str("  ");
            key_into(&mut out, k);
            out.push_str(" = ");
            match v {
                Value::Array(rows) if rows.iter().any(Value::is_object) => {
                    out.push_str("{\n");
                    for r in rows {
                        out.push_str("    ");
                        inline_into(&mut out, r);
                        out.push_str(",\n");
                    }
                    out.push_str("  }");
                }
                other => pretty_into(&mut out, other, 1),
            }
            out.push_str(",\n");
        }
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_file_reads_back_as_the_value_it_was_written_from() {
        let v = json!({
            "card": 1,
            "name": "wood\\crate \"a\"",
            "size": [1.5, 0.25, 2],
            "flags": { "end": true, "has space": false },
            "rows": [{ "name": "a", "n": 1 }, { "name": "b", "n": -2.5 }],
            "long": (0..80).map(|i| json!({ "index": i, "text": "some words here" })).collect::<Vec<_>>(),
        });
        for text in [file("generated\nline two", &v), rows_file("generated", &v)] {
            assert!(text.starts_with("-- generated\n"));
            let back = gz_luadata::parse(&text).unwrap_or_else(|e| panic!("{e:?}\n{text}"));
            assert_eq!(gz_luadata::json::to_json(&back), v, "{text}");
        }
        // a row stands on one line
        let rows = rows_file("x", &v);
        assert!(rows.contains("    { name = \"a\", n = 1 },\n"), "{rows}");
        // a reserved word and a name with a space are bracketed keys
        assert!(inline(&v["flags"]).contains("[\"end\"] = true"));
    }
}
