//! The shell: the tabs, the session, the keys, the requests - everything the frame draws from.
//!
//! [`GrizzlyShell`] is what the window runs. Its [`ShellState`] lives on the UI thread; other
//! threads reach it through [`crate::UiQueue`]. The drawing is in [`crate::chrome`].

use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{Context, Rect, TextureHandle, Ui};
use egui_wgpu::RenderState;
use gz_editor::{Editor, JobId, SmolStr};
use gz_ui::dialog::{self, Answer, Asker};
use gz_ui::font::FontFiles;
use gz_ui::{Icon, Tone, Tx, audit, layout, state};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::actions::{Action, ActionCx, ActionTable};
use crate::builder::{
    DialogDesc, DrawFn, DrawerDesc, LogState, MenuContent, MenuEntry, PageCard, PanelDesc, PrefGroup,
    ShellBuilder, ToolbarFn,
};
use crate::doc::{
    DocCx, Document, Drawer, EditAction, Factory, OpenRequest, PanelOp, Request, ShellRequests,
};
use crate::keys::{self, Chord};
use crate::queue::UiQueue;
use crate::settings::{EditorSettings, Placement};
use crate::tabs::{self, LEVEL, SessionTab};
use crate::window::{FrameReport, Shell, WindowOptions};

/// A `DocCx` over the shell's own fields, so a tab can be borrowed beside it.
macro_rules! cx {
    ($s:ident, $ctx:expr) => {
        DocCx {
            editor: &$s.editor,
            gpu: &$s.gpu,
            ctx: $ctx,
            active_tab: &$s.active,
            requests: &mut $s.requests,
        }
    };
}
pub(crate) use cx;

/// How the shell starts.
#[derive(Clone, Default)]
pub struct ShellOptions {
    /// The three faces, read before the window opens (a startup stage of their own).
    pub fonts: FontFiles,
    /// `-iconsheet`: the work area shows every icon of the kit and nothing else.
    pub iconsheet: bool,
    /// `-kitsheet`: the work area shows every widget of the kit in every state, and the tokens.
    pub kitsheet: bool,
    /// `-uiscale <f>` (0.5..4): the UI at this scale, whatever the monitor says
    /// . Taken again when the window meets another monitor's scale.
    pub ui_scale: Option<f32>,
    /// `-lang en|ru`: the window in this language until Preferences or `app.prefs` picks one.
    pub lang: Option<gz_ui::Lang>,
}

/// How a tab is closed when asked over MCP.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CloseMode {
    IfClean,
    SaveFirst,
    Discard,
}

/// A major tab: a document, or a placeholder the session restored.
pub(crate) struct Tab {
    pub id: SmolStr,
    pub kind: SmolStr,
    pub icon: Icon,
    /// A placeholder's label, from the session.
    pub label: String,
    /// A placeholder's argument for its factory.
    pub path: String,
    pub doc: Option<Box<dyn Document>>,
    /// The factory could not open it: "File not found".
    pub failed: bool,
    /// Its body stopped on an internal error.
    pub panicked: Option<String>,
    /// Where it was in the strip last frame (pt); `None` under the chevron.
    pub rect: Option<Rect>,
    /// It showed its icon alone last frame (a tool tab gives way before any name is cut).
    pub icon_only: bool,
}

impl Tab {
    pub fn loaded(&self) -> bool {
        self.doc.is_some()
    }

    /// What the tab says: the document's label; a placeholder's, as the session kept it in
    /// English (the editor's own word for a tool tab, a file's name for a file tab).
    pub fn label(&self) -> Tx<'static> {
        if let Some(d) = &self.doc {
            let l = d.label();
            if !l.is_empty() {
                return l;
            }
        }
        if !self.label.is_empty() {
            return if self.path().is_empty() {
                Tx::ui(self.label.clone())
            } else {
                Tx::raw(self.label.clone())
            };
        }
        let p = self.path();
        Tx::raw(if p.is_empty() { self.id.to_string() } else { tabs::label_of_path(&p) })
    }

    /// The label in English: what MCP and the session read.
    pub fn label_en(&self) -> String {
        self.label().en().into_owned()
    }

    pub fn path(&self) -> String {
        self.doc.as_ref().and_then(|d| d.path()).unwrap_or_else(|| self.path.clone())
    }

    pub fn dirty(&self) -> bool {
        self.doc.as_ref().is_some_and(|d| d.dirty())
    }

    pub fn missing(&self) -> bool {
        self.failed || self.doc.as_ref().is_some_and(|d| d.missing())
    }

    pub fn closable(&self) -> bool {
        self.id != LEVEL && self.doc.as_ref().is_none_or(|d| d.closable())
    }

    /// The tab's tooltip in the window's language: the document's, its path, or its label, then
    /// what is wrong with it.
    pub fn tooltip(&self, ctx: &Context) -> String {
        let mut text = match self.doc.as_ref().and_then(|d| d.tooltip()) {
            Some(t) => t.text(ctx),
            None => self.path(),
        };
        if text.is_empty() {
            text = self.label().text(ctx);
        }
        if self.dirty() {
            text.push('\n');
            let what = if self.id == LEVEL { "The scene has unsaved changes." } else { "Unsaved changes." };
            text.push_str(&gz_ui::i18n::t(ctx, what));
        }
        if self.missing() {
            text.push('\n');
            text.push_str(&gz_ui::i18n::t(ctx, "File not found."));
        }
        text
    }

    pub fn info_json(&self, active: bool) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label_en(),
            "path": self.path(),
            "dirty": self.dirty(),
            "active": active,
            "missing": self.missing(),
            "closable": self.closable(),
            "loaded": self.loaded(),
        })
    }
}

/// What the builder handed over, kept for the life of the window.
pub(crate) struct Parts {
    pub factories: FxHashMap<SmolStr, Factory>,
    /// What menus, toolbars, keys and the palette run (an `Editor` service too).
    pub actions: Arc<crate::actions::Actions>,
    pub menus: Vec<MenuEntry>,
    pub project_toolbar: Vec<(i32, ToolbarFn)>,
    pub drawers: [Option<DrawerDesc>; 2],
    pub status_chips: Vec<(i32, DrawFn)>,
    pub start_page: Option<DrawFn>,
    pub page_cards: Vec<PageCard>,
    pub dialogs: Vec<DialogDesc>,
    pub panels: Vec<PanelDesc>,
    pub log_state: Option<crate::builder::LogStateFn>,
    pub reset_layout: Option<crate::builder::ResetLayoutFn>,
}

/// The groups areas added to Preferences: an `Editor` service the Preferences tab reads.
#[derive(Default)]
pub struct PrefGroups(pub Vec<PrefGroup>);

/// A popup on screen, as `ui.list` names it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopupInfo {
    pub id: String,
    pub title: String,
    pub window: String,
    /// pt.
    pub rect: Rect,
    /// Where each row's label starts (pt), for a menu: one x is one label column.
    pub label_x: Vec<f32>,
}

/// The caption's geometry as the last frame left it, in pt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caption {
    pub bar: Rect,
    pub menus_x0: f32,
    pub menus_end: f32,
    pub buttons_x: f32,
    /// The row of the major tabs under the caption; nothing on the Start page.
    pub tab_row: Rect,
    pub valid: bool,
}

impl Default for Caption {
    fn default() -> Self {
        Self {
            bar: Rect::NOTHING,
            menus_x0: 0.0,
            menus_end: 0.0,
            buttons_x: 0.0,
            tab_row: Rect::NOTHING,
            valid: false,
        }
    }
}

pub(crate) struct PaletteState {
    pub open: bool,
    pub query: String,
    pub selected: usize,
    pub focus: bool,
    pub scroll_to_selected: bool,
    /// Where it was drawn last frame (pt).
    pub rect: Rect,
    /// The rows for `ranked_for`, ranked: worked out when the query changes or the palette
    /// opens, never per frame.
    pub ranked: Vec<crate::palette::PaletteRow>,
    pub ranked_for: Option<String>,
}

impl Default for PaletteState {
    fn default() -> Self {
        Self {
            open: false,
            query: String::new(),
            selected: 0,
            focus: false,
            scroll_to_selected: false,
            rect: Rect::NOTHING,
            ranked: Vec::new(),
            ranked_for: None,
        }
    }
}

#[derive(Default)]
struct Session {
    root: Option<PathBuf>,
    /// Where the project's tabs are kept (the project area says: this machine's state of it).
    file: Option<PathBuf>,
    dirty: bool,
    /// The session file's writes in the order they were asked for.
    order: Arc<gz_editor::fsx::InOrder>,
}

