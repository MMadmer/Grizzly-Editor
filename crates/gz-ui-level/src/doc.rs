//! The Level document: tab 1, never closed.
//!
//! It holds the view state of the Level tab - the dock, the panels' own state, the viewport
//! pane, the open chooser and dialogs - and draws from snapshots. Commands of the UI, MCP and the
//! shell's hooks reach it through [`post_to_level`]: an [`Act`] queued for its next frame.

use std::any::Any;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use egui::{CornerRadius, Id, Rect, Ui, Vec2, pos2};
use gz_app::chrome::MenuBar;
use gz_app::{Chord, DocCx, Document, EditAction, PanelInfo, PanelOp, UiQueue};
use gz_base::Rev;
use gz_editor::{Args, Editor, Value};
use gz_scene::SceneService;
use gz_ui::{Icon, MenuItem, Toolbar, audit, menu_item, palette};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::choose::{self, Chooser, Outcome, Purpose};
use crate::detail_objects::{self, DetailObjects};
use crate::details::{self, PropsPanel};
use crate::dock::{self, STRIP};
use crate::layout::{self, DockLayout, PanelId, SizeClass, Slot};
use crate::local::child_at;
use crate::menus;
use crate::outliner::Outliner;
use crate::status;
use crate::toolbar;
use crate::tools::ToolsPanel;

/// What the Level document is asked to do on its next frame.
#[derive(Debug)]
pub enum Act {
    /// Reset Layout; `true` also forgot every split and grid width.
    ResetLayout(bool),
    /// Scene > Find in World Outliner: the Outliner up, its search focused.
    FindInOutliner,
    /// Open, close or focus a panel without the shell (MCP readers that hold no `DocCx`).
    Panel(PanelId, PanelOp),
    /// Expand All (`true`) / Collapse All of Details (0) or Level Settings (1).
    Fold(usize, bool),
    /// A chooser on a property, its row read off the UI thread (`scene.choose`).
    Choose(Box<Chooser>),
    /// Closes the chooser that is open.
    CloseChooser,
    /// The viewport over the whole tab: on, off, or the other way (`None`).
    Maximize(Option<bool>),
}

/// What the Level tab shows that its actions ask about without the document at hand.
#[derive(Default)]
struct ViewFlags {
    maximized: AtomicBool,
}

/// Whether the viewport takes the whole Level tab (F11).
#[must_use]
pub fn maximized(editor: &Editor) -> bool {
    editor.services.get::<ViewFlags>().is_some_and(|f| f.maximized.load(Ordering::Relaxed))
}

/// Queues an act for the Level document's next frame.
pub fn post_to_level(editor: &Editor, act: Act) {
    if let Ok(q) = UiQueue::of(editor) {
        q.post(move |shell, _| {
            let _ = with_level(shell, |doc| doc.pending.push(act));
        });
    }
}

/// Runs `f` on the Level document, when it is this crate's.
pub fn with_level<R>(shell: &mut gz_app::ShellState, f: impl FnOnce(&mut LevelDoc) -> R) -> Option<R> {
    let doc = shell.document_mut(crate::LEVEL)?;
    let any = doc.as_any_mut()?;
    any.downcast_mut::<LevelDoc>().map(f)
}

/// The Level tab.
pub struct LevelDoc {
    editor: Arc<Editor>,
    pane: Option<gz_ui_viewport::ViewportPane>,
    layout: Option<DockLayout>,
    maximized: bool,
    pub(crate) pending: Vec<Act>,
    /// What each panel said of itself last frame.
    infos: FxHashMap<&'static str, PanelInfo>,
    focused: Option<PanelId>,
    viewport_focused: bool,
    pub outliner: Outliner,
    pub tools: ToolsPanel,
    pub details: PropsPanel,
    pub world: PropsPanel,
    chooser: Option<Chooser>,
    pub detail_objects: DetailObjects,
    /// The content edit the author chose to lose (Don't Save): not dirty until the next edit.
    discarded: Option<u64>,
    /// The scene's dirty state last frame: a save takes a new preview.
    was_dirty: bool,
    /// When the last preview was taken (or the scene came up), of which revision and scene.
    preview_at: Option<(Instant, Rev, String)>,
    project_rev: Rev,
    /// What stopped the save a command's caller asked for: that caller's answer.
    save_error: SaveAnswer,
}

/// Marks the close hook that takes `preview.png` as registered.
struct PreviewOnClose;

