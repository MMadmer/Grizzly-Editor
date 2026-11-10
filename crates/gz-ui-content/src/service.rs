//! The browser's state, an `Editor` service: where it is, its history and favorites, the
//! selection, the clipboard, the rename box, each host's view; the catalogs of the sources and
//! the folder views as queries.
//!
//! The view state is locked by the frame that draws the browser and by the commands, which run
//! their part on the UI thread (`on_ui`): the two never wait for each other. Jobs on the pools
//! write only into the catalogs and the inbox.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Rev, RevCounter};
use gz_editor::Editor;
use gz_project::{FileTree, Project};
use gz_query::{Answer, Query};
use parking_lot::Mutex;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::catalog::{Catalog, View, build_view, veiled_folders};
use crate::model::{CATEGORIES, HISTORY_LIMIT, Host, Location, Source, category, normalize_folder};
use crate::pictures::Pictures;
use crate::select::{Entry, Selection};

/// A catalog's key: the source and, for the Editor source, the category.
pub(crate) type CatKey = (Source, usize);

#[must_use]
pub(crate) fn cat_key(loc: &Location) -> CatKey {
    (loc.source, if loc.source == Source::Editor { loc.category } else { 0 })
}

#[derive(Default)]
struct Slot {
    /// What the catalog was built from.
    rev: Rev,
    cat: Option<Arc<Catalog>>,
    building: bool,
    error: Option<String>,
}

/// What a frame learns of a source's catalog.
#[derive(Clone, Default)]
pub(crate) struct CatState {
    pub cat: Option<Arc<Catalog>>,
    /// The revision the catalog was built for, mixed with the listing scope.
    pub rev: Rev,
    pub busy: bool,
    pub error: Option<String>,
}

/// The sources' catalogs, built on the pools.
pub(crate) struct Catalogs {
    slots: Mutex<FxHashMap<CatKey, Slot>>,
    /// Bumped by Refresh for each Editor category: its folder is walked again.
    editor_want: [RevCounter; 10],
    /// Moves when the project or the linked game changes: what was listed is another source.
    pub scope: RevCounter,
    /// What a finished build pings: the window draws the next frame.
    wake: gz_task::Wake,
}

impl Catalogs {
    pub(crate) fn new(wake: gz_task::Wake) -> Self {
        Self { slots: Mutex::new(FxHashMap::default()), editor_want: Default::default(), scope: RevCounter::new(), wake }
    }
}

/// How a source is read now.
enum Want {
    Project(Arc<Project>, Rev),
    Game(Arc<Project>, Rev),
    Editor(PathBuf, usize, Rev),
    Nothing(Option<String>),
}

fn want_of(editor: &Editor, cats: &Catalogs, key: CatKey) -> Want {
    match key.0 {
        Source::Project => match Project::of(editor) {
            Ok((_, p)) => {
                let rev = p.tree.rev();
                Want::Project(p, rev)
            }
            Err(_) => Want::Nothing(None),
        },
        Source::Game => match Project::of(editor) {
            Ok((_, p)) if !p.game_root().is_empty() => {
                let rev = p.game_rev();
                Want::Game(p, rev)
            }
            Ok(_) => Want::Nothing(Some(gz_project::NOT_LINKED.to_owned())),
            Err(_) => Want::Nothing(Some("no project is open".to_owned())),
        },
        Source::Editor => {
            let rev = cats.editor_want.get(key.1).map_or(Rev::ZERO, RevCounter::get).next();
            Want::Editor(editor.paths.root.clone(), key.1, rev)
        }
    }
}

fn editor_catalog(root: &Path, cat: usize) -> Catalog {
    match crate::catalog::editor_folder(root, cat) {
        Some(dir) if dir.is_dir() => Catalog::from_tree(&FileTree::scan(&dir), Source::Editor, cat, false),
        _ => Catalog::empty(Source::Editor, cat),
    }
}

impl Catalogs {
    fn store(&self, key: CatKey, rev: Rev, cat: Option<Catalog>, error: Option<String>) {
        let mut slots = self.slots.lock();
        let slot = slots.entry(key).or_default();
        slot.building = false;
        slot.rev = rev;
        if let Some(cat) = cat {
            slot.cat = Some(Arc::new(cat));
        }
        slot.error = error;
    }

