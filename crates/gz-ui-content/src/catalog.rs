//! A source's listing as one immutable tree ([`Catalog`]), and what a folder of it shows
//! ([`View`]). Both are built on the pools: the project's from its file tree, a Library kind's by
//! [`crate::library`].
//!
//! **One tile per asset.** What belongs to an asset is never a tile of its own: a `.thm`
//! thumbnail, a `.~` backup, an object's `.skl` / `.bones` beside it, the folder of a `.scene`
//! (and the part folder of an X-Ray SDK `.level`). They stay with the asset as its
//! [`FileItem::companions`]: copied, moved, renamed and deleted with it.
//!
//! **A media asset's source is its companion** ([`media_claims`]): the file a model, a texture
//! or a sound asset takes its data from, and what its source names beside it (a glTF's buffers,
//! an OBJ's material libraries, the pictures no texture asset wraps). The media index says what
//! each asset names; an asset it does not know yet takes the source beside it with its own name.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::SmolStr;
use gz_mod::exclude::{ShipState, ship_state};
use gz_project::asset_kind::MediaKind;
use gz_project::{AssetKind, EntryKind, FileTree, is_editor_only_entry};
use gz_ui::Icon;
use gz_vfs::GameFiles;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::icons;
use crate::model::{self, KINDS, Source, search_active};
use crate::select::Entry;

/// Where a file's bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Origin {
    /// A file of the source's folder on the disk ([`Catalog::disk_root`]).
    Disk,
    /// A file of the linked game, under [`Catalog::game_prefix`].
    Game,
    /// No file of its own: a particle effect, a spawn section.
    None,
}

/// A file of a catalog.
#[derive(Clone, Debug)]
pub struct FileItem {
    /// As the source names it, backslashes.
    pub path: SmolStr,
    pub size: u64,
    pub asset: Option<AssetKind>,
    pub origin: Origin,
    /// What the tile says instead of the file's name (a spawn section's in-game name).
    pub label: Option<SmolStr>,
    /// A line the tooltip adds (a spawn section's id and class).
    pub note: Option<SmolStr>,
    /// What goes with it and is never a tile: sidecar files and a scene's folder, as the source
    /// names them.
    pub companions: Vec<SmolStr>,
    /// A media asset's name in the game (`bunker\wall_brick`), from the media index.
    pub published: Option<SmolStr>,
}

