//! A text file's bytes as text and back. Inside, a text holds `\n` line ends only; the file's
//! own encoding and line endings come back as it is written, and a line the author did not touch
//! keeps the ending it had.

use std::fmt;

use encoding_rs::WINDOWS_1251;

use crate::merge::same_lines;

/// Above this a file is not opened as text.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How a file's characters are bytes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Encoding {
    #[default]
    Utf8,
    /// UTF-8 that starts with its byte order mark.
    Utf8Bom,
    /// What the game reads its scripts, configs and XML in.
    Cp1251,
}

impl Encoding {
    /// `utf-8`, `utf-8-bom`, `windows-1251`: the word answers and parameters use.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Utf8 => "utf-8",
            Self::Utf8Bom => "utf-8-bom",
            Self::Cp1251 => "windows-1251",
        }
    }

    /// What the window calls it.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Utf8 => "UTF-8",
            Self::Utf8Bom => "UTF-8 with BOM",
            Self::Cp1251 => "Windows-1251",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Utf8, Self::Utf8Bom, Self::Cp1251].into_iter().find(|e| e.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// How a file ends its lines.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Eol {
    #[default]
    Lf,
    Crlf,
}

impl Eol {
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::Lf => "lf",
            Self::Crlf => "crlf",
        }
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Lf => "LF",
            Self::Crlf => "CRLF",
        }
    }

    #[must_use]
    pub fn from_id(id: &str) -> Option<Self> {
        [Self::Lf, Self::Crlf].into_iter().find(|e| e.id().eq_ignore_ascii_case(id.trim()))
    }
}

/// Why a file is not opened as text.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReadError {
    /// Its first bytes hold a NUL: it is binary.
    NotText,
    /// Over [`MAX_BYTES`].
    TooLarge(u64),
}

impl fmt::Display for ReadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotText => f.write_str("the file is no text: it holds binary data"),
            Self::TooLarge(n) => write!(
                f,
                "the file is {} MB, over the {} MB a text is opened up to",
                n / (1024 * 1024),
                MAX_BYTES / (1024 * 1024)
            ),
        }
    }
}

/// A character the encoding cannot hold, where it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EncodeError {
    pub line: u32,
    pub column: u32,
    pub ch: char,
}

impl fmt::Display for EncodeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Line {} has '{}', which Windows-1251 cannot hold", self.line, self.ch)
    }
}

/// A file read as text: the text with `\n` line ends, and what it was as bytes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TextFile {
    pub text: String,
    /// The encoding the file was read in; for a file of ASCII alone, the one the game reads a
    /// file of its name in (its bytes are the same in either).
    pub encoding: Encoding,
    /// The ending most of its lines have; for a file of one line, the one its kind is written
    /// with.
    pub eol: Eol,
    /// Every byte was ASCII and there was no byte order mark: the bytes said nothing of the
    /// encoding, and the first other character takes the one the game reads (see
    /// [`TextFile::encoding_for`]).
    pub ascii: bool,
    /// Per line of `text` that has an ending, whether the file ended it with CRLF.
    ends: Vec<bool>,
}

const BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// The extensions of the files that are text by their name.
const TEXT_EXTENSIONS: &[&str] = &[
    "script",
    "lua",
    "s",
    "ltx",
    "ltxp",
    "ini",
    "cfg",
    "xml",
    "xmlp",
    "html",
    "htm",
    "md",
    "json",
    "txt",
    "diff",
    "patch",
    "log",
    "csv",
    "tsv",
    "yaml",
    "yml",
    "toml",
    "gitignore",
    "gitattributes",
    "editorconfig",
];

/// Whole names of files that are text.
const TEXT_NAMES: &[&str] = &["mod.ltx", "AGENTS.md", "README"];

/// What the game reads in Windows-1251: scripts, configs, patches.
const CP1251_EXTENSIONS: &[&str] = &["script", "lua", "s", "ltx", "ltxp", "ini", "cfg", "xmlp"];

/// The extension of a file name, without the dot (`""` for none).
fn extension(name: &str) -> &str {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    base.rsplit_once('.').map_or("", |(_, e)| e)
}

fn base_name(name: &str) -> &str {
    name.rsplit(['/', '\\']).next().unwrap_or(name)
}

/// Whether a file's name says it is text (its extension).
#[must_use]
pub fn is_text_name(name: &str) -> bool {
    let base = base_name(name);
    let ext = extension(base);
    TEXT_NAMES.iter().any(|n| n.eq_ignore_ascii_case(base))
        || TEXT_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext))
}

/// Whether the first bytes of a file are text: no NUL in its first 8 KB.
#[must_use]
pub fn sniff(head: &[u8]) -> bool {
    !head.iter().take(8192).any(|&b| b == 0)
}

