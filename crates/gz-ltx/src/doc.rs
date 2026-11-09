//! A lossless LTX document.

use std::fmt;
use std::ops::Range;

use gz_base::SmolStr;

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
    Section { name: SmolStr, parents: Vec<SmolStr> },
    /// `value` is the byte range of the trimmed value inside `raw`.
    Entry { key: SmolStr, value: Range<usize> },
    /// Blank, comment, `#include`, anything else: kept, never interpreted.
    Other,
}

#[derive(Clone, Debug)]
struct Line {
    /// The line as written, without its line break. A quoted value that spans lines keeps its
    /// inner line breaks here.
    raw: String,
    eol: Eol,
    kind: Kind,
}

impl Line {
    fn is_blank(&self) -> bool {
        matches!(self.kind, Kind::Other) && self.raw.trim().is_empty()
    }
}

/// An LTX file that writes back exactly what it read, except for the lines an edit touched.
///
/// Section names compare without ASCII case (the engine lower-cases them); keys compare
/// exactly. A key written twice in a section reads as its LAST value, as in the engine. The
/// section named `""` is what stands before the first header.
#[derive(Clone, Debug)]
pub struct LtxDoc {
    lines: Vec<Line>,
    /// The line break new lines get.
    eol: Eol,
}

impl Default for LtxDoc {
    fn default() -> Self {
        Self::new()
    }
}

