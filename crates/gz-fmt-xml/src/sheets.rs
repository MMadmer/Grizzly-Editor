//! Texture descriptions (`configs\ui\textures_descr\*.xml`): which sheet a UI texture id is
//! cut from, and where.
//!
//! The reading: comments dropped, then every `<file ...>` names the sheet of the
//! `<texture ...>` tags after it, whatever the nesting. A texture without an id, a sheet or a
//! positive size is no entry.
//!
//! - [`parse_descr`]: one file's entries in order.
//! - [`TextureSheets::build`]: `id -> (sheet, rect)` over many files (parsed in parallel), ids
//!   compared without case, a later description winning - the way the game lets a mod
//!   redefine an id.

use std::sync::Arc;

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::cnum::{atof, atoi};

/// One `<texture>` of a description, its numbers as written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureEntry {
    /// As written.
    pub id: String,
    pub sheet: String,
    pub x: String,
    pub y: String,
    pub width: String,
    pub height: String,
}

impl TextureEntry {
    /// `x, y, width, height` read with `atof`, as the PDA preview reads them.
    #[must_use]
    pub fn rect(&self) -> [f32; 4] {
        [&self.x, &self.y, &self.width, &self.height].map(|s| atof(s) as f32)
    }

    /// `x, y, width, height` read with `atoi`, as the pickers read them.
    #[must_use]
    pub fn rect_int(&self) -> [i32; 4] {
        [&self.x, &self.y, &self.width, &self.height].map(|s| atoi(s))
    }
}

/// The text without `<!-- ... -->` (an unclosed comment runs to the end).
fn strip_comments(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut pos = 0;
    while pos < raw.len() {
        let Some(c) = raw.get(pos..).and_then(|r| r.find("<!--")).map(|c| pos + c) else {
            text.push_str(raw.get(pos..).unwrap_or(""));
            break;
        };
        text.push_str(raw.get(pos..c).unwrap_or(""));
        pos = raw.get(c + 4..).and_then(|r| r.find("-->")).map_or(raw.len(), |e| c + 4 + e + 3);
    }
    text
}

/// The value of `name="..."` inside a tag, `""` when it is not there; a name that
/// is the tail of a longer one does not count.
fn tag_attr(tag: &str, name: &str) -> String {
    let key = format!("{name}=\"");
    let mut from = 0;
    while let Some(at) = tag.get(from..).and_then(|r| r.find(&key)).map(|i| from + i) {
        from = at + 1;
        if at > 0 && !matches!(tag.as_bytes().get(at - 1), Some(b' ' | b'\t' | b'\n' | b'\r')) {
            continue;
        }
        let b = at + key.len();
        return tag
            .get(b..)
            .and_then(|r| r.find('"'))
            .and_then(|e| tag.get(b..b + e))
            .unwrap_or("")
            .to_owned();
    }
    String::new()
}

/// The entries of one description, in order, before the size check.
fn scan(raw: &str) -> Vec<TextureEntry> {
    let text = strip_comments(raw);
    let find = |pat: &str, from: usize| text.get(from..).and_then(|r| r.find(pat)).map(|i| from + i);
    let mut sheet = String::new();
    let mut out = Vec::new();
    let mut pos = 0;
    loop {
        let (f, t) = (find("<file", pos), find("<texture", pos));
        let at = match (f, t) {
            (None, None) => break,
            (Some(f), None) => f,
            (None, Some(t)) => t,
            (Some(f), Some(t)) => f.min(t),
        };
        let Some(gt) = find(">", at) else { break };
        let tag = text.get(at..gt).unwrap_or("");
        pos = gt + 1;
        if Some(at) == f {
            sheet = tag_attr(tag, "name");
            continue;
        }
        out.push(TextureEntry {
            id: tag_attr(tag, "id"),
            sheet: sheet.clone(),
            x: tag_attr(tag, "x"),
            y: tag_attr(tag, "y"),
            width: tag_attr(tag, "width"),
            height: tag_attr(tag, "height"),
        });
    }
    out
}

