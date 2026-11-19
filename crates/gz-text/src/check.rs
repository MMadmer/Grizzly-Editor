//! A text file's problems, with positions: a script's syntax as LuaJIT's own parser reads it,
//! what the module build says of its code, XML and JSON that do not read. The same list is the
//! Text tab's Problems and `asset.check` of the file.
//!
//! The codes: `T001` a script that does not compile (an error), `T010` code the game's protected
//! mode refuses (a warning: the Mod Browser lists no module that has it), `T011` code the Mod
//! Browser's review looks at (a note), `T020` XML that does not read (an error), `T030` JSON that
//! does not read (an error). That the code works with the network is a fact of the module, never
//! a problem of a file.

use std::sync::Arc;

use futures::future::BoxFuture;
use gz_base::{Problem, RelPath};
use gz_editor::Editor;

use crate::file::{MAX_BYTES, decode};
use crate::lex;

/// The extensions whose files have a check: `asset.check` reads them with it.
pub const CHECKED: &[&str] = &["script", "lua", "s", "xml", "xmlp", "html", "htm", "json"];

/// How many places of an ambiguous token are told apart by compiling the text up to each.
const MAX_CANDIDATES: usize = 8;

fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

/// The problems of a text, by the file's name (its extension); they name no file.
#[must_use]
pub fn check_text(name: &str, text: &str) -> Vec<Problem> {
    match extension(name).as_str() {
        "script" | "lua" | "s" => {
            let mut out = lua_syntax(text);
            out.extend(code_findings(text));
            out
        }
        "xml" => xml_problems(text),
        "xmlp" if first_is_tag(text) => xml_problems(text),
        "html" | "htm" if declares_xml(text) => xml_problems(text),
        "json" => json_problems(text),
        _ => Vec::new(),
    }
}

/// The problems of a file of the project, read from the disk, in that file.
pub async fn check_file(editor: &Arc<Editor>, rel: &RelPath) -> Vec<Problem> {
    let Ok(session) = editor.project() else { return Vec::new() };
    let path = session.fs(rel);
    let name = rel.file_name().to_owned();
    let read = editor
        .exec
        .io(move || {
            let meta = std::fs::metadata(&path).ok()?;
            if !meta.is_file() || meta.len() > MAX_BYTES {
                return None;
            }
            let bytes = std::fs::read(&path).ok()?;
            decode(&bytes, &name).ok().map(|f| (f.text, name))
        })
        .await;
    let Some((text, name)) = read else { return Vec::new() };
    let found = editor.exec.cpu(move || check_text(&name, &text)).await;
    found.into_iter().map(|p| p.in_file(rel.clone())).collect()
}

fn check_hook(editor: Arc<Editor>, rel: RelPath) -> BoxFuture<'static, Vec<Problem>> {
    Box::pin(async move { check_file(&editor, &rel).await })
}

/// Puts these checks into `asset.check`.
pub(crate) fn install(editor: &Arc<Editor>) {
    gz_asset::Kinds::of(editor).add_text_check(CHECKED, check_hook);
}

// ---- positions ---------------------------------------------------------------------------------

fn to_u32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The 1-based line and character column of a byte of `text`.
fn place_of(text: &str, at: usize) -> (u32, u32) {
    let at = (0..=at.min(text.len())).rev().find(|&i| text.is_char_boundary(i)).unwrap_or(0);
    let before = text.get(..at).unwrap_or("");
    let line = before.bytes().filter(|&b| b == b'\n').count() + 1;
    let start = before.rfind('\n').map_or(0, |n| n + 1);
    let column = before.get(start..).map_or(0, |l| l.chars().count()) + 1;
    (to_u32(line), to_u32(column))
}

/// The byte where a 1-based line starts.
fn line_start(text: &str, line: u32) -> Option<usize> {
    if line <= 1 {
        return Some(0);
    }
    text.match_indices('\n').nth(usize::try_from(line).ok()? - 2).map(|(i, _)| i + 1)
}