/// Where the code of a line ends and its comment starts: `;` or `//` outside quotes.
fn code_end(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => in_quote = !in_quote,
            b';' if !in_quote => return i,
            b'/' if !in_quote && bytes.get(i + 1) == Some(&b'/') => return i,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

/// The byte range of `s[range]` with ASCII whitespace trimmed off both ends.
fn trim_range(s: &str, range: Range<usize>) -> Range<usize> {
    let part = &s[range.clone()];
    let start = range.start + (part.len() - part.trim_start().len());
    let end = range.start + part.trim_end().len();
    if end < start { start..start } else { start..end }
}

fn classify(raw: &str) -> Kind {
    let end = code_end(raw);
    let code = raw[..end].trim();
    if code.is_empty() || code.starts_with('#') {
        return Kind::Other;
    }
    if let Some(rest) = code.strip_prefix('[') {
        let Some(close) = rest.find(']') else { return Kind::Other };
        let name = rest[..close].trim();
        let parents = match rest[close + 1..].trim().strip_prefix(':') {
            Some(list) => gz_base::text::split_list(list).map(SmolStr::new).collect(),
            None => Vec::new(),
        };
        return Kind::Section { name: SmolStr::new(name), parents };
    }
    match raw[..end].find('=') {
        Some(eq) => Kind::Entry { key: SmolStr::new(raw[..eq].trim()), value: trim_range(raw, eq + 1..end) },
        None => {
            let key = trim_range(raw, 0..end);
            Kind::Entry { key: SmolStr::new(&raw[key.clone()]), value: key.end..key.end }
        }
    }
}

fn physical_lines(text: &str) -> Vec<(&str, Eol)> {
    let mut out = Vec::new();
    let mut rest = text;
    while !rest.is_empty() {
        match rest.find('\n') {
            Some(i) => {
                let (line, eol) = match rest[..i].strip_suffix('\r') {
                    Some(line) => (line, Eol::CrLf),
                    None => (&rest[..i], Eol::Lf),
                };
                out.push((line, eol));
                rest = &rest[i + 1..];
            }
            None => {
                out.push((rest, Eol::None));
                rest = "";
            }
        }
    }
    out
}

impl LtxDoc {
    /// An empty document with the engine's `\r\n` line breaks.
    #[must_use]
    pub fn new() -> Self {
        Self { lines: Vec::new(), eol: Eol::CrLf }
    }

    /// Reads a document. Never fails: a line that is not LTX is kept as it is.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let physical = physical_lines(text);
        let eol = physical.iter().map(|l| l.1).find(|e| *e != Eol::None).unwrap_or(Eol::CrLf);
        let mut lines: Vec<Line> = Vec::with_capacity(physical.len());
        let mut i = 0;
        while i < physical.len() {
            let (first, mut line_eol) = physical[i];
            i += 1;
            let mut kind = classify(first);
            let mut raw = first.to_owned();
            // a quoted value left open continues on the next lines, as CInifile reads it
            if let Kind::Entry { key, value } = &kind
                && raw[value.clone()].bytes().filter(|&b| b == b'"').count() % 2 == 1
            {
                let key = key.clone();
                let start = value.start;
                while i < physical.len() && raw[start..].bytes().filter(|&b| b == b'"').count() % 2 == 1 {
                    raw.push_str(line_eol.as_str());
                    raw.push_str(physical[i].0);
                    line_eol = physical[i].1;
                    i += 1;
                }
                let end = raw.rfind('"').map_or(raw.len(), |q| q + 1).max(start);
                kind = Kind::Entry { key, value: start..end };
            }
            lines.push(Line { raw, eol: line_eol, kind });
        }
        Self { lines, eol }
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// The names of the sections, in file order.
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().filter_map(|l| match &l.kind {
            Kind::Section { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }

    /// The sections whose name starts with `prefix` (ASCII case ignored), in file order.
    pub fn sections_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.sections().filter(move |n| n.len() >= prefix.len() && n.as_bytes()[..prefix.len()].eq_ignore_ascii_case(prefix.as_bytes()))
    }

    fn header(&self, section: &str) -> Option<usize> {
        self.lines.iter().position(|l| matches!(&l.kind, Kind::Section { name, .. } if name.eq_ignore_ascii_case(section)))
    }

    /// The lines of a section's body: after its header, up to the next header.
    fn body(&self, section: &str) -> Option<Range<usize>> {
        let start = if section.is_empty() { 0 } else { self.header(section)? + 1 };
        let end = self.lines[start..]
            .iter()
            .position(|l| matches!(l.kind, Kind::Section { .. }))
            .map_or(self.lines.len(), |n| start + n);
        Some(start..end)
    }

    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        self.header(section).is_some()
    }

    /// What a section inherits from: `[section]:parent, parent`.
    #[must_use]
    pub fn parents(&self, section: &str) -> &[SmolStr] {
        match self.header(section).map(|i| &self.lines[i].kind) {
            Some(Kind::Section { parents, .. }) => parents,
            _ => &[],
        }
    }

    /// The `(key, value)` lines of a section in file order, duplicates included.
    pub fn entries<'a>(&'a self, section: &str) -> impl Iterator<Item = (&'a str, &'a str)> + use<'a> {
        let range = self.body(section).unwrap_or(0..0);
        self.lines[range].iter().filter_map(|l| match &l.kind {
            Kind::Entry { key, value } => Some((key.as_str(), &l.raw[value.clone()])),
            _ => None,
        })
    }

    /// The value of a key, as written (quotes included). The last one when written twice.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries(section).filter(|(k, _)| *k == key).last().map(|(_, v)| v)
    }

    #[must_use]
    pub fn has_key(&self, section: &str, key: &str) -> bool {
        self.entries(section).any(|(k, _)| k == key)
    }

    fn new_line(&self, raw: String, kind: Kind) -> Line {
        Line { raw, eol: self.eol, kind }
    }

    /// Makes sure the last line ends in a line break, so that a line can follow it.
    fn close_last_line(&mut self) {
        let eol = self.eol;
        if let Some(last) = self.lines.last_mut()
            && last.eol == Eol::None
        {
            last.eol = eol;
        }
    }

    /// Adds an empty section at the end unless it exists.
    pub fn ensure_section(&mut self, section: &str) {
        if section.is_empty() || self.has_section(section) {
            return;
        }
        self.close_last_line();
        if self.lines.last().is_some_and(|l| !l.is_blank()) {
            self.lines.push(self.new_line(String::new(), Kind::Other));
        }
        let line = self.new_line(format!("[{section}]"), Kind::Section { name: SmolStr::new(section), parents: Vec::new() });
        self.lines.push(line);
    }

    /// The text a new `key = value` line takes, shaped like the entry lines around it.
    fn shape_entry(&self, model: Option<&Line>, key: &str, value: &str) -> (String, Range<usize>) {
        let mut raw = String::new();
        let mut separator = " = ".to_owned();
        if let Some(Line { raw: m, kind: Kind::Entry { key: mkey, value: mvalue }, .. }) = model
            && let Some(eq) = m[..code_end(m)].find('=')
        {
            let indent = &m[..m.len() - m.trim_start().len()];
            raw.push_str(indent);
            raw.push_str(key);
            let key_end = indent.len() + mkey.len();
            let before = &m[key_end.min(eq)..eq];
            // keep the '=' in its column when keys are padded to one
            let column = eq;
            let here = raw.len();
            let pad = if before.is_empty() { 0 } else { column.saturating_sub(here).max(1) };
            separator = format!("{}={}", " ".repeat(pad), &m[eq + 1..mvalue.start.max(eq + 1)]);
            if mvalue.is_empty() && separator.ends_with('=') {
                separator.push(' ');
            }
        } else {
            raw.push_str(key);
        }
        if value.is_empty() {
            raw.push_str(separator.trim_end());
            let at = raw.len();
            return (raw, at..at);
        }
        raw.push_str(&separator);
        let start = raw.len();
        raw.push_str(value);
        let end = raw.len();
        (raw, start..end)
    }

    /// Sets a key. An existing line keeps its key, its spacing and its comment; a new line goes
    /// after the section's last entry; a missing section is added at the end.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        self.ensure_section(section);
        let Some(body) = self.body(section) else { return };
        let existing = body.clone().rev().find(|&i| matches!(&self.lines[i].kind, Kind::Entry { key: k, .. } if k == key));
        if let Some(i) = existing {
            let line = &mut self.lines[i];
            if let Kind::Entry { value: range, .. } = &mut line.kind {
                let has_eq = line.raw[..range.start].contains('=');
                let insert = if has_eq { value.to_owned() } else { format!(" = {value}") };
                line.raw.replace_range(range.clone(), &insert);
                let start = range.start + (insert.len() - value.len());
                *range = start..start + value.len();
            }
            return;
        }
        let last_entry = body.clone().rev().find(|&i| matches!(self.lines[i].kind, Kind::Entry { .. }));
        let model = last_entry
            .map(|i| &self.lines[i])
            .or_else(|| self.lines.iter().find(|l| matches!(l.kind, Kind::Entry { .. })));
        let (raw, range) = self.shape_entry(model, key, value);
        let line = self.new_line(raw, Kind::Entry { key: SmolStr::new(key), value: range });
        let at = last_entry.map_or(body.start, |i| i + 1);
        if at == self.lines.len() {
            self.close_last_line();
        }
        self.lines.insert(at, line);
    }

    /// Removes every line of a key. Returns whether there was one.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        let Some(body) = self.body(section) else { return false };
        let before = self.lines.len();
        let mut i = body.start;
        let mut end = body.end;
        while i < end {
            if matches!(&self.lines[i].kind, Kind::Entry { key: k, .. } if k == key) {
                self.lines.remove(i);
                end -= 1;
            } else {
                i += 1;
            }
        }
        self.lines.len() != before
    }

    /// Replaces the entries of a section by `entries`, in that order. Comments inside the
    /// section stay where they are; the section is added when missing.
    pub fn replace_entries<K: AsRef<str>, V: AsRef<str>>(&mut self, section: &str, entries: impl IntoIterator<Item = (K, V)>) {
        self.ensure_section(section);
        let Some(body) = self.body(section) else { return };
        let model = body.clone().find(|&i| matches!(self.lines[i].kind, Kind::Entry { .. })).map(|i| self.lines[i].clone());
        let first = body.clone().find(|&i| matches!(self.lines[i].kind, Kind::Entry { .. })).unwrap_or(body.start);
        let mut kept: Vec<Line> = Vec::with_capacity(body.len());
        let mut insert_at = None;
        for (n, i) in body.clone().enumerate() {
            if i == first {
                insert_at = Some(kept.len());
            }
            if !matches!(self.lines[i].kind, Kind::Entry { .. }) {
                kept.push(self.lines[i].clone());
            }
            let _ = n;
        }
        let insert_at = insert_at.unwrap_or(0);
        let fresh: Vec<Line> = entries
            .into_iter()
            .map(|(k, v)| {
                let (raw, range) = self.shape_entry(model.as_ref(), k.as_ref(), v.as_ref());
                self.new_line(raw, Kind::Entry { key: SmolStr::new(k.as_ref()), value: range })
            })
            .collect();
        kept.splice(insert_at..insert_at, fresh);
        if body.end == self.lines.len() {
            self.close_last_line();
            if let Some(last) = kept.last_mut()
                && last.eol == Eol::None
            {
                last.eol = self.eol;
            }
        }
        self.lines.splice(body, kept);
    }

    /// Removes a section with its body. A comment block that leans on the next header stays
    /// with that header. Returns whether the section existed.
    pub fn remove_section(&mut self, section: &str) -> bool {
        let Some(header) = self.header(section) else { return false };
        let Some(body) = self.body(section) else { return false };
        let mut end = body.end;
        if end < self.lines.len() {
            while end > body.start && matches!(self.lines[end - 1].kind, Kind::Other) && !self.lines[end - 1].is_blank() {
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
        true
    }
}

impl fmt::Display for LtxDoc {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for line in &self.lines {
            f.write_str(&line.raw)?;
            f.write_str(line.eol.as_str())?;
        }
        Ok(())
    }
}

