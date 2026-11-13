//! A declarative Lua file kept as a lossless document ([`LuaDoc`]): the text is the document,
//! byte for byte, and the syntax tree over it knows where every table, key, value and separator
//! is. Reading gives the value ([`LuaDoc::value`]); writing is an edit of the bytes a change is
//! about ([`LuaDoc::set`], [`LuaDoc::remove`], [`LuaDoc::insert`], [`LuaDoc::move_item`]), or
//! the difference between two values applied at once ([`LuaDoc::apply_diff`]). Comments,
//! blank lines, key order, quoting, number spelling and keys no model knows stay as they are.
//!
//! What a file means is decided by the reader: for a file the game reads ([`Limits::asset`])
//! the LuaJIT model of [`crate::parse_named`] judges it first, with the same messages; for a
//! file only the editor reads ([`Limits::scene`], megabytes) the document's own parser judges
//! it with the same rules and wider limits.

mod cst;
mod diff;
mod frag;
mod run;

use smol_str::SmolStr;

pub use diff::{Conflict, DiffRules};

use crate::LuaDataError;
use crate::path::Seg;
use crate::value::{Table, Value};
use cst::{KeyN, PErr, TableN, Tree};
use frag::Layout;
use run::{Ev, Lv};

/// What a reader accepts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The deepest nesting of tables (the root is depth 0).
    pub depth: u32,
    /// The most values a file holds, tables included.
    pub values: u32,
    /// The game reads the file: the LuaJIT model judges it (its syntax and compile limits).
    pub game: bool,
}

impl Limits {
    /// An asset the game reads: the game's limits.
    #[must_use]
    pub const fn asset() -> Self {
        Self { depth: 48, values: 200_000, game: true }
    }

    /// A file only the editor reads, of a few megabytes (a scene's data files).
    #[must_use]
    pub const fn scene() -> Self {
        Self { depth: 64, values: 50_000_000, game: false }
    }
}

/// Where something stands in the text: 1-based lines and columns, columns counted in
/// characters, the end exclusive.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize)]
pub struct Span {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// Where [`LuaDoc::insert`] puts a new entry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Slot {
    /// A named field: after the field `after`, or last.
    Key { key: SmolStr, after: Option<SmolStr> },
    /// A named field before every other entry (where a header key goes).
    Head(SmolStr),
    /// The n-th item of the array part (from 0; the length appends).
    Position(usize),
}

/// One replacement of bytes.
#[derive(Clone, Debug)]
pub(crate) struct Splice {
    pub start: u32,
    pub end: u32,
    pub text: String,
}

/// Where an inserted entry goes, relative to the entries a table keeps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Anchor {
    After(usize),
    Before(usize),
    End,
}

/// What an insert writes.
#[derive(Clone, Debug)]
pub(crate) enum Body {
    /// A new entry, laid out canonically.
    Entry { key: Option<SmolStr>, value: Value, node: bool },
    /// An entry moved as it was written: its comment lines (indented, with their line ends)
    /// and its text.
    Raw { comments: String, text: String },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct TableEdits {
    pub removed: Vec<usize>,
    pub inserts: Vec<(Anchor, Body)>,
}

/// A declarative Lua file as a lossless document.
#[derive(Clone, Debug)]
pub struct LuaDoc {
    text: String,
    tree: Tree,
    limits: Limits,
    bom: usize,
    layout: Layout,
    name: String,
}

fn plain(message: impl Into<String>) -> LuaDataError {
    LuaDataError { line: 0, column: 0, message: message.into() }
}

/// The 1-based line and character column of a byte offset.
pub(crate) fn position(text: &str, lines: &[u32], off: usize) -> (u32, u32) {
    let off = off.min(text.len());
    let line = lines.partition_point(|&l| l as usize <= off).max(1);
    let start = lines.get(line - 1).map_or(0, |&l| l as usize);
    let col = text.get(start..off).map_or(off - start, |s| s.chars().count()) + 1;
    (u32::try_from(line).unwrap_or(u32::MAX), u32::try_from(col).unwrap_or(u32::MAX))
}

pub(crate) use cst::line_starts;

/// Where the document's reader finds a refusal of the game's reader: the same message from the
/// syntax tree or from the run of its value. `skip`: bytes before what the game's reader saw.
pub(crate) fn refusal_at(text: &str, skip: usize, message: &str) -> Option<usize> {
    let tree = match cst::parse(text.as_bytes(), skip) {
        Ok(t) => t,
        Err(e) => return (e.msg == message).then_some(e.at),
    };
    if message.starts_with("execution limit") {
        return Some(tree.ret_at as usize);
    }
    let lim = Limits::asset();
    let ev = Ev { s: text.as_bytes(), tree: &tree, max_depth: lim.depth, max_values: lim.values };
    match ev.value(tree.root) {
        Err(e) if e.msg == message => Some(e.at),
        _ => None,
    }
}

fn layout_of(text: &str, tree: &Tree) -> Layout {
    let nl = match text.find('\n') {
        Some(i) if i > 0 && text.as_bytes().get(i - 1) == Some(&b'\r') => "\r\n",
        _ => "\n",
    };
    let b = text.as_bytes();
    let line_indent = |off: u32| -> &str {
        let ls =
            tree.lines.get(tree.lines.partition_point(|&l| l <= off).saturating_sub(1)).copied().unwrap_or(0);
        let mut e = ls as usize;
        while matches!(b.get(e), Some(b' ' | b'\t')) {
            e += 1;
        }
        if e == off as usize { text.get(ls as usize..e).unwrap_or("") } else { "\u{0}" }
    };
    let mut unit = String::new();
    // the indentation of the first entry that starts a line, less its table's
    for t in &tree.tables {
        let outer = line_indent_any(text, tree, t.open);
        for e in &t.entries {
            let ind = line_indent(e.start);
            if ind != "\u{0}" && ind.len() > outer.len() && ind.starts_with(outer) {
                unit = ind.get(outer.len()..).unwrap_or("").to_owned();
                break;
            }
        }
        if !unit.is_empty() {
            break;
        }
    }
    if unit.starts_with('\t') {
        unit = "\t".into();
    }
    if unit.is_empty() || unit.contains(|c| c != ' ' && c != '\t') {
        unit = "  ".into();
    }
    Layout { unit, nl }
}

/// Whether a text is a line comment (`--` that is no long comment).
fn line_comment(s: &str) -> bool {
    let b = s.as_bytes();
    s.starts_with("--") && !(b.get(2) == Some(&b'[') && cst::long_open(b, 2).is_some())
}

/// The leading spaces and tabs of the line holding `off`.
fn line_indent_any<'t>(text: &'t str, tree: &Tree, off: u32) -> &'t str {
    let ls =
        tree.lines.get(tree.lines.partition_point(|&l| l <= off).saturating_sub(1)).copied().unwrap_or(0);
    let b = text.as_bytes();
    let mut e = ls as usize;
    while matches!(b.get(e), Some(b' ' | b'\t')) && e < off as usize {
        e += 1;
    }
    text.get(ls as usize..e).unwrap_or("")
}

