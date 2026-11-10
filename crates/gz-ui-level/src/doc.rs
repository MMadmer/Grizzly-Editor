//! The Level document: tab 1, never closed (`UIMainForm::RegisterLevel` / `DrawLevel`).
//!
//! It holds the view state of the Level tab - the dock, the panels' own state, the viewport
//! pane, the open chooser and dialogs - and draws from snapshots. Commands of the UI, MCP and the
//! shell's hooks reach it through [`post_to_level`]: an [`Act`] queued for its next frame.

use std::any::Any;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{CornerRadius, Id, Rect, Ui, Vec2, pos2};
use gz_app::chrome::MenuBar;
use gz_app::{Chord, DocCx, Document, EditAction, PanelInfo, PanelOp, UiQueue};
use gz_base::Rev;
use gz_editor::{Args, Editor, Value};
use gz_scene::{ObjClass, SceneService};
use gz_ui::{Icon, MenuItem, Toolbar, audit, menu_item, palette};
use rustc_hash::FxHashMap;

use crate::choose::{self, Chooser, Outcome, Purpose};
use crate::detail_objects::{self, DetailObjects};
use crate::details::{self, PropsPanel};
use crate::dock::{self, STRIP};
use crate::layout::{self, DockLayout, PanelId, SizeClass, Slot};
use crate::local::{TabResponse, child_at, local_tab, local_tab_width};
use crate::menus;
use crate::outliner::Outliner;
use crate::status;
use crate::toolbar::{self, ToolbarState};
use crate::tools::ToolsPanel;

/// What the Level document is asked to do on its next frame.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    /// Reset Layout; `true` also forgot every split and grid width.
    ResetLayout(bool),
    /// Scene > Find in World Outliner: the Outliner up, its search focused.
    FindInOutliner,
    /// Open, close or focus a panel without the shell (MCP readers that hold no `DocCx`).
    Panel(PanelId, PanelOp),
    /// Expand All (`true`) / Collapse All of Details (0) or Level Settings (1).
    Fold(usize, bool),
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
    tools: ToolsPanel,
    pub details: PropsPanel,
    pub world: PropsPanel,
    chooser: Option<Chooser>,
    detail_objects: DetailObjects,
    toolbar: ToolbarState,
    /// The content edit the author chose to lose (Don't Save): not dirty until the next edit.
    discarded: Option<u64>,
    /// The scene's dirty state last frame: a save takes a new preview.
    was_dirty: bool,
    preview_at: Option<(Instant, Rev)>,
    project_rev: Rev,
}

