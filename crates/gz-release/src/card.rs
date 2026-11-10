//! The descriptor's `[card]` (MOD_CATALOG 4): what the Mod Browser shows of a module that is not
//! installed - the manifest with every string table id resolved from the project, the mode, the
//! tags, the thumbnail and `mod.ltx` pinned by size and hash. Built as text exactly as
//! `EditorCatalog::CardSection` builds it: it is signed byte for byte.

use std::path::Path;

use gz_base::text::decode_guess;
use gz_mod::Manifest;

/// A file of the release the card names: the path as in the file index, size, SHA-256.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CardFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

/// The string tables of the project a card reads (`<project>\gamedata\configs\text\<lang>\*.xml`),
/// each file read once, in the order Windows lists a folder.
#[derive(Clone, Debug, Default)]
pub struct StringTables {
    rus: Vec<Vec<u8>>,
    eng: Vec<Vec<u8>>,
}

/// A string table file is read up to this size.
const TABLE_LIMIT: u64 = 16 << 20;

fn read_tables(dir: &Path) -> Vec<Vec<u8>> {
    let Ok(listing) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<(String, std::path::PathBuf)> = listing
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .filter_map(|e| {
            let name = e.file_name().to_str()?.to_owned();
            let xml = name.len() > 4 && name.get(name.len() - 4..).is_some_and(|x| x.eq_ignore_ascii_case(".xml"));
            xml.then(|| (name.to_ascii_lowercase(), e.path()))
        })
        .collect();
    // NTFS lists a folder in its name order, without case
    names.sort();
    names
        .into_iter()
        .filter_map(|(_, path)| {
            let size = std::fs::metadata(&path).ok()?.len();
            if size > TABLE_LIMIT {
                return None;
            }
            std::fs::read(&path).ok()
        })
        .collect()
}

impl StringTables {
    /// Reads the project's Russian and English tables. Blocking: call it on the IO pool.
    #[must_use]
    pub fn read(project_root: &Path) -> Self {
        let text = project_root.join("gamedata").join("configs").join("text");
        Self { rus: read_tables(&text.join("rus")), eng: read_tables(&text.join("eng")) }
    }

    /// Tables given as they are, for a test.
    #[must_use]
    pub fn from_texts(rus: Vec<Vec<u8>>, eng: Vec<Vec<u8>>) -> Self {
        Self { rus, eng }
    }

    /// `<string id="id"><text>...</text>` of a language, UTF-8 (`LookUpString`): the first file
    /// that holds the id at the first place it is named, the text as written there with the
    /// tables' own `\n` made a line break.
    #[must_use]
    pub fn lookup(&self, language: &str, id: &str) -> Option<String> {
        let files = if language == "eng" { &self.eng } else { &self.rus };
        let open = format!("id=\"{id}\"");
        for file in files {
            let Some(at) = find(file, open.as_bytes(), 0) else { continue };
            let Some(begin) = find(file, b"<text>", at) else { continue };
            let Some(end) = find(file, b"</text>", begin) else { continue };
            if find(file, b"<string", at + open.len()).is_some_and(|next| begin > next) {
                continue;
            }
            let raw = file.get(begin + 6..end).unwrap_or(&[]);
            let text = decode_guess(raw);
            return Some(text.replace("\\n", "\n"));
        }
        None
    }
}

fn find(hay: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() {
        return Some(from);
    }
    hay.get(from..)?.windows(needle.len()).position(|w| w == needle).map(|p| p + from)
}

/// A display text that could be a string table id: short, no blank, ASCII.
fn looks_like_id(text: &str) -> bool {
    !text.is_empty() && text.len() <= 128 && !text.bytes().any(|b| matches!(b, b' ' | b'\t' | b'\n') || b >= 0x80)
}

/// The Russian (or the only) text of a display value and the English one, if any.
fn resolve(tables: &StringTables, value: &str) -> (String, String) {
    let mut rus = value.to_owned();
    let mut eng = String::new();
    if !looks_like_id(value) {
        return (rus, eng);
    }
    if let Some(t) = tables.lookup("rus", value) {
        rus = t;
    }
    if let Some(t) = tables.lookup("eng", value) {
        eng = t;
    }
    if rus == value && !eng.is_empty() {
        rus.clone_from(&eng);
    }
    (rus, eng)
}