/// Whether an XML text's declaration names UTF-8 (`<?xml version="1.0" encoding="utf-8"?>`).
fn declares_utf8(text: &str) -> bool {
    let head: String = text.chars().take(256).collect::<String>().to_ascii_lowercase();
    let Some(at) = head.find("<?xml") else { return false };
    let decl = head.get(at..).and_then(|r| r.find("?>").and_then(|e| r.get(..e))).unwrap_or("");
    decl.contains("utf-8") || decl.contains("utf8")
}

/// The line ending a file of this name is written with when nothing else says: CRLF for what
/// the game reads in Windows-1251, LF otherwise.
fn default_eol(name: &str) -> Eol {
    if default_encoding(name, "") == Encoding::Cp1251 { Eol::Crlf } else { Eol::Lf }
}

/// A file's bytes as text: UTF-8 (with or without a BOM) when they read as it, Windows-1251
/// otherwise. `name` is the file's name (its extension says what an ASCII file would gain).
pub fn decode(bytes: &[u8], name: &str) -> Result<TextFile, ReadError> {
    let size = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if size > MAX_BYTES {
        return Err(ReadError::TooLarge(size));
    }
    if !sniff(bytes) {
        return Err(ReadError::NotText);
    }
    let (raw, encoding) = match bytes.strip_prefix(BOM).map(std::str::from_utf8) {
        Some(Ok(body)) => (body.to_owned(), Encoding::Utf8Bom),
        _ => match std::str::from_utf8(bytes) {
            Ok(text) => (text.to_owned(), Encoding::Utf8),
            Err(_) => (gz_base::text::cp1251_to_utf8(bytes).into_owned(), Encoding::Cp1251),
        },
    };
    let ascii = bytes.is_ascii();
    // a line break is LF; the CR before one is the file's, kept per line
    let mut text = String::with_capacity(raw.len());
    let mut ends = Vec::new();
    let mut rest = raw.as_str();
    while let Some(at) = rest.find('\n') {
        let line = rest.get(..at).unwrap_or("");
        let crlf = line.ends_with('\r');
        text.push_str(if crlf { line.get(..line.len() - 1).unwrap_or("") } else { line });
        text.push('\n');
        ends.push(crlf);
        rest = rest.get(at + 1..).unwrap_or("");
    }
    text.push_str(rest);
    let crlf = ends.iter().filter(|&&c| c).count();
    let eol = match crlf.cmp(&(ends.len() - crlf)) {
        std::cmp::Ordering::Greater => Eol::Crlf,
        std::cmp::Ordering::Equal if crlf > 0 => Eol::Crlf,
        std::cmp::Ordering::Equal => default_eol(name),
        std::cmp::Ordering::Less => Eol::Lf,
    };
    let encoding = if ascii { default_encoding(name, &text) } else { encoding };
    Ok(TextFile { text, encoding, eol, ascii, ends })
}

/// The encoding the game reads a file of this name in, for a text that is no longer ASCII:
/// Windows-1251 for a script, a config, a patch and XML that declares no UTF-8; UTF-8 otherwise
/// (the module manifest, which the game reads as UTF-8 first, Markdown, JSON, the editor's own
/// assets).
#[must_use]
pub fn default_encoding(name: &str, text: &str) -> Encoding {
    let base = base_name(name);
    let ext = extension(base);
    if base.eq_ignore_ascii_case("mod.ltx") {
        return Encoding::Utf8;
    }
    if CP1251_EXTENSIONS.iter().any(|e| e.eq_ignore_ascii_case(ext)) {
        return Encoding::Cp1251;
    }
    if ext.eq_ignore_ascii_case("xml") && !declares_utf8(text) {
        return Encoding::Cp1251;
    }
    Encoding::Utf8
}

/// `text` in Windows-1251, or where the first character it cannot hold is.
fn to_cp1251(text: &str) -> Result<Vec<u8>, EncodeError> {
    let (bytes, _, bad) = WINDOWS_1251.encode(text);
    if !bad {
        return Ok(bytes.into_owned());
    }
    let mut buf = [0u8; 4];
    for (n, line) in text.split('\n').enumerate() {
        for (c, ch) in line.chars().enumerate() {
            if ch.is_ascii() {
                continue;
            }
            if WINDOWS_1251.encode(ch.encode_utf8(&mut buf)).2 {
                let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
                let column = u32::try_from(c + 1).unwrap_or(u32::MAX);
                return Err(EncodeError { line, column, ch });
            }
        }
    }
    Err(EncodeError { line: 1, column: 1, ch: '?' })
}

impl TextFile {
    /// A file that is not on the disk yet: nothing in it, the name's encoding.
    #[must_use]
    pub fn new_file(name: &str) -> Self {
        Self {
            text: String::new(),
            encoding: default_encoding(name, ""),
            eol: default_eol(name),
            ascii: true,
            ends: Vec::new(),
        }
    }