    /// The catalog to draw, and a build started when it is not current. Never waits.
    pub(crate) fn ensure(self: &Arc<Self>, editor: &Arc<Editor>, key: CatKey) -> CatState {
        let scope = self.scope.get();
        let want = want_of(editor, self, key);
        let mut slots = self.slots.lock();
        let slot = slots.entry(key).or_default();
        let fresh = match &want {
            Want::Project(_, rev) | Want::Game(_, rev) | Want::Editor(_, _, rev) => slot.rev == *rev && (slot.cat.is_some() || slot.error.is_some()),
            Want::Nothing(why) => {
                slot.cat = None;
                slot.error.clone_from(why);
                true
            }
        };
        // a game not read yet has revision zero: the first read is what mounts it
        let game_unread = matches!(want, Want::Game(_, rev) if rev == Rev::ZERO) && slot.cat.is_none() && slot.error.is_none();
        if (!fresh || game_unread) && !slot.building {
            slot.building = true;
            let me = self.clone();
            let exec = editor.exec.clone();
            match want {
                Want::Project(project, _) => {
                    let snap = project.tree.snapshot();
                    exec.spawn_cpu(move || {
                        let rev = snap.rev();
                        let cat = Catalog::from_tree(&snap, Source::Project, 0, true);
                        me.store(key, rev, Some(cat), None);
                        me.wake.ping();
                    });
                }
                Want::Game(project, _) => {
                    let ex = exec.clone();
                    exec.spawn(async move {
                        let read = ex.io(move || project.game_snapshot()).await;
                        match read {
                            Ok((files, rev)) => {
                                let cat = ex.cpu(move || Catalog::from_game(files)).await;
                                me.store(key, rev, Some(cat), None);
                            }
                            Err(e) => me.store(key, Rev::ZERO, None, Some(e)),
                        }
                        me.wake.ping();
                    });
                }
                Want::Editor(root, cat, rev) => {
                    exec.spawn_io(move || {
                        let built = editor_catalog(&root, cat);
                        me.store(key, rev, Some(built), None);
                        me.wake.ping();
                    });
                }
                Want::Nothing(_) => slot.building = false,
            }
        }
        let slot = slots.get(&key);
        CatState {
            cat: slot.and_then(|s| s.cat.clone()),
            rev: slot.map_or(Rev::ZERO, |s| s.rev).mix(scope),
            busy: slot.is_some_and(|s| s.building),
            error: slot.and_then(|s| s.error.clone()),
        }
    }

    /// The catalog as it is now, current or not.
    pub(crate) fn peek(&self, key: CatKey) -> Option<Arc<Catalog>> {
        self.slots.lock().get(&key).and_then(|s| s.cat.clone())
    }

    /// The catalog current with the disk: what a command reads. The project's tree first takes
    /// in every change marked so far.
    pub(crate) async fn fresh(self: &Arc<Self>, editor: &Arc<Editor>, key: CatKey) -> Result<Arc<Catalog>, String> {
        match want_of(editor, self, key) {
            Want::Nothing(why) => Err(why.unwrap_or_else(|| "no project is open".to_owned())),
            Want::Project(project, _) => {
                project.tree.refresh().await;
                let snap = project.tree.snapshot();
                let rev = snap.rev();
                if let Some(cat) = self.slots.lock().get(&key).filter(|s| s.rev == rev).and_then(|s| s.cat.clone()) {
                    return Ok(cat);
                }
                let cat = editor.exec.cpu(move || Catalog::from_tree(&snap, Source::Project, 0, true)).await;
                self.store(key, rev, Some(cat), None);
                self.peek(key).ok_or_else(|| "the project could not be listed".to_owned())
            }
            Want::Game(project, _) => {
                let (files, rev) = editor.exec.io(move || project.game_snapshot()).await.map_err(|e| {
                    self.store(key, Rev::ZERO, None, Some(e.clone()));
                    e
                })?;
                if let Some(cat) = self.slots.lock().get(&key).filter(|s| s.rev == rev).and_then(|s| s.cat.clone()) {
                    return Ok(cat);
                }
                let cat = editor.exec.cpu(move || Catalog::from_game(files)).await;
                self.store(key, rev, Some(cat), None);
                self.peek(key).ok_or_else(|| "the game could not be listed".to_owned())
            }
            Want::Editor(root, cat, rev) => {
                if let Some(found) = self.slots.lock().get(&key).filter(|s| s.rev == rev).and_then(|s| s.cat.clone()) {
                    return Ok(found);
                }
                let built = editor.exec.io(move || editor_catalog(&root, cat)).await;
                self.store(key, rev, Some(built), None);
                self.peek(key).ok_or_else(|| "the library could not be listed".to_owned())
            }
        }
    }