impl LuaDoc {
    /// Reads an asset the game reads ([`Limits::asset`]).
    pub fn parse(text: &str) -> Result<Self, LuaDataError> {
        Self::parse_with(text, "", Limits::asset())
    }

    /// Reads a file with the given reader limits; `name` is the chunk name of the messages
    /// ([`crate::parse_named`]).
    pub fn parse_with(text: &str, name: &str, limits: Limits) -> Result<Self, LuaDataError> {
        Self::read(text, name, limits, false).map(|(d, _)| d)
    }

    /// [`LuaDoc::parse_with`] that also gives the value, read once.
    pub fn parse_value(text: &str, name: &str, limits: Limits) -> Result<(Self, Value), LuaDataError> {
        let (d, v) = Self::read(text, name, limits, true)?;
        let v = match v {
            Some(v) => v,
            None => d.value(),
        };
        Ok((d, v))
    }

    /// A new file: the canonical text of `top` ([`crate::write_asset`] without a header).
    #[must_use]
    pub fn from_value(top: &Table, limits: Limits) -> Option<Self> {
        Self::parse_with(&crate::write_asset("", top), "", limits).ok()
    }

    fn read(
        text: &str,
        name: &str,
        limits: Limits,
        want: bool,
    ) -> Result<(Self, Option<Value>), LuaDataError> {
        let bom = if text.starts_with('\u{feff}') { 3 } else { 0 };
        let judged = if limits.game {
            Some(crate::judge(text.get(bom..).unwrap_or(""), name).map_err(|j| crate::locate(text, bom, j))?)
        } else {
            None
        };
        let tree = cst::parse(text.as_bytes(), bom).map_err(|e| Self::err_in(text, e))?;
        let layout = layout_of(text, &tree);
        let doc = Self { text: text.to_owned(), tree, limits, bom, layout, name: name.to_owned() };
        let value = match judged {
            Some(v) if want => Some(v),
            Some(_) => None,
            None => Some(doc.ev().value(doc.tree.root).map_err(|e| Self::err_in(text, e))?),
        };
        Ok((doc, value))
    }

    fn err_in(text: &str, e: PErr) -> LuaDataError {
        let lines = cst::line_starts(text.as_bytes());
        let (line, column) = position(text, &lines, e.at);
        LuaDataError { line, column, message: e.msg }
    }