    /// The encoding a save writes `text` in: the file's, unless the file was ASCII and `text`
    /// is not, then the one the game reads a file of this name in.
    #[must_use]
    pub fn encoding_for(&self, name: &str, text: &str) -> Encoding {
        if self.ascii && !text.is_ascii() { default_encoding(name, text) } else { self.encoding }
    }

    /// `text` as the file's bytes, in `encoding`, its lines ended with `eol` - a line that is
    /// the same as one of the file's keeps that line's own ending when `eol` is the file's.
    pub fn encode(&self, text: &str, encoding: Encoding, eol: Eol) -> Result<Vec<u8>, EncodeError> {
        let lines: Vec<&str> = text.split('\n').collect();
        // per line of `text` with an ending: whether it is CRLF
        let mut crlf = vec![eol == Eol::Crlf; lines.len() - 1];
        if eol == self.eol && self.ends.iter().any(|&e| e != (eol == Eol::Crlf)) {
            let old: Vec<&str> = self.text.split('\n').collect();
            for (i, j) in same_lines(&old, &lines) {
                if let (Some(&own), Some(slot)) = (self.ends.get(i), crlf.get_mut(j)) {
                    *slot = own;
                }
            }
        }
        let mut joined = String::with_capacity(text.len() + crlf.iter().filter(|&&c| c).count());
        for (n, line) in lines.iter().enumerate() {
            joined.push_str(line);
            match crlf.get(n) {
                Some(true) => joined.push_str("\r\n"),
                Some(false) => joined.push('\n'),
                None => {}
            }
        }
        match encoding {
            Encoding::Utf8 => Ok(joined.into_bytes()),
            Encoding::Utf8Bom => {
                let mut out = Vec::with_capacity(joined.len() + 3);
                out.extend_from_slice(BOM);
                out.extend_from_slice(joined.as_bytes());
                Ok(out)
            }
            Encoding::Cp1251 => to_cp1251(&joined),
        }
    }

    /// The line endings as the file has them: per line with an ending, whether it is CRLF.
    #[must_use]
    pub fn line_ends(&self) -> &[bool] {
        &self.ends
    }

