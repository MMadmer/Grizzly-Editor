//! The Mod Browser's content rules (MOD_CATALOG 8) and the script report (10.3), the same
//! tokenizer, rules and messages as the catalog's own `tools/catalog.py`):
//! change both or neither. A module outside the catalog ships what it
//! likes, so the release is written either way; the findings are warnings and
//! `catalog_content` in the answer.

use gz_mod::code::{CodeKind, code_kind};
use serde::Serialize;

/// The code of the finding that says a file works with the network (10.4): its text is
/// `<path>:<line>`, its level a note - the game marks such a module, nothing refuses it.
pub const NETWORK: &str = "network";

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
    "ltx",
    "ltxp",
    "xml",
    "xmlp",
    "script",
    "txt",
    "md",
    "json",
    "csv",
    "ini",
    "cfg",
    "seq",
    "gif",
    "mp4",
    "webm",
    "wmv",
    "html",
    "htm",
    "css",
    "png",
    "jpg",
    "jpeg",
    "nqasset",
    "behasset",
    "npcasset",
    "pdaasset",
    "texasset",
    "sndasset",
    "meshasset",
    "itemasset",
    "tradeasset",
    "lootasset",
    "recipeasset",
    "xspawn",
    "xcform",
    "cut",
    "aiedit",
    "aimap",
    "dds",
    "thm",
    "ogg",
    "ogm",
    "ogf",
    "omf",
    "skl",
    "skls",
    "anm",
    "ppe",
    "pe",
    "pg",
    "xr",
    "ps",
    "vs",
    "gs",
    "hs",
    "ds",
    "cs",
    "h",
    "hlsl",
    "s",
    "spawn",
    "graph",
    "ai",
    "gct",
    "cform",
    "details",
    "env_mod",
    "fog_vol",
    "game",
    "geom",
    "geomx",
    "hom",
    "ps_static",
    "snd_env",
    "snd_static",
    "som",
    "sectors",
    "wallmarks",
    "lanims",
];

/// MOD_CATALOG 8 for a path (module-relative, `/`): `Err` says why not.
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

/// MOD_CATALOG 8 for what a file starts with: `Err` names the program or container it is.
pub fn bytes_allowed(data: &[u8]) -> Result<(), &'static str> {
    if data.len() >= 0x40 && data.starts_with(b"MZ") {
        let offset = u32_at(data, 0x3C).unwrap_or(0) as usize;
        if offset.checked_add(4).is_some_and(|e| e <= data.len())
            && data.get(offset..offset + 4) == Some(b"PE\0\0")
        {
            return Err("a Windows program or library");
        }
    }
    match MAGICS.iter().find(|(magic, _)| data.starts_with(magic)) {
        Some((_, what)) => Err(what),
        None => Ok(()),
    }
}

fn is_script(path: &str) -> bool {
    code_kind(path).is_some()
}

/// How much of a file [`check_content`] reads: all of a file that carries code (16 MiB at most),
/// the first 64 KiB of anything else.
#[must_use]
pub fn content_head(path: &str, size: u64) -> u64 {
    let limit = if is_script(path) { 16 << 20 } else { 64 << 10 };
    size.min(limit)
}

/// The head read so far (`got` of `size` bytes) may point further: an MZ header names where its
/// PE header is. The bytes [`check_content`] needs, `got` when the head is enough.
#[must_use]
pub fn content_need(head: &[u8], size: u64) -> u64 {
    let got = head.len() as u64;
    if head.len() < 0x40 || !head.starts_with(b"MZ") {
        return got;
    }
    let need = u64::from(u32_at(head, 0x3C).unwrap_or(0)) + 4;
    if need > got && need <= size { need } else { got }
}

/// One file of a module against 8 and, for a file that carries code, 10.3 and 10.4. `path` is
/// module-relative with `/`; any other file needs only its first bytes.
pub fn check_content(path: &str, data: &[u8], out: &mut Vec<Finding>) {
    if let Err(why) = path_allowed(path) {
        out.push(Finding::new(Level::Block, "content", format!("{path}: {why}")));
        return;
    }
    if let Err(why) = bytes_allowed(data) {
        out.push(Finding::new(Level::Block, "content", format!("{path}: {why}")));
        return;
    }
    if let Some(kind) = code_kind(path) {
        code_report(path, kind, data, out);
    }
}