    /// Lists a source again: the project's tree re-walks, the game is checked against the disk,
    /// an Editor category is walked again.
    pub(crate) fn refresh(self: &Arc<Self>, editor: &Arc<Editor>, key: CatKey) {
        match key.0 {
            Source::Project => {
                if let Ok((_, p)) = Project::of(editor) {
                    p.tree.mark_all();
                }
            }
            Source::Game => {
                if let Ok((_, p)) = Project::of(editor) {
                    let me = self.clone();
                    let ex = editor.exec.clone();
                    editor.exec.spawn(async move {
                        let _ = ex.io(move || p.refresh_game()).await;
                        me.slots.lock().remove(&key);
                        me.wake.ping();
                    });
                }
            }
            Source::Editor => {
                if let Some(c) = self.editor_want.get(key.1) {
                    c.bump();
                }
            }
        }
    }

    /// Forgets the project's and the game's listings: another project, another game.
    pub(crate) fn forget_scoped(&self, project: bool) {
        let mut slots = self.slots.lock();
        if project {
            slots.remove(&(Source::Project, 0));
        }
        slots.remove(&(Source::Game, 0));
        self.scope.bump();
    }
}

/// A folder view's key: the catalog, the folder (lower case) and the search.
pub(crate) type ViewKey = (CatKey, String, String);

/// What happened off the UI thread for the next frame to take in.
#[derive(Default)]
pub(crate) struct Inbox {
    /// Open the rename box on this entry (a folder or an asset the Add menu just made).
    pub rename: Option<Entry>,
    /// A rename landed: `(was, now)` project paths.
    pub renamed: Option<(String, String)>,
    /// What was selected was deleted.
    pub clear_selection: bool,
}

/// The view of one host.
#[derive(Clone, Copy, Debug)]
pub(crate) struct HostView {
    /// The tile side picked (pt); 0 fits the host.
    pub tile: f32,
    pub list: bool,
    /// The grid's height at its last frame (pt), for fitting.
    pub grid_h: f32,
    /// The last frame it was drawn.
    pub frame: Option<u64>,
    /// The grid's scroll and viewport height at its last frame.
    pub scroll: f32,
    pub viewport: f32,
}

impl HostView {
    fn new(list: bool) -> Self {
        Self { tile: 0.0, list, grid_h: 0.0, frame: None, scroll: 0.0, viewport: 0.0 }
    }

    pub fn shown_tile(&self, host: Host) -> f32 {
        crate::model::shown_tile(self.tile, self.grid_h, host.tile_default())
    }

    pub fn recent(&self, now: u64) -> bool {
        self.frame.is_some_and(|f| f + 2 >= now)
    }
}

/// The inline rename box.
#[derive(Clone, Debug)]
pub(crate) struct Rename {
    pub entry: Entry,
    pub text: String,
    /// Bytes of `text` selected when the box opened: the name, never the extension.
    pub select: usize,
    pub focus: bool,
}

