//! String tables: `configs\text\<language>\*.xml`, `<string id=".."><text>..</text></string>`.
//!
//! - [`language`]: the language the editor reads, from `configs\localization.ltx`.
//! - [`is_table_file`] / [`order`]: which files make the table and in which order the game
//!   folds them (by name, the engine's own `grizzly.xml` / `openxray.xml` first; the caller then
//!   orders modules by layer, as the game does).
//! - [`file_strings`] / [`StringTable::build`]: `id -> text`, the files parsed in parallel and
//!   folded in order.
//!
//! Two readings, [`Mode`]:
//! - [`Mode::Editor`] is the editor's own scan (the
//!   captions of pickers and game modes): the text between `<text>` and `</text>` as written,
//!   trimmed, no entity decoded; inside one file and across files the first definition wins.
//! - [`Mode::Game`] is the game's string table: the XML tree, text condensed and decoded; a
//!   later definition overrides an earlier one, in a file and across files.
//!
//! In both, `\n` in a text stays the two characters `\` `n`; the game's UI breaks the line when
//! it draws ([`line_breaks`] does the same for a preview).

use std::borrow::Cow;
use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::doc::{Document, XmlWarning};

/// The engine's own string files, folded in before the game's.
pub const ENGINE_XML: [&str; 2] = ["grizzly.xml", "openxray.xml"];

/// How a table is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Editor,
    Game,
}

fn trim(s: &str) -> &str {
    s.trim_matches([' ', '\t', '\r', '\n'])
}

/// The language of `configs\localization.ltx` as the editor reads it: the first `language`,
/// its value up to the end of the line and before a `;`, trimmed; `rus` when there is none.
#[must_use]
pub fn language(localization_ltx: &str) -> String {
    let loc = localization_ltx;
    if let Some(at) = loc.find("language") {
        let eq = loc.get(at..).and_then(|r| r.find('=')).map(|e| at + e);
        let eol = loc.get(at..).and_then(|r| r.find('\n')).map(|e| at + e);
        if let Some(eq) = eq.filter(|&eq| eol.is_none_or(|eol| eq < eol)) {
            let mut v = trim(loc.get(eq + 1..eol.unwrap_or(loc.len())).unwrap_or(""));
            if let Some(sc) = v.find(';') {
                v = trim(v.get(..sc).unwrap_or(""));
            }
            if !v.is_empty() {
                return v.to_owned();
            }
        }
    }
    "rus".to_owned()
}

/// Whether a game path (`configs\text\rus\st_items.xml`) is a string table of `lang`; the
/// comparison ignores case, as the game's paths do.
#[must_use]
pub fn is_table_file(path: &str, lang: &str) -> bool {
    let prefix = format!("configs\\text\\{lang}\\");
    let p = path.as_bytes();
    p.len() > prefix.len() + 4
        && p.get(..prefix.len()).is_some_and(|h| h.eq_ignore_ascii_case(prefix.as_bytes()))
        && p.get(p.len() - 4..).is_some_and(|t| t.eq_ignore_ascii_case(b".xml"))
}

fn file_name(path: &str) -> &str {
    path.rsplit(['\\', '/']).next().unwrap_or(path)
}

/// Sorts table files as the game folds them: by name (lower case, byte order), the engine's
/// own files first.
pub fn order(files: &mut [String]) {
    files.sort_by_key(|f| f.to_ascii_lowercase());
    let is_engine = |f: &String| ENGINE_XML.iter().any(|e| file_name(f).eq_ignore_ascii_case(e));
    files.sort_by_key(|f| !is_engine(f));
}

/// One file's strings and what was wrong with it.
pub type FileStrings = (Vec<(String, String)>, Vec<XmlWarning>);

/// The entries of one file in file order, and what the reader read past.
#[must_use]
pub fn file_strings(text: &str, mode: Mode) -> (Vec<(String, String)>, Vec<XmlWarning>) {
    match mode {
        Mode::Editor => (editor_scan(text), Vec::new()),
        Mode::Game => game_read(text),
    }
}

/// A scan for `<string id="` ... `<text>` ... `</text>`.
fn editor_scan(text: &str) -> Vec<(String, String)> {
    const OPEN: &str = "<string id=\"";
    let find = |pat: &str, from: usize| text.get(from..).and_then(|r| r.find(pat)).map(|i| from + i);
    let mut out: Vec<(String, String)> = Vec::new();
    let mut seen: FxHashMap<&str, ()> = FxHashMap::default();
    let mut pos = 0;
    while let Some(at) = find(OPEN, pos) {
        let idb = at + OPEN.len();
        let Some(ide) = find("\"", idb) else { break };
        let open = find("<text>", ide);
        let next = find(OPEN, ide);
        pos = ide;
        let Some(open) = open.filter(|&o| next.is_none_or(|n| o <= n)) else { continue };
        let Some(close) = find("</text>", open) else { break };
        let id = text.get(idb..ide).unwrap_or("");
        if seen.insert(id, ()).is_none() {
            out.push((id.to_owned(), trim(text.get(open + 6..close).unwrap_or("")).to_owned()));
        }
        pos = close;
    }
    out
}

