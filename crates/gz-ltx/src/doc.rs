//! A lossless LTX document: the one line model of the LTX text the editor edits.
//!
//! Two readers of LTX lines exist and the document speaks both ([`Syntax`]): the engine's
//! `CInifile` (project files, the project list, the editor's settings) and the module manifest
//! reader of the game's XMS and the C++ `EditorMod::Load` (`mod.ltx`). They differ in what starts
//! a comment, what a header carries and how a section written twice reads; the lines, their
//! breaks and every edit are the same code.

use std::fmt;
use std::ops::Range;

use gz_base::SmolStr;

/// What the manifest reader (the C++ `Trim`) takes off both ends of a line.
pub const MANIFEST_WS: &[char] = &[' ', '\t', '\r', '\n'];

/// The rules a line is read by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Syntax {
    /// The engine's `CInifile`: `;` and `//` start a comment outside quotes, a `#` line is a
    /// directive, a header names its parents (`[s] : base`), a quoted value left open continues
    /// on the next lines, and a section written twice is read at its first header. The section
    /// named `""` is what stands before the first header.
    #[default]
    Ini,
    /// The module manifest reader: only `;` starts a comment, and not inside a quoted value
    /// (`\"` escapes there) - `website = https://...` keeps its `//`; a header is a name and no
    /// more, a `#` line is an entry, a value never spans lines, and every header of a name opens
    /// another body of that section.
    Manifest,
}

/// Where an edit puts what is new: the order a writer keeps sections and keys in, and the width
/// a key is padded to when no line of the file shows how. The default puts a section at the end
/// and a key after its section's last entry.
#[derive(Clone, Copy, Debug)]
pub struct Placement {
    /// A section goes before the first section of a higher rank; `usize::MAX` closes the file.
    pub section_rank: fn(&str) -> usize,
    /// A key goes before the first key of its section with a higher rank; `usize::MAX` after
    /// the last entry.
    pub key_rank: fn(&str, &str) -> usize,
    /// The pad of a new key of a section when no entry of the file shows the spacing.
    pub key_width: fn(&str) -> usize,
}

fn last_rank(_: &str) -> usize {
    usize::MAX
}

fn last_key_rank(_: &str, _: &str) -> usize {
    usize::MAX
}

fn no_width(_: &str) -> usize {
    0
}

impl Default for Placement {
    fn default() -> Self {
        Self { section_rank: last_rank, key_rank: last_key_rank, key_width: no_width }
    }
}

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
    /// The name trimmed as written; names compare without ASCII case.
    Section { name: SmolStr, parents: Vec<SmolStr> },
    /// Byte ranges inside the raw line: the key, the value (both trimmed), the code (the line
    /// without its comment, trimmed) and whether there was an `=`.
    Entry { key: Range<usize>, value: Range<usize>, code: Range<usize>, eq: bool },
    /// Blank, comment, an `#include` of `CInifile`, a header it cannot read: kept, never
    /// interpreted.
    Other,
}

#[derive(Clone, Debug)]
struct Line {
    /// The line as written, without its line break. A quoted value of `CInifile` that spans
    /// lines keeps its inner line breaks here.
    raw: String,
    eol: Eol,
    kind: Kind,
}

impl Line {
    fn is_blank(&self, syntax: Syntax) -> bool {
        matches!(self.kind, Kind::Other) && syntax.trim(&self.raw).is_empty()
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
pub enum LineItem<'a> {
    Section(&'a str),
    /// `code` is the line without its comment: what the C++ kept of a raw line (`[requires]`).
    Entry {
        key: &'a str,
        value: &'a str,
        code: &'a str,
    },
    Other,
}

fn sub<'a>(s: &'a str, r: &Range<usize>) -> &'a str {
    s.get(r.clone()).unwrap_or("")
}