/// Everything the shell draws from. UI thread only.
pub struct ShellState {
    pub(crate) editor: Arc<Editor>,
    pub(crate) gpu: RenderState,
    pub(crate) queue: Arc<UiQueue>,
    pub(crate) settings: Arc<EditorSettings>,
    pub(crate) parts: Parts,
    pub(crate) tabs: Vec<Tab>,
    pub(crate) active: SmolStr,
    pub(crate) requests: ShellRequests,
    pub(crate) drawer: Option<Drawer>,
    pub(crate) drawer_focus: bool,
    pub(crate) drawer_rect: Option<Rect>,
    pub(crate) status_rect: Rect,
    pub(crate) work_rect: Rect,
    pub(crate) caption: Caption,
    /// What the window's `WM_NCHITTEST` answers from.
    pub(crate) hit_test: Arc<gz_win::CaptionHitTest>,
    /// The panel that answers its own keys and had the focus this frame, and the last frame's.
    pub(crate) key_owner_now: Option<SmolStr>,
    pub(crate) key_owner_last: Option<SmolStr>,
    pub(crate) folded: bool,
    pub(crate) menus_shown: bool,
    pub(crate) start_page: bool,
    pub(crate) menu_bar: crate::chrome::MenuBarState,
    pub(crate) popups_cur: Vec<PopupInfo>,
    pub(crate) popups_last: Vec<PopupInfo>,
    pub(crate) palette: PaletteState,
    pub(crate) card_open: bool,
    /// The card came up by itself, for a job the window started: it goes a moment after the last
    /// job ended. One the author opened (its chip, `ui.tasks show=true`) stays.
    pub(crate) card_auto: bool,
    pub(crate) card_rect: Option<Rect>,
    /// Where the active document's status drew in the status bar's centre last frame; `None`
    /// on the Start page and its cards, which are no document.
    pub(crate) status_centre: Option<Rect>,
    seen_jobs: Vec<JobId>,
    pub(crate) cancelled: FxHashSet<JobId>,
    session: Session,
    pub(crate) logo: Option<TextureHandle>,
    pub(crate) iconsheet: bool,
    pub(crate) kitsheet: bool,
    pub(crate) tab_menu: Option<SmolStr>,
    pub(crate) drag_tab: Option<SmolStr>,
    last_settings_save: Instant,
    placement: Option<Placement>,
    last_frame: Option<Instant>,
    pub(crate) fps: f32,
    panic_guard: Arc<AtomicBool>,
    /// `ui.scale`: the test's scale, over the author's preference until the editor ends.
    pub(crate) test_scale: Option<f32>,
    /// The scale applied last: the preference, `-uiscale` or a test's.
    scale_applied: Option<(Option<f32>, f32)>,
    /// The window's language, its catalogs and `-lang`.
    pub(crate) language: crate::lang::Language,
    /// What the active tab takes of the Edit menu, and whether it is dirty: worked out once a
    /// frame, for the actions' checks.
    pub(crate) edits_now: Vec<EditAction>,
    pub(crate) dirty_now: bool,
    /// The active tab's file, project-relative, as of this frame.
    pub(crate) path_now: String,
    /// What the frame drew where (pt), for `ui.frame`.
    pub(crate) toolbar_rect: Option<Rect>,
    pub(crate) project_toolbar_rect: Option<Rect>,
    pub(crate) toolbar_status_shown: Option<String>,
    pub(crate) status_line_rect: Option<Rect>,
    pub(crate) status_chip_ids: Vec<&'static str>,
    /// The phases of the frame being drawn, for the stall monitor.
    pub(crate) phases: crate::stalls::FramePhases,
    /// The frames drawn: the first is no stall.
    frames: u64,
    /// When the last running job ended: the card shows its result a moment longer.
    pub(crate) last_job_end: Option<Instant>,
    /// The context the shell draws with: the kit's stores live in it.
    pub(crate) egui_ctx: Context,
    /// A surface reads the keys itself (this frame's ask, and the last frame's).
    capture_now: bool,
    capture_last: bool,
}

/// The Level tab of a shell no level editor was installed in (a test of `gz-app` alone): the
/// editor installs `gz-ui-level`'s, whose factory never fails.
struct LevelPlaceholder;

impl Document for LevelPlaceholder {
    fn id(&self) -> &str {
        LEVEL
    }
    fn kind(&self) -> &'static str {
        "level"
    }
    fn label(&self) -> Tx<'static> {
        Tx::from("Level")
    }
    fn icon(&self) -> Icon {
        Icon::Level
    }
    fn closable(&self) -> bool {
        false
    }
    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        state::empty_state(ui, Icon::Level, "Level", "No level editor is installed in this shell.", None);
    }
}

impl ShellState {
    fn new(
        editor: Arc<Editor>,
        gpu: RenderState,
        queue: Arc<UiQueue>,
        builder: ShellBuilder,
        sheets: (bool, bool),
        panic_guard: Arc<AtomicBool>,
        egui_ctx: Context,
    ) -> Self {
        let (iconsheet, kitsheet) = sheets;
        let settings = EditorSettings::of(&editor);
        let ShellBuilder {
            factories,
            actions,
            pref_groups,
            menus,
            project_toolbar,
            drawers,
            status_chips,
            start_page,
            page_cards,
            dialogs,
            panels,
            log_state,
            reset_layout,
            translations,
            // read by the areas as they install
            no_releases: _,
        } = builder;
        JobMarks::of(&editor).log_state.lock().clone_from(&log_state);
        // the window's own actions first, then the areas': a later id replaces an earlier one
        let mut table = ActionTable::default();
        for a in crate::shell_actions::shell_actions().into_iter().chain(actions) {
            table.add(a);
        }
        table.load_bindings(&settings);
        let mut menus = menus;
        for (menu, group, order, id, label) in crate::shell_actions::SHELL_MENU {
            menus.push(MenuEntry {
                menu: SmolStr::new(menu),
                submenu: None,
                group: *group,
                order: *order,
                content: MenuContent::Action { id: SmolStr::new(id), label: label.map(SmolStr::new) },
            });
        }
        for (group, order, id) in crate::shell_actions::PROJECT_MENU {
            menus.push(MenuEntry {
                menu: SmolStr::new("File"),
                submenu: Some(SmolStr::new("Project")),
                group: *group,
                order: *order,
                content: MenuContent::Action { id: SmolStr::new(id), label: None },
            });
        }
        // an area's preferences are the settings' too: `app.prefs` lists and sets them
        for g in &pref_groups {
            let defs: Vec<crate::settings::PrefDef> =
                g.rows.iter().filter_map(crate::builder::PrefRow::def).collect();
            settings.add_defs(&defs);
        }
        editor.services.insert(PrefGroups(pref_groups));
        // a job belongs to the project open when it starts; closing it stops its jobs
        let shared = editor.services.insert(crate::actions::Actions::new(table));
        let mut parts = Parts {
            factories,
            actions: shared,
            menus,
            project_toolbar,
            drawers,
            status_chips,
            start_page,
            page_cards,
            dialogs,
            panels,
            log_state,
            reset_layout,
        };
        parts.project_toolbar.sort_by_key(|(o, _)| *o);
        parts.status_chips.sort_by_key(|(o, _)| *o);
        let hit_test = editor.services.get_or_init(crate::window::WindowLink::default).caption().clone();
        crate::unsaved::install(&editor);
        let mut s = Self {
            editor,
            gpu,
            queue,
            settings,
            parts,
            tabs: Vec::new(),
            active: SmolStr::default(),
            requests: ShellRequests::default(),
            drawer: None,
            drawer_focus: false,
            drawer_rect: None,
            status_rect: Rect::NOTHING,
            work_rect: Rect::NOTHING,
            caption: Caption::default(),
            hit_test,
            key_owner_now: None,
            key_owner_last: None,
            folded: false,
            menus_shown: false,
            start_page: true,
            menu_bar: crate::chrome::MenuBarState::default(),
            popups_cur: Vec::new(),
            popups_last: Vec::new(),
            palette: PaletteState::default(),
            card_open: false,
            card_auto: false,
            status_centre: None,
            card_rect: None,
            seen_jobs: Vec::new(),
            cancelled: FxHashSet::default(),
            session: Session::default(),
            logo: None,
            iconsheet,
            kitsheet,
            tab_menu: None,
            drag_tab: None,
            last_settings_save: Instant::now(),
            placement: None,
            last_frame: None,
            fps: 0.0,
            panic_guard,
            test_scale: None,
            scale_applied: None,
            language: crate::lang::Language::new(translations, None),
            edits_now: Vec::new(),
            dirty_now: false,
            path_now: String::new(),
            toolbar_rect: None,
            project_toolbar_rect: None,
            toolbar_status_shown: None,
            status_line_rect: None,
            status_chip_ids: Vec::new(),
            phases: crate::stalls::FramePhases::default(),
            frames: 0,
            last_job_end: None,
            egui_ctx,
            capture_now: false,
            capture_last: false,
        };
        s.open_level();
        s
    }

    fn open_level(&mut self) {
        let doc: Box<dyn Document> = match self.parts.factories.get_mut("level") {
            Some(f) => match f(&self.editor, &OpenRequest::kind("level")) {
                Ok(doc) => doc,
                Err(e) => {
                    self.editor.log.error(format!("the Level tab did not open: {e}"));
                    Box::new(LevelPlaceholder)
                }
            },
            None => Box::new(LevelPlaceholder),
        };
        self.tabs.insert(
            0,
            Tab {
                id: SmolStr::new(LEVEL),
                kind: SmolStr::new("level"),
                icon: doc.icon(),
                label: String::new(),
                path: String::new(),
                doc: Some(doc),
                failed: false,
                panicked: None,
                rect: None,
                icon_only: false,
            },
        );
        self.active = SmolStr::new(LEVEL);
    }