impl LevelDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
        // the Start page's picture of the project, taken while the scene is still up
        editor.services.get_or_init(|| {
            gz_project::ProjectHooks::of(editor)
                .on_before_close(|ed, _session| async move { write_preview(&ed).await });
            PreviewOnClose
        });
        Self {
            editor: editor.clone(),
            pane: None,
            layout: None,
            maximized: false,
            pending: Vec::new(),
            infos: FxHashMap::default(),
            focused: None,
            viewport_focused: false,
            outliner: Outliner::new(editor),
            tools: ToolsPanel::new(editor),
            details: PropsPanel::new(editor, details::SELECTION),
            world: PropsPanel::new(editor, details::WORLD),
            chooser: None,
            detail_objects: DetailObjects::default(),
            discarded: None,
            was_dirty: false,
            preview_at: None,
            project_rev: editor.project_rev(),
            save_error: Arc::default(),
        }
    }

    fn scene(&self) -> Option<Arc<SceneService>> {
        self.editor.project_opt().and_then(|s| s.services.get::<SceneService>())
    }

    fn class(ctx: &egui::Context) -> SizeClass {
        let r = ctx.content_rect();
        SizeClass::of(r.width(), r.height())
    }

    /// The layout, read from the store or built for the window's class on first use; a class
    /// crossed rebuilds it unless the author customised it.
    fn layout_mut(&mut self, ctx: &egui::Context) -> &mut DockLayout {
        let class = Self::class(ctx);
        let mut l = match self.layout.take() {
            Some(l) => l,
            None => DockLayout::load(ctx).unwrap_or_else(|| {
                layout::reset_sizes(ctx);
                DockLayout::built(class, None)
            }),
        };
        if l.class != class && !l.custom {
            let rebuilt = DockLayout::built(class, Some(&l));
            layout::reset_sizes(ctx);
            l = rebuilt;
        }
        l.custom = !l.placement_is_built() || layout::sizes_customised(ctx, l.class);
        l.save(ctx);
        self.layout.insert(l)
    }

    fn reset_layout(&mut self, ctx: &egui::Context) {
        let class = Self::class(ctx);
        let before = self.layout.take();
        layout::reset_sizes(ctx);
        // a layout built anew has every panel of its class
        let l = DockLayout::built(class, before.as_ref());
        l.save(ctx);
        self.layout = Some(l);
        self.maximized = false;
    }

    fn run_pending(&mut self, cx: &mut DocCx<'_>) {
        let acts = std::mem::take(&mut self.pending);
        self.run_acts(cx, acts);
        self.editor
            .services
            .get_or_init(ViewFlags::default)
            .maximized
            .store(self.maximized, Ordering::Relaxed);
    }

    fn run_acts(&mut self, cx: &mut DocCx<'_>, acts: Vec<Act>) {
        for act in acts {
            match act {
                Act::ResetLayout(_) => self.reset_layout(cx.ctx),
                Act::FindInOutliner => {
                    cx.requests.activate(crate::LEVEL);
                    self.open_panel(cx.ctx, PanelId::Outliner);
                    self.outliner.focus_search = true;
                }
                Act::Panel(p, op) => {
                    match op {
                        PanelOp::Close => self.layout_mut(cx.ctx).close_panel(p),
                        PanelOp::Toggle if self.layout_mut(cx.ctx).is_open(p) => {
                            self.layout_mut(cx.ctx).close_panel(p)
                        }
                        _ => self.open_panel(cx.ctx, p),
                    };
                }
                Act::Fold(t, open) => {
                    if t == details::WORLD {
                        self.world.fold_all(cx.ctx, open);
                    } else {
                        self.details.fold_all(cx.ctx, open);
                    }
                }
                Act::Choose(c) => {
                    cx.requests.activate(crate::LEVEL);
                    self.chooser = Some(*c);
                }
                Act::CloseChooser => self.chooser = None,
                Act::Maximize(on) => self.maximized = on.unwrap_or(!self.maximized),
            }
        }
    }

    /// Opens a panel, brings its tab up and takes the viewport out of F11.
    pub fn open_panel(&mut self, ctx: &egui::Context, p: PanelId) {
        self.layout_mut(ctx).open_panel(p);
        self.maximized = false;
        self.focused = Some(p);
    }

    pub fn close_panel(&mut self, ctx: &egui::Context, p: PanelId) {
        self.layout_mut(ctx).close_panel(p);
    }

    /// Expand All / Collapse All of Details (0) or Level Settings (1).
    pub fn fold(&mut self, target: usize, open: bool) {
        self.pending.push(Act::Fold(target, open));
    }

    /// Whether a panel is open.
    #[must_use]
    pub fn panel_open(&self, p: PanelId) -> bool {
        self.layout.as_ref().is_some_and(|l| l.is_open(p))
    }

    fn ensure_pane(&mut self, cx: &DocCx<'_>) {
        if self.pane.is_none() {
            self.pane = Some(gz_ui_viewport::ViewportPane::new(cx.editor, cx.gpu));
        }
    }

    /// A save takes a preview; so does a scene that changed a minute after the last one, so a
    /// close that clears the scene finds a picture of it.
    fn tick_preview(&mut self) {
        let Some(svc) = self.scene() else { return };
        let snap = svc.store.snapshot();
        if !snap.valid || snap.objects.is_empty() {
            return;
        }
        let dirty = svc.store.is_dirty();
        let saved = self.was_dirty && !dirty;
        self.was_dirty = dirty;
        // a scene just come up starts the minute: its preview is a pass of its own, and taken at
        // once it would build beside the viewport's first pass (a close takes one before it clears)
        let scene = snap.meta.name();
        let Some((t, rev, _)) = self.preview_at.clone().filter(|(_, _, s)| *s == scene) else {
            self.preview_at = Some((Instant::now(), snap.rev, scene));
            return;
        };
        let due = rev != snap.rev && t.elapsed() >= Duration::from_secs(60);
        if saved || due {
            self.preview_at = Some((Instant::now(), snap.rev, scene));
            store_preview(&self.editor);
        }
    }

    /// The Preferences' recovery rows reach the scene's autosave, and the author's input holds
    /// it off for the interaction delay.
    fn sync_recovery(&self, ctx: &egui::Context) {
        let Some(svc) = self.scene() else { return };
        let settings = gz_app::EditorSettings::of(&self.editor);
        let get = |key: &str, default: f64| settings.pref(key).unwrap_or(default);
        let prefs = gz_scene::recovery::RecoveryPrefs {
            enabled: get("recovery_enabled", 1.0) != 0.0,
            interval_minutes: get("recovery_interval_minutes", 10.0) as u32,
            interaction_delay_seconds: get("recovery_interaction_delay_seconds", 15.0) as u32,
            retention: get("recovery_retention", 10.0) as u32,
        };
        {
            let mut cur = svc.recovery.prefs.lock();
            if *cur != prefs {
                *cur = prefs;
            }
        }
        let touched = ctx.input(|i| {
            i.events.iter().any(|e| {
                matches!(
                    e,
                    egui::Event::Key { .. }
                        | egui::Event::PointerButton { .. }
                        | egui::Event::Text(_)
                        | egui::Event::MouseWheel { .. }
                )
            })
        });
        if touched {
            svc.recovery.note_input();
        }
    }

    fn sync_view(&mut self) {
        let Some(svc) = self.scene() else { return };
        let (d, w) = (self.panel_open(PanelId::Details), self.panel_open(PanelId::LevelSettings));
        let mut v = svc.view.lock();
        if let Some(o) = v.open.get_mut(0) {
            *o = d;
        }
        if let Some(o) = v.open.get_mut(1) {
            *o = w;
        }
    }

    /// A tool's chooser asked for over MCP: the one the tool's button opens.
    fn follow_tool_chooser(&mut self) {
        let Some(svc) = self.scene() else { return };
        let tool = svc.view.lock().tool_chooser.take();
        if tool.as_deref() == Some("group_pivot") && self.chooser.is_none() {
            self.chooser = crate::tools::pivot_chooser(&svc.store.snapshot());
        }
    }

    /// The chooser that is open, for `level.show` and `scene.choose`.
    #[must_use]
    pub fn chooser(&self) -> Option<&Chooser> {
        self.chooser.as_ref()
    }

    /// The chooser that is open, to move its search or its highlight (`scene.choose`).
    pub fn chooser_mut(&mut self) -> Option<&mut Chooser> {
        self.chooser.as_mut()
    }

    fn draw_chooser(&mut self, ui: &mut Ui, cx: &DocCx<'_>) {
        let Some(mut c) = self.chooser.take() else { return };
        let outcome = choose::draw(ui, &self.editor, Some((&cx.gpu.device, &cx.gpu.queue)), &mut c);
        let editor = self.editor.clone();
        let svc = self.scene();
        match outcome {
            Outcome::Open => self.chooser = Some(c),
            Outcome::Cancelled => {}
            Outcome::Picked(items) => {
                let first = items.first().cloned().unwrap_or_default();
                match &c.purpose {
                    Purpose::Property { target, key } => {
                        let mut args = Args::new();
                        args.insert("world".into(), Value::Bool(*target == details::WORLD));
                        // a multi-select row (a respawn point's profiles) takes the list
                        let value = if c.multi { items.join(",") } else { first };
                        let mut set = serde_json::Map::new();
                        set.insert(key.clone(), Value::String(value));
                        args.insert("set".into(), Value::Object(set));
                        editor.post("scene.set", args);
                    }
                    Purpose::DetailObjects => crate::tools_bulk::add_objects(&editor, &items),
                    Purpose::ToolCommand { cmd, action, arg } => {
                        let mut args = Args::new();
                        args.insert("action".into(), Value::String((*action).into()));
                        args.insert((*arg).into(), Value::String(first));
                        editor.post(cmd, args);
                    }
                    Purpose::ColorIndexObjects(i) => {
                        if let Some(svc) = &svc {
                            detail_objects::set_color_objects(&editor, svc, *i, &items);
                        }
                    }
                }
            }
        }
    }

    /// Draws the dock: the slots with their tab strips and panels, the viewport, the splitters.
    fn draw_dock(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let rect = ui.max_rect();
        let class = Self::class(&ctx);
        let p = palette(&ctx);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.chrome);
        self.infos.clear();
        if self.maximized {
            self.draw_viewport(ui, cx, rect, ppp);
            return;
        }
        let l = self.layout_mut(&ctx).clone();
        let g = dock::geometry(&ctx, rect, &l, class);
        // a press decides which panel has the keys
        let pressed_at = ctx.input(|i| if i.pointer.any_pressed() { i.pointer.interact_pos() } else { None });
        if let Some(at) = pressed_at {
            if g.viewport.contains(at) {
                self.focused = None;
                self.viewport_focused = true;
            } else if let Some(s) =
                Slot::ALL.iter().find(|s| g.slots[s.index()].is_some_and(|r| r.contains(at)))
            {
                self.focused = l.front_of(*s);
                self.viewport_focused = false;
            }
        }
        for s in Slot::ALL {
            let Some(r) = g.slots[s.index()] else { continue };
            self.draw_slot(ui, cx, &l, s, r, ppp);
        }
        self.draw_viewport(ui, cx, g.viewport, ppp);
        if dock::splitters(ui, &g) {
            ctx.request_repaint();
        }
    }

    fn draw_viewport(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>, rect: Rect, ppp: f32) {
        self.ensure_pane(cx);
        let mut child = child_at(ui, "gz-ui-level.viewport", rect);
        if let Some(pane) = self.pane.as_mut() {
            audit::surface(ui.ctx(), "Viewport###Render", || pane.ui(&mut child, cx));
        }
        self.infos.insert(
            "viewport",
            PanelInfo {
                open: true,
                visible: true,
                docked: true,
                floating: false,
                focused: self.viewport_focused,
                rect: Some(px(rect, ppp)),
                scroll_y: 0.0,
                scroll_max_y: 0.0,
            },
        );
    }

    fn draw_slot(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>, l: &DockLayout, s: Slot, rect: Rect, ppp: f32) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let panels = l.panels_in(s);
        let front = l.front_of(s);
        // the tab strip
        let strip = Rect::from_min_size(rect.min, Vec2::new(rect.width(), STRIP));
        ui.painter().rect_filled(strip, CornerRadius::ZERO, p.chrome);
        let mut x = strip.min.x + 2.0;
        let each_max = ((strip.width() - 4.0) / panels.len().max(1) as f32).max(48.0);
        let mut picked: Option<(PanelId, gz_ui::TabResponse)> = None;
        for panel in &panels {
            let label =
                if *panel == PanelId::Place { crate::tools::tab_label(&self.editor) } else { panel.label() };
            // a dock's tab may say it shorter than the Window menu does (context `dock`)
            let label = gz_ui::Tx::ctx("dock", label);
            let w = gz_ui::tabs::tab_width(&ctx, label.borrow(), true).min(each_max);
            let tab = Rect::from_min_size(pos2(x, strip.min.y + 2.0), Vec2::new(w, STRIP - 2.0));
            let resp = gz_ui::PanelTab::new(Id::unique(("gz-ui-level.tab", panel.id())), tab, label)
                .front(front == Some(*panel))
                .closable(true)
                .show(ui);
            if resp.clicked || resp.close || resp.menu {
                picked = Some((*panel, resp));
            }
            if resp.menu {
                egui::Popup::open_id(&ctx, tab_menu_id(*panel));
            }
            let layer = ui.layer_id();
            let mut moved = None;
            let mut close = false;
            let may_float = gz_app::floating_panels(&ctx);
            let mut float = false;
            gz_ui::menu::menu_popup(&ctx, tab_menu_id(*panel), tab, layer, |ui| {
                gz_app::chrome::local_submenu(ui, "Move To", Icon::None, true, |ui| {
                    for to in Slot::ALL {
                        if menu_item(
                            ui,
                            to.label(),
                            MenuItem { selected: l.slot_of(*panel) == Some(to), ..MenuItem::default() },
                        ) {
                            moved = Some(to);
                        }
                    }
                    if may_float {
                        ui.separator();
                        if menu_item(ui, "Own Window", MenuItem::default()) {
                            float = true;
                        }
                    }
                });
                ui.separator();
                if menu_item(ui, "Close", MenuItem { icon: Icon::Close, ..MenuItem::default() }) {
                    close = true;
                }
            });
            if let Some(to) = moved {
                self.layout_mut(&ctx).move_panel(*panel, to);
            }
            // dragged out: a press on the tab let go outside the editor's window
            if may_float {
                let (pressed_here, released_at, inner) = ctx.input(|i| {
                    let press = i.pointer.primary_pressed()
                        && i.pointer.interact_pos().is_some_and(|at| tab.contains(at));
                    let release = if i.pointer.primary_released() { i.pointer.latest_pos() } else { None };
                    (press, release, i.viewport().inner_rect)
                });
                let drag = Id::unique(("gz-ui-level.tab_drag", panel.id()));
                if pressed_here {
                    ctx.data_mut(|d| d.insert_temp(drag, true));
                }
                if let Some(at) = released_at
                    && ctx.data_mut(|d| d.remove_temp::<bool>(drag)).unwrap_or(false)
                    && !ctx.content_rect().contains(at)
                {
                    float = true;
                    let origin = inner.map_or(egui::Pos2::ZERO, |r| r.min);
                    self.layout_mut(&ctx).float_panel(*panel, Some([origin.x + at.x, origin.y + at.y]));
                }
            }
            if float && !self.layout.as_ref().is_some_and(|l| l.is_floating(*panel)) {
                self.layout_mut(&ctx).float_panel(*panel, None);
            }
            if close {
                self.layout_mut(&ctx).close_panel(*panel);
            }
            x += w + 2.0;
        }
        if let Some((panel, resp)) = picked {
            if resp.close {
                self.layout_mut(&ctx).close_panel(panel);
            } else if resp.clicked {
                self.layout_mut(&ctx).open_panel(panel);
                self.focused = Some(panel);
            }
        }
        // the body of the panel in front
        let body = Rect::from_min_max(pos2(rect.min.x, strip.max.y), rect.max);
        ui.painter().rect_filled(body, CornerRadius::ZERO, p.base);
        for panel in &panels {
            let visible = front == Some(*panel);
            let mut info = PanelInfo {
                open: true,
                visible,
                docked: true,
                floating: false,
                focused: self.focused == Some(*panel),
                rect: None,
                scroll_y: 0.0,
                scroll_max_y: 0.0,
            };
            if visible {
                info.rect = Some(px(rect, ppp));
                let inner = body.shrink(8.0);
                let mut child = child_at(ui, ("gz-ui-level.panel", panel.id()), inner);
                audit::surface(&ctx, panel.window(), || self.draw_panel(&mut child, cx, *panel));
            }
            self.infos.insert(panel.id(), info);
        }
    }

    /// The panels out in windows of their own: each drawn in its window, docked again when its
    /// close is pressed, its place kept.
    fn draw_floating(&mut self, cx: &mut DocCx<'_>) {
        let ctx = cx.ctx.clone();
        if !gz_app::floating_panels(&ctx) {
            return;
        }
        let l = self.layout_mut(&ctx).clone();
        for p in l.floating_panels() {
            let stood = l.float_rect.get(p.index()).copied().flatten();
            let window = p.window();
            let f = dock::floating_window(&ctx, p, stood, |ui| {
                audit::surface(&ctx, window, || self.draw_panel(ui, cx, p));
            });
            if f.closed {
                self.layout_mut(&ctx).dock_panel(p);
                continue;
            }
            // the place is kept rounded: a window moving by a fraction is not a new layout
            if let Some(r) = f.rect.map(|r| r.map(f32::round))
                && stood != Some(r)
            {
                self.layout_mut(&ctx).set_float_rect(p, r);
            }
            self.infos.insert(
                p.id(),
                PanelInfo {
                    open: true,
                    visible: true,
                    docked: false,
                    floating: true,
                    focused: self.focused == Some(p),
                    rect: None,
                    scroll_y: 0.0,
                    scroll_max_y: 0.0,
                },
            );
        }
    }

    fn draw_panel(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>, panel: PanelId) {
        match panel {
            PanelId::Place => self.tools.draw(ui, cx.editor, &mut self.chooser),
            PanelId::Outliner => self.outliner.draw(ui, cx.editor),
            PanelId::Details => self.details.draw(ui, cx.editor, &mut self.chooser),
            PanelId::LevelSettings => self.world.draw(ui, cx.editor, &mut self.chooser),
            PanelId::ContentBrowser => crate::content::content_browser(ui, cx),
            PanelId::OutputLog => crate::content::output_log(ui, cx),
            PanelId::Problems => crate::problems::draw(ui, cx.editor),
        }
        if std::mem::take(&mut self.tools.want_detail_objects) {
            self.detail_objects.open = true;
        }
    }

    /// Saves the scene into its file, off the UI thread; what stops the save is the answer of
    /// whoever waits for it ([`Document::save_failed`]), never a notice of the tab's own. A scene
    /// that has no file yet asks a person where (Save As) - cancelled there, the save ends as
    /// [`gz_app::SAVE_CANCELLED`]; `by_caller`, a command is told which command names the file.
    fn save_scene(&mut self, ctx: &egui::Context, by_caller: bool) -> Result<(), String> {
        let Some(svc) = self.scene() else { return Ok(()) };
        let snap = svc.store.snapshot();
        if !snap.valid {
            return Ok(());
        }
        *self.save_error.lock() = None;
        let origin = if by_caller { gz_base::Origin::Mcp } else { self.editor.ui_origin() };
        match snap.meta.file.clone() {
            Some(file) => {
                write_scene(&self.editor, &self.save_error, &file, false, origin);
                Ok(())
            }
            None if by_caller => {
                Err("the scene has no file yet: scene.save path=<file> names it and saves the scene"
                    .to_owned())
            }
            None => {
                let gave_up =
                    GaveUp { editor: self.editor.clone(), answer: self.save_error.clone(), armed: true };
                let failed = self.save_error.clone();
                crate::level_cmds::save_as_with(&self.editor, ctx, move |ed, file| {
                    gave_up.picked();
                    // the system's dialog asked before it gave a file that is there
                    write_scene(ed, &failed, &file, true, origin);
                })
            }
        }
    }
}