impl FileItem {
    fn new(path: &str, size: u64, asset: Option<AssetKind>, origin: Origin) -> Self {
        Self {
            path: SmolStr::new(path),
            size,
            asset,
            origin,
            label: None,
            note: None,
            companions: Vec::new(),
            published: None,
        }
    }
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

/// One source's listing: the project, or one kind of the Library.
#[derive(Debug)]
pub struct Catalog {
    pub source: Source,
    /// The Library's kind; 0 for the project.
    pub category: usize,
    /// Where [`Origin::Disk`] files are on disk.
    pub disk_root: Option<PathBuf>,
    /// The game's index, which reads [`Origin::Game`] files.
    pub game: Option<Arc<GameFiles>>,
    /// The game folder a Library kind's paths are below (`textures`), backslashes.
    pub game_prefix: String,
    nodes: Vec<Node>,
    /// A folder's node by its lower-case path.
    by_path: FxHashMap<String, u32>,
    /// A file by its lower-case path: (node, index in it).
    file_at: FxHashMap<String, (u32, u32)>,
    /// A sentence for what could not be read (the game not linked): shown over what was.
    pub note: Option<String>,
}

fn key(path: &str) -> String {
    path.to_lowercase()
}

/// The extension of a name, lower case, with its dot.
fn ext_lower(name: &str) -> String {
    icons::extension(name).map(str::to_ascii_lowercase).unwrap_or_default()
}

/// A name without its extension.
pub(crate) fn stem(name: &str) -> &str {
    icons::extension(name).map_or(name, |e| &name[..name.len() - e.len()])
}

/// A backup an editor or a tool left (`.~object`, `name.ext~`, `.bak`, `.old`).
#[must_use]
pub fn is_backup(name: &str) -> bool {
    let ext = ext_lower(name);
    name.ends_with('~') || ext.starts_with(".~") || ext == ".bak" || ext == ".old"
}

/// The folders that are part of a scene beside them (`escape\` of `escape.scene`, the part folder
/// of an X-Ray SDK `escape.level`), lower case: never folders of the tree.
fn scene_folders(tree: &FileTree, exts: &[&str]) -> FxHashSet<String> {
    let mut out = FxHashSet::default();
    for dir in tree.dirs() {
        let path = dir.path.to_backslash();
        for e in dir.entries.iter() {
            if matches!(e.kind, EntryKind::File { .. }) && exts.contains(&ext_lower(&e.name).as_str()) {
                let s = stem(&e.name);
                let full = if path.is_empty() { s.to_owned() } else { format!("{path}\\{s}") };
                out.insert(full.to_lowercase());
            }
        }
    }
    out
}

/// Whether `path` is one of `hidden` or lies inside one.
fn inside_hidden(hidden: &FxHashSet<String>, path: &str) -> bool {
    let lower = path.to_lowercase();
    let mut at = lower.as_str();
    loop {
        if hidden.contains(at) {
            return true;
        }
        match at.rfind('\\') {
            Some(i) => at = &at[..i],
            None => return false,
        }
    }
}

/// What a file is beside another of the folder: `Some(owner stem)` when it belongs to an asset
/// named `stem` there - a thumbnail, an object's skeleton files. A `.thm` is always somebody's.
fn sidecar_of(name: &str) -> Option<&str> {
    match ext_lower(name).as_str() {
        ".thm" | ".skl" | ".skls" | ".bones" => Some(stem(name)),
        _ => None,
    }
}

/// The files media assets take their data from, by lower-case backslash path, each with the
/// asset it goes with (backslashes, as the tree spells it); and every asset's published name, by
/// its lower-case path. A file is the companion of the asset whose source it is first, else of
/// one that uses it; a media asset, the asset itself and a file the tree lacks are nobody's. An
/// asset `media` does not list takes the file beside it with its own name and one of its kind's
/// source extensions.
#[must_use]
pub fn media_claims(
    tree: &FileTree,
    media: Option<&gz_media::Index>,
) -> (FxHashMap<String, String>, FxHashMap<String, SmolStr>) {
    let mut on_disk: FxHashMap<String, String> = FxHashMap::default();
    let mut assets: Vec<(String, MediaKind)> = Vec::new();
    for dir in tree.dirs() {
        let path = dir.path.to_backslash();
        for e in dir.entries.iter() {
            if !matches!(e.kind, EntryKind::File { .. }) {
                continue;
            }
            let full = if path.is_empty() { e.name.to_string() } else { format!("{path}\\{}", e.name) };
            if let Some(k) = MediaKind::of_file(&e.name) {
                assets.push((full.clone(), k));
            }
            on_disk.insert(full.to_lowercase(), full);
        }
    }
    let mut claims: FxHashMap<String, String> = FxHashMap::default();
    let mut names: FxHashMap<String, SmolStr> = FxHashMap::default();
    let claim = |file: &str, owner: &str, claims: &mut FxHashMap<String, String>| {
        let key = file.replace('/', "\\").to_lowercase();
        if MediaKind::of_file(&key).is_some() || claims.contains_key(&key) {
            return;
        }
        if let Some(owner) = on_disk.get(&owner.replace('/', "\\").to_lowercase())
            && on_disk.contains_key(&key)
        {
            claims.insert(key, owner.clone());
        }
    };
    let mut known: FxHashSet<String> = FxHashSet::default();
    if let Some(index) = media {
        for e in &index.entries {
            let owner = e.path.to_backslash();
            known.insert(owner.to_lowercase());
            names.insert(owner.to_lowercase(), SmolStr::new(&e.name));
            if let Some(src) = &e.source {
                claim(&src.to_backslash(), &owner, &mut claims);
            }
        }
        for e in &index.entries {
            let owner = e.path.to_backslash();
            for u in &e.uses {
                claim(&u.to_backslash(), &owner, &mut claims);
            }
        }
    }
    for (asset, kind) in assets.iter().filter(|(a, _)| !known.contains(&a.to_lowercase())) {
        let base = stem(asset);
        for ext in gz_media::files::source_extensions(*kind) {
            claim(&format!("{base}.{ext}"), asset, &mut claims);
        }
    }
    (claims, names)
}

impl Catalog {
    /// A catalog of nothing: a source that cannot be read.
    #[must_use]
    pub fn empty(source: Source, category: usize) -> Self {
        let mut c = Self {
            source,
            category,
            disk_root: None,
            game: None,
            game_prefix: String::new(),
            nodes: Vec::new(),
            by_path: FxHashMap::default(),
            file_at: FxHashMap::default(),
            note: None,
        };
        c.node_for("");
        c
    }