    pub(crate) fn asker(&self) -> Asker {
        if self.editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
    }

    // ---- tabs ------------------------------------------------------------------------------

    pub(crate) fn find(&self, id: &str) -> Option<usize> {
        self.tabs.iter().position(|t| t.id == id)
    }

    /// The document of a tab, for its own area: `doc.as_any_mut()` reaches its type.
    pub fn document_mut(&mut self, id: &str) -> Option<&mut Box<dyn Document>> {
        let i = self.find(id)?;
        self.tabs.get_mut(i)?.doc.as_mut()
    }

    /// The id of the tab that is up.
    #[must_use]
    pub fn active(&self) -> &str {
        &self.active
    }

    /// Every tab, as `ui.tab action=list` answers it.
    #[must_use]
    pub fn tabs_json(&self) -> Vec<serde_json::Value> {
        self.tabs.iter().map(|t| t.info_json(t.id == self.active)).collect()
    }

    pub(crate) fn tab_json(&self, id: &str) -> Option<serde_json::Value> {
        self.find(id).and_then(|i| self.tabs.get(i)).map(|t| t.info_json(t.id == self.active))
    }

    /// Brings a tab up; a restored one opens its document now. Takes effect at once: the next
    /// work area drawn is this tab's.
    pub fn activate(&mut self, ctx: &Context, id: &str) -> bool {
        let Some(i) = self.find(id) else { return false };
        if self.active == id {
            if !self.tabs.get(i).is_some_and(Tab::loaded) {
                self.load(i);
            }
            return true;
        }
        if let Some(old) = self.find(&self.active)
            && let Some(tab) = self.tabs.get_mut(old)
            && let Some(doc) = tab.doc.as_mut()
        {
            doc.on_deactivate(&mut cx!(self, ctx));
        }
        // a drawer opened over the tab that goes must not cover the next one's canvas
        self.drawer = None;
        self.active = SmolStr::new(id);
        if !self.tabs.get(i).is_some_and(Tab::loaded) {
            self.load(i);
        }
        if let Some(tab) = self.tabs.get_mut(i)
            && let Some(doc) = tab.doc.as_mut()
        {
            doc.on_activate(&mut cx!(self, ctx));
        }
        true
    }

    /// A placeholder's document, made by the factory of its kind.
    fn load(&mut self, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        if tab.loaded() {
            return;
        }
        let req = OpenRequest {
            kind: tab.kind.to_string(),
            path: (!tab.path.is_empty()).then(|| tab.path.clone()),
            id: Some(tab.id.to_string()),
            arg: None,
            activate: true,
        };
        let made = match self.parts.factories.get_mut(tab.kind.as_str()) {
            Some(f) => f(&self.editor, &req),
            None => Err(format!("no tab of the kind '{}' is installed in this shell", req.kind)),
        };
        let Some(tab) = self.tabs.get_mut(i) else { return };
        match made {
            Ok(doc) => {
                tab.icon = doc.icon();
                tab.doc = Some(doc);
                tab.failed = false;
            }
            Err(e) => {
                self.editor.log.warn(format!("the tab '{}' could not be restored: {e}", tab.id));
                tab.failed = true;
            }
        }
    }

    /// Opens a document, or brings up the one of that id. A singleton that switches to a new
    /// target asks first while dirty: `discard` drops its changes, otherwise an MCP caller gets
    /// `unsaved` and a person the question.
    pub(crate) fn open(
        &mut self,
        ctx: &Context,
        req: OpenRequest,
        discard: bool,
        asker: Asker,
    ) -> Result<(), OpenError> {
        if let Some(id) = &req.id
            && self.find(id).is_some()
        {
            return if self.activate(ctx, id) {
                Ok(())
            } else {
                Err(OpenError::Other(format!("no tab '{id}'")))
            };
        }
        if req.kind == LEVEL {
            self.activate(ctx, LEVEL);
            return Ok(());
        }
        let Some(factory) = self.parts.factories.get_mut(req.kind.as_str()) else {
            return Err(OpenError::Other(if crate::contract::DOCUMENT_KINDS.contains(&req.kind.as_str()) {
                format!("no tab of the kind '{}' is installed in this shell", req.kind)
            } else {
                format!("unknown kind '{}' (ui.contract lists the document kinds)", req.kind)
            }));
        };
        let doc = factory(&self.editor, &req).map_err(OpenError::Other)?;
        let id = SmolStr::new(doc.id());
        let activate = req.activate || self.active.is_empty();
        match self.find(&id) {
            Some(i) => {
                let Some(tab) = self.tabs.get(i) else {
                    return Err(OpenError::Other("the tab went away".to_owned()));
                };
                let same = tab.loaded() && tab.path() == doc.path().unwrap_or_default();
                if same {
                    if activate {
                        self.activate(ctx, &id);
                    }
                    return Ok(());
                }
                if tab.dirty() && !discard {
                    if asker == Asker::Ui {
                        let label = tab.label();
                        let queue = self.queue.clone();
                        let retarget = req.clone();
                        let target = id.to_string();
                        let editor = self.editor.clone();
                        dialog::unsaved_prompt(ctx, Asker::Ui, "retarget_tab", &[label], move |answer| {
                            match answer {
                                Answer::Cancel => {}
                                // the other file opens once this one is written
                                Answer::Yes => {
                                    crate::unsaved::save_then(&editor, &queue, vec![target], |shell, ctx| {
                                        let _ = shell.open(ctx, retarget, false, Asker::Ui);
                                    })
                                }
                                _ => queue.post(move |shell, ctx| {
                                    let _ = shell.open(ctx, retarget, true, Asker::Ui);
                                }),
                            }
                        });
                        return Ok(());
                    }
                    return Err(OpenError::Unsaved { id: id.to_string(), label: tab.label_en() });
                }
                if let Some(tab) = self.tabs.get_mut(i) {
                    if let Some(old) = tab.doc.as_mut() {
                        if tab.id == self.active {
                            old.on_deactivate(&mut cx!(self, ctx));
                        }
                        old.discard(&mut cx!(self, ctx));
                        old.on_close(&mut cx!(self, ctx));
                    }
                    tab.icon = doc.icon();
                    tab.kind = SmolStr::new(doc.kind());
                    tab.doc = Some(doc);
                    tab.failed = false;
                    tab.panicked = None;
                }
                self.session.dirty = true;
                if self.active == id {
                    if let Some(tab) = self.tabs.get_mut(i)
                        && let Some(d) = tab.doc.as_mut()
                    {
                        d.on_activate(&mut cx!(self, ctx));
                    }
                } else if activate {
                    self.activate(ctx, &id);
                }
            }
            None => {
                let tab = Tab {
                    id: id.clone(),
                    kind: SmolStr::new(doc.kind()),
                    icon: doc.icon(),
                    label: String::new(),
                    path: String::new(),
                    doc: Some(doc),
                    failed: false,
                    panicked: None,
                    rect: None,
                    icon_only: false,
                };
                self.tabs.push(tab);
                self.session.dirty = true;
                if activate {
                    self.activate(ctx, &id);
                }
            }
        }
        Ok(())
    }

    /// The UI's close: a tab with unsaved changes asks first.
    pub(crate) fn request_close(&mut self, ctx: &Context, id: &str) {
        let Some(i) = self.find(id) else { return };
        let Some(tab) = self.tabs.get(i) else { return };
        if !tab.closable() {
            return;
        }
        if !tab.dirty() {
            self.remove(ctx, i);
            return;
        }
        let what = [tab.label()];
        let target = id.to_owned();
        let queue = self.queue.clone();
        let asker = self.asker();
        let editor = self.editor.clone();
        let line = dialog::unsaved_prompt(ctx, asker, "close_tab", &what, move |answer| match answer {
            Answer::Cancel => {}
            // the tab closes once its file is written; a save that fails keeps it open
            Answer::Yes => {
                let id = target.clone();
                crate::unsaved::save_then(&editor, &queue, vec![target], move |shell, ctx| {
                    let _ = shell.close_tab(ctx, &id, CloseMode::IfClean, true);
                });
            }
            _ => queue.post(move |shell, ctx| {
                let _ = shell.close_tab(ctx, &target, CloseMode::Discard, true);
            }),
        });
        if let Some(line) = line {
            self.editor.log.finding(&line);
        }
    }

    /// MCP's close: `IfClean` refuses a dirty tab ("unsaved"), `SaveFirst` saves it, `Discard`
    /// drops the changes.
    pub(crate) fn close_tab(
        &mut self,
        ctx: &Context,
        id: &str,
        mode: CloseMode,
        person: bool,
    ) -> Result<(), String> {
        let Some(i) = self.find(id) else { return Err("not open".to_owned()) };
        let Some(tab) = self.tabs.get(i) else { return Err("not open".to_owned()) };
        if !tab.closable() {
            return Err("the Level tab does not close".to_owned());
        }
        if tab.dirty() {
            match mode {
                CloseMode::IfClean => return Err("unsaved".to_owned()),
                CloseMode::SaveFirst => self.save_tab(ctx, id, person)?,
                CloseMode::Discard => {
                    if let Some(tab) = self.tabs.get_mut(i)
                        && let Some(doc) = tab.doc.as_mut()
                    {
                        doc.discard(&mut cx!(self, ctx));
                    }
                }
            }
        }
        self.remove(ctx, i);
        Ok(())
    }

