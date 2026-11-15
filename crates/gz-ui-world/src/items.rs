//! Items: the `items` singleton tab (docs/UI_DESIGN.md, section 6, "Items").
//!
//! Three columns: the module's items under two headings (New, Changes), the form of the one
//! selected, and - where there is room - the item as the game will show it. The tab keeps VIEW
//! state and the item being edited: the roster comes from `gz_items::Items` on the pools; a save is
//! `Items::save` - the difference between the item as the tab read it and as it is now, written
//! into the file's own text; a deletion is `file.delete`; the + menu's pick makes the file at once
//! (`Items::save` of a file that is not there writes it canonically). The form binds to what is
//! STORED (`gz_items::Item`), never to the resolved values; what the item resolves to and its
//! problems are worked out on the pools for the item as it is edited (`Items::check_edit`).

use std::sync::Arc;

use egui::{Align, Layout, Rect, ScrollArea, Ui, UiBuilder, pos2};
use gz_app::{DocCx, Document, ShellBuilder};
use gz_base::{RelPath, Rev};
use gz_characters::{text_collapse, text_in, text_raw, text_set};
use gz_editor::{Editor, Value, json};
use gz_items::model::{diff_rules, from_value, to_value, write};
use gz_items::{CatalogRead, Icon as ItemIcon, Issue, Item, Items, Resolution, Roster};
use gz_luadata::Value as LuaValue;
use gz_luadata::{Conflict, Limits, LuaDoc, set_at};
use gz_mod::ManifestService;
use gz_project::Project;
use gz_ui::dialog::{Answer, ConfirmSpec, confirm, unsaved_prompt};
use gz_ui::{Icon, Kind, Tone, Toolbar, Tx, tr};
use gz_ui_level::choose::{self, Chooser, Outcome, Purpose, RowIcons};
use gz_ui_level::lists::ListKey;
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::call::{self, args, s};
use crate::item_lists::{ItemLists, ItemListsView, Pictures};
use crate::item_model::{self, Form, Place};
use crate::lists::{Lists, ListsView};

/// The tab's id and kind.
pub const TAB: &str = "items";
/// The scroll `ui.scroll id=items` moves: the work area of the tab that is up.
pub(crate) const HOST: &str = "###doc.host";
/// The narrowest the preview column is drawn; under it the form keeps the room.
pub(crate) const PREVIEW_MIN: f32 = 300.0;
/// The scroll `ui.scroll id=items.preview` moves: the preview column.
pub(crate) const PREVIEW_SCROLL: &str = "items.preview";
/// Where the + menu's pick makes its file.
const FOLDER: &str = "items";

/// What the game's inventory, its tooltip and its world read of an item by these names: the
/// preview and the list draw them as the game does. What each means is the catalog's.
pub(crate) mod game_keys {
    pub const ICON_TEXTURE: &str = "icons_texture";
    pub const GRID_X: &str = "inv_grid_x";
    pub const GRID_Y: &str = "inv_grid_y";
    pub const GRID_W: &str = "inv_grid_width";
    pub const GRID_H: &str = "inv_grid_height";
    /// The texture of the icons when an item names none.
    pub const DEFAULT_ICONS: &str = "ui\\ui_icon_equipment";
    pub const NAME: &str = "inv_name";
    pub const SHORT: &str = "inv_name_short";
    pub const DESCRIPTION: &str = "description";
    pub const WEIGHT: &str = "inv_weight";
    pub const COST: &str = "cost";
    pub const VISUAL: &str = "visual";
}

/// The text fields of the form, in this order: Name, Short Name, Description, each in rus and
/// eng.
pub(crate) const TEXTS: [(&str, &str); 6] = [
    ("name", "rus"),
    ("name", "eng"),
    ("short_name", "rus"),
    ("short_name", "eng"),
    ("description", "rus"),
    ("description", "eng"),
];

/// A pick list or chooser of the form.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Pick {
    /// The + menu's New Item...: a new item that starts from the picked one.
    New,
    /// The + menu's Change a Game Item...
    Change,
    /// Starts From: another base.
    Base,
    /// A key's value from a list of the game (a model, a texture, a sound, a particle effect).
    Key { scope: String, key: String },
}

impl Pick {
    /// `items.view show=` of it.
    #[must_use]
    pub(crate) fn word(&self) -> String {
        match self {
            Pick::New => "new".into(),
            Pick::Change => "change".into(),
            Pick::Base => "base".into(),
            Pick::Key { scope, key } => {
                if scope == gz_items::catalog::ITEM {
                    format!("key:{key}")
                } else {
                    format!("key:{scope}.{key}")
                }
            }
        }
    }
}

/// What replaces the edited item once the unsaved question is answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Then {
    Nothing,
    /// The item of this file.
    Select(String),
    /// A chooser of the + menu.
    Choose(Pick),
}

/// Where an icon's pixels are.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum IconSource {
    /// A picture file beside an asset, with its write time.
    Picture { path: std::path::PathBuf, stamp: u64 },
    /// A rectangle of a texture of the game, in cells of 50 px.
    Cell { sheet: String, x: u32, y: u32, w: u32, h: u32 },
}

impl IconSource {
    /// Its size in cells: a picture's is worked out from its pixels by the caller.
    #[must_use]
    pub(crate) fn cells(&self) -> Option<(u32, u32)> {
        match self {
            IconSource::Cell { w, h, .. } => Some((*w, *h)),
            IconSource::Picture { .. } => None,
        }
    }
}

/// A whole number of a section's line.
fn number_of(v: Option<&str>) -> Option<u32> {
    v.and_then(|s| s.trim().trim_matches('"').parse::<f64>().ok()).filter(|n| *n >= 0.0).map(|n| n as u32)
}

/// The icon a section's lines name, as the inventory draws it.
pub(crate) fn cell_icon(get: &dyn Fn(&str) -> Option<String>) -> Option<IconSource> {
    let (x, y) =
        (number_of(get(game_keys::GRID_X).as_deref())?, number_of(get(game_keys::GRID_Y).as_deref())?);
    let w = number_of(get(game_keys::GRID_W).as_deref()).unwrap_or(1).max(1);
    let h = number_of(get(game_keys::GRID_H).as_deref()).unwrap_or(1).max(1);
    let sheet = get(game_keys::ICON_TEXTURE)
        .map(|t| t.trim().trim_matches('"').to_owned())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| game_keys::DEFAULT_ICONS.to_owned());
    Some(IconSource::Cell { sheet, x, y, w, h })
}

/// The icon of a section of a tree (the game's, or the module's composition).
pub(crate) fn section_icon(ltx: &gz_ltx::Ltx, section: &str) -> Option<IconSource> {
    let s = ltx.section(&section.trim().to_ascii_lowercase())?;
    cell_icon(&|k: &str| s.get(k).and_then(|it| it.value.as_deref()).map(str::to_owned))
}