/// One entry of the clipboard.
#[derive(Clone, Debug)]
pub(crate) struct Clip {
    /// How the source names it.
    pub src: String,
    /// Where it goes below the paste target, its own name last.
    pub rel: String,
    /// On disk (the project, the library), absolute.
    pub abs: Option<PathBuf>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct Clipboard {
    pub source: Source,
    pub cut: bool,
    pub items: Vec<Clip>,
}

/// The rubber band over the tiles.
#[derive(Clone, Debug)]
pub(crate) struct Marquee {
    pub start: egui::Pos2,
    pub base: Vec<Entry>,
}

/// The view state.
pub(crate) struct Inner {
    pub loc: Location,
    pub back: Vec<Location>,
    pub forward: Vec<Location>,
    /// Live for the editor process, never written to a project.
    pub favorites: Vec<Location>,
    /// The project root and the game root the listing and the favorites belong to.
    pub scope: (String, String),
    pub search: String,
    pub sel: Selection,
    pub clip: Clipboard,
    pub rename: Option<Rename>,
    pub hosts: [HostView; 2],
    pub sources_pane: bool,
    /// The pane had the width last frame.
    pub sources_shown: bool,
    pub sources_fit: bool,
    pub view_loaded: bool,
    pub view_dirty: bool,
    /// A selection made from outside lands on screen with the next frame.
    pub scroll_to_selection: bool,
    pub shown_folder: Option<(Source, usize, String)>,
    /// Folders open in the tree, lower case; the root is always open.
    pub tree_open: FxHashSet<String>,
    pub marquee: Option<Marquee>,
    /// The entry whose menu is up, or the grid's (`None` path).
    pub menu: Option<(Option<Entry>, egui::Pos2)>,
    /// The browser had the last click: its keys are its own.
    pub focused: bool,
    pub pics: Pictures,
    /// What the grid drew last: entries and their rectangles, for the marquee.
    pub drawn: Vec<(Entry, egui::Rect)>,
    /// The catalog the pictures were made from: a new listing drops them.
    pub listed: Option<(CatKey, Rev)>,
    /// A drag of tiles started here, and what follows the pointer.
    pub drag_started: bool,
    pub drag_icon: Option<(gz_ui::Icon, Option<gz_project::AssetKind>, Option<crate::pictures::Shown>, String, usize)>,
    /// Where the grid's own menu is up.
    pub grid_menu_at: egui::Pos2,
    /// Open, picked from a menu: run where the frame has the shell's requests.
    pub pending_open: Option<Entry>,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            loc: Location::default(),
            back: Vec::new(),
            forward: Vec::new(),
            favorites: Vec::new(),
            scope: (String::new(), String::new()),
            search: String::new(),
            sel: Selection::default(),
            clip: Clipboard::default(),
            rename: None,
            // the docked panel lists: the Wide layout's strip holds a row and a half of tiles
            hosts: [HostView::new(false), HostView::new(true)],
            sources_pane: true,
            sources_shown: false,
            sources_fit: true,
            view_loaded: false,
            view_dirty: false,
            scroll_to_selection: false,
            shown_folder: None,
            tree_open: FxHashSet::default(),
            marquee: None,
            menu: None,
            focused: false,
            pics: Pictures::default(),
            drawn: Vec::new(),
            listed: None,
            drag_started: false,
            drag_icon: None,
            grid_menu_at: egui::Pos2::ZERO,
            pending_open: None,
        }
    }
}

fn push_history(history: &mut Vec<Location>, loc: &Location) {
    if history.last().is_some_and(|l| l.same(loc)) {
        return;
    }
    if history.len() >= HISTORY_LIMIT {
        history.remove(0);
    }
    history.push(loc.clone());
}

fn remove_scoped(list: &mut Vec<Location>, project: bool, game: bool) {
    list.retain(|l| !((project && l.source == Source::Project) || (game && l.source == Source::Game)));
}

impl Inner {
    pub fn host(&self, host: Host) -> &HostView {
        &self.hosts[host.index()]
    }

    pub fn host_mut(&mut self, host: Host) -> &mut HostView {
        &mut self.hosts[host.index()]
    }

    /// The host the commands' view settings go to: the drawer while it is drawn, the panel
    /// while Level draws it, the drawer otherwise.
    pub fn target_host(&self, now: u64, level_active: bool) -> Host {
        if self.host(Host::Drawer).recent(now) {
            Host::Drawer
        } else if level_active && self.host(Host::Panel).recent(now) {
            Host::Panel
        } else {
            Host::Drawer
        }
    }