    pub(crate) fn node_for(&mut self, path: &str) -> u32 {
        if let Some(&i) = self.by_path.get(&key(path)) {
            return i;
        }
        let parent = if path.is_empty() { None } else { Some(self.node_for(model::parent_of(path))) };
        let i = self.nodes.len() as u32;
        self.nodes.push(Node {
            path: SmolStr::new(path),
            name: SmolStr::new(model::leaf_of(path)),
            ..Node::default()
        });
        self.by_path.insert(key(path), i);
        if let Some(p) = parent.and_then(|p| self.nodes.get_mut(p as usize)) {
            p.children.push(i);
        }
        i
    }

    pub(crate) fn add(&mut self, item: FileItem) {
        let dir = self.node_for(model::parent_of(&item.path));
        if let Some(n) = self.nodes.get_mut(dir as usize) {
            self.file_at.insert(key(&item.path), (dir, n.files.len() as u32));
            n.files.push(item);
        }
    }

    pub(crate) fn sort(&mut self) {
        let names: Vec<String> = self.nodes.iter().map(|n| n.name.to_lowercase()).collect();
        for n in &mut self.nodes {
            n.children.sort_by(|a, b| names.get(*a as usize).cmp(&names.get(*b as usize)));
            n.files.sort_by_cached_key(|f| f.path.to_lowercase());
        }
        self.file_at.clear();
        for (ni, n) in self.nodes.iter().enumerate() {
            for (fi, f) in n.files.iter().enumerate() {
                self.file_at.insert(key(&f.path), (ni as u32, fi as u32));
            }
        }
    }

    /// The project's listing: the editor's own scratch at its root, `mod.ltx` (Mod Settings edits
    /// it) and the project file are no tiles; sidecars, backups and a scene's folder go with their
    /// asset. Links are not listed.
    #[must_use]
    pub fn from_project(tree: &FileTree) -> Self {
        Self::from_project_with(tree, None)
    }