/// Every `string` of the root, its `id` and the text of `string:text`.
fn game_read(text: &str) -> (Vec<(String, String)>, Vec<XmlWarning>) {
    let doc = Document::parse(text);
    let mut warnings = doc.warnings().to_vec();
    let mut out = Vec::new();
    let Some(root) = doc.root() else { return (out, warnings) };
    let n = doc.count(Some(root), Some("string"));
    for i in 0..n {
        let node = doc.navigate(root, "string", i);
        let Some(id) = doc.attr(node, "id") else {
            warnings.push(XmlWarning { line: 0, message: format!("String #{} has no id.", i + 1) });
            continue;
        };
        let Some(value) = doc.read(doc.navigate(root, "string:text", i)) else {
            warnings.push(XmlWarning { line: 0, message: format!("The string '{id}' has no text.") });
            continue;
        };
        out.push((id.to_owned(), value.to_owned()));
    }
    (out, warnings)
}

/// An immutable `id -> text` map.
#[derive(Clone, Debug, Default)]
pub struct StringTable {
    map: FxHashMap<Arc<str>, Arc<str>>,
}

impl StringTable {
    /// Builds the table from `(name, text)` pairs already in fold order ([`order`]); the files
    /// are parsed in parallel on the caller's pool. Returns the warnings with their file.
    #[must_use]
    pub fn build(files: &[(&str, &str)], mode: Mode) -> (Self, Vec<(String, XmlWarning)>) {
        let parsed: Vec<FileStrings> = files.par_iter().map(|(_, text)| file_strings(text, mode)).collect();
        let mut map: FxHashMap<Arc<str>, Arc<str>> = FxHashMap::default();
        let mut warnings = Vec::new();
        for ((name, _), (entries, w)) in files.iter().zip(parsed) {
            warnings.extend(w.into_iter().map(|w| ((*name).to_owned(), w)));
            for (id, text) in entries {
                if mode == Mode::Editor && map.contains_key(id.as_str()) {
                    continue;
                }
                map.insert(Arc::from(id), Arc::from(text));
            }
        }
        (Self { map }, warnings)
    }

    #[must_use]
    pub fn get(&self, id: &str) -> Option<&str> {
        self.map.get(id).map(|v| &**v)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.map.iter().map(|(k, v)| (&**k, &**v))
    }
}

/// A text as the game's UI draws it: `\n` breaks the line.
#[must_use]
pub fn line_breaks(s: &str) -> Cow<'_, str> {
    if s.contains("\\n") { Cow::Owned(s.replace("\\n", "\n")) } else { Cow::Borrowed(s) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "<?xml version=\"1.0\" encoding=\"windows-1251\"?>\n<string_table>\n<string id=\"a\">\n  <text>  First\\nline &amp; more </text>\n</string>\n<string id=\"none\"></string>\n<string id=\"b\"><text>B1</text></string>\n<string id=\"b\"><text>B2</text></string>\n</string_table>";
    const B: &str = "<string_table><string id=\"a\"><text>Override</text></string></string_table>";

    #[test]
    fn language_of_localization() {
        assert_eq!(language("[string_table]\nlanguage = eng ; english\nfont_prefix = _west"), "eng");
        assert_eq!(language("[string_table]\nfont = x"), "rus");
        assert_eq!(language("language\n= eng"), "rus");
    }

    #[test]
    fn files_and_order() {
        assert!(is_table_file("Configs\\Text\\rus\\st_a.XML", "rus"));
        assert!(!is_table_file("configs\\text\\eng\\st_a.xml", "rus"));
        let mut f = vec!["c\\st_b.xml".to_owned(), "c\\Grizzly.xml".to_owned(), "c\\st_a.xml".to_owned()];
        order(&mut f);
        assert_eq!(f, ["c\\Grizzly.xml", "c\\st_a.xml", "c\\st_b.xml"]);
    }

    #[test]
    fn both_readings() {
        let (e, _) = file_strings(A, Mode::Editor);
        assert_eq!(
            e,
            [("a".to_owned(), "First\\nline &amp; more".to_owned()), ("b".to_owned(), "B1".to_owned())]
        );
        let (g, w) = file_strings(A, Mode::Game);
        assert_eq!(
            g,
            [
                ("a".to_owned(), "First\\nline & more".to_owned()),
                ("b".to_owned(), "B1".to_owned()),
                ("b".to_owned(), "B2".to_owned())
            ]
        );
        assert_eq!(w.len(), 1);
        let (t, _) = StringTable::build(&[("a.xml", A), ("b.xml", B)], Mode::Game);
        assert_eq!(t.get("a"), Some("Override"));
        assert_eq!(t.get("b"), Some("B2"));
        let (t, _) = StringTable::build(&[("a.xml", A), ("b.xml", B)], Mode::Editor);
        assert_eq!(t.get("a"), Some("First\\nline &amp; more"));
        assert_eq!(line_breaks("x\\ny"), "x\ny");
    }
}