    /// Where the browser is seen: `drawer`, `panel` or `none`.
    pub fn visible_host(&self, now: u64) -> &'static str {
        if self.host(Host::Drawer).recent(now) {
            "drawer"
        } else if self.host(Host::Panel).recent(now) {
            "panel"
        } else {
            "none"
        }
    }

    /// Reads the view settings from the layout once (`LoadView`).
    pub fn load_view(&mut self, ctx: &egui::Context) {
        if self.view_loaded {
            return;
        }
        self.view_loaded = true;
        for host in [Host::Drawer, Host::Panel] {
            let (tile_key, list_key) = host.keys();
            let size = gz_ui::layout::get(ctx, tile_key, 0.0);
            let default_list = if host == Host::Panel { 1.0 } else { 0.0 };
            let v = self.host_mut(host);
            v.tile = if size > 0.0 { size.clamp(gz_app::contract::TILE_MIN, gz_app::contract::TILE_MAX) } else { 0.0 };
            v.list = gz_ui::layout::get(ctx, list_key, default_list) != 0.0;
        }
        self.sources_pane = gz_ui::layout::get(ctx, "content_browser.sources_pane", 1.0) != 0.0;
    }

    /// Writes the view settings into the layout (`StoreView`).
    pub fn store_view(&mut self, ctx: &egui::Context) {
        self.view_loaded = true;
        self.view_dirty = false;
        for host in [Host::Drawer, Host::Panel] {
            let (tile_key, list_key) = host.keys();
            let v = *self.host(host);
            gz_ui::layout::set(ctx, tile_key, v.tile);
            gz_ui::layout::set(ctx, list_key, if v.list { 1.0 } else { 0.0 });
        }
        gz_ui::layout::set(ctx, "content_browser.sources_pane", if self.sources_pane { 1.0 } else { 0.0 });
    }

    /// The project and the game the listing belongs to changed: what pointed into them goes
    /// (`EnsureListing`, `SyncFavoriteScopes`). Returns whether anything changed.
    pub fn sync_scope(&mut self, project_root: &str, game_root: &str, cats: &Catalogs) -> bool {
        let project_changed = !self.scope.0.eq_ignore_ascii_case(project_root);
        let game_changed = !self.scope.1.eq_ignore_ascii_case(game_root);
        if !project_changed && !game_changed {
            return false;
        }
        self.scope = (project_root.to_owned(), game_root.to_owned());
        let game = project_changed || game_changed;
        remove_scoped(&mut self.back, project_changed, game);
        remove_scoped(&mut self.forward, project_changed, game);
        remove_scoped(&mut self.favorites, project_changed, game);
        cats.forget_scoped(project_changed);
        if (self.loc.source == Source::Project && project_changed) || (self.loc.source == Source::Game && game) {
            self.loc.folder.clear();
            self.sel.clear();
            self.rename = None;
        }
        self.pics.forget_all();
        true
    }

    fn reset_after_move(&mut self) {
        self.sel.clear();
        self.rename = None;
        self.scroll_to_selection = false;
    }

    /// Goes to a location (`ApplyLocation`). `record` keeps where it came from in Back.
    pub fn apply_location(&mut self, mut target: Location, record: bool, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        target.folder = normalize_folder(&target.folder).map_err(str::to_owned)?;
        if target.category >= CATEGORIES.len() {
            return Err("asset category is out of range".into());
        }
        if target.source == Source::Project && !has_project {
            return Err("no project is open".into());
        }
        if self.loc.same(&target) && self.loc.category == target.category {
            return Ok(());
        }
        if !target.folder.is_empty() {
            let cat = cats.peek(cat_key(&target));
            let Some(node) = cat.as_ref().and_then(|c| c.folder(&target.folder)) else {
                return Err("folder is not available in this source".into());
            };
            target.folder = node.path.to_string();
        }
        let origin = std::mem::replace(&mut self.loc, target);
        if record && !origin.same(&self.loc) {
            push_history(&mut self.back, &origin);
            self.forward.clear();
        }
        self.reset_after_move();
        Ok(())
    }

    /// Back: the newest history entry that still opens.
    pub fn go_back(&mut self, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        let current = self.loc.clone();
        let mut last = String::new();
        while let Some(target) = self.back.last().cloned() {
            match self.apply_location(target.clone(), false, cats, has_project) {
                Ok(()) => {
                    if self.back.last().is_some_and(|l| l.same(&target)) {
                        self.back.pop();
                    }
                    push_history(&mut self.forward, &current);
                    return Ok(());
                }
                Err(e) => {
                    last = e;
                    if self.back.last().is_some_and(|l| l.same(&target)) {
                        self.back.pop();
                    }
                }
            }
        }
        Err(if last.is_empty() { "back history is empty".into() } else { last })
    }

    pub fn go_forward(&mut self, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        let current = self.loc.clone();
        let mut last = String::new();
        while let Some(target) = self.forward.last().cloned() {
            match self.apply_location(target.clone(), false, cats, has_project) {
                Ok(()) => {
                    if self.forward.last().is_some_and(|l| l.same(&target)) {
                        self.forward.pop();
                    }
                    push_history(&mut self.back, &current);
                    return Ok(());
                }
                Err(e) => {
                    last = e;
                    if self.forward.last().is_some_and(|l| l.same(&target)) {
                        self.forward.pop();
                    }
                }
            }
        }
        Err(if last.is_empty() { "forward history is empty".into() } else { last })
    }

    /// Up: the root has no parent, which is no failure.
    pub fn go_up(&mut self, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        if self.loc.folder.is_empty() {
            return Ok(());
        }
        let parent = crate::model::parent_of(&self.loc.folder).to_owned();
        let target = Location { folder: parent, ..self.loc.clone() };
        self.apply_location(target, true, cats, has_project)
    }

    pub fn go_home(&mut self, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        let target = Location { folder: String::new(), ..self.loc.clone() };
        self.apply_location(target, true, cats, has_project)
    }

    /// The source shown changes: its root, the category kept.
    pub fn switch_source(&mut self, source: Source, cats: &Catalogs, has_project: bool) {
        if self.loc.source == source {
            return;
        }
        let target = Location::new(source, self.loc.category, "");
        let _ = self.apply_location(target, true, cats, has_project);
    }

    /// After a navigation from outside: the history of a move nobody recorded.
    pub fn remember(&mut self, origin: &Location) {
        if origin.same(&self.loc) {
            return;
        }
        push_history(&mut self.back, origin);
        self.forward.clear();
    }

    pub fn find_favorite(&self, loc: &Location) -> Option<usize> {
        self.favorites.iter().position(|f| f.same(loc))
    }

    /// Adds a favorite (`AddFavorite` + `ValidateFavorite`).
    pub fn add_favorite(&mut self, mut loc: Location, cats: &Catalogs, has_project: bool) -> Result<(), String> {
        if loc.category >= CATEGORIES.len() {
            return Err("asset category is out of range".into());
        }
        loc.folder = normalize_folder(&loc.folder).map_err(str::to_owned)?;
        if loc.source == Source::Project && !has_project {
            return Err("no project is open".into());
        }
        if self.find_favorite(&loc).is_some() {
            return Err("folder is already a favorite".into());
        }
        if loc.source == Source::Project && !loc.folder.is_empty() {
            let top = loc.folder.split('\\').next().unwrap_or("");
            if gz_project::is_editor_only_entry(top) {
                return Err("folder is editor-owned, not project content".into());
            }
        }
        if !loc.folder.is_empty() {
            let cat = cats.peek(cat_key(&loc));
            let Some(node) = cat.as_ref().and_then(|c| c.folder(&loc.folder)) else {
                return Err(match cats.slots.lock().get(&cat_key(&loc)).and_then(|s| s.error.clone()) {
                    Some(e) if loc.source == Source::Game => e,
                    _ => "folder is not available in this source".into(),
                });
            };
            loc.folder = node.path.to_string();
        }
        self.favorites.push(loc);
        Ok(())
    }

    /// A renamed folder takes the browser along: the folder shown, the history, the favorites.
    pub fn follow_rename(&mut self, was: &str, now: &str) {
        let fix = |l: &mut Location| {
            if l.source == Source::Project && crate::model::is_ancestor(was, &l.folder) {
                l.folder = format!("{now}{}", l.folder.get(was.len()..).unwrap_or(""));
            }
        };
        fix(&mut self.loc);
        self.back.iter_mut().for_each(fix);
        self.forward.iter_mut().for_each(fix);
        self.favorites.iter_mut().for_each(fix);
        self.sel.rename(was, now);
    }

    /// The selected entries that are tiles of the view (a folder picked in the tree is not one).
    pub fn selected_tiles(&self) -> usize {
        let folder = &self.loc.folder;
        let below = if folder.is_empty() { 0 } else { folder.len() + 1 };
        let searching = crate::model::search_active(&self.search);
        self.sel
            .entries()
            .iter()
            .filter(|e| {
                let path = e.path.as_str();
                if path.len() <= below || (below > 0 && !crate::model::is_ancestor(folder, path)) {
                    return false;
                }
                let rest = path.get(below..).unwrap_or("");
                if searching { gz_ui::search::matches(&self.search, rest) } else { !rest.contains('\\') }
            })
            .count()
    }

    /// Why Add is off here, or `None` while it works.
    pub fn add_blocked(&self, has_project: bool) -> Option<&'static str> {
        if self.loc.source.read_only() {
            Some("Content is added to the project: switch the source to Project.")
        } else if !has_project {
            Some("Open a project to add content to it.")
        } else {
            None
        }
    }
}

