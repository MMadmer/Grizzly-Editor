//! Declarative Lua data: the editor's own assets (`.nqasset`, `.behasset`, `.npcasset`,
//! `.pdaasset`, a scene's data files) are `return { ... }` tables that carry data and never
//! code. This crate reads them as the game does, keeps them as lossless documents and writes
//! new ones canonically.
//!
//! # API
//!
//! - [`Value`] / [`Table`]: what a file holds. A table keeps its array part in
//!   order and its string keys in the canonical order ([`key_cmp`]); typed reads
//!   (`to_text`, `to_number`, `to_bool`, `get_*`), `set` / `put` / `erase` / `push`,
//!   `canonicalize`, `patch`. `serde::Serialize` gives the JSON shape [`to_json`] gives.
//! - [`parse`] / [`parse_named`]: text to value, accepting exactly what the game's reader
//!   accepts and refusing with its message, at the exact line and column ([`LuaDataError`]).
//!   The chunk name is the file's base name (`"nqasset"` when empty); it matters only to a
//!   name longer than 59 bytes, which LuaJIT cuts short in its messages.
//! - [`LuaDoc`]: a file as a lossless document - its text byte for byte, its value, positions
//!   of everything ([`Span`], [`Seg`] paths), edits that touch only their bytes and
//!   [`LuaDoc::apply_diff`] / [`LuaDoc::rebase`], with the reader's [`Limits`].
//! - [`syntax_check`]: compiles custom Lua (a `lua` action's code) without running it.
//! - The canonical writer of new files and fragments ([`write`]): [`write_value`],
//!   [`write_node`], [`write_asset`], [`one_line`].
//! - Helpers: [`quote`], [`unquote`], [`is_ident`], [`is_keyword`], [`format_number`],
//!   [`number_to_text`], [`format_g`].
//! - [`to_json`] / [`from_json`].
//!
//! # How a file is read
//!
//! The game runs the text through LuaJIT in an empty environment. Here the same three steps are
//! modelled: the compile is LuaJIT 2.1's lexer and parser (`lex`, `parse`: every syntax message,
//! the nesting, local, upvalue and register limits); then the declarative scan (`decl`); then
//! the run of what is left - constructors, literals and minus signs - with the interpreter's
//! semantics (`eval`): template versus run-time fields, strings coerced in arithmetic, the
//! first failing operation's message and line, and the walk's limits (nesting 48, 200000
//! values).
//!
//! # Not certainly equal to LuaJIT
//!
//! - The instruction count behind `execution limit exceeded` (4,000,000 instructions) is
//!   modelled per instruction and may be off by one; no file the editor writes comes near it.
//! - Outside the data subset (arbitrary code given to [`syntax_check`]) the register model
//!   follows LuaJIT's allocator but not its jump lists bit for bit; the limits on bytecode size
//!   and jump distance (`control structure too long`) are not checked.
//! - A bytecode dump (text starting with ESC) is refused without being decoded.
//! - Lua strings are bytes; a string that is not UTF-8 (a `"\xff"` escape) is kept lossily.

#![forbid(unsafe_code)]

mod decl;
mod doc;
mod eval;
pub mod json;
mod lex;
mod num;
mod parse;
pub mod path;
pub mod value;
pub mod write;

pub use doc::{Conflict, DiffRules, Limits, LuaDoc, Slot, Span};
pub use json::{from_json, to_json};
pub use num::format_g;
pub use path::{Seg, covers, data_eq, get_at, set_at};
pub use value::{Table, Value, key_cmp, key_rank};
pub use write::{
    comment_block, format_number, inline, is_ident, is_keyword, number_to_text, one_line, quote, write_asset,
    write_node, write_value,
};

use eval::{Outcome, RunErr};
use parse::compile;

/// Why a text is not a declarative Lua file: the message the author reads, the 1-based line
/// and column (counted in characters) of what it is about; 0 only for a refusal that is about
/// no place (the bytecode loader's).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct LuaDataError {
    pub line: u32,
    pub column: u32,
    pub message: String,
}

/// Decodes one Lua string literal (`"..."`, `'...'`, `[[...]]`) as the reader does; `None`
/// when the text is not exactly one literal.
#[must_use]
pub fn unquote(literal: &str) -> Option<String> {
    lex::unescape(literal).map(|b| String::from_utf8_lossy(&b).into_owned())
}

/// The name as C sees it: up to the first NUL.
fn c_name<'a>(name: &'a str, default: &'a str) -> &'a [u8] {
    let b = name.as_bytes();
    let b = b.split(|&c| c == 0).next().unwrap_or(&[]);
    if b.is_empty() { default.as_bytes() } else { b }
}

/// `lj_debug_shortname` of `=name`: the first 59 bytes.
fn shortname(chunk: &[u8]) -> &[u8] {
    chunk.get(..59).unwrap_or(chunk)
}

