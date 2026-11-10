//! The Mod Browser's content rules (MOD_CATALOG 8) and the script report (10.3), the same
//! tokenizer, rules and messages as the catalog's own `tools/catalog.py` (ported through the
//! C++ `EditorCatalog`): change both or neither. A module outside the catalog ships what it
//! likes, so the release is written either way; the findings are warnings and
//! `catalog_content` in the answer.

use serde::Serialize;

/// How much a finding weighs. `Required` is a requirement not met yet that a submission meets
/// by itself (a VirusTotal report its scan waits for); the content rules give `Block` and
/// `Review` only.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Ok,
    Note,
    Review,
    Required,
    Block,
}

impl Level {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Level::Ok => "ok",
            Level::Note => "note",
            Level::Review => "review",
            Level::Required => "required",
            Level::Block => "block",
        }
    }
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Finding {
    pub level: Level,
    /// What was checked: `content`, `script`, ...
    pub code: &'static str,
    pub text: String,
}

impl Finding {
    fn new(level: Level, code: &'static str, text: String) -> Self {
        Self { level, code, text }
    }
}

const ALLOWED: &[&str] = &[
    "ltx", "ltxp", "xml", "xmlp", "script", "txt", "md", "json", "csv", "ini", "cfg", "seq", "gif", "mp4", "webm", "wmv", "html", "htm", "css",
    "png", "jpg", "jpeg", "nqasset", "behasset", "npcasset", "pdaasset", "xspawn", "xcform", "dds", "thm", "ogg", "ogm", "ogf", "omf", "skl",
    "skls", "anm", "ppe", "xr", "ps", "vs", "gs", "hs", "ds", "cs", "h", "hlsl", "s", "spawn", "graph", "ai", "gct", "cform", "details",
    "env_mod", "fog_vol", "game", "geom", "geomx", "hom", "ps_static", "snd_env", "snd_static", "som", "sectors", "wallmarks", "lanims",
];

/// MOD_CATALOG 8 for a path (module-relative, `/`): `Err` says why not (`PathAllowed`).
pub fn path_allowed(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.contains(['\\', ':']) {
        return Err("unsafe path".to_owned());
    }
    for part in path.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err("unsafe path".to_owned());
        }
        if part.starts_with('.') {
            return Err("a dot-named file or folder".to_owned());
        }
    }
    let name = path.rsplit('/').next().unwrap_or(path);
    let Some(dot) = name.rfind('.') else { return Ok(()) };
    let extension = name.get(dot + 1..).unwrap_or("").to_ascii_lowercase();
    if ALLOWED.contains(&extension.as_str()) {
        Ok(())
    } else {
        Err(format!(".{extension} files are not allowed in a Mod Browser module"))
    }
}

const MAGICS: &[(&[u8], &str)] = &[
    (b"\x7FELF", "a Linux program"),
    (b"\xFE\xED\xFA\xCE", "a macOS program"),
    (b"\xFE\xED\xFA\xCF", "a macOS program"),
    (b"\xCE\xFA\xED\xFE", "a macOS program"),
    (b"\xCF\xFA\xED\xFE", "a macOS program"),
    (b"\xCA\xFE\xBA\xBE", "a Java class or a universal binary"),
    (b"PK\x03\x04", "a ZIP archive"),
    (b"PK\x05\x06", "a ZIP archive"),
    (b"Rar!\x1A\x07", "a RAR archive"),
    (b"7z\xBC\xAF\x27\x1C", "a 7-Zip archive"),
    (b"MSCF", "a CAB archive"),
    (b"\x1F\x8B", "a GZIP archive"),
    (b"\xFD7zXZ", "an XZ archive"),
    (b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1", "an OLE container"),
    (b"L\0\0\0\x01\x14\x02\0", "a Windows shortcut"),
    (b"\0asm", "WebAssembly"),
    (b"\x1BLua", "Lua bytecode"),
    (b"\x1BLJ", "LuaJIT bytecode"),
];

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

/// MOD_CATALOG 8 for what a file starts with: `Err` names the program or container it is
/// (`BytesAllowed`).
pub fn bytes_allowed(data: &[u8]) -> Result<(), &'static str> {
    if data.len() >= 0x40 && data.starts_with(b"MZ") {
        let offset = u32_at(data, 0x3C).unwrap_or(0) as usize;
        if offset.checked_add(4).is_some_and(|e| e <= data.len()) && data.get(offset..offset + 4) == Some(b"PE\0\0") {
            return Err("a Windows program or library");
        }
    }
    match MAGICS.iter().find(|(magic, _)| data.starts_with(magic)) {
        Some((_, what)) => Err(what),
        None => Ok(()),
    }
}