    /// A tab's save without closing it; `Ok` when it is clean or its save has begun. `person`:
    /// a person asked in the window, so the tab may raise a question (the system's Save dialog
    /// for a scene without a file); a command's caller is refused in a sentence instead.
    pub(crate) fn save_tab(&mut self, ctx: &Context, id: &str, person: bool) -> Result<(), String> {
        let Some(i) = self.find(id) else { return Err("not open".to_owned()) };
        let Some(tab) = self.tabs.get_mut(i) else { return Err("not open".to_owned()) };
        // a failure an earlier save left is not this save's
        let _earlier = tab.doc.as_mut().and_then(|d| d.save_failed());
        if !tab.dirty() {
            return Ok(());
        }
        let Some(doc) = tab.doc.as_mut() else { return Ok(()) };
        // whatever the tab raises all the same gets the logged answers: nothing opens for a caller
        let _answers = (!person).then(|| dialog::answer_as(ctx, Asker::Unattended));
        self.requests.by_caller = !person;
        let saved = doc.save(&mut cx!(self, ctx));
        self.requests.by_caller = false;
        saved.map_err(|e| if e.is_empty() { "it cannot be saved".to_owned() } else { e })
    }

    /// A tab's discard without closing it; `true` when it is clean afterwards.
    pub(crate) fn discard_tab(&mut self, ctx: &Context, id: &str) -> bool {
        let Some(i) = self.find(id) else { return false };
        let Some(tab) = self.tabs.get_mut(i) else { return false };
        if let Some(doc) = tab.doc.as_mut()
            && doc.dirty()
        {
            doc.discard(&mut cx!(self, ctx));
        }
        !self.tabs.get(i).is_some_and(Tab::dirty)
    }

    fn remove(&mut self, ctx: &Context, i: usize) {
        let Some(tab) = self.tabs.get(i) else { return };
        let was_active = tab.id == self.active;
        let id = tab.id.clone();
        if let Some(tab) = self.tabs.get_mut(i)
            && let Some(doc) = tab.doc.as_mut()
        {
            if was_active {
                doc.on_deactivate(&mut cx!(self, ctx));
            }
            doc.on_close(&mut cx!(self, ctx));
        }
        if i < self.tabs.len() {
            self.tabs.remove(i);
        }
        if id != LEVEL {
            self.session.dirty = true;
        }
        if self.drag_tab.as_deref() == Some(id.as_str()) {
            self.drag_tab = None;
        }
        if was_active {
            // the neighbour to the left takes over, Level when there is none
            let next = i
                .checked_sub(1)
                .or((!self.tabs.is_empty()).then_some(0))
                .and_then(|j| self.tabs.get(j))
                .map(|t| t.id.clone());
            self.active = SmolStr::default();
            if let Some(next) = next {
                self.activate(ctx, &next);
            }
        }
    }

    /// Ctrl+Tab order: +1 the next tab, -1 the previous one.
    pub(crate) fn cycle(&mut self, ctx: &Context, step: i32) {
        let n = self.tabs.len() as i32;
        if n < 2 {
            return;
        }
        let at = self.find(&self.active).unwrap_or(0) as i32;
        let next = (at + step).rem_euclid(n) as usize;
        if let Some(id) = self.tabs.get(next).map(|t| t.id.clone()) {
            self.activate(ctx, &id);
        }
    }

    /// The tabs with unsaved changes: id and label.
    pub(crate) fn dirty_tabs(&self) -> Vec<(String, String)> {
        self.tabs.iter().filter(|t| t.dirty()).map(|t| (t.id.to_string(), t.label_en())).collect()
    }

    /// The unsaved work the window's questions name: the dirty tabs, and what no tab shows - a
    /// library a command edited with its tab closed.
    pub(crate) fn unsaved_work(&self) -> (Vec<(String, Tx<'static>)>, Vec<Tx<'static>>) {
        let tabs = self.tabs.iter().filter(|t| t.dirty()).map(|t| (t.id.to_string(), t.label())).collect();
        // the scene is the Level tab's; every other document goes by its tab's kind
        let shown = |kind: &str| kind == "scene" || self.tabs.iter().any(|t| t.kind == kind && t.dirty());
        let other = self
            .editor
            .dirty_docs()
            .into_iter()
            .filter(|d| !shown(d.kind.as_str()))
            // an area names its unsaved work in its own English ("Texture Library") or by a file
            .map(|d| Tx::ui(d.name))
            .collect();
        (tabs, other)
    }

    /// Quit, after the question that names every unsaved document. Save waits for the tabs' saves;
    /// `app.quit` then saves what is left, or drops it, and ends the editor.
    pub(crate) fn request_exit(&mut self, ctx: &Context) {
        let (dirty, other) = self.unsaved_work();
        if dirty.is_empty() && other.is_empty() {
            self.editor.request_quit();
            return;
        }
        let what: Vec<Tx<'static>> = dirty.iter().map(|(_, l)| l.clone()).chain(other).collect();
        let ids: Vec<String> = dirty.into_iter().map(|(id, _)| id).collect();
        let queue = self.queue.clone();
        let editor = self.editor.clone();
        let quit = |editor: &Arc<Editor>, how: &str| {
            let mut args = gz_editor::Args::new();
            args.insert(how.to_owned(), serde_json::Value::Bool(true));
            editor.post("app.quit", args);
        };
        let line = dialog::unsaved_prompt(ctx, self.asker(), "exit", &what, move |answer| match answer {
            Answer::Cancel => {}
            // every save is waited for: the editor does not end over a write under way, or under
            // the Save dialog of a scene that has no file yet
            Answer::Yes => {
                crate::unsaved::save_then(&editor, &queue, ids, move |shell, _| quit(&shell.editor, "save"));
            }
            _ => quit(&editor, "discard"),
        });
        if let Some(line) = line {
            self.editor.log.finding(&line);
        }
    }

    // ---- drawers, panels -------------------------------------------------------------------

    /// The panel that shows a drawer's object is on screen: the drawer gives way to it.
    pub(crate) fn drawer_panel_visible(&self, which: Drawer) -> bool {
        let Some(panel) =
            self.parts.drawers.get(which.index()).and_then(|d| d.as_ref()).and_then(|d| d.panel.clone())
        else {
            return false;
        };
        self.active == LEVEL && self.level_panel(&panel).is_some_and(|p| p.visible)
    }

    pub(crate) fn level_panel(&self, id: &str) -> Option<crate::PanelInfo> {
        self.tabs.first().and_then(|t| t.doc.as_ref()).and_then(|d| d.panel_state(id))
    }

    pub(crate) fn has_drawer(&self, which: Drawer) -> bool {
        self.parts.drawers.get(which.index()).is_some_and(Option::is_some)
    }

    /// Opens a drawer over the active tab. `false` when the shell has no body installed for it.
    pub(crate) fn open_drawer(&mut self, which: Drawer) -> bool {
        if !self.has_drawer(which) {
            return false;
        }
        if self.drawer != Some(which) {
            self.drawer_focus = true;
        }
        self.drawer = Some(which);
        true
    }

    pub(crate) fn toggle_drawer(&mut self, ctx: &Context, which: Drawer) {
        if self.drawer == Some(which) {
            self.drawer = None;
            return;
        }
        // its panel is on screen: that is where it goes instead
        if self.drawer_panel_visible(which) {
            self.drawer = None;
            if let Some(panel) =
                self.parts.drawers.get(which.index()).and_then(|d| d.as_ref()).and_then(|d| d.panel.clone())
            {
                self.panel(ctx, &panel, PanelOp::Focus);
            }
            return;
        }
        self.open_drawer(which);
    }

    /// A Level panel: opening or focusing one brings Level up; with no room for it in this size
    /// class its drawer shows it. Answers where it is shown.
    pub(crate) fn panel(&mut self, ctx: &Context, id: &str, op: PanelOp) -> Option<&'static str> {
        let desc = self.parts.panels.iter().find(|p| p.id == id).cloned()?;
        let open = match op {
            PanelOp::Open | PanelOp::Focus | PanelOp::Float | PanelOp::Dock => true,
            PanelOp::Close => false,
            PanelOp::Toggle => !self.level_panel(id).is_some_and(|p| p.open),
        };
        if op == PanelOp::Float && ctx.embed_viewports() {
            return None;
        }
        if open {
            self.activate(ctx, LEVEL);
        }
        let done = match self.tabs.first_mut().and_then(|t| t.doc.as_mut()) {
            Some(doc) => doc.panel(&mut cx!(self, ctx), id, if open { op_open(op) } else { PanelOp::Close }),
            None => false,
        };
        if !done {
            return None;
        }
        if !open {
            return Some("none");
        }
        if self.level_panel(id).is_some_and(|p| p.floating) || op == PanelOp::Float {
            return Some("window");
        }
        let has_room = desc.drawer.is_none() || self.size_class() == "wide";
        match desc.drawer {
            Some(d) if !has_room => {
                self.open_drawer(d);
                Some("drawer")
            }
            _ => Some("panel"),
        }
    }

