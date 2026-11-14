//! The Place panel: everything a level can be given, in one list with one search.
//!
//! Recent (the last twelve placed), then the categories - Objects (the library's tree, with
//! the objects' pictures), Lights (a light, a glow), Spawn Points (Player Start and the special
//! points, then the game's sections by the names the player reads, grouped as the game groups
//! them), Shapes and Volumes (sphere, box, the AI Map Bounds volume, fog, sound environment),
//! Sounds, Particles, Way Paths and Groups. A row is dragged into the viewport (the kit's
//! payload, [`payload`]) or clicked to arm it: Place mode then places it at each click until
//! Esc. A click on the armed row stops placing.
//!
//! The rows are worked out off the UI thread: a `Query` keyed by the search, the open folders
//! and Recent, its revision the lists it reads ([`rows`] is the pure function). A frame lays out
//! only the rows on screen and never reads a file; the pictures are read and decoded on the
//! pools and reach the GPU a few a frame.
//!
//! **What crosses to the viewport** is the viewport's own `Placeable`
//! (`gz_ui_viewport::place`): a dragged row is its payload (`Payload::Row { surface: "place", key:
//! "<kind>[:<reference>]" }`), a click arms it (`place::arm`). The one row Place mode cannot
//! arm - the AI Map Bounds volume, a box with a use of its own - is placed at once, on the ground
//! in front of the camera, and selected.

use std::sync::Arc;

use egui::{
    Align2, ColorImage, CornerRadius, Id, Pos2, Rect, Sense, TextureHandle, TextureOptions, Ui, Vec2, pos2,
};
use gz_base::Rev;
use gz_editor::{Editor, Value, json};
use gz_query::Query;
use gz_scene::SceneService;
use gz_scene::model::SceneMode;
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, SearchField, Section, icons, metrics, palette, state};
use gz_ui_viewport::place::{Kind, Placeable};
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::lists::{ListKey, Lists};

/// The drag payload's surface.
pub const SURFACE: &str = "place";
/// The layout store's list of what was placed last, newest first.
const RECENT_KEY: &str = "place.recent";
/// Recent keeps this many.
pub const RECENT: usize = 12;
const ROW_H: f32 = metrics::ROW;
const HEAD_H: f32 = metrics::SECTION_HEADER;
const INDENT: f32 = 12.0;
const THUMB: f32 = 18.0;
/// Pictures uploaded in one frame at most.
const UPLOADS_PER_FRAME: usize = 4;
/// Pictures read and decoded at once.
const READS_AT_ONCE: usize = 4;
/// Pictures kept.
const THUMBS_KEPT: usize = 256;

/// What a row places.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum What {
    /// A thing Place mode arms and a drag carries.
    Place(Placeable),
}

impl What {
    fn of(kind: Kind, reference: Option<&str>) -> Self {
        What::Place(Placeable::new(kind, reference))
    }

    /// The row's identity: the payload key.
    #[must_use]
    pub fn key(&self) -> String {
        match self {
            What::Place(p) => p.key(),
        }
    }

    /// A key as [`Self::key`] writes it.
    #[must_use]
    pub fn parse(key: &str) -> Option<Self> {
        Placeable::parse(key).map(What::Place)
    }

    fn placeable(&self) -> Option<&Placeable> {
        match self {
            What::Place(p) => Some(p),
        }
    }
}

/// Places the AI Map Bounds volume on the ground in front of the camera and selects it: the
/// AI Map painting panel's one click for a level that has none (the Place panel's row arms it).
pub fn place_bounds_volume(editor: &Arc<Editor>) {
    crate::local::run(
        editor,
        "scene.place",
        crate::local::args(&[
            ("kind", json!("shape")),
            ("box", json!(true)),
            ("usage", json!("ai_map_bounds")),
            ("on_surface", json!(true)),
        ]),
    );
}

/// A category of the panel.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Recent,
    Objects,
    Lights,
    SpawnPoints,
    Shapes,
    Sounds,
    Particles,
    Ways,
    Groups,
}

impl Category {
    pub const ALL: [Category; 9] = [
        Category::Recent,
        Category::Objects,
        Category::Lights,
        Category::SpawnPoints,
        Category::Shapes,
        Category::Sounds,
        Category::Particles,
        Category::Ways,
        Category::Groups,
    ];

    /// Its name over MCP.
    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Category::Recent => "recent",
            Category::Objects => "objects",
            Category::Lights => "lights",
            Category::SpawnPoints => "spawn_points",
            Category::Shapes => "shapes",
            Category::Sounds => "sounds",
            Category::Particles => "particles",
            Category::Ways => "way_paths",
            Category::Groups => "groups",
        }
    }

    #[must_use]
    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id || c.title().eq_ignore_ascii_case(id))
    }

    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Category::Recent => "Recent",
            Category::Objects => "Objects",
            Category::Lights => "Lights",
            Category::SpawnPoints => "Spawn Points",
            Category::Shapes => "Shapes and Volumes",
            Category::Sounds => "Sounds",
            Category::Particles => "Particles",
            Category::Ways => "Way Paths",
            Category::Groups => "Groups",
        }
    }

    /// Open until the author folds it: the short ones; the long lists start folded.
    #[must_use]
    pub fn open_by_default(self) -> bool {
        matches!(self, Category::Recent | Category::Lights | Category::Shapes | Category::Ways)
    }

    fn store_key(self) -> String {
        format!("place.{}", self.id())
    }
}

/// One thing a row places.
#[derive(Clone, Debug, PartialEq)]
pub struct Item {
    pub category: Category,
    /// What the row reads: an object's leaf name, a section's in-game name.
    pub label: String,
    /// What it is in the library or the game: the reference, the section.
    pub detail: String,
    pub icon: Icon,
    pub what: What,
}

impl Item {
    fn new(category: Category, label: &str, detail: &str, icon: Icon, what: What) -> Self {
        Self { category, label: label.to_owned(), detail: detail.to_owned(), icon, what }
    }
}

