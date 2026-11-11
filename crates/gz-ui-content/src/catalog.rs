//! A source's listing as one immutable tree ([`Catalog`]), and what a folder of it shows
//! ([`View`]). Both are built on the pools from snapshots: the project's file tree, the game's
//! index, a walk of an Editor category's folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::SmolStr;
use gz_mod::exclude::{ShipState, ship_state};
use gz_project::{AssetKind, EntryKind, FileTree, is_editor_only_entry};
use gz_ui::Icon;
use gz_vfs::GameFiles;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::icons;
use crate::model::{Source, search_active};
use crate::select::Entry;

/// A file of a catalog.
#[derive(Clone, Debug)]
pub struct FileItem {
    /// As the source names it, backslashes.
    pub path: SmolStr,
    pub size: u64,
    pub asset: Option<AssetKind>,
}

/// A folder of a catalog.
#[derive(Clone, Debug, Default)]
pub struct Node {
    /// `""` for the root.
    pub path: SmolStr,
    pub name: SmolStr,
    /// Sub-folders, by name.
    pub children: Vec<u32>,
    pub files: Vec<FileItem>,
}

/// One source's listing.
#[derive(Debug)]
pub struct Catalog {
    pub source: Source,
    pub category: usize,
    /// Where the source's files are on disk; `None` for the game (its files are read through
    /// its index).
    pub disk_root: Option<PathBuf>,
    /// The game's index, which reads the game's files.
    pub game: Option<Arc<GameFiles>>,
    nodes: Vec<Node>,
    /// A folder's node by its lower-case path.
    by_path: FxHashMap<String, u32>,
    /// A file by its lower-case path: (node, index in it).
    file_at: FxHashMap<String, (u32, u32)>,
}

fn key(path: &str) -> String {
    path.to_lowercase()
}

impl Catalog {
    /// A catalog of nothing: a source that cannot be read, a category without a folder.
    #[must_use]
    pub fn empty(source: Source, category: usize) -> Self {
        let mut c = Self {
            source,
            category,
            disk_root: None,
            game: None,
            nodes: Vec::new(),
            by_path: FxHashMap::default(),
            file_at: FxHashMap::default(),
        };
        c.node_for("");
        c
    }

    fn node_for(&mut self, path: &str) -> u32 {
        if let Some(&i) = self.by_path.get(&key(path)) {
            return i;
        }
        let parent = if path.is_empty() { None } else { Some(self.node_for(crate::model::parent_of(path))) };
        let i = self.nodes.len() as u32;
        self.nodes.push(Node {
            path: SmolStr::new(path),
            name: SmolStr::new(crate::model::leaf_of(path)),
            ..Node::default()
        });
        self.by_path.insert(key(path), i);
        if let Some(p) = parent.and_then(|p| self.nodes.get_mut(p as usize)) {
            p.children.push(i);
        }
        i
    }

    fn add_file(&mut self, path: &str, size: u64, asset: Option<AssetKind>) {
        let dir = self.node_for(crate::model::parent_of(path));
        if let Some(n) = self.nodes.get_mut(dir as usize) {
            self.file_at.insert(key(path), (dir, n.files.len() as u32));
            n.files.push(FileItem { path: SmolStr::new(path), size, asset });
        }
    }

    fn sort(&mut self) {
        let names: Vec<String> = self.nodes.iter().map(|n| n.name.to_lowercase()).collect();
        for n in &mut self.nodes {
            n.children.sort_by(|a, b| names.get(*a as usize).cmp(&names.get(*b as usize)));
        }
    }

    /// From a file tree: the project's (`project` hides the editor's own scratch at its root and
    /// `mod.ltx`, which Mod Settings edits) or an Editor category's. Links are not listed.
    #[must_use]
    pub fn from_tree(tree: &FileTree, source: Source, category: usize, project: bool) -> Self {
        let mut c = Self::empty(source, category);
        c.disk_root = Some(tree.root().to_path_buf());
        for dir in tree.dirs() {
            let path = dir.path.to_backslash();
            let top = path.split('\\').next().unwrap_or("");
            if project && !path.is_empty() && is_editor_only_entry(top) {
                continue;
            }
            c.node_for(&path);
            for e in dir.entries.iter() {
                let full = if path.is_empty() { e.name.to_string() } else { format!("{path}\\{}", e.name) };
                match e.kind {
                    EntryKind::Dir => {
                        if project && path.is_empty() && is_editor_only_entry(&e.name) {
                            continue;
                        }
                        c.node_for(&full);
                    }
                    EntryKind::File { size, asset, .. } => {
                        if project
                            && path.is_empty()
                            && (is_editor_only_entry(&e.name) || e.name.eq_ignore_ascii_case("mod.ltx"))
                        {
                            continue;
                        }
                        c.add_file(&full, size, asset);
                    }
                    EntryKind::Link => {}
                }
            }
        }
        c.sort();
        c
    }

