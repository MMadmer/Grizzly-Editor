//! The engine's resolved view of a config tree: what `system.ltx` and the hundreds
//! of files it includes mean once the engine has read them.
//!
//! # API
//!
//! - [`parse_file`] (`text`, [`Dialect`]) -> [`ParsedFile`]: one file, on its own, so a caller
//!   can read and parse hundreds of files in parallel. [`ParsedFile::includes`] names what it
//!   pulls in; [`include_path`] resolves a name the way the engine does (the including file's
//!   folder + the name, concatenated, nothing normalised).
//! - [`Ltx::assemble`] (`root`, a [`Loader`], [`Dialect`]) -> [`Loaded`]: the same tree as the
//!   sequential load. [`Builder`] does the same in steps for the game's composition:
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
//! What the engine treats as fatal - a missing section or key, a duplicate section, an unknown
//! parent, a missing include, a section line without `]` - is an [`LtxError`] naming the
//! section, the key, the file and the line.
//!
//! # Semantics (both dialects)
//!
//! - A line is what the engine's reader returns: runs of `\r` / `\n` end it, so blank lines
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
//! # The editor's dialect ([`Dialect::Editor`])
//!
//! A missing include is skipped with a warning (the editor cannot see the game's archives).
//! A multi-line value that outgrows the engine's 4096-byte buffer is an error.
//!
//! # The game's dialect ([`Dialect::Game`], Dead Air Refined's ini reader + XMS)
//!
//! - `#include_optional "name"`: an include that may be absent. A plain missing include is an
//!   error.
//! - `#include "dir\*.ltx"`: every file the [`Loader`] lists under the folder (sorted by name);
//!   the folder is the including file's folder + the name's folder, and - as in the game - the
//!   name's folder once more (`configs\x\*.ltx` lists `configs\x\x\`).
//! - A multi-line value that meets a line holding `[...]`, or outgrows the buffer, is cut back
//!   to its first line (a warning) and reading goes on with the next line.
//! - Layers ([`Loader::layer`]): with merging on ([`Builder::set_merge`], the module stage; the
//!   editor composes a module over a game's tree with it in its own dialect too), a section that
//!   exists in another layer is merged instead of refused - new keys are appended, a key from a
//!   layer at least as high wins - and the merged keys follow the sorted ones.
//! - `.ltxp` directive patches ([`Builder::patch`], parsed by [`crate::patch`]): `[section]`
//!   (created, `: parents` inherited, when new; the parents of a section that exists are left
//!   as they are), `[!section]` deletes it, `key = value` sets (the value kept as written),
//!   `+key = a, b` appends missing list items, `-key = a` removes them, `!key` deletes the key;
//!   `;` starts a comment. New keys go to the end of the section. `#file <path under configs>`
//!   (the word in any case; the path lower-cased, `.` and `..` resolved) sends the directives
//!   after it to that file ([`crate::FilePatch`]) - and to the system config too when the tree was
//!   read from it; a path that climbs out or names a drive discards them up to the next `#file`;
//!   `#file` alone returns to the system config.
//!
//! # Changes reach what inherits
//!
//! A section remembers its parents ([`Section::parents`]) and which keys are lines of its own
//! ([`Item::own`]). A change of a key by a module - an overlay merge, a `.ltxp` directive, a
//! directive of [`Builder::apply`] - reaches every section that inherits the key from the
//! changed one: a section that has no line of its own for the key and whose later parents do
//! not give it takes the value of the last of its parents that has the key, or loses the key
//! when none has it any more, and the change goes on to its own heirs. A key deleted by `!key`
//! counts as a line of its own: a later change of a parent does not bring it back.

use std::sync::Arc;

use rustc_hash::FxHashMap;

use crate::patch::{Directive, FilePatch, Op};

/// Which reader's rules apply.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Dialect {
    /// The editor's ini reader: a missing include is skipped with a warning.
    Editor,
    /// The game's the ini reader and its module composition.
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

/// The next line and where reading goes on.
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