/// Where the `;` that starts a manifest comment is, `None` when the line has none. A `;`
/// between a pair of double quotes belongs to the value (`\"` escapes inside them); a quote that
/// never closes is an ordinary character. `//` is not a comment here.
#[must_use]
pub fn find_manifest_comment(s: &str) -> Option<usize> {
    let b = s.as_bytes();
    let mut i = 0;
    while let Some(&c) = b.get(i) {
        if c == b';' {
            return Some(i);
        }
        if c == b'"' {
            let mut close = i + 1;
            while let Some(&q) = b.get(close) {
                if q == b'"' {
                    break;
                }
                if q == b'\\' && close + 1 < b.len() {
                    close += 1;
                }
                close += 1;
            }
            if close < b.len() {
                i = close;
            }
        }
        i += 1;
    }
    None
}

/// Where the code of a `CInifile` line ends and its comment starts: `;` or `//` outside quotes.
fn ini_code_end(line: &str) -> usize {
    let bytes = line.as_bytes();
    let mut in_quote = false;
    let mut i = 0;
    while let Some(&c) = bytes.get(i) {
        match c {
            b'"' => in_quote = !in_quote,
            b';' if !in_quote => return i,
            b'/' if !in_quote && bytes.get(i + 1) == Some(&b'/') => return i,
            _ => {}
        }
        i += 1;
    }
    bytes.len()
}

impl Syntax {
    fn is_ws(self, c: char) -> bool {
        match self {
            Syntax::Ini => c.is_whitespace(),
            Syntax::Manifest => MANIFEST_WS.contains(&c),
        }
    }

    fn trim(self, s: &str) -> &str {
        s.trim_matches(|c| self.is_ws(c))
    }

    fn code_end(self, line: &str) -> usize {
        match self {
            Syntax::Ini => ini_code_end(line),
            Syntax::Manifest => find_manifest_comment(line).unwrap_or(line.len()),
        }
    }

    /// The byte range of `s[range]` with this syntax's whitespace trimmed off both ends.
    fn trim_range(self, s: &str, range: Range<usize>) -> Range<usize> {
        let part = sub(s, &range);
        let start = range.start + (part.len() - part.trim_start_matches(|c| self.is_ws(c)).len());
        let end = range.start + part.trim_end_matches(|c| self.is_ws(c)).len();
        if end < start { start..start } else { start..end }
    }

