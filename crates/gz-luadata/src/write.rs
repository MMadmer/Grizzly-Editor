//! The canonical writer of `NqLua.cpp`: the same bytes for the same value. Layout, key quoting,
//! number formatting and string escaping follow the C++ line by line, including where it is
//! odd (a value that needs a long bracket cuts the inline form short, see [`inline_into`]).

use crate::num::format_g;
use crate::value::{Table, Value};

/// The line width the pretty form fits into (`kWidth`).
pub const WIDTH: usize = 100;

const KEYWORDS: [&str; 22] = [
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "if", "in", "local", "nil",
    "not", "or", "repeat", "return", "then", "true", "until", "while", "goto",
];

/// The part of a key the C++ sees through `c_str()`: everything before the first NUL.
fn c_str(s: &str) -> &str {
    s.split('\0').next().unwrap_or("")
}

/// A Lua reserved word (`IsLuaKeyword`).
#[must_use]
pub fn is_keyword(s: &str) -> bool {
    KEYWORDS.contains(&c_str(s))
}

/// A name that can stand as a key without brackets (`IsIdent`): ASCII letters, digits and `_`,
/// not starting with a digit, not a reserved word.
#[must_use]
pub fn is_ident(s: &str) -> bool {
    let s = c_str(s);
    let b = s.as_bytes();
    let Some(&first) = b.first() else { return false };
    if !(first.is_ascii_alphabetic() || first == b'_') {
        return false;
    }
    b.iter().all(|&c| c.is_ascii_alphanumeric() || c == b'_') && !is_keyword(s)
}

/// A number as the file shows it (`FormatNumber`): a whole number below 9e15 as an integer, a
/// value that is exactly a float at float precision, anything else in the shortest of 15 or 17
/// significant digits that reads back.
#[must_use]
pub fn format_number(n: f64) -> String {
    if let Some(i) = whole(n) {
        return i.to_string();
    }
    if n == f64::from(n as f32) {
        let s = format_g(n, 7);
        if parse_c(&s).map(|x| x as f32) == Some(n as f32) {
            return s;
        }
    }
    let s = format_g(n, 15);
    if parse_c(&s) == Some(n) { s } else { format_g(n, 17) }
}

/// A number as `AsString` gives it: a whole number below 9e15 as an integer, anything else
/// `%.15g`.
#[must_use]
pub fn number_to_text(n: f64) -> String {
    match whole(n) {
        Some(i) => i.to_string(),
        None => format_g(n, 15),
    }
}

/// `n == (double)(long long)n && fabs(n) < 9.0e15`.
fn whole(n: f64) -> Option<i64> {
    (n.is_finite() && n.fract() == 0.0 && n.abs() < 9.0e15).then_some(n as i64)
}

/// `strtod` over text this writer printed.
fn parse_c(s: &str) -> Option<f64> {
    match s {
        "inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        _ => s.parse().ok(),
    }
}

/// Whether a string is written in a long bracket (`WantsLongBracket`): it holds a newline and
/// nothing the lexer would normalise away (`\r`) or that needs an escape anyway.
#[must_use]
pub fn wants_long_bracket(s: &str) -> bool {
    let mut nl = false;
    for &c in s.as_bytes() {
        if c == b'\n' {
            nl = true;
        } else if c == b'\r' || (c < 0x20 && c != b'\t') {
            return false;
        }
    }
    nl
}