/// What stopped the scene's save, kept for whoever waits for it.
type SaveAnswer = Arc<Mutex<Option<gz_editor::CmdError>>>;

/// Writes the scene into `file` through `scene.save`; a failure is left in `failed`.
fn write_scene(
    editor: &Arc<Editor>,
    failed: &SaveAnswer,
    file: &std::path::Path,
    overwrite: bool,
    origin: gz_base::Origin,
) {
    let mut args = Args::new();
    args.insert("path".into(), Value::String(file.display().to_string()));
    if overwrite {
        args.insert("overwrite".into(), Value::Bool(true));
    }
    let (ed, failed) = (editor.clone(), failed.clone());
    let _task = editor.exec.spawn(async move {
        let reply = ed.call("scene.save", args, origin).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let why = reply.get("error").and_then(Value::as_str).unwrap_or("the save failed");
            let code = reply.get("code").and_then(Value::as_str).unwrap_or("failed");
            *failed.lock() = Some(gz_editor::CmdError::new(code, why));
            ed.exec.wake().ping();
            return;
        }
        // a file that does not read was left as it is and the scene still holds changes of its
        // objects: a person who saved is told, a command's caller has it in the answer
        let unwritten: Vec<&str> = reply
            .get("unwritten")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        if !unwritten.is_empty()
            && origin != gz_base::Origin::Mcp
            && let Ok(q) = UiQueue::of(&ed)
        {
            let text = gz_ui::tr!(
                "{files} does not read: the changes to its objects are not saved until it does",
                files = gz_ui::Tx::raw(unwritten.join(", "))
            );
            q.post(move |_, ctx| gz_ui::state::toast(ctx, gz_ui::Tone::Warning, text));
        }
    });
}