impl LevelDoc {
    #[must_use]
    pub fn new(editor: &Arc<Editor>) -> Self {
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
            tools: ToolsPanel::new(),
            details: PropsPanel::new(editor, details::SELECTION),
            world: PropsPanel::new(editor, details::WORLD),
            chooser: None,
            detail_objects: DetailObjects::default(),
            toolbar: ToolbarState::default(),
            discarded: None,
            was_dirty: false,
            preview_at: None,
            project_rev: editor.project_rev(),
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
        for act in std::mem::take(&mut self.pending) {
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
                        PanelOp::Toggle if self.layout_mut(cx.ctx).is_open(p) => self.layout_mut(cx.ctx).close_panel(p),
                        _ => self.open_panel(cx.ctx, p),
                    };
                }
                Act::Fold(t, open) => {
                    if t == details::WORLD {
                        self.world.fold_all(open);
                    } else {
                        self.details.fold_all(open);
                    }
                }
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
        let due = self.preview_at.is_none_or(|(t, rev)| rev != snap.rev && t.elapsed() >= Duration::from_secs(60));
        if saved || due {
            self.preview_at = Some((Instant::now(), snap.rev));
            store_preview(&self.editor);
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

    /// The chooser `property_choose` opened over MCP, or that closed under one.
    fn follow_mcp_chooser(&mut self) {
        let Some(svc) = self.scene() else { return };
        let choosing = svc.view.lock().choosing.clone();
        match (&choosing, &self.chooser) {
            (Some(key), None) => {
                let snap = svc.store.snapshot();
                self.chooser = details::chooser_for(&snap, &svc.library, key);
                if self.chooser.is_none() {
                    svc.view.lock().choosing = None;
                }
            }
            (None, Some(c)) if matches!(c.purpose, Purpose::Property { target: 0, .. }) => self.chooser = None,
            _ => {}
        }
    }

    fn draw_chooser(&mut self, ui: &mut Ui) {
        let Some(mut c) = self.chooser.take() else { return };
        let outcome = choose::draw(ui, &self.editor, &mut c);
        let editor = self.editor.clone();
        let svc = self.scene();
        match outcome {
            Outcome::Open => self.chooser = Some(c),
            Outcome::Cancelled => {
                if let (Purpose::Property { target: 0, .. }, Some(svc)) = (&c.purpose, &svc) {
                    svc.view.lock().choosing = None;
                }
            }
            Outcome::Picked(items) => {
                let first = items.first().cloned().unwrap_or_default();
                match &c.purpose {
                    Purpose::Property { target, key } => {
                        if *target == details::SELECTION && let Some(svc) = &svc {
                            let mut v = svc.view.lock();
                            v.choosing = None;
                            v.chosen.clone_from(&first);
                        }
                        let mut args = Args::new();
                        args.insert("target".into(), Value::String(if *target == details::WORLD { "world" } else { "selection" }.into()));
                        args.insert("key".into(), Value::String(key.clone()));
                        args.insert("value".into(), Value::String(first));
                        editor.post("property_set", args);
                    }
                    Purpose::ToolObject if c.multi => crate::tools::add_several(&editor, &items),
                    Purpose::ToolObject => {
                        crate::tools::set_choice(&editor, ObjClass::SceneObject, &first);
                    }
                    Purpose::ToolGroup => crate::tools::set_choice(&editor, ObjClass::Group, &first),
                    Purpose::DetailObjects => {
                        if let Some(svc) = &svc {
                            detail_objects::add_objects(ui, &editor, svc, &items);
                        }
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
            } else if let Some(s) = Slot::ALL.iter().find(|s| g.slots[s.index()].is_some_and(|r| r.contains(at))) {
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
            PanelInfo { open: true, visible: true, docked: true, focused: self.viewport_focused, rect: Some(px(rect, ppp)), scroll_y: 0.0, scroll_max_y: 0.0 },
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
        let mut picked: Option<(PanelId, TabResponse)> = None;
        for panel in &panels {
            let w = local_tab_width(&ctx, panel.label()).min(each_max);
            let tab = Rect::from_min_size(pos2(x, strip.min.y + 2.0), Vec2::new(w, STRIP - 2.0));
            let resp = local_tab(ui, Id::unique(("gz-ui-level.tab", panel.id())), tab, panel.label(), front == Some(*panel), true);
            if resp.clicked || resp.close || resp.menu {
                picked = Some((*panel, resp));
            }
            if resp.menu {
                egui::Popup::open_id(&ctx, tab_menu_id(*panel));
            }
            let layer = ui.layer_id();
            let mut moved = None;
            let mut close = false;
            gz_ui::menu::menu_popup(&ctx, tab_menu_id(*panel), tab, layer, |ui| {
                gz_app::chrome::local_submenu(ui, "Move To", Icon::None, true, |ui| {
                    for to in Slot::ALL {
                        if menu_item(ui, to.label(), MenuItem { selected: l.slot_of(*panel) == Some(to), ..MenuItem::default() }) {
                            moved = Some(to);
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
            let mut info = PanelInfo { open: true, visible, docked: true, focused: self.focused == Some(*panel), rect: None, scroll_y: 0.0, scroll_max_y: 0.0 };
            if visible {
                info.rect = Some(px(rect, ppp));
                let inner = body.shrink(8.0);
                let mut child = child_at(ui, ("gz-ui-level.panel", panel.id()), inner);
                audit::surface(&ctx, panel.window(), || self.draw_panel(&mut child, cx, *panel));
            }
            self.infos.insert(panel.id(), info);
        }
    }

    fn draw_panel(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>, panel: PanelId) {
        match panel {
            PanelId::Tools => self.tools.draw(ui, cx.editor, &mut self.chooser),
            PanelId::Outliner => self.outliner.draw(ui, cx.editor),
            PanelId::Details => self.details.draw(ui, cx.editor, &mut self.chooser),
            PanelId::LevelSettings => self.world.draw(ui, cx.editor, &mut self.chooser),
            PanelId::ContentBrowser => crate::content::content_browser(ui, cx),
            PanelId::OutputLog => crate::content::output_log(ui, cx),
        }
        if std::mem::take(&mut self.tools.want_detail_objects) {
            self.detail_objects.open = true;
        }
    }

    fn save_scene(&mut self) {
        let Some(svc) = self.scene() else { return };
        let snap = svc.store.snapshot();
        if !snap.valid {
            return;
        }
        match snap.meta.file.clone() {
            Some(file) => {
                let mut args = Args::new();
                args.insert("file".into(), Value::String(file.display().to_string()));
                self.editor.post("save_scene", args);
            }
            None => save_as(&self.editor, &svc),
        }
    }
}

fn tab_menu_id(p: PanelId) -> Id {
    Id::unique(("gz-ui-level.tab_menu", p.id()))
}

fn px(r: Rect, ppp: f32) -> [f32; 4] {
    [(r.min.x * ppp).round(), (r.min.y * ppp).round(), (r.width() * ppp).round(), (r.height() * ppp).round()]
}

/// Save As: the system's save dialog off the UI thread, then `save_scene` with the file.
fn save_as(editor: &Arc<Editor>, svc: &SceneService) {
    let dir = svc.maps_dir();
    let ed = editor.clone();
    let spawned = std::thread::Builder::new().name("gz-ui-level.save_as".into()).spawn(move || {
        let _ = std::fs::create_dir_all(&dir);
        let picked = rfd::FileDialog::new().set_title("Save Scene As").add_filter("Level", &["level"]).set_directory(&dir).set_file_name("level.level").save_file();
        if let Some(file) = picked {
            let mut args = Args::new();
            args.insert("file".into(), Value::String(file.display().to_string()));
            ed.post("save_scene", args);
        }
    });
    if let Err(e) = spawned {
        editor.log.warn(format!("the save dialog did not open: {e}"));
    }
}

/// `CapturePreview`: the viewport's own pass, no helpers, written as the project's
/// `preview.png`; a picture of nothing keeps the last good one.
pub fn store_preview(editor: &Arc<Editor>) {
    let Some(root) = editor.project_root() else { return };
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let Ok(img) = gz_ui_viewport::capture_preview(&ed).await else { return };
        let (w, h) = (img.width as usize, img.height as usize);
        let rgb: Vec<u8> = img.rgba.chunks_exact(4).flat_map(|c| [c[0], c[1], c[2]]).collect();
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
    });
}

impl Document for LevelDoc {
    fn id(&self) -> &str {
        crate::LEVEL
    }

    fn kind(&self) -> &'static str {
        "level"
    }

    fn label(&self) -> String {
        let name = self.scene().map(|s| s.store.snapshot().meta.name()).unwrap_or_default();
        if name.is_empty() { "Untitled".to_owned() } else { name }
    }

    fn icon(&self) -> Icon {
        Icon::Level
    }

    fn path(&self) -> Option<String> {
        self.scene().and_then(|s| s.store.snapshot().meta.file.as_ref().map(|f| f.display().to_string()))
    }

    fn tooltip(&self) -> Option<String> {
        Some(self.path().unwrap_or_else(|| "The scene has not been saved yet".to_owned()))
    }

    fn closable(&self) -> bool {
        false
    }

    fn dirty(&self) -> bool {
        self.scene().is_some_and(|s| s.store.is_dirty() && self.discarded != Some(s.store.snapshot().edit))
    }

    fn padded(&self) -> bool {
        false
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let mut save = false;
        toolbar::draw(tb, cx.editor, &mut self.toolbar, &mut save);
        if save {
            self.save_scene();
        }
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
        self.follow_mcp_chooser();
        self.draw_dock(ui, cx);
        self.draw_chooser(ui);
        self.detail_objects.draw(ui, cx.editor, &mut self.chooser);
        self.sync_view();
        self.tick_preview();
    }

    fn status(&mut self, ui: &mut Ui, cx: &mut DocCx<'_>) {
        status::centre(ui, cx);
    }

    fn menus(&mut self, bar: &mut MenuBar<'_, '_>, cx: &mut DocCx<'_>) {
        let picks = menus::draw(bar, cx, self.maximized);
        if picks.maximize {
            self.maximized = !self.maximized;
        }
    }

    fn shortcut(&mut self, cx: &mut DocCx<'_>, chord: &Chord) -> bool {
        if chord.key == egui::Key::F11 && !chord.ctrl && !chord.shift && !chord.alt {
            self.maximized = !self.maximized;
            return true;
        }
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
        let id = match action {
            EditAction::Undo => "COMMAND_UNDO",
            EditAction::Redo => "COMMAND_REDO",
            EditAction::Delete => "COMMAND_DELETE_SELECTION",
            EditAction::Find => "COMMAND_SHOW_OBJECTLIST",
            EditAction::Cut | EditAction::Copy | EditAction::Paste | EditAction::Duplicate => return false,
        };
        if run {
            menus::run_scene_command(id, cx.editor, cx.ctx);
        }
        true
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        self.save_scene();
        Ok(())
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        self.discarded = self.scene().map(|s| s.store.snapshot().edit);
    }

    fn on_activate(&mut self, cx: &mut DocCx<'_>) {
        cx.ctx.request_repaint();
    }

    fn inactive(&mut self, cx: &mut DocCx<'_>) {
        self.run_pending(cx);
        self.ensure_pane(cx);
        if let Some(pane) = self.pane.as_mut() {
            pane.inactive(cx);
        }
        self.infos.clear();
    }

    fn mcp_state(&self) -> serde_json::Value {
        serde_json::json!({
            "layout_custom": self.layout.as_ref().is_some_and(|l| l.custom),
            "viewport_maximized": self.maximized,
            "viewport": self.pane.as_ref().map_or(serde_json::Value::Null, gz_ui_viewport::ViewportPane::mcp_state),
        })
    }

    fn panel_state(&self, id: &str) -> Option<PanelInfo> {
        if id == "viewport" {
            return Some(self.infos.get("viewport").cloned().unwrap_or(PanelInfo { open: true, docked: true, ..PanelInfo::default() }));
        }
        let p = PanelId::parse(id)?;
        let open = self.panel_open(p);
        let mut info = self.infos.get(p.id()).cloned().unwrap_or_default();
        info.open = open;
        info.docked = self.layout.as_ref().is_some_and(|l| l.slot_of(p).is_some());
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
            _ => self.open_panel(cx.ctx, p),
        }
        true
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn Any> {
        Some(self)
    }
}
