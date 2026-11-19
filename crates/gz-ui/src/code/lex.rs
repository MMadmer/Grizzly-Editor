//! The tokens of a text in its language: spans of byte ranges, in order, each with the kind of
//! token it is; the text between spans is plain. Lexing never fails: a text that does not read
//! as its language is coloured as far as it does.

use super::CodeLang;

/// The kind of a token, by the colour it takes (`Syntax`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Tok {
    Keyword,
    Str,
    Number,
    Comment,
    Function,
    Name,
    Tag,
    Attribute,
    Punct,
    Added,
    Removed,
}

/// A token: its byte range and its kind.
pub type Span = (usize, usize, Tok);

/// The tokens of `text` as `lang` reads it.
#[must_use]
pub fn spans(lang: CodeLang, text: &str) -> Vec<Span> {
    match lang {
        CodeLang::Plain => Vec::new(),
        CodeLang::Lua | CodeLang::LuaData => lua(text),
        CodeLang::Ltx => ltx(text),
        CodeLang::Xml => xml(text),
        CodeLang::Markdown => markdown(text),
        CodeLang::Json => json(text),
        CodeLang::Diff => diff(text),
    }
}

// ---- Lua ----------------------------------------------------------------------------------------

/// Lua's reserved words (5.1, which LuaJIT reads, and `goto` of 5.2 that LuaJIT takes).
pub const LUA_KEYWORDS: &[&str] = &[
    "and", "break", "do", "else", "elseif", "end", "false", "for", "function", "goto", "if", "in", "local",
    "nil", "not", "or", "repeat", "return", "then", "true", "until", "while",
];

fn is_ident_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

/// The level of a long bracket that opens at `i` (`[[` is 0, `[==[` is 2), if one does.
fn long_open(b: &[u8], i: usize) -> Option<usize> {
    if b.get(i) != Some(&b'[') {
        return None;
    }
    let mut j = i + 1;
    while b.get(j) == Some(&b'=') {
        j += 1;
    }
    (b.get(j) == Some(&b'[')).then_some(j - i - 1)
}

/// Where a long bracket of `level` that opened before `from` ends (past its `]==]`).
fn long_close(t: &str, from: usize, level: usize) -> usize {
    let close = format!("]{}]", "=".repeat(level));
    t.get(from..).and_then(|r| r.find(&close)).map_or(t.len(), |e| from + e + close.len())
}

/// A quoted string from its quote at `i` to past the closing one, or to the line's end.
fn quoted(b: &[u8], i: usize) -> usize {
    let q = b[i];
    let mut j = i + 1;
    while j < b.len() && b[j] != q && b[j] != b'\n' {
        j += if b[j] == b'\\' { 2 } else { 1 };
    }
    (j + 1).min(b.len())
}

/// A number from `i`: hex with its binary exponent, decimals with their exponent, LuaJIT's
/// `LL`, `ULL` and `i` suffixes.
fn number(b: &[u8], i: usize) -> usize {
    let mut j = i;
    if b.get(j) == Some(&b'0') && matches!(b.get(j + 1), Some(b'x' | b'X')) {
        j += 2;
        while j < b.len() && (b[j].is_ascii_hexdigit() || b[j] == b'.') {
            j += 1;
        }
        if matches!(b.get(j), Some(b'p' | b'P')) {
            j += 1;
            if matches!(b.get(j), Some(b'+' | b'-')) {
                j += 1;
            }
        }
    }
    while j < b.len() && (b[j].is_ascii_digit() || b[j] == b'.') {
        j += 1;
    }
    if matches!(b.get(j), Some(b'e' | b'E')) {
        j += 1;
        if matches!(b.get(j), Some(b'+' | b'-')) {
            j += 1;
        }
    }
    while j < b.len() && (b[j].is_ascii_alphanumeric()) {
        j += 1;
    }
    j
}

/// The first byte at or after `i` that is no blank (space or tab).
fn skip_blank(b: &[u8], mut i: usize) -> usize {
    while i < b.len() && (b[i] == b' ' || b[i] == b'\t') {
        i += 1;
    }
    i
}

