//! `mod.ltx` as written: a lossless, line based document.
//!
//! It reads a line the way the C++ `EditorMod::Load` (and the game's XMS reader) does: the line
//! is trimmed, a `;` outside a quoted value starts a comment, a `[` starts a section header whose
//! name ends at the first `]`, and a key ends at the first `=`. `gz_ltx::LtxDoc` reads `//` as a
//! comment and knows no `\"` escape, which would cut `website = https://...` short - so the
//! manifest has a document of its own.

use std::fmt;
use std::ops::Range;

use gz_base::SmolStr;

use crate::text::{WS, find_comment};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Eol {
    CrLf,
    Lf,
    /// The last line of a file that does not end in a line break.
    None,
}

impl Eol {
    fn as_str(self) -> &'static str {
        match self {
            Eol::CrLf => "\r\n",
            Eol::Lf => "\n",
            Eol::None => "",
        }
    }
}

#[derive(Clone, Debug)]
enum Kind {
    /// `[name]`, the name trimmed as written. Names compare without ASCII case.
    Section(SmolStr),
    /// Byte ranges inside the raw line: the key, the value (both trimmed), the code (the line
    /// without its comment, trimmed) and whether there was an `=`.
    Entry { key: Range<usize>, value: Range<usize>, code: Range<usize>, eq: bool },
    /// Blank or comment only.
    Other,
}

#[derive(Clone, Debug)]
struct Line {
    raw: String,
    eol: Eol,
    kind: Kind,
}

impl Line {
    fn is_blank(&self) -> bool {
        matches!(self.kind, Kind::Other) && self.raw.trim_matches(WS).is_empty()
    }

    fn is_entry(&self) -> bool {
        matches!(self.kind, Kind::Entry { .. })
    }

    fn key(&self) -> Option<&str> {
        match &self.kind {
            Kind::Entry { key, .. } => Some(sub(&self.raw, key)),
            _ => None,
        }
    }
}

/// One line of the document as a reader sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item<'a> {
    Section(&'a str),
    /// `code` is the line without its comment: what the C++ kept of a `[requires]` line.
    Entry { key: &'a str, value: &'a str, code: &'a str },
    Other,
}

fn sub<'a>(s: &'a str, r: &Range<usize>) -> &'a str {
    s.get(r.clone()).unwrap_or("")
}

/// `range` of `s` with the C++ whitespace trimmed off both ends.
fn trim_range(s: &str, range: Range<usize>) -> Range<usize> {
    let part = sub(s, &range);
    let start = range.start + (part.len() - part.trim_start_matches(WS).len());
    let end = range.start + part.trim_end_matches(WS).len();
    if end < start { start..start } else { start..end }
}

fn classify(raw: &str) -> Kind {
    let cut = find_comment(raw).unwrap_or(raw.len());
    let code = trim_range(raw, 0..cut);
    let text = sub(raw, &code);
    if text.is_empty() {
        return Kind::Other;
    }
    if let Some(rest) = text.strip_prefix('[') {
        let name = rest.find(']').and_then(|c| rest.get(..c)).unwrap_or("");
        return Kind::Section(SmolStr::new(name.trim_matches(WS)));
    }
    match text.find('=') {
        Some(at) => {
            let eq = code.start + at;
            Kind::Entry { key: trim_range(raw, code.start..eq), value: trim_range(raw, eq + 1..code.end), code, eq: true }
        }
        None => Kind::Entry { key: code.clone(), value: code.end..code.end, code, eq: false },
    }
}

/// A line of text cannot hold a line break.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// The pad of a key in the editor's own lines: `AppendKey`'s 11 ("description"), 15 for the
/// longer keys of `[world]` and its kind, 17 for the screens.
#[must_use]
pub fn key_width(section: &str) -> usize {
    let s = section.to_ascii_lowercase();
    if matches!(s.as_str(), "world" | "start_kit" | "vars" | "commands") {
        15
    } else if s == "screens" || s.starts_with("screen.") || s.starts_with("track.") {
        17
    } else {
        11
    }
}

/// `key = value` with the key padded to `width`, as `AppendKey` writes it.
#[must_use]
pub fn entry_line(key: &str, value: &str, width: usize) -> String {
    let line = format!("{key:<width$} = {value}");
    line.trim_end_matches(WS).to_owned()
}