fn line_text(text: &str, line: u32) -> &str {
    let Some(start) = line_start(text, line) else { return "" };
    let rest = text.get(start..).unwrap_or("");
    rest.split('\n').next().unwrap_or("")
}

// ---- Lua ---------------------------------------------------------------------------------------

/// Which lines of the text the fork's lines are: the fork counts no line inside a `/* */`
/// comment, so each one that spans lines folds them into one. Per fork line (from 1), the text's
/// first and last line.
fn fork_lines(folded: &[(u32, u32)], total: u32) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    let mut regions = folded.iter().peekable();
    let mut line = 1;
    while line <= total {
        let mut end = line;
        while let Some(&&(a, b)) = regions.peek() {
            if a <= end {
                end = end.max(b);
                regions.next();
            } else {
                break;
            }
        }
        out.push((line, end));
        line = end + 1;
    }
    out
}

/// A compile of `src` with LuaJIT's own parser: the error message, or `None` when it reads.
/// Nothing is run, and a binary chunk is never loaded.
fn compile(src: &str) -> Option<String> {
    let lua = mlua::Lua::new_with(mlua::StdLib::NONE, mlua::LuaOptions::default()).ok()?;
    let vanilla = gz_lua::vanilla_source(src);
    match lua.load(vanilla.as_bytes()).set_name("=t").set_mode(mlua::chunk::ChunkMode::Text).into_function() {
        Ok(_) => None,
        Err(mlua::Error::SyntaxError { message, .. }) => Some(message),
        Err(e) => Some(format!("t:1: {e}")),
    }
}

/// `t:12: '=' expected near 'x'` as its line and its message.
fn split_message(message: &str) -> Option<(u32, &str)> {
    let rest = message.strip_prefix("t:")?;
    let (line, text) = rest.split_once(':')?;
    Some((line.trim().parse().ok()?, text.trim()))
}

/// The token a message is about: what stands in its last `near '...'`.
fn near_token(text: &str) -> Option<&str> {
    let at = text.rfind(" near '")?;
    let token = text.get(at + 7..)?;
    token.strip_suffix('\'')
}