/// The lists the panel reads; `None` while one is still being read.
#[derive(Clone, Default)]
pub struct Inputs {
    pub objects: Option<Arc<Vec<String>>>,
    pub spawns: Option<Arc<Vec<String>>>,
    /// A spawn section's in-game name, by section.
    pub names: Option<Arc<FxHashMap<String, String>>>,
    pub sounds: Option<Arc<Vec<String>>>,
    pub particles: Option<Arc<Vec<String>>>,
    pub groups: Option<Arc<Vec<String>>>,
}

impl Inputs {
    /// The revision of what is read: each list's identity.
    #[must_use]
    pub fn rev(&self) -> Rev {
        fn of<T>(l: &Option<Arc<T>>) -> Rev {
            Rev(l.as_ref().map_or(0, |a| Arc::as_ptr(a) as *const u8 as usize as u64))
        }
        of(&self.objects)
            .mix(of(&self.spawns))
            .mix(of(&self.names))
            .mix(of(&self.sounds))
            .mix(of(&self.particles))
            .mix(of(&self.groups))
    }

    /// What the lists hold now, asking for the ones not read yet (a frame never waits).
    #[must_use]
    pub fn now(editor: &Arc<Editor>) -> Self {
        Self {
            objects: Lists::get(editor, &ListKey::Objects),
            spawns: Lists::get(editor, &ListKey::SpawnSections),
            names: SpawnNames::get(editor),
            sounds: Lists::get(editor, &ListKey::Sounds),
            particles: Lists::get(editor, &ListKey::Particles),
            groups: Lists::get(editor, &ListKey::Groups),
        }
    }

    /// Every list, read now where it is missing: for a command.
    pub async fn wait(editor: &Arc<Editor>) -> Self {
        Self {
            objects: Some(Lists::wait(editor, &ListKey::Objects).await),
            spawns: Some(Lists::wait(editor, &ListKey::SpawnSections).await),
            names: Some(SpawnNames::wait(editor).await),
            sounds: Some(Lists::wait(editor, &ListKey::Sounds).await),
            particles: Some(Lists::wait(editor, &ListKey::Particles).await),
            groups: Some(Lists::wait(editor, &ListKey::Groups).await),
        }
    }
}

/// The in-game names of the spawn sections (the items' `inv_name` through the string table),
/// read once per project from the linked game's pickers (`game.lookup type=item_section`).
#[derive(Default)]
pub struct SpawnNames {
    slot: Mutex<NamesSlot>,
}

/// The names by section: the project they were read for, the names, a read on its way.
type NamesSlot = (Option<std::path::PathBuf>, Option<Arc<FxHashMap<String, String>>>, bool);

/// Pictures read and decoded, waiting for their upload: the item's key and the picture.
type Arrived = Arc<Mutex<Vec<(String, Option<ColorImage>)>>>;

impl SpawnNames {
    fn of(editor: &Editor) -> Arc<SpawnNames> {
        editor.services.get_or_init(SpawnNames::default)
    }

    fn get(editor: &Arc<Editor>) -> Option<Arc<FxHashMap<String, String>>> {
        let this = Self::of(editor);
        let root = editor.project_root();
        {
            let mut s = this.slot.lock();
            if s.0 != root {
                *s = (root, None, false);
            }
            if s.1.is_some() || s.2 {
                return s.1.clone();
            }
            s.2 = true;
        }
        let ed = editor.clone();
        editor.exec.spawn(async move {
            let names = Arc::new(read_names(&ed).await);
            let this = Self::of(&ed);
            let mut s = this.slot.lock();
            s.1 = Some(names);
            s.2 = false;
            drop(s);
            ed.exec.wake().ping();
        });
        None
    }

    async fn wait(editor: &Arc<Editor>) -> Arc<FxHashMap<String, String>> {
        if let Some(n) = Self::get(editor) {
            return n;
        }
        let names = Arc::new(read_names(editor).await);
        Self::of(editor).slot.lock().1 = Some(names.clone());
        names
    }
}

async fn read_names(ed: &Arc<Editor>) -> FxHashMap<String, String> {
    let a = crate::local::args(&[("type", json!("item_section")), ("limit", json!(1_000_000))]);
    let reply = ed.call("game.lookup", a, ed.ui_origin()).await;
    let mut out = FxHashMap::default();
    for e in reply.get("entries").and_then(Value::as_array).into_iter().flatten() {
        let (Some(id), Some(name)) =
            (e.get("id").and_then(Value::as_str), e.get("name").and_then(Value::as_str))
        else {
            continue;
        };
        if !name.is_empty() && name != id {
            out.insert(id.to_owned(), name.to_owned());
        }
    }
    out
}

/// One row of the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    /// A category's fold header with how many it holds (or matches).
    Header {
        category: Category,
        count: usize,
        open: bool,
    },
    /// A folder of a long list.
    Folder {
        category: Category,
        path: Arc<str>,
        depth: u16,
        open: bool,
    },
    Item {
        item: Arc<Item>,
        depth: u16,
    },
    /// What a category says while it has nothing to show ("Reading...", "Nothing here").
    Note {
        category: Category,
        text: &'static str,
        /// The list is still being read: the note carries the kit's spinner.
        busy: bool,
    },
}

impl Row {
    fn height(&self) -> f32 {
        match self {
            Row::Header { .. } => HEAD_H,
            _ => ROW_H,
        }
    }
}

/// What the panel lays out: the rows and where each starts, pt from the top.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Rows {
    pub rows: Vec<Row>,
    pub tops: Vec<f32>,
    pub height: f32,
    /// Items that pass the search, over every category.
    pub matches: usize,
}

/// What the rows depend on besides the lists.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct Key {
    pub search: String,
    /// The categories open, in order.
    pub open: Vec<Category>,
    /// The folders open, per category; `epoch` moves with every toggle.
    pub epoch: u64,
    pub recent: Vec<String>,
}