    /// Whether the file's lines end in more than one way.
    #[must_use]
    pub fn mixed_endings(&self) -> bool {
        self.ends.windows(2).any(|w| w.first() != w.get(1))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    fn round_trip(bytes: &[u8], name: &str) -> TextFile {
        let f = decode(bytes, name).unwrap();
        assert_eq!(f.encode(&f.text, f.encoding, f.eol).unwrap(), bytes, "{name}: {bytes:?}");
        f
    }

    #[test]
    fn every_file_that_reads_is_written_back_as_it_was() {
        let f = round_trip(b"a\r\nb\nc\r\nd", "a.script");
        assert_eq!(
            (f.text.as_str(), f.eol, f.line_ends()),
            ("a\nb\nc\nd", Eol::Crlf, &[true, false, true][..])
        );
        assert!(f.mixed_endings());
        let f = round_trip(b"\xEF\xBB\xBFlocal a = 1\n", "a.lua");
        assert_eq!((f.encoding, f.text.as_str(), f.ascii), (Encoding::Utf8Bom, "local a = 1\n", false));
        // Windows-1251 Cyrillic
        let f = round_trip(b"name = \xcf\xf0\xe8\xe2\xe5\xf2\r\n", "a.ltx");
        assert_eq!(
            (f.encoding, f.text.as_str()),
            (Encoding::Cp1251, "name = \u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\n")
        );
        // UTF-8 Cyrillic
        let f = round_trip("# \u{41f}\u{440}\u{438}\n".as_bytes(), "a.md");
        assert_eq!(f.encoding, Encoding::Utf8);
        // no last line break, a lone CR, a CR before a CRLF
        let f = round_trip(b"x\ry\r\r\nz", "a.txt");
        assert_eq!((f.text.as_str(), f.line_ends()), ("x\ry\r\nz", &[true][..]));
        round_trip(b"", "a.txt");
        round_trip(b"\n\n\r\n", "a.txt");
        round_trip(b"\xEF\xBB\xBF", "a.txt");
        // a BOM before bytes that are no UTF-8: all of it Windows-1251
        assert_eq!(round_trip(b"\xEF\xBB\xBF\xff", "a.txt").encoding, Encoding::Cp1251);
        // every byte Windows-1251 has
        let all: Vec<u8> = (1..=255u8).collect();
        assert_eq!(round_trip(&all, "a.script").encoding, Encoding::Cp1251);
    }

    #[test]
    fn a_line_nobody_touched_keeps_its_own_ending() {
        let f = decode(b"one\r\ntwo\nthree\r\nfour\n", "a.script").unwrap();
        let edited = f.text.replace("two", "TWO").replace("four\n", "four\nfive\n");
        let out = f.encode(&edited, f.encoding, f.eol).unwrap();
        assert_eq!(out, b"one\r\nTWO\r\nthree\r\nfour\nfive\r\n");
        // another ending asked for: every line takes it
        assert_eq!(f.encode(&f.text, f.encoding, Eol::Lf).unwrap(), b"one\ntwo\nthree\nfour\n");
        // a line moved keeps its ending where the text still has it
        let moved = "two\none\nthree\nfour\n";
        assert_eq!(f.encode(moved, f.encoding, f.eol).unwrap(), b"two\none\r\nthree\r\nfour\n");
    }

    #[test]
    fn a_character_windows_1251_lacks_refuses_the_save_by_line() {
        let f = decode(b"a\r\n", "a.script").unwrap();
        let e = f.encode("a\nb = '\u{432}\u{2603}'\n", Encoding::Cp1251, Eol::Crlf).unwrap_err();
        assert_eq!(e, EncodeError { line: 2, column: 7, ch: '\u{2603}' });
        assert!(e.to_string().starts_with("Line 2 has"), "{e}");
        assert_eq!(f.encode("\u{432}", Encoding::Cp1251, Eol::Crlf).unwrap(), b"\xe2");
        assert_eq!(f.encode("\u{2603}", Encoding::Utf8Bom, Eol::Lf).unwrap(), "\u{feff}\u{2603}".as_bytes());
    }

    #[test]
    fn an_ascii_file_takes_the_encoding_the_game_reads_it_in() {
        let f = decode(b"local a = 1\r\n", "a.script").unwrap();
        assert!(f.ascii);
        assert_eq!(f.encoding, Encoding::Cp1251);
        assert_eq!(f.encoding_for("a.script", "-- \u{432}"), Encoding::Cp1251);
        let f = decode(b"{}", "a.json").unwrap();
        assert_eq!((f.encoding_for("a.json", "\u{432}"), f.eol), (Encoding::Utf8, Eol::Lf));
        // a file that was not ASCII keeps its own
        let f = decode("\u{432}".as_bytes(), "a.script").unwrap();
        assert_eq!(f.encoding_for("a.script", "\u{432}"), Encoding::Utf8);
        assert_eq!(
            default_encoding("ui/a.xml", "<?xml version=\"1.0\" encoding=\"windows-1251\"?>"),
            Encoding::Cp1251
        );
        assert_eq!(
            default_encoding("ui/a.xml", "<?xml version=\"1.0\" encoding=\"UTF-8\" ?>\n<w/>"),
            Encoding::Utf8
        );
        assert_eq!(default_encoding("ui/a.xml", ""), Encoding::Cp1251);
        assert_eq!(default_encoding("MOD.LTX", ""), Encoding::Utf8);
        assert_eq!(default_encoding("patch/a.ltxp", ""), Encoding::Cp1251);
        assert_eq!(default_encoding("quests/a.nqasset", ""), Encoding::Utf8);
        let new = TextFile::new_file("scripts/a.script");
        assert_eq!(
            (new.encoding, new.eol, new.ascii, new.text.as_str()),
            (Encoding::Cp1251, Eol::Crlf, true, "")
        );
        let new = TextFile::new_file("notes.md");
        assert_eq!((new.encoding, new.eol), (Encoding::Utf8, Eol::Lf));
        assert_eq!(new.encode("a\nb", new.encoding, new.eol).unwrap(), b"a\nb");
    }

    #[test]
    fn what_is_text() {
        for name in [
            "a.script",
            "B.LUA",
            "x/y.s",
            "mod.ltx",
            "p.ltxp",
            "a.ini",
            "a.cfg",
            "ui.xml",
            "a.xmlp",
            "a.html",
            "README.md",
            "a.json",
            "a.txt",
            "a.diff",
            ".gitignore",
            "a.yml",
            "README",
            "readme",
            "agents.md",
        ] {
            assert!(is_text_name(name), "{name}");
        }
        for name in ["a.dds", "a.ogf", "a.nqasset", "script", "a.scene"] {
            assert!(!is_text_name(name), "{name}");
        }
        assert_eq!(decode(b"a\0b", "a.txt"), Err(ReadError::NotText));
        assert!(sniff(&[b'a'; 9000]) && !sniff(b"\0"));
        let mut big = vec![b'a'; 8192];
        big.push(0);
        assert!(sniff(&big), "only the first 8 KB are looked at");
        let huge = vec![b'a'; usize::try_from(MAX_BYTES).unwrap() + 1];
        assert_eq!(decode(&huge, "a.txt"), Err(ReadError::TooLarge(MAX_BYTES + 1)));
        assert_eq!(Encoding::from_id(" UTF-8-BOM"), Some(Encoding::Utf8Bom));
        assert_eq!(Eol::from_id("CRLF"), Some(Eol::Crlf));
    }
}
