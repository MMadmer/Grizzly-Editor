//! The project's items as one [`Roster`]: every file read and parsed, the module laid over the
//! game's configs ([`crate::compose`]), the icon atlas planned - once per state of the files,
//! the manifest and the game.

use std::path::Path;
use std::sync::Arc;

use gz_base::Rev;
use gz_ltx::Ltx;
use gz_luadata::Seg;
use rustc_hash::FxHashMap;

use crate::atlas::{self, Plan, Wanted};
use crate::catalog::Catalog;
use crate::compose::{self, Composition, Lines, ModuleConfigs};
use crate::model::{Icon, Issue, Item, Kind, load_text};
use crate::resolve::{Resolution, resolve};
use crate::validate::TWICE;

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

/// Every item of the project, read, composed and planned.
#[derive(Clone, Debug, Default)]
pub struct Roster {
    /// The state of the files, the manifest and the game it was made from.
    pub stamp: Rev,
    pub module: String,
    /// In path order.
    pub entries: Vec<Entry>,
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
    configs: Arc<ModuleConfigs>,
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
    /// The roster of these files: `(file, text or the read's error)`, in path order.
    #[must_use]
    pub fn build(
        stamp: Rev,
        module: &str,
        files: Vec<(String, Result<String, String>)>,
        game: Arc<Ltx>,
        configs: &ModuleConfigs,
        root: &Path,
        picture: &atlas::PictureSize,
    ) -> Self {
        let sources: Vec<Source> = files.into_iter().map(|(file, text)| Source::read(file, text)).collect();
        Self::assemble(stamp, module, Arc::new(sources), game, Arc::new(configs.clone()), root, picture)
    }

    /// The roster of files that are read: composed and planned.
    fn assemble(
        stamp: Rev,
        module: &str,
        sources: Arc<Vec<Source>>,
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
        let composition = compose::compose(&game, &configs, &items, &lines, &malformed, module, &plan);
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
            game,
            composition: Arc::new(composition),
            plan,
            strings,
            sources,
            configs,
        };
        roster.mark_twins();
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
        Self::assemble(self.stamp, &self.module, Arc::new(sources), game, configs, root, picture)
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
