//! The project's items and tables as one [`Roster`]: every file read and parsed, the module laid
//! over the game's configs ([`crate::compose`]), the icon atlas planned - once per state of the
//! files, the manifest and the game. [`Roster::found`] answers where an item is found: the rows
//! of the module's traders, loot tables and recipes that name it.

use std::path::Path;
use std::sync::Arc;

use gz_base::Rev;
use gz_ltx::Ltx;
use gz_luadata::Seg;
use rustc_hash::FxHashMap;

use crate::atlas::{self, Plan, Wanted};
use crate::catalog::Catalog;
use crate::compose::{self, Composition, Lines, ModuleConfigs};
use crate::model::{Count, Icon, Issue, Item, Kind, load_text};
use crate::resolve::{Resolution, resolve};
use crate::tables::{self, Rows, TableFile, TableKind};
use crate::validate::{NOT_YOURS, TWICE};

/// The file does not read.
pub const UNREADABLE: &str = "I400";

/// One file of the roster.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Entry {
    /// As it reads; a file that does not read is an empty item of its path.
    pub item: Item,
    /// The problems of its own shape, and why it does not read (`I400`).
    pub issues: Vec<Issue>,
    pub readable: bool,
}

/// One table file of the roster.
#[derive(Clone, Debug, PartialEq)]
pub struct TableEntry {
    /// As it reads; a file that does not read is an empty file of its kind.
    pub table: TableFile,
    /// The problems of its own shape, and why it does not read (`I400`).
    pub issues: Vec<Issue>,
    pub readable: bool,
}

/// The detail of a stock row that takes the item off the trader's shelf.
pub const OFF_SHELF: &str = "taken off the shelf";

/// Where an item is found: one row of a trader, a loot table or a recipes file of the module.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct Found {
    /// `sold`, `carried`, `made` or `part` (the item is what a recipe takes).
    pub how: &'static str,
    /// The file that says it, project-relative.
    pub file: String,
    /// The row of the file, from 0.
    pub row: usize,
    /// The trader (`sold`), the tool or workbench (`made`), what the recipe makes (`part`).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub at: String,
    /// What the row says, as the file writes it: `2, 50%`, `0 - 1, 5%, bandit 10%`, `x 40`.
    pub detail: String,
}

/// Every item and table of the project, read, composed and planned.
#[derive(Clone, Debug, Default)]
pub struct Roster {
    /// The state of the files, the manifest and the game it was made from.
    pub stamp: Rev,
    pub module: String,
    /// In path order.
    pub entries: Vec<Entry>,
    /// The module's traders, then its loot tables, then its recipes, each kind in path order: the
    /// order the game reads them in.
    pub tables: Vec<TableEntry>,
    /// The game's tree without the module: what a change is compared with.
    pub game: Arc<Ltx>,
    /// The game's tree with the module.
    pub composition: Arc<Composition>,
    /// The icon atlas of the pictures the items name.
    pub plan: Plan,
    /// The string ids the module's items make: `<section>_name`, `_short`, `_descr`.
    pub strings: Vec<String>,
    /// What it was made from: the files as they read and the module's configs
    /// ([`Roster::with_edit`]).
    sources: Arc<Vec<Source>>,
    table_sources: Arc<Vec<TableSource>>,
    configs: Arc<ModuleConfigs>,
}

/// A table file as it reads, with the lines of its text.
#[derive(Clone, Debug)]
struct TableSource {
    entry: TableEntry,
    lines: Arc<Lines>,
}

impl TableSource {
    fn read(file: String, text: Result<String, String>) -> Option<Self> {
        let kind = TableKind::of_file(&file)?;
        Some(match text.and_then(|t| tables::load_text(kind, &t, &file).map(|r| (r, t))) {
            Ok(((table, issues), t)) => Self {
                entry: TableEntry { table, issues, readable: true },
                lines: Arc::new(Lines::of(Some(&t))),
            },
            Err(e) => {
                let issues = vec![Issue::error(UNREADABLE, Vec::new(), format!("{file}: {e}"))];
                Self {
                    entry: TableEntry { table: TableFile::new(kind, &file), issues, readable: false },
                    lines: Arc::new(Lines::of(None)),
                }
            }
        })
    }
}

/// The game's order of the table files: traders, loot tables, recipes, each kind by path.
fn table_order(file: &str) -> (u8, Vec<u8>) {
    let kind = match TableKind::of_file(file) {
        Some(TableKind::Trade) => 0,
        Some(TableKind::Loot) => 1,
        _ => 2,
    };
    (kind, gz_base::text::game_order_key(file))
}

/// A file as it reads, before the module is composed: its entry and the lines of its text. Read
/// once - a form's check of one item reads that item's text and nothing else again.
#[derive(Clone, Debug)]
struct Source {
    entry: Entry,
    lines: Arc<Lines>,
}

