//! The engine's resolved view of a config tree (`CInifile`): what `system.ltx` and the hundreds
//! of files it includes mean once the engine has read them.
//!
//! # API
//!
//! - [`parse_file`] (`text`, [`Dialect`]) -> [`ParsedFile`]: one file, on its own, so a caller
//!   can read and parse hundreds of files in parallel. [`ParsedFile::includes`] names what it
//!   pulls in; [`include_path`] resolves a name the way the engine does (the including file's
//!   folder + the name, concatenated, nothing normalised).
//! - [`Ltx::assemble`] (`root`, a [`Loader`], [`Dialect`]) -> [`Loaded`]: the same tree as the
//!   sequential C++ load. [`Builder`] does the same in steps for the game's composition:
//!   [`Builder::load`], [`Builder::overlay`] (a module's whole-file overlay), [`Builder::patch`]
//!   (a `.ltxp` directive file), [`Builder::finish`].
//! - [`Ltx`]: immutable, `Send + Sync`, O(1) section lookup, `Arc`-shared strings.
//!   `section_exist`, `line_exist`, `line_count`, `section_count`, `r_line` (the pair at an
//!   index, in the engine's item order), `r_string` (quotes stripped, `r_string_wb`),
//!   `r_string_raw` (as stored, `None` for an empty value), `r_bool`, `r_u8` / `r_u16` /
//!   `r_u32` / `r_s32` / `r_u64` / `r_s64`, `r_float`, `r_fvector2/3/4`, `r_ivector2/3/4`,
//!   `r_fcolor`, `r_color`, `r_clsid`, `r_token`; `sections` (by name, the engine's order),
//!   `sections_in_file_order`, `section`.
//!
//! What the C++ treats as fatal - a missing section or key, a duplicate section, an unknown
//! parent, a missing include, a section line without `]` - is an [`LtxError`] naming the
//! section, the key, the file and the line.
//!
//! # Semantics (both dialects)
//!
//! - A line is what `IReader::r_string` returns: runs of `\r` / `\n` end it, so blank lines
//!   vanish; it is trimmed of everything up to the space.
//! - A comment starts at the first `;`, or at the first `/` when the next character is `/` and
//!   no `;` comes before it; it does not start inside the first pair of quotes.
//! - `#include "name"` (anything starting with `#` and holding `#include`) loads
//!   [`include_path`]`(this file, name)` at that point.
//! - `[name]:parent1, parent2` copies the parents' lines in that order, each overriding the
//!   one before; the section's own lines override them. Section names are lower-cased (ASCII),
//!   keys are not. A duplicate key overrides in place.
//! - A value loses every blank outside quotes; quotes stay (`r_string` strips one pair). A
//!   value whose quotes do not close continues on the next lines, joined by `\r\n`.
//! - Items are kept sorted by key (byte order), which is the order `r_line` walks; sections are
//!   sorted by name.
//!
//! # The editor's dialect ([`Dialect::Editor`], `Source/XrCore/Xr_ini.cpp`)
//!
//! A missing include is skipped with a warning (the editor cannot see the game's archives).
//! A multi-line value that outgrows the engine's 4096-byte buffer is an error.
//!
//! # The game's dialect ([`Dialect::Game`], Dead Air Refined's `xrCore/Xr_ini.cpp` + XMS)
//!
//! - `#include_optional "name"`: an include that may be absent. A plain missing include is an
//!   error.
//! - `#include "dir\*.ltx"`: every file the [`Loader`] lists under the folder (sorted by name);
//!   the folder is the including file's folder + the name's folder, and - as in the game - the
//!   name's folder once more (`configs\x\*.ltx` lists `configs\x\x\`).
//! - A multi-line value that meets a line holding `[...]`, or outgrows the buffer, is cut back
//!   to its first line (a warning) and reading goes on with the next line.
//! - Layers ([`Loader::layer`]): with merging on ([`Builder::set_merge`]), a section that
//!   exists in another layer is merged instead of refused - new keys are appended, a key from a
//!   layer at least as high wins - and the merged keys follow the sorted ones.
//! - `.ltxp` directive patches ([`Builder::patch`], `XMS/xms_ltx.cpp`): `[section]` (created,
//!   `: parents` inherited, when new), `[!section]` deletes it, `key = value` sets (the value
//!   kept as written), `+key = a, b` appends missing list items, `-key = a` removes them,
//!   `!key` deletes the key; `;` starts a comment. New keys go to the end of the section.

use std::sync::Arc;

use rustc_hash::FxHashMap;

/// Which reader's rules apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// The C++ editor's `CInifile`.
    Editor,
    /// The game's `CInifile` and its module composition.
    Game,
}

/// The engine's line buffer (`string4096`).
const BUF: usize = 4096;
const MAX_INCLUDE_DEPTH: usize = 64;

/// Why a config tree cannot be read; each of these stops the engine.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LtxError {
    #[error("Can't find section '{section}'.")]
    NoSection { section: String },
    #[error("Can't find variable '{key}' in section [{section}] ({file}, line {line}).")]
    NoKey { section: String, key: String, file: String, line: u32 },
    #[error(
        "Duplicate section '{section}' found ({file}, line {line}; first in {first_file}, line {first_line})."
    )]
    DuplicateSection { section: String, file: String, line: u32, first_file: String, first_line: u32 },
    #[error("Section [{section}] inherits from '{parent}', which does not exist ({file}, line {line}).")]
    UnknownParent { section: String, parent: String, file: String, line: u32 },
    #[error("Can't find include file '{name}' ({file}, line {line}).")]
    MissingInclude { name: String, file: String, line: u32 },
    #[error("The include of '{name}' loops back on itself ({file}, line {line}).")]
    IncludeCycle { name: String, file: String, line: u32 },
    #[error("Bad ini section found: {text} ({file}, line {line}).")]
    BadSection { text: String, file: String, line: u32 },
    #[error("A line is longer than the engine's 4096-byte buffer ({file}, line {line}).")]
    LineTooLong { file: String, line: u32 },
    #[error(
        "Incorrect inifile format: section [{section}], variable [{key}]. Odd number of quotes (\") found, but should be even ({file}, line {line})."
    )]
    OddQuotes { section: String, key: String, file: String, line: u32 },
    #[error("Can't find the config file '{file}'.")]
    MissingRoot { file: String },
}

/// What the engine reads past with a message in its log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LtxWarning {
    /// The editor skips an include it cannot find.
    IncludeSkipped { name: String, file: String, line: u32 },
    /// The game skips a missing `#include_optional`.
    OptionalSkipped { name: String, file: String, line: u32 },
    /// The game cut a value with an odd number of quotes back to its first line.
    QuotesTrimmed { section: String, key: String, file: String, line: u32 },
    /// A `.ltxp` section inherits from one that does not exist; its lines are skipped.
    PatchParentMissing { section: String, parent: String, line: u32 },
}

// -- one file ----------------------------------------------------------------------------------

/// An include a file names.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Include {
    /// The name between the quotes, as written.
    pub name: Arc<str>,
    /// `#include_optional` (game dialect).
    pub optional: bool,
    /// A `*.ltx` mask (game dialect).
    pub wildcard: bool,
    pub line: u32,
}