    /// `wide` (1400 x 640 pt and up) or `compact`.
    pub(crate) fn size_class(&self) -> &'static str {
        let r = self.screen_pt();
        if r.width() >= 1400.0 && r.height() >= 640.0 { "wide" } else { "compact" }
    }

    pub(crate) fn screen_pt(&self) -> Rect {
        Rect::from_min_max(
            self.caption.bar.min,
            egui::pos2(self.caption.bar.max.x, self.status_rect.max.y.max(self.caption.bar.max.y)),
        )
    }

    // ---- keys ------------------------------------------------------------------------------

    pub(crate) fn modal_up(&self, ctx: &Context) -> bool {
        self.palette.open
            || dialog::modal_on_top(ctx).is_some()
            || ctx.memory(|m| m.top_modal_layer().is_some())
    }

    /// What is in front of the author, for an action's checks.
    pub(crate) fn action_cx(&self) -> ActionCx<'_> {
        let kind = self.find(&self.active).and_then(|i| self.tabs.get(i)).map_or("", |t| t.kind.as_str());
        ActionCx {
            editor: &self.editor,
            tab: &self.active,
            kind,
            path: &self.path_now,
            project: !self.start_page,
            dirty: self.dirty_now,
            edits: &self.edits_now,
            drawer: self.drawer,
        }
    }

    /// Runs an action by its id when it is on; `Err` says why it did not run.
    pub(crate) fn run_action(&mut self, id: &str) -> Result<(), String> {
        let Some(action) = self.parts.actions.table.read().get(id).copied() else {
            return Err(format!("no action '{id}' (app.keys lists them)"));
        };
        let tab = self.active.clone();
        let path = self.active_path();
        let kind = self.find(&tab).and_then(|i| self.tabs.get(i)).map(|t| t.kind.clone()).unwrap_or_default();
        let cx = ActionCx {
            editor: &self.editor,
            tab: &tab,
            kind: &kind,
            path: &path,
            project: !self.start_page,
            dirty: self.dirty_now,
            edits: &self.edits_now,
            drawer: self.drawer,
        };
        let on = (action.enabled)(&cx);
        if !on.on() {
            return Err(if on.why().is_empty() {
                format!("{} is off here.", action.title)
            } else {
                on.why().to_owned()
            });
        }
        crate::actions::run(&self.editor, &action, &cx);
        Ok(())
    }

    /// The active tab's file, project-relative.
    pub(crate) fn active_path(&self) -> String {
        let Some(t) = self.find(&self.active).and_then(|i| self.tabs.get(i)) else { return String::new() };
        let p = t.path();
        project_relative(self.editor.project_root().as_deref(), &p)
    }

    /// What the active tab takes of the Edit menu and whether it is dirty, worked out once a frame.
    fn refresh_edits(&mut self, ctx: &Context) {
        let id = self.active.to_string();
        let mut edits = Vec::with_capacity(8);
        let mut dirty = false;
        if let Some(i) = self.find(&id)
            && let Some(tab) = self.tabs.get_mut(i)
            && !tab.failed
            && tab.panicked.is_none()
        {
            dirty = tab.dirty();
            if let Some(doc) = tab.doc.as_mut()
                && !doc.missing()
            {
                for a in EditAction::ALL {
                    if doc.edit(&mut cx!(self, ctx), a, false) {
                        edits.push(a);
                    }
                }
            }
        }
        self.edits_now = edits;
        self.dirty_now = dirty;
        self.path_now = self.active_path();
        let kind = self
            .find(&self.active)
            .and_then(|i| self.tabs.get(i))
            .map(|t| t.kind.to_string())
            .unwrap_or_default();
        self.parts.actions.set_facts(crate::actions::Facts {
            tab: self.active.to_string(),
            kind,
            path: self.path_now.clone(),
            project: !self.start_page,
            dirty: self.dirty_now,
            edits: self.edits_now.clone(),
            drawer: self.drawer,
        });
    }

    /// The part of the active document that has the keys ("canvas").
    fn key_part(&self, ctx: &Context) -> Option<&'static str> {
        self.find(&self.active)
            .and_then(|i| self.tabs.get(i))
            .and_then(|t| t.doc.as_ref())
            .and_then(|d| d.key_part(ctx))
    }

    /// Keys, first taker wins: a text field, the actions (a tab's own before the window's), then
    /// the document's own keys (a gesture's Esc).
    fn keys(&mut self, ctx: &Context) {
        if self.start_page || ctx.egui_wants_keyboard_input() || self.modal_up(ctx) || self.capture_last {
            return;
        }
        let presses = ctx.input(|i| keys::pressed(&i.events));
        for chord in presses {
            // a panel that answers its own keys keeps them, but for the frame's own
            if let Some(id) = self.action_for_chord(ctx, &chord) {
                ctx.input_mut(|i| keys::consume(i, &chord));
                if let Err(e) = self.run_action(id) {
                    self.editor.log.plain(e);
                }
                continue;
            }
            let id = self.active.to_string();
            let taken = match self.find(&id).and_then(|i| self.tabs.get_mut(i)) {
                Some(tab) if !tab.failed && tab.panicked.is_none() => match tab.doc.as_mut() {
                    Some(doc) => doc.shortcut(&mut cx!(self, ctx), &chord),
                    None => false,
                },
                _ => false,
            };
            if taken {
                ctx.input_mut(|i| keys::consume(i, &chord));
            }
        }
    }

    /// The action a chord runs now: a panel that answers its own keys keeps them, and so does a
    /// popup that is up, but for the frame's own; the part of the document that has the keys
    /// narrows the rest.
    fn action_for_chord(&self, ctx: &Context, chord: &Chord) -> Option<&'static str> {
        let owned = self.key_owner_last.is_some() || self.drawer.is_some() || egui::Popup::is_any_open(ctx);
        let part = self.key_part(ctx);
        let cx = self.action_cx();
        self.parts.actions.table.read().for_chord(chord, &cx, part, owned).map(|a| a.id)
    }

    /// What a chord runs on the active tab now, as the key pass finds it: an action's id; `None`
    /// when none takes it - the document may still take it as a key of its own.
    pub(crate) fn resolve_chord(&self, ctx: &Context, chord: &Chord) -> Option<String> {
        self.action_for_chord(ctx, chord).map(str::to_owned)
    }

    pub(crate) fn open_palette(&mut self, query: &str) {
        self.palette.open = true;
        query.clone_into(&mut self.palette.query);
        self.palette.selected = 0;
        self.palette.focus = true;
        self.palette.scroll_to_selected = true;
    }

    // ---- requests --------------------------------------------------------------------------

    pub(crate) fn apply_requests(&mut self, ctx: &Context) {
        // what one request does may ask for more; a loop that never settles stops here
        for _ in 0..8 {
            let list = std::mem::take(&mut self.requests.list);
            if list.is_empty() {
                return;
            }
            for r in list {
                self.apply(ctx, r);
            }
        }
    }

    fn apply(&mut self, ctx: &Context, r: Request) {
        match r {
            Request::Open(req) => match self.open(ctx, req, false, self.asker()) {
                Ok(()) => {}
                Err(OpenError::Unsaved { label, .. }) => state::toast(
                    ctx,
                    Tone::Warning,
                    gz_ui::tr!("{label} has unsaved changes.", label = label),
                ),
                Err(OpenError::Other(e)) => state::toast(ctx, Tone::Error, Tx::ui(e)),
            },
            Request::Activate(id) => {
                self.activate(ctx, &id);
            }
            Request::Close(id) => self.request_close(ctx, &id),
            Request::Dialog(id) => {
                if let Err(e) = self.open_dialog(ctx, &id) {
                    state::toast(ctx, Tone::Error, Tx::ui(e));
                }
            }
            Request::Drawer(Some(d)) => {
                self.open_drawer(d);
            }
            Request::Drawer(None) => self.drawer = None,
            Request::ToggleDrawer(d) => self.toggle_drawer(ctx, d),
            Request::Toast(tone, text) => state::toast(ctx, tone, text),
            Request::Palette => self.open_palette(""),
            Request::Panel(id, op) => {
                self.panel(ctx, &id, op);
            }
            Request::Exit => self.request_exit(ctx),
            Request::TaskCard(shown) => self.show_card(shown),
            Request::Popup(name) => {
                let _ = self.show_popup(ctx, &name);
            }
            Request::OwnKeys(who) => self.key_owner_now = Some(SmolStr::new(who)),
            Request::Action(id) => {
                if let Err(e) = self.run_action(&id) {
                    state::toast(ctx, Tone::Warning, Tx::ui(e));
                }
            }
            Request::CaptureKeys => self.capture_now = true,
        }
    }

    /// Opens a registered dialog by its MCP id.
    pub(crate) fn open_dialog(&mut self, ctx: &Context, id: &str) -> Result<(), String> {
        let Some(i) = self.parts.dialogs.iter().position(|d| d.id == id) else {
            return Err(if crate::contract::DIALOGS.iter().any(|(d, _)| *d == id) {
                format!("the dialog '{id}' is not installed in this shell")
            } else {
                format!("unknown dialog '{id}' (ui.contract lists them)")
            });
        };
        let needs_project = self.parts.dialogs.get(i).is_some_and(|d| d.needs_project);
        if needs_project && self.editor.project_opt().is_none() {
            return Err("no project is open".to_owned());
        }
        let Some(d) = self.parts.dialogs.get_mut(i) else { return Err("the dialog went away".to_owned()) };
        (d.open)(&mut DocCx {
            editor: &self.editor,
            gpu: &self.gpu,
            ctx,
            active_tab: &self.active,
            requests: &mut self.requests,
        })
    }

    // ---- the frame -------------------------------------------------------------------------

    fn frame(&mut self, ui: &mut Ui) -> FrameReport {
        let ctx = ui.ctx().clone();
        let now = Instant::now();
        if let Some(last) = self.last_frame.replace(now) {
            let dt = now.duration_since(last).as_secs_f32();
            if dt > 0.0 {
                self.fps = if self.fps == 0.0 { 1.0 / dt } else { self.fps * 0.9 + 0.1 / dt };
            }
        }
        // what other threads asked for lands in this frame, each timed for the stall monitor
        self.phases.clear();
        for job in self.queue.drain() {
            let t = Instant::now();
            (job.run)(self, &ctx);
            self.phases.add(crate::stalls::command_of(job.label), t.elapsed().as_secs_f32() * 1000.0);
        }
        self.apply_requests(&ctx);
        if audit::wanted(&ctx) {
            audit::new_frame(&ctx);
        }
        self.popups_cur.clear();
        let t = Instant::now();
        self.begin_frame(&ctx);
        self.refresh_edits(&ctx);
        self.keys(&ctx);
        self.apply_requests(&ctx);
        self.phases.add("frame start", t.elapsed().as_secs_f32() * 1000.0);

        let t = Instant::now();
        gz_ui::drop::begin(&ctx);
        self.draw_frame(ui);
        self.apply_requests(&ctx);
        self.phases.add(format!("ui pass ({})", self.active), t.elapsed().as_secs_f32() * 1000.0);
        let t = Instant::now();
        dialog::draw_overlays(ui);
        self.apply_requests(&ctx);
        self.end_frame(&ctx);
        self.phases.add("dialogs and the end of the frame", t.elapsed().as_secs_f32() * 1000.0);
        self.frames += 1;
        FrameReport {
            active_tab: self.active.clone(),
            during: self.phases.longest(),
            first: self.frames == 1,
        }
    }

    fn begin_frame(&mut self, ctx: &Context) {
        self.sync_session(ctx);
        // an owner that closed its document took the tab with it
        while let Some(i) =
            self.tabs.iter().position(|t| t.id != LEVEL && t.doc.as_ref().is_some_and(|d| !d.alive()))
        {
            self.remove(ctx, i);
        }
        if self.find(&self.active).is_none() {
            self.active = SmolStr::default();
            if let Some(first) = self.tabs.first().map(|t| t.id.clone()) {
                self.activate(ctx, &first);
            }
        }
        let project_ready = self.editor.project_opt().is_some_and(|_| project_linked(&self.editor));
        let card_wants = self.parts.page_cards.iter().any(|c| (c.wants)(&self.editor));
        self.start_page = !project_ready;
        self.menus_shown = project_ready && !card_wants;
        // a job the UI started comes with its card; an MCP caller never gets one
        let running: Vec<gz_editor::JobInfo> =
            self.editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running).collect();
        for job in &running {
            if !self.seen_jobs.contains(&job.id)
                && let Some(card) = crate::tasks::card_of(&self.editor, job.id)
            {
                if !card.from_mcp && !self.card_open {
                    self.card_open = true;
                    self.card_auto = true;
                }
                self.seen_jobs.push(job.id);
            }
        }
        let ended = !self.seen_jobs.is_empty() && running.is_empty();
        self.seen_jobs.retain(|id| running.iter().any(|j| j.id == *id));
        self.cancelled.retain(|id| running.iter().any(|j| j.id == *id));
        JobMarks::of(&self.editor).cancelled.lock().retain(|id| running.iter().any(|j| j.id == *id));
        crate::tasks::sweep(&self.editor, &running);
        // the last job ended: its row says how for a moment, then the card goes
        if ended {
            self.last_job_end = Some(Instant::now());
        }
        if running.is_empty()
            && self.card_open
            && self.card_auto
            && self.last_job_end.is_none_or(|t| t.elapsed() >= Duration::from_secs(4))
        {
            self.card_open = false;
            self.card_auto = false;
        }
    }

    /// The Task Center as the author asked for it: it stays until they close it.
    pub(crate) fn show_card(&mut self, on: bool) {
        self.card_open = on;
        self.card_auto = false;
    }

    fn end_frame(&mut self, ctx: &Context) {
        JobMarks::of(&self.editor).card_open.store(self.card_open, std::sync::atomic::Ordering::Release);
        self.popups_last = std::mem::take(&mut self.popups_cur);
        self.key_owner_last = self.key_owner_now.take();
        self.capture_last = std::mem::take(&mut self.capture_now);
        if self.session.dirty {
            self.write_session();
        }
        // the window's place and the layout store, at most once a second
        let viewport = ctx.input(|i| i.viewport().clone());
        if viewport.minimized != Some(true)
            && let Some(inner) = viewport.inner_rect
        {
            let maximized = viewport.maximized == Some(true);
            let p = Placement {
                pos: if maximized {
                    self.placement.and_then(|p| p.pos)
                } else {
                    viewport.outer_rect.map(|r| [r.min.x, r.min.y])
                },
                size: if maximized {
                    self.placement.map_or([inner.width(), inner.height()], |p| p.size)
                } else {
                    [inner.width(), inner.height()]
                },
                maximized,
            };
            if self.placement != Some(p) {
                self.placement = Some(p);
                self.settings.set_placement(p);
            }
        }
        if layout::take_dirty(ctx) {
            self.settings.set_layout(&layout::all(ctx));
            self.settings.set_lists(&layout::all_lists(ctx));
        }
        if self.settings.is_dirty() {
            let since = self.last_settings_save.elapsed();
            if since >= Duration::from_secs(1) {
                self.last_settings_save = Instant::now();
                self.settings.save_soon(&self.editor);
            } else {
                ctx.request_repaint_after(Duration::from_secs(1) - since);
            }
        }
    }

    // ---- the session -----------------------------------------------------------------------

    fn session_text(&self) -> String {
        let tabs: Vec<SessionTab> = self
            .tabs
            .iter()
            .filter(|t| t.id != LEVEL && self.parts.factories.contains_key(t.kind.as_str()))
            .map(|t| {
                let arg = match &t.doc {
                    None => t.path.clone(),
                    Some(d) => d.restore_arg().unwrap_or_else(|| {
                        t.id.split_once(':').map(|(_, rest)| rest.to_owned()).unwrap_or_default()
                    }),
                };
                SessionTab {
                    id: t.id.to_string(),
                    kind: t.kind.to_string(),
                    icon: t.icon,
                    label: t.label_en(),
                    arg,
                }
            })
            .collect();
        tabs::encode_session(&tabs)
    }

    fn write_session(&mut self) {
        self.session.dirty = false;
        let Some(path) = self.session.file.clone() else { return };
        let text = self.session_text();
        // the UI thread asks for every write of the session, so the tickets are in its order
        let (log, order, ticket) =
            (self.editor.log.clone(), self.session.order.clone(), self.session.order.ticket());
        self.editor.exec.spawn_io(move || {
            if let Err(e) = order.write(ticket, &path, text.as_bytes()) {
                log.warn(format!("the open tabs could not be written to {}: {e}", path.display()));
            }
        });
    }

    /// Writes the session now: the editor is closing.
    pub(crate) fn flush_session(&mut self) {
        if !self.session.dirty {
            return;
        }
        self.session.dirty = false;
        if let Some(path) = self.session.file.clone() {
            let ticket = self.session.order.ticket();
            if let Err(e) = self.session.order.write(ticket, &path, self.session_text().as_bytes()) {
                self.editor
                    .log
                    .warn(format!("the open tabs could not be written to {}: {e}", path.display()));
            }
        }
    }

    /// Another project: the last one's tabs are written to its session first - before its
    /// documents close, which would leave that session empty - and the new one's come back,
    /// lazily, from a read on the IO pool.
    fn sync_session(&mut self, ctx: &Context) {
        let root = self.editor.project_root();
        if root == self.session.root {
            return;
        }
        let first = self.session.root.is_none();
        if self.session.root.is_some() {
            if self.session.dirty {
                self.write_session();
            }
            // the project's tabs close with it and come back when it opens again
            while let Some(i) = self.tabs.iter().position(|t| t.id != LEVEL) {
                self.remove(ctx, i);
            }
        }
        self.session.root = root.clone();
        self.session.file = root.as_ref().and_then(|_| crate::project::project_info(&self.editor).tabs_file);
        self.session.dirty = false;
        let Some(root) = root else {
            self.activate(ctx, LEVEL);
            return;
        };
        // documents opened before the project's session was known are this project's
        if first && self.tabs.iter().any(|t| t.id != LEVEL && t.loaded()) {
            self.session.dirty = true;
        }
        let queue = self.queue.clone();
        let file = self.session.file.clone();
        self.editor.exec.spawn_io(move || {
            let text = file
                .and_then(|f| std::fs::read(f).ok())
                .map(|b| gz_base::text::decode_guess(&b).into_owned())
                .unwrap_or_default();
            let tabs = tabs::decode_session(&text);
            queue.post(move |shell, _ctx| shell.restore_session(&root, tabs));
        });
        if self.find(&self.active).is_none() {
            self.activate(ctx, LEVEL);
        }
    }

    fn restore_session(&mut self, root: &Path, list: Vec<SessionTab>) {
        if self.session.root.as_deref() != Some(root) {
            return;
        }
        for t in list {
            if self.find(&t.id).is_some() || !self.parts.factories.contains_key(t.kind.as_str()) {
                continue;
            }
            self.tabs.push(Tab {
                id: SmolStr::new(&t.id),
                kind: SmolStr::new(&t.kind),
                icon: t.icon,
                label: t.label,
                path: t.arg,
                doc: None,
                failed: false,
                panicked: None,
                rect: None,
                icon_only: false,
            });
        }
    }

    // ---- the document body, guarded --------------------------------------------------------

    /// Runs the active document's body; a panic shows an error page and the tab stays closable.
    pub(crate) fn draw_body(&mut self, ui: &mut Ui, i: usize) {
        let ctx = ui.ctx().clone();
        let Some(tab) = self.tabs.get_mut(i) else { return };
        let Some(doc) = tab.doc.as_mut() else { return };
        let mut cx = DocCx {
            editor: &self.editor,
            gpu: &self.gpu,
            ctx: &ctx,
            active_tab: &self.active,
            requests: &mut self.requests,
        };
        self.panic_guard.store(true, Ordering::SeqCst);
        let ran = std::panic::catch_unwind(AssertUnwindSafe(|| doc.body(ui, &mut cx)));
        self.panic_guard.store(false, Ordering::SeqCst);
        if let Err(payload) = ran {
            let text = gz_task::panic_message(&*payload);
            self.editor.log.error(format!("the tab '{}' stopped on an internal error: {text}", tab.id));
            tab.panicked = Some(text);
        }
    }

    /// Ticks the documents that are not drawn.
    pub(crate) fn tick_inactive(&mut self, ctx: &Context) {
        for i in 0..self.tabs.len() {
            let Some(tab) = self.tabs.get_mut(i) else { continue };
            if tab.id == self.active || tab.panicked.is_some() {
                continue;
            }
            if let Some(doc) = tab.doc.as_mut() {
                doc.inactive(&mut DocCx {
                    editor: &self.editor,
                    gpu: &self.gpu,
                    ctx,
                    active_tab: &self.active,
                    requests: &mut self.requests,
                });
            }
        }
    }

    // ---- what MCP reads --------------------------------------------------------------------

    pub(crate) fn log_state(&self) -> LogState {
        match &self.parts.log_state {
            Some(f) => f(&self.editor),
            None => default_log_state(&self.editor),
        }
    }

    pub(crate) fn note_popup(&mut self, id: &str, title: &str, window: &str, rect: Rect) {
        self.popups_cur.push(PopupInfo {
            id: id.to_owned(),
            title: title.to_owned(),
            window: window.to_owned(),
            rect,
            label_x: Vec::new(),
        });
    }

    // ---- what MCP reads of the frame (pt, as the last frame left it) --------------------------

    #[must_use]
    pub fn palette_open(&self) -> bool {
        self.palette.open
    }

    pub(crate) fn palette_rect(&self) -> Rect {
        self.palette.rect
    }

    pub fn close_palette(&mut self) {
        self.palette.open = false;
    }

    pub(crate) fn popups(&self) -> &[PopupInfo] {
        &self.popups_last
    }

    pub(crate) fn work(&self) -> Rect {
        self.work_rect
    }

    #[must_use]
    pub fn start_page(&self) -> bool {
        self.start_page
    }

    #[must_use]
    pub fn folded(&self) -> bool {
        self.folded
    }

    pub(crate) fn has_logo(&self) -> bool {
        self.logo.is_some()
    }

    pub(crate) fn caption_geom(&self) -> Caption {
        self.caption
    }

    /// The tabs in the strip, where they are and whether they show their icon alone; none on
    /// the Start page.
    pub(crate) fn tab_rects(&self) -> Vec<(String, Rect, bool)> {
        if !self.menus_shown {
            return Vec::new();
        }
        self.tabs.iter().filter_map(|t| t.rect.map(|r| (t.id.to_string(), r, t.icon_only))).collect()
    }

    /// What the Level document says of itself to MCP (`layout_custom`, `viewport_maximized`).
    pub(crate) fn level_state(&self) -> serde_json::Value {
        self.tabs.first().and_then(|t| t.doc.as_ref()).map_or(serde_json::Value::Null, |d| d.mcp_state())
    }

    pub(crate) fn has_panel(&self, id: &str) -> bool {
        self.parts.panels.iter().any(|p| p.id == id)
    }

    pub(crate) fn panel_drawer(&self, id: &str) -> Option<Drawer> {
        self.parts.panels.iter().find(|p| p.id == id).and_then(|p| p.drawer)
    }

    /// The window an MCP id names: a panel, a drawer, a dialog, a kit dialog, the work area of
    /// the active tab, or else a window's own name.
    pub(crate) fn window_of(&self, id: &str) -> String {
        if let Some(w) = crate::contract::window_of_known(id) {
            return w.to_owned();
        }
        if id == "work_area" || id == self.active {
            return "###doc.host".to_owned();
        }
        if id.starts_with("dlg.") {
            return format!("###{id}");
        }
        id.to_owned()
    }

    // ---- the Keyboard page's table ---------------------------------------------------------

    /// Every action with its keys, as `app.keys` lists them.
    pub(crate) fn key_rows(&self) -> Vec<serde_json::Value> {
        let t = self.parts.actions.table.read();
        t.all().iter().map(|a| crate::actions::key_row(&t, a)).collect()
    }

    /// One row of `key_rows`, by its id.
    pub(crate) fn key_row(&self, id: &str) -> serde_json::Value {
        let t = self.parts.actions.table.read();
        t.get(id).map_or(serde_json::Value::Null, |a| crate::actions::key_row(&t, a))
    }

    /// Binds an action to `keys` (empty takes them away); answers what let a chord go. Saved
    /// soon.
    pub(crate) fn bind_keys(&mut self, id: &str, keys: Vec<Chord>) -> Result<Vec<String>, String> {
        if self.parts.actions.table.read().get(id).is_none() {
            return Err(format!("no action '{id}' takes keys (app.keys lists them)"));
        }
        let moved = self.parts.actions.table.write().bind(id, keys, &self.settings);
        self.settings.save_soon(&self.editor);
        Ok(moved)
    }

    /// Back to the defaults: one action's, or every one's.
    pub(crate) fn reset_keys(&mut self, id: Option<&str>) -> Result<(), String> {
        if let Some(one) = id
            && self.parts.actions.table.read().get(one).is_none()
        {
            return Err(format!("no action '{one}' (app.keys lists them)"));
        }
        self.parts.actions.table.write().reset(id, &self.settings);
        self.settings.save_soon(&self.editor);
        Ok(())
    }

    /// The keys an action is bound to now, as a menu row shows them.
    pub(crate) fn action_keys(&self, id: &str) -> String {
        self.parts.actions.table.read().keys_label(id)
    }

    /// An action by its id.
    pub(crate) fn action(&self, id: &str) -> Option<Action> {
        self.parts.actions.table.read().get(id).copied()
    }

    /// The Edit menu's action on a tab; `run = false` asks whether it is there at all.
    pub(crate) fn edit_tab(&mut self, ctx: &Context, id: &str, action: EditAction, run: bool) -> bool {
        let Some(i) = self.find(id) else { return false };
        let Some(tab) = self.tabs.get_mut(i) else { return false };
        if tab.failed {
            return false;
        }
        let Some(doc) = tab.doc.as_mut() else { return false };
        if doc.missing() {
            return false;
        }
        doc.edit(&mut cx!(self, ctx), action, run)
    }

    /// Reads a tab's file again.
    pub(crate) fn reload_tab(&mut self, ctx: &Context, id: &str, discard: bool) -> Result<(), String> {
        let Some(i) = self.find(id) else { return Err("not open".to_owned()) };
        let Some(tab) = self.tabs.get_mut(i) else { return Err("not open".to_owned()) };
        let Some(doc) = tab.doc.as_mut() else { return Err("the tab is not loaded".to_owned()) };
        doc.reload(&mut cx!(self, ctx), discard)
    }

    /// Runs `f` with a `DocCx` over this shell: a surface's command does its window's work
    /// through it (a question for a person, a request of the shell).
    pub fn with_cx<R>(&mut self, ctx: &Context, f: impl FnOnce(&mut DocCx<'_>) -> R) -> R {
        let r = f(&mut cx!(self, ctx));
        self.apply_requests(ctx);
        r
    }

    pub(crate) fn mark_session_dirty(&mut self) {
        self.session.dirty = true;
    }

    pub(crate) fn job_cancel(&mut self, id: JobId) -> bool {
        self.cancelled.insert(id);
        JobMarks::of(&self.editor).cancelled.lock().insert(id);
        self.editor.jobs.cancel(id)
    }

    /// The UI's scale: a test's (`ui.scale`), else `-uiscale`, else Preferences > Interface >
    /// Interface Scale, else the monitor's. Applied when one of them or the monitor changes.
    fn apply_scale(&mut self, ctx: &Context, command_line: Option<f32>) {
        let native = ctx.native_pixels_per_point().unwrap_or(1.0).max(0.1);
        let wanted = self.test_scale.or(command_line).or_else(|| self.settings.interface_scale());
        if self.scale_applied == Some((wanted, native)) {
            return;
        }
        self.scale_applied = Some((wanted, native));
        ctx.set_zoom_factor(wanted.map_or(1.0, |s| s / native));
        ctx.request_repaint();
    }

    fn close_requested(&mut self, ctx: Option<&Context>) -> bool {
        if self.dirty_tabs().is_empty() {
            return true;
        }
        if let Some(ctx) = ctx {
            self.request_exit(ctx);
        }
        false
    }

    fn exiting(&mut self) {
        self.flush_session();
        self.settings.save_now(&self.editor);
    }
}