    pub(crate) fn ev(&self) -> Ev<'_> {
        Ev {
            s: self.text.as_bytes(),
            tree: &self.tree,
            max_depth: self.limits.depth,
            max_values: self.limits.values,
        }
    }

    /// The text: the input of an unchanged document, byte for byte.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn into_text(self) -> String {
        self.text
    }

    #[must_use]
    pub fn limits(&self) -> Limits {
        self.limits
    }

    /// Whether the file starts with a byte order mark (kept; the game refuses one).
    #[must_use]
    pub fn has_bom(&self) -> bool {
        self.bom > 0
    }

    /// The value the file holds, as [`crate::parse_named`] reads it.
    #[must_use]
    pub fn value(&self) -> Value {
        self.ev().value(self.tree.root).unwrap_or_else(|_| Value::table())
    }

    /// The 1-based line and column of a byte offset.
    #[must_use]
    pub fn position(&self, offset: usize) -> (u32, u32) {
        position(&self.text, &self.tree.lines, offset)
    }

    fn span_of(&self, start: u32, end: u32) -> Span {
        let (line, column) = self.position(start as usize);
        let (end_line, end_column) = self.position(end as usize);
        Span { line, column, end_line, end_column }
    }

    fn table(&self, id: u32) -> Option<&TableN> {
        self.tree.tables.get(id as usize)
    }

    /// The entry of table `tid` that one step names.
    fn find(&self, tid: u32, seg: &Seg) -> Option<usize> {
        let ev = self.ev();
        let eff = ev.effective(tid).ok()?;
        match seg {
            Seg::Key(k) => eff.key(k.as_bytes()),
            Seg::Index(n) => eff.nums.get(*n).map(|x| x.1),
            Seg::Item { field, value } => eff
                .nums
                .iter()
                .map(|x| x.1)
                .find(|&j| self.item_key(tid, j, field).is_some_and(|k| k == value.as_str())),
        }
    }

    /// The `field` of the item that entry `j` of table `tid` holds, as text.
    pub(crate) fn item_key(&self, tid: u32, j: usize, field: &str) -> Option<String> {
        let ev = self.ev();
        let ct = self.table(tid)?.entries.get(j)?.val.table?;
        let item = self.table(ct)?;
        // a field written once by name, in a table without `[key]` entries, is the one
        let named = |e: &&cst::EntryN| matches!(e.key, KeyN::Name { start, end } if self.text.as_bytes().get(start as usize..end as usize) == Some(field.as_bytes()));
        let plain = item.entries.iter().all(|e| !matches!(e.key, KeyN::Bracket { .. }));
        let mut hits = item.entries.iter().filter(named);
        let fe = match (plain, hits.next(), hits.next()) {
            (true, Some(e), None) => e,
            (true, None, _) => return None,
            _ => {
                let fj = ev.effective(ct).ok()?.key(field.as_bytes())?;
                item.entries.get(fj)?
            }
        };
        match ev.scalar(fe.val).ok()?.0 {
            Lv::Str(s) => Some(String::from_utf8_lossy(&s).into_owned()),
            Lv::Num(n) => Some(Value::Number(n).to_text("")),
            _ => None,
        }
    }

    /// The table and entry a path names; `None` for the root and for a path that names
    /// nothing.
    fn locate(&self, path: &[Seg]) -> Option<(u32, usize)> {
        let mut tid = self.tree.root;
        for (i, seg) in path.iter().enumerate() {
            let j = self.find(tid, seg)?;
            if i + 1 == path.len() {
                return Some((tid, j));
            }
            tid = self.table(tid)?.entries.get(j)?.val.table?;
        }
        None
    }

    /// The table a path names.
    fn table_at(&self, path: &[Seg]) -> Option<u32> {
        if path.is_empty() {
            return Some(self.tree.root);
        }
        let (tid, j) = self.locate(path)?;
        self.table(tid)?.entries.get(j)?.val.table
    }

    pub(crate) fn entry_value(&self, tid: u32, j: usize) -> Value {
        let ev = self.ev();
        let Some(e) = self.table(tid).and_then(|t| t.entries.get(j)) else { return Value::Nil };
        if let Some(ct) = e.val.table {
            return ev.value(ct).unwrap_or_else(|_| Value::table());
        }
        match ev.scalar(e.val) {
            Ok((Lv::Nil, _)) | Err(_) => Value::Nil,
            Ok((Lv::Bool(b), _)) => Value::Bool(b),
            Ok((Lv::Num(n), _)) => Value::Number(n),
            Ok((Lv::Str(s), _)) => Value::String(SmolStr::new(String::from_utf8_lossy(&s))),
            Ok((Lv::Table(t), _)) => ev.value(t).unwrap_or_else(|_| Value::table()),
            Ok((Lv::Cdata, _)) => Value::Nil,
        }
    }

    /// The value a path names.
    #[must_use]
    pub fn get(&self, path: &[Seg]) -> Option<Value> {
        if path.is_empty() {
            return Some(self.value());
        }
        let (tid, j) = self.locate(path)?;
        Some(self.entry_value(tid, j))
    }

    /// Where the thing a path names is written: the whole entry, from its key to the end of
    /// its value; the root table for the empty path.
    #[must_use]
    pub fn span(&self, path: &[Seg]) -> Option<Span> {
        if path.is_empty() {
            let t = self.table(self.tree.root)?;
            return Some(self.span_of(t.open, t.close + 1));
        }
        let (tid, j) = self.locate(path)?;
        let e = self.table(tid)?.entries.get(j)?;
        Some(self.span_of(e.start, e.val.end))
    }

    /// Where the value a path names is written.
    #[must_use]
    pub fn value_span(&self, path: &[Seg]) -> Option<Span> {
        if path.is_empty() {
            return self.span(path);
        }
        let (tid, j) = self.locate(path)?;
        let e = self.table(tid)?.entries.get(j)?;
        Some(self.span_of(e.val.start, e.val.end))
    }

    /// The span of the longest prefix of `path` that names something: where a problem about
    /// a thing that is not written points.
    #[must_use]
    pub fn nearest_span(&self, path: &[Seg]) -> Span {
        for n in (0..=path.len()).rev() {
            if let Some(s) = path.get(..n).and_then(|p| self.span(p)) {
                return s;
            }
        }
        self.span_of(self.tree.ret_at, self.tree.ret_at)
    }

    /// The span of the `return` that opens the file: where a problem of the whole file points.
    #[must_use]
    pub fn head_span(&self) -> Span {
        let end = self.table(self.tree.root).map_or(self.tree.ret_at + 6, |t| t.open + 1);
        self.span_of(self.tree.ret_at, end)
    }

    /// The string keys a table holds, in the file's order, with their spans.
    #[must_use]
    pub fn keys(&self, table: &[Seg]) -> Vec<(SmolStr, Span)> {
        let Some(tid) = self.table_at(table) else { return Vec::new() };
        let Ok(eff) = self.ev().effective(tid) else { return Vec::new() };
        let Some(t) = self.table(tid) else { return Vec::new() };
        let mut out: Vec<(usize, SmolStr)> =
            eff.strs.iter().map(|(k, j)| (*j, SmolStr::new(String::from_utf8_lossy(k)))).collect();
        out.sort_by_key(|x| x.0);
        out.into_iter()
            .filter_map(|(j, k)| t.entries.get(j).map(|e| (k, self.span_of(e.start, e.val.end))))
            .collect()
    }

    // -- edits ---------------------------------------------------------------------------------

    /// Applies a batch of splices: touching ranges of pure removals merge, any other overlap is
    /// refused. The new text is read again; a text the reader would refuse is not taken.
    pub(crate) fn commit(&mut self, mut edits: Vec<Splice>) -> Result<bool, LuaDataError> {
        if edits.is_empty() {
            return Ok(false);
        }
        edits.sort_by_key(|e| (e.start, e.end));
        let mut merged: Vec<Splice> = Vec::with_capacity(edits.len());
        for e in edits {
            if let Some(last) = merged.last_mut()
                && e.start < last.end
            {
                if e.text.is_empty() && last.text.is_empty() {
                    last.end = last.end.max(e.end);
                    continue;
                }
                return Err(plain("two edits of one change overlap"));
            }
            merged.push(e);
        }
        let mut out =
            String::with_capacity(self.text.len() + merged.iter().map(|e| e.text.len()).sum::<usize>());
        let mut at = 0usize;
        for e in &merged {
            out.push_str(self.text.get(at..e.start as usize).unwrap_or(""));
            out.push_str(&e.text);
            at = e.end as usize;
        }
        out.push_str(self.text.get(at..).unwrap_or(""));
        if out == self.text {
            return Ok(false);
        }
        if self.limits.game {
            crate::judge(out.get(self.bom..).unwrap_or(""), &self.name)
                .map_err(|j| crate::locate(&out, self.bom, j))?;
        }
        let lo = merged.first().map_or(0, |e| e.start);
        let hi = merged.iter().map(|e| e.end).max().unwrap_or(0);
        let delta = out.len() as i64 - self.text.len() as i64;
        if !cst::reparse_within(&mut self.tree, out.as_bytes(), lo, hi, delta) {
            self.tree = cst::parse(out.as_bytes(), self.bom).map_err(|e| Self::err_in(&out, e))?;
        }
        // the tests hold every moved tree to a fresh read (but the speed test's)
        #[cfg(test)]
        if out.len() < 1 << 20 {
            assert_eq!(self.tree, cst::parse(out.as_bytes(), self.bom).map_err(|e| e.msg).unwrap());
        }
        self.text = out;
        Ok(true)
    }

    /// Sets the value a path names: in place when it is written, as a new entry of its table
    /// when it is not; nil removes it. The empty path replaces the whole table.
    pub fn set(&mut self, path: &[Seg], value: &Value) -> Result<(), LuaDataError> {
        if !frag::writable(value) {
            return Err(plain("a NaN cannot be written as data"));
        }
        if value.is_nil() {
            return self.remove(path).map(|_| ());
        }
        let Some((last, parent)) = path.split_last() else {
            if !value.is_table() {
                return Err(plain("the file must return one table"));
            }
            let t = self.table(self.tree.root).ok_or_else(|| plain("no table"))?;
            let used = frag::width(line_indent_any(&self.text, &self.tree, t.open));
            let text = frag::value(value, "", used, &self.layout);
            let s = Splice { start: t.open, end: t.close + 1, text };
            return self.commit(vec![s]).map(|_| ());
        };
        let tid = self
            .table_at(parent)
            .ok_or_else(|| plain(format!("nothing at {}", crate::path::show(parent))))?;
        if let Some(j) = self.find(tid, last) {
            let s = self.replace_splice(tid, j, value);
            return self.commit(vec![s]).map(|_| ());
        }
        let body = match last {
            Seg::Key(k) => Body::Entry { key: Some(k.clone()), value: value.clone(), node: false },
            Seg::Index(n) if *n == self.ev().effective(tid).map(|e| e.nums.len()).unwrap_or(0) => {
                Body::Entry { key: None, value: value.clone(), node: false }
            }
            _ => return Err(plain(format!("nothing at {}", crate::path::show(path)))),
        };
        let anchor = self.end_anchor(tid, matches!(last, Seg::Index(_)));
        let mut ed = TableEdits::default();
        ed.inserts.push((anchor, body));
        let mut out = Vec::new();
        self.table_splices(tid, &ed, &mut out)?;
        self.commit(out).map(|_| ())
    }

    /// Where something appended to a table goes: after its last array item for an item,
    /// last otherwise.
    fn end_anchor(&self, tid: u32, item: bool) -> Anchor {
        if item
            && let Ok(eff) = self.ev().effective(tid)
            && let Some(&(_, j)) = eff.nums.last()
        {
            return Anchor::After(j);
        }
        Anchor::End
    }

    /// Removes what a path names, with the comment lines directly above it and its comment on
    /// the same line; a key written twice goes in every place. False when nothing was there.
    pub fn remove(&mut self, path: &[Seg]) -> Result<bool, LuaDataError> {
        let Some((tid, j)) = self.locate(path) else { return Ok(false) };
        let mut ed = TableEdits::default();
        ed.removed.push(j);
        if let Some(Seg::Key(k)) = path.last() {
            ed.removed.extend(self.entries_of_key(tid, k).into_iter().filter(|&x| x != j));
        }
        let mut out = Vec::new();
        self.table_splices(tid, &ed, &mut out)?;
        self.commit(out)
    }

    /// Every entry of a table written with key `k`.
    fn entries_of_key(&self, tid: u32, k: &str) -> Vec<usize> {
        let ev = self.ev();
        let Some(t) = self.table(tid) else { return Vec::new() };
        t.entries
            .iter()
            .enumerate()
            .filter(|(_, e)| match e.key {
                KeyN::Name { start, end } => self.text.get(start as usize..end as usize) == Some(k),
                KeyN::Bracket { expr, .. } => {
                    matches!(ev.scalar(expr), Ok((Lv::Str(s), _)) if s.as_ref() == k.as_bytes())
                }
                KeyN::Pos => false,
            })
            .map(|(i, _)| i)
            .collect()
    }

    /// Inserts a new entry into the table a path names.
    pub fn insert(&mut self, table: &[Seg], slot: Slot, value: &Value) -> Result<(), LuaDataError> {
        if !frag::writable(value) {
            return Err(plain("a NaN cannot be written as data"));
        }
        let tid =
            self.table_at(table).ok_or_else(|| plain(format!("no table at {}", crate::path::show(table))))?;
        let eff = self.ev().effective(tid).map_err(|e| Self::err_in(&self.text, e))?;
        let (anchor, body) = match slot {
            Slot::Key { key, after } => {
                if eff.key(key.as_bytes()).is_some() {
                    return Err(plain(format!("'{key}' is already there")));
                }
                let anchor = after.and_then(|a| eff.key(a.as_bytes())).map_or(Anchor::End, Anchor::After);
                (anchor, Body::Entry { key: Some(key), value: value.clone(), node: false })
            }
            Slot::Head(key) => {
                if eff.key(key.as_bytes()).is_some() {
                    return Err(plain(format!("'{key}' is already there")));
                }
                let anchor = if self.table(tid).is_some_and(|t| !t.entries.is_empty()) {
                    Anchor::Before(0)
                } else {
                    Anchor::End
                };
                (anchor, Body::Entry { key: Some(key), value: value.clone(), node: false })
            }
            Slot::Position(n) => {
                let anchor = match n {
                    0 => eff.nums.first().map_or(Anchor::End, |x| Anchor::Before(x.1)),
                    n if n <= eff.nums.len() => Anchor::After(eff.nums[n - 1].1),
                    _ => return Err(plain(format!("the list holds {} items", eff.nums.len()))),
                };
                (anchor, Body::Entry { key: None, value: value.clone(), node: false })
            }
        };
        let mut ed = TableEdits::default();
        ed.inserts.push((anchor, body));
        let mut out = Vec::new();
        self.table_splices(tid, &ed, &mut out)?;
        self.commit(out).map(|_| ())
    }

    /// Moves the `from`-th item of a list to position `to` (both from 0), its text and comments
    /// as they are.
    pub fn move_item(&mut self, list: &[Seg], from: usize, to: usize) -> Result<(), LuaDataError> {
        let tid =
            self.table_at(list).ok_or_else(|| plain(format!("no list at {}", crate::path::show(list))))?;
        let eff = self.ev().effective(tid).map_err(|e| Self::err_in(&self.text, e))?;
        let items: Vec<usize> = eff.nums.iter().map(|x| x.1).collect();
        let Some(&j) = items.get(from) else {
            return Err(plain(format!("the list holds {} items", items.len())));
        };
        if to >= items.len() {
            return Err(plain(format!("the list holds {} items", items.len())));
        }
        if from == to {
            return Ok(());
        }
        let t = self.table(tid).ok_or_else(|| plain("no table"))?;
        let e = t.entries.get(j).ok_or_else(|| plain("no item"))?;
        let rest: Vec<usize> = items.iter().copied().filter(|&x| x != j).collect();
        let anchor = if to == 0 { Anchor::Before(rest[0]) } else { Anchor::After(rest[to - 1]) };
        let comments = if self.is_multiline(t) && self.starts_line(e.start) {
            let bs = self.block_start(t, j);
            self.text.get(bs as usize..self.line_start(e.start) as usize).unwrap_or("").to_owned()
        } else {
            String::new()
        };
        let text = self.text.get(e.start as usize..e.val.end as usize).unwrap_or("").to_owned();
        let mut ed = TableEdits::default();
        ed.removed.push(j);
        ed.inserts.push((anchor, Body::Raw { comments, text }));
        let mut out = Vec::new();
        self.table_splices(tid, &ed, &mut out)?;
        self.commit(out).map(|_| ())
    }

    // -- text helpers --------------------------------------------------------------------------

    fn line_start(&self, off: u32) -> u32 {
        let i = self.tree.lines.partition_point(|&l| l <= off);
        self.tree.lines.get(i.saturating_sub(1)).copied().unwrap_or(0)
    }

    /// The offset of the line end at or after `off` (the `\r` of a `\r\n`, or the end).
    fn line_end(&self, off: u32) -> u32 {
        let b = self.text.as_bytes();
        let mut i = off as usize;
        while let Some(&c) = b.get(i) {
            if c == b'\n' {
                if i > off as usize && b.get(i - 1) == Some(&b'\r') {
                    return (i - 1) as u32;
                }
                return i as u32;
            }
            i += 1;
        }
        b.len() as u32
    }

    /// The offset after the line end at `le`.
    fn after_line(&self, le: u32) -> u32 {
        let b = self.text.as_bytes();
        let mut i = le as usize;
        if b.get(i) == Some(&b'\r') {
            i += 1;
        }
        if b.get(i) == Some(&b'\n') {
            i += 1;
        }
        i as u32
    }

    fn starts_line(&self, off: u32) -> bool {
        let ls = self.line_start(off);
        self.text.get(ls as usize..off as usize).is_some_and(|s| s.bytes().all(|c| c == b' ' || c == b'\t'))
    }

    fn indent_at(&self, off: u32) -> &str {
        line_indent_any(&self.text, &self.tree, off)
    }

    /// Whether only spaces or a line comment follow `off` on its line.
    fn rest_blank(&self, off: u32) -> bool {
        let le = self.line_end(off);
        let rest =
            self.text.get(off as usize..le as usize).unwrap_or("").trim_start_matches([' ', '\t', '\r']);
        rest.is_empty() || line_comment(rest)
    }

    /// Whether a line is blank (only spaces).
    fn blank_line(&self, ls: u32) -> bool {
        let le = self.line_end(ls);
        le < self.text.len() as u32
            && self.text.get(ls as usize..le as usize).is_some_and(|s| s.trim().is_empty())
    }

    /// Whether a table is laid out over several lines: a line end between two of its own tokens.
    fn is_multiline(&self, t: &TableN) -> bool {
        let gap = |a: u32, b: u32| self.text.get(a as usize..b as usize).is_some_and(|s| s.contains('\n'));
        let mut prev = t.open + 1;
        for e in &t.entries {
            if gap(prev, e.start) {
                return true;
            }
            prev = e.end_with_sep();
        }
        gap(prev, t.close)
    }

    /// Whether the entries of a table are separated by blank lines.
    fn blank_separated(&self, t: &TableN) -> bool {
        t.entries.windows(2).any(|w| {
            let gap = self.text.get(w[0].end_with_sep() as usize..w[1].start as usize).unwrap_or("");
            let mut lines = gap.split('\n');
            lines.next();
            let mut inner: Vec<&str> = lines.collect();
            inner.pop();
            inner.iter().any(|l| l.trim().is_empty())
        })
    }

    /// The first line of an entry's block: the comment lines directly above it.
    fn block_start(&self, t: &TableN, j: usize) -> u32 {
        let Some(e) = t.entries.get(j) else { return 0 };
        let floor = if j == 0 { t.open + 1 } else { t.entries[j - 1].end_with_sep() };
        let mut start = self.line_start(e.start);
        while start > 0 {
            let prev = self.line_start(start - 1);
            if prev < floor {
                break;
            }
            let line = self.text.get(prev as usize..start as usize).unwrap_or("").trim();
            if line_comment(line) {
                start = prev;
            } else {
                break;
            }
        }
        start
    }

    /// The indentation new entries of a table take: a sibling's that starts its line, else the
    /// table's line and one unit.
    fn entry_indent(&self, t: &TableN, prefer: Option<usize>) -> String {
        let starts = |e: &cst::EntryN| self.starts_line(e.start);
        if let Some(e) = prefer.and_then(|j| t.entries.get(j)).filter(|e| starts(e)) {
            return self.indent_at(e.start).to_owned();
        }
        if let Some(e) = t.entries.iter().find(|e| starts(e)) {
            return self.indent_at(e.start).to_owned();
        }
        format!("{}{}", self.indent_at(t.open), self.layout.unit)
    }

    /// The splice that writes `value` over the value of entry `j`.
    pub(crate) fn replace_splice(&self, tid: u32, j: usize, value: &Value) -> Splice {
        let Some(e) = self.table(tid).and_then(|t| t.entries.get(j)) else {
            return Splice { start: 0, end: 0, text: String::new() };
        };
        let text = if value.is_table() {
            let ind = self.indent_at(e.start).to_owned();
            let ls = self.line_start(e.val.start);
            let used = frag::width(self.text.get(ls as usize..e.val.start as usize).unwrap_or(""));
            frag::value(value, &ind, used, &self.layout)
        } else {
            let old = if e.val.table.is_none() {
                self.text.get(e.val.start as usize..e.val.end as usize)
            } else {
                None
            };
            frag::scalar(value, old.map(str::as_bytes))
        };
        Splice { start: e.val.start, end: e.val.end, text }
    }

    fn body_text(&self, body: &Body, ind: &str) -> (String, String) {
        match body {
            Body::Entry { key, value, node } => {
                (String::new(), frag::entry(key.as_deref(), value, ind, *node, &self.layout))
            }
            Body::Raw { comments, text } => (comments.clone(), text.clone()),
        }
    }

    /// The splices of one table's removals and inserts, together: separators stay right
    /// whatever ends up last.
    pub(crate) fn table_splices(
        &self,
        tid: u32,
        ed: &TableEdits,
        out: &mut Vec<Splice>,
    ) -> Result<(), LuaDataError> {
        let t = self.table(tid).ok_or_else(|| plain("no table"))?;
        let n = t.entries.len();
        let mut removed = vec![false; n];
        for &j in &ed.removed {
            if let Some(r) = removed.get_mut(j) {
                *r = true;
            }
        }
        let kept: Vec<usize> = (0..n).filter(|&j| !removed[j]).collect();
        let nl = self.layout.nl;
        if kept.is_empty() {
            if ed.removed.is_empty() && ed.inserts.is_empty() {
                return Ok(());
            }
            out.push(self.rewrite_table(t, ed));
            return Ok(());
        }
        let multiline = self.is_multiline(t);
        let trailing = t.entries.last().is_some_and(|e| e.sep.is_some());
        let sep_char = t
            .entries
            .iter()
            .find_map(|e| e.sep)
            .and_then(|s| self.text.get(s as usize..s as usize + 1))
            .unwrap_or(",")
            .to_owned();
        let blank = multiline && self.blank_separated(t);
        for &j in &ed.removed {
            if j < n {
                out.push(self.removal(t, j, &removed, multiline));
            }
        }
        // the final order of the table: kept entries with the inserts around them
        #[derive(Clone, Copy)]
        enum It {
            Kept(usize),
            Ins(usize),
        }
        let last_kept = *kept.last().unwrap_or(&0);
        let anchor_of = |a: Anchor| match a {
            Anchor::End => Anchor::After(last_kept),
            Anchor::After(j) | Anchor::Before(j) if removed.get(j).copied().unwrap_or(true) => {
                Anchor::After(last_kept)
            }
            a => a,
        };
        let mut seq: Vec<It> = Vec::with_capacity(kept.len() + ed.inserts.len());
        for &j in &kept {
            for (k, (a, _)) in ed.inserts.iter().enumerate() {
                if anchor_of(*a) == Anchor::Before(j) {
                    seq.push(It::Ins(k));
                }
            }
            seq.push(It::Kept(j));
            for (k, (a, _)) in ed.inserts.iter().enumerate() {
                if anchor_of(*a) == Anchor::After(j) {
                    seq.push(It::Ins(k));
                }
            }
        }
        let count = seq.len();
        for (pos, it) in seq.iter().enumerate() {
            let last = pos + 1 == count;
            match *it {
                It::Kept(j) => {
                    let e = &t.entries[j];
                    if !last && e.sep.is_none() {
                        out.push(Splice { start: e.val.end, end: e.val.end, text: sep_char.clone() });
                    }
                    if last
                        && !trailing
                        && j + 1 != n
                        && let Some(s) = e.sep
                    {
                        out.push(Splice { start: s, end: s + 1, text: String::new() });
                    }
                }
                It::Ins(k) => {
                    let (a, body) = &ed.inserts[k];
                    let sep = if !last || trailing { sep_char.as_str() } else { "" };
                    let a = anchor_of(*a);
                    let (Anchor::After(j) | Anchor::Before(j)) = a else { continue };
                    let e = &t.entries[j];
                    let ind = self.entry_indent(t, Some(j));
                    let (comments, text) = self.body_text(body, &ind);
                    let s = match a {
                        Anchor::After(_) => {
                            let inline_at = e.sep.map_or(e.val.end, |s| s + 1);
                            if multiline && self.rest_blank(e.end_with_sep()) {
                                let le = self.line_end(e.end_with_sep());
                                let at = self.after_line(le);
                                let lead = if at == le { nl } else { "" };
                                let gap = if blank { nl } else { "" };
                                let ci = if comments.is_empty() { String::new() } else { comments };
                                let pre = if ci.is_empty() { ind.clone() } else { format!("{ci}{ind}") };
                                Splice {
                                    start: at,
                                    end: at,
                                    text: format!("{lead}{gap}{pre}{text}{sep}{nl}"),
                                }
                            } else if multiline {
                                Splice {
                                    start: inline_at,
                                    end: inline_at,
                                    text: format!("{nl}{ind}{text}{sep}"),
                                }
                            } else {
                                Splice { start: inline_at, end: inline_at, text: format!(" {text}{sep}") }
                            }
                        }
                        _ => {
                            if multiline && self.starts_line(e.start) {
                                let at = self.block_start(t, j);
                                let gap = if blank { nl } else { "" };
                                let pre = if comments.is_empty() {
                                    ind.clone()
                                } else {
                                    format!("{comments}{ind}")
                                };
                                let sep = if sep.is_empty() { sep_char.as_str() } else { sep };
                                Splice { start: at, end: at, text: format!("{pre}{text}{sep}{nl}{gap}") }
                            } else {
                                let sep = if sep.is_empty() { sep_char.as_str() } else { sep };
                                Splice { start: e.start, end: e.start, text: format!("{text}{sep} ") }
                            }
                        }
                    };
                    out.push(s);
                }
            }
        }
        Ok(())
    }

    /// A table none of whose entries stay: written anew from the inserts.
    fn rewrite_table(&self, t: &TableN, ed: &TableEdits) -> Splice {
        let nl = self.layout.nl;
        if ed.inserts.is_empty() {
            return Splice { start: t.open + 1, end: t.close, text: String::new() };
        }
        let outer = self.indent_at(t.open).to_owned();
        let inner = format!("{outer}{}", self.layout.unit);
        let texts: Vec<(String, String)> =
            ed.inserts.iter().map(|(_, b)| self.body_text(b, &inner)).collect();
        let was_multi = self.text.get(t.open as usize..t.close as usize).is_some_and(|s| s.contains('\n'));
        let ls = self.line_start(t.open);
        let used = frag::width(self.text.get(ls as usize..t.open as usize).unwrap_or(""));
        let one = format!("{{ {} }}", texts.iter().map(|(_, x)| x.as_str()).collect::<Vec<_>>().join(", "));
        let text = if !was_multi
            && texts.iter().all(|(c, x)| c.is_empty() && !x.contains('\n'))
            && used + one.chars().count() <= crate::write::WIDTH
        {
            one
        } else {
            let mut s = String::from("{");
            s.push_str(nl);
            for (c, x) in &texts {
                s.push_str(c);
                s.push_str(&inner);
                s.push_str(x);
                s.push(',');
                s.push_str(nl);
            }
            s.push_str(&outer);
            s.push('}');
            s
        };
        Splice { start: t.open, end: t.close + 1, text }
    }

    /// The range one removed entry takes with it.
    fn removal(&self, t: &TableN, j: usize, removed: &[bool], multiline: bool) -> Splice {
        let e = &t.entries[j];
        let end = e.end_with_sep();
        if multiline && self.starts_line(e.start) && self.rest_blank(end) {
            let start = self.block_start(t, j);
            let mut stop = self.after_line(self.line_end(end));
            if start > 0 {
                let before = self.line_start(start - 1);
                if self.blank_line(before) && (stop as usize) < self.text.len() && self.blank_line(stop) {
                    stop = self.after_line(self.line_end(stop));
                }
            }
            return Splice { start, end: stop, text: String::new() };
        }
        if e.sep.is_some() {
            let b = self.text.as_bytes();
            let mut stop = end as usize;
            while matches!(b.get(stop), Some(b' ' | b'\t')) {
                stop += 1;
            }
            return Splice { start: e.start, end: stop as u32, text: String::new() };
        }
        // the last entry, written without a separator: from the kept entry before it
        let prev = (0..j).rev().find(|&k| !removed[k]);
        let start = prev.map_or(e.start, |k| t.entries[k].end_with_sep());
        Splice { start, end: e.val.end, text: String::new() }
    }
}

#[cfg(test)]
mod tests;