fn lua(t: &str) -> Vec<Span> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    // the brackets open at this point: a name before `=` right inside `{` is a table's key
    let mut open: Vec<u8> = Vec::new();
    // the last word was `function`: the names up to `(` are the function's
    let mut naming = false;
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'-' && b.get(i + 1) == Some(&b'-') {
            i = match long_open(b, i + 2) {
                Some(level) => long_close(t, i + 2 + level + 2, level),
                None => t.get(i..).and_then(|r| r.find('\n')).map_or(b.len(), |e| i + e),
            };
            out.push((start, i, Tok::Comment));
        } else if c == b'"' || c == b'\'' {
            i = quoted(b, i);
            out.push((start, i, Tok::Str));
        } else if let Some(level) = long_open(b, i) {
            i = long_close(t, i + level + 2, level);
            out.push((start, i, Tok::Str));
        } else if c.is_ascii_digit() || (c == b'.' && b.get(i + 1).is_some_and(u8::is_ascii_digit)) {
            i = number(b, i);
            out.push((start, i, Tok::Number));
        } else if is_ident_start(c) {
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            let word = &t[start..i];
            let next = skip_blank(b, i);
            let after = b.get(next).copied();
            let kind = if LUA_KEYWORDS.contains(&word) {
                naming = word == "function";
                Some(Tok::Keyword)
            } else if naming {
                if matches!(after, Some(b'.' | b':')) { Some(Tok::Name) } else { Some(Tok::Function) }
            } else if word == "self" {
                Some(Tok::Name)
            } else if after == Some(b'=') && b.get(next + 1) != Some(&b'=') && open.last() == Some(&b'{') {
                Some(Tok::Attribute)
            } else if matches!(after, Some(b'(' | b'"' | b'\'' | b'{'))
                || (after == Some(b'[') && long_open(b, next).is_some())
            {
                Some(Tok::Function)
            } else if matches!(after, Some(b'.' | b':'))
                && b.get(next + 1).is_some_and(|&x| is_ident_start(x))
            {
                Some(Tok::Name)
            } else {
                None
            };
            if let Some(kind) = kind {
                out.push((start, i, kind));
            }
            continue;
        } else {
            i += 1;
            while i < b.len() && !t.is_char_boundary(i) {
                i += 1;
            }
            match c {
                b'(' | b'{' | b'[' => {
                    open.push(c);
                    if c == b'(' {
                        naming = false;
                    }
                }
                b')' | b'}' | b']' => {
                    open.pop();
                }
                _ => {}
            }
            if c.is_ascii_punctuation() {
                out.push((start, i, Tok::Punct));
            }
        }
    }
    merge(out)
}

/// Neighbouring spans of one kind as one: fewer sections for the layout.
fn merge(spans: Vec<Span>) -> Vec<Span> {
    let mut out: Vec<Span> = Vec::with_capacity(spans.len());
    for s in spans {
        if let Some(last) = out.last_mut()
            && last.2 == s.2
            && last.1 == s.0
        {
            last.1 = s.1;
            continue;
        }
        out.push(s);
    }
    out
}

// ---- LTX ----------------------------------------------------------------------------------------

/// Every line of `t` with its byte offset, without its line end.
fn lines(t: &str) -> impl Iterator<Item = (usize, &str)> {
    let mut at = 0usize;
    t.split_inclusive('\n').map(move |l| {
        let start = at;
        at += l.len();
        (start, l.trim_end_matches(['\n', '\r']))
    })
}

/// Where a comment starts in an LTX line (`;` or `//` outside quotes).
fn ltx_comment(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut quote = false;
    for (i, &c) in b.iter().enumerate() {
        match c {
            b'"' => quote = !quote,
            b';' if !quote => return Some(i),
            b'/' if !quote && b.get(i + 1) == Some(&b'/') => return Some(i),
            _ => {}
        }
    }
    None
}

fn is_number_list(v: &str) -> bool {
    !v.is_empty()
        && v.split(',').all(|p| {
            let p = p.trim();
            !p.is_empty() && p.parse::<f64>().is_ok()
        })
}