/// The fixed rows of the short categories.
fn builtin() -> Vec<Item> {
    use Category as C;
    vec![
        Item::new(C::Lights, "Light", "", Icon::Light, What::of(Kind::Light, None)),
        Item::new(C::Lights, "Glow", "", Icon::Glow, What::of(Kind::Glow, None)),
        Item::new(
            C::SpawnPoints,
            "Player Start",
            "player_start",
            Icon::PlayerStart,
            What::of(Kind::Spawn, Some("player_start")),
        ),
        Item::new(
            C::SpawnPoints,
            "Environment Modifier",
            "env_mod",
            Icon::Spawn,
            What::of(Kind::Spawn, Some("env_mod")),
        ),
        Item::new(
            C::SpawnPoints,
            "Respawn Point",
            "rpoint",
            Icon::Spawn,
            What::of(Kind::Spawn, Some("rpoint")),
        ),
        Item::new(C::Shapes, "Sphere Shape", "", Icon::Shape, What::of(Kind::Shape, None)),
        Item::new(C::Shapes, "Box Shape", "", Icon::Shape, What::of(Kind::Shape, Some("box"))),
        Item::new(C::Shapes, "AI Map Bounds", "", Icon::AIMap, What::of(Kind::Shape, Some("ai_map_bounds"))),
        Item::new(C::Shapes, "Fog Volume", "", Icon::FogVolume, What::of(Kind::Fog, None)),
        Item::new(C::Shapes, "Sound Environment", "", Icon::SoundEnv, What::of(Kind::SoundEnv, None)),
        Item::new(C::Ways, "Way Path", "", Icon::Way, What::of(Kind::Way, None)),
    ]
}

/// The leaf of a `\`-separated name.
fn leaf(path: &str) -> &str {
    path.rsplit('\\').next().unwrap_or(path)
}

/// The item a recent key names, as the lists read it now.
fn recent_item(key: &str, names: Option<&FxHashMap<String, String>>) -> Option<Item> {
    let what = What::parse(key)?;
    if let Some(b) = builtin().into_iter().find(|b| b.what == what) {
        return Some(Item { category: Category::Recent, ..b });
    }
    let p = what.placeable()?;
    let r = p.reference.clone()?;
    let (label, icon) = match p.kind {
        Kind::Spawn => (names.and_then(|n| n.get(&r)).cloned().unwrap_or_else(|| r.clone()), Icon::Spawn),
        k => (leaf(&r).to_owned(), crate::classes::icon(k.class())),
    };
    Some(Item { category: Category::Recent, label, detail: r, icon, what })
}

/// The items of a long list: `(folder path, item)` in the list's order.
fn list_items(cat: Category, inputs: &Inputs) -> Option<Vec<(String, Item)>> {
    let make = |list: &Arc<Vec<String>>, kind: Kind, icon: Icon| -> Vec<(String, Item)> {
        list.iter()
            .map(|r| {
                let folder = r.rsplit_once('\\').map_or("", |(f, _)| f).to_owned();
                (folder, Item::new(cat, leaf(r), r, icon, What::of(kind, Some(r))))
            })
            .collect()
    };
    Some(match cat {
        Category::Objects => make(inputs.objects.as_ref()?, Kind::Object, Icon::SceneObject),
        Category::Sounds => make(inputs.sounds.as_ref()?, Kind::Sound, Icon::SoundSource),
        Category::Particles => make(inputs.particles.as_ref()?, Kind::Particles, Icon::Particles),
        Category::Groups => make(inputs.groups.as_ref()?, Kind::Group, Icon::Group),
        Category::SpawnPoints => {
            let names = inputs.names.as_deref();
            inputs
                .spawns
                .as_ref()?
                .iter()
                .map(|r| {
                    let (folder, section) = r.rsplit_once('\\').unwrap_or(("", r));
                    let label = names.and_then(|n| n.get(section)).map_or(section, String::as_str);
                    (
                        folder.to_owned(),
                        Item::new(cat, label, section, Icon::Spawn, What::of(Kind::Spawn, Some(section))),
                    )
                })
                .collect()
        }
        _ => return Some(Vec::new()),
    })
}

/// Whether an item passes the search: every word, in its label or what it is.
fn passes(search: &str, item: &Item) -> bool {
    search.trim().is_empty() || gz_ui::search::matches(search, &format!("{} {}", item.label, item.detail))
}

/// The rows of the panel: a pure function of the lists, the search, what is open and Recent.
/// While a search is on, every category and folder that holds a match is open and the others
/// are left out.
#[must_use]
pub fn rows(inputs: &Inputs, key: &Key, folders: &FxHashMap<Category, FxHashSet<String>>) -> Rows {
    let searching = !key.search.trim().is_empty();
    let fixed = builtin();
    let mut out = Rows::default();
    let push = |out: &mut Rows, r: Row| {
        out.tops.push(out.height);
        out.height += r.height();
        out.rows.push(r);
    };
    for cat in Category::ALL {
        let open = searching || key.open.contains(&cat);
        // the items: fixed ones first, then the category's list
        let mut items: Vec<(String, Item)> =
            fixed.iter().filter(|b| b.category == cat).map(|b| (String::new(), b.clone())).collect();
        if cat == Category::Recent {
            items.extend(
                key.recent
                    .iter()
                    .filter_map(|k| recent_item(k, inputs.names.as_deref()))
                    .map(|i| (String::new(), i)),
            );
        }
        let reading = match list_items(cat, inputs) {
            Some(more) => {
                items.extend(more);
                false
            }
            None => true,
        };
        let shown: Vec<(String, Item)> = items.into_iter().filter(|(_, i)| passes(&key.search, i)).collect();
        out.matches += shown.len();
        if (searching || cat == Category::Recent) && shown.is_empty() && !(reading && searching) {
            continue;
        }
        push(&mut out, Row::Header { category: cat, count: shown.len(), open });
        if !open {
            continue;
        }
        if reading {
            push(&mut out, Row::Note { category: cat, text: "Reading the list...", busy: true });
        } else if shown.is_empty() {
            push(
                &mut out,
                Row::Note {
                    category: cat,
                    text: "Nothing here: the linked game or the library has none.",
                    busy: false,
                },
            );
        }
        let open_folders = folders.get(&cat);
        // the folders already emitted, as a stack of paths
        let mut stack: Vec<String> = Vec::new();
        for (folder, item) in shown {
            let dirs: Vec<&str> = if folder.is_empty() { Vec::new() } else { folder.split('\\').collect() };
            let common =
                stack.iter().zip(dirs.iter()).take_while(|(a, b)| a.rsplit('\\').next() == Some(**b)).count();
            stack.truncate(common);
            let is_open = |p: &str| searching || open_folders.is_some_and(|s| s.contains(p));
            let mut visible = stack.iter().all(|d| is_open(d));
            for d in dirs.iter().skip(common) {
                let path = match stack.last() {
                    Some(parent) => format!("{parent}\\{d}"),
                    None => (*d).to_owned(),
                };
                if visible {
                    push(
                        &mut out,
                        Row::Folder {
                            category: cat,
                            path: Arc::from(path.as_str()),
                            depth: stack.len() as u16,
                            open: is_open(&path),
                        },
                    );
                }
                visible = visible && is_open(&path);
                stack.push(path);
            }
            if visible {
                push(&mut out, Row::Item { item: Arc::new(item), depth: stack.len() as u16 });
            }
        }
    }
    out
}

