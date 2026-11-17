//! What a module's code does (the game's `MOD_CATALOG.md` 10.3 and 10.4): the script report -
//! what only a module that runs unprotected would do, and what a reviewer looks at - and whether
//! the code works with the network. One scan for the build, the packaging and the catalog's check;
//! the tokenizer, the rules and the words are the catalog's own `tools/catalog/catalog.py` and the
//! network rule is the game's own (its `MOD_CATALOG.md` 10.4), ported: change all of them or none.
//!
//! Code is what the game runs as Lua: scripts (`.script`, `.lua`), shader scripts (`.s`) and the
//! strings of the assets that carry code as text (`.nqasset`, `.behasset`, `.pdaasset`).

/// What the author is told when the module's code works with the network: the game says so to
/// its players. A fact of the module, never a problem of a build or a release.
pub const NETWORK_NOTE: &str = "The module's code works with the network. The game marks it Network access in the Mods menu and the Mod Browser and says so before a player installs it.";

/// How a file carries code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodeKind {
    /// Lua text: a script, a shader script.
    Script,
    /// Lua data whose strings may be code: a quest, a behaviour graph, a PDA interface.
    Asset,
}

/// Whether a file of a module carries code, by its name.
#[must_use]
pub fn code_kind(path: &str) -> Option<CodeKind> {
    let ext = path.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase())?;
    match ext.as_str() {
        "script" | "lua" | "s" => Some(CodeKind::Script),
        "nqasset" | "behasset" | "pdaasset" => Some(CodeKind::Asset),
        _ => None,
    }
}

/// One place of the report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodeFinding {
    /// The Mod Browser refuses the module for it; otherwise its review looks at it.
    pub blocks: bool,
    pub line: u32,
    pub what: String,
}

/// What a file's code does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CodeReport {
    pub findings: Vec<CodeFinding>,
    /// The first line that works with the network.
    pub network: Option<u32>,
}

/// The report of one file. An asset's own text is data: each of its strings that calls something
/// is read as a script.
#[must_use]
pub fn report(kind: CodeKind, text: &[u8]) -> CodeReport {
    let mut out = CodeReport { findings: Vec::new(), network: network_line(text) };
    match kind {
        CodeKind::Script => script_findings(text, 0, &mut out.findings),
        CodeKind::Asset => {
            let tokens = tokenize(text);
            string_findings(&tokens, 0, &mut out.findings);
            for t in &tokens {
                if t.kind != Kind::String {
                    continue;
                }
                let code = unquoted(t.text);
                if code.contains(&b'(') {
                    script_findings(&code, t.line - 1, &mut out.findings);
                }
            }
        }
    }
    out
}

/// A string token's content: a long string as it is, a quoted one with its escapes of a quote, a
/// backslash and a line break read.
fn unquoted(token: &[u8]) -> Vec<u8> {
    if let Some(level) = long_open(token, 0) {
        let from = level + 2;
        let to = token.len().saturating_sub(level + 2).max(from);
        return token.get(from..to).unwrap_or(&[]).to_vec();
    }
    let inner = token.get(1..token.len().saturating_sub(1).max(1)).unwrap_or(&[]);
    let mut out = Vec::with_capacity(inner.len());
    let mut i = 0;
    while let Some(&c) = inner.get(i) {
        if c == b'\\' {
            match inner.get(i + 1) {
                Some(b'n') => out.push(b'\n'),
                Some(b't') | Some(b'r') => out.push(b' '),
                Some(&other) => out.push(other),
                None => {}
            }
            i += 2;
        } else {
            out.push(c);
            i += 1;
        }
    }
    out
}

// ---- the network (10.4) ----------------------------------------------------------------------------