fn is_script(path: &str) -> bool {
    path.len() > 7 && path.get(path.len() - 7..).is_some_and(|e| e.eq_ignore_ascii_case(".script"))
}

/// How much of a file [`check_content`] reads: all of a script (16 MiB at most), the first
/// 64 KiB of anything else (`ContentHead`).
#[must_use]
pub fn content_head(path: &str, size: u64) -> u64 {
    let limit = if is_script(path) { 16 << 20 } else { 64 << 10 };
    size.min(limit)
}

/// The head read so far (`got` of `size` bytes) may point further: an MZ header names where its
/// PE header is. The bytes [`check_content`] needs, `got` when the head is enough
/// (`ContentNeed`).
#[must_use]
pub fn content_need(head: &[u8], size: u64) -> u64 {
    let got = head.len() as u64;
    if head.len() < 0x40 || !head.starts_with(b"MZ") {
        return got;
    }
    let need = u64::from(u32_at(head, 0x3C).unwrap_or(0)) + 4;
    if need > got && need <= size { need } else { got }
}

/// One file of a module against 8 and, for a script, 10.3 (`CheckContent`). `path` is
/// module-relative with `/`; a non-script needs only its first bytes.
pub fn check_content(path: &str, data: &[u8], out: &mut Vec<Finding>) {
    if let Err(why) = path_allowed(path) {
        out.push(Finding::new(Level::Block, "content", format!("{path}: {why}")));
        return;
    }
    if let Err(why) = bytes_allowed(data) {
        out.push(Finding::new(Level::Block, "content", format!("{path}: {why}")));
        return;
    }
    if is_script(path) {
        script_report(path, data, out);
    }
}

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
            let hex = c == b'0' && q + 2 < end && matches!(t.get(q + 1), Some(b'x' | b'X')) && (xdigit_at(q + 2) || t.get(q + 2) == Some(&b'.'));
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
        let mut n = LONGER.iter().find(|op| t.get(p..).is_some_and(|rest| rest.starts_with(op))).map_or(0, |op| op.len());
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
    (b"StartDedicatedServer", None, "StartDedicatedServer"),
];

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