/// The write time of a file of the project, as the file tree has it.
pub(crate) fn stamp_of(editor: &Editor, path: &std::path::Path) -> u64 {
    let Ok((session, project)) = Project::of(editor) else { return 0 };
    let Some(rel) = RelPath::from_fs(&session.root, path) else { return 0 };
    match project.tree.snapshot().entry(&rel).map(|e| e.kind) {
        Some(gz_project::EntryKind::File { size, modified, .. }) => {
            let t = modified
                .and_then(|m| m.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            t ^ size
        }
        _ => 0,
    }
}

/// The icon of an item as the file says it, or as its section resolves: a picture beside the
/// asset, or a cell of a texture.
pub(crate) fn item_icon(
    editor: &Editor,
    root: &std::path::Path,
    item: &Item,
    get: &dyn Fn(&str) -> Option<String>,
) -> Option<IconSource> {
    match &item.icon {
        Some(ItemIcon::Picture { file, .. }) => {
            let path = crate::item_lists::picture_path(root, &item.file, file);
            let stamp = stamp_of(editor, &path);
            Some(IconSource::Picture { path, stamp })
        }
        Some(ItemIcon::Cell { texture, x, y, w, h }) => {
            let from = cell_icon(get);
            let (fw, fh, fsheet) = match &from {
                Some(IconSource::Cell { w, h, sheet, .. }) => (*w, *h, sheet.clone()),
                _ => (1, 1, game_keys::DEFAULT_ICONS.to_owned()),
            };
            Some(IconSource::Cell {
                sheet: texture.clone().unwrap_or(fsheet),
                x: *x,
                y: *y,
                w: w.unwrap_or(fw),
                h: h.unwrap_or(fh),
            })
        }
        None => cell_icon(get),
    }
}

/// The texts the game shows for an item the form does not set: the game's (a change) or the
/// base's (a new item), by `TEXTS`.
pub(crate) type Hints = [String; 6];

async fn text_hints(ed: &Arc<Editor>, res: Option<&Resolution>, change: bool) -> Hints {
    let mut out: Hints = Default::default();
    let Some(res) = res else { return out };
    let view = if change { res.before.as_ref() } else { Some(&res.main) };
    let Some(view) = view else { return out };
    let id_of = |key: &str| -> Option<String> {
        view.get(key)
            .filter(|r| change || r.from != gz_items::From::This)
            .and_then(|r| r.value.clone())
            .map(|v| v.trim().trim_matches('"').to_owned())
            .filter(|v| !v.is_empty())
    };
    let ids = [game_keys::NAME, game_keys::SHORT, game_keys::DESCRIPTION].map(id_of);
    let Ok(link) = gz_game::GameLink::of(ed) else { return out };
    for (li, lang) in ["rus", "eng"].iter().enumerate() {
        let Ok(table) = link.strings(lang).await else { continue };
        for (ki, id) in ids.iter().enumerate() {
            if let Some(text) = id.as_deref().and_then(|id| table.get(id)) {
                out[ki * 2 + li] = text.to_owned();
            }
        }
    }
    out
}

/// What tasks leave for the tab; taken at the start of its next frame.
#[derive(Default)]
struct Inbox {
    roster: Option<Loaded>,
    checked: Option<(u64, Option<Resolution>, Vec<Issue>, Hints)>,
    saved: Option<(Result<String, String>, Then)>,
    deleted: Option<Result<(), String>>,
    made: Option<Result<String, String>>,
    answered: Option<(Answer, Then)>,
    delete_yes: bool,
}

/// A roster as the list shows it.
pub(crate) struct Loaded {
    pub roster: Arc<Roster>,
    pub catalog: Arc<CatalogRead>,
    pub problems: Vec<Vec<Issue>>,
    pub icons: Vec<Option<IconSource>>,
    pub families: Vec<Option<String>>,
}

/// Reads the roster, the catalog and every item's problems and icon. The icons and families come
/// from the composition.
pub(crate) async fn load(ed: &Arc<Editor>, fresh: bool) -> Option<Loaded> {
    let items = Items::of(ed).await.ok()?;
    let roster = if fresh { items.fresh_roster(ed).await } else { items.roster(ed).await };
    let catalog = items.catalog(ed).await;
    let problems = items.all_problems(ed, roster.clone()).await;
    let (r, c, e) = (roster.clone(), catalog.clone(), ed.clone());
    let root = items.root().to_path_buf();
    let (icons, families) = ed
        .exec
        .io(move || {
            let mut icons = Vec::with_capacity(r.entries.len());
            let mut families = Vec::with_capacity(r.entries.len());
            for (entry, applied) in r.entries.iter().zip(&r.composition.applied) {
                let s = r.composition.ltx.section(&applied.section.to_ascii_lowercase());
                let get =
                    |k: &str| s.and_then(|s| s.get(k)).and_then(|it| it.value.as_deref()).map(str::to_owned);
                icons.push(item_icon(&e, &root, &entry.item, &get));
                let class = get("class");
                families.push(class.and_then(|cl| c.catalog.family_of_class(&cl).map(|f| f.title.clone())));
            }
            (icons, families)
        })
        .await;
    Some(Loaded { roster, catalog, problems, icons, families })
}

/// The file a pick of the + menu makes: `items/<section>.itemasset`, or the first of
/// `<section>_2`, `_3`, ... that is free.
fn free_file(project: &Project, section: &str) -> String {
    let stem: String = section
        .trim()
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') { c } else { '_' })
        .collect();
    let stem = if stem.is_empty() { "item".to_owned() } else { stem };
    let tree = project.tree.snapshot();
    let taken = |f: &str| RelPath::new(f).ok().is_some_and(|r| tree.entry(&r).is_some());
    let mut file = format!("{FOLDER}/{stem}.itemasset");
    for n in 2..10_000 {
        if !taken(&file) {
            break;
        }
        file = format!("{FOLDER}/{stem}_{n}.itemasset");
    }
    file
}

/// Makes the file of a pick of the + menu: a new item starting from `section`, or a change of
/// it. Answers the file, project-relative.
pub(crate) async fn make_item(ed: &Arc<Editor>, change: bool, section: &str) -> Result<String, String> {
    let section = section.trim().to_ascii_lowercase();
    if section.is_empty() {
        return Err("no item was picked".into());
    }
    let items = Items::of(ed).await.map_err(|e| e.message)?;
    let (_, project) = Project::of(ed).map_err(|e| e.message)?;
    project.tree.refresh().await;
    let file = free_file(&project, &section);
    let item = if change {
        Item { file: file.clone(), edit: Some(section), ..Item::default() }
    } else {
        Item { file: file.clone(), base: vec![section], ..Item::default() }
    };
    items.save(ed, &file, None, &item).await.map_err(|e| e.message)
}

/// A save as it is carried out.
#[derive(Clone, Debug)]
pub(crate) struct SavePlan {
    pub file: String,
    pub before: Option<Item>,
    pub after: Item,
}

pub(crate) async fn run_save(ed: &Arc<Editor>, plan: SavePlan) -> Result<String, String> {
    let items = Items::of(ed).await.map_err(|e| e.message)?;
    items.save(ed, &plan.file, plan.before.as_ref(), &plan.after).await.map_err(|e| e.message)
}

/// The form's edits (`edit`, read as `base`) put onto the item the file holds now (`disk`);
/// the values both changed keep the form's and come back as conflicts.
pub(crate) fn merge_item(base: &Item, edit: &Item, disk: &Item) -> Option<(Item, Vec<Conflict>)> {
    let rules = diff_rules();
    let (from, mine) = (to_value(base), to_value(edit));
    let theirs = write(disk);
    let (doc, conflicts) =
        LuaDoc::rebase_with(&from, &mine, &theirs, &rules, &disk.file, Limits::asset()).ok()?;
    let (mut merged, _) = from_value(&doc.value()).ok()?;
    merged.file.clone_from(&edit.file);
    Some((merged, conflicts))
}

/// What the drawing asked for, done once it is drawn.
pub(crate) enum Action {
    Select(String),
    Choose(Pick),
    Delete,
    ChoosePicture,
    /// Files dropped on the icon.
    Dropped(Vec<std::path::PathBuf>),
    /// A model of the game in the model viewer.
    OpenModel(String),
    OpenTexture(String),
    Play(String),
}

/// The Items tab.
pub struct ItemsDoc {
    pub(crate) editor: Arc<Editor>,
    root: String,
    pub(crate) loaded: Option<Loaded>,
    roster_key: Option<Rev>,
    loading: bool,
    fresh_wanted: bool,
    pub(crate) module: String,
    /// The file of the item selected.
    pub(crate) sel: Option<String>,
    pub(crate) edit: Item,
    pub(crate) base: Option<Item>,
    pub(crate) conflicts: Vec<Conflict>,
    pub(crate) dirty: bool,
    /// The text fields, by `TEXTS`.
    pub(crate) texts: [String; 6],
    pub(crate) hints: Hints,
    /// What the edited item resolves to and its problems, as the last check found them.
    pub(crate) res: Option<Resolution>,
    pub(crate) own: Vec<Issue>,
    pub(crate) places: Vec<(Place, Tone, String)>,
    pub(crate) edit_gen: u64,
    check_gen: u64,
    checking: bool,
    pub(crate) res_gen: u64,
    pub(crate) form: Form,
    form_key: (u64, u64),
    pub(crate) status: String,
    pub(crate) list_filter: String,
    inbox: Arc<Mutex<Inbox>>,
    pending_open: Option<String>,
    saving: bool,
    save_error: Option<String>,
    pub(crate) lists: ListsView,
    pub(crate) item_lists: ItemListsView,
    pub(crate) pictures: Option<Arc<Pictures>>,
    /// The textures icons are cells of.
    pub(crate) icon_sheets: Option<Arc<crate::item_lists::IconSheets>>,
    pub(crate) actions: Vec<Action>,
    scroll_max: f32,
    pub(crate) model: Option<(String, Box<dyn Document>)>,
    pub(crate) chooser: Option<(Pick, Chooser)>,
    /// The chooser's rows of game items, by the label shown: the section.
    chooser_rows: Arc<FxHashMap<String, String>>,
    /// The HUD row the author is on: what a preview of the hands will pose.
    pub(crate) hud_row: Option<String>,
    /// The icons of sections a row names, by section: worked out once.
    pub(crate) section_icons: FxHashMap<String, Option<IconSource>>,
    /// A question put up to be looked at (`items.view ask_delete=true`).
    look_delete: bool,
    /// The In the Hands picture, once the item has had a HUD section.
    pub(crate) hands: Option<gz_ui_previews::hands::HandsView>,
    /// What the picture was last made for: the edit, the check, the row, the roster read.
    pub(crate) hands_key: Option<(u64, u64, Option<String>, bool)>,
    /// The file the picture was last made for.
    pub(crate) hands_file: Option<String>,
    /// Where the world model was drawn last frame: the wheel over it turns the model, not the
    /// column.
    pub(crate) model_rect: Option<Rect>,
    preview_max: f32,
}

impl ItemsDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>, open: Option<String>) -> Self {
        Self {
            editor: editor.clone(),
            root: String::new(),
            loaded: None,
            roster_key: None,
            loading: false,
            fresh_wanted: true,
            module: String::new(),
            sel: None,
            edit: Item::default(),
            base: None,
            conflicts: Vec::new(),
            dirty: false,
            texts: Default::default(),
            hints: Default::default(),
            res: None,
            own: Vec::new(),
            places: Vec::new(),
            edit_gen: 1,
            check_gen: 0,
            checking: false,
            res_gen: 0,
            form: Form::default(),
            form_key: (0, 0),
            status: String::new(),
            list_filter: String::new(),
            inbox: Arc::new(Mutex::new(Inbox::default())),
            pending_open: open,
            saving: false,
            save_error: None,
            lists: ListsView::default(),
            item_lists: ItemListsView::default(),
            pictures: None,
            icon_sheets: None,
            actions: Vec::new(),
            scroll_max: 0.0,
            model: None,
            chooser: None,
            chooser_rows: Arc::default(),
            hud_row: None,
            section_icons: FxHashMap::default(),
            look_delete: false,
            hands: None,
            hands_key: None,
            hands_file: None,
            model_rect: None,
            preview_max: 0.0,
        }
    }

    // ---- the roster -----------------------------------------------------------------------------

    fn sync(&mut self) {
        let Ok((session, project)) = Project::of(&self.editor) else { return };
        let root = session.root.to_string_lossy().into_owned();
        if root != self.root {
            self.root = root;
            self.loaded = None;
            self.roster_key = None;
            self.sel = None;
            self.edit = Item::default();
            self.dirty = false;
            self.pull();
        }
        let manifest = ManifestService::get(&session).map_or(Rev::ZERO, |m| m.rev());
        let key = project.tree.rev().mix(manifest).mix(project.game_rev());
        if self.loading || self.roster_key == Some(key) {
            return;
        }
        self.roster_key = Some(key);
        self.loading = true;
        let fresh = std::mem::take(&mut self.fresh_wanted);
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let loaded = load(&ed, fresh).await;
            inbox.lock().roster = Some(loaded.unwrap_or_else(|| Loaded {
                roster: Arc::new(Roster::default()),
                catalog: Arc::new(CatalogRead::default()),
                problems: Vec::new(),
                icons: Vec::new(),
                families: Vec::new(),
            }));
            call::wake_ui(&ed);
        });
    }

    pub(crate) fn reload_fresh(&mut self) {
        self.fresh_wanted = true;
        self.roster_key = None;
    }

    #[must_use]
    pub(crate) fn roster(&self) -> Option<&Arc<Roster>> {
        self.loaded.as_ref().map(|l| &l.roster)
    }

    /// A roster arrived: the list as the disk has it, edits in progress kept and put onto what the
    /// file says now; a file asked for is selected.
    pub(crate) fn take_loaded(&mut self, loaded: Loaded) {
        self.module = loaded.roster.module.clone();
        self.section_icons.clear();
        let roster = loaded.roster.clone();
        self.loaded = Some(loaded);
        let found = self.sel.as_deref().and_then(|f| roster.index_of_file(f));
        if self.dirty {
            if let Some(disk) = found.and_then(|i| roster.entries.get(i)).map(|e| e.item.clone()) {
                self.rebase(&disk);
            }
        } else {
            let keep =
                found.or(if self.sel.is_none() && !roster.entries.is_empty() { Some(0) } else { None });
            self.select_index(keep);
        }
        if let Some(path) = self.pending_open.take() {
            match self.find(&path) {
                Some(file) if Some(&file) != self.sel.as_ref() => self.ask_then(Then::Select(file)),
                Some(_) => {}
                // not there yet: the next read finds it
                None if roster.entries.is_empty() || self.loading => self.pending_open = Some(path),
                None => {}
            }
        }
        self.edit_gen += 1;
    }

    /// The file of an item: a file of the roster (either slash, any case) or the section one of
    /// its items makes or changes.
    #[must_use]
    pub(crate) fn find(&self, what: &str) -> Option<String> {
        let roster = self.roster()?;
        roster.find(what).and_then(|i| roster.entries.get(i)).map(|e| e.item.file.clone())
    }

    fn select_index(&mut self, index: Option<usize>) {
        let picked = index.and_then(|i| self.roster().and_then(|r| r.entries.get(i)).map(|e| e.item.clone()));
        // the same item read again keeps the row the author is on
        let same = picked.as_ref().is_some_and(|p| self.sel.as_deref() == Some(p.file.as_str()));
        let row = self.hud_row.take().filter(|_| same);
        // and, when its file says what the form held, what it resolved to until the next check:
        // the column does not lose its picture and its model for a frame
        let unchanged = same && picked.as_ref() == Some(&self.edit);
        match picked {
            Some(item) => {
                self.sel = Some(item.file.clone());
                self.base = Some(item.clone());
                self.edit = item;
            }
            None => {
                self.sel = None;
                self.base = None;
                self.edit = Item::default();
            }
        }
        self.conflicts.clear();
        self.dirty = false;
        if !unchanged {
            self.res = None;
            self.own.clear();
            self.places.clear();
            self.hints = Default::default();
        }
        self.hud_row = row;
        self.pull();
        self.edit_gen += 1;
    }

    /// Selects the item of a file, its edits gone.
    pub(crate) fn select(&mut self, file: Option<&str>) {
        let index = file.and_then(|f| self.roster().and_then(|r| r.index_of_file(f)));
        self.select_index(index);
    }

    fn rebase(&mut self, disk: &Item) {
        let Some(base) = self.base.clone() else { return };
        if *disk == base {
            return;
        }
        if let Some((item, conflicts)) = merge_item(&base, &self.edit, disk) {
            self.edit = item;
            self.conflicts = conflicts;
            self.pull();
        }
        self.base = Some(disk.clone());
        self.dirty = self.base.as_ref() != Some(&self.edit);
    }

    pub(crate) fn take_theirs(&mut self, index: usize) {
        if index >= self.conflicts.len() {
            return;
        }
        let c = self.conflicts.remove(index);
        let mut v = to_value(&self.edit);
        set_at(&mut v, &c.path, c.theirs.unwrap_or(LuaValue::Nil));
        if let Ok((mut taken, _)) = from_value(&v) {
            taken.file.clone_from(&self.edit.file);
            self.edit = taken;
            self.pull();
        }
        self.dirty = self.base.as_ref() != Some(&self.edit);
        self.edit_gen += 1;
    }

    fn text_value(&mut self, field: &str) -> &mut LuaValue {
        match field {
            "name" => &mut self.edit.name,
            "short_name" => &mut self.edit.short_name,
            _ => &mut self.edit.description,
        }
    }

    /// The text fields take what is stored.
    fn pull(&mut self) {
        for (i, (field, lang)) in TEXTS.iter().enumerate() {
            let v = self.text_value(field).clone();
            self.texts[i] = text_raw(&v, lang);
        }
    }

    /// The text fields go back into the item, every language kept, a `rus` alone a plain string.
    pub(crate) fn push_texts(&mut self) {
        let texts = self.texts.clone();
        for (i, (field, lang)) in TEXTS.iter().enumerate() {
            let v = self.text_value(field);
            text_set(v, lang, &texts[i]);
        }
        for field in ["name", "short_name", "description"] {
            text_collapse(self.text_value(field));
        }
    }

    /// The form changed the item.
    pub(crate) fn changed(&mut self) {
        self.push_texts();
        self.dirty = self.base.as_ref() != Some(&self.edit);
        self.edit_gen += 1;
    }

    /// The item is checked again when it changed: one check at a time, the newest state next.
    fn recheck(&mut self) {
        if self.sel.is_none() || self.checking || self.check_gen == self.edit_gen {
            return;
        }
        self.checking = true;
        let (ed, inbox, serial, edit) =
            (self.editor.clone(), self.inbox.clone(), self.edit_gen, self.edit.clone());
        let _task = self.editor.exec.spawn(async move {
            let (res, issues, hints) = check(&ed, &edit).await;
            inbox.lock().checked = Some((serial, res, issues, hints));
            call::wake_ui(&ed);
        });
    }

    /// A check's answer, for the item as it was then.
    pub(crate) fn set_checked(
        &mut self,
        serial: u64,
        res: Option<Resolution>,
        issues: Vec<Issue>,
        hints: Hints,
    ) {
        self.check_gen = serial;
        self.res = res;
        self.places = issues
            .iter()
            .filter(|i| i.severity != gz_base::Severity::Note)
            .map(|i| {
                (
                    item_model::place_of(i),
                    if i.is_error() { Tone::Error } else { Tone::Warning },
                    item_model::sentence(i),
                )
            })
            .collect();
        self.own = issues;
        self.hints = hints;
        self.res_gen += 1;
    }

    /// The form's model, made again when the item or what it resolves to changed.
    pub(crate) fn refresh_form(&mut self) {
        let key = (self.edit_gen, self.res_gen);
        if self.form_key == key {
            return;
        }
        self.form_key = key;
        let empty = gz_items::Catalog::default();
        let catalog = self.loaded.as_ref().map_or(&empty, |l| &l.catalog.catalog);
        self.form = item_model::build(&self.edit, self.res.as_ref(), catalog);
    }

    /// Whether the check of the item as it is now is in.
    #[must_use]
    pub(crate) fn checked_now(&self) -> bool {
        self.check_gen == self.edit_gen
    }

    // ---- the inbox ------------------------------------------------------------------------------

    fn take_inbox(&mut self) {
        let mut inbox = self.inbox.lock();
        let roster = inbox.roster.take();
        let checked = inbox.checked.take();
        let saved = inbox.saved.take();
        let deleted = inbox.deleted.take();
        let made = inbox.made.take();
        let answered = inbox.answered.take();
        let delete_yes = std::mem::take(&mut inbox.delete_yes);
        drop(inbox);
        if let Some(loaded) = roster {
            self.loading = false;
            self.take_loaded(loaded);
        }
        if let Some((serial, res, issues, hints)) = checked {
            self.checking = false;
            self.set_checked(serial, res, issues, hints);
        }
        if let Some((result, then)) = saved {
            self.saved(result, then);
        }
        if let Some(result) = deleted {
            match result {
                Ok(()) => {
                    self.status.clear();
                    self.select_index(None);
                }
                Err(e) => self.status = format!("Not deleted: {e}"),
            }
            self.reload_fresh();
        }
        if let Some(result) = made {
            match result {
                Ok(file) => {
                    self.status.clear();
                    self.pending_open = Some(file);
                    self.reload_fresh();
                }
                Err(e) => self.status = format!("Not made: {e}"),
            }
        }
        if let Some((answer, then)) = answered {
            match answer {
                Answer::Cancel => {}
                Answer::Yes => {
                    if let Err(e) = self.start_save(then) {
                        self.status = format!("Not saved: {e}");
                    }
                }
                Answer::No => {
                    self.discard_edits();
                    self.run_then(then);
                }
            }
        }
        if delete_yes {
            self.delete_now();
        }
    }

    // ---- saving ---------------------------------------------------------------------------------

    pub(crate) fn save_plan(&mut self) -> Result<SavePlan, String> {
        if self.edit.file.is_empty() {
            return Err("no item is selected".into());
        }
        self.push_texts();
        // a list change of a row of More Sections that names no member says nothing
        for s in &mut self.edit.sections {
            s.changes.add.retain(|(_, m)| !m.is_empty());
            s.changes.drop.retain(|(_, m)| !m.is_empty());
        }
        Ok(SavePlan { file: self.edit.file.clone(), before: self.base.clone(), after: self.edit.clone() })
    }

    fn start_save(&mut self, then: Then) -> Result<(), String> {
        if self.saving {
            return Ok(());
        }
        let plan = self.save_plan()?;
        self.saving = true;
        self.save_error = None;
        self.dirty = false;
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let _task = self.editor.exec.spawn(async move {
            let result = run_save(&ed, plan).await;
            inbox.lock().saved = Some((result, then));
            call::wake_ui(&ed);
        });
        Ok(())
    }

    pub(crate) fn saved(&mut self, result: Result<String, String>, then: Then) {
        self.saving = false;
        match result {
            Ok(file) => {
                self.status.clear();
                self.dirty = false;
                self.edit.file.clone_from(&file);
                self.sel = Some(file);
                self.base = Some(self.edit.clone());
                self.conflicts.clear();
                self.reload_fresh();
                self.run_then(then);
            }
            Err(e) => {
                self.dirty = true;
                self.status = format!("Not saved: {e}");
                self.save_error = Some(e);
            }
        }
    }

    pub(crate) fn discard_edits(&mut self) {
        let sel = self.sel.clone();
        self.select(sel.as_deref());
        self.status.clear();
    }

    fn run_then(&mut self, then: Then) {
        match then {
            Then::Nothing => {}
            Then::Select(file) => self.select(Some(&file)),
            Then::Choose(pick) => self.open_chooser(pick),
        }
    }

    /// Unsaved edits ask before anything replaces them: Save, Don't Save, or stay.
    pub(crate) fn ask_then(&mut self, then: Then) {
        if !self.dirty {
            self.run_then(then);
            return;
        }
        let what: Vec<Tx<'static>> = vec![Tx::raw(self.edit.file.clone())];
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let Ok(queue) = gz_app::UiQueue::of(&self.editor) else { return };
        queue.post(move |_, ctx| {
            let inbox2 = inbox.clone();
            let ed2 = ed.clone();
            let line = unsaved_prompt(ctx, call::asker(&ed), "item", &what, move |answer| {
                inbox2.lock().answered = Some((answer, then));
                call::wake_ui(&ed2);
            });
            call::log_line(&ed, line);
            ctx.request_repaint();
        });
    }

    // ---- the choosers ---------------------------------------------------------------------------

    /// The game's items as a chooser lists them: the family as the folder, the name the player
    /// reads with the section as the leaf.
    fn game_item_rows(&mut self) -> Vec<String> {
        let mut rows = FxHashMap::default();
        let mut out = Vec::with_capacity(self.item_lists.game_items.len());
        for g in self.item_lists.game_items.iter() {
            let clean = |s: &str| s.replace(['\\', '/'], "-");
            let leaf = if g.name.is_empty() {
                g.section.clone()
            } else {
                format!("{}  ({})", clean(&g.name), g.section)
            };
            let label =
                if g.family_title.is_empty() { leaf } else { format!("{}\\{leaf}", clean(&g.family_title)) };
            rows.insert(label.clone(), g.section.clone());
            out.push(label);
        }
        self.chooser_rows = Arc::new(rows);
        out
    }

    /// The inventory icons of the chooser's rows of items: a row's section, its cell of the icon
    /// sheet (worked out once per section), the sheet read off the UI thread.
    fn row_icons(&self) -> Option<RowIcons> {
        let sheets = self.icon_sheets.clone()?;
        let comp = self.loaded.as_ref()?.roster.composition.clone();
        let (editor, rows) = (self.editor.clone(), self.chooser_rows.clone());
        let looks: Arc<Mutex<FxHashMap<String, Option<IconSource>>>> = Arc::default();
        Some(RowIcons(Arc::new(move |label: &str| {
            let section = rows.get(label)?;
            let look = looks
                .lock()
                .entry(section.clone())
                .or_insert_with(|| section_icon(&comp.ltx, section))
                .clone();
            let Some(IconSource::Cell { sheet, x, y, w, h }) = look else { return None };
            let c = crate::item_form::CELL;
            let px = [x as f32 * c, y as f32 * c, w as f32 * c, h as f32 * c];
            let (tex, uv) = sheets.cell(&editor, &sheet, px)?;
            Some((tex, uv, w as f32 / h.max(1) as f32))
        })))
    }

    /// Puts a chooser of the form up.
    pub(crate) fn open_chooser(&mut self, pick: Pick) {
        let purpose = Purpose::Property { target: 0, key: pick.word() };
        let chooser = match &pick {
            Pick::New | Pick::Change | Pick::Base => {
                let rows = self.game_item_rows();
                let mut c = Chooser::new(purpose, "Item", None, "").with_items(rows);
                if let Some(icons) = self.row_icons() {
                    c = c.with_icons(icons);
                }
                c.title = match pick {
                    Pick::New => Tx::from("New Item"),
                    Pick::Change => Tx::from("Change a Game Item"),
                    _ => Tx::from("Starts From"),
                };
                c
            }
            Pick::Key { scope, key } => {
                let row = self
                    .form
                    .groups
                    .iter()
                    .chain(self.form.hud.iter().flat_map(|(_, g)| g.iter()))
                    .flat_map(|g| g.rows.iter())
                    .find(|r| r.scope == *scope && r.key == *key);
                let Some(row) = row else { return };
                let current = row.shown().map(|v| first_name(&v)).unwrap_or_default();
                let title = tr!("Select {what}", what = crate::item_form::row_label(row));
                let section = row.ty == gz_items::KeyType::Section;
                let mut c = if section {
                    // a row that names an item picks it among the game's items, by their icons
                    let rows = self.game_item_rows();
                    let now = rows.iter().find(|l| self.chooser_rows.get(*l) == Some(&current)).cloned();
                    let mut c =
                        Chooser::new(purpose, "Item", None, now.as_deref().unwrap_or("")).with_items(rows);
                    if let Some(icons) = self.row_icons() {
                        c = c.with_icons(icons);
                    }
                    c
                } else {
                    let Some((list, what)) = list_of(&row.ty) else { return };
                    Chooser::new(purpose, what, Some(list), &current)
                };
                c.title = title;
                c
            }
        };
        self.chooser = Some((pick, chooser));
    }

    fn picked(&mut self, pick: Pick, items: Vec<String>) {
        let Some(item) = items.into_iter().next() else { return };
        match pick {
            Pick::New | Pick::Change => {
                let Some(section) = self.chooser_rows.get(&item).cloned() else { return };
                let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
                let change = pick == Pick::Change;
                let _task = self.editor.exec.spawn(async move {
                    let made = make_item(&ed, change, &section).await;
                    inbox.lock().made = Some(made);
                    call::wake_ui(&ed);
                });
            }
            Pick::Base => {
                let Some(section) = self.chooser_rows.get(&item).cloned() else { return };
                if !self.edit.base.iter().any(|b| b.eq_ignore_ascii_case(&section)) {
                    self.edit.base.push(section);
                    self.changed();
                }
            }
            Pick::Key { scope, key } => {
                // a row of the item chooser is the section it names
                let value = self.chooser_rows.get(&item).cloned().unwrap_or(item);
                let changes = self.changes_mut(&scope);
                item_model::set_own(changes, &key, gz_items::SetValue::Text(value));
                self.changed();
            }
        }
    }

    /// The changes of a scope's section: the item's own, or its HUD's.
    pub(crate) fn changes_mut(&mut self, scope: &str) -> &mut gz_items::Changes {
        if scope == gz_items::catalog::HUD {
            &mut self.edit.hud.get_or_insert_with(gz_items::Hud::default).changes
        } else {
            &mut self.edit.changes
        }
    }

    fn draw_chooser(&mut self, ui: &mut Ui, cx: &DocCx<'_>) {
        let Some((pick, mut chooser)) = self.chooser.take() else { return };
        let gpu = Some((&cx.gpu.device, &cx.gpu.queue));
        match choose::draw(ui, &self.editor, gpu, &mut chooser) {
            Outcome::Open => self.chooser = Some((pick, chooser)),
            Outcome::Cancelled => {}
            Outcome::Picked(items) => self.picked(pick, items),
        }
    }

    // ---- deleting, previews ---------------------------------------------------------------------

    fn ask_delete(&mut self, ctx: &egui::Context, asker: gz_ui::Asker) {
        if self.edit.file.is_empty() {
            return;
        }
        let section = self.res.as_ref().map(|r| r.section.clone()).unwrap_or_default();
        let headline = if section.is_empty() {
            tr!("Delete the item file \"{file}\"?", file = self.edit.file.as_str())
        } else {
            tr!("Delete the item \"{section}\"?", section = section.as_str())
        };
        let spec = ConfirmSpec {
            id: "delete_item".to_owned(),
            title: "Delete Item".into(),
            headline,
            items: vec![Tx::raw(self.edit.file.clone())],
            total: 0,
            consequence: "Deleted for good - this cannot be undone. The next build takes the item out of the module; a change of a game item gives the game's item back as it was."
                .into(),
            action: "Delete".into(),
            destructive: true,
        };
        let (ed, inbox) = (self.editor.clone(), self.inbox.clone());
        let line = confirm(ctx, asker, spec, move |answer| {
            if answer == Answer::Yes {
                inbox.lock().delete_yes = true;
                call::wake_ui(&ed);
            }
        });
        call::log_line(&self.editor, line);
    }

    fn delete_now(&mut self) {
        let file = self.edit.file.clone();
        if file.is_empty() {
            return;
        }
        let inbox = self.inbox.clone();
        call::call_then(&self.editor, "file.delete", args([("path", s(&file))]), move |reply, _| {
            inbox.lock().deleted = Some(if call::ok(reply) { Ok(()) } else { Err(call::error_text(reply)) });
        });
    }

    fn choose_picture(&mut self, ctx: &egui::Context) {
        let ed = self.editor.clone();
        let file = self.edit.file.clone();
        let request = gz_ui::file_dialog::FileRequest::open("Choose Picture")
            .filter("Pictures", &["png", "tga", "jpg", "jpeg", "bmp", "dds"]);
        let picked = gz_ui::file_dialog::pick(
            &self.editor.exec,
            Some(ctx),
            call::asker(&self.editor),
            request,
            move |path| {
                take_picture(&ed, &file, path);
            },
        );
        if let Err(line) = picked {
            self.editor.log.finding(&line);
        }
    }

    fn run_actions(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.actions) {
            match action {
                Action::Select(file) => self.ask_then(Then::Select(file)),
                Action::Choose(pick @ (Pick::New | Pick::Change)) => self.ask_then(Then::Choose(pick)),
                Action::Choose(pick) => self.open_chooser(pick),
                Action::Delete => self.ask_delete(ctx, call::asker(&self.editor)),
                Action::ChoosePicture => self.choose_picture(ctx),
                Action::Dropped(files) => {
                    if let Some(path) = files.into_iter().find(|p| is_picture(p)) {
                        take_picture(&self.editor, &self.edit.file, path);
                    }
                }
                Action::OpenModel(visual) => open_model(&self.editor, &visual),
                Action::OpenTexture(name) => open_texture(&self.editor, &name),
                Action::Play(name) => play(&self.editor, &name),
            }
        }
    }

    /// A picture taken for the icon: the file's name beside the asset, or the copy made beside it.
    pub(crate) fn set_picture(&mut self, file: &str) {
        let (w, h) = match &self.edit.icon {
            Some(ItemIcon::Picture { w, h, .. }) => (*w, *h),
            _ => (None, None),
        };
        self.edit.icon =
            Some(ItemIcon::Picture { file: file.to_owned(), w, h, plain: w.is_none() && h.is_none() });
        self.changed();
    }

    // ---- MCP ------------------------------------------------------------------------------------

    /// The list as `items.view` answers it.
    #[must_use]
    pub(crate) fn list_json(&self) -> Vec<Value> {
        let Some(l) = &self.loaded else { return Vec::new() };
        l.roster
            .entries
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let section =
                    l.roster.composition.applied.get(i).map(|a| a.section.clone()).unwrap_or_default();
                let problems = l
                    .problems
                    .get(i)
                    .map_or(0, |p| p.iter().filter(|x| x.severity != gz_base::Severity::Note).count());
                let selected = self.sel.as_deref().is_some_and(|f| f.eq_ignore_ascii_case(&e.item.file));
                json!({
                    "file": e.item.file,
                    "kind": if item_model::is_change(&e.item) { "change" } else { "new" },
                    "section": section,
                    "name": list_name(&e.item, &self.item_lists, &section),
                    "family": l.families.get(i).cloned().flatten(),
                    "template": e.item.is_abstract(),
                    "dirty": selected && self.dirty,
                    "problems": problems,
                    "icon": l.icons.get(i).cloned().flatten().map(|s| icon_json(&s)),
                })
            })
            .collect()
    }

    #[must_use]
    pub fn view_json(&self, active: bool) -> Value {
        let picker = self.chooser.as_ref().map(|(p, c)| json!({ "show": p.word(), "chooser": c.state() }));
        json!({
            "tab": TAB,
            "open": true,
            "active": active,
            "selected": self.sel.clone().unwrap_or_default(),
            "dirty": self.dirty,
            "checked": self.checked_now(),
            "count": self.roster().map_or(0, |r| r.entries.len()),
            "items": self.list_json(),
            "form": if self.sel.is_some() { crate::item_view_cmd::form_json(self) } else { Value::Null },
            "conflicts": self.conflicts.iter().map(crate::char_form::conflict_line).collect::<Vec<_>>(),
            "hud_row": self.hud_row,
            "picker": picker,
            "status": self.status,
            "model": self.model.as_ref().map(|(key, d)| json!({ "visual": key, "viewer": d.mcp_state() })),
            "hands": self.hands_json(),
        })
    }

    #[must_use]
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Puts the Delete Item question up to be looked at (a person's window answers it).
    pub(crate) fn look_at_delete(&mut self) {
        self.look_delete = true;
    }

    // ---- drawing --------------------------------------------------------------------------------

    fn draw_body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let full = ui.max_rect();
        let mut head = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.items.split")
                .max_rect(full)
                .layout(Layout::left_to_right(Align::Min)),
        );
        let split = gz_ui::splitter::splitter(&mut head, "items.list", true, 220.0, 400.0, 260.0);
        let gap = head.spacing().item_spacing.x;
        let list_rect =
            Rect::from_min_max(full.min, pos2((full.min.x + split.size).min(full.max.x), full.max.y));
        let mut form_rect =
            Rect::from_min_max(pos2((list_rect.max.x + gap).min(full.max.x), full.min.y), full.max);
        let form_w = gz_ui::metrics::FORM_W + 24.0;
        if form_rect.width() >= form_w + PREVIEW_MIN && self.sel.is_some() {
            let preview = Rect::from_min_max(
                pos2(form_rect.min.x + form_w + gap, full.min.y + 8.0),
                pos2(full.max.x - 12.0, full.max.y - 8.0),
            );
            form_rect.max.x = form_rect.min.x + form_w;
            let mut side = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-world.items.preview")
                    .max_rect(preview)
                    .layout(Layout::top_down(Align::Min)),
            );
            let mut scroll = ScrollArea::vertical()
                .id_salt("gz-ui-world.items.preview_scroll")
                .auto_shrink([false, false]);
            // the wheel over the world model turns the model, not the column
            let over_model =
                self.model_rect.zip(ctx.pointer_hover_pos()).is_some_and(|(r, at)| r.contains(at));
            if over_model {
                scroll = scroll.scroll_source(egui::containers::scroll_area::ScrollSource {
                    mouse_wheel: false,
                    ..Default::default()
                });
            }
            match gz_app::take_scroll(&ctx, PREVIEW_SCROLL) {
                Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
                Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(self.preview_max),
                Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
                _ => {}
            }
            let view_h = preview.height();
            let out = scroll.show(&mut side, |ui| self.draw_preview(ui, cx, view_h));
            let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
            self.preview_max = max;
            gz_app::report_scroll(&ctx, PREVIEW_SCROLL, out.state.clamped_offset().y.min(max), max);
        } else {
            self.model_rect = None;
        }
        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.items.side")
                .max_rect(list_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        self.draw_list(&mut left);
        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-ui-world.items.form")
                .max_rect(form_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        let mut scroll =
            ScrollArea::vertical().id_salt("gz-ui-world.items.scroll").auto_shrink([false, false]);
        match gz_app::take_scroll(&ctx, HOST) {
            Some(gz_app::ScrollTo::Top) => scroll = scroll.vertical_scroll_offset(0.0),
            Some(gz_app::ScrollTo::Bottom) => scroll = scroll.vertical_scroll_offset(self.scroll_max),
            Some(gz_app::ScrollTo::Offset(y)) => scroll = scroll.vertical_scroll_offset(y),
            _ => {}
        }
        let out = scroll.show(&mut right, |ui| {
            egui::Frame::new().inner_margin(egui::Margin::symmetric(12, 0)).show(ui, |ui| {
                let column = ui.available_width().clamp(1.0, gz_ui::metrics::FORM_W);
                ui.set_max_width(column);
                ui.set_min_width(column);
                if self.sel.is_none() {
                    if gz_ui::state::empty_state(
                        ui,
                        Icon::Item,
                        "No item selected",
                        "Pick one in the list, or add one with +.",
                        Some("New Item...".into()),
                    ) {
                        self.actions.push(Action::Choose(Pick::New));
                    }
                } else {
                    self.draw_form(ui, column);
                }
            });
        });
        let max = (out.content_size.y - out.inner_rect.height()).max(0.0);
        self.scroll_max = max;
        gz_app::report_scroll(&ctx, HOST, out.state.clamped_offset().y.min(max), max);
        ui.allocate_rect(full, egui::Sense::hover());
    }
}