/// Lua text without its comments, in lower case. Strings stay - code is passed around as text -
/// and so does every line break, so a place in the result is a line of the file.
fn code_of(t: &[u8]) -> Vec<u8> {
    let n = t.len();
    let mut out = Vec::with_capacity(n);
    let at = |i: usize| t.get(i).copied().unwrap_or(0);
    // the place after the long bracket of `level` that closes, or the end
    let close = |from: usize, level: usize| -> usize {
        let mut j = from;
        while j < n {
            if at(j) == b']' {
                let mut k = j + 1;
                let mut l = 0;
                while k < n && at(k) == b'=' {
                    k += 1;
                    l += 1;
                }
                if l == level && k < n && at(k) == b']' {
                    return k + 1;
                }
            }
            j += 1;
        }
        n
    };
    let mut i = 0;
    while i < n {
        let c = at(i);
        if c == b'-' && at(i + 1) == b'-' && i + 1 < n {
            let j = i + 2;
            let end = match long_open(t, j) {
                Some(level) => close(j + 2 + level, level),
                None => t.get(j..).and_then(|s| s.iter().position(|&x| x == b'\n')).map_or(n, |p| j + p),
            };
            out.extend(t.get(i..end).unwrap_or(&[]).iter().filter(|&&x| x == b'\n'));
            out.push(b' ');
            i = end;
            continue;
        }
        if c == b'"' || c == b'\'' {
            let mut j = i + 1;
            while j < n && at(j) != c && at(j) != b'\n' {
                j += if at(j) == b'\\' && j + 1 < n { 2 } else { 1 };
            }
            if j < n && at(j) == c {
                j += 1;
            }
            let j = j.min(n);
            out.extend(t.get(i..j).unwrap_or(&[]).iter().map(u8::to_ascii_lowercase));
            i = j;
            continue;
        }
        if c == b'['
            && let Some(level) = long_open(t, i)
        {
            let end = close(i + 2 + level, level);
            out.extend(t.get(i..end).unwrap_or(&[]).iter().map(u8::to_ascii_lowercase));
            i = end;
            continue;
        }
        out.push(c.to_ascii_lowercase());
        i += 1;
    }
    out
}

/// What reaches the network from a script, by its name: the libraries of the system and of Lua,
/// and the programs a command line fetches with. Whole words, any case.
const NETWORK_WORDS: &[&[u8]] = &[
    b"winhttp",
    b"wininet",
    b"urlmon",
    b"ws2_32",
    b"wsock32",
    b"luasocket",
    b"socket.core",
    b"socket.http",
    b"curl",
    b"wget",
    b"bitsadmin",
    b"invoke-webrequest",
    b"invoke-restmethod",
    b"webclient",
    b"urldownloadtofile",
];

fn find_from(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| from + p)
}

/// The first line of Lua text that works with the network: the game's own API (`xms.http`,
/// `xms["http"]`) or one of [`NETWORK_WORDS`], in code or in a string, never in a comment.
#[must_use]
pub fn network_line(text: &[u8]) -> Option<u32> {
    let code = code_of(text);
    let word = |i: usize| code.get(i).is_some_and(|&c| name_char(c));
    let mut first: Option<usize> = None;
    let mut from = 0;
    while let Some(at) = find_from(&code, b"xms", from) {
        from = at + 1;
        if at > 0 && word(at - 1) {
            continue;
        }
        let mut j = at + 3;
        while code.get(j).is_some_and(|c| b" \t.[]\"'".contains(c)) {
            j += 1;
        }
        if j == at + 3 || code.get(j..j + 4) != Some(b"http") || word(j + 4) {
            continue;
        }
        first = Some(at);
        break;
    }
    for needle in NETWORK_WORDS {
        let mut from = 0;
        while let Some(at) = find_from(&code, needle, from) {
            if first.is_some_and(|f| at >= f) {
                break;
            }
            from = at + 1;
            if (at > 0 && word(at - 1)) || word(at + needle.len()) {
                continue;
            }
            first = Some(at);
            break;
        }
    }
    first.map(|at| 1 + lines_in(&code, 0, at))
}

// ---- the script report (10.3) ----------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Name,
    String,
    Number,
    Op,
    Other,
}

struct Token<'a> {
    kind: Kind,
    text: &'a [u8],
    line: u32,
}

fn name_start(c: u8) -> bool {
    c.is_ascii_alphabetic() || c == b'_'
}

fn name_char(c: u8) -> bool {
    name_start(c) || c.is_ascii_digit()
}

/// `[=*[` at `p`: the level of the long bracket.
fn long_open(t: &[u8], p: usize) -> Option<usize> {
    if t.get(p) != Some(&b'[') {
        return None;
    }
    let mut q = p + 1;
    while t.get(q) == Some(&b'=') {
        q += 1;
    }
    (t.get(q) == Some(&b'[')).then_some(q - p - 1)
}