/// 10.3 and 10.4 over one file that carries code: what only a module that runs unprotected would
/// do, and whether the code works with the network (a note, never a problem).
pub fn code_report(path: &str, kind: CodeKind, text: &[u8], out: &mut Vec<Finding>) {
    let report = gz_mod::code::report(kind, text);
    for f in report.findings {
        let level = if f.blocks { Level::Block } else { Level::Review };
        out.push(Finding::new(level, "script", format!("{path}:{} {}", f.line, f.what)));
    }
    if let Some(line) = report.network {
        out.push(Finding::new(Level::Note, NETWORK, format!("{path}:{line}")));
    }
}

/// 10.3 over one script.
pub fn script_report(path: &str, text: &[u8], out: &mut Vec<Finding>) {
    code_report(path, CodeKind::Script, text, out);
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
        // the media descriptions, the items and the level's cuts, AI edits and AI map, as the game's list has them
        for p in [
            "gamedata/textures/od/wall.texasset",
            "gamedata/sounds/od/hum.SndAsset",
            "gamedata/meshes/od/crate.meshasset",
            "items/wpn_sil_diy.ItemAsset",
            "levels/l01_escape/overlay.cut",
            "levels/l01_escape/overlay.aiedit",
            "levels/l01_escape/overlay.aimap",
        ] {
            assert!(path_allowed(p).is_ok(), "{p}");
        }
        assert_eq!(
            path_allowed("models/wall.glb").unwrap_err(),
            ".glb files are not allowed in a Mod Browser module"
        );
        // program-files: in memory
        let refused = path_allowed("bin/evil.dll").unwrap_err();
        // program-files: in memory
        assert_eq!(refused, ".dll files are not allowed in a Mod Browser module");
        assert_eq!(path_allowed("a/.git/config").unwrap_err(), "a dot-named file or folder");
        assert_eq!(path_allowed("a//b").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("/a").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("a\\b").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("c:x").unwrap_err(), "unsafe path");
        assert_eq!(path_allowed("a/../b").unwrap_err(), "unsafe path");
        assert_eq!(
            path_allowed("gamedata.db").unwrap_err(),
            ".db files are not allowed in a Mod Browser module"
        );
    }

    #[test]
    fn magic_bytes() {
        let mut pe = vec![0u8; 0x100];
        // program-files: in memory
        pe[0] = b'M';
        pe[1] = b'Z';
        pe[0x3C] = 0x80;
        pe[0x80..0x84].copy_from_slice(b"PE\0\0");
        assert_eq!(
            findings("qa_disguised/program.txt", &pe),
            [(Level::Block, "qa_disguised/program.txt: a Windows program or library".to_owned())]
        );
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
            (
                "local c = get_console() c:execute('x') get_console():execute('y')",
                Level::Review,
                "runs a console command",
            ),
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
        assert_eq!(
            findings("a.script", script.as_bytes()),
            [(Level::Review, "a.script:1 builds a string of 17 characters with string.char".to_owned())]
        );
        let short = "local s = string.char(65, 66)\n";
        assert!(findings("a.script", short.as_bytes()).is_empty());
        let blob = format!("local b = \"{}\"\n", "QUJD".repeat(600));
        assert_eq!(
            findings("b.script", blob.as_bytes()),
            [(Level::Review, format!("b.script:1 holds a long encoded blob ({} characters)", 2402))]
        );
    }

    #[test]
    fn every_file_that_carries_code_is_read_and_the_network_is_a_note() {
        // a shader script and an asset's code are scripts as much as a .script
        assert_eq!(
            findings("gamedata/shaders/r3/x.s", b"os.execute('x')"),
            [(Level::Block, "gamedata/shaders/r3/x.s:1 uses os.execute".to_owned())]
        );
        let asset = b"return { nodes = { { kind = \"lua\", code = \"xms.http.request(u, io.popen)\" } } }";
        let mut out = Vec::new();
        check_content("quests/a.nqasset", asset, &mut out);
        let got: Vec<(Level, &str, &str)> = out.iter().map(|f| (f.level, f.code, f.text.as_str())).collect();
        assert_eq!(
            got,
            [
                (Level::Block, "script", "quests/a.nqasset:1 uses io.popen"),
                (Level::Note, NETWORK, "quests/a.nqasset:1"),
            ]
        );
        assert_eq!(content_head("quests/a.nqasset", 1 << 20), 1 << 20);
    }
}