/// The name the player reads of an item of the list: its own text, else the game's.
pub(crate) fn list_name(item: &Item, lists: &ItemListsView, section: &str) -> String {
    let own = text_in(&item.name, "eng");
    let own = if own.is_empty() { text_in(&item.name, "rus") } else { own };
    if !own.is_empty() {
        return own;
    }
    let from = item.edit.as_deref().or_else(|| item.base.first().map(String::as_str)).unwrap_or(section);
    lists.game_item(from).map(|g| g.name.clone()).unwrap_or_default()
}

pub(crate) fn icon_json(s: &IconSource) -> Value {
    match s {
        IconSource::Picture { path, .. } => json!({ "picture": path.to_string_lossy() }),
        IconSource::Cell { sheet, x, y, w, h } => json!({ "texture": sheet, "x": x, "y": y, "w": w, "h": h }),
    }
}

/// What a chooser lists for a key of this type, and its noun.
pub(crate) fn list_of(ty: &gz_items::KeyType) -> Option<(ListKey, &'static str)> {
    use gz_items::KeyType as T;
    match ty {
        T::Visual => Some((ListKey::Visuals, "Model")),
        T::Texture => Some((ListKey::Textures, "Texture")),
        T::Sound => Some((ListKey::Sounds, "Sound")),
        T::Particles => Some((ListKey::Particles, "Particle System")),
        _ => None,
    }
}