/// 10.3 over one script (`ScriptReport`): what only a module that runs unprotected would do.
pub fn script_report(path: &str, text: &[u8], out: &mut Vec<Finding>) {
    let tokens = tokenize(text);
    let add = |out: &mut Vec<Finding>, level: Level, line: u32, what: &str| {
        out.push(Finding::new(level, "script", format!("{path}:{line} {what}")));
    };
    let text_at = |i: usize| -> &[u8] { tokens.get(i).map_or(&[][..], |t| t.text) };
    let kind_at = |i: usize| tokens.get(i).map(|t| t.kind);

    for (i, token) in tokens.iter().enumerate() {
        if token.kind != Kind::Name {
            continue;
        }
        if i > 0 && matches!(text_at(i - 1), b"." | b":") {
            continue;
        }
        // the dotted name starting here: a, a.b, a:b
        let mut parts: Vec<&[u8]> = vec![token.text];
        let mut after = i + 1;
        while after + 1 < tokens.len() && matches!(text_at(after), b"." | b":") && kind_at(after + 1) == Some(Kind::Name) {
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
        if first == b"require" && text_at(after) == b"(" && kind_at(after + 1) == Some(Kind::String) && contains(text_at(after + 1), b"ffi") {
            add(out, Level::Block, line, "loads ffi");
        }
        if matches!(first, b"load" | b"loadstring" | b"require" | b"dofile" | b"loadfile") && text_at(after) == b"(" {
            let literal = kind_at(after + 1) == Some(Kind::String) && matches!(text_at(after + 2), b")" | b",");
            if !literal {
                add(out, Level::Review, line, &format!("{} of a computed string", String::from_utf8_lossy(first)));
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
        let console = (n >= 2 && parts.get(n - 2) == Some(&&b"get_console"[..]) && parts.last() == Some(&&b"execute"[..]))
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
                add(out, Level::Review, line, &format!("builds a string of {} characters with string.char", commas + 1));
            }
        }
    }
    for t in &tokens {
        if t.kind == Kind::String && t.text.len() > 2000 && encoded_blob(t.text) {
            add(out, Level::Review, t.line, &format!("holds a long encoded blob ({} characters)", t.text.len()));
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn findings(path: &str, data: &[u8]) -> Vec<(Level, String)> {
        let mut out = Vec::new();
        check_content(path, data, &mut out);
        out.into_iter().map(|f| (f.level, f.text)).collect()
    }

    #[test]
    fn paths() {
        assert!(path_allowed("gamedata/configs/mod.ltx").is_ok());
        assert!(path_allowed("README").is_ok());
        assert!(path_allowed("Textures/A.DDS").is_ok());
        assert_eq!(path_allowed("bin/evil.dll").unwrap_err(), ".dll files are not allowed in a Mod Browser module");
        assert_eq!(path_allowed("a/.git/config").unwrap_err(), "a dot-named file or folder");
        assert_eq!(path_allowed("a//b").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("/a").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("a\\b").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("c:x").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("a/../b").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("gamedata.db").unwrap_err(), ".db files are not allowed in a Mod Browser module");
    }

    #[test]
    fn magic_bytes() {
        let mut pe = vec![0u8; 0x100];
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        assert_eq!(findings("qa_disguised/program.txt", &pe), [(Level::Block, "qa_disguised/program.txt: a Windows program or library".to_owned())]);
        // an MZ header alone is no program
        let mut mz = pe.clone();
        mz[0x80] = b'X';
        assert!(findings("a.txt", &mz).is_empty());
        assert_eq!(findings("a.txt", b"PK\x03\x04rest"), [(Level::Block, "a.txt: a ZIP archive".to_owned())]);
        assert_eq!(findings("s.ltx", b"\x1BLJ\x02"), [(Level::Block, "s.ltx: LuaJIT bytecode".to_owned())]);
        assert_eq!(findings("m.ogg", b"\x1F\x8B\x08"), [(Level::Block, "m.ogg: a GZIP archive".to_owned())]);
        // the head reaches the PE header the MZ header names
        assert_eq!(content_need(&pe[..0x40], 0x100), 0x84);
        assert_eq!(content_need(&pe[..0x40], 0x50), 0x40);
        assert_eq!(content_head("x.script", 50 << 20), 16 << 20);
        assert_eq!(content_head("x.dds", 50 << 20), 64 << 10);
        assert_eq!(content_head("x.dds", 10), 10);
    }

    #[test]
    fn the_script_report() {
        let script = b"-- a comment os.execute('x')\n\
            local f = loadstring(code)\n\
            local g = loadstring(\"return 1\")\n\
            os.execute(\"calc\")\n\
            local x = io.popen\n\
            local d = debug.getinfo(1)\n\
            _G[name] = 1\n\
            get_console():execute(\"quit\")\n\
            ini:save_as(\"x\")\n\
            local ffi = require(\"ffi\")\n\
            --[[ long\n comment ]] setfenv(1, {})\n\
            local s = \"os.execute\"\n\
            local t = a.os.execute\n";
        let got = findings("gamedata/scripts/qa_bad.script", script);
        let p = "gamedata/scripts/qa_bad.script";
        let want = [
            (Level::Review, format!("{p}:2 loadstring of a computed string")),
            (Level::Block, format!("{p}:4 uses os.execute")),
            (Level::Block, format!("{p}:5 uses io.popen")),
            (Level::Review, format!("{p}:6 uses debug.*")),
            (Level::Review, format!("{p}:7 indexes _G by a computed name")),
            (Level::Review, format!("{p}:8 runs a console command")),
            (Level::Review, format!("{p}:9 uses ini_file:save_as")),
            (Level::Block, format!("{p}:10 uses ffi")),
            (Level::Block, format!("{p}:10 loads ffi")),
            (Level::Review, format!("{p}:12 uses setfenv")),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn every_magic_and_every_pattern() {
        for (magic, what) in MAGICS {
            let mut data = magic.to_vec();
            data.extend_from_slice(b"rest of the file");
            assert_eq!(findings("x.txt", &data), [(Level::Block, format!("x.txt: {what}"))]);
        }
        let lines = [
            ("package.loadlib(a, b)", Level::Block, "uses package.loadlib"),
            ("string.dump(f)", Level::Block, "uses string.dump"),
            ("lfs.link(a, b)", Level::Block, "uses lfs.link"),
            ("StartDedicatedServer()", Level::Block, "uses StartDedicatedServer"),
            ("getfenv(1)", Level::Review, "uses getfenv"),
            ("fs:w_open(a, b)", Level::Review, "uses getFS():w_open"),
            ("fs:file_delete(a)", Level::Review, "uses getFS():file_delete"),
            ("fs:file_rename(a, b)", Level::Review, "uses getFS():file_rename"),
            ("fs:file_copy(a, b)", Level::Review, "uses getFS():file_copy"),
            ("fs:dir_delete(a)", Level::Review, "uses getFS():dir_delete"),
            ("dofile(name)", Level::Review, "dofile of a computed string"),
            ("loadfile(name)", Level::Review, "loadfile of a computed string"),
            ("require(name)", Level::Review, "require of a computed string"),
            ("load(\"a\" .. b)", Level::Review, "load of a computed string"),
            ("local c = get_console() c:execute('x') get_console():execute('y')", Level::Review, "runs a console command"),
        ];
        for (code, level, what) in lines {
            let got = findings("s.script", code.as_bytes());
            assert!(got.contains(&(level, format!("s.script:1 {what}"))), "{code}: {got:?}");
        }
        // what a literal name loads is no finding, a string that only names a call neither
        assert!(findings("s.script", b"require(\"lua_help\")\nlocal s = 'os.execute'\n").is_empty());
    }

    #[test]
    fn chains_blobs_and_numbers() {
        let chars: Vec<String> = (0..17).map(|i| (65 + i).to_string()).collect();
        let script = format!("local s = string.char({})\n", chars.join(", "));
        assert_eq!(findings("a.script", script.as_bytes()), [(Level::Review, "a.script:1 builds a string of 17 characters with string.char".to_owned())]);
        let short = "local s = string.char(65, 66)\n";
        assert!(findings("a.script", short.as_bytes()).is_empty());
        let blob = format!("local b = \"{}\"\n", "QUJD".repeat(600));
        assert_eq!(findings("b.script", blob.as_bytes()), [(Level::Review, format!("b.script:1 holds a long encoded blob ({} characters)", 2402))]);
        // numbers and exponents do not swallow names; 0x1p4 is one number
        let tokens = tokenize(b"x = 0x1p4 + 1e e2 .5");
        let texts: Vec<&[u8]> = tokens.iter().map(|t| t.text).collect();
        assert_eq!(texts, [&b"x"[..], b"=", b"0x1p4", b"+", b"1", b"e", b"e2", b".5"]);
        // an unterminated quote is a stray character, the rest still reads
        let tokens = tokenize(b"a = \"open\nos.execute");
        assert!(tokens.iter().any(|t| t.text == b"execute" && t.line == 2));
    }
}