/// Parses one file the way the engine's ini reader does.
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
    /// Where the value was written: [`Ltx::files`]; a copied line keeps its parent's place.
    pub file: u32,
    pub line: u32,
    /// A line of the section itself (or a module's change to it), not a copy of a parent's.
    pub own: bool,
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
    /// What it inherits from, as its header named it (lower case), in order.
    pub parents: Vec<Arc<str>>,
    /// Keys a `!key` took away: they hold against what the parents give.
    dropped: Vec<Arc<str>>,
    layer: u16,
    item_layers: Option<Vec<u16>>,
}

impl Section {
    fn new(name: Arc<str>, file: u32, line: u32, layer: u16) -> Self {
        Self {
            name,
            items: Vec::new(),
            index: FxHashMap::default(),
            file,
            line,
            parents: Vec::new(),
            dropped: Vec::new(),
            layer,
            item_layers: None,
        }
    }

    /// The line of a key.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Item> {
        self.index.get(key).and_then(|&i| self.items.get(i as usize))
    }

    /// Whether the section says its own word on a key: a line of its own, or a `!key`. An inventory icon is its
    /// texture and its rectangle together, as the game reads it: a section that states its own rectangle keeps
    /// its texture when a parent's changes.
    #[must_use]
    pub fn holds(&self, key: &str) -> bool {
        let says = |k: &str| self.get(k).is_some_and(|it| it.own) || self.dropped.iter().any(|d| &**d == k);
        says(key) || key == "icons_texture" && (says("inv_grid_x") || says("inv_grid_y"))
    }

    /// A parent's line, copied: never a line of the section's own.
    fn inherit(&mut self, it: &Item) {
        self.insert(Item { own: false, ..it.clone() });
    }

    /// `insert_item`: overrides a key in place or appends it.
    fn insert(&mut self, it: Item) {
        if let Some(&i) = self.index.get(&*it.key) {
            if let Some(x) = self.items.get_mut(i as usize) {
                x.value = it.value;
                x.file = it.file;
                x.line = it.line;
                x.own = it.own;
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

    /// Takes a key out, with its layer.
    fn take(&mut self, key: &str) -> bool {
        let Some(&at) = self.index.get(key) else { return false };
        self.items.remove(at as usize);
        if let Some(v) = self.item_layers.as_mut()
            && (at as usize) < v.len()
        {
            v.remove(at as usize);
        }
        self.rebuild_index();
        true
    }

    /// A module's line: set in place or appended, a line of the section's own from now on.
    fn put(&mut self, it: Item, layer: u16) {
        self.dropped.retain(|d| *d != it.key);
        if let Some(&k) = self.index.get(&*it.key) {
            if let Some(x) = self.items.get_mut(k as usize) {
                *x = it;
            }
            if let Some(l) = self.item_layers.as_mut().and_then(|v| v.get_mut(k as usize)) {
                *l = layer;
            }
            return;
        }
        self.index.insert(it.key.clone(), u32::try_from(self.items.len()).unwrap_or(u32::MAX));
        self.items.push(it);
        if let Some(v) = self.item_layers.as_mut() {
            v.push(layer);
        }
    }

    /// A key a parent gives anew (`Some`) or no longer gives (`None`), for a section that has no
    /// word of its own on it. Whether its value changed.
    fn follow(&mut self, key: &str, from: Option<&Item>) -> bool {
        match from {
            None => self.take(key),
            Some(it) => {
                let same = self
                    .get(key)
                    .is_some_and(|x| x.value == it.value && x.file == it.file && x.line == it.line);
                if same {
                    return false;
                }
                let layer = self.layer;
                let it = Item { own: false, ..it.clone() };
                if let Some(&k) = self.index.get(key) {
                    if let Some(x) = self.items.get_mut(k as usize) {
                        *x = it;
                    }
                    return true;
                }
                self.index.insert(it.key.clone(), u32::try_from(self.items.len()).unwrap_or(u32::MAX));
                self.items.push(it);
                if let Some(v) = self.item_layers.as_mut() {
                    v.push(layer);
                }
                true
            }
        }
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
    /// The names of the files below `dir` (as the game's file listing without
    /// its root-only flag lists them) that match `mask` (`*` any run of characters). Game dialect only.
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

/// The composition of a tree, step by step, the way the game builds its settings.
pub struct Builder {
    dialect: Dialect,
    merge: bool,
    sections: Vec<Section>,
    by_name: FxHashMap<Arc<str>, usize>,
    created: Vec<usize>,
    removed: Vec<bool>,
    /// The sections that name each one among their parents, in the order they were made.
    children: Vec<Vec<usize>>,
    /// The heirs of a name no section has now - it was removed, or never made: a section made
    /// under that name later has them, as the game finds heirs by the parent's name.
    waiting: FxHashMap<Arc<str>, Vec<usize>>,
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
            children: Vec::new(),
            waiting: FxHashMap::default(),
            files: Vec::new(),
            file_ids: FxHashMap::default(),
            warnings: Vec::new(),
            stack: Vec::new(),
        }
    }

    /// A builder that goes on from a finished tree: its sections with their parents and their
    /// own lines, its files. What a module's composition is laid over a game's tree with.
    #[must_use]
    pub fn from_ltx(ltx: &Ltx, dialect: Dialect) -> Self {
        let mut b = Self::new(dialect);
        let n = ltx.sections.len();
        // the order the sections were made in: the file order, then any the order lacks
        let mut order: Vec<usize> = ltx.file_order.iter().map(|&i| i as usize).filter(|&i| i < n).collect();
        let mut listed = vec![false; n];
        for &i in &order {
            if let Some(l) = listed.get_mut(i) {
                *l = true;
            }
        }
        order.extend((0..n).filter(|&i| !listed.get(i).copied().unwrap_or(true)));
        for i in order {
            let Some(s) = ltx.sections.get(i) else { continue };
            let at = b.sections.len();
            b.by_name.insert(s.name.clone(), at);
            b.sections.push(s.clone());
            b.created.push(at);
            b.removed.push(false);
            let heirs = b.waiting.remove(&s.name).unwrap_or_default();
            b.children.push(heirs);
            b.link_parents(at);
        }
        for f in ltx.files.iter() {
            b.file_id(f);
        }
        b
    }

    /// Cross-layer merging of duplicate sections: the game's composition of its modules, also
    /// as the editor lays a module over a game's tree.
    pub fn set_merge(&mut self, on: bool) {
        self.merge = on;
    }

    /// A section as it stands now, by its name (any case).
    #[must_use]
    pub fn section(&self, name: &str) -> Option<&Section> {
        self.section_lc(name).and_then(|i| self.sections.get(i))
    }

    /// Whether a section stands now (any case).
    #[must_use]
    pub fn has(&self, name: &str) -> bool {
        self.section_lc(name).is_some()
    }

    /// The files read so far; an [`Item`]'s `file` indexes them.
    #[must_use]
    pub fn files(&self) -> &[Arc<str>] {
        &self.files
    }

    /// The id of a file in the tree, added when it is new: what [`Item::file`] holds.
    pub fn file_id(&mut self, path: &str) -> u32 {
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

    /// Loads a file and everything it includes (on a fresh reader).
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
            // the section that exists keeps its parents: what the merge wrote is its own
            let changed = self.sections.get_mut(i).map(|dst| merge_section(dst, s)).unwrap_or_default();
            for key in changed {
                self.propagate(i, &key);
            }
            return Ok(());
        }
        self.add_section(s);
        Ok(())
    }

    /// A new section at the end, its parents told of it.
    fn add_section(&mut self, s: Section) -> usize {
        let i = self.sections.len();
        self.by_name.insert(s.name.clone(), i);
        self.sections.push(s);
        self.created.push(i);
        self.removed.push(false);
        // the heirs of a section of this name that was removed, or that was not there yet
        let heirs = self.sections.get(i).and_then(|s| self.waiting.remove(&s.name)).unwrap_or_default();
        self.children.push(heirs);
        self.link_parents(i);
        i
    }

    /// Lists section `i` among the children of each of its parents - by their names: a parent
    /// that is not there now has its heirs kept for the section that takes the name.
    fn link_parents(&mut self, i: usize) {
        let parents: Vec<Arc<str>> = self.sections.get(i).map(|s| s.parents.clone()).unwrap_or_default();
        for name in parents {
            let list = match self.find(&name) {
                Some(p) => self.children.get_mut(p),
                None => Some(self.waiting.entry(name).or_default()),
            };
            if let Some(c) = list
                && !c.contains(&i)
            {
                c.push(i);
            }
        }
    }

    /// Carries a change of `key` in section `from` to every section that inherits it from there:
    /// one without a word of its own on the key takes it from the last of its parents
    /// that has it, or loses it, and passes the change on.
    fn propagate(&mut self, from: usize, key: &str) {
        let mut queue: std::collections::VecDeque<usize> =
            self.children.get(from).cloned().unwrap_or_default().into();
        while let Some(c) = queue.pop_front() {
            if self.removed.get(c).copied().unwrap_or(true) {
                continue;
            }
            let Some(sec) = self.sections.get(c) else { continue };
            if sec.holds(key) {
                continue;
            }
            let given = sec
                .parents
                .iter()
                .rev()
                .filter_map(|p| self.find(p))
                .find_map(|p| self.sections.get(p).and_then(|ps| ps.get(key)).cloned());
            let changed = self.sections.get_mut(c).is_some_and(|s| s.follow(key, given.as_ref()));
            if changed {
                queue.extend(self.children.get(c).cloned().unwrap_or_default());
            }
        }
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
                                s.inherit(it);
                            }
                            s.parents.push(ps.name.clone());
                        }
                    }
                    current = Some(s);
                }
                Ev::Item { key, value, line } => {
                    if let Some(s) = current.as_mut() {
                        s.insert(Item {
                            key: key.clone(),
                            value: value.clone(),
                            file: fid,
                            line: *line,
                            own: true,
                        });
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
            // the game adds the mask's folder to a folder that already holds it
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

    /// `[name]` of a patch: a section that exists is patched as it is (its parents stay), a new
    /// one is made with its parents' lines. False when a parent is missing: the lines that
    /// follow are skipped.
    fn patch_create(
        &mut self,
        name: &str,
        parents: Option<&[String]>,
        layer: u16,
        fid: u32,
        line: u32,
    ) -> bool {
        if self.find(name).is_some() {
            return true;
        }
        let mut s = Section::new(arc(name), fid, line, layer);
        for p in parents.unwrap_or_default() {
            let lc = p.to_ascii_lowercase();
            let Some(pi) = self.find(&lc) else {
                self.warnings.push(LtxWarning::PatchParentMissing {
                    section: name.to_owned(),
                    parent: p.clone(),
                    line,
                });
                return false;
            };
            if let Some(ps) = self.sections.get(pi) {
                for it in &ps.items {
                    s.inherit(it);
                }
                s.parents.push(ps.name.clone());
            }
        }
        self.add_section(s);
        true
    }

    fn patch_set(&mut self, section: &str, key: &str, value: &str, layer: u16, fid: u32, line: u32) {
        let Some(i) = self.find(section) else { return };
        let Some(s) = self.sections.get_mut(i) else { return };
        let value = (!value.is_empty()).then(|| arc(value));
        s.put(Item { key: arc(key), value, file: fid, line, own: true }, layer);
        self.propagate(i, key);
    }

    fn patch_delete(&mut self, section: &str, key: &str) {
        let Some(i) = self.find(section) else { return };
        let Some(s) = self.sections.get_mut(i) else { return };
        s.take(key);
        if !s.dropped.iter().any(|d| &**d == key) {
            s.dropped.push(arc(key));
        }
        self.propagate(i, key);
    }

    fn list_items(value: &str) -> Vec<String> {
        value
            .split(',')
            .map(|x| x.trim_matches([' ', '\t', '\r']))
            .filter(|x| !x.is_empty())
            .map(str::to_owned)
            .collect()
    }

    /// Whether the tree was read from `configs\<file>` (a [`crate::patch::config_key`]).
    #[must_use]
    pub fn reads(&self, file: &str) -> bool {
        self.files.iter().any(|f| crate::patch::config_key(f) == file)
    }

    /// Applies a `.ltxp` directive file (`apply_patch_file`). `path` names it in the tree;
    /// `layer` is its module's layer. The directives `#file` sends to a file are answered, for
    /// the configs that read it ([`compose_file`]); a file the tree itself was read from takes
    /// them here too.
    pub fn patch(&mut self, path: &str, text: &str, layer: u16) -> Vec<FilePatch> {
        let mut out = Vec::new();
        for block in crate::patch::parse(text) {
            match block.file {
                Some(file) => {
                    if self.reads(&file) {
                        self.apply(path, &block.directives, layer);
                    }
                    out.push(FilePatch {
                        file,
                        source: path.to_owned(),
                        layer,
                        directives: block.directives,
                    });
                }
                None => self.apply(path, &block.directives, layer),
            }
        }
        out
    }

    /// Applies directives as one `.ltxp` block of `source` does: a header picks the section
    /// the lines after it change.
    pub fn apply(&mut self, source: &str, directives: &[Directive], layer: u16) {
        let fid = self.file_id(source);
        let mut section = String::new();
        let mut valid = false;
        for d in directives {
            let line = d.line;
            match &d.op {
                Op::Section { name, parents } => {
                    section = name.to_ascii_lowercase();
                    valid = self.patch_create(&section, parents.as_deref(), layer, fid, line);
                }
                Op::DropSection(name) => {
                    let del = name.to_ascii_lowercase();
                    if let Some((name, i)) = self.by_name.remove_entry(del.as_str()) {
                        if let Some(r) = self.removed.get_mut(i) {
                            *r = true;
                        }
                        // its heirs keep what they copied and wait for a section of that name
                        let heirs = self.children.get_mut(i).map(std::mem::take).unwrap_or_default();
                        if !heirs.is_empty() {
                            self.waiting.entry(name).or_default().extend(heirs);
                        }
                    }
                    valid = false;
                }
                _ if !valid => {}
                Op::Set { key, value } => self.patch_set(&section, key, value, layer, fid, line),
                Op::Append { key, value } => self.list_op(&section, key, value, true, layer, fid, line),
                Op::Remove { key, value } => self.list_op(&section, key, value, false, layer, fid, line),
                Op::Delete(key) => self.patch_delete(&section, key),
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

/// A config file read with everything it includes, as a script's `ini_file` or an engine reader
/// opens it, and the `#file` patches of the files it read applied in their order.
pub fn compose_file(
    root: &str,
    loader: &dyn Loader,
    dialect: Dialect,
    patches: &[FilePatch],
) -> Result<Loaded, LtxError> {
    let mut b = Builder::new(dialect);
    b.load(root, loader)?;
    for p in patches {
        if b.reads(&p.file) {
            b.apply(&p.source, &p.directives, p.layer);
        }
    }
    Ok(b.finish())
}

/// `xms_merge_section`: `src` into `dst`, layer against layer; answers the keys it wrote. A merge
/// is `dst` saying a key itself - every key of a source without parents, the source's own lines of
/// one that restates them (the lines it copied from them are written, and stay inherited) -
/// whichever side wins the value.
fn merge_section(dst: &mut Section, src: Section) -> Vec<Arc<str>> {
    let dst_layer = dst.layer;
    let n = dst.items.len();
    let layers = dst.item_layers.get_or_insert_with(|| vec![dst_layer; n]);
    let says_all = src.parents.is_empty();
    let heir = !dst.parents.is_empty();
    let mut changed = Vec::new();
    for it in src.items {
        let says = !heir || says_all || it.own;
        match dst.index.get(&*it.key) {
            None => {
                // a key `!key` took away stays the section's own word
                let held = dst.dropped.contains(&it.key);
                dst.dropped.retain(|d| *d != it.key);
                dst.index.insert(it.key.clone(), u32::try_from(dst.items.len()).unwrap_or(u32::MAX));
                changed.push(it.key.clone());
                dst.items.push(Item { own: says || held, ..it });
                layers.push(src.layer);
            }
            Some(&i) => {
                let old = layers.get(i as usize).copied().unwrap_or(0);
                let wins = src.layer >= old;
                if let Some(x) = dst.items.get_mut(i as usize) {
                    if wins {
                        changed.push(it.key.clone());
                        x.value = it.value;
                        x.file = it.file;
                        x.line = it.line;
                    }
                    x.own |= says;
                }
                if wins && let Some(l) = layers.get_mut(i as usize) {
                    *l = src.layer;
                }
            }
        }
    }
    dst.layer = dst.layer.max(src.layer);
    changed
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

/// The C runtime's string-to-u64: blanks, a sign, digits; `u64::MAX` when it overflows, and a minus
/// sign negates in two's complement, as the C runtime does.
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
    /// Loads `root` and everything it includes, as the engine does in one the ini reader.
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
    /// The string-to-u64 of a negative number wraps.
    /// The string-to-u64 of a GUID half above `i64::MAX` reads whole.
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
    /// A colour as `[r, g, b, a]`.
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
    /// Eight characters, padded with spaces, as a big-endian `u64`.
    pub fn r_clsid(&self, section: &str, key: &str) -> Result<u64, LtxError> {
        let t = self.text(section, key)?.as_bytes();
        let t = t.split(|&c| c == 0).next().unwrap_or(&[]);
        let mut id: u64 = 0;
        for k in 0..8 {
            // the game reads a char as signed: a byte above 0x7f spreads over the higher ones
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

    // -- 1.2: a change reaches what inherits ------------------------------------------------

    const WEAPONS: &str = "[base]\ncost = 1\nweight = 1\n[wpn_ak74]:base\nammo_class = a, b\nscopes = x\n[wpn_ak74_kobra]:wpn_ak74\nscope = kobra\n[wpn_ak74_own]:wpn_ak74\ncost = 50\n[wpn_ak74_m1]:wpn_ak74_kobra\n";

    fn tree(text: &str) -> Builder {
        let files: FxHashMap<String, ParsedFile> =
            [("c\\system.ltx".to_owned(), parse_file(text, Dialect::Game))].into_iter().collect();
        let mut b = Builder::new(Dialect::Game);
        b.load("c\\system.ltx", &Game(files)).unwrap();
        b
    }

    fn val(l: &Ltx, s: &str, k: &str) -> Option<String> {
        l.section(s).and_then(|x| x.get(k)).map(|it| it.value.as_deref().unwrap_or("").to_owned())
    }

    #[test]
    fn a_parents_patch_reaches_its_heirs_and_their_own_lines_hold() {
        let mut b = tree(WEAPONS);
        b.patch("p.ltxp", "[base]\ncost = 2\n[wpn_ak74]\n+ammo_class = c\n", 1);
        let l = b.finish().ltx;
        // a parent's change reaches a child and a grandchild, three levels down
        for s in ["wpn_ak74", "wpn_ak74_kobra", "wpn_ak74_m1"] {
            assert_eq!(val(&l, s, "cost").as_deref(), Some("2"), "{s}");
            assert_eq!(val(&l, s, "ammo_class").as_deref(), Some("a,b,c"), "{s}");
        }
        // a line of the child's own holds
        assert_eq!(val(&l, "wpn_ak74_own", "cost").as_deref(), Some("50"));
        assert_eq!(val(&l, "wpn_ak74_own", "ammo_class").as_deref(), Some("a,b,c"));
        // where the value comes from: the patch, for the child that took it
        let it = l.section("wpn_ak74_kobra").and_then(|s| s.get("cost")).unwrap();
        assert!(!it.own);
        assert_eq!(&*l.files()[it.file as usize], "p.ltxp");
        assert!(l.section("wpn_ak74_own").and_then(|s| s.get("cost")).unwrap().own);
        assert_eq!(l.section("wpn_ak74_m1").unwrap().parents, [Arc::<str>::from("wpn_ak74_kobra")]);
    }

    #[test]
    fn a_later_parent_wins_and_a_removed_key_falls_back() {
        let mut b = tree("[a]\nk = a\nonly_a = 1\n[b]\nk = b\n[c]:a, b\n[d]:c\n");
        b.patch("p.ltxp", "[a]\nk = x\n", 1);
        let l = b.finish().ltx;
        // the later parent gives k: a's change does not reach c
        assert_eq!(val(&l, "c", "k").as_deref(), Some("b"));
        // a finished tree goes on with what it knew of parents and own lines
        let mut b = Builder::from_ltx(&l, Dialect::Game);
        b.patch("q.ltxp", "[b]\nk = y\n", 1);
        let l = b.finish().ltx;
        assert_eq!(val(&l, "d", "k").as_deref(), Some("y"));
        let mut b = Builder::from_ltx(&l, Dialect::Game);
        b.patch("r.ltxp", "[b]\n!k\n", 1);
        let l = b.finish().ltx;
        // b no longer gives k: c and d fall back to a's
        assert_eq!(val(&l, "c", "k").as_deref(), Some("x"));
        assert_eq!(val(&l, "d", "k").as_deref(), Some("x"));
        let mut b = Builder::from_ltx(&l, Dialect::Game);
        b.patch("s.ltxp", "[a]\n!k\n!only_a\n", 1);
        let l = b.finish().ltx;
        // nobody gives it any more: it is gone below
        assert_eq!(val(&l, "c", "k"), None);
        assert_eq!(val(&l, "d", "only_a"), None);
    }

    #[test]
    fn a_childs_removal_holds_and_an_existing_header_keeps_its_parents() {
        let mut b = tree(WEAPONS);
        b.patch(
            "p.ltxp",
            "[wpn_ak74_kobra]\n!scopes\n[wpn_ak74_kobra] : nothing_here\n[wpn_ak74]\nscopes = y\n",
            1,
        );
        let l = b.finish().ltx;
        assert_eq!(val(&l, "wpn_ak74_kobra", "scopes"), None);
        // the grandchild inherits from the child, which said no
        assert_eq!(val(&l, "wpn_ak74_m1", "scopes"), None);
        assert_eq!(val(&l, "wpn_ak74_own", "scopes").as_deref(), Some("y"));
        assert_eq!(l.section("wpn_ak74_kobra").unwrap().parents, [Arc::<str>::from("wpn_ak74")]);
    }

    #[test]
    fn an_overlay_merge_reaches_what_inherits() {
        let files: FxHashMap<String, ParsedFile> = [
            ("c\\system.ltx", WEAPONS),
            ("mod\\over.ltx", "[base]\nweight = 3\n[wpn_ak74_own]\nweight = 9\n"),
        ]
        .into_iter()
        .map(|(p, t)| (p.to_owned(), parse_file(t, Dialect::Game)))
        .collect();
        let loader = Game(files);
        let mut b = Builder::new(Dialect::Game);
        b.load("c\\system.ltx", &loader).unwrap();
        b.set_merge(true);
        b.overlay("mod\\over.ltx", &loader).unwrap();
        let l = b.finish().ltx;
        assert_eq!(val(&l, "wpn_ak74_m1", "weight").as_deref(), Some("3"));
        assert_eq!(val(&l, "wpn_ak74_own", "weight").as_deref(), Some("9"));
    }

    #[test]
    fn an_overlay_that_restates_parents_keeps_what_it_copied_inherited() {
        let files: FxHashMap<String, ParsedFile> =
            [("c\\system.ltx", WEAPONS), ("mod\\over.ltx", "[wpn_ak74_kobra]:wpn_ak74\nscope = k2\n")]
                .into_iter()
                .map(|(p, t)| (p.to_owned(), parse_file(t, Dialect::Game)))
                .collect();
        let loader = Game(files);
        let mut b = Builder::new(Dialect::Game);
        b.load("c\\system.ltx", &loader).unwrap();
        b.set_merge(true);
        b.overlay("mod\\over.ltx", &loader).unwrap();
        // the overlay's header copied the parent's lines: merged, they are no word of the child's own
        b.patch("p.ltxp", "[wpn_ak74]\ncost = 9\n", 1);
        let l = b.finish().ltx;
        assert_eq!(val(&l, "wpn_ak74_kobra", "scope").as_deref(), Some("k2"));
        assert_eq!(val(&l, "wpn_ak74_kobra", "cost").as_deref(), Some("9"));
        assert_eq!(val(&l, "wpn_ak74_m1", "cost").as_deref(), Some("9"));
        let kobra = l.section("wpn_ak74_kobra").unwrap();
        assert!(kobra.get("scope").unwrap().own && !kobra.get("cost").unwrap().own);
    }

    #[test]
    fn a_variant_with_its_own_icon_cells_keeps_its_texture() {
        let mut b = tree(
            "[base]\ninv_grid_x = 1\ninv_grid_y = 1\n[item]:base\ninv_grid_x = 4\n[variant]:item\ninv_grid_y = 9\n[plain]:item\ncost = 3\n",
        );
        // the item gets a picture of its own: a texture and a rectangle on it
        b.patch("p.ltxp", "[item]\nicons_texture = ui\\mod_icons\ninv_grid_x = 0\ninv_grid_y = 0\n", 1);
        let l = b.finish().ltx;
        // a variant with cells of its own keeps the texture it had: none, the game's atlas
        assert_eq!(val(&l, "variant", "icons_texture"), None);
        assert_eq!(val(&l, "variant", "inv_grid_x").as_deref(), Some("0"));
        assert_eq!(val(&l, "variant", "inv_grid_y").as_deref(), Some("9"));
        // one without takes the whole new icon
        assert_eq!(val(&l, "plain", "icons_texture").as_deref(), Some("ui\\mod_icons"));
        assert_eq!(val(&l, "plain", "inv_grid_y").as_deref(), Some("0"));
    }

    #[test]
    fn a_section_made_again_keeps_its_heirs() {
        let mut b = tree(WEAPONS);
        b.patch("p.ltxp", "[!wpn_ak74]\n[wpn_ak74] : base\ncost = 9\n", 1);
        let l = b.finish().ltx;
        // the heirs are found by the parent's name: the new section's change reaches them
        assert_eq!(val(&l, "wpn_ak74_kobra", "cost").as_deref(), Some("9"));
        assert_eq!(val(&l, "wpn_ak74_m1", "cost").as_deref(), Some("9"));
        assert_eq!(val(&l, "wpn_ak74_own", "cost").as_deref(), Some("50"));
        // what they copied from the removed one stays until the new one says otherwise
        assert_eq!(val(&l, "wpn_ak74_kobra", "scopes").as_deref(), Some("x"));
        // and a finished tree goes on the same way
        let mut b = Builder::from_ltx(&l, Dialect::Game);
        b.patch("q.ltxp", "[!wpn_ak74]\n[wpn_ak74] : base\ncost = 11\n", 1);
        assert_eq!(val(&b.finish().ltx, "wpn_ak74_m1", "cost").as_deref(), Some("11"));
    }

    #[test]
    fn file_patches_go_to_the_files_that_read_them() {
        let files: FxHashMap<String, ParsedFile> = [
            ("configs\\system.ltx", "#include \"items.ltx\"\n"),
            ("configs\\items.ltx", "[base]\ncost = 1\n"),
            ("configs\\misc\\trade\\presets.ltx", "[merchant]\nbread = 1\n[cheap]:merchant\n"),
            (
                "configs\\misc\\trade\\trade_military.ltx",
                "#include \"presets.ltx\"\n[supplies_1]\nbread = 1, 1\n",
            ),
        ]
        .into_iter()
        .map(|(p, t)| (p.to_owned(), parse_file(t, Dialect::Game)))
        .collect();
        let loader = Game(files);
        let mut b = Builder::new(Dialect::Game);
        b.load("configs\\system.ltx", &loader).unwrap();
        let text = "[base]\ncost = 2\n#file Misc/Trade/Presets.ltx\n[merchant]\nvodka = 2\n#file items.ltx\n[base]\nweight = 5\n#file\n[base]\n+tags = x\n";
        let out = b.patch("mod\\p.ltxp", text, 1);
        let l = b.finish().ltx;
        // the system tree took its own blocks and the one naming a file it was read from
        assert_eq!(val(&l, "base", "cost").as_deref(), Some("2"));
        assert_eq!(val(&l, "base", "weight").as_deref(), Some("5"));
        assert_eq!(val(&l, "base", "tags").as_deref(), Some("x"));
        assert!(!l.section_exist("merchant"));
        // a block of a file of the tree is kept too, for another config that reads that file
        assert_eq!(
            out.iter().map(|p| p.file.as_str()).collect::<Vec<_>>(),
            ["misc\\trade\\presets.ltx", "items.ltx"]
        );
        // a trader's file reads the presets by its include: the patch is there, inherited too
        let trade = compose_file("configs\\misc\\trade\\trade_military.ltx", &loader, Dialect::Game, &out)
            .unwrap()
            .ltx;
        assert_eq!(val(&trade, "merchant", "vodka").as_deref(), Some("2"));
        assert_eq!(val(&trade, "cheap", "vodka").as_deref(), Some("2"));
        assert!(trade.section_exist("supplies_1"));
    }
}