fn ltx(t: &str) -> Vec<Span> {
    let mut out = Vec::new();
    for (at, line) in lines(t) {
        let (body, comment) = match ltx_comment(line) {
            Some(c) => (&line[..c], Some(c)),
            None => (line, None),
        };
        let lead = body.len() - body.trim_start().len();
        let rest = body.trim_start();
        let base = at + lead;
        if rest.starts_with('[') {
            // [name]:parent, parent
            out.push((base, base + 1, Tok::Punct));
            let close = rest.find(']').unwrap_or(rest.len());
            out.push((base + 1, base + close, Tok::Function));
            if close < rest.len() {
                out.push((base + close, base + close + 1, Tok::Punct));
                let tail = &rest[close + 1..];
                if let Some(colon) = tail.find(':') {
                    let c0 = base + close + 1 + colon;
                    out.push((c0, c0 + 1, Tok::Punct));
                    out.push((c0 + 1, base + rest.trim_end().len(), Tok::Name));
                }
            }
        } else if rest.starts_with('#') || rest.starts_with('!') {
            // `#include "x"`, a patch's directives (`#file`, `!section`)
            let word = rest.find([' ', '\t', '"']).unwrap_or(rest.len());
            out.push((base, base + word, Tok::Keyword));
            let tail = &rest[word..];
            if let Some(q) = tail.find('"') {
                out.push((base + word + q, base + rest.trim_end().len(), Tok::Str));
            }
        } else if let Some(eq) = rest.find('=') {
            let key = rest[..eq].trim_end();
            if !key.is_empty() {
                out.push((base, base + key.len(), Tok::Attribute));
            }
            out.push((base + eq, base + eq + 1, Tok::Punct));
            let value = &rest[eq + 1..];
            let vlead = value.len() - value.trim_start().len();
            let v = value.trim();
            let v0 = base + eq + 1 + vlead;
            if !v.is_empty() {
                let kind = if v.starts_with('"') {
                    Some(Tok::Str)
                } else if is_number_list(v) {
                    Some(Tok::Number)
                } else if ["true", "false", "on", "off", "yes", "no"]
                    .iter()
                    .any(|w| v.eq_ignore_ascii_case(w))
                {
                    Some(Tok::Keyword)
                } else {
                    None
                };
                if let Some(kind) = kind {
                    out.push((v0, v0 + v.len(), kind));
                }
            }
        } else if !rest.trim().is_empty() {
            out.push((base, base + rest.trim_end().len(), Tok::Attribute));
        }
        if let Some(c) = comment {
            out.push((at + c, at + line.len(), Tok::Comment));
        }
    }
    out
}

// ---- XML ----------------------------------------------------------------------------------------

fn xml(t: &str) -> Vec<Span> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    let find = |from: usize, what: &str| {
        t.get(from..).and_then(|r| r.find(what)).map_or(t.len(), |e| from + e + what.len())
    };
    while i < b.len() {
        let c = b[i];
        if c == b'<' {
            let start = i;
            if t[i..].starts_with("<!--") {
                i = find(i + 4, "-->");
                out.push((start, i, Tok::Comment));
            } else if t[i..].starts_with("<![CDATA[") {
                i = find(i + 9, "]]>");
                out.push((start, i, Tok::Str));
            } else if t[i..].starts_with("<?") || t[i..].starts_with("<!") {
                i = find(i + 2, ">");
                out.push((start, i, Tok::Keyword));
            } else {
                // <tag attr="v" ...> or </tag>
                let mut j = i + 1;
                if b.get(j) == Some(&b'/') {
                    j += 1;
                }
                out.push((start, j, Tok::Punct));
                let n0 = j;
                while j < b.len() && !b[j].is_ascii_whitespace() && !matches!(b[j], b'>' | b'/') {
                    j += 1;
                }
                out.push((n0, j, Tok::Tag));
                while j < b.len() && b[j] != b'>' {
                    if b[j] == b'"' || b[j] == b'\'' {
                        let q = b[j];
                        let s0 = j;
                        j += 1;
                        while j < b.len() && b[j] != q {
                            j += 1;
                        }
                        j = (j + 1).min(b.len());
                        out.push((s0, j, Tok::Str));
                    } else if b[j] == b'=' || b[j] == b'/' {
                        out.push((j, j + 1, Tok::Punct));
                        j += 1;
                    } else if b[j].is_ascii_whitespace() {
                        j += 1;
                    } else {
                        let a0 = j;
                        while j < b.len()
                            && !b[j].is_ascii_whitespace()
                            && !matches!(b[j], b'=' | b'>' | b'/')
                        {
                            j += 1;
                        }
                        out.push((a0, j, Tok::Attribute));
                    }
                }
                if j < b.len() {
                    out.push((j, j + 1, Tok::Punct));
                    j += 1;
                }
                i = j;
            }
        } else if c == b'&' {
            let start = i;
            let end = t[i..].find(';').filter(|&e| e <= 10).map_or(i + 1, |e| i + e + 1);
            out.push((start, end, Tok::Number));
            i = end;
        } else {
            i += 1;
            while i < b.len() && !t.is_char_boundary(i) {
                i += 1;
            }
        }
    }
    merge(out)
}