/// An object's or a group's picture, as the panel keeps it.
enum Thumb {
    Reading,
    Shown(TextureHandle),
    None,
}

/// The pictures of the rows on screen: read and decoded on the pools, uploaded a few a frame.
#[derive(Default)]
struct Thumbs {
    cache: FxHashMap<String, (Thumb, u64)>,
    done: Arrived,
    reading: Arc<std::sync::atomic::AtomicUsize>,
}

impl Thumbs {
    /// Uploads what arrived, a few a frame; forgets the coldest past the budget.
    fn begin_frame(&mut self, ctx: &egui::Context, frame: u64) {
        let arrived: Vec<(String, Option<ColorImage>)> = {
            let mut d = self.done.lock();
            let n = d.len().min(UPLOADS_PER_FRAME);
            d.drain(..n).collect()
        };
        if !self.done.lock().is_empty() {
            ctx.request_repaint();
        }
        for (key, img) in arrived {
            let thumb = match img {
                Some(img) => {
                    Thumb::Shown(ctx.load_texture(format!("place:{key}"), img, TextureOptions::LINEAR))
                }
                None => Thumb::None,
            };
            self.cache.insert(key, (thumb, frame));
        }
        if self.cache.len() > THUMBS_KEPT {
            let mut used: Vec<(u64, String)> = self.cache.iter().map(|(k, (_, f))| (*f, k.clone())).collect();
            used.sort_unstable();
            for (_, k) in used.into_iter().take(self.cache.len() - THUMBS_KEPT) {
                self.cache.remove(&k);
            }
        }
    }

    /// The picture of an item, asked for when it is not there yet.
    fn get(&mut self, editor: &Arc<Editor>, item: &Item, frame: u64) -> Option<egui::TextureId> {
        let kind = item.what.placeable().map(|p| p.kind)?;
        let key = format!("{}:{}", kind.key(), item.detail.to_lowercase());
        if let Some((t, used)) = self.cache.get_mut(&key) {
            *used = frame;
            return match t {
                Thumb::Shown(h) => Some(h.id()),
                _ => None,
            };
        }
        use std::sync::atomic::Ordering;
        if self.reading.load(Ordering::Acquire) >= READS_AT_ONCE {
            return None;
        }
        let files: Vec<std::path::PathBuf> = match kind {
            Kind::Object => {
                let svc = editor.project_opt().and_then(|s| s.services.get::<SceneService>())?;
                let rel = format!("{}.thm", gz_scene::library::normalize(&item.detail));
                svc.library.roots().iter().map(|r| r.join(&rel)).collect()
            }
            Kind::Group => {
                let root = gz_project::library_root(editor).join("rawdata").join("groups");
                let rel = item.detail.trim_end_matches(".group");
                vec![root.join(format!("{rel}.thm"))]
            }
            _ => return None,
        };
        self.cache.insert(key.clone(), (Thumb::Reading, frame));
        self.reading.fetch_add(1, Ordering::AcqRel);
        let (done, reading, ed) = (self.done.clone(), self.reading.clone(), editor.clone());
        editor.exec.spawn_io(move || {
            let img = files.iter().find_map(|f| std::fs::read(f).ok()).and_then(|b| thumb_image(&b));
            done.lock().push((key, img));
            reading.fetch_sub(1, std::sync::atomic::Ordering::AcqRel);
            ed.exec.wake().ping();
        });
        None
    }
}

/// The 128 x 128 picture a library thumbnail file holds.
fn thumb_image(bytes: &[u8]) -> Option<ColorImage> {
    let pixels = match gz_fmt_tex::read_thm(bytes).ok()? {
        gz_fmt_tex::Thm::Object(o) => o.pixels,
        gz_fmt_tex::Thm::Group(g) => g.pixels,
        _ => return None,
    };
    (pixels.len() == 128 * 128 * 4).then(|| ColorImage::from_rgba_unmultiplied([128, 128], &pixels))
}

/// The panel's own state.
pub struct PlacePanel {
    pub search: String,
    query: Query<Key, Rows>,
    shown: Arc<Rows>,
    folders: FxHashMap<Category, FxHashSet<String>>,
    epoch: u64,
    thumbs: Thumbs,
}

impl PlacePanel {
    #[must_use]
    pub fn new(editor: &Editor) -> Self {
        Self {
            search: String::new(),
            query: Query::new(editor.exec.clone()),
            shown: Arc::new(Rows::default()),
            folders: FxHashMap::default(),
            epoch: 0,
            thumbs: Thumbs::default(),
        }
    }