/// The browser: an `Editor` service.
pub struct ContentBrowser {
    pub(crate) inner: Mutex<Inner>,
    pub(crate) cats: Arc<Catalogs>,
    pub(crate) views: Query<ViewKey, View>,
    pub(crate) veils: Query<CatKey, FxHashSet<String>>,
    pub(crate) inbox: Arc<Mutex<Inbox>>,
    pub(crate) thumbs: Arc<crate::thumbs::Thumbs>,
}

impl ContentBrowser {
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<ContentBrowser> {
        editor.services.get_or_init(|| {
            let cats = Arc::new(Catalogs::new(editor.exec.wake().clone()));
            ContentBrowser {
                inner: Mutex::new(Inner::default()),
                cats,
                views: Query::new(editor.exec.clone()),
                veils: Query::new(editor.exec.clone()),
                inbox: Arc::new(Mutex::new(Inbox::default())),
                thumbs: Arc::new(crate::thumbs::Thumbs::default()),
            }
        })
    }

    /// The module's `[exclude]` and the manifest's revision: what the tiles veil.
    pub(crate) fn rules(editor: &Editor) -> (Arc<[String]>, Rev) {
        let Some(session) = editor.project_opt() else { return (Arc::from(Vec::new()), Rev::ZERO) };
        match gz_mod::ManifestService::get(&session) {
            Some(m) => {
                let snap = m.snapshot();
                let rules: Vec<String> = snap.manifest().map(|m| m.exclude.clone()).unwrap_or_default();
                (rules.into(), m.rev())
            }
            None => (Arc::from(Vec::new()), Rev::ZERO),
        }
    }