    /// [`Catalog::from_project`] with what the media index says: the sources of the media assets
    /// are their companions, and each asset carries its published name.
    #[must_use]
    pub fn from_project_with(tree: &FileTree, media: Option<&gz_media::Index>) -> Self {
        let mut c = Self::empty(Source::Project, 0);
        c.disk_root = Some(tree.root().to_path_buf());
        let hidden = scene_folders(tree, &[".scene", ".level"]);
        let (claims, published) = media_claims(tree, media);
        let mut media_companions: FxHashMap<String, Vec<SmolStr>> = FxHashMap::default();
        for dir in tree.dirs() {
            let path = dir.path.to_backslash();
            let top = path.split('\\').next().unwrap_or("");
            if !path.is_empty() && (is_editor_only_entry(top) || inside_hidden(&hidden, &path)) {
                continue;
            }
            let join = |name: &str| if path.is_empty() { name.to_owned() } else { format!("{path}\\{name}") };
            // what this folder holds that owns companions: the stems of its assets
            let mut owners: FxHashMap<String, usize> = FxHashMap::default();
            let mut scenes: FxHashSet<String> = FxHashSet::default();
            for e in dir.entries.iter() {
                if let EntryKind::File { .. } = e.kind {
                    let ext = ext_lower(&e.name);
                    if sidecar_of(&e.name).is_none() && !is_backup(&e.name) {
                        owners.insert(stem(&e.name).to_lowercase(), 0);
                    }
                    if ext == ".scene" || ext == ".level" {
                        scenes.insert(stem(&e.name).to_lowercase());
                    }
                }
            }
            if !path.is_empty() {
                c.node_for(&path);
            }
            let mut items: Vec<FileItem> = Vec::new();
            let mut companions: Vec<(String, SmolStr)> = Vec::new();
            for e in dir.entries.iter() {
                let full = join(&e.name);
                match e.kind {
                    EntryKind::Dir => {
                        if path.is_empty() && is_editor_only_entry(&e.name) {
                            continue;
                        }
                        // a scene's folder is part of the scene
                        if scenes.contains(&e.name.to_lowercase()) {
                            companions.push((e.name.to_lowercase(), SmolStr::new(&full)));
                            continue;
                        }
                        c.node_for(&full);
                    }
                    EntryKind::File { size, asset, .. } => {
                        let at_root = path.is_empty();
                        if at_root
                            && (is_editor_only_entry(&e.name)
                                || e.name.eq_ignore_ascii_case("mod.ltx")
                                || asset == Some(AssetKind::Project))
                        {
                            continue;
                        }
                        if let Some(owner) = sidecar_of(&e.name) {
                            let owner = owner.to_lowercase();
                            // a thumbnail is never a tile; a skeleton file is one while it has no object
                            if ext_lower(&e.name) == ".thm" || owners.contains_key(&owner) {
                                companions.push((owner, SmolStr::new(&full)));
                                continue;
                            }
                        }
                        if is_backup(&e.name) {
                            continue;
                        }
                        if let Some(owner) = claims.get(&full.to_lowercase()) {
                            media_companions
                                .entry(owner.to_lowercase())
                                .or_default()
                                .push(SmolStr::new(&full));
                            continue;
                        }
                        items.push(FileItem::new(&full, size, asset, Origin::Disk));
                    }
                    EntryKind::Link => {}
                }
            }
            for mut item in items {
                let own = stem(model::leaf_of(&item.path)).to_lowercase();
                item.companions = companions
                    .iter()
                    .filter(|(o, p)| *o == own && *p != item.path)
                    .map(|(_, p)| p.clone())
                    .collect();
                c.add(item);
            }
        }
        for n in &mut c.nodes {
            for f in &mut n.files {
                let key = f.path.to_lowercase();
                if let Some(more) = media_companions.remove(&key) {
                    f.companions.extend(more);
                }
                f.published = published.get(&key).cloned();
            }
        }
        c.sort();
        c
    }