/// The lines of a message (`(to close 'function' at line 3)`) as the text's lines.
fn remap_lines(text: &str, forks: &[(u32, u32)]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("at line ") {
        let (head, tail) = rest.split_at(at + 8);
        out.push_str(head);
        let digits = tail.bytes().take_while(u8::is_ascii_digit).count();
        let (num, after) = tail.split_at(digits);
        match num.parse::<usize>().ok().and_then(|n| forks.get(n.checked_sub(1)?)) {
            Some((first, _)) => out.push_str(&first.to_string()),
            None => out.push_str(num),
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// A script's syntax error (`T001`), at its line and, when the token it is near is found there,
/// its column.
fn lua_syntax(text: &str) -> Vec<Problem> {
    // a CR alone is a line break to Lua and none to the editor: a blank keeps the lines one
    let prepared: std::borrow::Cow<'_, str> = if text.contains('\r') {
        let mut s = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        while let Some(c) = chars.next() {
            s.push(if c == '\r' && chars.peek() != Some(&'\n') { ' ' } else { c });
        }
        s.into()
    } else {
        text.into()
    };
    let text = prepared.as_ref();
    let Some(message) = compile(text) else { return Vec::new() };
    let Some((fork_line, said)) = split_message(&message) else {
        return vec![Problem::error("T001", format!("The script does not compile: {message}"))];
    };
    let scan = lex::scan(text);
    let total = to_u32(text.split('\n').count());
    let forks = fork_lines(&scan.folded, total);
    let span = forks
        .get(usize::try_from(fork_line).unwrap_or(0).saturating_sub(1))
        .copied()
        .unwrap_or((fork_line.min(total.max(1)), fork_line.min(total.max(1))));
    let said = remap_lines(said, &forks);
    let near = near_token(&said).map(str::to_owned);
    let sentence = |line: u32| {
        let body = said.replace(" near '<eof>'", " at the end of the file");
        format!("Line {line}: {body}")
    };
    let Some(token) = near else {
        return vec![Problem::error("T001", sentence(span.0)).at_line(span.0)];
    };
    if token == "<eof>" {
        // the end of the file: its last line that holds anything
        let last = (1..=total).rev().find(|&l| !line_text(text, l).trim().is_empty()).unwrap_or(1);
        let column = to_u32(line_text(text, last).chars().count()) + 1;
        return vec![Problem::error("T001", sentence(last)).at(last, column, last, column)];
    }
    let candidates: Vec<&lex::Token<'_>> = scan
        .tokens
        .iter()
        .filter(|t| t.line >= span.0 && t.line <= span.1 && t.text == token)
        .take(MAX_CANDIDATES + 1)
        .collect();
    let at = match candidates.as_slice() {
        [] => (span.0..=span.1).find_map(|l| {
            let start = line_start(text, l)?;
            line_text(text, l).find(token.as_str()).map(|i| start + i)
        }),
        [one] => Some(one.start),
        many if many.len() > MAX_CANDIDATES => many.first().map(|t| t.start),
        // the one the parser stopped at: the first whose text up to it fails the same way
        many => {
            let (mut lo, mut hi) = (0, many.len() - 1);
            while lo < hi {
                let mid = (lo + hi) / 2;
                let end = many.get(mid).map_or(text.len(), |t| t.end);
                if compile(text.get(..end).unwrap_or(text)).as_deref() == Some(message.as_str()) {
                    hi = mid;
                } else {
                    lo = mid + 1;
                }
            }
            many.get(lo).map(|t| t.start)
        }
    };
    let problem = match at {
        Some(at) => {
            let (line, column) = place_of(text, at);
            let width = to_u32(token.chars().count().max(1));
            Problem::error("T001", sentence(line)).at(line, column, line, column + width)
        }
        None => Problem::error("T001", sentence(span.0)).at_line(span.0),
    };
    vec![problem]
}

/// What the module build says of a script's code: what the game's protected mode refuses
/// (`T010`, a warning) and what the Mod Browser's review looks at (`T011`, a note).
fn code_findings(text: &str) -> Vec<Problem> {
    let report = gz_mod::code::report(gz_mod::code::CodeKind::Script, text.as_bytes());
    report
        .findings
        .into_iter()
        .map(|f| {
            let p = if f.blocks {
                Problem::warning(
                    "T010",
                    format!(
                        "Line {} {}, which the game's protected mode refuses: the Mod Browser does not list a module that does.",
                        f.line, f.what
                    ),
                )
            } else {
                Problem::note("T011", format!("Line {} {}: the Mod Browser's review looks at it.", f.line, f.what))
            };
            p.at_line(f.line)
        })
        .collect()
}

// ---- XML ---------------------------------------------------------------------------------------

/// Whether a text's first thing is a tag (a patch of XML that is XML).
fn first_is_tag(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').trim_start().starts_with('<')
}

/// Whether a page says it is XML (`<?xml ...?>` at its top).
fn declares_xml(text: &str) -> bool {
    text.trim_start_matches('\u{feff}').trim_start().starts_with("<?xml")
}

fn name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_' || c == b':' || c >= 0x80
}

fn name_char(c: u8) -> bool {
    c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b':') || c >= 0x80
}

fn xml_error(text: &str, at: usize, message: impl Into<String>) -> Vec<Problem> {
    let (line, column) = place_of(text, at);
    let message = message.into();
    vec![Problem::error("T020", format!("Line {line}: {message}")).at(line, column, line, column + 1)]
}

