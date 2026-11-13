//! New text for a document: the canonical layout of `write`, at the indentation of the place it
//! goes into, with the file's own indent unit and line ends.

use crate::value::Value;
use crate::write::{WIDTH, format_number, key, long_string_into, quote_into, wants_long_bracket};

/// How a file lays itself out: the indent unit (a tab or n spaces) and its line end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Layout {
    pub unit: String,
    pub nl: &'static str,
}

impl Default for Layout {
    fn default() -> Self {
        Self { unit: "  ".into(), nl: "\n" }
    }
}

/// The width of indentation, a tab counted as four columns.
pub(crate) fn width(s: &str) -> usize {
    s.chars().map(|c| if c == '\t' { 4 } else { 1 }).sum()
}

/// Whether a value can be written as data: a NaN cannot.
pub(crate) fn writable(v: &Value) -> bool {
    match v {
        Value::Number(n) => !n.is_nan(),
        Value::Table(t) => t.arr.iter().all(writable) && t.fields.iter().all(|(_, x)| writable(x)),
        _ => true,
    }
}

fn number(n: f64) -> String {
    if n == f64::INFINITY {
        "1e999".into()
    } else if n == f64::NEG_INFINITY {
        "-1e999".into()
    } else {
        format_number(n)
    }
}

fn single_quote_into(out: &mut String, s: &str) {
    out.push('\'');
    for ch in s.chars() {
        match ch {
            '\'' => out.push_str("\\'"),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\{:03}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('\'');
}

/// A scalar as the file writes it. `old` is the literal it replaces: a string written in single
/// quotes keeps them.
pub(crate) fn scalar(v: &Value, old: Option<&[u8]>) -> String {
    let mut out = String::new();
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&number(*n)),
        Value::String(s) => {
            if wants_long_bracket(s) {
                long_string_into(&mut out, s);
            } else if old.is_some_and(|o| o.first() == Some(&b'\'')) {
                single_quote_into(&mut out, s);
            } else {
                quote_into(&mut out, s);
            }
        }
        Value::Table(_) => out.push_str("{}"),
    }
    out
}

/// The one-line form; `None` when a string needs a long bracket.
fn inline_into(out: &mut String, v: &Value) -> bool {
    match v {
        Value::Table(t) => {
            if t.is_empty() {
                out.push_str("{}");
                return true;
            }
            out.push_str("{ ");
            let mut first = true;
            for x in &t.arr {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                if !inline_into(out, x) {
                    return false;
                }
            }
            for (k, x) in &t.fields {
                if !first {
                    out.push_str(", ");
                }
                first = false;
                out.push_str(&key(k));
                out.push_str(" = ");
                if !inline_into(out, x) {
                    return false;
                }
            }
            out.push_str(" }");
            true
        }
        Value::String(s) if wants_long_bracket(s) => false,
        _ => {
            out.push_str(&scalar(v, None));
            true
        }
    }
}

fn pretty_into(out: &mut String, v: &Value, ind: &str, used: usize, lay: &Layout) {
    let Value::Table(t) = v else {
        out.push_str(&scalar(v, None));
        return;
    };
    let mut one = String::new();
    if inline_into(&mut one, v) && used + one.chars().count() <= WIDTH {
        out.push_str(&one);
        return;
    }
    let inner = format!("{ind}{}", lay.unit);
    let w = width(&inner);
    out.push_str("{\n");
    for x in &t.arr {
        out.push_str(&inner);
        pretty_into(out, x, &inner, w, lay);
        out.push_str(",\n");
    }
    for (k, x) in &t.fields {
        out.push_str(&inner);
        let kk = key(k);
        out.push_str(&kk);
        out.push_str(" = ");
        pretty_into(out, x, &inner, w + kk.len() + 3, lay);
        out.push_str(",\n");
    }
    out.push_str(ind);
    out.push('}');
}

/// A node table: `{ id = .., kind = ..[, once = ..],` on the first line, one field per line
/// after it, the closing brace on the last field's line.
fn node_into(out: &mut String, v: &Value, ind: &str, lay: &Layout) {
    let mut one = String::new();
    if inline_into(&mut one, v) && width(ind) + one.chars().count() <= WIDTH {
        out.push_str(&one);
        return;
    }
    let fields = v.fields();
    out.push_str("{ ");
    let mut i = 0;
    while let Some((k, x)) = fields.get(i) {
        if !matches!(k.as_str(), "id" | "kind" | "once") {
            break;
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&key(k));
        out.push_str(" = ");
        inline_into(out, x);
        i += 1;
    }
    if i >= fields.len() {
        out.push_str(" }");
        return;
    }
    out.push_str(",\n");
    let inner = format!("{ind}{}", lay.unit);
    let w = width(&inner);
    while let Some((k, x)) = fields.get(i) {
        out.push_str(&inner);
        let kk = key(k);
        out.push_str(&kk);
        out.push_str(" = ");
        pretty_into(out, x, &inner, w + kk.len() + 3, lay);
        out.push_str(if i + 1 < fields.len() { ",\n" } else { " }" });
        i += 1;
    }
}

fn line_ends(s: String, lay: &Layout) -> String {
    if lay.nl == "\n" { s } else { s.replace('\n', lay.nl) }
}

/// A value at indentation `ind`, its first line already `used` columns wide.
pub(crate) fn value(v: &Value, ind: &str, used: usize, lay: &Layout) -> String {
    let mut out = String::new();
    pretty_into(&mut out, v, ind, used, lay);
    line_ends(out, lay)
}

/// An entry: `key = value`, or the value alone for a positional one.
pub(crate) fn entry(k: Option<&str>, v: &Value, ind: &str, node: bool, lay: &Layout) -> String {
    let mut out = String::new();
    let mut used = width(ind);
    if let Some(k) = k {
        let kk = key(k);
        used += kk.len() + 3;
        out.push_str(&kk);
        out.push_str(" = ");
    }
    if node && k.is_none() && v.is_table() {
        node_into(&mut out, v, ind, lay);
    } else {
        pretty_into(&mut out, v, ind, used, lay);
    }
    line_ends(out, lay)
}