// ---- Markdown -----------------------------------------------------------------------------------

/// Inline marks of a Markdown line: code, emphasis, links.
fn md_inline(line: &str, at: usize, out: &mut Vec<Span>) {
    let b = line.as_bytes();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        if c == b'`' {
            let end = line[i + 1..].find('`').map_or(b.len(), |e| i + 1 + e + 1);
            out.push((at + i, at + end, Tok::Str));
            i = end;
        } else if (c == b'*' || c == b'_') && b.get(i + 1).is_some_and(|x| !x.is_ascii_whitespace()) {
            let double = b.get(i + 1) == Some(&c);
            let mark = if double { &line[i..i + 2] } else { &line[i..=i] };
            let from = i + mark.len();
            match line[from..].find(mark) {
                Some(e) if e > 0 => {
                    let end = from + e + mark.len();
                    out.push((at + i, at + end, Tok::Keyword));
                    i = end;
                }
                _ => i += mark.len(),
            }
        } else if c == b'[' {
            // [text](url)
            match (line[i..].find("]("), line[i..].find(')')) {
                (Some(m), Some(e)) if m < e => {
                    out.push((at + i, at + i + m + 1, Tok::Function));
                    out.push((at + i + m + 1, at + i + e + 1, Tok::Name));
                    i += e + 1;
                }
                _ => i += 1,
            }
        } else if c == b'<' && line[i..].starts_with("<http") {
            let end = line[i..].find('>').map_or(b.len(), |e| i + e + 1);
            out.push((at + i, at + end, Tok::Name));
            i = end;
        } else {
            i += 1;
            while i < b.len() && !line.is_char_boundary(i) {
                i += 1;
            }
        }
    }
}

fn markdown(t: &str) -> Vec<Span> {
    let mut out = Vec::new();
    let mut fence: Option<&str> = None;
    for (at, line) in lines(t) {
        let rest = line.trim_start();
        let lead = line.len() - rest.len();
        if let Some(f) = fence {
            out.push((at, at + line.len(), Tok::Str));
            if rest.starts_with(f) {
                fence = None;
            }
            continue;
        }
        if rest.starts_with("```") || rest.starts_with("~~~") {
            fence = Some(&rest[..3]);
            out.push((at, at + line.len(), Tok::Str));
        } else if rest.starts_with('#') && rest.trim_start_matches('#').starts_with([' ', '\t'])
            || rest.trim_end() == "#"
        {
            out.push((at, at + line.len(), Tok::Tag));
        } else if rest.starts_with('>') {
            out.push((at + lead, at + lead + 1, Tok::Punct));
            md_inline(&line[lead + 1..], at + lead + 1, &mut out);
        } else if ["---", "***", "___"].iter().any(|r| {
            rest.trim_end().starts_with(r)
                && rest.trim_end().chars().all(|c| c == r.chars().next().unwrap_or('-') || c == ' ')
        }) {
            out.push((at, at + line.len(), Tok::Punct));
        } else {
            // a list's marker: - * + or a number and a dot
            let marker = if rest.starts_with(['-', '*', '+']) && rest[1..].starts_with(' ') {
                1
            } else {
                let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
                if digits > 0 && rest[digits..].starts_with(['.', ')']) { digits + 1 } else { 0 }
            };
            if marker > 0 {
                out.push((at + lead, at + lead + marker, Tok::Punct));
            }
            md_inline(&line[lead + marker..], at + lead + marker, &mut out);
        }
    }
    let mut out = merge(out);
    out.sort_by_key(|s| s.0);
    out
}