fn op_open(op: PanelOp) -> PanelOp {
    if op == PanelOp::Toggle { PanelOp::Open } else { op }
}

/// Whether the open project has its game linked: what the Start page gives way to.
fn project_linked(editor: &Editor) -> bool {
    crate::project::project_info(editor).game_linked
}

/// What the log says by itself: its last line worth reading and its counts.
#[must_use]
/// What `progress` reads without the UI thread: the jobs whose Stop was asked for, and the log
/// state the status bar shows (the shell's hook, set when the shell is made).
#[derive(Default)]
pub(crate) struct JobMarks {
    pub cancelled: parking_lot::Mutex<FxHashSet<JobId>>,
    /// Whether the Task Center's card is up, as the last frame left it: what `ui.tasks`
    /// answers without the UI thread.
    pub card_open: std::sync::atomic::AtomicBool,
    pub log_state: parking_lot::Mutex<Option<crate::builder::LogStateFn>>,
}

impl JobMarks {
    pub(crate) fn of(editor: &Editor) -> std::sync::Arc<JobMarks> {
        editor.services.get_or_init(JobMarks::default)
    }

    pub(crate) fn log_state(&self, editor: &Editor) -> LogState {
        let hook = self.log_state.lock().clone();
        hook.map_or_else(|| default_log_state(editor), |f| f(editor))
    }
}