/// Appends a string in double quotes (`QuoteString`).
pub fn quote_into(out: &mut String, s: &str) {
    out.push('"');
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 || c as u32 == 0x7f => {
                out.push_str(&format!("\\{:03}", c as u32));
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

/// A string in double quotes, escaped (`QuoteString`).
#[must_use]
pub fn quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    quote_into(&mut out, s);
    out
}

/// Appends a string in a long bracket whose closer the text does not hold (`LongString`).
pub fn long_string_into(out: &mut String, s: &str) {
    let mut level = usize::from(s.ends_with(']'));
    loop {
        let close = format!("]{}]", "=".repeat(level));
        if !s.contains(&close) {
            break;
        }
        level += 1;
    }
    let eq = "=".repeat(level);
    out.push('[');
    out.push_str(&eq);
    out.push_str("[\n");
    out.push_str(s);
    out.push(']');
    out.push_str(&eq);
    out.push(']');
}

/// Appends a key: bare when it is a name, `["..."]` otherwise (`WriteKey`).
pub fn key_into(out: &mut String, k: &str) {
    if is_ident(k) {
        out.push_str(k);
        return;
    }
    out.push('[');
    quote_into(out, k);
    out.push(']');
}

/// A key as the file writes it.
#[must_use]
pub fn key(k: &str) -> String {
    let mut out = String::with_capacity(k.len() + 4);
    key_into(&mut out, k);
    out
}

/// Appends the single-line form (`Inline`). False when a value needs a long bracket; what was
/// appended up to that point stays, as in the C++.
pub fn inline_into(out: &mut String, v: &Value) -> bool {
    match v {
        Value::Nil => out.push_str("nil"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => out.push_str(&format_number(*n)),
        Value::String(s) => {
            if wants_long_bracket(s) {
                return false;
            }
            quote_into(out, s);
        }
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
                key_into(out, k);
                out.push_str(" = ");
                if !inline_into(out, x) {
                    return false;
                }
            }
            out.push_str(" }");
        }
    }
    true
}

/// The single-line form, or `None` when a value needs a long bracket.
#[must_use]
pub fn inline(v: &Value) -> Option<String> {
    let mut out = String::new();
    inline_into(&mut out, v).then_some(out)
}

fn indent(out: &mut String, n: usize) {
    out.extend(std::iter::repeat_n(' ', n));
}

/// Appends the pretty form (`Pretty`): inline when it fits in what is left of the line, one
/// entry per line otherwise.
pub fn pretty_into(out: &mut String, v: &Value, ind: usize, first_line_used: usize) {
    let Value::Table(t) = v else {
        if let Value::String(s) = v
            && wants_long_bracket(s)
        {
            long_string_into(out, s);
            return;
        }
        inline_into(out, v);
        return;
    };
    let mut one = String::new();
    if inline_into(&mut one, v) && first_line_used + one.len() <= WIDTH {
        out.push_str(&one);
        return;
    }
    if t.is_empty() {
        out.push_str("{}");
        return;
    }
    out.push_str("{\n");
    for x in &t.arr {
        indent(out, ind + 2);
        pretty_into(out, x, ind + 2, ind + 2);
        out.push_str(",\n");
    }
    for (k, x) in &t.fields {
        indent(out, ind + 2);
        let kk = key(k);
        out.push_str(&kk);
        out.push_str(" = ");
        pretty_into(out, x, ind + 2, ind + 2 + kk.len() + 3);
        out.push_str(",\n");
    }
    indent(out, ind);
    out.push('}');
}

/// Any value as Lua text at `indent` spaces (`NqLua::WriteValue`).
#[must_use]
pub fn write_value(v: &Value, indent: usize) -> String {
    let mut out = String::new();
    pretty_into(&mut out, v, indent, indent);
    out
}

/// Appends a node table in the node layout (`WriteNodeAt`): `{ id = .., kind = ..[, once = ..],`
/// then one field per line, the closing brace on the last field's line; a node whose inline
/// form fits stays on one line. The array part of the table is not written.
pub fn node_into(out: &mut String, nv: &Value, ind: usize) {
    let mut one = String::new();
    if inline_into(&mut one, nv) && ind + one.len() <= WIDTH {
        out.push_str(&one);
        return;
    }
    let fields = nv.fields();
    out.push_str("{ ");
    let mut i = 0;
    while let Some((k, x)) = fields.get(i) {
        if k != "id" && k != "kind" && k != "once" {
            break;
        }
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(k);
        out.push_str(" = ");
        inline_into(out, x);
        i += 1;
    }
    if i >= fields.len() {
        out.push_str(" }");
        return;
    }
    out.push_str(",\n");
    while let Some((k, x)) = fields.get(i) {
        indent(out, ind + 2);
        let kk = key(k);
        out.push_str(&kk);
        out.push_str(" = ");
        pretty_into(out, x, ind + 2, ind + 2 + kk.len() + 3);
        out.push_str(if i + 1 < fields.len() { ",\n" } else { " }" });
        i += 1;
    }
}