/// An LTX file that writes back exactly what it read, except for the lines an edit touched.
#[derive(Clone, Debug)]
pub struct ManifestDoc {
    lines: Vec<Line>,
    /// The line break new lines get.
    eol: Eol,
}

impl Default for ManifestDoc {
    fn default() -> Self {
        Self { lines: Vec::new(), eol: Eol::CrLf }
    }
}

fn physical_lines(text: &str) -> Vec<(&str, Eol)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(i) => {
                let head = rest.get(..i).unwrap_or("");
                match head.strip_suffix('\r') {
                    Some(line) => out.push((line, Eol::CrLf)),
                    None => out.push((head, Eol::Lf)),
                }
                rest = rest.get(i + 1..).unwrap_or("");
            }
            None => {
                out.push((rest, Eol::None));
                rest = "";
            }
        }
    }
    out
}

impl ManifestDoc {
    /// Reads a document. Never fails: a line that is not LTX is kept as it is. A BOM is dropped.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let physical = physical_lines(text);
        let eol = physical.iter().map(|l| l.1).find(|e| *e != Eol::None).unwrap_or(Eol::CrLf);
        let lines = physical.into_iter().map(|(raw, eol)| Line { kind: classify(raw), raw: raw.to_owned(), eol }).collect();
        Self { lines, eol }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Every line, in file order.
    pub fn items(&self) -> impl Iterator<Item = Item<'_>> {
        self.lines.iter().map(|l| match &l.kind {
            Kind::Section(name) => Item::Section(name.as_str()),
            Kind::Entry { key, value, code, .. } => Item::Entry { key: sub(&l.raw, key), value: sub(&l.raw, value), code: sub(&l.raw, code) },
            Kind::Other => Item::Other,
        })
    }

    /// The section headers in file order (a section written twice is named twice).
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().filter_map(|l| match &l.kind {
            Kind::Section(n) => Some(n.as_str()),
            _ => None,
        })
    }

    /// The sections whose name starts with `prefix` (ASCII case ignored), each once, in file
    /// order: `screen.` gives every `[screen.<id>]`.
    #[must_use]
    pub fn sections_with_prefix(&self, prefix: &str) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for name in self.sections() {
            let hit = name.len() >= prefix.len() && name.get(..prefix.len()).is_some_and(|h| h.eq_ignore_ascii_case(prefix));
            if hit && !out.iter().any(|o| o.eq_ignore_ascii_case(name)) {
                out.push(name);
            }
        }
        out
    }

    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        self.lines.iter().any(|l| matches!(&l.kind, Kind::Section(n) if n.eq_ignore_ascii_case(section)))
    }

    /// The bodies of a section: the lines after each of its headers, up to the next header.
    fn bodies(&self, section: &str) -> Vec<Range<usize>> {
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(line) = self.lines.get(i) {
            if matches!(&line.kind, Kind::Section(n) if n.eq_ignore_ascii_case(section)) {
                let start = i + 1;
                let end = self.lines.get(start..).and_then(|rest| rest.iter().position(|l| matches!(l.kind, Kind::Section(_)))).map_or(self.lines.len(), |n| start + n);
                out.push(start..end);
                i = end;
            } else {
                i += 1;
            }
        }
        out
    }

    fn body_lines(&self, section: &str) -> Vec<usize> {
        self.bodies(section).into_iter().flatten().collect()
    }

    /// The `(key, value)` lines of a section in file order, duplicates included; values as
    /// written (quotes included).
    #[must_use]
    pub fn section_entries(&self, section: &str) -> Vec<(&str, &str)> {
        self.body_lines(section)
            .into_iter()
            .filter_map(|i| self.lines.get(i))
            .filter_map(|l| match &l.kind {
                Kind::Entry { key, value, .. } => Some((sub(&l.raw, key), sub(&l.raw, value))),
                _ => None,
            })
            .collect()
    }

    /// The entry lines of a section without their comments: what the C++ kept of a raw-line
    /// section (`[requires]`, `[exclude]`).
    #[must_use]
    pub fn section_lines(&self, section: &str) -> Vec<&str> {
        self.body_lines(section)
            .into_iter()
            .filter_map(|i| self.lines.get(i))
            .filter_map(|l| match &l.kind {
                Kind::Entry { code, .. } => Some(sub(&l.raw, code)),
                _ => None,
            })
            .collect()
    }

    /// The value of a key as written, the last one when written twice.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.section_entries(section).into_iter().filter(|(k, _)| *k == key).last().map(|(_, v)| v)
    }

    fn line(&self, raw: String) -> Line {
        let raw = one_line(&raw);
        Line { kind: classify(&raw), raw, eol: self.eol }
    }

    /// Every line but the last ends in a line break.
    fn close_lines(&mut self) {
        let eol = self.eol;
        let n = self.lines.len();
        for line in self.lines.iter_mut().take(n.saturating_sub(1)) {
            if line.eol == Eol::None {
                line.eol = eol;
            }
        }
    }

    /// Adds an empty section at the end, after a blank line.
    fn append_section(&mut self, section: &str) {
        if self.lines.last().is_some_and(|l| !l.is_blank()) {
            let blank = self.line(String::new());
            self.lines.push(blank);
        }
        let header = self.line(format!("[{section}]"));
        self.lines.push(header);
        self.close_lines();
    }

    /// The text of a new entry, shaped like `model` (its indent and the column of its `=`), or
    /// `AppendKey`'s when there is none.
    fn shape(model: Option<&Line>, key: &str, value: &str, width: usize) -> String {
        if let Some(Line { raw, kind: Kind::Entry { key: mk, value: mv, eq: true, .. }, .. }) = model
            && let Some(at) = raw.get(mk.end..mv.start.max(mk.end)).and_then(|s| s.find('='))
        {
            let eq = mk.end + at;
            let indent = raw.get(..mk.start).unwrap_or("");
            let before = raw.get(mk.end..eq).unwrap_or("");
            let after = raw.get(eq + 1..mv.start.max(eq + 1)).unwrap_or("");
            let mut out = format!("{indent}{key}");
            if !before.is_empty() {
                let pad = eq.saturating_sub(out.len()).max(1);
                out.push_str(&" ".repeat(pad));
            }
            out.push('=');
            out.push_str(after);
            out.push_str(value);
            return out.trim_end_matches(WS).to_owned();
        }
        entry_line(key, value, width)
    }

    fn model_entry(&self, section: &str) -> Option<&Line> {
        // a line with a value shows how this file spaces one; `key =` alone does not
        let model = |l: &&Line| matches!(&l.kind, Kind::Entry { value, eq: true, .. } if !value.is_empty());
        let own = self.body_lines(section).into_iter().rev().filter_map(|i| self.lines.get(i)).find(model);
        own.or_else(|| self.lines.iter().find(model))
    }

    /// Where a new entry of a section goes: after its last entry, or right after its last header.
    fn insert_point(&mut self, section: &str) -> usize {
        if !self.has_section(section) {
            self.append_section(section);
        }
        let bodies = self.bodies(section);
        let last_entry = bodies.iter().flat_map(Clone::clone).filter(|&i| self.lines.get(i).is_some_and(Line::is_entry)).last();
        match last_entry {
            Some(i) => i + 1,
            None => bodies.last().map_or(self.lines.len(), |b| b.start),
        }
    }

    fn insert(&mut self, at: usize, line: Line) {
        let at = at.min(self.lines.len());
        self.lines.insert(at, line);
        self.close_lines();
    }

    /// Sets a key. An existing line (the last of that key) keeps its key, its spacing and its
    /// comment; a new line goes after the section's last entry, shaped like its neighbours; a
    /// missing section is added at the end.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let value = one_line(value);
        let existing = self.body_lines(section).into_iter().rev().find(|&i| self.lines.get(i).and_then(Line::key) == Some(key));
        if let Some(i) = existing
            && let Some(line) = self.lines.get_mut(i)
        {
            if let Kind::Entry { value: range, code, eq, .. } = line.kind.clone() {
                if eq {
                    line.raw.replace_range(range, &value);
                } else if !value.is_empty() {
                    line.raw.insert_str(code.end, &format!(" = {value}"));
                }
                line.kind = classify(&line.raw);
            }
            return;
        }
        let raw = Self::shape(self.model_entry(section), key, &value, key_width(section));
        let at = self.insert_point(section);
        let line = self.line(raw);
        self.insert(at, line);
    }

    /// Replaces the first entry whose key matches `key` without ASCII case by `raw`. `false`
    /// when there is none.
    pub fn set_line_ci(&mut self, section: &str, key: &str, raw: &str) -> bool {
        let found = self.body_lines(section).into_iter().find(|&i| self.lines.get(i).and_then(Line::key).is_some_and(|k| k.eq_ignore_ascii_case(key)));
        let Some(i) = found else { return false };
        let line = self.line(raw.to_owned());
        if let Some(slot) = self.lines.get_mut(i) {
            slot.raw = line.raw;
            slot.kind = line.kind;
        }
        true
    }

    /// Adds `raw` after the section's last entry.
    pub fn push_line(&mut self, section: &str, raw: &str) {
        let at = self.insert_point(section);
        let line = self.line(raw.to_owned());
        self.insert(at, line);
    }

    /// Removes every entry of `section` whose key `pred` accepts. Returns whether one went.
    pub fn remove_where(&mut self, section: &str, pred: impl Fn(&str) -> bool) -> bool {
        let doomed: Vec<usize> = self.body_lines(section).into_iter().filter(|&i| self.lines.get(i).and_then(Line::key).is_some_and(&pred)).collect();
        for &i in doomed.iter().rev() {
            if i < self.lines.len() {
                self.lines.remove(i);
            }
        }
        if !doomed.is_empty() {
            self.close_lines();
        }
        !doomed.is_empty()
    }

    /// Removes every line of a key. Returns whether there was one.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        self.remove_where(section, |k| k == key)
    }

    /// Replaces the entries of a section by `lines` (raw entry text, in that order), where its
    /// first entry was. Comments in the section stay where they are; a section written twice
    /// keeps its second header with no entries. A missing section is added unless `lines` is
    /// empty.
    pub fn set_lines(&mut self, section: &str, lines: &[String]) {
        if !self.has_section(section) {
            if lines.is_empty() {
                return;
            }
            self.append_section(section);
        }
        let bodies = self.bodies(section);
        let in_body: Vec<usize> = bodies.iter().flat_map(Clone::clone).collect();
        let first_entry = in_body.iter().copied().find(|&i| self.lines.get(i).is_some_and(Line::is_entry));
        let insert_at = first_entry.or_else(|| bodies.first().map(|b| b.start)).unwrap_or(self.lines.len());
        let mut fresh: Vec<Line> = lines.iter().map(|l| self.line(l.clone())).collect();
        let old = std::mem::take(&mut self.lines);
        let total = old.len();
        let mut out: Vec<Line> = Vec::with_capacity(total + fresh.len());
        for (i, line) in old.into_iter().enumerate() {
            if i == insert_at {
                out.append(&mut fresh);
            }
            if line.is_entry() && in_body.contains(&i) {
                continue;
            }
            out.push(line);
        }
        out.append(&mut fresh);
        self.lines = out;
        self.close_lines();
    }

    /// Removes a section (every header of that name) with its body. A comment block that leans
    /// on the next header stays with that header. Returns whether the section existed.
    pub fn remove_section(&mut self, section: &str) -> bool {
        let mut any = false;
        while let Some(body) = self.bodies(section).into_iter().next() {
            any = true;
            let header = body.start.saturating_sub(1);
            let mut end = body.end;
            if end < self.lines.len() {
                while end > body.start && self.lines.get(end - 1).is_some_and(|l| matches!(l.kind, Kind::Other) && !l.is_blank()) {
                    end -= 1;
                }
            }
            self.lines.drain(header..end);
            if header >= self.lines.len() {
                // the section closed the file: blank lines that separated it go with it
                while self.lines.last().is_some_and(Line::is_blank) {
                    self.lines.pop();
                }
            }
        }
        any
    }

    /// Removes a section that holds nothing but blank lines: what the editor leaves when its
    /// last key goes. A section with a comment of the author's stays.
    pub fn prune(&mut self, section: &str) -> bool {
        let bodies = self.bodies(section);
        if bodies.is_empty() {
            return false;
        }
        let empty = bodies.iter().flat_map(Clone::clone).all(|i| self.lines.get(i).is_some_and(Line::is_blank));
        empty && self.remove_section(section)
    }
}