    /// A Library kind's listing from a folder of the editor's library: only the kind's own files
    /// are entries, everything beside them is a companion or left out.
    #[must_use]
    pub fn from_library_dir(tree: &FileTree, category: usize) -> Self {
        let mut c = Self::empty(Source::Library, category);
        c.disk_root = Some(tree.root().to_path_buf());
        let want = model::kind(category).map_or("", |k| k.ext);
        let hidden = if want == ".level" { scene_folders(tree, &[".level"]) } else { FxHashSet::default() };
        for dir in tree.dirs() {
            let path = dir.path.to_backslash();
            if !path.is_empty() && inside_hidden(&hidden, &path) {
                continue;
            }
            let join = |name: &str| if path.is_empty() { name.to_owned() } else { format!("{path}\\{name}") };
            let scenes: FxHashSet<String> = dir
                .entries
                .iter()
                .filter(|e| matches!(e.kind, EntryKind::File { .. }) && ext_lower(&e.name) == want)
                .map(|e| stem(&e.name).to_lowercase())
                .collect();
            for e in dir.entries.iter() {
                let full = join(&e.name);
                match e.kind {
                    EntryKind::Dir => {
                        // a base scene's part folder is the scene's
                        if !(want == ".level" && scenes.contains(&e.name.to_lowercase())) {
                            c.node_for(&full);
                        }
                    }
                    EntryKind::File { size, .. } if ext_lower(&e.name) == want => {
                        let mut item = FileItem::new(&full, size, None, Origin::Disk);
                        let s = stem(&e.name);
                        item.companions = dir
                            .entries
                            .iter()
                            .filter(|o| !std::ptr::eq(*o, e))
                            .filter(|o| match o.kind {
                                EntryKind::File { .. } => {
                                    sidecar_of(&o.name).is_some_and(|st| st.eq_ignore_ascii_case(s))
                                }
                                EntryKind::Dir => want == ".level" && o.name.eq_ignore_ascii_case(s),
                                EntryKind::Link => false,
                            })
                            .map(|o| SmolStr::new(join(&o.name)))
                            .collect();
                        c.add(item);
                    }
                    _ => {}
                }
            }
        }
        c.sort();
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

    /// A file named by its reference: the path, or the path without its extension.
    #[must_use]
    pub fn find_reference(&self, name: &str) -> Option<&FileItem> {
        if let Some(f) = self.file(name) {
            return Some(f);
        }
        let want = key(name);
        self.nodes.iter().flat_map(|n| n.files.iter()).find(|f| key(stem(&f.path)) == want)
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

    /// Every file of the catalog.
    pub fn files(&self) -> impl Iterator<Item = &FileItem> {
        self.nodes.iter().flat_map(|n| n.files.iter())
    }

    /// Whether an entry is still there.
    #[must_use]
    pub fn contains(&self, e: &Entry) -> bool {
        if e.folder { self.folder(&e.path).is_some() } else { self.file(&e.path).is_some() }
    }

    /// Where an entry is on disk: a folder, or a file of [`Origin::Disk`].
    #[must_use]
    pub fn disk_path(&self, path: &str) -> Option<PathBuf> {
        let root = self.disk_root.as_ref()?;
        if let Some(f) = self.file(path)
            && f.origin != Origin::Disk
        {
            return None;
        }
        Some(if path.is_empty() { root.clone() } else { root.join(path) })
    }

    /// The game path of an [`Origin::Game`] file, `gamedata`-relative.
    #[must_use]
    pub fn game_path(&self, path: &str) -> Option<String> {
        let f = self.file(path)?;
        (f.origin == Origin::Game).then(|| {
            if self.game_prefix.is_empty() {
                path.to_owned()
            } else {
                format!("{}\\{path}", self.game_prefix)
            }
        })
    }

    #[must_use]
    pub fn folder_count(&self) -> usize {
        self.nodes.len()
    }

    #[must_use]
    pub fn file_count(&self) -> usize {
        self.file_at.len()
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
    /// A model: its `.thm` thumbnail when it has one, rendered otherwise.
    Model,
    /// A group's `.thm` thumbnail.
    Thumb,
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
    /// What the tile says instead of the name.
    pub label: Option<SmolStr>,
    /// A line for the tooltip.
    pub note: Option<SmolStr>,
    /// A Library asset a scene of the project uses that the project does not hold.
    pub wanted: bool,
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
    let ext = ext_lower(path);
    if icons::is_image(path) {
        Picture::Image
    } else if ext == ".object" || ext == ".ogf" {
        Picture::Model
    } else if ext == ".group" && (source == Source::Project || category == model::GROUPS) {
        Picture::Thumb
    } else {
        Picture::None
    }
}

/// What a folder view knows besides the catalog: the module's `[exclude]` (the project) and the
/// library references the project's scenes take from the library (the Library).
#[derive(Clone, Copy, Default)]
pub struct ViewInput<'a> {
    pub rules: Option<&'a [String]>,
    pub wanted: Option<&'a FxHashSet<String>>,
}

/// The Library's top: its kinds as folders, filtered by `search`.
#[must_use]
pub fn home_view(search: &str) -> View {
    let mut view = View::default();
    for k in KINDS {
        if search_active(search) && !gz_ui::search::matches(search, k.caption) {
            continue;
        }
        view.entries.push(ViewEntry {
            entry: Entry::new(k.caption, true),
            glyph: k.icon,
            own: None,
            ship: Ship::Shipped,
            size: 0,
            picture: Picture::None,
            label: None,
            note: None,
            wanted: false,
        });
    }
    view.folders = view.entries.len();
    view
}

/// The entries of `folder`, filtered by `search`.
#[must_use]
pub fn build_view(cat: &Catalog, folder: &str, search: &str, input: ViewInput<'_>) -> View {
    let mut view = View::default();
    let Some(cur) = cat.folder(folder) else {
        view.missing = true;
        return view;
    };
    let searching = search_active(search);
    let below = if cur.path.is_empty() { 0 } else { cur.path.len() + 1 };
    let passes = |path: &str, label: Option<&SmolStr>| {
        !searching
            || gz_ui::search::matches(search, path.get(below..).unwrap_or(path))
            || label.is_some_and(|l| gz_ui::search::matches(search, l))
    };
    let ship = |path: &str| input.rules.map_or(Ship::Shipped, |r| Ship::of(r, path));
    let folder_entry = |n: &Node| ViewEntry {
        entry: Entry::new(&n.path, true),
        glyph: Icon::Folder,
        own: None,
        ship: ship(&n.path),
        size: 0,
        picture: Picture::None,
        label: None,
        note: None,
        wanted: false,
    };
    let lib_icon = model::kind(cat.category).filter(|_| cat.source == Source::Library).map(|k| k.icon);
    let file_entry = |f: &FileItem| ViewEntry {
        entry: Entry::new(&f.path, false),
        glyph: icons::tile_icon(&f.path, false, f.asset, lib_icon),
        own: f.asset,
        ship: ship(&f.path),
        size: f.size,
        picture: match f.asset {
            // a texture asset shows its picture, a model asset its model rendered
            Some(AssetKind::Texture) => Picture::Image,
            Some(AssetKind::Model) => Picture::Model,
            Some(_) => Picture::None,
            None if f.origin == Origin::None => Picture::None,
            None => picture_of(cat.source, cat.category, &f.path),
        },
        label: f.label.clone(),
        note: f.note.clone(),
        wanted: cat.source == Source::Library
            && input.wanted.is_some_and(|w| w.contains(&stem(&f.path).to_lowercase())),
    };
    if searching {
        // depth first, in tree order
        let mut stack: Vec<u32> = cur.children.iter().rev().copied().collect();
        while let Some(i) = stack.pop() {
            let Some(n) = cat.node(i) else { continue };
            if passes(&n.path, None) {
                view.entries.push(folder_entry(n));
            }
            stack.extend(n.children.iter().rev().copied());
        }
        view.folders = view.entries.len();
        for f in cat.files_below(&cur.path) {
            if passes(&f.path, f.label.as_ref()) {
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

/// A catalog with its revision, shared with the frames.
pub type Shared = Arc<Catalog>;

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Catalog {
        let mut c = Catalog::empty(Source::Library, model::TEXTURES);
        for p in ["ui\\cursor.dds", "ui\\ui_icons.dds", "water.dds"] {
            c.add(FileItem::new(p, 10, None, Origin::Game));
        }
        c.node_for("empty\\deep");
        c.sort();
        c
    }

    #[test]
    fn folders_come_first_then_files() {
        let c = sample();
        let v = build_view(&c, "", "", ViewInput::default());
        assert_eq!(v.folders, 2);
        assert_eq!(v.files, 1);
        assert_eq!(v.entries.first().map(|e| e.entry.path.as_str()), Some("empty"));
        assert!(c.folder("EMPTY\\deep").is_some());
        assert!(build_view(&c, "nope", "", ViewInput::default()).missing);
    }

    #[test]
    fn a_search_looks_below_the_folder() {
        let c = sample();
        let v = build_view(&c, "ui", "cursor", ViewInput::default());
        assert_eq!(v.entries.len(), 1);
        assert_eq!(v.entries[0].entry.path, "ui\\cursor.dds");
        assert_eq!(v.entries[0].picture, Picture::Image);
        // the folder's own name does not count: the path under it does
        assert!(build_view(&c, "ui", "ui cursor", ViewInput::default()).entries.is_empty());
        assert_eq!(c.files_below("").len(), 3);
    }

    #[test]
    fn references_match_without_the_extension() {
        let mut c = Catalog::empty(Source::Library, model::OBJECTS);
        c.add(FileItem::new("box\\box_1a.object", 1, None, Origin::Disk));
        assert!(c.find_reference("box\\BOX_1A").is_some());
        assert!(c.find_reference("box\\box_1a.object").is_some());
        assert!(c.find_reference("box").is_none());
    }

    #[test]
    fn excluded_entries_are_veiled() {
        let mut c = Catalog::empty(Source::Project, 0);
        c.add(FileItem::new("config\\a.ltx", 1, None, Origin::Disk));
        c.sort();
        let rules = vec!["config".to_owned()];
        let v = build_view(&c, "", "", ViewInput { rules: Some(&rules), wanted: None });
        let config = v.entries.iter().find(|e| e.entry.path == "config");
        assert!(config.is_some_and(|e| e.ship.veiled()));
        assert!(veiled_folders(&c, &rules).contains("config"));
    }

    fn put(root: &std::path::Path, rel: &str) {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn an_object_and_its_thumbnail_are_one_tile() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        for f in [
            "rawdata\\objects\\corps\\corp_dog.object",
            "rawdata\\objects\\corps\\corp_dog.thm",
            "rawdata\\objects\\corps\\corp_dog.skl",
            "rawdata\\objects\\corps\\corp_dog.~object",
            "rawdata\\objects\\equipment\\only.thm",
            "levels\\escape.scene",
            "levels\\escape\\objects.lua",
            "levels\\old.level",
            "levels\\old\\part.part",
            "notes.txt",
            "Mine.grizzly",
        ] {
            put(root, f);
        }
        let c = Catalog::from_project(&FileTree::scan(root));
        let corps = build_view(&c, "rawdata\\objects\\corps", "", ViewInput::default());
        let names: Vec<&str> = corps.entries.iter().map(|e| e.entry.path.as_str()).collect();
        assert_eq!(names, ["rawdata\\objects\\corps\\corp_dog.object"]);
        let dog = c.file("rawdata\\objects\\corps\\corp_dog.object").unwrap();
        assert_eq!(dog.companions.len(), 2, "{:?}", dog.companions);
        assert_eq!(corps.entries[0].picture, Picture::Model);
        // a folder of thumbnails alone is empty
        assert!(build_view(&c, "rawdata\\objects\\equipment", "", ViewInput::default()).entries.is_empty());
        // a scene is one tile, its folder goes with it
        let levels = build_view(&c, "levels", "", ViewInput::default());
        let names: Vec<&str> = levels.entries.iter().map(|e| e.entry.path.as_str()).collect();
        assert_eq!(names, ["levels\\escape.scene", "levels\\old.level"]);
        assert_eq!(c.file("levels\\escape.scene").unwrap().companions, ["levels\\escape"]);
        // the project file is the project, not a tile
        let root_view = build_view(&c, "", "", ViewInput::default());
        assert!(root_view.entries.iter().all(|e| !e.entry.path.ends_with(".grizzly")));
    }

    fn media_entry(
        path: &str,
        kind: MediaKind,
        name: &str,
        source: Option<&str>,
        uses: &[&str],
    ) -> gz_media::Entry {
        let rel = |p: &str| gz_base::RelPath::new(p).unwrap();
        gz_media::Entry {
            kind,
            path: rel(path),
            name: name.to_owned(),
            key: name.to_lowercase(),
            source: source.map(rel),
            uses: uses.iter().map(|u| rel(u)).collect(),
            dynamic: false,
            parts: false,
            excluded: false,
            error: None,
        }
    }

    #[test]
    fn a_media_asset_and_what_it_takes_its_data_from_are_one_tile() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        for f in [
            "models\\wall.meshasset",
            "models\\wall.glb",
            "models\\wall.png",
            "models\\crate.meshasset",
            "models\\crate.gltf",
            "models\\crate.bin",
            "models\\tex\\crate_d.png",
            "models\\tex\\crate_d.texasset",
            "models\\tex\\crate_n.png",
            "sounds\\hum.sndasset",
            "sounds\\hum_take2.wav",
            "sounds\\hum.wav",
            "notes.png",
        ] {
            put(root, f);
        }
        let tree = FileTree::scan(root);
        // without the index: an asset takes the source beside it with its own name and its kind's extension
        let c = Catalog::from_project(&tree);
        let wall = c.file("models\\wall.meshasset").unwrap();
        assert_eq!(wall.companions, ["models\\wall.glb"]);
        assert_eq!(wall.asset, Some(AssetKind::Model));
        // a picture beside a model is no source of the model's kind: a tile of its own
        assert!(c.file("models\\wall.png").is_some());
        assert!(c.file("models\\wall.glb").is_none());
        assert_eq!(c.file("sounds\\hum.sndasset").unwrap().companions, ["sounds\\hum.wav"]);
        assert!(c.file("sounds\\hum_take2.wav").is_some());
        // with it: the source the asset names, what its source uses, the pictures no texture asset wraps
        let index = gz_media::Index {
            stamp: gz_base::Rev(1),
            module: "qa".to_owned(),
            entries: vec![
                media_entry(
                    "models/crate.meshasset",
                    MediaKind::Model,
                    "qa\\crate",
                    Some("models/crate.gltf"),
                    &["models/crate.bin", "models/tex/crate_d.png", "models/tex/crate_n.png"],
                ),
                media_entry(
                    "models/tex/crate_d.texasset",
                    MediaKind::Texture,
                    "qa\\crate_d",
                    Some("models/tex/CRATE_D.png"),
                    &[],
                ),
                media_entry(
                    "sounds/hum.sndasset",
                    MediaKind::Sound,
                    "ambient\\hum",
                    Some("sounds/hum_take2.wav"),
                    &[],
                ),
                // a source that is another asset is nobody's companion
                media_entry(
                    "models/wall.meshasset",
                    MediaKind::Model,
                    "qa\\wall",
                    Some("models/crate.meshasset"),
                    &[],
                ),
            ],
        };
        let c = Catalog::from_project_with(&tree, Some(&index));
        let mut crate_ = c.file("models\\crate.meshasset").unwrap().companions.clone();
        crate_.sort();
        assert_eq!(crate_, ["models\\crate.bin", "models\\crate.gltf", "models\\tex\\crate_n.png"]);
        let tex = c.file("models\\tex\\crate_d.texasset").unwrap();
        assert_eq!(tex.companions, ["models\\tex\\crate_d.png"], "the tree's own spelling");
        assert_eq!(tex.published.as_deref(), Some("qa\\crate_d"));
        assert_eq!(c.file("sounds\\hum.sndasset").unwrap().companions, ["sounds\\hum_take2.wav"]);
        // the index knows hum.sndasset names another file: the one with its name is a tile
        assert!(c.file("sounds\\hum.wav").is_some());
        assert!(
            c.file("models\\crate.meshasset").is_some()
                && c.file("models\\wall.meshasset").unwrap().companions.is_empty()
        );
        assert!(c.file("notes.png").is_some());
        let tiles = build_view(&c, "models\\tex", "", ViewInput::default());
        let names: Vec<&str> = tiles.entries.iter().map(|e| e.entry.path.as_str()).collect();
        assert_eq!(names, ["models\\tex\\crate_d.texasset"]);
        assert_eq!(tiles.entries[0].picture, Picture::Image);
        let models = build_view(&c, "models", "", ViewInput::default());
        let crate_tile = models.entries.iter().find(|e| e.entry.path.ends_with("crate.meshasset")).unwrap();
        assert_eq!(crate_tile.picture, Picture::Model);
        let sounds = build_view(&c, "sounds", "", ViewInput::default());
        assert!(sounds.entries.iter().all(|e| e.picture == Picture::None));
    }

    #[test]
    fn a_library_kind_lists_its_own_files_only() {
        let t = tempfile::tempdir().unwrap();
        let root = t.path();
        for f in ["corps\\corp_dog.object", "corps\\corp_dog.thm", "corps\\readme.txt", "equipment\\a.thm"] {
            put(root, f);
        }
        let c = Catalog::from_library_dir(&FileTree::scan(root), model::OBJECTS);
        assert_eq!(c.file_count(), 1);
        assert_eq!(c.file("corps\\corp_dog.object").unwrap().companions, ["corps\\corp_dog.thm"]);
        let wanted: FxHashSet<String> = ["corps\\corp_dog".to_owned()].into_iter().collect();
        let v = build_view(&c, "corps", "", ViewInput { rules: None, wanted: Some(&wanted) });
        assert!(v.entries[0].wanted);
        assert!(build_view(&c, "equipment", "", ViewInput::default()).entries.is_empty());
    }
}