impl Source {
    fn read(file: String, text: Result<String, String>) -> Self {
        match text.and_then(|t| load_text(&t, &file).map(|r| (r, t))) {
            Ok(((item, issues), t)) => {
                Self { entry: Entry { item, issues, readable: true }, lines: Arc::new(Lines::of(Some(&t))) }
            }
            Err(e) => {
                let item = Item { file: file.clone(), ..Item::default() };
                let issues = vec![Issue::error(UNREADABLE, Vec::new(), format!("{file}: {e}"))];
                Self { entry: Entry { item, issues, readable: false }, lines: Arc::new(Lines::of(None)) }
            }
        }
    }
}

impl Roster {
    /// The roster of these files: `(file, text or the read's error)`, the items in path order,
    /// the table files in any.
    #[must_use]
    #[allow(clippy::too_many_arguments)]
    pub fn build(
        stamp: Rev,
        module: &str,
        files: Vec<(String, Result<String, String>)>,
        tables: Vec<(String, Result<String, String>)>,
        game: Arc<Ltx>,
        configs: &ModuleConfigs,
        root: &Path,
        picture: &atlas::PictureSize,
    ) -> Self {
        let sources: Vec<Source> = files.into_iter().map(|(file, text)| Source::read(file, text)).collect();
        let mut table_sources: Vec<TableSource> =
            tables.into_iter().filter_map(|(file, text)| TableSource::read(file, text)).collect();
        table_sources.sort_by_cached_key(|s| table_order(&s.entry.table.file));
        Self::assemble(
            stamp,
            module,
            Arc::new(sources),
            Arc::new(table_sources),
            game,
            Arc::new(configs.clone()),
            root,
            picture,
        )
    }

    /// The roster of files that are read: composed and planned.
    #[allow(clippy::too_many_arguments)]
    fn assemble(
        stamp: Rev,
        module: &str,
        sources: Arc<Vec<Source>>,
        table_sources: Arc<Vec<TableSource>>,
        game: Arc<Ltx>,
        configs: Arc<ModuleConfigs>,
        root: &Path,
        picture: &atlas::PictureSize,
    ) -> Self {
        let entries: Vec<Entry> = sources.iter().map(|s| s.entry.clone()).collect();
        let lines: Vec<Arc<Lines>> = sources.iter().map(|s| s.lines.clone()).collect();
        // the pictures the build packs: each one that is there and reads
        let mut wanted = Vec::new();
        for e in entries.iter().filter(|e| e.readable && !e.item.is_abstract()) {
            let Some(Icon::Picture { file, w, h, .. }) = &e.item.icon else { continue };
            let asset = root.join(e.item.file.replace('/', "\\"));
            let path = gz_media::files::beside(&asset, file);
            if !gz_media::files::inside(root, &path) || !path.is_file() {
                continue;
            }
            if let Ok(px) = picture(&path) {
                wanted.push(Wanted {
                    file: e.item.file.clone(),
                    section: e.item.game_section(module),
                    picture: path,
                    px,
                    w: *w,
                    h: *h,
                });
            }
        }
        let plan = atlas::plan(module, wanted);
        let items: Vec<Item> = entries.iter().map(|e| e.item.clone()).collect();
        // any malformed part makes the game leave the whole item out
        let malformed: Vec<bool> =
            entries.iter().map(|e| !e.readable || e.issues.iter().any(Issue::is_error)).collect();
        let tables: Vec<TableEntry> = table_sources.iter().map(|s| s.entry.clone()).collect();
        let table_files: Vec<TableFile> = tables.iter().map(|t| t.table.clone()).collect();
        let table_lines: Vec<Arc<Lines>> = table_sources.iter().map(|s| s.lines.clone()).collect();
        // the game takes a table file whole or not at all
        let table_malformed: Vec<bool> =
            tables.iter().map(|t| !t.readable || t.issues.iter().any(Issue::is_error)).collect();
        let composition = compose::compose(
            &game,
            &configs,
            &items,
            &lines,
            &malformed,
            module,
            &plan,
            compose::Tables { files: &table_files, lines: &table_lines, malformed: &table_malformed },
        );
        let mut strings = Vec::new();
        for (e, a) in entries.iter().zip(&composition.applied) {
            if e.readable && !e.item.is_abstract() && !a.section.is_empty() {
                for suffix in ["_name", "_short", "_descr"] {
                    strings.push(format!("{}{suffix}", a.section));
                }
            }
        }
        let mut roster = Self {
            stamp,
            module: module.to_owned(),
            entries,
            tables,
            game,
            composition: Arc::new(composition),
            plan,
            strings,
            sources,
            table_sources,
            configs,
        };
        roster.mark_twins();
        roster.mark_foreign_sections();
        roster
    }