/// The long bracket of `level` opened at `p` closes: the index past its `]=*]`.
fn long_close(t: &[u8], p: usize, level: usize) -> Option<usize> {
    let mut i = p + level + 2;
    while i < t.len() {
        if t.get(i) == Some(&b']') {
            let mut q = i + 1;
            let mut n = 0;
            while t.get(q) == Some(&b'=') {
                q += 1;
                n += 1;
            }
            if n == level && t.get(q) == Some(&b']') {
                return Some(q + 1);
            }
        }
        i += 1;
    }
    None
}

fn lines_in(t: &[u8], a: usize, b: usize) -> u32 {
    t.get(a..b).map_or(0, |s| s.iter().filter(|&&c| c == b'\n').count() as u32)
}

const LONGER: &[&[u8]] = &[b"...", b"..", b"==", b"~=", b"<=", b">=", b"::"];
const OPS: &[u8] = b"-+*/%^#&~|<>=(){}[];:,.";

fn push<'a>(out: &mut Vec<Token<'a>>, t: &'a [u8], kind: Kind, a: usize, b: usize, line: u32) {
    out.push(Token { kind, text: t.get(a..b).unwrap_or(&[]), line });
}

fn tokenize(t: &[u8]) -> Vec<Token<'_>> {
    let mut out = Vec::new();
    let mut p = 0usize;
    let mut line = 1u32;
    let end = t.len();
    while p < end {
        let c = t.get(p).copied().unwrap_or(0);
        if matches!(c, b' ' | b'\t' | b'\r' | b'\n' | 0x0C | 0x0B) {
            line += u32::from(c == b'\n');
            p += 1;
            continue;
        }
        if c == b'-' && t.get(p + 1) == Some(&b'-') {
            if let Some(level) = long_open(t, p + 2)
                && let Some(stop) = long_close(t, p + 2, level)
            {
                line += lines_in(t, p, stop);
                p = stop;
                continue;
            }
            // an unclosed long comment is an ordinary line comment, as the catalog reads it
            while p < end && t.get(p) != Some(&b'\n') {
                p += 1;
            }
            continue;
        }
        let start_line = line;
        if let Some(level) = long_open(t, p)
            && let Some(stop) = long_close(t, p, level)
        {
            push(&mut out, t, Kind::String, p, stop, start_line);
            line += lines_in(t, p, stop);
            p = stop;
            continue;
        }
        if c == b'"' || c == b'\'' {
            let mut q = p + 1;
            while q < end && t.get(q) != Some(&c) && t.get(q) != Some(&b'\n') {
                q += if t.get(q) == Some(&b'\\') && q + 1 < end { 2 } else { 1 };
            }
            if q < end && t.get(q) == Some(&c) {
                push(&mut out, t, Kind::String, p, q + 1, start_line);
                line += lines_in(t, p, q + 1);
                p = q + 1;
                continue;
            }
            // an unterminated quote is a stray character
            push(&mut out, t, Kind::Other, p, p + 1, start_line);
            p += 1;
            continue;
        }
        let digit_at = |i: usize| t.get(i).is_some_and(u8::is_ascii_digit);
        let xdigit_at = |i: usize| t.get(i).is_some_and(u8::is_ascii_hexdigit);
        if c.is_ascii_digit() || (c == b'.' && digit_at(p + 1)) {
            let mut q = p;
            // only a complete exponent belongs to the number
            let exponent = |q: &mut usize, lower: u8, upper: u8| {
                let mut e = *q;
                if !matches!(t.get(e), Some(&x) if x == lower || x == upper) {
                    return;
                }
                e += 1;
                if matches!(t.get(e), Some(b'+' | b'-')) {
                    e += 1;
                }
                if !digit_at(e) {
                    return;
                }
                while digit_at(e) {
                    e += 1;
                }
                *q = e;
            };
            let hex = c == b'0'
                && q + 2 < end
                && matches!(t.get(q + 1), Some(b'x' | b'X'))
                && (xdigit_at(q + 2) || t.get(q + 2) == Some(&b'.'));
            if hex {
                q += 2;
                while xdigit_at(q) || t.get(q) == Some(&b'.') {
                    q += 1;
                }
                exponent(&mut q, b'p', b'P');
            } else {
                while digit_at(q) {
                    q += 1;
                }
                if t.get(q) == Some(&b'.') {
                    q += 1;
                }
                while digit_at(q) {
                    q += 1;
                }
                exponent(&mut q, b'e', b'E');
            }
            push(&mut out, t, Kind::Number, p, q, start_line);
            p = q;
            continue;
        }
        if name_start(c) {
            let mut q = p;
            while t.get(q).copied().is_some_and(name_char) {
                q += 1;
            }
            push(&mut out, t, Kind::Name, p, q, start_line);
            p = q;
            continue;
        }
        let mut n = LONGER
            .iter()
            .find(|op| t.get(p..).is_some_and(|rest| rest.starts_with(op)))
            .map_or(0, |op| op.len());
        if n == 0 && c != 0 && OPS.contains(&c) {
            n = 1;
        }
        push(&mut out, t, if n > 0 { Kind::Op } else { Kind::Other }, p, p + n.max(1), start_line);
        p += n.max(1);
    }
    out
}