/// The first place where an XML text does not read as the game's reader (its TinyXML) takes it:
/// a comment, a CDATA section or a declaration that is not closed, a tag cut off by the end, an
/// attribute without a value, a quoted value that is not closed, a character where an attribute
/// should be, a closing tag that closes another element or nothing, an element left open. What
/// that reader lets pass (text outside the root, an unquoted value, a stray `<`) is no problem.
fn xml_problems(text: &str) -> Vec<Problem> {
    let b = text.as_bytes();
    let find = |from: usize, what: &str| text.get(from..).and_then(|r| r.find(what)).map(|n| from + n);
    let mut open: Vec<(&str, usize)> = Vec::new();
    let mut i = 0;
    while let Some(lt) = find(i, "<") {
        let rest = text.get(lt..).unwrap_or("");
        if rest.starts_with("<!--") {
            match find(lt + 4, "-->") {
                Some(end) => i = end + 3,
                None => return xml_error(text, lt, "a comment is not closed."),
            }
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            match find(lt + 9, "]]>") {
                Some(end) => i = end + 3,
                None => return xml_error(text, lt, "a CDATA section is not closed."),
            }
            continue;
        }
        if rest.starts_with("<?") {
            match find(lt + 2, "?>") {
                Some(end) => i = end + 2,
                None => return xml_error(text, lt, "a declaration is not closed."),
            }
            continue;
        }
        if rest.starts_with("<!") {
            match find(lt + 2, ">") {
                Some(end) => i = end + 1,
                None => return xml_error(text, lt, "a declaration is not closed."),
            }
            continue;
        }
        let closing = rest.starts_with("</");
        let name_at = lt + if closing { 2 } else { 1 };
        if !b.get(name_at).is_some_and(|&c| name_start(c)) {
            // a '<' that starts no tag is read as text
            i = lt + 1;
            continue;
        }
        let mut j = name_at;
        while b.get(j).is_some_and(|&c| name_char(c)) {
            j += 1;
        }
        let name = text.get(name_at..j).unwrap_or("");
        if closing {
            while b.get(j).is_some_and(u8::is_ascii_whitespace) {
                j += 1;
            }
            if b.get(j) != Some(&b'>') {
                return xml_error(text, lt, format!("the closing tag </{name}> is not finished with '>'."));
            }
            match open.last() {
                Some(&(top, _)) if top == name => {
                    open.pop();
                }
                Some(&(top, at)) => {
                    let (line, _) = place_of(text, at);
                    return xml_error(
                        text,
                        lt,
                        format!("</{name}> closes <{top}> of line {line}: the tags are not nested."),
                    );
                }
                None => return xml_error(text, lt, format!("the closing tag </{name}> closes nothing.")),
            }
            i = j + 1;
            continue;
        }
        // the attributes, up to '>' or '/>', as the game's reader takes them
        let cut = || xml_error(text, lt, format!("the tag <{name}> is cut off by the end of the file."));
        let skip_blanks = |mut j: usize| {
            while b.get(j).is_some_and(u8::is_ascii_whitespace) {
                j += 1;
            }
            j
        };
        loop {
            j = skip_blanks(j);
            match b.get(j) {
                None => return cut(),
                Some(b'>') => {
                    open.push((name, lt));
                    j += 1;
                    break;
                }
                Some(b'/') if b.get(j + 1) == Some(&b'>') => {
                    j += 2;
                    break;
                }
                Some(&c) if c.is_ascii_alphabetic() || c == b'_' || c >= 0x80 => {
                    let from = j;
                    while b.get(j).is_some_and(|&c| name_char(c)) {
                        j += 1;
                    }
                    let attr = text.get(from..j).unwrap_or("");
                    j = skip_blanks(j);
                    if b.get(j) != Some(&b'=') {
                        return xml_error(
                            text,
                            from,
                            format!("the attribute '{attr}' of <{name}> has no value."),
                        );
                    }
                    j = skip_blanks(j + 1);
                    match b.get(j) {
                        None => return cut(),
                        Some(&q @ (b'"' | b'\'')) => {
                            let quote = if q == b'"' { "\"" } else { "'" };
                            match find(j + 1, quote) {
                                Some(end) => j = end + 1,
                                None => {
                                    return xml_error(
                                        text,
                                        j,
                                        format!("the value of '{attr}' in <{name}> is not closed."),
                                    );
                                }
                            }
                        }
                        // a value without quotes, which the game's reader takes - up to a blank
                        // or the tag's end, and never with a quote in it
                        Some(_) => {
                            while let Some(&c) = b.get(j) {
                                if c.is_ascii_whitespace() || c == b'/' || c == b'>' {
                                    break;
                                }
                                if c == b'"' || c == b'\'' {
                                    return xml_error(
                                        text,
                                        j,
                                        format!(
                                            "the value of '{attr}' in <{name}> ends in a quote it does not start with."
                                        ),
                                    );
                                }
                                j += 1;
                            }
                        }
                    }
                }
                Some(_) => {
                    let ch = text.get(j..).and_then(|r| r.chars().next()).unwrap_or('?');
                    return xml_error(
                        text,
                        j,
                        format!(
                            "'{ch}' stands in the tag <{name}> where an attribute or the tag's end should."
                        ),
                    );
                }
            }
        }
        i = j;
    }
    match open.last() {
        Some(&(name, at)) => xml_error(text, at, format!("the element <{name}> is not closed.")),
        None => Vec::new(),
    }
}