/// The textures of one description that have an id, a sheet and a positive size (read as
/// integers, as the pickers check it).
#[must_use]
pub fn parse_descr(raw: &str) -> Vec<TextureEntry> {
    scan(raw)
        .into_iter()
        .filter(|e| {
            let [_, _, w, h] = e.rect_int();
            !e.id.is_empty() && !e.sheet.is_empty() && w > 0 && h > 0
        })
        .collect()
}

/// Where a texture id is: its sheet and its rectangle.
#[derive(Clone, Debug, PartialEq)]
pub struct TexRect {
    pub sheet: Arc<str>,
    pub rect: [f32; 4],
    /// Which of the files given to [`TextureSheets::build`] defined it.
    pub file: u32,
}

/// `id -> (sheet, rect)` over many descriptions.
#[derive(Clone, Debug, Default)]
pub struct TextureSheets {
    map: FxHashMap<Arc<str>, TexRect>,
}

impl TextureSheets {
    /// Builds the map from descriptions in order (the game's files, then the project's); a
    /// later one redefines an id (a positive size read as floats).
    #[must_use]
    pub fn build(files: &[&str]) -> Self {
        let parsed: Vec<Vec<TextureEntry>> = files.par_iter().map(|t| scan(t)).collect();
        let mut map = FxHashMap::default();
        for (i, entries) in parsed.into_iter().enumerate() {
            for e in entries {
                let rect = e.rect();
                if e.id.is_empty() || e.sheet.is_empty() || rect[2] <= 0.0 || rect[3] <= 0.0 {
                    continue;
                }
                let file = u32::try_from(i).unwrap_or(u32::MAX);
                map.insert(
                    Arc::from(e.id.to_ascii_lowercase()),
                    TexRect { sheet: Arc::from(e.sheet), rect, file },
                );
            }
        }
        Self { map }
    }

    /// A texture by id, any case.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&TexRect> {
        self.map.get(id.to_ascii_lowercase().as_str())
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.map.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.map.is_empty()
    }

    /// Every id (lower case) with its place.
    pub fn iter(&self) -> impl Iterator<Item = (&str, &TexRect)> {
        self.map.iter().map(|(k, v)| (&**k, v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const D: &str = "<w>\n<!-- <file name=\"dead\"> -->\n<file name=\"ui\\ui_icons\">\n  <texture id=\"Icon_A\" x=\"10\" y=\"20\" width=\"32\" height=\"16.5\" />\n  <texture xid=\"q\" id=\"zero\" x=\"0\" y=\"0\" width=\"0\" height=\"4\"/>\n</file>\n<file name=\"ui\\other\"><texture id=\"b\" x=\"1\" y=\"2\" width=\"3\" height=\"4\"/></file>\n</w>";

    #[test]
    fn reads_descriptions() {
        let e = parse_descr(D);
        assert_eq!(e.len(), 2);
        assert_eq!(e[0].id, "Icon_A");
        assert_eq!(e[0].sheet, "ui\\ui_icons");
        assert_eq!(e[0].rect(), [10.0, 20.0, 32.0, 16.5]);
        assert_eq!(e[0].rect_int(), [10, 20, 32, 16]);
        assert_eq!(e[1].sheet, "ui\\other");
        assert_eq!(tag_attr("<texture xid=\"q\" id=\"z\"", "id"), "z");
        let later =
            "<file name=\"mod\"><texture id=\"ICON_A\" x=\"0\" y=\"0\" width=\"8\" height=\"8\"/></file>";
        let s = TextureSheets::build(&[D, later]);
        assert_eq!(s.len(), 2);
        let a = s.get("icon_a").unwrap();
        assert_eq!(&*a.sheet, "mod");
        assert_eq!(a.file, 1);
    }
}