/// A long string of base64-ish characters only.
fn encoded_blob(s: &[u8]) -> bool {
    let (mut a, mut b) = (0usize, s.len());
    while a < b && s.get(a).is_some_and(|c| b"\"'[=".contains(c)) {
        a += 1;
    }
    while b > a && s.get(b - 1).is_some_and(|c| b"\"']=".contains(c)) {
        b -= 1;
    }
    if a == b {
        return false;
    }
    s.get(a..b).unwrap_or(&[]).iter().all(|&c| name_char(c) || b"+/=\\- \t\r\n".contains(&c))
}

/// A dotted name's first part, its second when it matters, and what the finding calls it.
type Pattern = (&'static [u8], Option<&'static [u8]>, &'static str);

const BLOCKING: &[Pattern] = &[
    (b"ffi", None, "ffi"),
    (b"os", Some(b"execute"), "os.execute"),
    (b"io", Some(b"popen"), "io.popen"),
    (b"package", Some(b"loadlib"), "package.loadlib"),
    (b"string", Some(b"dump"), "string.dump"),
    (b"lfs", Some(b"link"), "lfs.link"),
    (b"jit", Some(b"util"), "jit.util"),
    (b"StartDedicatedServer", None, "StartDedicatedServer"),
];

/// Modules of LuaJIT that hand out native memory: loading one is the same as using it.
const NATIVE_MODULES: &[&str] = &["ffi", "string.buffer", "jit.util"];

const REVIEW: &[(&[u8], &str)] = &[
    (b"setfenv", "setfenv"),
    (b"getfenv", "getfenv"),
    (b"debug", "debug.*"),
    (b"save_as", "ini_file:save_as"),
    (b"w_open", "getFS():w_open"),
    (b"file_delete", "getFS():file_delete"),
    (b"file_rename", "getFS():file_rename"),
    (b"file_copy", "getFS():file_copy"),
    (b"dir_delete", "getFS():dir_delete"),
];

/// The programs of the system a script has no business naming.
const PROGRAMS: &[&[u8]] =
    &[b"cmd.exe", b"powershell", b"rundll32", b"regsvr32", b"mshta", b"wscript", b"cscript", b"schtasks"];

/// What the strings of a file say by themselves: a long encoded blob, a program of the system.
fn string_findings(tokens: &[Token<'_>], base: u32, out: &mut Vec<CodeFinding>) {
    for t in tokens {
        if t.kind != Kind::String {
            continue;
        }
        if t.text.len() > 2000 && encoded_blob(t.text) {
            out.push(CodeFinding {
                blocks: false,
                line: base + t.line,
                what: format!("holds a long encoded blob ({} characters)", t.text.len()),
            });
        }
        let lower = t.text.to_ascii_lowercase();
        if let Some(program) = PROGRAMS.iter().find(|p| contains(&lower, p)) {
            out.push(CodeFinding {
                blocks: false,
                line: base + t.line,
                what: format!("names a program of the system ({})", String::from_utf8_lossy(program)),
            });
        }
    }
}

/// 10.3 over Lua text: what only a module that runs unprotected would do. `base` is added to
/// every line: the text may be a string of a larger file.
fn script_findings(text: &[u8], base: u32, out: &mut Vec<CodeFinding>) {
    let tokens = tokenize(text);
    let add = |out: &mut Vec<CodeFinding>, level: Level, line: u32, what: &str| {
        out.push(CodeFinding { blocks: level == Level::Block, line: base + line, what: what.to_owned() });
    };
    let text_at = |i: usize| -> &[u8] { tokens.get(i).map_or(&[][..], |t| t.text) };
    let kind_at = |i: usize| tokens.get(i).map(|t| t.kind);

    for (i, token) in tokens.iter().enumerate() {
        if token.kind != Kind::Name {
            continue;
        }
        // it hands its argument to a command interpreter; a method, so whatever it is called on
        if token.text == b"forgot_password" {
            add(out, Level::Block, token.line, "uses login_manager:forgot_password");
        }
        if i > 0 && matches!(text_at(i - 1), b"." | b":") {
            continue;
        }
        // the dotted name starting here: a, a.b, a:b
        let mut parts: Vec<&[u8]> = vec![token.text];
        let mut after = i + 1;
        while after + 1 < tokens.len()
            && matches!(text_at(after), b"." | b":")
            && kind_at(after + 1) == Some(Kind::Name)
        {
            parts.push(text_at(after + 1));
            after += 2;
        }
        let first = parts.first().copied().unwrap_or(&[]);
        let line = token.line;
        for (a, b, what) in BLOCKING {
            if first == *a && b.is_none_or(|b| parts.get(1) == Some(&b)) {
                add(out, Level::Block, line, &format!("uses {what}"));
            }
        }
        // require("ffi") and require "ffi" alike
        if first == b"require" {
            let name = if text_at(after) == b"(" { after + 1 } else { after };
            if kind_at(name) == Some(Kind::String)
                && let Some(module) = NATIVE_MODULES.iter().find(|m| contains(text_at(name), m.as_bytes()))
            {
                add(out, Level::Block, line, &format!("loads {module}"));
            }
        }
        if first == b"package"
            && let Some(field) = parts.get(1)
            && matches!(*field, b"searchpath" | b"loaders" | b"searchers" | b"preload" | b"cpath")
        {
            add(out, Level::Review, line, &format!("uses package.{}", String::from_utf8_lossy(field)));
        }
        if first == b"os" && parts.get(1) == Some(&&b"exit"[..]) {
            add(out, Level::Review, line, "uses os.exit");
        }
        if matches!(first, b"load" | b"loadstring" | b"require" | b"dofile" | b"loadfile")
            && text_at(after) == b"("
        {
            let literal =
                kind_at(after + 1) == Some(Kind::String) && matches!(text_at(after + 2), b")" | b",");
            if !literal {
                add(
                    out,
                    Level::Review,
                    line,
                    &format!("{} of a computed string", String::from_utf8_lossy(first)),
                );
            }
        }
        if first == b"_G" && text_at(after) == b"[" {
            add(out, Level::Review, line, "indexes _G by a computed name");
        }
        for (a, what) in REVIEW {
            if parts.iter().any(|p| p == a) {
                add(out, Level::Review, line, &format!("uses {what}"));
            }
        }
        let n = parts.len();
        let console = (n >= 2
            && parts.get(n - 2) == Some(&&b"get_console"[..])
            && parts.last() == Some(&&b"execute"[..]))
            || (first == b"get_console" && text_at(after + 2) == b":" && text_at(after + 3) == b"execute");
        if console {
            add(out, Level::Review, line, "runs a console command");
        }
        if n == 2 && first == b"string" && parts.get(1) == Some(&&b"char"[..]) && text_at(after) == b"(" {
            let (mut depth, mut commas) = (0i32, 0u32);
            for t in tokens.iter().skip(after) {
                match t.text {
                    b"(" => depth += 1,
                    b")" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    b"," if depth == 1 => commas += 1,
                    _ => {}
                }
            }
            if commas >= 16 {
                add(
                    out,
                    Level::Review,
                    line,
                    &format!("builds a string of {} characters with string.char", commas + 1),
                );
            }
        }
    }
    string_findings(&tokens, base, out);
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// How much a place of the report weighs, inside this file only.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Level {
    Review,
    Block,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(kind: CodeKind, text: &str) -> Vec<(bool, u32, String)> {
        report(kind, text.as_bytes()).findings.into_iter().map(|f| (f.blocks, f.line, f.what)).collect()
    }

    #[test]
    fn the_kinds_of_code() {
        assert_eq!(code_kind("gamedata/scripts/a.script"), Some(CodeKind::Script));
        assert_eq!(code_kind("gamedata/shaders/r3/x.S"), Some(CodeKind::Script));
        assert_eq!(code_kind("quests/a.nqasset"), Some(CodeKind::Asset));
        assert_eq!(code_kind("pda/a.PdaAsset"), Some(CodeKind::Asset));
        assert_eq!(code_kind("items/a.itemasset"), None);
        assert_eq!(code_kind("README"), None);
    }

    #[test]
    fn the_tokenizer_reads_numbers_and_stray_quotes() {
        // numbers and exponents do not swallow names; 0x1p4 is one number
        let tokens = tokenize(b"x = 0x1p4 + 1e e2 .5");
        let texts: Vec<&[u8]> = tokens.iter().map(|t| t.text).collect();
        assert_eq!(texts, [&b"x"[..], b"=", b"0x1p4", b"+", b"1", b"e", b"e2", b".5"]);
        // an unterminated quote is a stray character, the rest still reads
        let tokens = tokenize(b"a = \"open\nos.execute");
        assert!(tokens.iter().any(|t| t.text == b"execute" && t.line == 2));
    }

    #[test]
    fn what_escapes_protected_mode_blocks() {
        let found = lines(
            CodeKind::Script,
            "local b = require \"string.buffer\"\nlocal u = require(\"jit.util\")\nmain_menu.get_main_menu():GetLoginMngr():forgot_password(x)\njit.util.funcinfo(f)\n",
        );
        assert_eq!(
            found,
            [
                (true, 1, "loads string.buffer".to_owned()),
                (true, 2, "loads jit.util".to_owned()),
                (true, 3, "uses login_manager:forgot_password".to_owned()),
                (true, 4, "uses jit.util".to_owned()),
            ]
        );
    }

    #[test]
    fn what_a_reviewer_looks_at() {
        let found = lines(
            CodeKind::Script,
            "package.searchpath(\"x\", p)\nos.exit()\nlocal c = \"PowerShell -enc AAAA\"\n",
        );
        assert_eq!(
            found,
            [
                (false, 1, "uses package.searchpath".to_owned()),
                (false, 2, "uses os.exit".to_owned()),
                (false, 3, "names a program of the system (powershell)".to_owned()),
            ]
        );
    }

    #[test]
    fn the_code_of_an_asset_is_in_its_strings() {
        let asset = "return {\n\tnq = 1,\n\tnodes = {\n\t\t{ kind = \"lua\", code = \"os.execute(\\\"calc\\\")\" },\n\t\t{ kind = \"lua\", code = [[\nlocal f = require(\"ffi\")\n]] },\n\t\t{ kind = \"say\", text = \"ffi is a word here\" },\n\t},\n}\n";
        let found = lines(CodeKind::Asset, asset);
        assert_eq!(
            found,
            [(true, 4, "uses os.execute".to_owned()), (true, 6, "loads ffi".to_owned())],
            "{asset}"
        );
        // a string that calls nothing is prose
        assert!(lines(CodeKind::Asset, "return { title = \"os.execute and debug\" }").is_empty());
    }

    #[test]
    fn the_network_is_the_games_api_or_a_library_by_name() {
        for (text, line) in [
            ("xms.http.request(u, f)", Some(1)),
            ("local h = xms . http", Some(1)),
            ("-- xms.http\nlocal h = xms[\"http\"]", Some(2)),
            ("local s = [[\nrequire('socket.http')\n]]", Some(2)),
            ("os.execute('CURL -s x')", Some(1)),
            ("--[[ xms.http\n winhttp ]] local a = 1", None),
            ("-- curl", None),
            ("local xms_http = 1; local recurl = 2; xms.httpx()", None),
            ("local url = \"https://example.org\"", None),
            ("xmshttp()", None),
        ] {
            assert_eq!(network_line(text.as_bytes()), line, "{text}");
        }
        let asset = "return { nodes = { { kind = \"lua\", code = \"xms.http.request(u)\" } } }";
        assert_eq!(report(CodeKind::Asset, asset.as_bytes()).network, Some(1));
    }
}