/// A node table as Lua text (`NqLua::WriteNode`, given the node already as a value).
#[must_use]
pub fn write_node(nv: &Value, indent: usize) -> String {
    let mut out = String::new();
    node_into(&mut out, nv, indent);
    out
}

/// Text on one line: newlines and tabs become spaces (`OneLine`).
#[must_use]
pub fn one_line(s: &str) -> String {
    s.chars().map(|c| if matches!(c, '\n' | '\r' | '\t') { ' ' } else { c }).collect()
}

/// Every line of `text` as a `-- ` comment line (the outline header of `NqLua::Write`).
#[must_use]
pub fn comment_block(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    let mut pos = 0;
    while pos < text.len() {
        let eol = text.get(pos..).and_then(|r| r.find('\n')).map_or(text.len(), |e| pos + e);
        out.push_str("-- ");
        out.push_str(text.get(pos..eol).unwrap_or(""));
        out.push('\n');
        pos = eol + 1;
    }
    out
}

/// A whole asset file as `NqLua::Write` lays it out: `head` (the comment header, written as
/// given), then `return {`, one top-level key per line - `nodes` as a list with a blank line
/// between nodes, `tasks` one task per line when it does not fit - and `}`.
///
/// Top-level keys are written as they are, without brackets, as the C++ does.
#[must_use]
pub fn write_asset(head: &str, top: &Table) -> String {
    let mut out = String::with_capacity(head.len() + 4096);
    out.push_str(head);
    out.push_str("return {\n");
    for (k, v) in &top.fields {
        out.push_str("  ");
        out.push_str(k);
        out.push_str(" = ");
        if k == "nodes" {
            out.push_str("{\n");
            for (n, nv) in v.arr().iter().enumerate() {
                if n > 0 {
                    out.push('\n');
                }
                indent(&mut out, 4);
                node_into(&mut out, nv, 4);
                out.push_str(",\n");
            }
            out.push_str("  }");
        } else if k == "tasks" {
            let mut one = String::new();
            if inline_into(&mut one, v) && 2 + k.len() + 3 + one.len() <= WIDTH {
                out.push_str(&one);
            } else {
                out.push_str("{\n");
                for (tk, tv) in v.fields() {
                    indent(&mut out, 4);
                    let kk = key(tk);
                    out.push_str(&kk);
                    out.push_str(" = ");
                    pretty_into(&mut out, tv, 4, 4 + kk.len() + 3);
                    out.push_str(",\n");
                }
                out.push_str("  }");
            }
        } else {
            pretty_into(&mut out, v, 2, 2 + k.len() + 3);
        }
        out.push_str(",\n");
    }
    out.push_str("}\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(pairs: &[(&str, Value)]) -> Value {
        let mut t = Table::new();
        for (k, v) in pairs {
            t.put(k, v.clone());
        }
        Value::Table(t)
    }

    #[test]
    fn numbers() {
        assert_eq!(format_number(3.0), "3");
        assert_eq!(format_number(-0.0), "0");
        assert_eq!(format_number(12.5), "12.5");
        assert_eq!(format_number(f64::from(0.1f32)), "0.1");
        assert_eq!(format_number(0.1), "0.1");
        assert_eq!(format_number(1.0 / 3.0), "0.33333333333333331");
        assert_eq!(format_number(1e20), "1e+20");
        assert_eq!(format_number(f64::INFINITY), "inf");
        assert_eq!(number_to_text(1.0 / 3.0), "0.333333333333333");
        assert_eq!(number_to_text(9.0e15), "9e+15");
    }

    #[test]
    fn strings_and_keys() {
        assert_eq!(quote("a\"b\\c\nd\te\r\u{1}\u{7f}é"), "\"a\\\"b\\\\c\\nd\\te\\r\\001\\127é\"");
        assert!(wants_long_bracket("a\nb"));
        assert!(!wants_long_bracket("a\r\nb"));
        assert!(!wants_long_bracket("a\u{1}\nb"));
        let mut s = String::new();
        long_string_into(&mut s, "x\n]]y]");
        assert_eq!(s, "[=[\nx\n]]y]]=]");
        let mut s = String::new();
        long_string_into(&mut s, "a]]\nb]=]");
        assert_eq!(s, "[==[\na]]\nb]=]]==]");
        assert_eq!(key("kind"), "kind");
        assert_eq!(key("end"), "[\"end\"]");
        assert_eq!(key("1a"), "[\"1a\"]");
        assert_eq!(key("a-b"), "[\"a-b\"]");
        assert!(is_ident("_x9"));
        assert!(!is_ident(""));
    }

    #[test]
    fn pretty_and_inline() {
        let v = t(&[("kind", "x".into()), ("params", t(&[("a", 1.into()), ("b", true.into())]))]);
        assert_eq!(write_value(&v, 0), "{ kind = \"x\", params = { a = 1, b = true } }");
        assert_eq!(write_value(&Value::table(), 2), "{}");
        let long = "a".repeat(95);
        let v = t(&[("text", long.as_str().into()), ("n", 2.into())]);
        assert_eq!(write_value(&v, 0), format!("{{\n  text = \"{long}\",\n  n = 2,\n}}"));
        let v = t(&[("text", "line1\nline2".into())]);
        assert_eq!(write_value(&v, 0), "{\n  text = [[\nline1\nline2]],\n}");
        assert_eq!(inline(&v), None);
        let list: Value = vec![Value::from(1), Value::from("x")].into();
        assert_eq!(write_value(&list, 0), "{ 1, \"x\" }");
    }

    #[test]
    fn node_layout() {
        let n = t(&[("id", "a".into()), ("kind", "k".into())]);
        assert_eq!(write_node(&n, 4), "{ id = \"a\", kind = \"k\" }");
        let text = "t".repeat(90);
        let n = t(&[
            ("id", "a".into()),
            ("kind", "dialog.say".into()),
            ("once", true.into()),
            ("params", t(&[("text", text.as_str().into())])),
            ("out", t(&[("next", "b".into())])),
        ]);
        assert_eq!(
            write_node(&n, 4),
            format!(
                "{{ id = \"a\", kind = \"dialog.say\", once = true,\n      params = {{\n        text = \"{text}\",\n      }},\n      out = {{ next = \"b\" }} }}"
            )
        );
    }

    #[test]
    fn asset_layout() {
        let mut top = Table::new();
        top.put("nq", 1);
        top.put("id", "q");
        top.put("tasks", t(&[("t1", t(&[("title", "T".into()), ("type", "additional".into())]))]));
        let nodes: Value = vec![
            t(&[("id", "a".into()), ("kind", "trigger.start".into())]),
            t(&[("id", "b".into()), ("kind", "flow.end".into())]),
        ]
        .into();
        top.put("nodes", nodes);
        let head = comment_block("quest q \"\"\na (trigger.start)\n");
        assert_eq!(head, "-- quest q \"\"\n-- a (trigger.start)\n");
        assert_eq!(
            write_asset(&head, &top),
            "-- quest q \"\"\n-- a (trigger.start)\nreturn {\n  nq = 1,\n  id = \"q\",\n  tasks = { t1 = { title = \"T\", type = \"additional\" } },\n  nodes = {\n    { id = \"a\", kind = \"trigger.start\" },\n\n    { id = \"b\", kind = \"flow.end\" },\n  },\n}\n"
        );
    }
}