// ---- JSON ---------------------------------------------------------------------------------------

fn json(t: &str) -> Vec<Span> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < b.len() {
        let c = b[i];
        let start = i;
        if c == b'"' {
            i = quoted(b, i);
            let after = skip_blank(b, i);
            let after = {
                let mut k = after;
                while k < b.len() && b[k].is_ascii_whitespace() {
                    k += 1;
                }
                k
            };
            out.push((start, i, if b.get(after) == Some(&b':') { Tok::Function } else { Tok::Str }));
        } else if c == b'-' || c.is_ascii_digit() {
            i += 1;
            while i < b.len() && (b[i].is_ascii_digit() || matches!(b[i], b'.' | b'e' | b'E' | b'+' | b'-')) {
                i += 1;
            }
            out.push((start, i, Tok::Number));
        } else if c.is_ascii_alphabetic() {
            while i < b.len() && b[i].is_ascii_alphabetic() {
                i += 1;
            }
            if matches!(&t[start..i], "true" | "false" | "null") {
                out.push((start, i, Tok::Keyword));
            }
        } else {
            i += 1;
            while i < b.len() && !t.is_char_boundary(i) {
                i += 1;
            }
            if matches!(c, b'{' | b'}' | b'[' | b']' | b':' | b',') {
                out.push((start, i, Tok::Punct));
            }
        }
    }
    merge(out)
}

// ---- diff ---------------------------------------------------------------------------------------