pub fn default_log_state(editor: &Editor) -> LogState {
    let (errors, warnings) = editor.log.counts();
    let last = editor.log.last_report().map(|r| r.text.clone()).unwrap_or_default();
    LogState { last, errors, warnings }
}

/// A path as a tab shows it in the palette: project-relative when it lies in the project.
#[must_use]
pub fn project_relative(root: Option<&Path>, path: &str) -> String {
    let Some(root) = root else { return path.to_owned() };
    let p = Path::new(path);
    match p.strip_prefix(root) {
        Ok(rel) => rel.display().to_string().replace('\\', "/"),
        Err(_) if p.is_absolute() => path.to_owned(),
        Err(_) => path.replace('\\', "/"),
    }
}

/// Why an open did not happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum OpenError {
    /// The tab to switch has unsaved changes.
    Unsaved {
        id: String,
        label: String,
    },
    Other(String),
}

/// The window's first size and place, from the editor's settings.
#[must_use]
pub fn window_options(editor: &Editor) -> WindowOptions {
    // app.window resize goes down to 320 x 240 pt: the frame does not stop it
    let mut opts = WindowOptions {
        decorations: false,
        min_size: [320.0, 240.0],
        icon: crate::chrome::logo_rgba(64).map(|rgba| (rgba, 64)),
        ..WindowOptions::default()
    };
    let settings = EditorSettings::of(editor);
    if let Some(p) = settings.placement() {
        opts.size = [p.size[0].max(opts.min_size[0]), p.size[1].max(opts.min_size[1])];
        opts.position = p.pos;
        opts.maximized = p.maximized;
    }
    // read once: both take a restart
    opts.gpu_adapter = settings.gpu_adapter();
    opts.floating = settings.pref("floating_panels").is_some_and(|v| v != 0.0);
    // what the window shows before its device exists: the theme's background
    let base = settings.theme().palette().base;
    opts.background = [base.r(), base.g(), base.b()];
    opts
}