    /// What the rows are worked out from, as the store and the panel hold it now.
    #[must_use]
    pub fn key(&self, ctx: &egui::Context) -> Key {
        Key {
            search: self.search.clone(),
            open: Category::ALL
                .into_iter()
                .filter(|c| gz_ui::section::is_open(ctx, &c.store_key(), c.open_by_default()))
                .collect(),
            epoch: self.epoch,
            recent: gz_ui::layout::list(ctx, RECENT_KEY),
        }
    }

    /// What a command needs to work the rows out as the panel will: the panel's query (so the
    /// answer also primes the frame's), the key and the open folders.
    #[must_use]
    pub fn job(&self, ctx: &egui::Context) -> RowsJob {
        RowsJob { query: self.query.clone(), key: self.key(ctx), folders: self.folders.clone() }
    }

    /// Opens or folds a category, as a click on its header does.
    pub fn set_open(ctx: &egui::Context, category: Category, open: bool) {
        gz_ui::section::set_open(ctx, &category.store_key(), open);
    }

    /// Opens or closes a folder of a category.
    pub fn set_folder(&mut self, category: Category, path: &str, open: bool) {
        let set = self.folders.entry(category).or_default();
        let changed = if open { set.insert(path.to_owned()) } else { set.remove(path) };
        if changed {
            self.epoch += 1;
        }
    }

    /// What Place mode places now, while it is up (armed here, by a command or by the viewport).
    #[must_use]
    pub fn armed(&self, editor: &Editor, snap: &gz_scene::SceneState) -> Option<Placeable> {
        if snap.tools.mode != SceneMode::Place {
            return None;
        }
        gz_ui_viewport::place::armed(editor)
    }

