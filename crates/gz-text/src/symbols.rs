//! What Go to Symbol lists of a file: a script's functions, a config's sections, a Markdown
//! file's headings.

use crate::lex::{self, Tk, Token};

/// What a symbol is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Value,
    Section,
    Heading,
}

/// A symbol of a file: its name, what to show beside it (a function's parameters, a section's
/// parents, a heading's level), its 1-based line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Symbol {
    pub name: String,
    pub detail: String,
    pub line: u32,
    pub kind: SymbolKind,
}

fn extension(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.').map_or(String::new(), |(_, e)| e.to_ascii_lowercase())
}

/// The symbols of a text, by the file's name (its extension), in their order.
#[must_use]
pub fn symbols(name: &str, text: &str) -> Vec<Symbol> {
    match extension(name).as_str() {
        "script" | "lua" | "s" => lua(text),
        "ltx" | "ltxp" | "ini" | "cfg" => ltx(text),
        "md" | "markdown" => markdown(text),
        _ => Vec::new(),
    }
}

/// The parameters of the list that opens at `at` (a `(`), as written: `(npc, ini, ...)`, and
/// the index after its `)`.
pub(crate) fn params_at(tokens: &[Token<'_>], at: usize) -> (String, usize) {
    let mut names: Vec<&str> = Vec::new();
    let mut j = at + 1;
    while let Some(t) = tokens.get(j) {
        j += 1;
        if t.is(")") {
            break;
        }
        if t.kind == Tk::Name || t.is("...") {
            names.push(t.text);
        } else if !t.is(",") {
            // no parameter list after all
            break;
        }
    }
    (format!("({})", names.join(", ")), j)
}

/// The dotted name a `function` keyword at `at` declares (`a.b:c`), with the index of the
/// token after it and the line of its first part; empty for an anonymous function.
pub(crate) fn declared_name<'a>(tokens: &[Token<'a>], at: usize) -> (Vec<&'a str>, usize, u32) {
    let mut parts: Vec<&str> = Vec::new();
    let mut j = at + 1;
    let mut line = tokens.get(at).map_or(1, |t| t.line);
    while let Some(t) = tokens.get(j) {
        let want_name = parts.last().is_none_or(|p| *p == "." || *p == ":");
        if want_name && t.kind == Tk::Name {
            if parts.is_empty() {
                line = t.line;
            }
            parts.push(t.text);
        } else if !want_name && (t.is(".") || t.is(":")) {
            parts.push(t.text);
        } else {
            break;
        }
        j += 1;
    }
    (parts, j, line)
}