fn located(chunk: &[u8], line: u32, msg: &[u8]) -> Vec<u8> {
    let mut raw = shortname(chunk).to_vec();
    raw.extend(format!(":{line}: ").into_bytes());
    raw.extend_from_slice(msg);
    raw
}

/// The line and the message of a LuaJIT error, without its `name:line:` prefix.
fn split_lua_error(raw: &[u8], chunk: &[u8]) -> (u32, Vec<u8>) {
    let raw = raw.split(|&c| c == 0).next().unwrap_or(&[]);
    let mut p = 0;
    if !chunk.is_empty() && raw.starts_with(chunk) {
        p = chunk.len();
    }
    if p == 0
        && raw.first() == Some(&b'[')
        && let Some(close) = raw.windows(2).position(|w| w == b"\"]")
    {
        p = close + 2;
    }
    if raw.get(p) == Some(&b':') {
        // strtol: spaces, a sign, digits
        let mut q = p + 1;
        while raw.get(q).is_some_and(|c| matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')) {
            q += 1;
        }
        let mut neg = false;
        if let Some(&s @ (b'+' | b'-')) = raw.get(q) {
            neg = s == b'-';
            q += 1;
        }
        let start = q;
        let mut line: i64 = 0;
        while let Some(&d) = raw.get(q).filter(|d| d.is_ascii_digit()) {
            line = line.saturating_mul(10).saturating_add(i64::from(d - b'0'));
            q += 1;
        }
        let end = if q == start { p + 1 } else { q };
        let line = if neg { -line } else { line };
        if raw.get(end) == Some(&b':') && line > 0 {
            let mut m = end + 1;
            while raw.get(m) == Some(&b' ') {
                m += 1;
            }
            return (u32::try_from(line).unwrap_or(u32::MAX), raw.get(m..).unwrap_or(&[]).to_vec());
        }
    }
    (0, raw.to_vec())
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

fn bytecode_error(src: &[u8], chunk: &[u8]) -> Vec<u8> {
    let what: &[u8] = if src.len() >= 4 && !src.starts_with(b"\x1bLJ\x02") {
        b"cannot load incompatible bytecode"
    } else {
        b"cannot load malformed bytecode"
    };
    let mut raw = chunk.to_vec();
    raw.extend(b": ");
    raw.extend(what);
    raw
}

/// A refusal of the reader: the message, the line the model reports, and the offset of what it
/// is about where the model knows it.
pub(crate) struct Judged {
    pub at: Option<usize>,
    pub line: u32,
    pub message: String,
}

impl Judged {
    fn plain(message: impl Into<String>) -> Self {
        Self { at: None, line: 0, message: message.into() }
    }
}

/// The compile step: `luaL_loadbuffer` with `=chunk`.
fn compile_step(src: &[u8], chunk: &[u8]) -> Result<parse::Compiled, Judged> {
    let (raw, at) = match compile(src) {
        Ok(Some(c)) => return Ok(c),
        Ok(None) => (bytecode_error(src, chunk), None),
        Err(lex::SynErr::Raw(m)) => (m, None),
        Err(lex::SynErr::At { line, msg, near, at }) => {
            let mut raw = located(chunk, line, &msg);
            if let Some(near) = near {
                raw.extend(b" near '");
                raw.extend(near);
                raw.push(b'\'');
            }
            (raw, Some(at))
        }
    };
    let (line, msg) = split_lua_error(&raw, chunk);
    Err(Judged { at, line, message: lossy(&msg) })
}

/// The reader that judges a file the game reads: the compile, the declarative scan, the run.
pub(crate) fn judge(text: &str, chunk_name: &str) -> Result<Value, Judged> {
    let chunk = c_name(chunk_name, "nqasset");
    let src = text.as_bytes();
    let compiled = compile_step(src, chunk)?;
    if let Some((at, p)) = decl::problem(src) {
        return Err(Judged { at: Some(at), line: 0, message: lossy(&p) });
    }
    match eval::run(&compiled) {
        Ok(Outcome::Value(v)) => Ok(v),
        Ok(Outcome::NotOneTable) => Err(Judged::plain("the file must return one table (return { ... })")),
        Ok(Outcome::Walk(m)) => Err(Judged::plain(m)),
        Err(RunErr::Limit) => {
            Err(Judged::plain("execution limit exceeded (a quest file must be plain data)"))
        }
        Err(RunErr::At { line, msg }) => {
            let raw = located(chunk, line, msg.as_bytes());
            let (line, m) = split_lua_error(&raw, chunk);
            let mut message = lossy(&m);
            if !message.contains("execution limit") {
                message.insert_str(0, "code outside strings: ");
            }
            Err(Judged { at: None, line, message })
        }
    }
}

/// A refusal with its exact place: the offset the model knows, else the place the document's
/// own reader finds for the same refusal. `skip` is what lies before the judged text (a byte
/// order mark).
pub(crate) fn locate(text: &str, skip: usize, j: Judged) -> LuaDataError {
    let at = j.at.map(|a| a + skip).or_else(|| doc::refusal_at(text, skip, &j.message));
    match at {
        Some(a) => {
            let (line, column) = doc::position(text, &doc::line_starts(text.as_bytes()), a);
            LuaDataError { line, column, message: j.message }
        }
        None => LuaDataError { line: j.line, column: 0, message: j.message },
    }
}

/// [`parse_named`] with the default chunk name.
pub fn parse(text: &str) -> Result<Value, LuaDataError> {
    parse_named(text, "")
}

/// Reads a declarative Lua file: compiles it, refuses anything that is not one literal table,
/// runs it and returns the table. A refusal carries the line and column of what it is about.
pub fn parse_named(text: &str, chunk_name: &str) -> Result<Value, LuaDataError> {
    judge(text, chunk_name).map_err(|j| locate(text, 0, j))
}

/// Compiles custom Lua without running it; the line and column are relative to the code.
pub fn syntax_check(code: &str) -> Result<(), LuaDataError> {
    // the code ends at its first NUL, as a C string does
    let src = code.as_bytes().split(|&c| c == 0).next().unwrap_or(&[]);
    compile_step(src, b"code").map(|_| ()).map_err(|j| {
        let text = std::str::from_utf8(src).unwrap_or("");
        match j.at {
            Some(a) => {
                let (line, column) = doc::position(text, &doc::line_starts(text.as_bytes()), a);
                LuaDataError { line, column, message: j.message }
            }
            None => LuaDataError { line: j.line, column: 0, message: j.message },
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(text: &str) -> (u32, u32, String) {
        match parse(text) {
            Ok(v) => panic!("accepted: {v:?}"),
            Err(e) => (e.line, e.column, e.message),
        }
    }

    fn keys(v: &Value) -> Vec<&str> {
        v.fields().iter().map(|(k, _)| k.as_str()).collect()
    }

    #[test]
    fn reads_plain_tables() {
        let v = parse("-- header\nreturn { zeta = 1, name = \"n\", kind = \"x\", 2, 3 } -- tail").unwrap();
        assert_eq!(keys(&v), ["kind", "name", "zeta"]);
        assert_eq!(v.arr(), [Value::from(2), Value::from(3)]);
        let v = parse("return { a = 0x10, b = 1e2, c = .5, d = -2, e = - -3, f = true, g = nil, goto = 1 }")
            .unwrap();
        assert_eq!(v.get_number("a", 0.0), 16.0);
        assert_eq!(v.get_number("b", 0.0), 100.0);
        assert_eq!(v.get_number("c", 0.0), 0.5);
        assert_eq!(v.get_number("d", 0.0), -2.0);
        assert_eq!(v.get_number("e", 0.0), 3.0);
        assert_eq!(v.get("f"), Some(&Value::Bool(true)));
        assert!(!v.has("g"));
        assert!(v.has("goto"));
        let v = parse("return { s = \"a\\tb\\\\\\\"\\65\\x41\\u{416}\", l = [[\nline]], q = 'it''s' }");
        assert!(v.is_err());
        let v =
            parse("return { s = \"a\\tb\\\\\\\"\\65\\x41\\u{416}\", l = [[\nline]], q = 'x\"' }").unwrap();
        assert_eq!(v.get_string("s", ""), "a\tb\\\"AAЖ");
        assert_eq!(v.get_string("l", ""), "line");
        assert_eq!(v.get_string("q", ""), "x\"");
    }

    #[test]
    fn runs_like_the_interpreter() {
        assert_eq!(parse("return { a = -\"5\" }").unwrap().get_number("a", 0.0), -5.0);
        assert_eq!(parse("return { a = \"10\" - 3 }").unwrap().get_number("a", 0.0), 7.0);
        assert!(parse("return { a = -0 }").unwrap().get_number("a", 1.0).is_sign_negative());
        // a computed value is stored after the template: it wins whatever the order
        assert_eq!(parse("return { a = -\"5\", a = 1 }").unwrap().get_number("a", 0.0), -5.0);
        assert_eq!(parse("return { a = 1, a = 2 }").unwrap().get_number("a", 0.0), 2.0);
        assert_eq!(parse("return { [1] = -\"7\", \"x\" }").unwrap().arr(), [Value::from(-7)]);
        let v = parse("return { [2] = \"b\", [1] = \"a\", [-1] = \"z\", [true] = 1 }").unwrap();
        assert_eq!(v.arr(), [Value::from("z"), Value::from("a"), Value::from("b")]);
        assert!(v.fields().is_empty());
    }

    #[test]
    fn syntax_errors_read_as_luajit_writes_them() {
        assert_eq!(err("return { a = }"), (1, 14, "unexpected symbol near '}'".into()));
        assert_eq!(
            err("return {\n a = 1\n b = 2 }"),
            (3, 2, "'}' expected (to close '{' at line 1) near 'b'".into())
        );
        assert_eq!(err("return { a = \"abc }"), (1, 14, "unfinished string near '<eof>'".into()));
        assert_eq!(err("return { a = \"abc\n}"), (1, 14, "unfinished string near '\"abc'".into()));
        assert_eq!(err("return { a = 1 } x"), (1, 18, "'<eof>' expected near 'x'".into()));
        assert_eq!(err("return { \"a\" \"b\" }"), (1, 14, "'}' expected near '\"b\"'".into()));
        assert_eq!(err("return { a = 1abc }"), (1, 14, "malformed number near '1abc'".into()));
        assert_eq!(err("return { a = \"\\q\" }"), (1, 14, "invalid escape sequence near '\"'".into()));
        let deep = format!("return {}{}", "{".repeat(199), "}".repeat(199));
        assert_eq!(err(&deep), (1, 206, "chunk has too many syntax levels".into()));
        let name = "x".repeat(70);
        let e = parse_named("return { a = }", &name).unwrap_err();
        assert_eq!((e.line, e.column), (1, 14));
        assert_eq!(e.message, format!("{}:1: unexpected symbol near '}}'", "x".repeat(59)));
    }

    #[test]
    fn code_is_refused() {
        let code = "'b' is code, not data (only literals and 'key =' names are allowed)";
        assert_eq!(err("return { a = b }"), (1, 14, code.into()));
        assert_eq!(err("local x = 1 return { x }").2, "'local' before the table is code, not data");
        assert_eq!(err("return { a = 1 + 2 }").2, "'+' outside a string is code, not data");
        assert_eq!(err("return { a = 1 // c\n }").2, "'/' outside a string is code, not data");
        assert_eq!(err("").2, "no 'return' statement");
        assert_eq!(err("return 1").2, "the file must return one table (return { ... })");
        assert_eq!(err("return {}, {}").2, "the file must return one table (return { ... })");
        assert_eq!(
            err("return {\n  a = 1,\n  b = -{},\n}"),
            (3, 7, "code outside strings: attempt to perform arithmetic on a table value".into())
        );
        assert_eq!(
            err("return { a = \"x\" - 3 }").2,
            "code outside strings: attempt to perform arithmetic on a string value"
        );
        assert_eq!(
            err("return { a = -nil }").2,
            "code outside strings: attempt to perform arithmetic on a nil value"
        );
        assert_eq!(err("return { [nil] = 1 }"), (1, 10, "code outside strings: table index is nil".into()));
        let nested = |n: usize| format!("return {}{}", "{".repeat(n), "}".repeat(n));
        assert!(parse(&nested(49)).is_ok());
        assert_eq!(err(&nested(50)).2, "table nesting too deep (self reference?)");
    }

    #[test]
    fn checks_code_syntax() {
        assert!(syntax_check("local x = f(1) if x then return x end").is_ok());
        assert!(syntax_check("").is_ok());
        let e = |c: &str| syntax_check(c).map_err(|e| (e.line, e.message)).unwrap_err();
        assert_eq!(e("local x = "), (1, "unexpected symbol near '<eof>'".into()));
        assert_eq!(e("::a:: ::a::"), (1, "duplicate label 'a'".into()));
        assert_eq!(e("break"), (1, "no loop to break".into()));
        assert_eq!(e("goto x"), (1, "undefined label 'x'".into()));
        assert_eq!(e("x = = 1"), (1, "unexpected symbol near '='".into()));
        assert_eq!(e("a\n(f)()"), (2, "ambiguous syntax (function call x new statement) near '('".into()));
        assert_eq!(e("for i do end"), (1, "'=' or 'in' expected near 'do'".into()));
        assert_eq!(e("function f(a,) end"), (1, "<name> or '...' expected near ')'".into()));
        assert!(syntax_check("return ...").is_ok());
        assert_eq!(
            e("function f() return ... end"),
            (1, "cannot use '...' outside a vararg function near '...'".into())
        );
        assert!(syntax_check("do local x end goto l ::l::").is_ok());
        assert_eq!(
            e("goto l local x ::l:: print(x)"),
            (1, "<goto l> jumps into the scope of local 'x'".into())
        );
    }

    #[test]
    fn unquotes_literals() {
        assert_eq!(unquote("\"a\\nb\""), Some("a\nb".into()));
        assert_eq!(unquote("[==[\nx]]==]"), Some("x]".into()));
        assert_eq!(unquote("\"a\" .. \"b\""), None);
    }
}