    /// The roster as it would be with the file `file` holding `text`, a file of the roster or a
    /// new one: what a form checks the item it edits against before it is saved. Blocking.
    #[must_use]
    pub fn with_edit(&self, file: &str, text: String, root: &Path, picture: &atlas::PictureSize) -> Self {
        let key = file.replace('\\', "/");
        // the one file is read; every other is as the roster read it
        let one = Source::read(key.clone(), Ok(text));
        let mut sources: Vec<Source> = self.sources.as_ref().clone();
        match sources.iter_mut().find(|s| s.entry.item.file.eq_ignore_ascii_case(&key)) {
            Some(slot) => *slot = one,
            None => {
                sources.push(one);
                sources.sort_by_cached_key(|s| gz_base::text::game_order_key(&s.entry.item.file));
            }
        }
        let (game, configs) = (self.game.clone(), self.configs.clone());
        let tables = self.table_sources.clone();
        Self::assemble(self.stamp, &self.module, Arc::new(sources), tables, game, configs, root, picture)
    }

    /// The roster as it would be with the table file `file` holding `text`, a file of the roster
    /// or a new one: what a table's editor checks before it writes. Blocking.
    #[must_use]
    pub fn with_table_edit(
        &self,
        file: &str,
        text: String,
        root: &Path,
        picture: &atlas::PictureSize,
    ) -> Self {
        let key = file.replace('\\', "/");
        let Some(one) = TableSource::read(key.clone(), Ok(text)) else { return self.clone() };
        let mut tables: Vec<TableSource> = self.table_sources.as_ref().clone();
        match tables.iter_mut().find(|s| s.entry.table.file.eq_ignore_ascii_case(&key)) {
            Some(slot) => *slot = one,
            None => {
                tables.push(one);
                tables.sort_by_cached_key(|s| table_order(&s.entry.table.file));
            }
        }
        let (game, configs) = (self.game.clone(), self.configs.clone());
        let sources = self.sources.clone();
        Self::assemble(self.stamp, &self.module, sources, Arc::new(tables), game, configs, root, picture)
    }

    /// A section has one file: a row of `sections` that makes or changes the section another
    /// item asset of the module is about is a problem of that row.
    fn mark_foreign_sections(&mut self) {
        let mut owner: FxHashMap<String, usize> = FxHashMap::default();
        for (i, (e, a)) in self.entries.iter().zip(&self.composition.applied).enumerate() {
            if e.readable && !a.section.is_empty() {
                owner.entry(a.section.to_ascii_lowercase()).or_insert(i);
            }
        }
        let mut found: Vec<(usize, usize, String, String)> = Vec::new();
        for (i, e) in self.entries.iter().enumerate().filter(|(_, e)| e.readable) {
            for (row, s) in e.item.sections.iter().enumerate() {
                let name = if s.name.is_empty() { &s.edit } else { &s.name }.to_ascii_lowercase();
                if let Some(&j) = owner.get(&name).filter(|&&j| j != i)
                    && let Some(other) = self.entries.get(j)
                {
                    found.push((i, row, name, other.item.file.clone()));
                }
            }
        }
        for (i, row, name, file) in found {
            if let Some(e) = self.entries.get_mut(i) {
                e.issues.push(Issue::error(
                    NOT_YOURS,
                    vec![Seg::key("sections"), Seg::Index(row)],
                    format!(
                        "[{name}] is the item of {file}: what changes it is written there, in its own file"
                    ),
                ));
            }
        }
    }

    /// Two items of one section: a problem on each.
    fn mark_twins(&mut self) {
        let mut by: FxHashMap<String, Vec<usize>> = FxHashMap::default();
        for (i, (e, a)) in self.entries.iter().zip(&self.composition.applied).enumerate() {
            if e.readable && e.item.kind() == Kind::New && !a.section.is_empty() {
                by.entry(a.section.to_ascii_lowercase()).or_default().push(i);
            }
        }
        for (section, list) in by.into_iter().filter(|(_, l)| l.len() > 1) {
            for &i in &list {
                let others: Vec<String> = list
                    .iter()
                    .filter(|&&j| j != i)
                    .filter_map(|&j| self.entries.get(j).map(|e| e.item.file.clone()))
                    .collect();
                let path = match self.entries.get(i) {
                    Some(e) if e.item.section.is_some() => vec![Seg::key("section")],
                    _ => Vec::new(),
                };
                if let Some(e) = self.entries.get_mut(i) {
                    e.issues.push(Issue::error(
                        TWICE,
                        path,
                        format!("another item makes the section '{section}' too: {}", others.join(", ")),
                    ));
                }
            }
        }
    }