/// The name an anonymous function at `at` is given by the assignment before it: `name = `,
/// `a.b = `, `["key"] = `; with the line of that name.
fn assigned_name(tokens: &[Token<'_>], at: usize) -> Option<(String, u32)> {
    let eq = at.checked_sub(1).and_then(|i| tokens.get(i))?;
    if !eq.is("=") {
        return None;
    }
    let mut j = at.checked_sub(2)?;
    let last = tokens.get(j)?;
    if last.is("]") {
        let key = tokens.get(j.checked_sub(1)?)?;
        let open = tokens.get(j.checked_sub(2)?)?;
        return (key.kind == Tk::Str && open.is("[")).then(|| (key.string_body().to_owned(), key.line));
    }
    if last.kind != Tk::Name {
        return None;
    }
    let mut parts = vec![last.text];
    let mut line = last.line;
    while j >= 2 {
        let (sep, name) = (tokens.get(j - 1)?, tokens.get(j - 2)?);
        if !(sep.is(".") || sep.is(":")) || name.kind != Tk::Name {
            break;
        }
        parts.push(sep.text);
        parts.push(name.text);
        line = name.line;
        j -= 2;
    }
    parts.reverse();
    Some((parts.concat(), line))
}

/// A script's functions, nested ones too: `function a.b:c(x)`, `local function f(...)`,
/// `name = function(...)`.
fn lua(text: &str) -> Vec<Symbol> {
    let tokens = lex::tokens(text);
    let mut out = Vec::new();
    for (i, t) in tokens.iter().enumerate() {
        if !t.is("function") {
            continue;
        }
        let (parts, after, line) = declared_name(&tokens, i);
        if !tokens.get(after).is_some_and(|t| t.is("(")) {
            continue;
        }
        let (detail, _) = params_at(&tokens, after);
        let named = if parts.is_empty() { assigned_name(&tokens, i) } else { Some((parts.concat(), line)) };
        if let Some((name, line)) = named {
            out.push(Symbol { name, detail, line, kind: SymbolKind::Function });
        }
    }
    out
}

/// A config's sections: `[name]:parent, parent`, the parents beside the name.
fn ltx(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    for (n, raw) in text.split('\n').enumerate() {
        let line = raw.split(';').next().unwrap_or("").trim();
        let Some(body) = line.strip_prefix('[') else { continue };
        let Some(close) = body.find(']') else { continue };
        let name = body.get(..close).unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let rest = body.get(close + 1..).unwrap_or("").trim();
        let detail = rest.strip_prefix(':').map_or(String::new(), |p| {
            p.split(',').map(str::trim).filter(|x| !x.is_empty()).collect::<Vec<_>>().join(", ")
        });
        let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
        out.push(Symbol { name: name.to_owned(), detail, line, kind: SymbolKind::Section });
    }
    out
}

/// A Markdown file's headings (`#` to `######`), outside fenced code; the level beside each.
fn markdown(text: &str) -> Vec<Symbol> {
    let mut out = Vec::new();
    let mut fence: Option<(char, usize)> = None;
    for (n, raw) in text.split('\n').enumerate() {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        let indent = raw.len() - raw.trim_start_matches(' ').len();
        if indent > 3 {
            continue;
        }
        let body = raw.trim_start_matches(' ');
        let mark = body.chars().next().filter(|c| *c == '`' || *c == '~');
        if let Some(c) = mark {
            let run = body.chars().take_while(|x| *x == c).count();
            if run >= 3 {
                match fence {
                    Some((f, len)) if f == c && run >= len && body.trim_end().chars().all(|x| x == c) => {
                        fence = None;
                    }
                    Some(_) => {}
                    None => fence = Some((c, run)),
                }
                continue;
            }
        }
        if fence.is_some() {
            continue;
        }
        let level = body.chars().take_while(|c| *c == '#').count();
        if !(1..=6).contains(&level) {
            continue;
        }
        let rest = body.get(level..).unwrap_or("");
        if !(rest.is_empty() || rest.starts_with([' ', '\t'])) {
            continue;
        }
        // a closing run of '#' after a blank is no part of the heading
        let mut title = rest.trim();
        let stripped = title.trim_end_matches('#');
        if stripped.is_empty() || stripped.ends_with([' ', '\t']) {
            title = stripped.trim_end();
        }
        let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
        out.push(Symbol {
            name: title.to_owned(),
            detail: format!("H{level}"),
            line,
            kind: SymbolKind::Heading,
        });
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn list(name: &str, text: &str) -> Vec<(String, String, u32)> {
        symbols(name, text).into_iter().map(|s| (s.name, s.detail, s.line)).collect()
    }

    fn row(name: &str, detail: &str, line: u32) -> (String, String, u32) {
        (name.to_owned(), detail.to_owned(), line)
    }

    #[test]
    fn a_scripts_functions_in_order_nested_ones_too() {
        let text = "\
-- function commented(a)
local s = \"function in_string(b)\"
function top(npc, ini)
  local function inner(...)
  end
  local cb = function(x) end
end
function a.b:c(x)
end
t.field = function() end
local tbl = { name = function(self, y) end, [\"key\"] = function() end }
/* function forked() end */
local function last()
end";
        assert_eq!(
            list("a.script", text),
            [
                row("top", "(npc, ini)", 3),
                row("inner", "(...)", 4),
                row("cb", "(x)", 6),
                row("a.b:c", "(x)", 8),
                row("t.field", "()", 10),
                row("name", "(self, y)", 11),
                row("key", "()", 11),
                row("last", "()", 13),
            ]
        );
        assert!(symbols("a.script", "x = function").is_empty());
    }

    #[test]
    fn a_configs_sections_with_their_parents() {
        // a patch's section that drops one keeps its mark in the name
        let text = "#include \"x.ltx\"\n[base]\nkey = 1 ; [not a section]\n[wpn_ak]:base, other ; comment\n  [!gone]\n[]\n";
        let found = list("weapons.ltx", text);
        assert_eq!(found[..2], [row("base", "", 2), row("wpn_ak", "base, other", 4)]);
        assert_eq!((found[2].0.strip_prefix('!'), found[2].2, found.len()), (Some("gone"), 5, 3));
        assert_eq!(list("mod.ltx", "[module]\nid = x\n"), [row("module", "", 1)]);
    }

    #[test]
    fn a_markdown_files_headings_outside_code() {
        let text = "# Title\ntext\n## Part two ##\n```lua\n# not a heading\n```\n#no\n   ### Three\n    # code\n~~~\n# no\n~~~\n###### Six";
        assert_eq!(
            list("README.md", text),
            [row("Title", "H1", 1), row("Part two", "H2", 3), row("Three", "H3", 8), row("Six", "H6", 13)]
        );
        assert!(symbols("a.txt", "# x").is_empty());
    }
}