impl fmt::Display for ManifestDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            f.write_str(&line.raw)?;
            f.write_str(line.eol.as_str())?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    const SAMPLE: &str = "; my module\r\n[module]\r\nid          = demo\r\nname        = \"Over; Night\" ; the title\r\nversion     = 1.2.0\r\nwebsite     = https://www.moddb.com/mods/demo\r\n\r\n; what is left out\r\n[exclude]\r\ndocs\r\n*.psd\r\n";

    #[test]
    fn untouched_is_identical() {
        for text in [SAMPLE, "", "a = 1", "[s]\nk=v\n", "\r\n\r\n", "[s]\r\nk = \"x\" ; c\r\nz"] {
            assert_eq!(ManifestDoc::parse(text).to_string(), text);
        }
        assert_eq!(ManifestDoc::parse("\u{feff}[s]\n").to_string(), "[s]\n");
    }

    #[test]
    fn reads_like_the_cpp() {
        let doc = ManifestDoc::parse(SAMPLE);
        assert_eq!(doc.get("MODULE", "website"), Some("https://www.moddb.com/mods/demo"));
        assert_eq!(doc.get("module", "name"), Some("\"Over; Night\""));
        assert_eq!(doc.section_lines("exclude"), ["docs", "*.psd"]);
        assert_eq!(doc.get("module", "ID"), None);
        let twice = ManifestDoc::parse("[s]\nk = 1\n[t]\n[S]\nk = 2\n");
        assert_eq!(twice.get("s", "k"), Some("2"));
        assert_eq!(twice.section_entries("s").len(), 2);
    }

    #[test]
    fn an_edit_touches_its_line_only() {
        let mut doc = ManifestDoc::parse(SAMPLE);
        doc.set("module", "version", "1.3.0");
        assert_eq!(doc.to_string(), SAMPLE.replace("1.2.0", "1.3.0"));
        doc.set("module", "name", "\"New\"");
        assert!(doc.to_string().contains("name        = \"New\" ; the title\r\n"));
    }

    #[test]
    fn a_new_key_follows_the_column() {
        let mut doc = ManifestDoc::parse(SAMPLE);
        doc.set("module", "author", "Madmer");
        assert!(doc.to_string().contains("website     = https://www.moddb.com/mods/demo\r\nauthor      = Madmer\r\n\r\n; what"));
        doc.set("update", "github", "Madmer/demo");
        assert!(doc.to_string().ends_with("*.psd\r\n\r\n[update]\r\ngithub      = Madmer/demo\r\n"));
        let mut fresh = ManifestDoc::parse("[module]\nid=x");
        fresh.set("module", "api", "1");
        assert_eq!(fresh.to_string(), "[module]\nid=x\napi=1\n");
    }

    #[test]
    fn lines_replace_and_keep_comments() {
        let mut doc = ManifestDoc::parse("[vars]\n; counters\nkills = 0\nmet   = false\n\n[next]\n");
        doc.set_lines("vars", &["stage = 1".to_owned(), "met = true".to_owned()]);
        assert_eq!(doc.to_string(), "[vars]\n; counters\nstage = 1\nmet = true\n\n[next]\n");
        doc.set_lines("vars", &[]);
        assert_eq!(doc.to_string(), "[vars]\n; counters\n\n[next]\n");
        assert!(!doc.prune("vars"));
        let mut empty = ManifestDoc::parse("[a]\nk = 1\n\n[exclude]\ndocs\n");
        empty.set_lines("exclude", &[]);
        assert!(empty.prune("exclude"));
        assert_eq!(empty.to_string(), "[a]\nk = 1\n");
    }

    #[test]
    fn removing_and_case_blind_lines() {
        let mut doc = ManifestDoc::parse("[commands]\nStatus = a.b\nother = c.d\n\n[command.status]\nhelp = x\n");
        assert!(doc.set_line_ci("commands", "status", "status          = a.c"));
        assert!(doc.remove_section("COMMAND.STATUS"));
        assert_eq!(doc.to_string(), "[commands]\nstatus          = a.c\nother = c.d\n");
        assert!(doc.remove_where("commands", |k| k.eq_ignore_ascii_case("OTHER")));
        doc.push_line("commands", "next            = e.f");
        assert_eq!(doc.to_string(), "[commands]\nstatus          = a.c\nnext            = e.f\n");
        assert_eq!(ManifestDoc::parse("[screen.a]\n[track.x]\n[Screen.B]\n[screen.a]\n").sections_with_prefix("screen."), ["screen.a", "Screen.B"]);
    }
}