/// Rides in what runs when the Save dialog gives a file. The kit drops that unrun when the dialog
/// is cancelled: dropped while armed, this leaves the save's answer - cancelled - so that who
/// waits for the save ends there instead of waiting for a file nobody picks.
struct GaveUp {
    editor: Arc<Editor>,
    answer: SaveAnswer,
    armed: bool,
}

impl GaveUp {
    /// The dialog gave a file.
    fn picked(mut self) {
        self.armed = false;
    }
}

impl Drop for GaveUp {
    fn drop(&mut self) {
        if self.armed {
            *self.answer.lock() =
                Some(gz_editor::CmdError::new(gz_app::SAVE_CANCELLED, "the save was cancelled"));
            self.editor.exec.wake().ping();
        }
    }
}

fn tab_menu_id(p: PanelId) -> Id {
    Id::unique(("gz-ui-level.tab_menu", p.id()))
}

fn px(r: Rect, ppp: f32) -> [f32; 4] {
    [(r.min.x * ppp).round(), (r.min.y * ppp).round(), (r.width() * ppp).round(), (r.height() * ppp).round()]
}

/// The Start page's picture of the project: the viewport's own pass, no helpers, written as the project's
/// `preview.png`; a picture of nothing keeps the last good one.
pub fn store_preview(editor: &Arc<Editor>) {
    let ed = editor.clone();
    editor.exec.spawn(async move { write_preview(&ed).await });
}

