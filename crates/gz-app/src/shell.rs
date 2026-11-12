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
use gz_ui::{Icon, Tone, audit, layout, state};
use rustc_hash::{FxHashMap, FxHashSet};

use crate::builder::{
    DialogDesc, DrawFn, DrawerDesc, LogState, MenuEntry, PageCard, PanelDesc, ShellBuilder, ToolbarFn,
    UiCommand,
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
    /// `-uiscale <f>` (0.5..4): the UI at this scale, whatever the monitor says
    /// (`CommandLineScale`). Taken again when the window meets another monitor's scale.
    pub ui_scale: Option<f32>,
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
}

impl Tab {
    pub fn loaded(&self) -> bool {
        self.doc.is_some()
    }

    pub fn label(&self) -> String {
        if let Some(d) = &self.doc {
            let l = d.label();
            if !l.is_empty() {
                return l;
            }
        }
        if !self.label.is_empty() {
            return self.label.clone();
        }
        let p = self.path();
        if p.is_empty() { self.id.to_string() } else { tabs::label_of_path(&p) }
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

    pub fn tooltip(&self) -> String {
        let mut text = self.doc.as_ref().and_then(|d| d.tooltip()).unwrap_or_else(|| self.path());
        if text.is_empty() {
            text = self.label();
        }
        if self.dirty() {
            text.push_str(if self.id == LEVEL {
                "\nThe scene has unsaved changes."
            } else {
                "\nUnsaved changes."
            });
        }
        if self.missing() {
            text.push_str("\nFile not found.");
        }
        text
    }

    pub fn info_json(&self, active: bool) -> serde_json::Value {
        serde_json::json!({
            "id": self.id,
            "kind": self.kind,
            "label": self.label(),
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
    pub commands: Vec<UiCommand>,
    pub menus: Vec<MenuEntry>,
    pub project_toolbar: Vec<(i32, ToolbarFn)>,
    pub drawers: [Option<DrawerDesc>; 2],
    pub status_chips: Vec<(i32, DrawFn)>,
    pub start_page: Option<DrawFn>,
    pub page_cards: Vec<PageCard>,
    pub dialogs: Vec<DialogDesc>,
    pub panels: Vec<PanelDesc>,
    pub log_state: Option<crate::builder::LogStateFn>,
    pub reveal: Option<crate::builder::RevealFn>,
    pub reset_layout: Option<crate::builder::ResetLayoutFn>,
    /// The C++ command table over `commands`.
    pub cpp: crate::cpp::Table,
}

/// A popup on screen, as `ui_list` names it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PopupInfo {
    pub id: String,
    pub title: String,
    pub window: String,
    /// pt.
    pub rect: Rect,
}

/// The caption's geometry as the last frame left it, in pt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Caption {
    pub bar: Rect,
    pub menus_x0: f32,
    pub menus_end: f32,
    pub buttons_x: f32,
    pub valid: bool,
}

impl Default for Caption {
    fn default() -> Self {
        Self { bar: Rect::NOTHING, menus_x0: 0.0, menus_end: 0.0, buttons_x: 0.0, valid: false }
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
        }
    }
}

#[derive(Default)]
struct Session {
    root: Option<PathBuf>,
    dirty: bool,
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
    /// Shortcuts the author bound: `None` is one taken away.
    pub(crate) bindings: FxHashMap<SmolStr, Option<Chord>>,
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
    pub(crate) card_rect: Option<Rect>,
    /// Where the active document's status drew in the status bar's centre last frame; `None`
    /// on the Start page and its cards, which are no document.
    pub(crate) status_centre: Option<Rect>,
    seen_jobs: Vec<JobId>,
    pub(crate) cancelled: FxHashSet<JobId>,
    session: Session,
    pub(crate) logo: Option<TextureHandle>,
    pub(crate) iconsheet: bool,
    pub(crate) tab_menu: Option<SmolStr>,
    pub(crate) drag_tab: Option<SmolStr>,
    warned: FxHashSet<SmolStr>,
    last_settings_save: Instant,
    placement: Option<Placement>,
    last_frame: Option<Instant>,
    pub(crate) fps: f32,
    panic_guard: Arc<AtomicBool>,
    /// `exec` logged the command it runs: `run_command` does not log it again.
    pub(crate) quiet_log: bool,
}

/// The Level tab a build without the level editor shows.
struct LevelPlaceholder;

impl Document for LevelPlaceholder {
    fn id(&self) -> &str {
        LEVEL
    }
    fn kind(&self) -> &'static str {
        "level"
    }
    fn label(&self) -> String {
        "Level".to_owned()
    }
    fn icon(&self) -> Icon {
        Icon::Level
    }
    fn closable(&self) -> bool {
        false
    }
    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        state::empty_state(ui, Icon::Level, "Level", "The level editor is not part of this build yet.", None);
    }
}