/// Whether panels may leave the window in this run (`[ui] floating_panels` as it was read when
/// the window opened).
#[must_use]
pub fn floating_panels(ctx: &Context) -> bool {
    !ctx.embed_viewports()
}

/// What the window runs: the shell with its tabs.
pub struct GrizzlyShell {
    editor: Arc<Editor>,
    queue: Arc<UiQueue>,
    builder: Option<ShellBuilder>,
    opts: ShellOptions,
    state: Option<ShellState>,
    ctx: Option<Context>,
    /// The window's language, its catalogs already being read while the window comes up.
    language: Option<crate::lang::Language>,
}

impl GrizzlyShell {
    #[must_use]
    pub fn new(editor: Arc<Editor>, builder: ShellBuilder, opts: ShellOptions) -> Self {
        let wake = editor.exec.wake().clone();
        let queue = editor.services.get_or_init(|| UiQueue::new(wake));
        // a start in another language than English reads its catalogs on the CPU pool now, while
        // the window and its device come up: the first frame finds them read
        let mut language = crate::lang::Language::new(builder.translations.clone(), opts.lang);
        let wanted = language.wanted(&EditorSettings::of(&editor));
        language.prefetch(wanted, &editor);
        Self { editor, queue, builder: Some(builder), opts, state: None, ctx: None, language: Some(language) }
    }
}

impl Shell for GrizzlyShell {
    fn init(&mut self, ctx: &Context, gpu: &RenderState) {
        let settings = EditorSettings::of(&self.editor);
        gz_ui::font::install(ctx, std::mem::take(&mut self.opts.fonts));
        gz_ui::theme::install(ctx, settings.theme());
        layout::load(ctx, settings.layout());
        layout::load_lists(ctx, settings.lists());
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        let guard = Arc::new(AtomicBool::new(false));
        if !self.editor.flags.unattended {
            install_tab_panic_guard(&self.editor, guard.clone());
        }
        let builder = self.builder.take().unwrap_or_default();
        let mut state = ShellState::new(
            self.editor.clone(),
            gpu.clone(),
            self.queue.clone(),
            builder,
            (self.opts.iconsheet, self.opts.kitsheet),
            guard,
            ctx.clone(),
        );
        state.logo = crate::chrome::load_logo(ctx);
        if let Some(language) = self.language.take() {
            state.language = language;
        }
        state.language.apply(ctx, &state.settings, &state.editor);
        self.state = Some(state);
        self.ctx = Some(ctx.clone());
    }

    fn frame(&mut self, ui: &mut Ui) -> FrameReport {
        match self.state.as_mut() {
            Some(s) => {
                s.apply_scale(ui.ctx(), self.opts.ui_scale);
                s.language.apply(ui.ctx(), &s.settings, &s.editor);
                s.frame(ui)
            }
            None => FrameReport::default(),
        }
    }

    fn close_requested(&mut self) -> bool {
        match self.state.as_mut() {
            Some(s) => s.close_requested(self.ctx.as_ref()),
            None => true,
        }
    }

    fn exiting(&mut self) {
        self.queue.close();
        if let Some(s) = self.state.as_mut() {
            s.exiting();
        }
    }
}

/// A panic inside a document's body must not end the editor: while a body runs on the UI
/// thread the hook only logs, and the body's `catch_unwind` shows the tab's error page. Any
/// other panic goes to the hook that was there before.
fn install_tab_panic_guard(editor: &Editor, guard: Arc<AtomicBool>) {
    let previous = std::panic::take_hook();
    let ui_thread = std::thread::current().id();
    let log = editor.log.clone();
    std::panic::set_hook(Box::new(move |info| {
        if std::thread::current().id() == ui_thread && guard.load(Ordering::SeqCst) {
            let at = info
                .location()
                .map_or_else(|| "unknown location".to_owned(), |l| format!("{}:{}", l.file(), l.line()));
            log.error(format!("internal error in a tab at {at}"));
            for line in std::backtrace::Backtrace::force_capture().to_string().lines() {
                log.plain(format!("    {}", line.trim_end()));
            }
            return;
        }
        previous(info);
    }));
}