/// The first name of a value: a sound's file, without what follows it.
pub(crate) fn first_name(v: &str) -> String {
    v.split(',').next().unwrap_or("").trim().trim_matches('"').to_owned()
}

/// Whether a file is a picture an icon takes.
pub(crate) fn is_picture(p: &std::path::Path) -> bool {
    p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
        ["png", "tga", "jpg", "jpeg", "bmp", "dds"].iter().any(|x| e.eq_ignore_ascii_case(x))
    })
}

/// The check of an item as it is edited, with the texts the game shows where it sets none.
pub(crate) async fn check(ed: &Arc<Editor>, edit: &Item) -> (Option<Resolution>, Vec<Issue>, Hints) {
    let Ok(items) = Items::of(ed).await else { return (None, Vec::new(), Hints::default()) };
    let (res, issues) = items.check_edit(ed, edit).await;
    let hints = text_hints(ed, res.as_ref(), item_model::is_change(edit)).await;
    (res, issues, hints)
}

/// A picture for the icon of the item in `asset`: kept where it is when it lies in the project,
/// copied beside the asset otherwise (never over another file). The icon then names it.
fn take_picture(editor: &Arc<Editor>, asset: &str, path: std::path::PathBuf) {
    let (ed, asset) = (editor.clone(), asset.to_owned());
    let _task = editor.exec.spawn(async move {
        let Ok((session, project)) = Project::of(&ed) else { return };
        let root = session.root.clone();
        let guard = project.write_guard(None);
        let asset_abs = root.join(asset.replace('/', "\\"));
        let result = ed
            .exec
            .io(move || -> Result<String, String> {
                let dir =
                    asset_abs.parent().map(std::path::Path::to_path_buf).unwrap_or_else(|| root.clone());
                let target = if gz_media::files::inside(&root, &path) {
                    path
                } else {
                    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    let (stem, ext) = match name.rsplit_once('.') {
                        Some((s, e)) => (s.to_owned(), format!(".{e}")),
                        None => (name.clone(), String::new()),
                    };
                    let mut dst = dir.join(&name);
                    let mut n = 2;
                    while dst.exists() {
                        dst = dir.join(format!("{stem}_{n}{ext}"));
                        n += 1;
                    }
                    guard.check(&dst).map_err(|e| e.to_string())?;
                    std::fs::copy(&path, &dst).map_err(|e| gz_base::io_reason(&e))?;
                    dst
                };
                Ok(relative(&dir, &target))
            })
            .await;
        project.tree.refresh().await;
        let _ = gz_app::on_ui(&ed, move |shell, ctx| {
            if let Some(doc) = crate::item_view_cmd::items_doc(shell) {
                match result {
                    Ok(file) => doc.set_picture(&file),
                    Err(e) => doc.status = format!("The picture was not taken: {e}"),
                }
            }
            ctx.request_repaint();
            Ok(())
        })
        .await;
    });
}