    /// From the linked game's index: every file of it, `gamedata`-relative.
    #[must_use]
    pub fn from_game(files: Arc<GameFiles>) -> Self {
        let mut c = Self::empty(Source::Game, 0);
        for f in files.iter() {
            let path = f.path.replace('/', "\\");
            let asset = gz_project::kind_of(&path, None);
            c.add_file(&path, f.size, asset);
        }
        c.sort();
        c.game = Some(files);
        c
    }

    #[must_use]
    pub fn root(&self) -> &Node {
        &self.nodes[0]
    }

    #[must_use]
    pub fn node(&self, i: u32) -> Option<&Node> {
        self.nodes.get(i as usize)
    }

    #[must_use]
    pub fn folder(&self, path: &str) -> Option<&Node> {
        self.by_path.get(&key(path)).and_then(|&i| self.nodes.get(i as usize))
    }

    #[must_use]
    pub fn file(&self, path: &str) -> Option<&FileItem> {
        let &(n, i) = self.file_at.get(&key(path))?;
        self.nodes.get(n as usize)?.files.get(i as usize)
    }

    /// A file named as an Editor library reference: the path, or the path without its extension.
    #[must_use]
    pub fn find_reference(&self, name: &str) -> Option<&FileItem> {
        if let Some(f) = self.file(name) {
            return Some(f);
        }
        let want = key(name);
        self.nodes.iter().flat_map(|n| n.files.iter()).find(|f| {
            let p = f.path.as_str();
            let stem = icons::extension(p).map_or(p, |e| &p[..p.len() - e.len()]);
            key(stem) == want
        })
    }

    /// Every file at or below a folder.
    #[must_use]
    pub fn files_below(&self, folder: &str) -> Vec<&FileItem> {
        let mut out = Vec::new();
        let Some(&start) = self.by_path.get(&key(folder)) else { return out };
        let mut stack = vec![start];
        while let Some(i) = stack.pop() {
            let Some(n) = self.nodes.get(i as usize) else { continue };
            out.extend(n.files.iter());
            stack.extend(n.children.iter().rev().copied());
        }
        out
    }

    /// Whether an entry is still there.
    #[must_use]
    pub fn contains(&self, e: &Entry) -> bool {
        if e.folder { self.folder(&e.path).is_some() } else { self.file(&e.path).is_some() }
    }

    /// Where an entry is on disk.
    #[must_use]
    pub fn disk_path(&self, path: &str) -> Option<PathBuf> {
        let root = self.disk_root.as_ref()?;
        Some(if path.is_empty() { root.clone() } else { root.join(path) })
    }

    #[must_use]
    pub fn folder_count(&self) -> usize {
        self.nodes.len()
    }
}

/// What the build does with an entry, as a tile shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ship {
    Shipped,
    /// `[exclude]` takes it out: by this rule.
    Excluded(SmolStr),
    /// A source or the editor's own: never shipped.
    Never,
}

impl Ship {
    #[must_use]
    pub fn veiled(&self) -> bool {
        matches!(self, Ship::Excluded(_))
    }

    #[must_use]
    pub fn of(rules: &[String], rel: &str) -> Ship {
        match ship_state(rules, rel) {
            ShipState::Shipped => Ship::Shipped,
            ShipState::Excluded => Ship::Excluded(SmolStr::new(rel.replace('\\', "/"))),
            ShipState::ExcludedByRule(rule) => Ship::Excluded(SmolStr::new(rule)),
            ShipState::Never => Ship::Never,
        }
    }
}

/// What a picture of an entry is made from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Picture {
    None,
    Image,
    Model,
}

/// An entry as the grid draws it.
#[derive(Clone, Debug)]
pub struct ViewEntry {
    pub entry: Entry,
    pub glyph: Icon,
    pub own: Option<AssetKind>,
    pub ship: Ship,
    pub size: u64,
    pub picture: Picture,
}

/// A folder's entries: the sub-folders, then the files; with a search, everything below that
/// matches its path under the folder.
#[derive(Debug, Default)]
pub struct View {
    pub entries: Vec<ViewEntry>,
    pub folders: usize,
    pub files: usize,
    /// The folder shown is not in the source (any more).
    pub missing: bool,
}

impl View {
    /// The complete view order, for ranges and Select All.
    #[must_use]
    pub fn order(&self) -> Vec<Entry> {
        self.entries.iter().map(|e| e.entry.clone()).collect()
    }

    #[must_use]
    pub fn find(&self, e: &Entry) -> Option<usize> {
        self.entries.iter().position(|x| &x.entry == e)
    }
}

fn picture_of(source: Source, category: usize, path: &str) -> Picture {
    if icons::is_image(path) {
        Picture::Image
    } else if icons::is_model(path)
        || (source == Source::Editor && category == crate::model::VISUALS && icons::extension(path).is_none())
    {
        Picture::Model
    } else {
        Picture::None
    }
}