impl ShellState {
    fn new(
        editor: Arc<Editor>,
        gpu: RenderState,
        queue: Arc<UiQueue>,
        builder: ShellBuilder,
        iconsheet: bool,
        panic_guard: Arc<AtomicBool>,
    ) -> Self {
        let settings = EditorSettings::of(&editor);
        let mut bindings = FxHashMap::default();
        for (id, keys) in settings.shortcuts() {
            bindings.insert(id, Chord::parse(&keys));
        }
        let ShellBuilder {
            factories,
            commands,
            menus,
            project_toolbar,
            drawers,
            status_chips,
            start_page,
            page_cards,
            dialogs,
            panels,
            log_state,
            reveal,
            reset_layout,
            cpp,
        } = builder;
        JobMarks::of(&editor).log_state.lock().clone_from(&log_state);
        let mut parts = Parts {
            factories,
            commands,
            menus,
            project_toolbar,
            drawers,
            status_chips,
            start_page,
            page_cards,
            dialogs,
            panels,
            log_state,
            reveal,
            reset_layout,
            cpp: crate::cpp::Table::default(),
        };
        parts.project_toolbar.sort_by_key(|(o, _)| *o);
        parts.status_chips.sort_by_key(|(o, _)| *o);
        add_shell_commands(&mut parts.commands);
        // the C++ table: what the shell knows of the C++ rows, the shell's own, then the areas'
        let mut rows = crate::cpp::known_rows();
        rows.extend(crate::cpp::shell_rows());
        rows.extend(cpp);
        parts.cpp = crate::cpp::Table::build(rows, &mut parts.commands);
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
            bindings,
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
            status_centre: None,
            card_rect: None,
            seen_jobs: Vec::new(),
            cancelled: FxHashSet::default(),
            session: Session::default(),
            logo: None,
            iconsheet,
            tab_menu: None,
            drag_tab: None,
            warned: FxHashSet::default(),
            last_settings_save: Instant::now(),
            placement: None,
            last_frame: None,
            fps: 0.0,
            panic_guard,
            quiet_log: false,
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

    /// Every tab, as `ui_tab action=list` answers it.
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
            None => Err(format!("no tab of the kind '{}' in this build", req.kind)),
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
                format!("{} is not part of this build yet", req.kind)
            } else {
                format!("unknown kind '{}' (ui_contract lists the document kinds)", req.kind)
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
                        dialog::unsaved_prompt(ctx, Asker::Ui, "retarget_tab", &[label], move |answer| {
                            if answer == Answer::Cancel {
                                return;
                            }
                            queue.post(move |shell, ctx| {
                                if answer == Answer::Yes
                                    && let Err(e) = shell.save_tab(ctx, &target)
                                {
                                    let _ = gz_ui::dialog::notice(
                                        ctx,
                                        shell.asker(),
                                        Tone::Error,
                                        "Save failed",
                                        &e,
                                    );
                                    return;
                                }
                                let _ = shell.open(ctx, retarget, true, Asker::Ui);
                            });
                        });
                        return Ok(());
                    }
                    return Err(OpenError::Unsaved { id: id.to_string(), label: tab.label() });
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
        let line = dialog::unsaved_prompt(ctx, asker, "close_tab", &what, move |answer| {
            if answer == Answer::Cancel {
                return;
            }
            let mode = if answer == Answer::Yes { CloseMode::SaveFirst } else { CloseMode::Discard };
            queue.post(move |shell, ctx| {
                if let Err(e) = shell.close_tab(ctx, &target, mode) {
                    gz_ui::dialog::notice(ctx, shell.asker(), Tone::Error, "Save failed", &e);
                }
            });
        });
        if let Some(line) = line {
            self.editor.log.plain(line);
        }
    }

    /// MCP's close: `IfClean` refuses a dirty tab ("unsaved"), `SaveFirst` saves it, `Discard`
    /// drops the changes.
    pub(crate) fn close_tab(&mut self, ctx: &Context, id: &str, mode: CloseMode) -> Result<(), String> {
        let Some(i) = self.find(id) else { return Err("not open".to_owned()) };
        let Some(tab) = self.tabs.get(i) else { return Err("not open".to_owned()) };
        if !tab.closable() {
            return Err("the Level tab does not close".to_owned());
        }
        if tab.dirty() {
            match mode {
                CloseMode::IfClean => return Err("unsaved".to_owned()),
                CloseMode::SaveFirst => self.save_tab(ctx, id)?,
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

    /// A tab's save without closing it; `Ok` when it is clean.
    pub(crate) fn save_tab(&mut self, ctx: &Context, id: &str) -> Result<(), String> {
        let Some(i) = self.find(id) else { return Err("not open".to_owned()) };
        let Some(tab) = self.tabs.get_mut(i) else { return Err("not open".to_owned()) };
        if !tab.dirty() {
            return Ok(());
        }
        let Some(doc) = tab.doc.as_mut() else { return Ok(()) };
        doc.save(&mut cx!(self, ctx))
            .map_err(|e| if e.is_empty() { "it cannot be saved".to_owned() } else { e })
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

    pub(crate) fn save_active(&mut self, ctx: &Context) {
        let id = self.active.to_string();
        let Some(i) = self.find(&id) else { return };
        let Some(tab) = self.tabs.get_mut(i) else { return };
        let Some(doc) = tab.doc.as_mut() else { return };
        if let Err(e) = doc.save(&mut cx!(self, ctx))
            && !e.is_empty()
            && let Some(line) = dialog::notice(ctx, self.asker(), Tone::Error, "Save failed", &e)
        {
            self.editor.log.plain(line);
        }
    }

    /// The Edit menu's action on the active tab; `run = false` asks whether it is there at all.
    pub(crate) fn edit(&mut self, ctx: &Context, action: EditAction, run: bool) -> bool {
        let id = self.active.to_string();
        let Some(i) = self.find(&id) else { return false };
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

    /// The tabs with unsaved changes: id and label.
    pub(crate) fn dirty_tabs(&self) -> Vec<(String, String)> {
        self.tabs.iter().filter(|t| t.dirty()).map(|t| (t.id.to_string(), t.label())).collect()
    }

    /// Quit, after the question that names every dirty document.
    pub(crate) fn request_exit(&mut self, ctx: &Context) {
        let dirty = self.dirty_tabs();
        if dirty.is_empty() {
            self.editor.request_quit();
            return;
        }
        let what: Vec<String> = dirty.iter().map(|(_, l)| l.clone()).collect();
        let ids: Vec<String> = dirty.into_iter().map(|(id, _)| id).collect();
        let queue = self.queue.clone();
        let line = dialog::unsaved_prompt(ctx, self.asker(), "exit", &what, move |answer| {
            if answer == Answer::Cancel {
                return;
            }
            queue.post(move |shell, ctx| {
                for id in &ids {
                    if answer == Answer::Yes {
                        if let Err(e) = shell.save_tab(ctx, id) {
                            let label = shell
                                .find(id)
                                .and_then(|i| shell.tabs.get(i))
                                .map(Tab::label)
                                .unwrap_or_default();
                            gz_ui::dialog::notice(
                                ctx,
                                shell.asker(),
                                Tone::Error,
                                "Save failed",
                                &format!("{label}: {e}"),
                            );
                            return;
                        }
                    } else {
                        shell.discard_tab(ctx, id);
                    }
                }
                shell.editor.request_quit();
            });
        });
        if let Some(line) = line {
            self.editor.log.plain(line);
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

    /// Opens a drawer over the active tab. `false` when this build has no body for it.
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

    // ---- commands and keys -----------------------------------------------------------------

    /// What a command is bound to now.
    pub(crate) fn binding(&self, i: usize) -> Option<Chord> {
        let c = self.parts.commands.get(i)?;
        match self.bindings.get(&c.id) {
            Some(over) => *over,
            None => c.default_keys,
        }
    }

    pub(crate) fn command_index(&self, id: &str) -> Option<usize> {
        self.parts.commands.iter().position(|c| c.id.eq_ignore_ascii_case(id))
    }

    pub(crate) fn command_enabled(&self, i: usize) -> bool {
        self.parts
            .commands
            .get(i)
            .is_some_and(|c| c.enabled.as_ref().is_none_or(|f| f(&self.editor, &self.active)))
    }

    /// Runs a command when it is on; `false` when it is not.
    pub(crate) fn run_command(&mut self, ctx: &Context, i: usize) -> bool {
        if !self.command_enabled(i) {
            return false;
        }
        // Log Commands: a face of a C++ row logs itself; a UiCommand of its own is logged here
        if !self.quiet_log && crate::cpp::CommandLog::of(&self.editor).on() {
            for row in &self.parts.cpp.rows {
                if !matches!(row.cmd.action, crate::cpp::CppAction::Ui) {
                    continue;
                }
                if let Some(k) = row.faces.iter().position(|f| *f == Some(i))
                    && let Some(sub) = row.cmd.subs.get(k)
                {
                    crate::cpp::log_command(&self.editor, &row.cmd.name, &sub.p1, &sub.p2, 0);
                }
            }
        }
        let Some(c) = self.parts.commands.get_mut(i) else { return false };
        (c.run)(&mut DocCx {
            editor: &self.editor,
            gpu: &self.gpu,
            ctx,
            active_tab: &self.active,
            requests: &mut self.requests,
        });
        true
    }

    /// The keys bound to a command id now, as text; `fallback` when it has none.
    pub(crate) fn keys_of(&self, id: &str, fallback: &str) -> String {
        self.command_index(id)
            .and_then(|i| self.binding(i))
            .map_or_else(|| fallback.to_owned(), |c| c.label())
    }

    pub(crate) fn modal_up(&self, ctx: &Context) -> bool {
        self.palette.open
            || dialog::modal_on_top(ctx).is_some()
            || ctx.memory(|m| m.top_modal_layer().is_some())
    }

    /// The scene's bindings may act: Level is up, nothing modal, no popup, no drawer.
    pub(crate) fn scene_keys_allowed(&self, ctx: &Context) -> bool {
        self.active == LEVEL
            && !self.modal_up(ctx)
            && !egui::Popup::is_any_open(ctx)
            && self.drawer.is_none()
            && self.key_owner_last.is_none()
    }

    /// Keys, first taker wins: a text field, the shell's chords, the document, the bindings.
    fn keys(&mut self, ctx: &Context) {
        if self.start_page || ctx.egui_wants_keyboard_input() || self.modal_up(ctx) {
            return;
        }
        let presses = ctx.input(|i| keys::pressed(&i.events));
        for chord in presses {
            if keys::shell_chord(&chord) {
                ctx.input_mut(|i| keys::consume(i, &chord));
                self.shell_chord(ctx, &chord);
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
                continue;
            }
            if !self.scene_keys_allowed(ctx) {
                continue;
            }
            let found = (0..self.parts.commands.len()).find(|&i| self.binding(i) == Some(chord));
            if let Some(i) = found {
                ctx.input_mut(|inp| keys::consume(inp, &chord));
                self.run_command(ctx, i);
            }
        }
    }

    fn shell_chord(&mut self, ctx: &Context, chord: &Chord) {
        use egui::Key;
        match (chord.key, chord.ctrl, chord.shift) {
            (Key::P, true, true) => self.open_palette(""),
            (Key::Tab, true, false) => self.cycle(ctx, 1),
            (Key::Tab, true, true) => self.cycle(ctx, -1),
            (Key::F4, true, false) => {
                let id = self.active.to_string();
                self.request_close(ctx, &id);
            }
            (Key::Space, true, false) => self.toggle_drawer(ctx, Drawer::Content),
            (Key::Backtick, true, false) => self.toggle_drawer(ctx, Drawer::Log),
            (Key::S, true, false) => self.save_active(ctx),
            (Key::F11, false, false) if self.active == LEVEL => {
                if let Some(doc) = self.tabs.first_mut().and_then(|t| t.doc.as_mut()) {
                    doc.shortcut(&mut cx!(self, ctx), chord);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn open_palette(&mut self, query: &str) {
        self.palette.open = true;
        query.clone_into(&mut self.palette.query);
        self.palette.selected = 0;
        self.palette.focus = true;
        self.palette.scroll_to_selected = true;
    }

    /// A binding that collides with one of the shell's chords is logged once and never acts.
    fn warn_collisions(&mut self) {
        for i in 0..self.parts.commands.len() {
            let Some(chord) = self.binding(i) else { continue };
            let Some(c) = self.parts.commands.get(i) else { continue };
            if keys::shell_chord(&chord)
                && c.id != "COMMAND_COMMAND_PALETTE"
                && self.warned.insert(c.id.clone())
            {
                self.editor.log.warn(format!(
                    "{} is bound to {}, which the editor's frame keeps for itself: the binding is ignored",
                    c.label,
                    chord.label()
                ));
            }
        }
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
                Err(OpenError::Unsaved { label, .. }) => {
                    state::toast(ctx, Tone::Warning, &format!("{label} has unsaved changes."))
                }
                Err(OpenError::Other(e)) => state::toast(ctx, Tone::Error, &e),
            },
            Request::Activate(id) => {
                self.activate(ctx, &id);
            }
            Request::Close(id) => self.request_close(ctx, &id),
            Request::Dialog(id) => {
                if let Err(e) = self.open_dialog(ctx, &id) {
                    state::toast(ctx, Tone::Error, &e);
                }
            }
            Request::Drawer(Some(d)) => {
                self.open_drawer(d);
            }
            Request::Drawer(None) => self.drawer = None,
            Request::ToggleDrawer(d) => self.toggle_drawer(ctx, d),
            Request::Toast(tone, text) => state::toast(ctx, tone, &text),
            Request::Palette => self.open_palette(""),
            Request::Panel(id, op) => {
                self.panel(ctx, &id, op);
            }
            Request::Exit => self.request_exit(ctx),
            Request::TaskCard(shown) => self.card_open = shown,
            Request::Popup(name) => {
                let _ = self.show_popup(ctx, &name);
            }
            Request::OwnKeys(who) => self.key_owner_now = Some(SmolStr::new(who)),
        }
    }

    /// Opens a registered dialog by its MCP id.
    pub(crate) fn open_dialog(&mut self, ctx: &Context, id: &str) -> Result<(), String> {
        let Some(i) = self.parts.dialogs.iter().position(|d| d.id == id) else {
            return Err(if crate::contract::DIALOGS.iter().any(|(d, _)| *d == id) {
                format!("{id} is not part of this build yet")
            } else {
                format!("unknown dialog '{id}' (ui_contract lists them)")
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
        // what other threads asked for lands in this frame
        for job in self.queue.drain() {
            job(self, &ctx);
        }
        self.apply_requests(&ctx);
        if audit::wanted(&ctx) {
            audit::new_frame(&ctx);
        }
        self.popups_cur.clear();
        self.begin_frame(&ctx);
        self.keys(&ctx);
        self.apply_requests(&ctx);

        self.draw_frame(ui);
        self.apply_requests(&ctx);
        dialog::draw_overlays(ui);
        self.apply_requests(&ctx);
        self.end_frame(&ctx);
        FrameReport { active_tab: self.active.clone() }
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
        self.warn_collisions();
        // a job the UI started comes with its card; an MCP caller never gets one
        let running: Vec<gz_editor::JobInfo> =
            self.editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running).collect();
        for job in &running {
            if !self.seen_jobs.contains(&job.id)
                && let Some(card) = crate::tasks::card_of(&self.editor, job.id)
            {
                if !card.from_mcp {
                    self.card_open = true;
                }
                self.seen_jobs.push(job.id);
            }
        }
        self.seen_jobs.retain(|id| running.iter().any(|j| j.id == *id));
        self.cancelled.retain(|id| running.iter().any(|j| j.id == *id));
        JobMarks::of(&self.editor).cancelled.lock().retain(|id| running.iter().any(|j| j.id == *id));
        crate::tasks::sweep(&self.editor, &running);
        if running.is_empty() {
            self.card_open = false;
        }
    }

    fn end_frame(&mut self, ctx: &Context) {
        self.popups_last = std::mem::take(&mut self.popups_cur);
        self.key_owner_last = self.key_owner_now.take();
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

    fn session_file(root: &Path) -> PathBuf {
        root.join("temp").join("session.ltx")
    }

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
                    label: t.label(),
                    arg,
                }
            })
            .collect();
        tabs::encode_session(&tabs)
    }

    fn write_session(&mut self) {
        self.session.dirty = false;
        let Some(root) = self.session.root.clone() else { return };
        let text = self.session_text();
        let log = self.editor.log.clone();
        self.editor.exec.spawn_io(move || {
            let path = Self::session_file(&root);
            if let Err(e) = gz_editor::fsx::write_atomic(&path, text.as_bytes()) {
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
        if let Some(root) = &self.session.root {
            let path = Self::session_file(root);
            if let Err(e) = gz_editor::fsx::write_atomic(&path, self.session_text().as_bytes()) {
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
        self.editor.exec.spawn_io(move || {
            let text = std::fs::read(Self::session_file(&root))
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

    /// The tabs in the strip and where they are; none on the Start page.
    pub(crate) fn tab_rects(&self) -> Vec<(String, Rect)> {
        if !self.menus_shown {
            return Vec::new();
        }
        self.tabs.iter().filter_map(|t| t.rect.map(|r| (t.id.to_string(), r))).collect()
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

    /// Every command that takes a shortcut: id, "group · label", keys.
    /// What `shortcuts` lists, as the C++ did: every sub-command of an editable C++ row, in the
    /// registry's order, then the bindable commands no row has.
    pub(crate) fn shortcut_rows(&self) -> Vec<(String, String, String)> {
        self.shortcut_faces().into_iter().filter_map(|i| self.shortcut_row(i)).collect()
    }

    fn shortcut_faces(&self) -> Vec<usize> {
        let mut rows: Vec<&crate::cpp::Row> = self.parts.cpp.rows.iter().collect();
        rows.sort_by_key(|r| r.cmd.id);
        let bindable = |f: &usize| self.parts.commands.get(*f).is_some_and(|c| c.bindable);
        let mut out: Vec<usize> =
            rows.iter().filter(|r| r.cmd.editable).flat_map(|r| r.faces.iter().flatten().copied()).collect();
        out.retain(bindable);
        let in_rows: Vec<usize> = rows.iter().flat_map(|r| r.faces.iter().flatten().copied()).collect();
        out.extend((0..self.parts.commands.len()).filter(|f| bindable(f) && !in_rows.contains(f)));
        out
    }

    /// The Keyboard page's name of a face of a C++ row (`CommandName`).
    fn cpp_name(&self, face: usize) -> Option<(String, String)> {
        self.parts.cpp.rows.iter().find_map(|r| {
            let k = r.faces.iter().position(|f| *f == Some(face))?;
            let sub = r.cmd.subs.get(k)?;
            let id = crate::cpp::sub_id(&r.cmd.name, &sub.desc);
            Some(crate::cpp::command_name(&id, &r.cmd.desc, &sub.desc))
        })
    }

    pub(crate) fn shortcut_row(&self, i: usize) -> Option<(String, String, String)> {
        let c = self.parts.commands.get(i).filter(|c| c.bindable)?;
        let (group, label) = self.cpp_name(i).unwrap_or_else(|| (c.group.to_string(), c.label.to_string()));
        Some((
            c.id.to_string(),
            format!("{group} \u{b7} {label}"),
            self.binding(i).map(|k| k.label()).unwrap_or_default(),
        ))
    }

    /// A command by its id or by the label the Keyboard page shows.
    pub(crate) fn shortcut_target(&self, name: &str) -> Option<usize> {
        self.shortcut_faces().into_iter().find(|&i| {
            self.shortcut_row(i).is_some_and(|(id, label, _)| {
                id.eq_ignore_ascii_case(name) || label.eq_ignore_ascii_case(name)
            })
        })
    }

    /// Binds a command (`None` takes its shortcut away); the command that had the chord lets it
    /// go. Saved at once. Answers that command's id.
    pub(crate) fn bind(&mut self, i: usize, chord: Option<Chord>) -> Option<String> {
        let mut moved = None;
        if let Some(chord) = chord {
            for j in 0..self.parts.commands.len() {
                if j != i
                    && self.binding(j) == Some(chord)
                    && let Some(c) = self.parts.commands.get(j)
                {
                    let id = c.id.clone();
                    self.bindings.insert(id.clone(), None);
                    self.settings.set_shortcut(&id, "");
                    moved = Some(id.to_string());
                }
            }
        }
        let Some(c) = self.parts.commands.get(i) else { return moved };
        let id = c.id.clone();
        self.bindings.insert(id.clone(), chord);
        self.settings.set_shortcut(&id, &chord.map(|k| k.label()).unwrap_or_default());
        self.settings.save_soon(&self.editor);
        moved
    }

    pub(crate) fn mark_session_dirty(&mut self) {
        self.session.dirty = true;
    }

    pub(crate) fn job_cancel(&mut self, id: JobId) -> bool {
        self.cancelled.insert(id);
        JobMarks::of(&self.editor).cancelled.lock().insert(id);
        self.editor.jobs.cancel(id)
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
    let last = editor.log.last_notable().map(|r| gz_log::marked(r.level, &r.text)).unwrap_or_default();
    LogState { last, errors, warnings }
}

/// The shell's own commands: in the palette and on the Keyboard page.
fn add_shell_commands(commands: &mut Vec<UiCommand>) {
    let mut add = |c: UiCommand| {
        if !commands.iter().any(|x| x.id == c.id) {
            commands.push(c);
        }
    };
    add(UiCommand::new("COMMAND_COMMAND_PALETTE", "Edit", "Command Palette", Icon::Search, |cx| {
        cx.requests.palette()
    })
    .keys("Ctrl+Shift+P"));
    add(UiCommand::new("COMMAND_EDITOR_PREF", "Edit", "Preferences", Icon::Settings, |cx| {
        cx.requests.open(OpenRequest::kind("preferences"))
    }));
    add(UiCommand::new("COMMAND_EXIT", "File", "Quit", Icon::Close, |cx| cx.requests.exit()));
    add(UiCommand::new("COMMAND_BREAK_LAST_OPERATION", "Edit", "Cancel Running Job", Icon::Stop, |cx| {
        for job in cx.editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running) {
            cx.editor.jobs.cancel(job.id);
        }
    })
    .enabled(|editor, _| editor.jobs.any_running()));
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
    // window_control resize goes down to 320 x 240 pt; the C++ editor's frame did not stop it either
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
    let base = gz_ui::theme::Palette::new(settings.accent()).base;
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
    /// The monitor's scale `-uiscale` was last applied over.
    native_seen: Option<f32>,
}

impl GrizzlyShell {
    #[must_use]
    pub fn new(editor: Arc<Editor>, builder: ShellBuilder, opts: ShellOptions) -> Self {
        let wake = editor.exec.wake().clone();
        let queue = editor.services.get_or_init(|| UiQueue::new(wake));
        Self { editor, queue, builder: Some(builder), opts, state: None, ctx: None, native_seen: None }
    }
}

impl Shell for GrizzlyShell {
    fn init(&mut self, ctx: &Context, gpu: &RenderState) {
        let settings = EditorSettings::of(&self.editor);
        gz_ui::font::install(ctx, std::mem::take(&mut self.opts.fonts));
        gz_ui::theme::install(ctx, settings.accent());
        layout::load(ctx, settings.layout());
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
            self.opts.iconsheet,
            guard,
        );
        state.logo = crate::chrome::load_logo(ctx);
        self.state = Some(state);
        self.ctx = Some(ctx.clone());
    }

    fn frame(&mut self, ui: &mut Ui) -> FrameReport {
        // -uiscale: the scale is the command line's, also after the monitor's scale changes
        if let Some(scale) = self.opts.ui_scale {
            let native = ui.ctx().native_pixels_per_point().unwrap_or(1.0).max(0.1);
            if self.native_seen != Some(native) {
                self.native_seen = Some(native);
                ui.ctx().set_zoom_factor(scale / native);
                ui.ctx().request_repaint();
            }
        }
        match self.state.as_mut() {
            Some(s) => s.frame(ui),
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