/// `target` as a path from `dir`, with `/` and `..` where it climbs.
pub(crate) fn relative(dir: &std::path::Path, target: &std::path::Path) -> String {
    let lower = |p: &std::path::Path| -> Vec<String> {
        p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect()
    };
    let (a, b) = (lower(dir), lower(target));
    let common = a.iter().zip(&b).take_while(|(x, y)| x.eq_ignore_ascii_case(y)).count();
    let mut parts: Vec<String> = std::iter::repeat_n("..".to_owned(), a.len() - common).collect();
    parts.extend(b.iter().skip(common).cloned());
    parts.join("/")
}

fn open_model(editor: &Arc<Editor>, visual: &str) {
    let ed = editor.clone();
    call::call_then(
        editor,
        "preview.model",
        args([("name", s(visual)), ("source", s("game"))]),
        move |reply, ctx| {
            if !call::ok(reply) {
                gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
                ed.log.warn(format!("the model preview did not open: {}", call::error_text(reply)));
            }
        },
    );
}

fn open_texture(editor: &Arc<Editor>, name: &str) {
    let path = format!("textures\\{}.dds", name.trim().trim_end_matches(".dds"));
    call::call_then(
        editor,
        "preview.texture",
        args([("action", s("open")), ("path", s(&path)), ("source", s("game"))]),
        move |reply, ctx| {
            if !call::ok(reply) {
                gz_ui::state::toast(ctx, Tone::Error, Tx::ui(call::error_text(reply)));
            }
        },
    );
}