/// The entries of `folder`, filtered by `search`. `rules` is the module's `[exclude]` for the
/// project source, `None` elsewhere.
#[must_use]
pub fn build_view(cat: &Catalog, folder: &str, search: &str, rules: Option<&[String]>) -> View {
    let mut view = View::default();
    let Some(cur) = cat.folder(folder) else {
        view.missing = true;
        return view;
    };
    let searching = search_active(search);
    let below = if cur.path.is_empty() { 0 } else { cur.path.len() + 1 };
    let passes = |path: &str| !searching || gz_ui::search::matches(search, path.get(below..).unwrap_or(path));
    let ship = |path: &str| rules.map_or(Ship::Shipped, |r| Ship::of(r, path));
    let folder_entry = |n: &Node| ViewEntry {
        entry: Entry::new(&n.path, true),
        glyph: Icon::Folder,
        own: None,
        ship: ship(&n.path),
        size: 0,
        picture: Picture::None,
    };
    let file_entry = |f: &FileItem| ViewEntry {
        entry: Entry::new(&f.path, false),
        glyph: icons::tile_icon(&f.path, false, f.asset, cat.source, cat.category),
        own: f.asset,
        ship: ship(&f.path),
        size: f.size,
        picture: if f.asset.is_some() {
            Picture::None
        } else {
            picture_of(cat.source, cat.category, &f.path)
        },
    };
    if searching {
        // depth first, in tree order
        let mut stack: Vec<u32> = cur.children.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            let Some(n) = cat.node(i) else { continue };
            if passes(&n.path) {
                view.entries.push(folder_entry(n));
            }
            stack.extend(n.children.iter().rev().copied());
        }
        view.folders = view.entries.len();
        for f in cat.files_below(&cur.path) {
            if passes(&f.path) {
                view.entries.push(file_entry(f));
            }
        }
    } else {
        view.entries.extend(cur.children.iter().filter_map(|&i| cat.node(i)).map(folder_entry));
        view.folders = view.entries.len();
        view.entries.extend(cur.files.iter().map(file_entry));
    }
    view.files = view.entries.len() - view.folders;
    view
}

/// The folders `[exclude]` takes out, by lower-case path: the tree veils them.
#[must_use]
pub fn veiled_folders(cat: &Catalog, rules: &[String]) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    if rules.is_empty() {
        return out;
    }
    for n in cat.nodes.iter().skip(1) {
        if Ship::of(rules, &n.path).veiled() {
            out.insert(key(&n.path));
        }
    }
    out
}

/// The disk folder of an Editor category, when it has one.
#[must_use]
pub fn editor_folder(root: &Path, category: usize) -> Option<PathBuf> {
    crate::model::category(category).folder.map(|f| root.join(f))
}

/// A catalog with its revision, shared with the frames.
pub type Shared = Arc<Catalog>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Catalog {
        let mut c = Catalog::empty(Source::Game, 0);
        c.add_file("textures\\ui\\cursor.dds", 10, None);
        c.add_file("textures\\ui\\ui_icons.dds", 10, None);
        c.add_file("textures\\water.dds", 10, None);
        c.add_file("config\\system.ltx", 10, None);
        c.node_for("empty\\deep");
        c.sort();
        c
    }

    #[test]
    fn folders_come_first_then_files() {
        let c = sample();
        let v = build_view(&c, "textures", "", None);
        assert_eq!(v.folders, 1);
        assert_eq!(v.files, 1);
        assert_eq!(v.entries.first().map(|e| e.entry.path.as_str()), Some("textures\\ui"));
        assert!(c.folder("EMPTY\\deep").is_some());
        assert_eq!(c.root().children.len(), 3);
        assert!(build_view(&c, "nope", "", None).missing);
    }

    #[test]
    fn a_search_looks_below_the_folder() {
        let c = sample();
        let v = build_view(&c, "textures", "ui cursor", None);
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.entries[0].entry.path, "textures\\ui\\cursor.dds");
        assert_eq!(v.entries[0].picture, Picture::Image);
        // the folder's own name does not count: the path under it does
        assert!(build_view(&c, "textures", "textures", None).entries.is_empty());
        assert_eq!(c.files_below("textures").len(), 3);
    }

    #[test]
    fn references_match_without_the_extension() {
        let mut c = Catalog::empty(Source::Editor, 0);
        c.add_file("box\\box_1a.object", 1, None);
        assert!(c.find_reference("box\\BOX_1A").is_some());
        assert!(c.find_reference("box\\box_1a.object").is_some());
        assert!(c.find_reference("box").is_none());
    }

    #[test]
    fn excluded_entries_are_veiled() {
        let c = sample();
        let rules = vec!["config".to_owned()];
        let v = build_view(&c, "", "", Some(&rules));
        let config = v.entries.iter().find(|e| e.entry.path == "config");
        assert!(config.is_some_and(|e| e.ship.veiled()));
        assert!(veiled_folders(&c, &rules).contains("config"));
    }
}