/// Takes the picture and writes it; nothing when no scene is up.
async fn write_preview(ed: &Arc<Editor>) {
    let Some(root) = ed.project_root() else { return };
    let up = ed
        .project_opt()
        .and_then(|s| s.services.get::<gz_scene::SceneService>())
        .is_some_and(|s| s.store.snapshot().valid);
    if !up {
        return;
    }
    let Ok(img) = gz_ui_viewport::capture_preview(ed).await else { return };
    let (w, h) = (img.width as usize, img.height as usize);
    let rgb: Vec<u8> = img.rgba.as_chunks::<4>().0.iter().flat_map(|c| [c[0], c[1], c[2]]).collect();
    if w == 0 || h == 0 || gz_ui_app::cards::shows_no_scene(&rgb, w, h) {
        return;
    }
    let Ok(png) = gz_app::encode_png(img.width, img.height, &img.rgba) else { return };
    let file = root.join("preview.png");
    let log = ed.log.clone();
    ed.exec
        .io(move || {
            if let Err(e) = gz_editor::fsx::write_atomic(&file, &png) {
                log.warn(format!("the project's preview could not be written: {e}"));
            }
        })
        .await;
}

impl Document for LevelDoc {
    fn id(&self) -> &str {
        crate::LEVEL
    }

    fn kind(&self) -> &'static str {
        "level"
    }

    fn label(&self) -> gz_ui::Tx<'static> {
        let name = self.scene().map(|s| s.store.snapshot().meta.name()).unwrap_or_default();
        if name.is_empty() { gz_ui::Tx::from("Untitled") } else { gz_ui::Tx::raw(name) }
    }

    fn icon(&self) -> Icon {
        Icon::Level
    }

    /// The file the scene is on the disk as: its `.scene`, or the X-Ray SDK scene it was converted
    /// from while the `.scene` is not written yet - what Show in Content Browser and Explorer find.
    fn path(&self) -> Option<String> {
        self.scene().and_then(|s| {
            let snap = s.store.snapshot();
            snap.meta.imported.as_ref().or(snap.meta.file.as_ref()).map(|f| f.display().to_string())
        })
    }

    fn tooltip(&self) -> Option<gz_ui::Tx<'static>> {
        Some(self.path().map_or_else(|| gz_ui::Tx::from("The scene has not been saved yet"), gz_ui::Tx::raw))
    }

    fn closable(&self) -> bool {
        false
    }

    fn dirty(&self) -> bool {
        self.scene().is_some_and(|s| s.store.is_dirty() && self.discarded != Some(s.store.snapshot().edit))
    }

    fn dirty_elsewhere(&self) -> bool {
        true
    }

    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        toolbar::draw(tb, cx);
    }

    fn body(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        if self.project_rev != cx.editor.project_rev() {
            self.project_rev = cx.editor.project_rev();
            self.chooser = None;
            self.detail_objects = DetailObjects::default();
            self.discarded = None;
            self.preview_at = None;
        }
        self.run_pending(cx);
        self.follow_tool_chooser();
        self.draw_dock(ui, cx);
        self.draw_floating(cx);
        if let Some(p) = self.focused.filter(|p| p.owns_keys()) {
            cx.requests.own_keys(p.id());
        }
        self.draw_chooser(ui, cx);
        self.detail_objects.draw(ui, cx.editor, &mut self.chooser);
        crate::motion_ui::draw(ui, cx.editor);
        self.sync_view();
        self.tick_preview();
        self.sync_recovery(ui.ctx());
    }

    fn status(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        status::centre(ui, cx);
    }

    fn menus(&mut self, bar: &mut MenuBar<'_, '_>, cx: &mut DocCx<'_>) {
        menus::draw(bar, cx);
    }

    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        // the browser and the log answer their own keys
        if self.focused.is_some_and(PanelId::owns_keys) {
            return false;
        }
        match self.pane.as_mut() {
            Some(pane) => pane.shortcut(cx, chord),
            None => false,
        }
    }

    fn edit(&mut self, cx: &mut DocCx<'_>, action: EditAction, run: bool) -> bool {
        let Some(svc) = self.scene().filter(|s| s.store.snapshot().valid) else {
            return false;
        };
        if !run {
            return true;
        }
        match action {
            EditAction::Undo | EditAction::Redo => {
                let undo = action == EditAction::Undo;
                cx.editor.exec.spawn(async move {
                    let _ = if undo { svc.store.undo().await } else { svc.store.redo().await };
                });
            }
            EditAction::Delete => crate::level_cmds::delete(cx.editor, &svc),
            EditAction::Find => self.pending.push(Act::FindInOutliner),
            // the clipboard of scene objects, whatever kinds the selection holds
            EditAction::Cut => crate::level_cmds::edit(cx.editor, "cut"),
            EditAction::Copy => crate::level_cmds::edit(cx.editor, "copy"),
            EditAction::Paste => crate::level_cmds::edit(cx.editor, "paste"),
            EditAction::Duplicate => crate::level_cmds::edit(cx.editor, "duplicate"),
        }
        true
    }

    fn save(&mut self, cx: &mut DocCx<'_>) -> Result<(), String> {
        self.save_scene(cx.ctx, cx.requests.by_caller())
    }

    fn save_failed(&mut self) -> Option<gz_editor::CmdError> {
        self.save_error.lock().take()
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.discarded = self.scene().map(|s| s.store.snapshot().edit);
    }

    fn on_activate(&mut self, cx: &mut DocCx<'_>) {
        cx.ctx.request_repaint();
    }

    fn inactive(&mut self, cx: &mut DocCx<'_>) {
        self.sync_recovery(cx.ctx);
        self.run_pending(cx);
        self.ensure_pane(cx);
        if let Some(pane) = self.pane.as_mut() {
            pane.inactive(cx);
        }
        self.infos.clear();
        self.draw_floating(cx);
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::json!({
            "layout_custom": self.layout.as_ref().is_some_and(|l| l.custom),
            "chooser": self.chooser.as_ref().map(Chooser::state),

            "viewport_maximized": self.maximized,
            "viewport": self.pane.as_ref().map_or(serde_json::Value::Null, gz_ui_viewport::ViewportPane::mcp_state),
        })
    }

    fn panel_state(&self, id: &str) -> Option<PanelInfo> {
        if id == "viewport" {
            return Some(self.infos.get("viewport").cloned().unwrap_or(PanelInfo {
                open: true,
                docked: true,
                ..PanelInfo::default()
            }));
        }
        let p = PanelId::parse(id)?;
        let open = self.panel_open(p);
        let mut info = self.infos.get(p.id()).cloned().unwrap_or_default();
        info.open = open;
        let floating = self.layout.as_ref().is_some_and(|l| l.is_floating(p));
        info.docked = !floating && self.layout.as_ref().is_some_and(|l| l.slot_of(p).is_some());
        info.floating = floating && open;
        if !open {
            info.visible = false;
            info.rect = None;
        }
        Some(info)
    }

    fn panel(&mut self, cx: &mut DocCx<'_>, id: &str, op: PanelOp) -> bool {
        if id == "viewport" {
            if op == PanelOp::Close {
                return false;
            }
            self.maximized = false;
            self.viewport_focused = true;
            return true;
        }
        let Some(p) = PanelId::parse(id) else { return false };
        match op {
            PanelOp::Close => self.layout_mut(cx.ctx).close_panel(p),
            PanelOp::Toggle if self.panel_open(p) => self.layout_mut(cx.ctx).close_panel(p),
            PanelOp::Float => {
                if !gz_app::floating_panels(cx.ctx) {
                    return false;
                }
                self.layout_mut(cx.ctx).float_panel(p, None);
                cx.ctx.request_repaint();
            }
            PanelOp::Dock => {
                self.layout_mut(cx.ctx).dock_panel(p);
                self.maximized = false;
            }
            _ => self.open_panel(cx.ctx, p),
        }
        true
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