    /// The panel's body.
    pub fn draw(&mut self, ui: &mut Ui, editor: &Arc<Editor>, svc: &Arc<SceneService>) {
        let ctx = ui.ctx().clone();
        let frame = ctx.cumulative_frame_nr();
        self.thumbs.begin_frame(&ctx, frame);
        let snap = svc.store.snapshot();
        SearchField::new(&mut self.search)
            .hint("Search everything to place")
            .id_salt("place_search")
            .show(ui);
        ui.add_space(4.0);
        let inputs = Inputs::now(editor);
        let key = self.key(&ctx);
        let rev = inputs.rev();
        let folders = self.folders.clone();
        let k = key.clone();
        if let Some(r) = self.query.get(&key, rev, move || rows(&inputs, &k, &folders)).value() {
            self.shown = r.clone();
        }
        let armed = self.armed(editor, &snap);
        let status_h = if armed.is_some() { ROW_H + 8.0 } else { 0.0 };
        let list_h = (ui.available_height() - status_h).max(ROW_H * 3.0);
        let rows = self.shown.clone();
        if rows.rows.is_empty() {
            ui.allocate_ui(Vec2::new(ui.available_width(), list_h), |ui| {
                if self.search.trim().is_empty() {
                    state::busy_line(ui, "Reading the library and the game...");
                } else if state::empty_state(
                    ui,
                    Icon::Search,
                    "Nothing matches",
                    "No placeable thing has this name.",
                    Some("Clear Search"),
                ) {
                    self.search.clear();
                }
            });
        } else {
            self.list(ui, editor, &rows, armed.as_ref(), list_h, frame);
        }
        if let Some(a) = armed {
            self.armed_line(ui, editor, &a);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn list(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        rows: &Rows,
        armed: Option<&Placeable>,
        height: f32,
        frame: u64,
    ) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let mut scroll = egui::ScrollArea::vertical()
            .id_salt("gz-ui-level.place_rows")
            .max_height(height)
            .auto_shrink([false, false]);
        match gz_app::take_scroll(&ctx, crate::layout::PanelId::Place.window()) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(rows.height),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            _ => {}
        }
        let mut toggle_cat: Option<Category> = None;
        let mut toggle_folder: Option<(Category, Arc<str>, bool)> = None;
        let mut clicked: Option<(What, bool)> = None;
        let mut dropped: Option<What> = None;
        let panel_rect = ui.max_rect();
        let out = scroll.show_viewport(ui, |ui, view| {
            let width = ui.available_width();
            ui.set_height(rows.height);
            let origin = ui.min_rect().min;
            // the rows the view shows: their tops are sorted
            let first = rows.tops.partition_point(|t| *t + HEAD_H < view.min.y);
            let last = rows.tops.partition_point(|t| *t <= view.max.y);
            for i in first..last.min(rows.rows.len()) {
                let (Some(row), Some(top)) = (rows.rows.get(i), rows.tops.get(i)) else { continue };
                let rect =
                    Rect::from_min_size(pos2(origin.x, origin.y + top), Vec2::new(width, row.height()));
                match row {
                    Row::Header { category, count, .. } => {
                        let mut child = crate::local::child_at(ui, ("place_head", category.id()), rect);
                        let count = count.to_string();
                        let head = Section::new(&category.store_key(), category.title())
                            .default_open(category.open_by_default())
                            .count(&count)
                            .show(&mut child);
                        if head.toggled {
                            toggle_cat = Some(*category);
                        }
                    }
                    Row::Folder { category, path, depth, open } => {
                        let id = Id::unique(("place_folder", category.id(), &**path));
                        let resp = ui.interact(rect, id, Sense::click());
                        if resp.hovered() {
                            ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
                        }
                        let x = rect.min.x + 4.0 + f32::from(*depth) * INDENT;
                        icons::draw(
                            ui.painter(),
                            if *open { Icon::ChevronDown } else { Icon::ChevronRight },
                            Rect::from_min_size(pos2(x, rect.center().y - 6.0), Vec2::splat(12.0)),
                            p.text_secondary,
                        );
                        icons::draw(
                            ui.painter(),
                            if *open { Icon::FolderOpen } else { Icon::Folder },
                            Rect::from_min_size(pos2(x + 14.0, rect.center().y - 7.0), Vec2::splat(14.0)),
                            p.text_muted,
                        );
                        text::paint(
                            ui.painter(),
                            &Font::Body.id(),
                            pos2(x + 34.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            p.text_secondary,
                            leaf(path),
                            Some((rect.max.x - x - 38.0).max(0.0)),
                            Clip::End,
                        );
                        if resp.clicked() && self.search.trim().is_empty() {
                            toggle_folder = Some((*category, path.clone(), !*open));
                        }
                    }
                    Row::Note { text, busy, .. } => {
                        if *busy {
                            gz_ui::state::spinner_at(
                                &ctx,
                                ui.painter(),
                                Rect::from_center_size(
                                    pos2(rect.min.x + 14.0, rect.center().y),
                                    Vec2::splat(metrics::GLYPH),
                                ),
                            );
                            gz_ui::audit::loading(&ctx, text, rect);
                        }
                        text::paint(
                            ui.painter(),
                            &Font::Body.id(),
                            pos2(rect.min.x + 24.0, rect.center().y),
                            Align2::LEFT_CENTER,
                            p.text_muted,
                            text,
                            Some((rect.width() - 28.0).max(0.0)),
                            Clip::End,
                        );
                    }
                    Row::Item { item, depth } => {
                        let is_armed = armed.is_some() && armed == item.what.placeable();
                        let r = self.item_row(ui, editor, item, *depth, rect, is_armed, frame);
                        if r.clicked {
                            clicked = Some((item.what.clone(), is_armed));
                        }
                        if r.dropped_outside_of(panel_rect) {
                            dropped = Some(item.what.clone());
                        }
                    }
                }
            }
        });
        gz_app::report_scroll(
            &ctx,
            crate::layout::PanelId::Place.window(),
            out.state.clamped_offset().y,
            (rows.height - out.inner_rect.height()).max(0.0),
        );
        if let Some(c) = toggle_cat {
            // the section stored its own state; the rows follow the key on the next frame
            let _ = c;
            ctx.request_repaint();
        }
        if let Some((c, path, open)) = toggle_folder {
            self.set_folder(c, &path, open);
        }
        match clicked {
            Some((_, true)) => self.disarm(editor),
            Some((what, false)) => self.pick(&ctx, editor, what),
            None => {}
        }
        if let Some(w) = dropped {
            gz_ui::layout::push_recent(&ctx, RECENT_KEY, &w.key(), RECENT);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn item_row(
        &mut self,
        ui: &mut Ui,
        editor: &Arc<Editor>,
        item: &Item,
        depth: u16,
        rect: Rect,
        armed: bool,
        frame: u64,
    ) -> ItemResponse {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let key = item.what.key();
        let id = Id::unique(("place_item", item.category.id(), key.as_str()));
        let draggable = item.what.placeable();
        let resp =
            ui.interact(rect, id, if draggable.is_some() { Sense::click_and_drag() } else { Sense::click() });
        if let Some(p) = draggable {
            gz_ui::drop::start(&resp, p.payload());
        }
        if armed {
            ui.painter().rect_filled(rect, CornerRadius::same(3), p.selection_focused);
        } else if resp.hovered() || resp.dragged() {
            ui.painter().rect_filled(rect, CornerRadius::same(3), p.hover);
        }
        let x = rect.min.x + 4.0 + f32::from(depth) * INDENT + if depth > 0 { 14.0 } else { 0.0 };
        let pic = Rect::from_min_size(pos2(x, rect.center().y - THUMB * 0.5), Vec2::splat(THUMB));
        let tex = self.thumbs.get(editor, item, frame);
        match tex {
            Some(t) => {
                ui.painter().image(
                    t,
                    pic,
                    Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            None => icons::draw(ui.painter(), item.icon, pic.shrink(2.0), p.text_secondary),
        }
        let tx = pic.max.x + 6.0;
        let detail_w = if item.detail.is_empty()
            || item.detail == item.label
            || item.what.placeable().is_none_or(|p| p.kind != Kind::Spawn)
        {
            0.0
        } else {
            (rect.width() * 0.35).min(text::width(&ctx, &Font::Small.id(), &item.detail) + 4.0)
        };
        let cut = text::paint(
            ui.painter(),
            &Font::Body.id(),
            pos2(tx, rect.center().y),
            Align2::LEFT_CENTER,
            p.text,
            &item.label,
            Some((rect.max.x - tx - detail_w - 6.0).max(0.0)),
            Clip::Middle,
        );
        if detail_w > 0.0 {
            text::paint(
                ui.painter(),
                &Font::Small.id(),
                pos2(rect.max.x - 4.0, rect.center().y),
                Align2::RIGHT_CENTER,
                p.text_muted,
                &item.detail,
                Some(detail_w),
                Clip::Middle,
            );
        }
        if resp.dragged() {
            drag_label(&ctx, &item.label);
        }
        let dropped_at = resp.drag_stopped().then(|| ctx.input(|i| i.pointer.latest_pos())).flatten();
        let clicked = resp.clicked();
        let what = if item.detail.is_empty() || item.detail == item.label {
            String::new()
        } else {
            format!("{}\n", item.detail)
        };
        let tip = format!(
            "{}\n{what}{}",
            item.label,
            if armed {
                "Placing it: each click in the viewport places one. Click again or press Esc to stop."
            } else if draggable.is_none() {
                "Click to place it on the ground in front of the camera."
            } else {
                "Click to place it with clicks in the viewport, or drag it into the viewport."
            }
        );
        let _ = cut;
        resp.on_hover_ui(|ui| {
            if let Some(t) = tex {
                ui.image(egui::load::SizedTexture::new(t, Vec2::splat(128.0)));
            }
            ui.label(tip);
        });
        ItemResponse { clicked, dropped_at }
    }

    /// What a click on a row does: arms the thing for Place mode, or places the bounds volume.
    pub fn pick(&mut self, ctx: &egui::Context, editor: &Arc<Editor>, what: What) {
        gz_ui::layout::push_recent(ctx, RECENT_KEY, &what.key(), RECENT);
        match what {
            What::Place(p) => {
                gz_ui_viewport::place::arm(editor, Some(p));
            }
        }
    }

    /// Stops placing: Select mode.
    pub fn disarm(&mut self, editor: &Arc<Editor>) {
        gz_ui_viewport::place::arm(editor, None);
    }

    fn armed_line(&mut self, ui: &mut Ui, editor: &Arc<Editor>, a: &Placeable) {
        let p = palette(ui.ctx());
        ui.add_space(4.0);
        let label = self
            .shown
            .rows
            .iter()
            .find_map(|r| match r {
                Row::Item { item, .. } if item.what.placeable() == Some(a) => Some(item.label.clone()),
                _ => None,
            })
            .unwrap_or_else(|| a.title());
        ui.horizontal(|ui| {
            icons::draw(
                ui.painter(),
                Icon::Place,
                Rect::from_min_size(ui.cursor().min + Vec2::new(0.0, 3.0), Vec2::splat(16.0)),
                p.accent_bright,
            );
            ui.add_space(20.0);
            let room = (ui.available_width() - metrics::ICON_BUTTON - 8.0).max(0.0);
            ui.allocate_ui(Vec2::new(room, ROW_H), |ui| {
                text::label(
                    ui,
                    &format!("Placing {label} \u{b7} Esc stops"),
                    Font::Body,
                    p.text,
                    Clip::Middle,
                );
            });
            if gz_ui::icon_button(ui, Icon::Close, "Stop placing", false, true).clicked() {
                self.disarm(editor);
            }
        });
    }

    /// What `level.view` answers of the panel.
    #[must_use]
    pub fn view_state(
        &self,
        editor: &Editor,
        snap: &gz_scene::SceneState,
        rows: &Rows,
        limit: usize,
    ) -> Value {
        let cats: Vec<Value> = rows
            .rows
            .iter()
            .filter_map(|r| match r {
                Row::Header { category, count, open } => Some(
                    json!({ "id": category.id(), "name": category.title(), "count": count, "open": open }),
                ),
                _ => None,
            })
            .collect();
        let shown: Vec<Value> = rows
            .rows
            .iter()
            .take(limit)
            .map(|r| match r {
                Row::Header { category, count, open } => {
                    json!({ "category": category.id(), "count": count, "open": open })
                }
                Row::Folder { category, path, depth, open } => {
                    json!({ "folder": &**path, "category": category.id(), "depth": depth, "open": open })
                }
                Row::Note { category, text, .. } => json!({ "note": text, "category": category.id() }),
                Row::Item { item, depth } => json!({
                    "item": item.label,
                    "what": item.detail,
                    "category": item.category.id(),
                    "depth": depth,
                    "payload": item.what.key(),
                }),
            })
            .collect();
        json!({
            "search": self.search,
            "armed": self.armed(editor, snap).map(|a| a.key()),
            "matches": rows.matches,
            "categories": cats,
            "rows": shown,
            "total_rows": rows.rows.len(),
        })
    }
}

/// The rows of the panel as a command asks for them: worked out on the CPU pool, fresh.
pub struct RowsJob {
    query: Query<Key, Rows>,
    key: Key,
    folders: FxHashMap<Category, FxHashSet<String>>,
}

impl RowsJob {
    pub async fn rows(self, inputs: Inputs) -> Arc<Rows> {
        let (key, folders, rev) = (self.key.clone(), self.folders, inputs.rev());
        self.query.fresh(&self.key, rev, move || rows(&inputs, &key, &folders)).await
    }
}

/// What an item's row was asked this frame.
struct ItemResponse {
    clicked: bool,
    /// Where a drag of the row ended.
    dropped_at: Option<Pos2>,
}

impl ItemResponse {
    /// A drag that ended outside the panel: it went somewhere it may have been placed.
    fn dropped_outside_of(&self, panel: Rect) -> bool {
        self.dropped_at.is_some_and(|at| !panel.contains(at))
    }
}

/// The name of what is dragged, under the pointer.
fn drag_label(ctx: &egui::Context, label: &str) {
    let Some(at) = ctx.pointer_latest_pos() else { return };
    let p = palette(ctx);
    egui::Area::new(Id::unique("gz-ui-level.place_drag"))
        .order(egui::Order::Tooltip)
        .fixed_pos(at + Vec2::new(14.0, 10.0))
        .interactable(false)
        .show(ctx, |ui| {
            gz_ui::menu::popup_frame(ctx).show(ui, |ui| {
                ui.horizontal(|ui| {
                    icons::draw(
                        ui.painter(),
                        Icon::Place,
                        Rect::from_min_size(ui.cursor().min + Vec2::new(0.0, 2.0), Vec2::splat(14.0)),
                        p.accent_bright,
                    );
                    ui.add_space(18.0);
                    text::label(ui, label, Font::Body, p.text, Clip::Middle);
                });
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn inputs() -> Inputs {
        let mut names = FxHashMap::default();
        names.insert("wpn_pm".to_owned(), "PM Pistol".to_owned());
        Inputs {
            objects: Some(Arc::new(vec![
                "props\\box_wood".into(),
                "props\\barrel".into(),
                "trees\\pine".into(),
            ])),
            spawns: Some(Arc::new(vec!["weapons\\pistols\\wpn_pm".into(), "monsters\\dog_weak".into()])),
            names: Some(Arc::new(names)),
            sounds: Some(Arc::new(vec!["ambient\\wind".into()])),
            particles: Some(Arc::new(Vec::new())),
            groups: None,
        }
    }

    fn item_labels(r: &Rows) -> Vec<&str> {
        r.rows
            .iter()
            .filter_map(|x| match x {
                Row::Item { item, .. } => Some(item.label.as_str()),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_search_opens_what_matches_and_leaves_the_rest_out() {
        let key = Key { search: "pistol".into(), ..Key::default() };
        let r = rows(&inputs(), &key, &FxHashMap::default());
        // the in-game name is what matches, its folders are open, nothing else shows
        assert_eq!(item_labels(&r), vec!["PM Pistol"]);
        assert!(
            r.rows
                .iter()
                .any(|x| matches!(x, Row::Folder { path, open: true, .. } if &**path == "weapons\\pistols"))
        );
        assert!(matches!(r.rows[0], Row::Header { category: Category::SpawnPoints, count: 1, open: true }));
        assert_eq!(r.matches, 1);
        // the section itself matches too
        let key = Key { search: "dog_weak".into(), ..Key::default() };
        assert_eq!(item_labels(&rows(&inputs(), &key, &FxHashMap::default())), vec!["dog_weak"]);
    }

    #[test]
    fn folded_categories_and_folders_hide_their_rows() {
        let key = Key { open: vec![Category::Objects, Category::Lights], ..Key::default() };
        let r = rows(&inputs(), &key, &FxHashMap::default());
        assert_eq!(item_labels(&r), vec!["Light", "Glow"]);
        let mut folders = FxHashMap::default();
        folders.insert(Category::Objects, ["props".to_owned()].into_iter().collect::<FxHashSet<String>>());
        let r = rows(&inputs(), &key, &folders);
        assert_eq!(item_labels(&r), vec!["box_wood", "barrel", "Light", "Glow"]);
        // a list still being read says so under its open header
        let key = Key { open: vec![Category::Groups], ..Key::default() };
        let r = rows(&inputs(), &key, &FxHashMap::default());
        assert!(r.rows.iter().any(|x| matches!(x, Row::Note { category: Category::Groups, .. })));
        // the tops add up
        assert_eq!(r.tops.len(), r.rows.len());
        assert!((r.height - r.rows.iter().map(Row::height).sum::<f32>()).abs() < 0.01);
    }

    #[test]
    fn recent_lists_what_was_placed_last() {
        let recent = vec![What::of(Kind::Spawn, Some("wpn_pm")).key(), What::of(Kind::Light, None).key()];
        let key = Key { open: vec![Category::Recent], recent, ..Key::default() };
        let r = rows(&inputs(), &key, &FxHashMap::default());
        assert!(matches!(r.rows[0], Row::Header { category: Category::Recent, count: 2, .. }));
        assert_eq!(item_labels(&r)[..2], ["PM Pistol", "Light"]);
        // without anything placed there is no Recent at all
        let r = rows(&inputs(), &Key::default(), &FxHashMap::default());
        assert!(!r.rows.iter().any(|x| matches!(x, Row::Header { category: Category::Recent, .. })));
    }

    #[test]
    fn a_row_drags_what_the_viewport_places() {
        let key = Key { search: "barrel".into(), ..Key::default() };
        let r = rows(&inputs(), &key, &FxHashMap::default());
        let item = r
            .rows
            .iter()
            .find_map(|x| match x {
                Row::Item { item, .. } => Some(item.clone()),
                _ => None,
            })
            .expect("the barrel");
        let What::Place(p) = &item.what;
        assert_eq!(p.key(), "object:props\\barrel");
        // the payload the row drags is the viewport's own, and reads back as the same thing
        assert_eq!(Placeable::of_payload(&p.payload()), vec![p.clone()]);
        // the bounds volume is placed at once: it has a key, no payload
        // the bounds volume is a shape with a use, armed and dragged like every other row
        assert_eq!(What::parse("shape:ai_map_bounds"), Some(What::of(Kind::Shape, Some("ai_map_bounds"))));
        assert_eq!(What::parse("shape:box"), Some(What::of(Kind::Shape, Some("box"))));
    }

    /// Frames of the panel's rows in the kit's look: a click on a row is its pick, a drag carries
    /// the viewport's payload.
    #[test]
    fn a_row_is_clicked_to_arm_and_dragged_into_the_viewport() {
        use egui::{Event, Modifiers, PointerButton, RawInput};
        let rt =
            gz_task::Runtime::start(gz_task::ExecOptions { async_threads: 1, cpu_threads: 1, io_threads: 1 })
                .expect("runtime");
        let ed = Editor::new(
            gz_editor::EditorOptions::default(),
            rt.exec(),
            gz_log::Log::memory(),
            gz_editor::Registry::new(),
        );
        let mut panel = PlacePanel::new(&ed);
        let ctx = egui::Context::default();
        gz_ui::theme::install(&ctx, gz_ui::theme::Accent::default());
        let item = Item::new(Category::Lights, "Light", "", Icon::Light, What::of(Kind::Light, None));
        let row = Rect::from_min_size(pos2(10.0, 10.0), Vec2::new(240.0, ROW_H));
        let mut run = |events: Vec<Event>, t: f64| -> ItemResponse {
            let mut got = None;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
                time: Some(t),
                events,
                ..RawInput::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                got = Some(panel.item_row(ui, &ed, &item, 0, row, false, 1));
            });
            out.textures_delta.clear();
            got.expect("drawn")
        };
        let at = row.center();
        let button = |pos: Pos2, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        run(vec![Event::PointerMoved(at)], 0.0);
        run(vec![button(at, true)], 0.05);
        let r = run(vec![button(at, false)], 0.1);
        assert!(r.clicked, "a click on the row picks it");
        // a press and a move far enough is a drag: the payload is the viewport's own
        run(vec![button(at, true)], 1.0);
        let away = at + Vec2::new(300.0, 120.0);
        run(vec![Event::PointerMoved(at + Vec2::new(20.0, 0.0))], 1.1);
        run(vec![Event::PointerMoved(away)], 1.2);
        let dragged = gz_ui::drop::dragged(&ctx).expect("a payload while dragging");
        assert_eq!(Placeable::of_payload(&dragged), vec![Placeable::new(Kind::Light, None)]);
        let r = run(vec![button(away, false)], 1.3);
        assert!(r.dropped_outside_of(Rect::from_min_size(Pos2::ZERO, Vec2::new(260.0, 400.0))));
        assert!(!r.clicked);
        drop(rt);
    }
}