#[derive(Clone, Debug)]
enum Ev {
    Include(usize),
    Section { name: Arc<str>, parents: Vec<Arc<str>>, line: u32 },
    Item { key: Arc<str>, value: Option<Arc<str>>, line: u32 },
    BadSection { text: String, line: u32 },
    LineTooLong { line: u32 },
    OddQuotes { key: String, line: u32 },
    QuotesTrimmed { key: String, line: u32 },
}

/// One file, parsed on its own. Its errors are raised when [`Ltx::assemble`] reaches them, as
/// the engine raises them.
#[derive(Clone, Debug, Default)]
pub struct ParsedFile {
    events: Vec<Ev>,
    includes: Vec<Include>,
}

impl ParsedFile {
    /// The includes, in file order.
    #[must_use]
    pub fn includes(&self) -> &[Include] {
        &self.includes
    }
}

/// `include_path(includer, name)`: the path the engine opens for `#include "name"` in the file
/// at `includer` - the includer's folder (up to its last `\` or `/`) followed by `name`.
#[must_use]
pub fn include_path(includer: &str, name: &str) -> String {
    let mut s = folder_of(includer).to_owned();
    s.push_str(name);
    s
}

/// The folder part of a path, with its trailing separator (`_splitpath` drive + dir).
fn folder_of(path: &str) -> &str {
    path.rfind(['\\', '/']).map_or("", |i| path.get(..=i).unwrap_or(""))
}

fn is_term(c: u8) -> bool {
    c == b'\r' || c == b'\n'
}

/// `IReader::advance_term_string`: the next line and where reading goes on.
fn next_line(data: &[u8], mut pos: usize) -> (usize, usize, usize) {
    let start = pos;
    let mut sz = 0;
    while pos < data.len() {
        pos += 1;
        sz += 1;
        if data.get(pos).copied().is_some_and(is_term) {
            while data.get(pos).copied().is_some_and(is_term) {
                pos += 1;
            }
            break;
        }
    }
    (start, start + sz, pos)
}

/// `_Trim`: everything up to the space off both ends.
fn trim(s: &str) -> &str {
    s.trim_matches(|c: char| u32::from(c) <= 0x20)
}

/// `_parse`: blanks outside quotes removed. True when a quote is left open.
fn parse_value(src: &str) -> (String, bool) {
    let mut out = String::with_capacity(src.len());
    let mut inside = false;
    for c in src.chars() {
        if c.is_ascii_whitespace() || c == '\x0b' {
            if inside {
                out.push(c);
            }
            continue;
        }
        if c == '"' {
            inside = !inside;
        }
        out.push(c);
    }
    (out, inside)
}

/// The comment-free part of a trimmed line.
fn strip_comment(s: &str) -> &str {
    let b = s.as_bytes();
    let mut comm = b.iter().position(|&c| c == b';');
    if let Some(c1) = b.iter().position(|&c| c == b'/')
        && b.get(c1 + 1) == Some(&b'/')
        && comm.is_none_or(|c| c1 < c)
    {
        comm = Some(c1);
    }
    let Some(comm) = comm else { return s };
    if let Some(q1) = b.iter().position(|&c| c == b'"').filter(|&q| q < comm)
        && let Some(q2) = b.get(q1 + 1..).and_then(|r| r.iter().position(|&c| c == b'"')).map(|p| p + q1 + 1)
        && q2 > comm
    {
        return s;
    }
    s.get(..comm).unwrap_or(s)
}

/// `_GetItemCount` + `_GetItem` over a comma list.
fn get_items(src: &str) -> Vec<&str> {
    let b = src.as_bytes();
    if b.is_empty() {
        return Vec::new();
    }
    let mut cnt = 0;
    let mut res = 0;
    let mut last_res = 0;
    while let Some(p) = b.get(res..).and_then(|r| r.iter().position(|&c| c == b',')) {
        res += p + 1;
        last_res = res;
        cnt += 1;
        if b.get(res) == Some(&b',') {
            break;
        }
    }
    if last_res < b.len() {
        cnt += 1;
    }
    let parts: Vec<&str> = src.split(',').collect();
    (0..cnt).map(|k| trim(parts.get(k).copied().unwrap_or(""))).collect()
}

/// Line numbers of byte offsets, counted as an editor shows them.
struct Lines<'a> {
    data: &'a [u8],
    at: usize,
    line: u32,
}

impl Lines<'_> {
    fn line_of(&mut self, pos: usize) -> u32 {
        if pos < self.at {
            self.at = 0;
            self.line = 1;
        }
        let end = pos.min(self.data.len());
        self.line += u32::try_from(
            self.data.get(self.at..end).map_or(0, |r| r.iter().filter(|&&c| c == b'\n').count()),
        )
        .unwrap_or(0);
        self.at = end;
        self.line
    }
}

fn arc(s: &str) -> Arc<str> {
    Arc::from(s)
}

/// Parses one file the way `CInifile::Load` reads it.
#[must_use]
pub fn parse_file(text: &str, dialect: Dialect) -> ParsedFile {
    let data = text.as_bytes();
    let mut lines = Lines { data, at: 0, line: 1 };
    let mut out = ParsedFile::default();
    let mut pos = 0;
    let line_text = |s: usize, e: usize| -> &str {
        let raw = text.get(s..e).unwrap_or("");
        raw.split('\0').next().unwrap_or("")
    };
    let too_long = |s: &str| s.chars().count() >= BUF - 1;
    let mut in_section = false;
    while pos < data.len() {
        let (s, e, next) = next_line(data, pos);
        let line = lines.line_of(s);
        pos = next;
        let raw = line_text(s, e);
        if too_long(raw) {
            out.events.push(Ev::LineTooLong { line });
            return out;
        }
        // the line is trimmed before its comment is cut, and not after
        let str = strip_comment(trim(raw));
        if str.starts_with('#') && str.contains("#include") {
            let name = str.split('"').nth(1).map_or("", trim);
            let optional = dialect == Dialect::Game && str.contains("#include_optional");
            let wildcard = dialect == Dialect::Game && name.contains("*.ltx");
            out.includes.push(Include { name: arc(name), optional, wildcard, line });
            out.events.push(Ev::Include(out.includes.len() - 1));
            continue;
        }
        if str.starts_with('[') {
            let Some(close) = str.find(']') else {
                out.events.push(Ev::BadSection { text: str.to_owned(), line });
                return out;
            };
            let parents =
                str.find("]:").map(|i| get_items(str.get(i + 2..).unwrap_or(""))).unwrap_or_default();
            let name = str.get(1..close).unwrap_or("").to_ascii_lowercase();
            out.events.push(Ev::Section {
                name: arc(&name),
                parents: parents.into_iter().map(arc).collect(),
                line,
            });
            in_section = true;
            continue;
        }
        if !in_section {
            continue;
        }
        let (key, value) = match str.find('=') {
            Some(eq) => {
                let key = trim(str.get(..eq).unwrap_or(""));
                let mut value_raw = str.get(eq + 1..).unwrap_or("").to_owned();
                let (mut parsed, mut inside) = parse_value(&value_raw);
                if inside {
                    let first = parsed.clone();
                    let resume = pos;
                    while inside {
                        value_raw.push_str("\r\n");
                        let (cs, ce, cnext) = next_line(data, pos);
                        let add = line_text(cs, ce);
                        let section_like =
                            add.find('[').is_some_and(|b| add.get(b..).is_some_and(|r| r.contains(']')));
                        let overflow = value_raw.chars().count() + add.chars().count() >= BUF;
                        // past the end the engine reads empty lines until the buffer overflows
                        let eof = cs >= data.len();
                        if dialect == Dialect::Game && (overflow || section_like || eof) {
                            parsed = first.trim_matches(|c: char| u32::from(c) <= u32::from(b'"')).to_owned();
                            pos = resume;
                            out.events.push(Ev::QuotesTrimmed { key: key.to_owned(), line });
                            break;
                        }
                        if overflow || eof {
                            out.events.push(Ev::OddQuotes { key: key.to_owned(), line });
                            return out;
                        }
                        pos = cnext;
                        value_raw.push_str(add);
                        (parsed, inside) = parse_value(&value_raw);
                        if inside && pos >= 4 && data.get(pos - 4..pos) == Some(b"\r\n\r\n") {
                            value_raw.push_str("\r\n");
                        }
                    }
                }
                (key, parsed)
            }
            None => (trim(str), String::new()),
        };
        if key.is_empty() {
            continue;
        }
        let value = (!value.is_empty()).then(|| arc(&value));
        out.events.push(Ev::Item { key: arc(key), value, line });
    }
    out
}