fn diff(t: &str) -> Vec<Span> {
    let mut out = Vec::new();
    for (at, line) in lines(t) {
        let kind = if line.starts_with("+++")
            || line.starts_with("---")
            || line.starts_with("diff ")
            || line.starts_with("index ")
        {
            Some(Tok::Keyword)
        } else if line.starts_with("@@") {
            Some(Tok::Function)
        } else if line.starts_with('+') {
            Some(Tok::Added)
        } else if line.starts_with('-') {
            Some(Tok::Removed)
        } else {
            None
        };
        if let Some(kind) = kind {
            out.push((at, at + line.len(), kind));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind_of(lang: CodeLang, t: &str, part: &str) -> Option<Tok> {
        let at = t.find(part)?;
        spans(lang, t).into_iter().find(|s| s.0 <= at && at + part.len() <= s.1).map(|s| s.2)
    }

    #[test]
    fn spans_are_in_order_and_inside_the_text() {
        let texts = [
            (CodeLang::Lua, "local x = { a = 1, [[s]] } --[==[ c ]==] f(x) 'é'"),
            (CodeLang::Ltx, "[a]:b ; c\nk = 1, 2\n#include \"x.ltx\"\n"),
            (CodeLang::Xml, "<?xml version=\"1.0\"?><a b=\"c\">&amp; é<!-- d --></a>"),
            (CodeLang::Markdown, "# T\n- *a* `b` [c](d)\n```lua\nx\n```\n"),
            (CodeLang::Json, "{\"a\": [1, -2.5e3, true, null, \"é\"]}"),
            (CodeLang::Diff, "--- a\n+++ b\n@@ -1 +1 @@\n-x\n+y\n"),
        ];
        for (lang, t) in texts {
            let s = spans(lang, t);
            for w in s.windows(2) {
                assert!(w[0].1 <= w[1].0, "{lang:?}: {w:?} overlap");
            }
            for (a, b, _) in &s {
                assert!(
                    a < b && *b <= t.len() && t.is_char_boundary(*a) && t.is_char_boundary(*b),
                    "{lang:?}"
                );
            }
        }
    }

    #[test]
    fn lua_reads_as_lua() {
        let t = "local function xr_go(npc)\n  -- note\n  local v = xr_logic.pick(npc, \"s\") or 0x1F\n  return { id = v, [[raw]] }\nend\n--[=[ long\n]] still ]=] self";
        let k = |p| kind_of(CodeLang::Lua, t, p);
        assert_eq!(k("local"), Some(Tok::Keyword));
        assert_eq!(k("xr_go"), Some(Tok::Function));
        assert_eq!(k("-- note"), Some(Tok::Comment));
        assert_eq!(k("xr_logic"), Some(Tok::Name));
        assert_eq!(k("pick"), Some(Tok::Function));
        assert_eq!(k("\"s\""), Some(Tok::Str));
        assert_eq!(k("0x1F"), Some(Tok::Number));
        assert_eq!(k("id"), Some(Tok::Attribute));
        assert_eq!(k("[[raw]]"), Some(Tok::Str));
        assert_eq!(k("]] still"), Some(Tok::Comment));
        assert_eq!(k("self"), Some(Tok::Name));
        // a plain assignment is no table key
        assert_eq!(kind_of(CodeLang::Lua, "x = 1", "x"), None);
    }

    #[test]
    fn ltx_reads_sections_keys_and_values() {
        let t = "[wpn_ak]:identity_immunities, weapon ; c\ncost = 1200\nflag = true\nname = st_ak\n#include \"w.ltx\"\n!removed\n";
        let k = |p| kind_of(CodeLang::Ltx, t, p);
        assert_eq!(k("wpn_ak"), Some(Tok::Function));
        assert_eq!(k("identity_immunities"), Some(Tok::Name));
        assert_eq!(k("; c"), Some(Tok::Comment));
        assert_eq!(k("cost"), Some(Tok::Attribute));
        assert_eq!(k("1200"), Some(Tok::Number));
        assert_eq!(k("true"), Some(Tok::Keyword));
        assert_eq!(k("st_ak"), None);
        assert_eq!(k("#include"), Some(Tok::Keyword));
        assert_eq!(k("\"w.ltx\""), Some(Tok::Str));
        assert_eq!(k("!removed"), Some(Tok::Keyword));
    }

    #[test]
    fn xml_json_markdown_and_diff_read() {
        let x = "<string id=\"a\"><text>b &amp; c</text></string><!-- n -->";
        assert_eq!(kind_of(CodeLang::Xml, x, "string"), Some(Tok::Tag));
        assert_eq!(kind_of(CodeLang::Xml, x, "id"), Some(Tok::Attribute));
        assert_eq!(kind_of(CodeLang::Xml, x, "\"a\""), Some(Tok::Str));
        assert_eq!(kind_of(CodeLang::Xml, x, "&amp;"), Some(Tok::Number));
        assert_eq!(kind_of(CodeLang::Xml, x, "<!-- n -->"), Some(Tok::Comment));
        let j = "{\"k\": \"v\", \"n\": 1}";
        assert_eq!(kind_of(CodeLang::Json, j, "\"k\""), Some(Tok::Function));
        assert_eq!(kind_of(CodeLang::Json, j, "\"v\""), Some(Tok::Str));
        let m = "## Head\nsome **bold** and `code`, [a link](x.md)\n";
        assert_eq!(kind_of(CodeLang::Markdown, m, "## Head"), Some(Tok::Tag));
        assert_eq!(kind_of(CodeLang::Markdown, m, "**bold**"), Some(Tok::Keyword));
        assert_eq!(kind_of(CodeLang::Markdown, m, "`code`"), Some(Tok::Str));
        assert_eq!(kind_of(CodeLang::Markdown, m, "[a link]"), Some(Tok::Function));
        assert_eq!(kind_of(CodeLang::Markdown, m, "(x.md)"), Some(Tok::Name));
        let d = "@@ -1 +1 @@\n-old\n+new\n";
        assert_eq!(kind_of(CodeLang::Diff, d, "-old"), Some(Tok::Removed));
        assert_eq!(kind_of(CodeLang::Diff, d, "+new"), Some(Tok::Added));
    }
}