// ---- JSON --------------------------------------------------------------------------------------

/// JSON that does not read (`T030`), where the reader stopped.
fn json_problems(text: &str) -> Vec<Problem> {
    let Err(e) = serde_json::from_str::<serde_json::Value>(text.trim_start_matches('\u{feff}')) else {
        return Vec::new();
    };
    let line = to_u32(e.line().max(1));
    let said = e.to_string();
    let said = said.rsplit_once(" at line ").map_or(said.as_str(), |(head, _)| head);
    // the reader counts bytes; the editor counts characters
    let row = line_text(text, line);
    let byte = e.column().saturating_sub(1).min(row.len());
    let byte = (0..=byte).rev().find(|&i| row.is_char_boundary(i)).unwrap_or(0);
    let column = to_u32(row.get(..byte).map_or(0, |r| r.chars().count()) + 1);
    vec![Problem::error("T030", format!("Line {line}: {said}.")).at(line, column, line, column + 1)]
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn one(name: &str, text: &str) -> Problem {
        let found = check_text(name, text);
        assert_eq!(found.len(), 1, "{found:?}");
        found.into_iter().next().unwrap()
    }

    #[test]
    fn a_scripts_syntax_error_is_at_its_line_and_column() {
        let p = one("a.script", "local a = 1\nfunction f(x)\n  return x +\nend\n");
        assert_eq!((p.code.as_str(), p.line, p.column), ("T001", Some(4), Some(1)), "{p:?}");
        assert!(p.is_error());
        assert_eq!(p.message, "Line 4: unexpected symbol near 'end'");
        assert_eq!(p.end_column, Some(4));
        assert!(!p.message.contains("[string"), "{}", p.message);
        // the token twice on its line: the one the parser stopped at
        let p = one("a.script", "local a = 1 b = = 2\n");
        assert_eq!((p.line, p.column), (Some(1), Some(17)), "{p:?}");
    }

    #[test]
    fn the_end_of_the_file_is_its_last_line() {
        let p = one("a.lua", "function f()\n  return 1\n\n\n");
        assert_eq!(p.line, Some(2), "{p:?}");
        assert!(p.message.contains("at the end of the file"), "{}", p.message);
        assert!(p.message.contains("at line 1"), "{}", p.message);
    }

    #[test]
    fn the_forks_comments_read_and_keep_the_lines() {
        assert!(
            check_text("a.script", "// a comment\nlocal a = 1 /* and\nanother */ local b = 2\n").is_empty()
        );
        // a block comment over three lines: the fork's line is the text's line
        let p = one("a.script", "/* one\ntwo\nthree */\nlocal x = = 1\n");
        assert_eq!((p.line, p.column), (Some(4), Some(11)), "{p:?}");
        let p = one("a.script", "local a /* x\ny */ = = 2\n");
        assert_eq!((p.line, p.column), (Some(2), Some(8)), "{p:?}");
    }

    #[test]
    fn a_script_is_compiled_never_run() {
        assert!(check_text("a.script", "os.exit(1)\nwhile true do end\n").iter().all(|p| !p.is_error()));
        // a binary chunk is refused as text, never loaded
        let p = one("a.lua", "\x1bLJ\x02junk");
        assert!(p.is_error());
    }

    #[test]
    fn what_the_build_says_of_code_is_at_its_line() {
        let found = check_text("a.script", "local a = 1\nos.execute('x')\nlocal t = debug.traceback()\n");
        let t010 = found.iter().find(|p| p.code == "T010").unwrap();
        assert_eq!((t010.line, t010.severity), (Some(2), gz_base::Severity::Warning));
        assert!(t010.message.starts_with("Line 2 uses os.execute"), "{}", t010.message);
        let t011 = found.iter().find(|p| p.code == "T011").unwrap();
        assert_eq!((t011.line, t011.severity), (Some(3), gz_base::Severity::Note));
        // the network is a fact, never a problem
        assert!(check_text("a.script", "xms.http.get('https://example.com', f)\n").is_empty());
    }

    #[test]
    fn xml_that_does_not_read() {
        assert!(
            check_text("a.xml", "<?xml version=\"1.0\"?>\n#include \"x.xml\"\n<w><a b=c/></w>\n").is_empty()
        );
        let p = one("a.xml", "<w>\n  <a>\n</w>\n");
        assert_eq!((p.code.as_str(), p.line, p.column), ("T020", Some(3), Some(1)), "{p:?}");
        let p = one("a.xml", "<w>\n  <!-- open\n</w>\n");
        assert_eq!((p.line, p.column), (Some(2), Some(3)));
        let p = one("a.xml", "<w a=\"x>\n</w>\n");
        assert_eq!((p.line, p.column), (Some(1), Some(6)));
        // a quote too many, an attribute without a value: the game's reader refuses both
        let p = one("a.xml", "<w>\n  <a id=\"x\"\"></a>\n</w>\n");
        assert_eq!((p.line, p.column), (Some(2), Some(12)), "{p:?}");
        assert!(p.message.contains("where an attribute or the tag's end should"), "{}", p.message);
        let p = one("a.xml", "<w checked></w>");
        assert!(p.message.contains("'checked' of <w> has no value"), "{}", p.message);
        assert_eq!(one("a.xml", "<w a=b\"></w>").column, Some(7));
        let p = one("a.xml", "<w>\n  <a/>\n");
        assert_eq!((p.line, p.column), (Some(1), Some(1)));
        assert!(p.message.contains("<w> is not closed"), "{}", p.message);
        let p = one("a.xml", "<w></w>\n</x>");
        assert_eq!(p.line, Some(2));
        // a page that is no XML is not read as XML; a patch that is, is
        assert!(check_text("a.html", "<p>one<br>two</p>").is_empty());
        assert_eq!(one("a.xmlp", "<xms-patch target=\"ui\\a.xml\">\n").code, "T020");
    }

    #[test]
    fn json_that_does_not_read() {
        assert!(check_text("a.json", "{\"a\": [1, 2]}").is_empty());
        let p = one("a.json", "{\n  \"a\": 1,\n  \"b\" 2\n}");
        assert_eq!((p.code.as_str(), p.line), ("T030", Some(3)), "{p:?}");
        assert!(p.message.starts_with("Line 3: expected `:`"), "{}", p.message);
        assert!(!p.message.contains("column"), "{}", p.message);
        // columns in characters
        let p = one("a.json", "{\"\u{0436}\u{0436}\": x}");
        assert_eq!(p.column, Some(8), "{p:?}");
    }

    #[test]
    fn other_files_have_no_problems() {
        assert!(check_text("a.ltx", "[s]\nkey = = =\n").is_empty());
        assert!(check_text("a.md", "# x\n```lua\nlocal = \n```").is_empty());
    }
}