/// A card text in quotes: `"` and `\` escaped, a line break `\n`, CR dropped (`QuoteText`).
#[must_use]
pub fn quote_text(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            '\r' => {}
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn key_line(out: &mut String, key: &str, value: &str) {
    out.push_str(&format!("{key:<11} = {value}\r\n"));
}

fn card_text(out: &mut String, key: &str, tables: &StringTables, value: &str) {
    if value.is_empty() {
        return;
    }
    let (rus, eng) = resolve(tables, value);
    key_line(out, key, &quote_text(&rus));
    if !eng.is_empty() && eng != rus {
        key_line(out, &format!("{key}@eng"), &quote_text(&eng));
    }
}

/// The `[card]` section, starting with its blank line (`CardSection`).
#[must_use]
pub fn card_section(m: &Manifest, tables: &StringTables, manifest: &CardFile, thumbnail: Option<&CardFile>) -> String {
    let mut out = String::from("\r\n[card]\r\n");
    card_text(&mut out, "name", tables, if m.name.is_empty() { &m.id } else { &m.name });
    card_text(&mut out, "author", tables, &m.author);
    card_text(&mut out, "description", tables, &m.description);
    if !m.website.is_empty() {
        key_line(&mut out, "website", &m.website);
    }

    // the mode: the plain game, one the module brings, or one somebody else's
    let (mut mode, mut title) = ("plain".to_owned(), String::new());
    if !m.mode.is_empty() && m.mode != "*" {
        let mut own = false;
        for p in m.provides_modes.iter().filter(|p| p.id == m.mode) {
            own = true;
            title.clone_from(&p.title);
        }
        mode = format!("{}{}", if own { "new " } else { "mode " }, m.mode);
        if title.is_empty() {
            title.clone_from(&m.mode);
        }
    }
    key_line(&mut out, "mode", &mode);
    card_text(&mut out, "mode_title", tables, &title);

    if !m.tags.is_empty() {
        key_line(&mut out, "tags", &m.tags.join(", "));
    }
    if let Some(t) = thumbnail {
        key_line(&mut out, "thumbnail", &format!("{}, {}, {}", t.path, t.size, t.sha256));
    }
    key_line(&mut out, "manifest", &format!("{}, {}, {}", manifest.path, manifest.size, manifest.sha256));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_mod::{ManifestDoc, ProvidesMode};

    fn manifest(text: &str) -> Manifest {
        Manifest::parse(&ManifestDoc::parse(text))
    }

    #[test]
    fn the_card_as_the_cpp_writes_it() {
        let m = manifest("[module]\r\nid = od\r\nname = od_name\r\nauthor = Madmer\r\ndescription = \"Night shifts.\\nA \\\"new\\\" mode.\"\r\nwebsite = https://ap-pro.ru/stuff/x/overnight-delivery-r600/\r\nmode = od_mode\r\ntags = story, quests\r\n\r\n[provides_mode]\r\nid = od_mode\r\ntitle = od_mode_title\r\n");
        let rus = "<string_table>\r\n<string id=\"od_name\">\r\n<text>\u{41d}\u{43e}\u{447}\u{43d}\u{430}\u{44f} \u{434}\u{43e}\u{441}\u{442}\u{430}\u{432}\u{43a}\u{430}</text>\r\n</string>\r\n</string_table>";
        let eng = "<string id=\"od_name\"><text>Overnight Delivery</text></string><string id=\"od_mode_title\"><text>Night\\nShift</text></string>";
        let tables = StringTables::from_texts(vec![rus.as_bytes().to_vec()], vec![eng.as_bytes().to_vec()]);
        let mf = CardFile { path: "mod.ltx".into(), size: 812, sha256: "9a1b".into() };
        let th = CardFile { path: "thumbnail.dds".into(), size: 43832, sha256: "5e8f".into() };
        let card = card_section(&m, &tables, &mf, Some(&th));
        assert_eq!(
            card,
            "\r\n[card]\r\n\
             name        = \"\u{41d}\u{43e}\u{447}\u{43d}\u{430}\u{44f} \u{434}\u{43e}\u{441}\u{442}\u{430}\u{432}\u{43a}\u{430}\"\r\n\
             name@eng    = \"Overnight Delivery\"\r\n\
             author      = \"Madmer\"\r\n\
             description = \"Night shifts.\\nA \\\"new\\\" mode.\"\r\n\
             website     = https://ap-pro.ru/stuff/x/overnight-delivery-r600/\r\n\
             mode        = new od_mode\r\n\
             mode_title  = \"Night\\nShift\"\r\n\
             tags        = story, quests\r\n\
             thumbnail   = thumbnail.dds, 43832, 5e8f\r\n\
             manifest    = mod.ltx, 812, 9a1b\r\n"
        );
    }

    #[test]
    fn modes_and_plain_text() {
        let tables = StringTables::default();
        let mf = CardFile { path: "mod.ltx".into(), size: 1, sha256: "00".into() };
        let mut m = manifest("[module]\r\nid = od\r\n");
        assert_eq!(card_section(&m, &tables, &mf, None), "\r\n[card]\r\nname        = \"od\"\r\nmode        = plain\r\nmanifest    = mod.ltx, 1, 00\r\n");
        m.mode = "*".into();
        assert!(card_section(&m, &tables, &mf, None).contains("mode        = plain\r\n"));
        m.mode = "da_story".into();
        let other = card_section(&m, &tables, &mf, None);
        assert!(other.contains("mode        = mode da_story\r\nmode_title  = \"da_story\"\r\n"), "{other}");
        m.provides_modes = vec![ProvidesMode { id: "da_story".into(), title: String::new() }];
        assert!(card_section(&m, &tables, &mf, None).contains("mode        = new da_story\r\n"));
        // an id the tables do not hold stays as written; English alone stands for both
        let eng = StringTables::from_texts(Vec::new(), vec![b"<string id=\"x\"><text>Only English</text></string>".to_vec()]);
        assert_eq!(resolve(&eng, "x"), ("Only English".to_owned(), "Only English".to_owned()));
        assert_eq!(resolve(&eng, "two words"), ("two words".to_owned(), String::new()));
        // a <text> that belongs to the next string is not this one's
        let cut = StringTables::from_texts(vec![b"<string id=\"x\"></string><string id=\"y\"><text>Y</text></string>".to_vec()], Vec::new());
        assert_eq!(cut.lookup("rus", "x"), None);
    }
}