    /// The entry of a file (either slash, any case).
    #[must_use]
    pub fn index_of_file(&self, file: &str) -> Option<usize> {
        let key = file.replace('\\', "/");
        self.entries.iter().position(|e| e.item.file.eq_ignore_ascii_case(&key))
    }

    /// The entry of a file, or of the item that makes or changes a section.
    #[must_use]
    pub fn find(&self, item: &str) -> Option<usize> {
        self.index_of_file(item).or_else(|| {
            let lc = item.trim().to_ascii_lowercase();
            self.composition.applied.iter().position(|a| a.section.eq_ignore_ascii_case(&lc))
        })
    }

    /// What the item of an entry resolves to.
    #[must_use]
    pub fn resolution(&self, i: usize, catalog: Option<&Catalog>) -> Option<Resolution> {
        let e = self.entries.get(i)?;
        let a = self.composition.applied.get(i)?;
        Some(resolve(&e.item, a, &self.composition, &self.game, catalog))
    }

    /// The table entry of a file (either slash, any case).
    #[must_use]
    pub fn table_index(&self, file: &str) -> Option<usize> {
        let key = file.replace('\\', "/");
        self.tables.iter().position(|t| t.table.file.eq_ignore_ascii_case(&key))
    }

    /// The items the module's loot tables give a count, lower case.
    #[must_use]
    pub fn counted(&self) -> rustc_hash::FxHashSet<String> {
        let mut out = rustc_hash::FxHashSet::default();
        for t in self.tables.iter().filter(|t| t.readable) {
            if let Rows::Loot(drops) = &t.table.rows {
                out.extend(
                    drops.iter().filter(|d| d.count.is_some()).map(|d| d.item.trim().to_ascii_lowercase()),
                );
            }
        }
        out
    }

    /// Where the item of `section` is found: the rows of the module's traders, loot tables and
    /// recipes that name it, in the files' order.
    #[must_use]
    pub fn found(&self, section: &str) -> Vec<Found> {
        let lc = section.trim().to_ascii_lowercase();
        let is = |s: &str| s.trim().eq_ignore_ascii_case(&lc);
        let pct = |c: f64| format!("{}%", gz_luadata::number_to_text((c * 1000.0).round() / 10.0));
        let num = gz_luadata::number_to_text;
        let mut out = Vec::new();
        if lc.is_empty() {
            return out;
        }
        for t in self.tables.iter().filter(|t| t.readable) {
            let file = &t.table.file;
            match &t.table.rows {
                Rows::Trade { trader, stock } => {
                    for (row, s) in stock.iter().enumerate().filter(|(_, s)| is(&s.item)) {
                        let detail = if s.removes() {
                            OFF_SHELF.to_owned()
                        } else {
                            format!("{}, {}", num(s.count.unwrap_or(1.0)), pct(s.chance.unwrap_or(1.0)))
                        };
                        out.push(Found { how: "sold", file: file.clone(), row, at: trader.clone(), detail });
                    }
                }
                Rows::Loot(drops) => {
                    for (row, d) in drops.iter().enumerate().filter(|(_, d)| is(&d.item)) {
                        let mut parts = Vec::new();
                        match d.count {
                            None => {}
                            Some(Count::One(n)) => parts.push(num(n)),
                            Some(Count::Range(a, b)) => parts.push(format!("{} - {}", num(a), num(b))),
                        }
                        if let Some(c) = d.chance {
                            parts.push(pct(c));
                        }
                        parts.extend(d.by.iter().map(|(who, c)| format!("{who} {}", pct(*c))));
                        out.push(Found {
                            how: "carried",
                            file: file.clone(),
                            row,
                            at: String::new(),
                            detail: parts.join(", "),
                        });
                    }
                }
                Rows::Recipes(list) => {
                    for (row, c) in list.iter().enumerate() {
                        if is(&c.makes) {
                            out.push(Found {
                                how: "made",
                                file: file.clone(),
                                row,
                                at: c.at.clone(),
                                detail: format!("x {}: {}", c.count.unwrap_or(1), c.parts.join(", ")),
                            });
                        } else if c.parts.iter().any(|p| is(p)) {
                            out.push(Found {
                                how: "part",
                                file: file.clone(),
                                row,
                                at: c.makes.clone(),
                                detail: c.at.clone(),
                            });
                        }
                    }
                }
            }
        }
        out
    }

    /// The problems a roster knows of an entry without the game's lookups: its reading, the
    /// composition's, another item of the same section.
    #[must_use]
    pub fn known_issues(&self, i: usize) -> Vec<Issue> {
        let mut out = self.entries.get(i).map(|e| e.issues.clone()).unwrap_or_default();
        if let Some(a) = self.composition.applied.get(i) {
            out.extend(a.issues.iter().cloned());
        }
        out
    }
}