// -- the tree ----------------------------------------------------------------------------------

/// A line of a section.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Item {
    pub key: Arc<str>,
    /// `None` for `key =` or a bare `key`.
    pub value: Option<Arc<str>>,
    pub file: u32,
    pub line: u32,
}

/// A section with its inherited lines resolved.
#[derive(Clone, Debug)]
pub struct Section {
    pub name: Arc<str>,
    /// In the engine's order: sorted by key, merged and patched keys after them.
    pub items: Vec<Item>,
    index: FxHashMap<Arc<str>, u32>,
    pub file: u32,
    pub line: u32,
    layer: u16,
    item_layers: Option<Vec<u16>>,
}

impl Section {
    fn new(name: Arc<str>, file: u32, line: u32, layer: u16) -> Self {
        Self { name, items: Vec::new(), index: FxHashMap::default(), file, line, layer, item_layers: None }
    }

    /// The line of a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Item> {
        self.index.get(key).and_then(|&i| self.items.get(i as usize))
    }

    /// `insert_item`: overrides a key in place or appends it.
    fn insert(&mut self, it: Item) {
        if let Some(&i) = self.index.get(&*it.key) {
            if let Some(x) = self.items.get_mut(i as usize) {
                x.value = it.value;
                x.file = it.file;
                x.line = it.line;
            }
            let layer = self.layer;
            if let Some(l) = self.item_layers.as_mut().and_then(|v| v.get_mut(i as usize)) {
                *l = layer;
            }
            return;
        }
        self.index.insert(it.key.clone(), u32::try_from(self.items.len()).unwrap_or(u32::MAX));
        self.items.push(it);
        let layer = self.layer;
        if let Some(v) = self.item_layers.as_mut() {
            v.push(layer);
        }
    }

    fn rebuild_index(&mut self) {
        self.index.clear();
        for (i, it) in self.items.iter().enumerate() {
            self.index.insert(it.key.clone(), u32::try_from(i).unwrap_or(u32::MAX));
        }
    }

    fn sort(&mut self) {
        self.items.sort_by(|a, b| a.key.as_bytes().cmp(b.key.as_bytes()));
        self.rebuild_index();
    }
}

/// A loaded config tree: immutable, shared between threads.
#[derive(Clone, Debug, Default)]
pub struct Ltx {
    sections: Arc<[Section]>,
    by_name: Arc<FxHashMap<Arc<str>, u32>>,
    file_order: Arc<[u32]>,
    files: Arc<[Arc<str>]>,
}

/// A tree and what the engine would have logged while reading it.
#[derive(Clone, Debug)]
pub struct Loaded {
    pub ltx: Ltx,
    pub warnings: Vec<LtxWarning>,
}

/// Where [`Ltx::assemble`] gets its files.
pub trait Loader: Sync {
    /// The parsed file at a path [`include_path`] produced (or the root as given), `None`
    /// when it does not exist.
    fn file(&self, path: &str) -> Option<&ParsedFile>;
    /// The names of the files below `dir` (as the game's `FS.file_list` without
    /// `FS_RootOnly` lists them) that match `mask` (`*` any run of characters). Game dialect only.
    fn list(&self, dir: &str, mask: &str) -> Vec<String> {
        let _ = (dir, mask);
        Vec::new()
    }
    /// The XMS layer of a file: what decides which of two modules wins a key (game dialect).
    fn layer(&self, path: &str) -> u16 {
        let _ = path;
        0
    }
}

/// The composition of a tree, step by step, the way the game builds `pSettings`.
pub struct Builder {
    dialect: Dialect,
    merge: bool,
    sections: Vec<Section>,
    by_name: FxHashMap<Arc<str>, usize>,
    created: Vec<usize>,
    removed: Vec<bool>,
    files: Vec<Arc<str>>,
    file_ids: FxHashMap<Arc<str>, u32>,
    warnings: Vec<LtxWarning>,
    stack: Vec<String>,
}

impl Builder {
    #[must_use]
    pub fn new(dialect: Dialect) -> Self {
        Self {
            dialect,
            merge: false,
            sections: Vec::new(),
            by_name: FxHashMap::default(),
            created: Vec::new(),
            removed: Vec::new(),
            files: Vec::new(),
            file_ids: FxHashMap::default(),
            warnings: Vec::new(),
            stack: Vec::new(),
        }
    }

    /// Cross-layer merging of duplicate sections (`XMS::SetLtxMergeEnabled`; game dialect).
    pub fn set_merge(&mut self, on: bool) {
        self.merge = on && self.dialect == Dialect::Game;
    }

    fn file_id(&mut self, path: &str) -> u32 {
        if let Some(&id) = self.file_ids.get(path) {
            return id;
        }
        let id = u32::try_from(self.files.len()).unwrap_or(u32::MAX);
        let a = arc(path);
        self.files.push(a.clone());
        self.file_ids.insert(a, id);
        id
    }

    fn file_name(&self, id: u32) -> String {
        self.files.get(id as usize).map_or_else(String::new, |s| s.to_string())
    }