/// The text of a value without the quotes around it.
#[must_use]
pub fn unquote(value: &str) -> &str {
    let v = value.trim();
    v.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(v)
}

/// A value in quotes when it would not survive bare: empty, with outer spaces, or with a
/// comment mark in it.
#[must_use]
pub fn quote(value: &str) -> String {
    let bare = !value.is_empty() && value.trim() == value && !value.contains(';') && !value.contains("//") && !value.contains('"');
    if bare { value.to_owned() } else { format!("\"{}\"", value.replace('"', "'")) }
}

/// The engine's `r_bool`: `on`, `yes`, `true`, `1`.
#[must_use]
pub fn parse_bool(value: &str) -> bool {
    let v = unquote(value);
    v.eq_ignore_ascii_case("on") || v.eq_ignore_ascii_case("yes") || v.eq_ignore_ascii_case("true") || v == "1"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "; the manifest\r\n[module]\r\nname        = \"Overnight; Delivery\" ; title\r\nversion     = 1.2.0\r\n\r\n; rules\r\n[world] : base\r\nmutants = false\r\n";

    #[test]
    fn untouched_is_identical() {
        for text in [SAMPLE, "", "a = 1", "[s]\nk=v\n", "\r\n\r\n", "[s]\r\nk = \"line one\r\nline two\" ; c\r\nz = 1\r\n"] {
            assert_eq!(LtxDoc::parse(text).to_string(), text);
        }
    }

    #[test]
    fn reads_like_the_engine() {
        let doc = LtxDoc::parse(SAMPLE);
        assert_eq!(doc.sections().collect::<Vec<_>>(), ["module", "world"]);
        assert_eq!(doc.get("MODULE", "name"), Some("\"Overnight; Delivery\""));
        assert_eq!(unquote(doc.get("module", "name").unwrap()), "Overnight; Delivery");
        assert_eq!(doc.get("module", "version"), Some("1.2.0"));
        assert_eq!(doc.get("module", "Version"), None);
        assert_eq!(doc.parents("world"), [SmolStr::new("base")]);
        assert!(!parse_bool(doc.get("world", "mutants").unwrap()));
        let twice = LtxDoc::parse("[s]\nk = 1\nk = 2\n");
        assert_eq!(twice.get("s", "k"), Some("2"));
    }

    #[test]
    fn an_edit_touches_its_line_only() {
        let mut doc = LtxDoc::parse(SAMPLE);
        doc.set("module", "version", "1.3.0");
        assert_eq!(doc.to_string(), SAMPLE.replace("1.2.0", "1.3.0"));
        doc.set("module", "name", "\"New\"");
        assert!(doc.to_string().contains("name        = \"New\" ; title\r\n"));
    }

    #[test]
    fn a_new_key_follows_the_column() {
        let mut doc = LtxDoc::parse(SAMPLE);
        doc.set("module", "author", "Madmer");
        assert!(doc.to_string().contains("version     = 1.2.0\r\nauthor      = Madmer\r\n\r\n; rules"));
        doc.set("world", "npc_quests", "true");
        assert!(doc.to_string().ends_with("mutants = false\r\nnpc_quests = true\r\n"));
    }

    #[test]
    fn a_new_section_goes_to_the_end() {
        let mut doc = LtxDoc::parse("[a]\nk = 1");
        doc.set("b", "x", "2");
        assert_eq!(doc.to_string(), "[a]\nk = 1\n\n[b]\nx = 2\n");
        let mut empty = LtxDoc::new();
        empty.set("module", "id", "demo");
        assert_eq!(empty.to_string(), "[module]\r\nid = demo\r\n");
    }

    #[test]
    fn removing_keeps_the_neighbours() {
        let mut doc = LtxDoc::parse(SAMPLE);
        assert!(doc.remove("module", "version"));
        assert!(!doc.remove("module", "version"));
        assert!(doc.remove_section("module"));
        assert_eq!(doc.to_string(), "; the manifest\r\n; rules\r\n[world] : base\r\nmutants = false\r\n");
        assert!(doc.remove_section("world"));
        assert_eq!(doc.to_string(), "; the manifest\r\n; rules\r\n");
    }

    #[test]
    fn replacing_entries_keeps_comments() {
        let mut doc = LtxDoc::parse("[vars]\n; counters\nkills = 0\nmet   = false\n\n[next]\n");
        doc.replace_entries("vars", [("stage", "1"), ("met", "true")]);
        assert_eq!(doc.to_string(), "[vars]\n; counters\nstage = 1\nmet   = true\n\n[next]\n");
        doc.replace_entries::<&str, &str>("vars", []);
        assert_eq!(doc.to_string(), "[vars]\n; counters\n\n[next]\n");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("a; b"), "\"a; b\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(unquote("\"x\""), "x");
    }
}