    fn classify(self, raw: &str) -> Kind {
        let end = self.code_end(raw);
        let code = self.trim_range(raw, 0..end);
        let text = sub(raw, &code);
        if text.is_empty() || (self == Syntax::Ini && text.starts_with('#')) {
            return Kind::Other;
        }
        if let Some(rest) = text.strip_prefix('[') {
            return match self {
                Syntax::Ini => {
                    let Some(close) = rest.find(']') else { return Kind::Other };
                    let name = rest.get(..close).unwrap_or("").trim();
                    let parents = match rest.get(close + 1..).unwrap_or("").trim().strip_prefix(':') {
                        Some(list) => gz_base::text::split_list(list).map(SmolStr::new).collect(),
                        None => Vec::new(),
                    };
                    Kind::Section { name: SmolStr::new(name), parents }
                }
                Syntax::Manifest => {
                    let name = rest.find(']').and_then(|c| rest.get(..c)).unwrap_or("");
                    Kind::Section { name: SmolStr::new(self.trim(name)), parents: Vec::new() }
                }
            };
        }
        // `CInifile` looks for the `=` from the line's start, the manifest reader in its code
        let from = if self == Syntax::Ini { 0 } else { code.start };
        match raw.get(from..end).and_then(|s| s.find('=')) {
            Some(at) => {
                let eq = from + at;
                Kind::Entry {
                    key: self.trim_range(raw, from..eq),
                    value: self.trim_range(raw, eq + 1..end),
                    code,
                    eq: true,
                }
            }
            None => Kind::Entry { key: code.clone(), value: code.end..code.end, code, eq: false },
        }
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

/// A line of text cannot hold a line break.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// `key = value` with the key padded to `width`, as the C++ `AppendKey` writes it.
#[must_use]
pub fn padded_entry(key: &str, value: &str, width: usize) -> String {
    let line = format!("{key:<width$} = {value}");
    line.trim_end_matches(MANIFEST_WS).to_owned()
}

/// An LTX file that writes back exactly what it read, except for the lines an edit touched.
///
/// Section names compare without ASCII case (the engine lower-cases them); keys compare
/// exactly. A key written twice in a section reads as its LAST value, as in the engine.
#[derive(Clone, Debug)]
pub struct LtxDoc {
    lines: Vec<Line>,
    /// The line break new lines get.
    eol: Eol,
    syntax: Syntax,
    placement: Placement,
}

impl Default for LtxDoc {
    fn default() -> Self {
        Self::new()
    }
}

impl LtxDoc {
    /// An empty `CInifile` document with the engine's `\r\n` line breaks.
    #[must_use]
    pub fn new() -> Self {
        Self::new_as(Syntax::Ini, Placement::default())
    }

    /// An empty document read and edited by `syntax`, new lines placed by `placement`.
    #[must_use]
    pub fn new_as(syntax: Syntax, placement: Placement) -> Self {
        Self { lines: Vec::new(), eol: Eol::CrLf, syntax, placement }
    }

    /// Reads a `CInifile` document. Never fails: a line that is not LTX is kept as it is.
    #[must_use]
    pub fn parse(text: &str) -> Self {
        Self::parse_as(text, Syntax::Ini, Placement::default())
    }

    /// Reads a document by `syntax`. Never fails: a line that is not LTX is kept as it is. A
    /// BOM is dropped.
    #[must_use]
    pub fn parse_as(text: &str, syntax: Syntax, placement: Placement) -> Self {
        let text = text.strip_prefix('\u{feff}').unwrap_or(text);
        let physical = physical_lines(text);
        let eol = physical.iter().map(|l| l.1).find(|e| *e != Eol::None).unwrap_or(Eol::CrLf);
        let mut lines: Vec<Line> = Vec::with_capacity(physical.len());
        let mut i = 0;
        while let Some(&(first, first_eol)) = physical.get(i) {
            i += 1;
            let mut line_eol = first_eol;
            let mut kind = syntax.classify(first);
            let mut raw = first.to_owned();
            // a quoted value left open continues on the next lines, as CInifile reads it
            if syntax == Syntax::Ini
                && let Kind::Entry { key, value, .. } = &kind
                && sub(&raw, value).bytes().filter(|&b| b == b'"').count() % 2 == 1
            {
                let key = key.clone();
                let start = value.start;
                let open = |raw: &str| {
                    raw.get(start..).unwrap_or("").bytes().filter(|&b| b == b'"').count() % 2 == 1
                };
                while let Some(&(next, next_eol)) = physical.get(i)
                    && open(&raw)
                {
                    raw.push_str(line_eol.as_str());
                    raw.push_str(next);
                    line_eol = next_eol;
                    i += 1;
                }
                let end = raw.rfind('"').map_or(raw.len(), |q| q + 1).max(start);
                kind = Kind::Entry { code: key.start..end, key, value: start..end, eq: true };
            }
            lines.push(Line { raw, eol: line_eol, kind });
        }
        Self { lines, eol, syntax, placement }
    }

    #[must_use]
    pub fn syntax(&self) -> Syntax {
        self.syntax
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    /// Every line, in file order.
    pub fn items(&self) -> impl Iterator<Item = LineItem<'_>> {
        self.lines.iter().map(|l| match &l.kind {
            Kind::Section { name, .. } => LineItem::Section(name.as_str()),
            Kind::Entry { key, value, code, .. } => {
                LineItem::Entry { key: sub(&l.raw, key), value: sub(&l.raw, value), code: sub(&l.raw, code) }
            }
            Kind::Other => LineItem::Other,
        })
    }

    /// The section headers in file order (a section written twice is named twice).
    pub fn sections(&self) -> impl Iterator<Item = &str> {
        self.lines.iter().filter_map(|l| match &l.kind {
            Kind::Section { name, .. } => Some(name.as_str()),
            _ => None,
        })
    }

    /// The headers whose name starts with `prefix` (ASCII case ignored), in file order.
    pub fn sections_with_prefix<'a>(&'a self, prefix: &'a str) -> impl Iterator<Item = &'a str> {
        self.sections().filter(move |n| {
            n.len() >= prefix.len()
                && n.as_bytes().get(..prefix.len()).is_some_and(|h| h.eq_ignore_ascii_case(prefix.as_bytes()))
        })
    }

    fn is_header_of(line: &Line, section: &str) -> bool {
        matches!(&line.kind, Kind::Section { name, .. } if name.eq_ignore_ascii_case(section))
    }

    fn header(&self, section: &str) -> Option<usize> {
        self.lines.iter().position(|l| Self::is_header_of(l, section))
    }

    /// The end of the body that starts at `start`: the next header, or the end of the file.
    fn body_end(&self, start: usize) -> usize {
        self.lines
            .get(start..)
            .and_then(|rest| rest.iter().position(|l| matches!(l.kind, Kind::Section { .. })))
            .map_or(self.lines.len(), |n| start + n)
    }

    /// The bodies of a section: the lines after its header (`CInifile`: the first one; the
    /// manifest: each one), up to the next header.
    fn bodies(&self, section: &str) -> Vec<Range<usize>> {
        if self.syntax == Syntax::Ini {
            let start = if section.is_empty() {
                0
            } else {
                match self.header(section) {
                    Some(h) => h + 1,
                    None => return Vec::new(),
                }
            };
            return std::iter::once(start..self.body_end(start)).collect();
        }
        let mut out = Vec::new();
        let mut i = 0;
        while let Some(line) = self.lines.get(i) {
            if Self::is_header_of(line, section) {
                let end = self.body_end(i + 1);
                out.push(i + 1..end);
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

    #[must_use]
    pub fn has_section(&self, section: &str) -> bool {
        self.header(section).is_some()
    }

    /// What a section inherits from: `[section]:parent, parent`.
    #[must_use]
    pub fn parents(&self, section: &str) -> &[SmolStr] {
        match self.header(section).and_then(|i| self.lines.get(i)).map(|l| &l.kind) {
            Some(Kind::Section { parents, .. }) => parents,
            _ => &[],
        }
    }

    /// The `(key, value)` lines of a section in file order, duplicates included; values as
    /// written (quotes included).
    pub fn entries<'a>(&'a self, section: &str) -> impl Iterator<Item = (&'a str, &'a str)> + use<'a> {
        self.body_lines(section).into_iter().filter_map(move |i| {
            let l = self.lines.get(i)?;
            match &l.kind {
                Kind::Entry { key, value, .. } => Some((sub(&l.raw, key), sub(&l.raw, value))),
                _ => None,
            }
        })
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

    /// The value of a key, as written (quotes included). The last one when written twice.
    #[must_use]
    pub fn get(&self, section: &str, key: &str) -> Option<&str> {
        self.entries(section).filter(|(k, _)| *k == key).last().map(|(_, v)| v)
    }

    #[must_use]
    pub fn has_key(&self, section: &str, key: &str) -> bool {
        self.entries(section).any(|(k, _)| k == key)
    }

    fn new_line(&self, raw: String) -> Line {
        let raw = if self.syntax == Syntax::Manifest { one_line(&raw) } else { raw };
        Line { kind: self.syntax.classify(&raw), raw, eol: self.eol }
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

    fn last_is_blank(&self) -> bool {
        self.lines.last().is_some_and(|l| l.is_blank(self.syntax))
    }

    /// Adds an empty section where the placement puts it: before the first section of a higher
    /// rank (with the comment block leaning on that header), else at the end after a blank line.
    fn append_section(&mut self, section: &str) {
        let rank = (self.placement.section_rank)(section);
        let next = self.lines.iter().position(
            |l| matches!(&l.kind, Kind::Section { name, .. } if (self.placement.section_rank)(name) > rank),
        );
        let syntax = self.syntax;
        if let Some(mut at) = next {
            while at > 0
                && self
                    .lines
                    .get(at - 1)
                    .is_some_and(|l| matches!(l.kind, Kind::Other) && !l.is_blank(syntax))
            {
                at -= 1;
            }
            let header = self.new_line(format!("[{section}]"));
            let blank = self.new_line(String::new());
            let lead_blank = at > 0 && self.lines.get(at - 1).is_some_and(|l| !l.is_blank(syntax));
            self.lines.insert(at, blank);
            self.lines.insert(at, header);
            if lead_blank {
                let blank = self.new_line(String::new());
                self.lines.insert(at, blank);
            }
            self.close_lines();
            return;
        }
        if !self.lines.is_empty() && !self.last_is_blank() {
            let blank = self.new_line(String::new());
            self.lines.push(blank);
        }
        let header = self.new_line(format!("[{section}]"));
        self.lines.push(header);
        self.close_lines();
    }

    /// Adds an empty section unless it exists, where the placement puts it.
    pub fn ensure_section(&mut self, section: &str) {
        if (section.is_empty() && self.syntax == Syntax::Ini) || self.has_section(section) {
            return;
        }
        self.append_section(section);
    }

    /// The text a new `key = value` line of `CInifile` takes, shaped like `model`.
    fn ini_shape(model: Option<&Line>, key: &str, value: &str) -> String {
        let mut raw = String::new();
        let mut separator = " = ".to_owned();
        if let Some(Line { raw: m, kind: Kind::Entry { key: mkey, value: mvalue, .. }, .. }) = model
            && let Some(eq) = m.get(..ini_code_end(m)).and_then(|c| c.find('='))
        {
            let indent = &m[..m.len() - m.trim_start().len()];
            raw.push_str(indent);
            raw.push_str(key);
            let key_end = indent.len() + mkey.len();
            let before = m.get(key_end.min(eq)..eq).unwrap_or("");
            // keep the '=' in its column when keys are padded to one
            let here = raw.len();
            let pad = if before.is_empty() { 0 } else { eq.saturating_sub(here).max(1) };
            separator =
                format!("{}={}", " ".repeat(pad), m.get(eq + 1..mvalue.start.max(eq + 1)).unwrap_or(""));
            if mvalue.is_empty() && separator.ends_with('=') {
                separator.push(' ');
            }
        } else {
            raw.push_str(key);
        }
        if value.is_empty() {
            raw.push_str(separator.trim_end());
            return raw;
        }
        raw.push_str(&separator);
        raw.push_str(value);
        raw
    }

    /// The text of a new manifest entry, shaped like `model` (its indent and the column of its
    /// `=`), or padded to the placement's width when there is none.
    fn manifest_shape(&self, model: Option<&Line>, section: &str, key: &str, value: &str) -> String {
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
            return out.trim_end_matches(MANIFEST_WS).to_owned();
        }
        padded_entry(key, value, (self.placement.key_width)(section))
    }

    /// The entry a new key of `section` is shaped like.
    fn model_entry(&self, section: &str) -> Option<&Line> {
        let usable = |l: &&Line| match (&l.kind, self.syntax) {
            (Kind::Entry { .. }, Syntax::Ini) => true,
            // a line with a value shows how this file spaces one; `key =` alone does not
            (Kind::Entry { value, eq: true, .. }, Syntax::Manifest) => !value.is_empty(),
            _ => false,
        };
        let own = self.body_lines(section).into_iter().rev().filter_map(|i| self.lines.get(i)).find(usable);
        own.or_else(|| self.lines.iter().find(usable))
    }

    /// Where a new entry of a section goes: after its last entry, or right after its last header.
    /// A missing section is added first.
    fn insert_point(&mut self, section: &str) -> usize {
        self.ensure_section(section);
        let bodies = self.bodies(section);
        let last_entry =
            bodies.iter().flat_map(Clone::clone).rfind(|&i| self.lines.get(i).is_some_and(Line::is_entry));
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
    /// comment; a new line goes after the section's last entry (or where the placement ranks
    /// the key), shaped like its neighbours; a missing section is added.
    pub fn set(&mut self, section: &str, key: &str, value: &str) {
        let syntax = self.syntax;
        let value = if syntax == Syntax::Manifest { one_line(value) } else { value.to_owned() };
        let existing = self
            .body_lines(section)
            .into_iter()
            .rev()
            .find(|&i| self.lines.get(i).and_then(Line::key) == Some(key));
        if let Some(i) = existing
            && let Some(line) = self.lines.get_mut(i)
        {
            if let Kind::Entry { value: range, code, eq, key: key_range } = line.kind.clone() {
                match syntax {
                    Syntax::Ini => {
                        let insert = if eq { value.clone() } else { format!(" = {value}") };
                        line.raw.replace_range(range.clone(), &insert);
                        let start = range.start + (insert.len() - value.len());
                        let end = start + value.len();
                        line.kind = Kind::Entry {
                            key: key_range,
                            value: start..end,
                            code: code.start..end,
                            eq: true,
                        };
                    }
                    Syntax::Manifest => {
                        if eq {
                            line.raw.replace_range(range, &value);
                        } else if !value.is_empty() {
                            line.raw.insert_str(code.end, &format!(" = {value}"));
                        }
                        line.kind = syntax.classify(&line.raw);
                    }
                }
            }
            return;
        }
        let raw = match syntax {
            Syntax::Ini => Self::ini_shape(self.model_entry(section), key, &value),
            Syntax::Manifest => self.manifest_shape(self.model_entry(section), section, key, &value),
        };
        let mut at = self.insert_point(section);
        // a key the placement ranks goes before the first one of a higher rank
        let rank = (self.placement.key_rank)(section, key);
        if rank != usize::MAX
            && let Some(i) = self.body_lines(section).into_iter().find(|&i| {
                self.lines
                    .get(i)
                    .and_then(Line::key)
                    .is_some_and(|k| (self.placement.key_rank)(section, k) > rank)
            })
        {
            at = i;
        }
        let line = self.new_line(raw);
        self.insert(at, line);
    }

    /// Replaces the first entry whose key matches `key` without ASCII case by `raw`. `false`
    /// when there is none.
    pub fn set_line_ci(&mut self, section: &str, key: &str, raw: &str) -> bool {
        let found = self
            .body_lines(section)
            .into_iter()
            .find(|&i| self.lines.get(i).and_then(Line::key).is_some_and(|k| k.eq_ignore_ascii_case(key)));
        let Some(i) = found else { return false };
        let line = self.new_line(raw.to_owned());
        if let Some(slot) = self.lines.get_mut(i) {
            slot.raw = line.raw;
            slot.kind = line.kind;
        }
        true
    }

    /// Adds the line `raw` after the section's last entry.
    pub fn push_line(&mut self, section: &str, raw: &str) {
        let at = self.insert_point(section);
        let line = self.new_line(raw.to_owned());
        self.insert(at, line);
    }

    /// Removes every entry of `section` whose key `pred` accepts. Returns whether one went.
    pub fn remove_where(&mut self, section: &str, pred: impl Fn(&str) -> bool) -> bool {
        let doomed: Vec<usize> = self
            .body_lines(section)
            .into_iter()
            .filter(|&i| self.lines.get(i).and_then(Line::key).is_some_and(&pred))
            .collect();
        for &i in doomed.iter().rev() {
            if i < self.lines.len() {
                self.lines.remove(i);
            }
        }
        if !doomed.is_empty() && self.syntax == Syntax::Manifest {
            self.close_lines();
        }
        !doomed.is_empty()
    }

    /// Removes every line of a key. Returns whether there was one.
    pub fn remove(&mut self, section: &str, key: &str) -> bool {
        self.remove_where(section, |k| k == key)
    }

    /// Replaces the entries of a section by `entries`, in that order, shaped like its first
    /// entry. Comments inside the section stay where they are; the section is added when
    /// missing.
    pub fn replace_entries<K: AsRef<str>, V: AsRef<str>>(
        &mut self,
        section: &str,
        entries: impl IntoIterator<Item = (K, V)>,
    ) {
        self.ensure_section(section);
        let Some(body) = self.bodies(section).into_iter().next() else { return };
        let model = body
            .clone()
            .find(|&i| self.lines.get(i).is_some_and(Line::is_entry))
            .and_then(|i| self.lines.get(i).cloned());
        let lines: Vec<String> = entries
            .into_iter()
            .map(|(k, v)| match self.syntax {
                Syntax::Ini => Self::ini_shape(model.as_ref(), k.as_ref(), v.as_ref()),
                Syntax::Manifest => self.manifest_shape(model.as_ref(), section, k.as_ref(), v.as_ref()),
            })
            .collect();
        self.set_lines(section, &lines);
    }

    /// Replaces the entries of a section by `lines` (raw entry text, in that order), where its
    /// first entry was. Comments in the section stay where they are; a section written twice
    /// keeps its later headers with no entries. A missing section is added unless `lines` is
    /// empty.
    pub fn set_lines(&mut self, section: &str, lines: &[String]) {
        if !self.has_section(section) && !(section.is_empty() && self.syntax == Syntax::Ini) {
            if lines.is_empty() {
                return;
            }
            self.append_section(section);
        }
        let bodies = self.bodies(section);
        let in_body: Vec<usize> = bodies.iter().flat_map(Clone::clone).collect();
        let first_entry = in_body.iter().copied().find(|&i| self.lines.get(i).is_some_and(Line::is_entry));
        let insert_at = first_entry.or_else(|| bodies.first().map(|b| b.start)).unwrap_or(self.lines.len());
        let mut fresh: Vec<Line> = lines.iter().map(|l| self.new_line(l.clone())).collect();
        let old = std::mem::take(&mut self.lines);
        let mut out: Vec<Line> = Vec::with_capacity(old.len() + fresh.len());
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

    /// Removes a section (`CInifile`: its first header; the manifest: every header of that
    /// name) with its body. A comment block that leans on the next header stays with that
    /// header. Returns whether the section existed.
    pub fn remove_section(&mut self, section: &str) -> bool {
        if section.is_empty() && self.syntax == Syntax::Ini {
            return false;
        }
        let syntax = self.syntax;
        let mut any = false;
        while let Some(body) = self.bodies(section).into_iter().next() {
            any = true;
            let header = body.start.saturating_sub(1);
            let mut end = body.end;
            if end < self.lines.len() {
                while end > body.start
                    && self
                        .lines
                        .get(end - 1)
                        .is_some_and(|l| matches!(l.kind, Kind::Other) && !l.is_blank(syntax))
                {
                    end -= 1;
                }
            }
            self.lines.drain(header..end);
            if header >= self.lines.len() {
                // the section closed the file: blank lines that separated it go with it
                while self.last_is_blank() {
                    self.lines.pop();
                }
            }
            if syntax == Syntax::Ini {
                break;
            }
        }
        any
    }

    /// Removes a section that holds nothing but blank lines: what an editor leaves when its last
    /// key goes. A section with a comment of the author's stays.
    pub fn prune(&mut self, section: &str) -> bool {
        let bodies = self.bodies(section);
        if bodies.is_empty() || !self.has_section(section) {
            return false;
        }
        let syntax = self.syntax;
        let empty = bodies
            .iter()
            .flat_map(Clone::clone)
            .all(|i| self.lines.get(i).is_some_and(|l| l.is_blank(syntax)));
        empty && self.remove_section(section)
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
    let bare = !value.is_empty()
        && value.trim() == value
        && !value.contains(';')
        && !value.contains("//")
        && !value.contains('"');
    if bare { value.to_owned() } else { format!("\"{}\"", value.replace('"', "'")) }
}

/// The engine's `r_bool`: `on`, `yes`, `true`, `1`.
#[must_use]
pub fn parse_bool(value: &str) -> bool {
    let v = unquote(value);
    v.eq_ignore_ascii_case("on")
        || v.eq_ignore_ascii_case("yes")
        || v.eq_ignore_ascii_case("true")
        || v == "1"
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "; the manifest\r\n[module]\r\nname        = \"Overnight; Delivery\" ; title\r\nversion     = 1.2.0\r\n\r\n; rules\r\n[world] : base\r\nmutants = false\r\n";

    fn manifest(text: &str) -> LtxDoc {
        LtxDoc::parse_as(text, Syntax::Manifest, Placement::default())
    }

    #[test]
    fn untouched_is_identical() {
        for text in [
            SAMPLE,
            "",
            "a = 1",
            "[s]\nk=v\n",
            "\r\n\r\n",
            "[s]\r\nk = \"line one\r\nline two\" ; c\r\nz = 1\r\n",
            "#include \"a.ltx\"\r\n[s]\r\n[broken\r\n",
        ] {
            assert_eq!(LtxDoc::parse(text).to_string(), text);
            assert_eq!(manifest(text).to_string(), text);
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
        let multi = LtxDoc::parse("[s]\nk = \"one\ntwo\"\n");
        assert_eq!(multi.get("s", "k"), Some("\"one\ntwo\""));
    }

    #[test]
    fn the_two_syntaxes_read_one_text_their_own_way() {
        let text =
            "#x = 1\n[s] : base\nsite = https://a.b ; c\nq = \"a; \\\"b\\\"; c\" ; d\nraw line\n[s]\nk = 2\n";
        let ini = LtxDoc::parse(text);
        let man = manifest(text);
        assert_eq!(ini.get("s", "site"), Some("https:"));
        assert_eq!(man.get("s", "site"), Some("https://a.b"));
        assert_eq!(man.get("s", "q"), Some("\"a; \\\"b\\\"; c\""));
        assert_eq!(ini.parents("s"), [SmolStr::new("base")]);
        assert!(man.parents("s").is_empty());
        // a section written twice: the engine reads its first header, the manifest both
        assert_eq!(ini.get("s", "k"), None);
        assert_eq!(man.get("s", "k"), Some("2"));
        assert_eq!(
            man.section_lines("s"),
            ["site = https://a.b", "q = \"a; \\\"b\\\"; c\"", "raw line", "k = 2"]
        );
        assert_eq!(man.items().next(), Some(LineItem::Entry { key: "#x", value: "1", code: "#x = 1" }));
        assert_eq!(ini.items().next(), Some(LineItem::Other));
    }

    #[test]
    fn an_edit_touches_its_line_only() {
        let mut doc = LtxDoc::parse(SAMPLE);
        doc.set("module", "version", "1.3.0");
        assert_eq!(doc.to_string(), SAMPLE.replace("1.2.0", "1.3.0"));
        doc.set("module", "name", "\"New\"");
        assert!(doc.to_string().contains("name        = \"New\" ; title\r\n"));
        let mut bare = LtxDoc::parse("[s]\nflag\n");
        bare.set("s", "flag", "1");
        assert_eq!(bare.to_string(), "[s]\nflag = 1\n");
        assert_eq!(bare.get("s", "flag"), Some("1"));
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
        assert!(!doc.remove_section(""));
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
    fn placement_orders_what_is_new() {
        fn rank(s: &str) -> usize {
            ["a", "b", "c"].iter().position(|o| *o == s).unwrap_or(usize::MAX)
        }
        fn key_rank(_: &str, k: &str) -> usize {
            ["x", "y"].iter().position(|o| *o == k).unwrap_or(usize::MAX)
        }
        let placement = Placement { section_rank: rank, key_rank, key_width: |_| 4 };
        let mut doc = LtxDoc::parse_as("[a]\ny = 1\n\n; c's\n[c]\n", Syntax::Manifest, placement);
        doc.set("b", "k", "v");
        doc.set("a", "x", "0");
        assert_eq!(doc.to_string(), "[a]\nx = 0\ny = 1\n\n[b]\nk = v\n\n; c's\n[c]\n");
        let mut fresh = LtxDoc::new_as(Syntax::Manifest, placement);
        fresh.set("a", "k", "v");
        assert_eq!(fresh.to_string(), "[a]\r\nk    = v\r\n");
    }

    #[test]
    fn quoting() {
        assert_eq!(quote("plain"), "plain");
        assert_eq!(quote("a; b"), "\"a; b\"");
        assert_eq!(quote(""), "\"\"");
        assert_eq!(unquote("\"x\""), "x");
        assert_eq!(find_manifest_comment("x = \"open ; c"), Some(10));
        assert_eq!(padded_entry("id", "", 4), "id   =");
    }
}