    fn find(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// `r_section` while loading: the parent named, lower-cased.
    fn section_lc(&self, name: &str) -> Option<usize> {
        self.find(&name.to_ascii_lowercase())
    }

    /// Loads a file and everything it includes (`CInifile::Load` on a fresh reader).
    pub fn load(&mut self, path: &str, loader: &dyn Loader) -> Result<(), LtxError> {
        let Some(file) = loader.file(path) else {
            return Err(LtxError::MissingRoot { file: path.to_owned() });
        };
        self.load_file(path, file, folder_of(path).to_owned(), loader)
    }

    /// A whole-file overlay on what is loaded (`xms_load_overlay`); a missing file is no error.
    pub fn overlay(&mut self, path: &str, loader: &dyn Loader) -> Result<(), LtxError> {
        let Some(file) = loader.file(path) else { return Ok(()) };
        self.load_file(path, file, folder_of(path).to_owned(), loader)
    }

    fn insert_section(&mut self, mut s: Section) -> Result<(), LtxError> {
        s.sort();
        if let Some(i) = self.find(&s.name) {
            let (layer, file, line) = self.sections.get(i).map_or((0, 0, 0), |e| (e.layer, e.file, e.line));
            if !self.merge || layer == s.layer {
                return Err(LtxError::DuplicateSection {
                    section: s.name.to_string(),
                    file: self.file_name(s.file),
                    line: s.line,
                    first_file: self.file_name(file),
                    first_line: line,
                });
            }
            if let Some(dst) = self.sections.get_mut(i) {
                merge_section(dst, s);
            }
            return Ok(());
        }
        let i = self.sections.len();
        self.by_name.insert(s.name.clone(), i);
        self.sections.push(s);
        self.created.push(i);
        self.removed.push(false);
        Ok(())
    }

    fn load_file(
        &mut self,
        path: &str,
        file: &ParsedFile,
        dir: String,
        loader: &dyn Loader,
    ) -> Result<(), LtxError> {
        if self.stack.len() >= MAX_INCLUDE_DEPTH || self.stack.iter().any(|p| p.eq_ignore_ascii_case(path)) {
            let (file, line) = self.stack.last().map_or((String::new(), 0), |p| (p.clone(), 0));
            return Err(LtxError::IncludeCycle { name: path.to_owned(), file, line });
        }
        self.stack.push(path.to_owned());
        let r = self.load_events(path, file, &dir, loader);
        self.stack.pop();
        r
    }

    fn load_events(
        &mut self,
        path: &str,
        file: &ParsedFile,
        dir: &str,
        loader: &dyn Loader,
    ) -> Result<(), LtxError> {
        let fid = self.file_id(path);
        let layer = loader.layer(path);
        let fname = path.to_owned();
        let mut current: Option<Section> = None;
        for ev in &file.events {
            match ev {
                Ev::Include(i) => {
                    let Some(inc) = file.includes.get(*i) else { continue };
                    self.include(inc, &fname, dir, loader)?;
                }
                Ev::Section { name, parents, line } => {
                    if let Some(s) = current.take() {
                        self.insert_section(s)?;
                    }
                    let mut s = Section::new(name.clone(), fid, *line, layer);
                    for p in parents {
                        let Some(pi) = self.section_lc(p) else {
                            return Err(LtxError::UnknownParent {
                                section: name.to_string(),
                                parent: p.to_string(),
                                file: fname,
                                line: *line,
                            });
                        };
                        if let Some(ps) = self.sections.get(pi) {
                            for it in &ps.items {
                                s.insert(it.clone());
                            }
                        }
                    }
                    current = Some(s);
                }
                Ev::Item { key, value, line } => {
                    if let Some(s) = current.as_mut() {
                        s.insert(Item { key: key.clone(), value: value.clone(), file: fid, line: *line });
                    }
                }
                Ev::BadSection { text, line } => {
                    return Err(LtxError::BadSection { text: text.clone(), file: fname, line: *line });
                }
                Ev::LineTooLong { line } => return Err(LtxError::LineTooLong { file: fname, line: *line }),
                Ev::OddQuotes { key, line } => {
                    let section = current.as_ref().map_or_else(String::new, |s| s.name.to_string());
                    return Err(LtxError::OddQuotes { section, key: key.clone(), file: fname, line: *line });
                }
                Ev::QuotesTrimmed { key, line } => {
                    let section = current.as_ref().map_or_else(String::new, |s| s.name.to_string());
                    self.warnings.push(LtxWarning::QuotesTrimmed {
                        section,
                        key: key.clone(),
                        file: fname.clone(),
                        line: *line,
                    });
                }
            }
        }
        if let Some(s) = current.take() {
            self.insert_section(s)?;
        }
        Ok(())
    }

    fn include(
        &mut self,
        inc: &Include,
        fname: &str,
        dir: &str,
        loader: &dyn Loader,
    ) -> Result<(), LtxError> {
        let path = format!("{dir}{}", inc.name);
        let inc_dir = folder_of(&path).to_owned();
        if inc.wildcard {
            // oracle: the game adds the mask's folder to a folder that already holds it
            let (mut list_dir, mut mask) = (inc_dir.clone(), inc.name.to_string());
            if let Some(sep) = mask.rfind('\\') {
                list_dir.push_str(mask.get(..sep).unwrap_or(""));
                list_dir.push('\\');
                mask = mask.get(sep + 1..).unwrap_or("").to_owned();
            }
            let mut names = loader.list(&list_dir, &mask);
            names.sort();
            names.dedup();
            for n in names {
                let p = format!("{list_dir}{n}");
                self.include_one(&p, &n, inc, fname, &inc_dir, loader)?;
            }
            return Ok(());
        }
        self.include_one(&path, &inc.name, inc, fname, &inc_dir, loader)
    }

    fn include_one(
        &mut self,
        path: &str,
        name: &str,
        inc: &Include,
        fname: &str,
        inc_dir: &str,
        loader: &dyn Loader,
    ) -> Result<(), LtxError> {
        match loader.file(path) {
            Some(f) => self.load_file(path, f, inc_dir.to_owned(), loader),
            None if self.dialect == Dialect::Editor => {
                self.warnings.push(LtxWarning::IncludeSkipped {
                    name: name.to_owned(),
                    file: fname.to_owned(),
                    line: inc.line,
                });
                Ok(())
            }
            None if inc.optional => {
                self.warnings.push(LtxWarning::OptionalSkipped {
                    name: name.to_owned(),
                    file: fname.to_owned(),
                    line: inc.line,
                });
                Ok(())
            }
            None => Err(LtxError::MissingInclude {
                name: name.to_owned(),
                file: fname.to_owned(),
                line: inc.line,
            }),
        }
    }

    // -- .ltxp ------------------------------------------------------------------------------

    fn patch_create(&mut self, name: &str, parents: Option<&str>, layer: u16, line: u32) -> bool {
        if self.find(name).is_some() {
            return true;
        }
        let mut s = Section::new(arc(name), 0, line, layer);
        if let Some(parents) = parents.filter(|p| !p.is_empty()) {
            for p in get_items(parents) {
                let Some(pi) = self.find(p) else {
                    self.warnings.push(LtxWarning::PatchParentMissing {
                        section: name.to_owned(),
                        parent: p.to_owned(),
                        line,
                    });
                    return false;
                };
                if let Some(ps) = self.sections.get(pi) {
                    for it in &ps.items {
                        s.insert(it.clone());
                    }
                }
            }
        }
        let i = self.sections.len();
        self.by_name.insert(s.name.clone(), i);
        self.sections.push(s);
        self.created.push(i);
        self.removed.push(false);
        true
    }

    fn patch_set(&mut self, section: &str, key: &str, value: &str, layer: u16, fid: u32, line: u32) {
        let Some(i) = self.find(section) else { return };
        let Some(s) = self.sections.get_mut(i) else { return };
        let value = (!value.is_empty()).then(|| arc(value));
        if let Some(&k) = s.index.get(key) {
            if let Some(x) = s.items.get_mut(k as usize) {
                x.value = value;
                x.file = fid;
                x.line = line;
            }
            if let Some(l) = s.item_layers.as_mut().and_then(|v| v.get_mut(k as usize)) {
                *l = layer;
            }
            return;
        }
        s.index.insert(arc(key), u32::try_from(s.items.len()).unwrap_or(u32::MAX));
        s.items.push(Item { key: arc(key), value, file: fid, line });
        if let Some(v) = s.item_layers.as_mut() {
            v.push(layer);
        }
    }

    fn list_items(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|x| x.trim_matches([' ', '\t', '\r']))
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Applies a `.ltxp` directive file (`apply_patch_file`). `path` names it in the tree;
    /// `layer` is its module's layer.
    pub fn patch(&mut self, path: &str, text: &str, layer: u16) {
        let fid = self.file_id(path);
        let tr = |s: &str| -> String {
            s.trim_start_matches([' ', '\t']).trim_end_matches([' ', '\t', '\r']).to_owned()
        };
        let mut section = String::new();
        let mut valid = false;
        for (n, raw) in text.split('\0').next().unwrap_or("").split('\n').enumerate() {
            let line = u32::try_from(n + 1).unwrap_or(u32::MAX);
            let raw = raw.split(';').next().unwrap_or("");
            let t = tr(raw);
            if t.is_empty() {
                continue;
            }
            if let Some(body) = t.strip_prefix('[') {
                let Some(close) = body.find(']') else { continue };
                let name = tr(body.get(..close).unwrap_or(""));
                let rest = tr(body.get(close + 1..).unwrap_or(""));
                let parents = rest.strip_prefix(':').map(tr);
                if let Some(del) = name.strip_prefix('!') {
                    let del = tr(del).to_ascii_lowercase();
                    if let Some(i) = self.by_name.remove(del.as_str())
                        && let Some(r) = self.removed.get_mut(i)
                    {
                        *r = true;
                    }
                    valid = false;
                    continue;
                }
                section = name.to_ascii_lowercase();
                valid = self.patch_create(&section, parents.as_deref(), layer, line);
                continue;
            }
            if !valid {
                continue;
            }
            if let Some(eq) = t.find('=') {
                let key = tr(t.get(..eq).unwrap_or(""));
                let value = tr(t.get(eq + 1..).unwrap_or(""));
                if let Some(k) = key.strip_prefix('+').map(tr) {
                    self.list_op(&section, &k, &value, true, layer, fid, line);
                } else if let Some(k) = key.strip_prefix('-').map(tr) {
                    self.list_op(&section, &k, &value, false, layer, fid, line);
                } else if !key.is_empty() {
                    self.patch_set(&section, &key, &value, layer, fid, line);
                }
            } else if let Some(k) = t.strip_prefix('!') {
                let k = tr(k);
                if let Some(s) = self.find(&section).and_then(|i| self.sections.get_mut(i))
                    && let Some(&at) = s.index.get(k.as_str())
                {
                    s.items.remove(at as usize);
                    if let Some(v) = s.item_layers.as_mut()
                        && (at as usize) < v.len()
                    {
                        v.remove(at as usize);
                    }
                    s.rebuild_index();
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn list_op(&mut self, section: &str, key: &str, value: &str, add: bool, layer: u16, fid: u32, line: u32) {
        let current = self
            .find(section)
            .and_then(|i| self.sections.get(i))
            .and_then(|s| s.get(key))
            .and_then(|it| it.value.clone());
        let mut items = Self::list_items(current.as_deref().unwrap_or(""));
        for x in Self::list_items(value) {
            if add {
                if !items.contains(&x) {
                    items.push(x);
                }
            } else {
                items.retain(|y| *y != x);
            }
        }
        self.patch_set(section, key, &items.join(","), layer, fid, line);
    }

    /// The finished tree and what was logged on the way.
    #[must_use]
    pub fn finish(self) -> Loaded {
        let Builder { sections, removed, created, files, warnings, .. } = self;
        let mut keep: Vec<(usize, Section)> = sections
            .into_iter()
            .enumerate()
            .filter(|(i, _)| !removed.get(*i).copied().unwrap_or(false))
            .collect();
        keep.sort_by(|a, b| a.1.name.as_bytes().cmp(b.1.name.as_bytes()));
        let mut pos_of = vec![u32::MAX; removed.len()];
        for (sorted, (orig, _)) in keep.iter().enumerate() {
            if let Some(p) = pos_of.get_mut(*orig) {
                *p = u32::try_from(sorted).unwrap_or(u32::MAX);
            }
        }
        let file_order: Vec<u32> =
            created.iter().filter_map(|&i| pos_of.get(i).copied()).filter(|&p| p != u32::MAX).collect();
        let sections: Vec<Section> = keep.into_iter().map(|(_, s)| s).collect();
        let mut by_name = FxHashMap::default();
        for (i, s) in sections.iter().enumerate() {
            by_name.insert(s.name.clone(), u32::try_from(i).unwrap_or(u32::MAX));
        }
        Loaded {
            ltx: Ltx {
                sections: sections.into(),
                by_name: Arc::new(by_name),
                file_order: file_order.into(),
                files: files.into(),
            },
            warnings,
        }
    }
}

/// `xms_merge_section`: `src` into `dst`, layer against layer.
fn merge_section(dst: &mut Section, src: Section) {
    let dst_layer = dst.layer;
    let n = dst.items.len();
    let layers = dst.item_layers.get_or_insert_with(|| vec![dst_layer; n]);
    for it in src.items {
        match dst.index.get(&*it.key) {
            None => {
                dst.index.insert(it.key.clone(), u32::try_from(dst.items.len()).unwrap_or(u32::MAX));
                dst.items.push(it);
                layers.push(src.layer);
            }
            Some(&i) => {
                let old = layers.get(i as usize).copied().unwrap_or(0);
                if src.layer >= old {
                    if let Some(x) = dst.items.get_mut(i as usize) {
                        x.value = it.value;
                        x.file = it.file;
                        x.line = it.line;
                    }
                    if let Some(l) = layers.get_mut(i as usize) {
                        *l = src.layer;
                    }
                }
            }
        }
    }
    dst.layer = dst.layer.max(src.layer);
}

// -- C conversions -----------------------------------------------------------------------------

fn c_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `strtol` base 10 over a prefix: the value (saturated) and the bytes it used (0: none).
fn c_long(s: &[u8]) -> (i64, usize) {
    let mut i = 0;
    while s.get(i).copied().is_some_and(c_space) {
        i += 1;
    }
    let mut neg = false;
    if let Some(&c @ (b'+' | b'-')) = s.get(i) {
        neg = c == b'-';
        i += 1;
    }
    let start = i;
    let mut v: i64 = 0;
    while let Some(&d) = s.get(i).filter(|d| d.is_ascii_digit()) {
        v = v.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    if i == start {
        return (0, 0);
    }
    (if neg { -v } else { v }, i)
}

/// `_strtoui64(s, NULL, 10)`: blanks, a sign, digits; `u64::MAX` when it overflows, and a minus
/// sign negates in two's complement, as the CRT does.
fn c_strtoui64(s: &[u8]) -> u64 {
    let mut i = 0;
    while s.get(i).copied().is_some_and(c_space) {
        i += 1;
    }
    let mut neg = false;
    if let Some(&c @ (b'+' | b'-')) = s.get(i) {
        neg = c == b'-';
        i += 1;
    }
    let mut v: u64 = 0;
    while let Some(&d) = s.get(i).filter(|d| d.is_ascii_digit()) {
        match v.checked_mul(10).and_then(|x| x.checked_add(u64::from(d - b'0'))) {
            Some(x) => v = x,
            None => return u64::MAX,
        }
        i += 1;
    }
    if neg { v.wrapping_neg() } else { v }
}

/// `atoi` as the MSVC runtime gives it: saturated at the bounds of `int`.
fn c_atoi(s: &str) -> i32 {
    let (v, _) = c_long(s.as_bytes());
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// `strtod` over a prefix: the value and the bytes it used (0: none).
fn c_strtod(s: &[u8]) -> (f64, usize) {
    let mut i = 0;
    while s.get(i).copied().is_some_and(c_space) {
        i += 1;
    }
    let start = i;
    if let Some(b'+' | b'-') = s.get(i) {
        i += 1;
    }
    let lower =
        |j: usize, w: &[u8]| -> bool { s.get(j..j + w.len()).is_some_and(|x| x.eq_ignore_ascii_case(w)) };
    let sign = if s.get(start) == Some(&b'-') { -1.0 } else { 1.0 };
    if lower(i, b"infinity") {
        return (sign * f64::INFINITY, i + 8);
    }
    if lower(i, b"inf") {
        return (sign * f64::INFINITY, i + 3);
    }
    if lower(i, b"nan") {
        return (f64::NAN, i + 3);
    }
    let digits_from = i;
    while s.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
    }
    let mut nd = i - digits_from;
    if s.get(i) == Some(&b'.') {
        i += 1;
        let f = i;
        while s.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
        }
        nd += i - f;
    }
    if nd == 0 {
        return (0.0, 0);
    }
    if let Some(b'e' | b'E') = s.get(i) {
        let mut j = i + 1;
        if let Some(b'+' | b'-') = s.get(j) {
            j += 1;
        }
        let e = j;
        while s.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > e {
            i = j;
        }
    }
    let text = std::str::from_utf8(s.get(start..i).unwrap_or(&[])).unwrap_or("0");
    let text = if text.ends_with('.') { text.trim_end_matches('.') } else { text };
    let text = if text == "-" || text == "+" || text.is_empty() { "0" } else { text };
    let v: f64 = text.parse().unwrap_or_else(|_| format!("{text}0").parse().unwrap_or(0.0));
    (v, i)
}

/// `sscanf(text, "%f,%f,...")`: the fields it fills, the rest left at their defaults.
fn scan_floats<const N: usize>(text: &str) -> [f32; N] {
    let mut out = [0.0f32; N];
    let b = text.as_bytes();
    let mut at = 0;
    for (k, slot) in out.iter_mut().enumerate() {
        if k > 0 {
            if b.get(at) != Some(&b',') {
                break;
            }
            at += 1;
        }
        let (v, used) = c_strtod(b.get(at..).unwrap_or(&[]));
        if used == 0 {
            break;
        }
        *slot = v as f32;
        at += used;
    }
    out
}

/// `sscanf(text, "%d,%d,...")` over `defaults`.
fn scan_ints<const N: usize>(text: &str, defaults: [i32; N]) -> [i32; N] {
    let mut out = defaults;
    let b = text.as_bytes();
    let mut at = 0;
    for (k, slot) in out.iter_mut().enumerate() {
        if k > 0 {
            if b.get(at) != Some(&b',') {
                break;
            }
            at += 1;
        }
        let (v, used) = c_long(b.get(at..).unwrap_or(&[]));
        if used == 0 {
            break;
        }
        *slot = v as i32;
        at += used;
    }
    out
}

// -- reads -------------------------------------------------------------------------------------

/// A lookup the engine resolves, left to the caller to load: the sequential load in one call.
struct MapLoader<'a, F: Fn(&str) -> Option<&'a ParsedFile> + Sync>(F);

impl<'a, F: Fn(&str) -> Option<&'a ParsedFile> + Sync> Loader for MapLoader<'a, F> {
    fn file(&self, path: &str) -> Option<&ParsedFile> {
        (self.0)(path)
    }
}

impl Ltx {
    /// Loads `root` and everything it includes, as the engine does in one `CInifile`.
    pub fn assemble(root: &str, loader: &dyn Loader, dialect: Dialect) -> Result<Loaded, LtxError> {
        let mut b = Builder::new(dialect);
        b.load(root, loader)?;
        Ok(b.finish())
    }

    /// [`Ltx::assemble`] over a map of parsed files keyed by the paths [`include_path`] gives.
    pub fn assemble_map(
        root: &str,
        files: &FxHashMap<String, ParsedFile>,
        dialect: Dialect,
    ) -> Result<Loaded, LtxError> {
        Self::assemble(root, &MapLoader(|p: &str| files.get(p)), dialect)
    }

    /// The files the tree was read from; an [`Item`]'s and a [`Section`]'s `file` index it.
    #[must_use]
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    #[must_use]
    pub fn section_count(&self) -> usize {
        self.sections.len()
    }

    /// The sections sorted by name, as the engine keeps and walks them.
    #[must_use]
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }

    /// The sections in the order they were first read.
    pub fn sections_in_file_order(&self) -> impl Iterator<Item = &Section> {
        self.file_order.iter().filter_map(|&i| self.sections.get(i as usize))
    }

    /// A section by its exact name (`section_exist` compares as given).
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.by_name.get(name).and_then(|&i| self.sections.get(i as usize))
    }

    #[must_use]
    pub fn section_exist(&self, name: &str) -> bool {
        self.by_name.contains_key(name)
    }

    #[must_use]
    pub fn line_exist(&self, section: &str, key: &str) -> bool {
        self.section(section).is_some_and(|s| s.get(key).is_some())
    }

    /// `r_section`: the name lower-cased first.
    pub fn r_section(&self, name: &str) -> Result<&Section, LtxError> {
        let lc = name.to_ascii_lowercase();
        self.section(&lc).ok_or_else(|| LtxError::NoSection { section: name.to_owned() })
    }

    pub fn line_count(&self, section: &str) -> Result<usize, LtxError> {
        Ok(self.r_section(section)?.items.len())
    }

    /// The key and value at an index, in the engine's item order; `None` past the end.
    pub fn r_line(&self, section: &str, index: usize) -> Result<Option<(&str, Option<&str>)>, LtxError> {
        Ok(self.r_section(section)?.items.get(index).map(|it| (&*it.key, it.value.as_deref())))
    }

    fn item(&self, section: &str, key: &str) -> Result<&Item, LtxError> {
        let s = self.r_section(section)?;
        s.get(key).ok_or_else(|| LtxError::NoKey {
            section: section.to_owned(),
            key: key.to_owned(),
            file: self.files.get(s.file as usize).map_or_else(String::new, |f| f.to_string()),
            line: s.line,
        })
    }

    /// The value as stored, quotes included; `None` for an empty value (`r_string`).
    pub fn r_string_raw(&self, section: &str, key: &str) -> Result<Option<&str>, LtxError> {
        Ok(self.item(section, key)?.value.as_deref())
    }

    /// The value with one pair of quotes stripped (`r_string_wb`): a trailing `"` goes, then a
    /// leading one. An empty value is `""`.
    pub fn r_string(&self, section: &str, key: &str) -> Result<&str, LtxError> {
        let v = self.r_string_raw(section, key)?.unwrap_or("");
        let v = v.strip_suffix('"').unwrap_or(v);
        Ok(v.strip_prefix('"').unwrap_or(v))
    }

    fn text(&self, section: &str, key: &str) -> Result<&str, LtxError> {
        Ok(self.r_string_raw(section, key)?.unwrap_or(""))
    }

    pub fn r_u8(&self, section: &str, key: &str) -> Result<u8, LtxError> {
        Ok(c_atoi(self.text(section, key)?) as u8)
    }
    pub fn r_u16(&self, section: &str, key: &str) -> Result<u16, LtxError> {
        Ok(c_atoi(self.text(section, key)?) as u16)
    }
    pub fn r_u32(&self, section: &str, key: &str) -> Result<u32, LtxError> {
        Ok(c_atoi(self.text(section, key)?) as u32)
    }
    pub fn r_s32(&self, section: &str, key: &str) -> Result<i32, LtxError> {
        Ok(c_atoi(self.text(section, key)?))
    }
    pub fn r_s64(&self, section: &str, key: &str) -> Result<i64, LtxError> {
        Ok(c_long(self.text(section, key)?.as_bytes()).0)
    }
    /// `_strtoui64`: a negative number wraps.
    /// `_strtoui64(.., 10)`: a GUID half above `i64::MAX` reads whole.
    pub fn r_u64(&self, section: &str, key: &str) -> Result<u64, LtxError> {
        Ok(c_strtoui64(self.text(section, key)?.as_bytes()))
    }
    pub fn r_float(&self, section: &str, key: &str) -> Result<f32, LtxError> {
        Ok(c_strtod(self.text(section, key)?.as_bytes()).0 as f32)
    }
    pub fn r_fvector2(&self, section: &str, key: &str) -> Result<[f32; 2], LtxError> {
        Ok(scan_floats(self.text(section, key)?))
    }
    pub fn r_fvector3(&self, section: &str, key: &str) -> Result<[f32; 3], LtxError> {
        Ok(scan_floats(self.text(section, key)?))
    }
    pub fn r_fvector4(&self, section: &str, key: &str) -> Result<[f32; 4], LtxError> {
        Ok(scan_floats(self.text(section, key)?))
    }
    /// `Fcolor` as `[r, g, b, a]`.
    pub fn r_fcolor(&self, section: &str, key: &str) -> Result<[f32; 4], LtxError> {
        Ok(scan_floats(self.text(section, key)?))
    }
    pub fn r_ivector2(&self, section: &str, key: &str) -> Result<[i32; 2], LtxError> {
        Ok(scan_ints(self.text(section, key)?, [0; 2]))
    }
    pub fn r_ivector3(&self, section: &str, key: &str) -> Result<[i32; 3], LtxError> {
        Ok(scan_ints(self.text(section, key)?, [0; 3]))
    }
    pub fn r_ivector4(&self, section: &str, key: &str) -> Result<[i32; 4], LtxError> {
        Ok(scan_ints(self.text(section, key)?, [0; 4]))
    }
    /// `r, g, b[, a]` (alpha 255 when absent) as `0xAARRGGBB`.
    pub fn r_color(&self, section: &str, key: &str) -> Result<u32, LtxError> {
        let [r, g, b, a] = scan_ints(self.text(section, key)?, [0, 0, 0, 255]).map(|x| (x as u32) & 0xff);
        Ok((a << 24) | (r << 16) | (g << 8) | b)
    }
    /// `on`, `yes`, `true`, `1` (the first seven characters, any case).
    pub fn r_bool(&self, section: &str, key: &str) -> Result<bool, LtxError> {
        let v: String = self.text(section, key)?.chars().take(7).collect::<String>().to_ascii_lowercase();
        Ok(matches!(v.as_str(), "on" | "yes" | "true" | "1"))
    }
    /// `TEXT2CLSID`: eight characters, padded with spaces, as a big-endian `u64`.
    pub fn r_clsid(&self, section: &str, key: &str) -> Result<u64, LtxError> {
        let t = self.text(section, key)?.as_bytes();
        let t = t.split(|&c| c == 0).next().unwrap_or(&[]);
        let mut id: u64 = 0;
        for k in 0..8 {
            // a char of the C++ is signed: a byte above 0x7f spreads over the higher ones
            let c = t.get(k).copied().unwrap_or(b' ') as i8 as i64 as u64;
            id |= c << (56 - 8 * k);
        }
        Ok(id)
    }
    /// The id of the token named (any case), 0 when none is.
    pub fn r_token(&self, section: &str, key: &str, tokens: &[(&str, i32)]) -> Result<i32, LtxError> {
        let v = self.text(section, key)?;
        Ok(tokens.iter().find(|(n, _)| n.eq_ignore_ascii_case(v)).map_or(0, |(_, id)| *id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsigned_64_bit_values_read_whole() {
        assert_eq!(c_strtoui64(b"14137665947197255590"), 14_137_665_947_197_255_590);
        assert_eq!(c_strtoui64(b" 7"), 7);
        assert_eq!(c_strtoui64(b"99999999999999999999"), u64::MAX);
        assert_eq!(c_strtoui64(b"-1"), u64::MAX);
        let l =
            load(&[("a.ltx", "[guid]\nguid_g1 = 14137665947197255590\n")], "a.ltx", Dialect::Editor).unwrap();
        assert_eq!(l.ltx.r_u64("guid", "guid_g1").unwrap(), 14_137_665_947_197_255_590);
    }

    fn load(files: &[(&str, &str)], root: &str, dialect: Dialect) -> Result<Loaded, LtxError> {
        let map: FxHashMap<String, ParsedFile> =
            files.iter().map(|(p, t)| ((*p).to_owned(), parse_file(t, dialect))).collect();
        Ltx::assemble_map(root, &map, dialect)
    }

    fn keys(ltx: &Ltx, s: &str) -> Vec<String> {
        ltx.r_section(s).unwrap().items.iter().map(|i| i.key.to_string()).collect()
    }

    #[test]
    fn reads_values_comments_and_quotes() {
        let text = "junk = 1\r\n[Main] ; c\r\n\r\nb = 2 // c\r\na = \"x ; y\" \r\nc = a b , c\r\nd\r\ne =\r\nf = 1 ;\"\r\nurl = a/b//c\r\n";
        let l = load(&[("cfg\\a.ltx", text)], "cfg\\a.ltx", Dialect::Editor).unwrap().ltx;
        assert!(l.section_exist("main"));
        assert!(!l.section_exist("Main"));
        assert!(l.r_section("MAIN").is_ok());
        assert_eq!(keys(&l, "main"), ["a", "b", "c", "d", "e", "f", "url"]);
        assert_eq!(l.r_string("main", "a").unwrap(), "x ; y");
        assert_eq!(l.r_string_raw("main", "a").unwrap(), Some("\"x ; y\""));
        assert_eq!(l.r_string("main", "c").unwrap(), "ab,c");
        assert_eq!(l.r_string_raw("main", "d").unwrap(), None);
        assert_eq!(l.r_s32("main", "b").unwrap(), 2);
        assert_eq!(l.r_string("main", "url").unwrap(), "a/b//c");
        assert_eq!(l.r_line("main", 1).unwrap(), Some(("b", Some("2"))));
        assert_eq!(l.r_line("main", 9).unwrap(), None);
        assert!(matches!(l.r_string("main", "zz"), Err(LtxError::NoKey { .. })));
        assert!(matches!(l.r_section("nope"), Err(LtxError::NoSection { .. })));
    }

    #[test]
    fn inherits_in_order_and_overrides() {
        let text = "[base]\nv = 1\nw = base\n[mid]\nw = mid\nx = mid\n[child]:base, mid\nz = 3\nv = child\n";
        let l = load(&[("a.ltx", text)], "a.ltx", Dialect::Editor).unwrap().ltx;
        assert_eq!(keys(&l, "child"), ["v", "w", "x", "z"]);
        assert_eq!(l.r_string("child", "v").unwrap(), "child");
        assert_eq!(l.r_string("child", "w").unwrap(), "mid");
        let e = load(&[("a.ltx", "[c]:nobody\n")], "a.ltx", Dialect::Editor).unwrap_err();
        assert!(matches!(e, LtxError::UnknownParent { line: 1, .. }));
        let e = load(&[("a.ltx", "[s]\n[t]\n[s]\n")], "a.ltx", Dialect::Editor).unwrap_err();
        assert!(matches!(e, LtxError::DuplicateSection { line: 3, first_line: 1, .. }));
        let e = load(&[("a.ltx", "[s\n")], "a.ltx", Dialect::Editor).unwrap_err();
        assert!(matches!(e, LtxError::BadSection { .. }));
    }

    #[test]
    fn includes_resolve_against_the_includer() {
        let files = [
            (
                "configs\\system.ltx",
                "#include \"items\\items.ltx\"\n[sys]:item\nk = v\n#include \"missing.ltx\"\n",
            ),
            ("configs\\items\\items.ltx", "#include \"weapons.ltx\"\n[item]\ncost = 10\n"),
            ("configs\\items\\weapons.ltx", "[wpn]\nammo = a, b\n"),
        ];
        let r = load(&files, "configs\\system.ltx", Dialect::Editor).unwrap();
        let l = &r.ltx;
        assert_eq!(l.r_u32("sys", "cost").unwrap(), 10);
        assert_eq!(l.r_string("wpn", "ammo").unwrap(), "a,b");
        assert_eq!(r.warnings.len(), 1);
        let order: Vec<&str> = l.sections_in_file_order().map(|s| &*s.name).collect();
        assert_eq!(order, ["wpn", "item", "sys"]);
        let names: Vec<&str> = l.sections().iter().map(|s| &*s.name).collect();
        assert_eq!(names, ["item", "sys", "wpn"]);
        let e = load(&files, "configs\\system.ltx", Dialect::Game).unwrap_err();
        assert!(matches!(e, LtxError::MissingInclude { line: 4, .. }));
        let looping = [("a.ltx", "#include \"a.ltx\"\n")];
        assert!(matches!(load(&looping, "a.ltx", Dialect::Editor), Err(LtxError::IncludeCycle { .. })));
    }

    #[test]
    fn multiline_values() {
        let text = "[s]\r\nm = \"one\r\ntwo\"\r\nn = 1\r\n";
        let l = load(&[("a.ltx", text)], "a.ltx", Dialect::Editor).unwrap().ltx;
        assert_eq!(l.r_string("s", "m").unwrap(), "one\r\ntwo");
        assert_eq!(l.r_s32("s", "n").unwrap(), 1);
        let broken = "[s]\r\nm = \"one\r\n[t]\r\nk = 1\r\n";
        let r = load(&[("a.ltx", broken)], "a.ltx", Dialect::Game).unwrap();
        assert_eq!(r.ltx.r_string_raw("s", "m").unwrap(), Some("one"));
        assert_eq!(r.ltx.r_s32("t", "k").unwrap(), 1);
        assert!(matches!(r.warnings.as_slice(), [LtxWarning::QuotesTrimmed { .. }]));
        assert!(matches!(
            load(&[("a.ltx", "[s]\nm = \"open\n")], "a.ltx", Dialect::Editor),
            Err(LtxError::OddQuotes { .. })
        ));
    }

    #[test]
    fn typed_reads() {
        let text = "[s]\nb = TRUE\nf = 1.5x\nv = 1, 2,3\nw = 1 ,2\ni = 3,4\nc = 255,0,128\nu = -1\nid = W_AK74\nt = High\n";
        let l = load(&[("a.ltx", text)], "a.ltx", Dialect::Editor).unwrap().ltx;
        assert!(l.r_bool("s", "b").unwrap());
        assert_eq!(l.r_float("s", "f").unwrap(), 1.5);
        assert_eq!(l.r_fvector3("s", "v").unwrap(), [1.0, 2.0, 3.0]);
        assert_eq!(l.r_fvector3("s", "w").unwrap(), [1.0, 2.0, 0.0]);
        assert_eq!(l.r_ivector2("s", "i").unwrap(), [3, 4]);
        assert_eq!(l.r_color("s", "c").unwrap(), 0xFFFF_0080);
        assert_eq!(l.r_u32("s", "u").unwrap(), u32::MAX);
        assert_eq!(l.r_clsid("s", "id").unwrap(), u64::from_be_bytes(*b"W_AK74  "));
        assert_eq!(l.r_token("s", "t", &[("low", 1), ("high", 2)]).unwrap(), 2);
    }

    struct Game(FxHashMap<String, ParsedFile>);
    impl Loader for Game {
        fn file(&self, path: &str) -> Option<&ParsedFile> {
            self.0.get(path)
        }
        fn list(&self, dir: &str, mask: &str) -> Vec<String> {
            let (head, tail) = mask.split_once('*').unwrap_or((mask, ""));
            self.0
                .keys()
                .filter_map(|k| k.strip_prefix(dir))
                .filter(|n| n.starts_with(head) && n.ends_with(tail))
                .map(str::to_owned)
                .collect()
        }
        fn layer(&self, path: &str) -> u16 {
            u16::from(path.starts_with("mod\\"))
        }
    }

    #[test]
    fn game_dialect_wildcards_layers_and_patches() {
        let files: FxHashMap<String, ParsedFile> = [
            ("c\\system.ltx", "#include_optional \"gone.ltx\"\n#include \"mod_*.ltx\"\n[w]\na = 1\nz = 1\n"),
            ("c\\mod_b.ltx", "[b]\nk = 2\n"),
            ("c\\mod_a.ltx", "[a]\nk = 1\n"),
            ("mod\\over.ltx", "[w]\nz = 2\nnew = 3\n"),
        ]
        .into_iter()
        .map(|(p, t)| (p.to_owned(), parse_file(t, Dialect::Game)))
        .collect();
        let loader = Game(files);
        let mut b = Builder::new(Dialect::Game);
        b.load("c\\system.ltx", &loader).unwrap();
        b.set_merge(true);
        b.overlay("mod\\over.ltx", &loader).unwrap();
        b.patch(
            "mod\\patch\\p.ltxp",
            "[w]\n+list = x, y\n-list = x\n!a\ncost = 3 000 ; c\n[!b]\n[n] : w\n",
            1,
        );
        let r = b.finish();
        let l = &r.ltx;
        assert_eq!(r.warnings.len(), 1);
        assert_eq!(keys(l, "w"), ["z", "new", "list", "cost"]);
        assert_eq!(l.r_string("w", "z").unwrap(), "2");
        assert_eq!(l.r_string("w", "list").unwrap(), "y");
        assert_eq!(l.r_string("w", "cost").unwrap(), "3 000");
        assert!(!l.section_exist("b"));
        assert!(l.section_exist("a"));
        assert_eq!(l.r_string("n", "cost").unwrap(), "3 000");
        let order: Vec<&str> = l.sections_in_file_order().map(|s| &*s.name).collect();
        assert_eq!(order, ["a", "w", "n"]);
    }
}