/// Plays a sound of the game, decoded off the UI thread; a second press while it plays stops it.
fn play(editor: &Arc<Editor>, name: &str) {
    let player = gz_ui_previews::audio::Player::of(editor);
    if player.is_playing() {
        player.stop();
        return;
    }
    let ed = editor.clone();
    let rel = format!("sounds/{}.ogg", name.trim().replace('\\', "/").to_lowercase());
    editor.exec.spawn(async move {
        let Ok(link) = gz_game::GameLink::of(&ed) else { return };
        let Ok(files) = link.files().await else { return };
        let pcm =
            ed.exec.io(move || files.read(&rel).ok().and_then(|b| gz_fmt_sound::decode_ogg(&b).ok())).await;
        match pcm {
            Some(pcm) => gz_ui_previews::audio::Player::of(&ed).play(Arc::new(pcm)),
            None => ed.notices.error("The sound could not be read.".to_owned()),
        }
    });
}

impl Document for ItemsDoc {
    fn id(&self) -> &str {
        TAB
    }
    fn kind(&self) -> &'static str {
        TAB
    }
    fn label(&self) -> Tx<'static> {
        Tx::from("Items")
    }
    fn icon(&self) -> Icon {
        Icon::Item
    }
    fn tooltip(&self) -> Option<Tx<'static>> {
        Some(Tx::from("The module's inventory items: new ones and changes of the game's"))
    }
    fn dirty(&self) -> bool {
        self.dirty || self.saving
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let project = cx.editor.project_opt().is_some();
        let some = self.sel.is_some();
        tb.group(3);
        // Build Mod is the one Primary of every row: a Save with edits stands out as a Secondary
        let kind = if self.dirty { Kind::Secondary } else { Kind::Ghost };
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            "Write the item into its file (Ctrl+S)",
            self.dirty,
            false,
            kind,
        ) && let Err(e) = self.start_save(Then::Nothing)
        {
            self.status = e;
        }
        if tb.action(
            "revert",
            Icon::Undo,
            "Revert",
            "Drop the edits: the item as its file says",
            self.dirty,
            false,
            Kind::Ghost,
        ) {
            self.discard_edits();
        }
        if tb.action(
            "delete",
            Icon::Delete,
            "Delete...",
            "Delete the item's file for good",
            some,
            false,
            Kind::GhostDanger,
        ) {
            self.actions.push(Action::Delete);
        }
        tb.group(1);
        if tb.action(
            "reload",
            Icon::Refresh,
            "Reload",
            "Read the items, the linked game's items and its catalog again",
            project,
            false,
            Kind::Ghost,
        ) {
            self.reload_fresh();
            if let Some(lists) = Lists::of(cx.editor) {
                lists.force(cx.editor);
            }
            if let Some(lists) = ItemLists::of(cx.editor) {
                lists.force(cx.editor);
            }
            if let Some(rigs) = gz_ui_previews::hands::Rigs::of(cx.editor) {
                rigs.clear();
            }
            if let Some(h) = self.hands.as_mut() {
                h.forget();
            }
            if let Some(s) = &self.icon_sheets {
                s.clear();
            }
            self.hands_key = None;
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        self.take_inbox();
        if cx.editor.project_opt().is_none() {
            gz_ui::state::empty_state(
                ui,
                Icon::Item,
                "No project",
                "Items are .itemasset files of the open project.",
                None,
            );
            return;
        }
        self.sync();
        if let Some(lists) = Lists::of(cx.editor) {
            lists.want(cx.editor);
            self.lists = lists.view();
        }
        if let Some(lists) = ItemLists::of(cx.editor) {
            lists.want(cx.editor);
            self.item_lists = lists.view();
        }
        if self.pictures.is_none() {
            self.pictures = Pictures::of(cx.editor);
        }
        if self.icon_sheets.is_none() {
            self.icon_sheets = crate::item_lists::IconSheets::of(cx.editor);
        }
        self.recheck();
        self.refresh_form();
        self.draw_body(ui, cx);
        self.draw_chooser(ui, cx);
        if std::mem::take(&mut self.look_delete) {
            self.ask_delete(&ctx, gz_ui::Asker::Ui);
        }
        self.run_actions(&ctx);
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.start_save(Then::Nothing)
    }

    fn save_failed(&mut self) -> Option<gz_editor::CmdError> {
        self.take_inbox();
        self.save_error.take().map(|e| gz_editor::CmdError::new("failed", e))
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.discard_edits();
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.reload_fresh();
    }

    fn mcp_state(&self) -> Value {
        self.view_json(false)
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The `items.view` answer of a tab that is not open.
#[must_use]
pub fn closed_json() -> Value {
    json!({ "tab": TAB, "open": false, "active": false, "selected": "", "dirty": false, "count": 0, "items": [], "form": null })
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.document(TAB, |editor, req| {
        if editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        Ok(Box::new(ItemsDoc::new(editor, req.path.clone())))
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_from_the_asset_climbs_where_it_must() {
        let dir = std::path::Path::new("D:\\p\\items");
        assert_eq!(relative(dir, std::path::Path::new("D:\\p\\items\\icons\\a.png")), "icons/a.png");
        assert_eq!(relative(dir, std::path::Path::new("D:\\p\\art\\a.png")), "../art/a.png");
        assert_eq!(first_name("weapons\\ak74_shoot, 1, 0"), "weapons\\ak74_shoot");
        assert!(is_picture(std::path::Path::new("x.PNG")) && !is_picture(std::path::Path::new("x.psd")));
    }

    #[test]
    fn an_icon_is_read_from_a_sections_lines() {
        let lines = |k: &str| match k {
            "inv_grid_x" => Some("3".to_owned()),
            "inv_grid_y" => Some("7".to_owned()),
            "inv_grid_width" => Some("2".to_owned()),
            _ => None,
        };
        assert_eq!(
            cell_icon(&lines),
            Some(IconSource::Cell { sheet: game_keys::DEFAULT_ICONS.into(), x: 3, y: 7, w: 2, h: 1 })
        );
        assert_eq!(cell_icon(&|_: &str| None), None);
    }

    #[test]
    fn a_closed_tab_reports_nothing_selected() {
        let v = closed_json();
        assert_eq!(v["tab"], "items");
        assert_eq!(v["open"], false);
    }

    #[test]
    fn edits_merge_with_a_change_on_disk() {
        let mut base =
            Item { file: "items/a.itemasset".into(), edit: Some("bread".into()), ..Item::default() };
        base.changes.set.push(("cost".into(), gz_items::SetValue::Number(1.0)));
        let mut edit = base.clone();
        edit.changes.set[0].1 = gz_items::SetValue::Number(5.0);
        let mut disk = base.clone();
        disk.changes.set.push(("inv_weight".into(), gz_items::SetValue::Number(0.5)));
        let (merged, conflicts) = merge_item(&base, &edit, &disk).expect("merged");
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert_eq!(merged.changes.get("cost"), Some(&gz_items::SetValue::Number(5.0)));
        assert!(merged.changes.get("inv_weight").is_some());
    }
}