    /// A folder's view without waiting: fresh, the last one while the new one is made, or
    /// nothing yet.
    pub(crate) fn view(&self, editor: &Editor, loc: &Location, search: &str, cat: &CatState) -> Answer<View> {
        let Some(catalog) = cat.cat.clone() else { return Answer::Pending };
        let (rules, rules_rev) = Self::rules(editor);
        let project = loc.source == Source::Project;
        let rev = cat.rev.mix(if project { rules_rev } else { Rev::ZERO });
        let key: ViewKey = (cat_key(loc), loc.folder.to_lowercase(), search.to_owned());
        let folder = loc.folder.clone();
        let search = search.to_owned();
        self.views.get(&key, rev, move || build_view(&catalog, &folder, &search, project.then_some(&rules[..])))
    }

    /// The same, awaited: what a command reads.
    pub(crate) async fn fresh_view(&self, editor: &Arc<Editor>, loc: &Location, search: &str) -> Result<Arc<View>, String> {
        let key = cat_key(loc);
        let catalog = self.cats.fresh(editor, key).await?;
        let state = self.cats.ensure(editor, key);
        let (rules, rules_rev) = Self::rules(editor);
        let project = loc.source == Source::Project;
        let rev = state.rev.mix(if project { rules_rev } else { Rev::ZERO });
        let vkey: ViewKey = (key, loc.folder.to_lowercase(), search.to_owned());
        let folder = loc.folder.clone();
        let search = search.to_owned();
        Ok(self.views.fresh(&vkey, rev, move || build_view(&catalog, &folder, &search, project.then_some(&rules[..]))).await)
    }

    /// The folders the tree veils.
    pub(crate) fn veils(&self, editor: &Editor, loc: &Location, cat: &CatState) -> Option<Arc<FxHashSet<String>>> {
        if loc.source != Source::Project {
            return None;
        }
        let catalog = cat.cat.clone()?;
        let (rules, rules_rev) = Self::rules(editor);
        self.veils.get(&cat_key(loc), cat.rev.mix(rules_rev), move || veiled_folders(&catalog, &rules)).value().cloned()
    }
}

/// The category caption of a location, `""` outside the range.
#[must_use]
pub(crate) fn category_name(i: usize) -> &'static str {
    CATEGORIES.get(i).map_or("", |c| c.caption)
}

/// A folder's lower-case key, as the tree keeps its open state.
#[must_use]
pub(crate) fn tree_key(path: &str) -> String {
    path.to_lowercase()
}

/// Whether the Editor category lists files at all.
#[must_use]
pub(crate) fn category_has_files(i: usize) -> bool {
    category(i).folder.is_some()
}
