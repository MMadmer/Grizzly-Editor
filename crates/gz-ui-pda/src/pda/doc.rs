//! The `pda` tab (docs/UI_DESIGN.md, section 6, "PDA Interface"): a singleton that switches to
//! the next layout it is given. Toolbar: Save, Revert, Reload | Aspect, Language, Zoom |
//! Reset State | Source, Details. The canvas takes every resize; Details (320 pt) and Source
//! (a bottom split) keep their size. The tab follows the file unless the Source pane holds
//! edits; those are never lost silently.

use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use egui::{Align, Layout, Rect, ScrollArea, Sense, Ui, UiBuilder, Vec2, pos2};
use gz_app::{DocCx, Document, OpenRequest, ShellBuilder, UiCommand};
use gz_base::{Origin, RelPath, Rev};
use gz_editor::{Args, Editor, Value};
use gz_mod::ManifestService;
use gz_pda::{Aspect, PdaService, base_name, is_pda_path, project_relative, same_path};
use gz_project::Project;
use gz_ui::grid::read_only;
use gz_ui::list::list;
use gz_ui::state::{badge, busy_line, empty_state};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Answer, Asker, Button, ConfirmSpec, Font, Icon, Kind, PropFlags, PropertyGrid, Tone, Toolbar,
    ToolbarStyle, metrics, palette,
};

use crate::assets::PdaAssets;
use crate::local::{local_code_view, local_section, local_tab_strip};
use crate::pda::canvas::{CanvasCache, draw_canvas};
use crate::pda::{CANVAS_MIN_H, CANVAS_MIN_W, DETAILS_W, Model, PdaTab, SIDE_MIN, SOURCE_H, SOURCE_MIN};

/// What a new view of the tab is to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Target {
    /// A layout, project-relative with `/`.
    Path(String),
    /// The layout Mod > PDA Interface opens, when the preview holds none yet.
    Module,
}

/// The tab: a view of the project's [`PdaTab`].
pub struct PdaDoc {
    editor: Arc<Editor>,
    tab: Arc<PdaTab>,
    assets: Arc<PdaAssets>,
    serial: u64,
    target: Option<Target>,
    started: bool,
    /// The file tree's revision the file was last compared at.
    tree_rev: Option<Rev>,
    following: Arc<AtomicBool>,
    /// Waiting for the layout a target names.
    opening: Arc<AtomicBool>,
    canvas: CanvasCache,
    /// `[pda] layout` as the manifest last said it, and the manifest revision.
    used: Option<(Rev, String)>,
}

fn asker(editor: &Editor) -> Asker {
    if editor.flags.unattended { Asker::Unattended } else { Asker::Ui }
}

fn log_line(editor: &Editor, line: Option<String>) {
    if let Some(l) = line {
        editor.log.info(l);
    }
}

fn set_toast(m: &mut Model, text: String) {
    m.preview.toast.clone_from(&text);
    m.preview.toast_serial += 1;
    m.preview.push_log(text);
}

impl PdaDoc {
    fn new(
        editor: Arc<Editor>,
        tab: Arc<PdaTab>,
        assets: Arc<PdaAssets>,
        serial: u64,
        target: Target,
    ) -> Self {
        Self {
            editor,
            tab,
            assets,
            serial,
            target: Some(target),
            started: false,
            tree_rev: None,
            following: Arc::new(AtomicBool::new(false)),
            opening: Arc::new(AtomicBool::new(false)),
            canvas: CanvasCache::default(),
            used: None,
        }
    }

    fn spawn(
        &self,
        f: impl FnOnce(Arc<Editor>, Arc<PdaTab>) -> futures::future::BoxFuture<'static, ()> + Send + 'static,
    ) {
        let (ed, tab) = (self.editor.clone(), self.tab.clone());
        let _task = self.editor.exec.spawn(f(ed, tab));
    }

    /// The view takes the tab over: its target is read now that the shell accepted it.
    fn start(&mut self) {
        if self.started {
            return;
        }
        self.started = true;
        let target = self.target.take();
        let (have, current) = {
            let mut m = self.tab.m.lock();
            m.open = true;
            (!m.path.is_empty(), m.rel.clone())
        };
        let root = self.tab.root.clone();
        let opening = self.opening.clone();
        match target {
            Some(Target::Path(rel)) if rel != current => {
                let Ok(abs) = gz_project::ops::resolve_in_project(&root, &rel) else { return };
                opening.store(true, Ordering::SeqCst);
                self.spawn(move |ed, tab| {
                    Box::pin(async move {
                        if !super::load_file(&ed, &tab, &abs.replace('/', "\\")).await {
                            ed.notices.error(format!("{rel} cannot be read"));
                        }
                        opening.store(false, Ordering::SeqCst);
                    })
                });
            }
            Some(Target::Module) if !have => {
                opening.store(true, Ordering::SeqCst);
                self.spawn(move |ed, tab| {
                    Box::pin(async move {
                        if let Some(rel) = super::module_layout(&ed).await
                            && let Ok(abs) = gz_project::ops::resolve_in_project(&root, &rel)
                        {
                            super::load_file(&ed, &tab, &abs.replace('/', "\\")).await;
                        }
                        opening.store(false, Ordering::SeqCst);
                    })
                });
            }
            _ => {}
        }
    }

    /// The file follows the disk: the tree moved, so it is compared again (off the UI thread).
    fn follow_tick(&mut self) {
        let Ok((_, project)) = Project::of(&self.editor) else { return };
        let rev = project.tree.rev();
        if self.tree_rev == Some(rev) || self.following.swap(true, Ordering::SeqCst) {
            return;
        }
        self.tree_rev = Some(rev);
        let flag = self.following.clone();
        self.spawn(move |ed, tab| {
            Box::pin(async move {
                super::follow(&ed, &tab).await;
                flag.store(false, Ordering::SeqCst);
            })
        });
    }

    /// `[pda] layout` of the manifest, read again when the manifest moved.
    fn used_layout(&mut self) -> String {
        let Some(session) = self.editor.project_opt() else { return String::new() };
        let Some(manifest) = ManifestService::get(&session) else { return String::new() };
        let rev = manifest.rev();
        if self.used.as_ref().is_none_or(|(r, _)| *r != rev) {
            let used = manifest.snapshot().manifest().map(|m| m.pda_layout.clone()).unwrap_or_default();
            self.used = Some((rev, used));
        }
        self.used.as_ref().map(|(_, u)| u.clone()).unwrap_or_default()
    }

    fn save_async(&self) {
        let ed = self.editor.clone();
        self.spawn(move |e, tab| {
            Box::pin(async move {
                if let Err(err) = super::save_and_read(&e, &tab).await {
                    post_notice(&ed, "Save failed", err);
                }
            })
        });
    }

    fn read_async(&self) {
        self.spawn(|ed, tab| Box::pin(async move { super::read(&ed, &tab, true).await }));
    }

    fn reload_all_async(&self) {
        self.spawn(|ed, tab| Box::pin(async move { super::reload_all(&ed, &tab).await }));
    }

    /// Revert: the Source pane's edits go only after a question that names the file.
    fn revert(&self, ctx: &egui::Context) {
        let (dirty, path, name) = {
            let m = self.tab.m.lock();
            (m.text_dirty, m.path.clone(), m.shown_name())
        };
        if !dirty {
            self.spawn(move |ed, tab| {
                Box::pin(async move {
                    super::load_file(&ed, &tab, &path).await;
                })
            });
            return;
        }
        let spec = ConfirmSpec {
            id: "pda_revert".to_owned(),
            title: "Revert PDA Interface".to_owned(),
            headline: format!("Drop the unsaved edits of {name}?"),
            items: vec!["The Source pane's edits since the last save".to_owned()],
            total: 0,
            consequence: "The file stays as it is on disk; the edits are lost.".to_owned(),
            action: "Revert".to_owned(),
            destructive: true,
        };
        let (ed, tab) = (self.editor.clone(), self.tab.clone());
        let line = gz_ui::dialog::confirm(ctx, asker(&self.editor), spec, move |answer| {
            if answer != Answer::Yes {
                return;
            }
            let _task = ed.exec.clone().spawn(async move {
                let same = tab.m.lock().path == path;
                if same {
                    super::load_file(&ed, &tab, &path).await;
                }
            });
        });
        log_line(&self.editor, line);
    }

    fn draw_details(&mut self, ui: &mut Ui, m: &mut Model) {
        let used = self.used_layout();
        let p = palette(ui.ctx());
        // what the tab shows and whether the module uses it
        let name = m.shown_name();
        let is_used = !m.rel.is_empty() && same_path(&used, &m.rel);
        let right = if is_used {
            text::width(ui.ctx(), &Font::Small.id(), "Used by the module") + 12.0
        } else {
            gz_ui::units::button_width(ui.ctx(), "Use in Module", Kind::Secondary, false, 0.0)
        };
        let mut use_it = false;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            let (r, _) = ui.allocate_exact_size(Vec2::new(metrics::GLYPH, metrics::FRAME), Sense::hover());
            gz_ui::icons::draw(
                ui.painter(),
                Icon::PdaInterface,
                Rect::from_center_size(r.center(), Vec2::splat(metrics::GLYPH)),
                p.text_secondary,
            );
            let room = (ui.available_width() - right - 8.0).max(0.0);
            ui.scope(|ui| {
                ui.set_max_width(room);
                text::label(ui, &name, Font::Body, p.text, Clip::Start);
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if is_used {
                    ui.scope(|ui| badge(ui, "Used by the module", Tone::Success))
                        .response
                        .on_hover_text("mod.ltx [pda] layout names this file: the game shows it in the PDA.");
                } else {
                    let tip = format!(
                        "Write [pda] layout = {} into mod.ltx: the game shows this interface in the PDA.",
                        m.rel
                    );
                    use_it = Button::new("Use in Module").tip(&tip).show(ui).clicked();
                }
            });
        });
        if use_it {
            let rel = m.rel.clone();
            let ed = self.editor.clone();
            self.spawn(move |e, tab| {
                Box::pin(async move {
                    let Ok(service) = PdaService::of(&e) else { return };
                    match service.use_layout(&e, &rel, Origin::Ui).await {
                        Err(err) => post_notice(&ed, "Use in Module", err),
                        Ok(()) => set_toast(&mut tab.m.lock(), format!("mod.ltx: [pda] layout = {rel}")),
                    }
                    e.exec.wake().ping();
                })
            });
        }
        ui.add_space(4.0);

        let problems = m.preview.layout.problems.len() + usize::from(!m.read_ok);
        let labels = [
            if problems > 0 { format!("Problems ({problems})") } else { "Problems".to_owned() },
            "Pages".to_owned(),
            "Vars".to_owned(),
            "Log".to_owned(),
        ];
        if let Some(t) = m.want_tab.take() {
            m.tab = t;
        }
        let mut tab = m.tab.min(3);
        local_tab_strip(ui, "pda_details", &labels, &mut tab);
        m.tab = tab;
        ui.add_space(6.0);
        ScrollArea::vertical().id_salt(("gz-ui-pda.details", tab)).auto_shrink([false, false]).show(
            ui,
            |ui| match tab {
                0 => self.draw_problems(ui, m),
                1 => draw_pages(ui, m),
                2 => draw_vars(ui, m),
                _ => draw_log(ui, m),
            },
        );
    }

    fn draw_problems(&self, ui: &mut Ui, m: &mut Model) {
        let p = palette(ui.ctx());
        if !m.read_ok {
            let text = if m.read_line > 0 {
                format!("Line {}: {}", m.read_line, m.read_err)
            } else {
                m.read_err.clone()
            };
            text::status(ui, Tone::Error, &text);
            if m.read_line > 0 && Button::new("Go to Line").show(ui).clicked() {
                m.show_source = true;
                m.goto_line = m.read_line;
            }
            ui.add_space(8.0);
        } else if m.preview.layout.problems.is_empty() {
            empty_state(ui, Icon::Success, "No problems", "The game's reader accepts this layout.", None);
        }
        let layout = m.preview.layout.clone();
        for (i, pr) in layout.problems.iter().enumerate() {
            ui.push_id(i, |ui| {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 8.0;
                    badge(ui, &pr.code, if pr.is_error() { Tone::Error } else { Tone::Warning });
                    text::label(
                        ui,
                        if pr.path.is_empty() { "layout" } else { &pr.path },
                        Font::Small,
                        p.text_muted,
                        Clip::End,
                    );
                });
                text::prose(ui, &pr.message);
                ui.add_space(6.0);
            });
        }
        if !m.reader_source.is_empty() {
            text::label(
                ui,
                &format!("Checked by {}", m.reader_source),
                Font::Small,
                p.text_muted,
                Clip::Start,
            );
        }
    }

    fn draw_source(&mut self, ui: &mut Ui, m: &mut Model) {
        let mut goto = false;
        let apply;
        {
            // the toolbar keeps the pane's margin; the text runs edge to edge under it
            let w = ui.available_width();
            let bar = Rect::from_min_size(
                ui.cursor().min + Vec2::new(8.0, 0.0),
                Vec2::new((w - 16.0).max(1.0), ToolbarStyle::Panel.height()),
            );
            let mut bar_ui = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-pda.source_bar")
                    .max_rect(bar)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            ui.allocate_rect(
                Rect::from_min_size(ui.cursor().min, Vec2::new(w, ToolbarStyle::Panel.height())),
                Sense::hover(),
            );
            let mut tb = Toolbar::begin(&mut bar_ui, "pda_source", ToolbarStyle::Panel);
            tb.group(2);
            apply = tb.action(
                "apply",
                Icon::Check,
                "Apply",
                "Show the text in the preview without saving it (Ctrl+Enter)",
                true,
                false,
                Kind::Ghost,
            );
            if !m.read_ok && m.read_line > 0 {
                let label = format!("Line {}", m.read_line);
                goto = tb.action(
                    "goto_error",
                    Icon::Error,
                    &label,
                    "Scroll to the line the reader stopped at",
                    true,
                    false,
                    Kind::Ghost,
                );
            }
            tb.spring();
            tb.group(1);
            if !m.read_ok {
                tb.status(&m.read_err, Tone::Error);
            } else if m.text_dirty {
                tb.status("Unsaved edits", Tone::Warning);
            } else {
                tb.status("Saved", Tone::Muted);
            }
            tb.end();
        }
        if goto {
            m.goto_line = m.read_line;
        }
        let error = if m.read_ok { 0 } else { m.read_line };
        let mut goto_line = std::mem::take(&mut m.goto_line);
        let out = local_code_view(ui, "pda_source", &mut m.text, error, &mut goto_line);
        m.goto_line = goto_line;
        if out.changed {
            m.text_dirty = m.text != m.disk;
        }
        if out.save && m.text_dirty {
            self.save_async();
        }
        if apply || out.apply {
            self.read_async();
        }
    }
}

/// A notice for the author from off the UI thread.
fn post_notice(editor: &Arc<Editor>, title: &'static str, text: String) {
    let unattended = editor.flags.unattended;
    let log = editor.log.clone();
    match gz_app::UiQueue::of(editor) {
        Ok(q) => q.post(move |_, ctx| {
            if let Some(line) = gz_ui::dialog::notice(
                ctx,
                if unattended { Asker::Unattended } else { Asker::Ui },
                Tone::Error,
                title,
                &text,
            ) {
                log.info(line);
            }
        }),
        Err(_) => editor.notices.error(format!("{title}: {text}")),
    }
}

fn draw_pages(ui: &mut Ui, m: &mut Model) {
    let p = palette(ui.ctx());
    let layout = m.preview.layout.clone();
    if layout.pages.is_empty() {
        empty_state(ui, Icon::PdaInterface, "No pages", "The layout declares no page.", None);
    } else {
        let h = metrics::ROW * layout.pages.len() as f32 + 10.0;
        list(ui, "pda_pages", Vec2::new(0.0, h), |l| {
            for pg in &layout.pages {
                let resp = l.row(&pg.id, m.preview.page == pg.id, |ui| {
                    let room = ui.available_width();
                    let title = if pg.title.is_empty() { &pg.id } else { &pg.title };
                    ui.scope(|ui| {
                        ui.set_max_width(room * 0.6);
                        text::label(ui, title, Font::Body, p.text, Clip::End);
                    });
                    let id = if pg.stock.is_empty() {
                        pg.id.clone()
                    } else {
                        format!("{} \u{b7} stock {}", pg.id, pg.stock)
                    };
                    text::label(ui, &id, Font::Body, p.text_muted, Clip::End);
                });
                if resp.clicked() {
                    m.preview.closed = false;
                    m.preview.set_page(&pg.id);
                }
            }
        });
    }
    ui.add_space(8.0);
    if local_section(ui, "pda_keys", "Keys", true) {
        let keys = [
            ("map", "Map key", layout.key_map.clone()),
            ("contacts", "Contacts key", layout.key_contacts.clone()),
            ("tasks", "Tasks key", layout.key_tasks.clone()),
        ];
        let now = super::canvas::clock_now();
        list(ui, "pda_keys_list", Vec2::new(0.0, metrics::ROW * 3.0 + 10.0), |l| {
            for (key, label, action) in &keys {
                let resp = l.row(key, false, |ui| {
                    let room = ui.available_width();
                    ui.scope(|ui| {
                        ui.set_max_width(room * 0.45);
                        text::label(ui, label, Font::Body, p.text, Clip::End);
                    });
                    text::label(
                        ui,
                        if action.is_empty() { "the stock section" } else { action },
                        Font::Body,
                        p.text_muted,
                        Clip::End,
                    );
                });
                if resp.clicked() {
                    m.preview.open_key(key, &now);
                }
            }
        });
        text::label(ui, "Click a key to press it in the preview.", Font::Small, p.text_muted, Clip::End);
    }
}

fn draw_vars(ui: &mut Ui, m: &mut Model) {
    let layout = m.preview.layout.clone();
    let declared = layout.vars.len();
    let view = m.preview.vars_view();
    if view.is_empty() {
        empty_state(
            ui,
            Icon::Variable,
            "No vars",
            "The layout declares none, and no action has set one yet.",
            None,
        );
        return;
    }
    PropertyGrid::new("pda_vars").show(ui, |g| {
        for (i, (name, value)) in view.iter().enumerate() {
            let label = format!("{{{name}}}");
            let help =
                if i < declared { "Declared by the layout" } else { "Set by an action in the preview" };
            g.row(&label, help, PropFlags::READ_ONLY, 0, |ui| read_only(ui, value, Clip::End));
        }
    });
}

fn draw_log(ui: &mut Ui, m: &mut Model) {
    let p = palette(ui.ctx());
    if m.preview.log.is_empty() {
        empty_state(
            ui,
            Icon::Log,
            "No actions yet",
            "Click a button in the preview: every action it runs is listed here, the newest first.",
            None,
        );
        return;
    }
    if Button::new("Clear").kind(Kind::Ghost).show(ui).clicked() {
        m.preview.log.clear();
        return;
    }
    let lines: Vec<String> = m.preview.log.iter().rev().cloned().collect();
    let n = lines.len();
    list(ui, "pda_log", Vec2::new(0.0, metrics::ROW * n as f32 + 10.0), |l| {
        for (k, line) in lines.iter().enumerate() {
            l.row(&(n - 1 - k).to_string(), false, |ui| {
                text::label(ui, line, Font::Body, p.text, Clip::End);
            });
        }
    });
}

impl Document for PdaDoc {
    fn id(&self) -> &str {
        "pda"
    }

    fn kind(&self) -> &'static str {
        "pda"
    }

    fn label(&self) -> String {
        let m = self.tab.m.lock();
        let name = match &self.target {
            Some(Target::Path(rel)) => base_name(rel),
            _ => base_name(&m.path),
        };
        if name.is_empty() { "PDA".to_owned() } else { name }
    }

    fn icon(&self) -> Icon {
        Icon::PdaInterface
    }

    fn path(&self) -> Option<String> {
        let rel = match &self.target {
            Some(Target::Path(rel)) => rel.clone(),
            _ => self.tab.m.lock().rel.clone(),
        };
        (!rel.is_empty()).then_some(rel)
    }

    fn tooltip(&self) -> Option<String> {
        let rel = self.tab.m.lock().rel.clone();
        Some(if rel.is_empty() { "The module's PDA interface".to_owned() } else { rel })
    }

    fn dirty(&self) -> bool {
        self.tab.m.lock().text_dirty
    }

    fn dirty_elsewhere(&self) -> bool {
        true
    }

    fn padded(&self) -> bool {
        false
    }

    fn alive(&self) -> bool {
        let open = !self.started || self.tab.m.lock().open;
        open && self.editor.project_opt().is_some_and(|s| s.root.display().to_string() == self.tab.root)
    }

    fn toolbar<'a>(&'a mut self, tb: &mut Toolbar<'_, 'a>, cx: &mut DocCx<'_>) {
        let clean = "No unsaved edits";
        let (dirty, wide, lang, zoom) = {
            let m = self.tab.m.lock();
            (m.text_dirty, m.preview.aspect == Aspect::Wide, m.preview.lang.clone(), m.zoom)
        };
        tb.group(3);
        if tb.action(
            "save",
            Icon::Save,
            "Save",
            if dirty { "Write the Source pane's text to the file (Ctrl+S)" } else { clean },
            dirty,
            false,
            Kind::Ghost,
        ) {
            self.save_async();
        }
        if tb.action(
            "revert",
            Icon::Undo,
            "Revert",
            if dirty { "Drop the unsaved edits and read the file again" } else { clean },
            dirty,
            false,
            Kind::Ghost,
        ) {
            self.revert(cx.ctx);
        }
        if tb.action("reload", Icon::Reload, "Reload", "Read the game's textures, fonts and layout reader again, and the file when the Source pane holds no edits", true, false, Kind::Ghost) {
            self.reload_all_async();
        }

        // in the overflow each of these is a set of radio items
        tb.group(5);
        let mut aspect = usize::from(wide);
        if tb.segmented(
            "aspect",
            "Aspect",
            &mut aspect,
            &["4:3", "16:9"],
            "The device's shape: 4:3 draws the game's pda.xml, 16:9 its pda_16.xml",
            true,
        ) {
            self.tab.m.lock().preview.aspect = if aspect == 1 { Aspect::Wide } else { Aspect::Standard };
        }
        let mut lang_i = usize::from(lang == "eng");
        if tb.segmented(
            "lang",
            "Language",
            &mut lang_i,
            &["rus", "eng"],
            "The language the texts show in: { rus = ..., eng = ... }",
            true,
        ) {
            self.tab.m.lock().preview.lang = if lang_i == 1 { "eng" } else { "rus" }.to_owned();
            self.read_async();
        }
        let mut zoom_i = if zoom <= 0.0 {
            0
        } else if zoom == 1.0 {
            1
        } else {
            usize::MAX
        };
        if tb.segmented(
            "zoom",
            "Zoom",
            &mut zoom_i,
            &["Fit", "100%"],
            "Fit the device into the canvas (F), or show the game's 1024 x 768 PDA pixel for pixel. The wheel zooms at the mouse; the right button drags.",
            true,
        ) {
            let mut m = self.tab.m.lock();
            m.zoom = if zoom_i == 1 { 1.0 } else { 0.0 };
            m.pan = Vec2::ZERO;
        }

        tb.group(0);
        if tb.action(
            "reset_state",
            Icon::Refresh,
            "Reset State",
            "Back to the start page, with every var and panel as the file declares them",
            true,
            false,
            Kind::Ghost,
        ) {
            self.tab.m.lock().reset_state();
        }

        tb.group(4);
        let (src, side) = {
            let m = self.tab.m.lock();
            (m.show_source, m.show_side)
        };
        if tb.action(
            "source",
            Icon::FileScript,
            "Source",
            "Show the layout's text under the preview",
            true,
            src,
            Kind::Ghost,
        ) {
            let mut m = self.tab.m.lock();
            m.show_source = !m.show_source;
        }
        if tb.action(
            "details",
            Icon::SidebarRight,
            "Details",
            "Show the problems, pages, vars and the action log beside the preview",
            true,
            side,
            Kind::Ghost,
        ) {
            let mut m = self.tab.m.lock();
            m.show_side = !m.show_side;
        }
    }

    fn body(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        self.start();
        self.follow_tick();
        let ctx = ui.ctx().clone();
        self.assets.set_ctx(&ctx);
        self.assets.sync(&self.editor);
        let device = self.assets.device(&self.editor);
        let p = palette(&ctx);
        let full = ui.max_rect();
        let tab = self.tab.clone();
        let mut m = tab.m.lock();
        m.drawn_frame = Some(ctx.cumulative_frame_nr());
        if m.path.is_empty() {
            drop(m);
            let mut child = ui
                .new_child(UiBuilder::new().max_rect(full.shrink(16.0)).layout(Layout::top_down(Align::Min)));
            if self.opening.load(Ordering::SeqCst) {
                busy_line(&mut child, "Opening the PDA interface...");
            } else {
                empty_state(
                    &mut child,
                    Icon::PdaInterface,
                    "No PDA interface",
                    "Open a .pdaasset from the Content Browser, or Mod > PDA Interface.",
                    None,
                );
            }
            ui.allocate_rect(full, Sense::hover());
            return;
        }

        // canvas | Details, over the Source pane; Details and Source keep their pt size, the canvas takes every resize
        let gap_y = ui.spacing().item_spacing.y;
        let gap_x = ui.spacing().item_spacing.x;
        let source_h = if m.show_source {
            gz_ui::splitter::splitter_tail(ui, "pda.source", false, CANVAS_MIN_H, SOURCE_MIN, SOURCE_H).size
        } else {
            0.0
        };
        m.source_h = source_h;
        let top = if m.show_source {
            Rect::from_min_max(
                full.min,
                pos2(full.max.x, (full.max.y - source_h - gap_y).max(full.min.y + 1.0)),
            )
        } else {
            full
        };
        let mut top_ui = ui.new_child(
            UiBuilder::new().id_salt("gz-ui-pda.top").max_rect(top).layout(Layout::left_to_right(Align::Min)),
        );
        let details_w = if m.show_side {
            gz_ui::splitter::splitter_tail(
                &mut top_ui,
                "pda.details",
                true,
                CANVAS_MIN_W,
                SIDE_MIN,
                DETAILS_W,
            )
            .size
        } else {
            0.0
        };
        m.details_w = details_w;
        let canvas = if m.show_side {
            Rect::from_min_max(top.min, pos2((top.max.x - details_w - gap_x).max(top.min.x + 1.0), top.max.y))
        } else {
            top
        };
        top_ui.painter().rect_filled(canvas, egui::CornerRadius::ZERO, p.canvas);
        let mut canvas_ui =
            top_ui.new_child(UiBuilder::new().id_salt("gz-ui-pda.canvas_ui").max_rect(canvas));
        canvas_ui.set_clip_rect(canvas.intersect(ui.clip_rect()));
        match &device {
            Some(dev) => {
                draw_canvas(&mut canvas_ui, canvas, &mut m, dev, &self.assets, &self.editor, &mut self.canvas)
            }
            None => {
                let mut c = canvas_ui.new_child(
                    UiBuilder::new().max_rect(canvas.shrink(16.0)).layout(Layout::top_down(Align::Min)),
                );
                busy_line(&mut c, "Reading the game's PDA...");
            }
        }
        if m.show_side {
            let side = Rect::from_min_max(pos2(canvas.max.x + gap_x, top.min.y), top.max);
            top_ui.painter().rect_filled(side, egui::CornerRadius::ZERO, p.base);
            let mut d = top_ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-pda.details_ui")
                    .max_rect(side.shrink(8.0))
                    .layout(Layout::top_down(Align::Min)),
            );
            d.set_clip_rect(side.intersect(ui.clip_rect()));
            self.draw_details(&mut d, &mut m);
        }
        if m.show_source {
            let src = Rect::from_min_max(pos2(full.min.x, top.max.y + gap_y), full.max);
            let mut s = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-ui-pda.source_ui")
                    .max_rect(src)
                    .layout(Layout::top_down(Align::Min)),
            );
            s.set_clip_rect(src.intersect(ui.clip_rect()));
            self.draw_source(&mut s, &mut m);
        }
        drop(m);
        ui.allocate_rect(full, Sense::hover());
    }

    fn status(&mut self, ui: &mut Ui, _cx: &mut DocCx<'_>) {
        let p = palette(ui.ctx());
        let m = self.tab.m.lock();
        let page = m.preview.layout.find_page(&m.preview.page);
        let where_ = if !m.preview.stock.is_empty() {
            format!("Stock {}", m.preview.stock)
        } else {
            page.filter(|p| !p.title.is_empty()).map_or_else(|| m.preview.page.clone(), |p| p.title.clone())
        };
        let scale = m.geom.map_or(1.0, |g| g.ky * m.ppp);
        let text = format!(
            "Page: {where_} \u{b7} {} \u{b7} {} \u{b7} {:.0}%",
            m.preview.aspect.label(),
            m.preview.lang,
            scale * 100.0
        );
        drop(m);
        text::label(ui, &text, Font::Small, p.text_secondary, Clip::End);
    }

    fn shortcut(&mut self, _cx: &mut DocCx<'_>, chord: &gz_app::Chord) -> bool {
        // F fits the device into the canvas
        if chord.key != egui::Key::F || chord.ctrl || chord.shift || chord.alt {
            return false;
        }
        let mut m = self.tab.m.lock();
        m.zoom = 0.0;
        m.pan = Vec2::ZERO;
        true
    }

    fn save(&mut self, _cx: &mut DocCx<'_>) -> Result<(), String> {
        // written off the UI thread; the pane reads clean at once and a failure says so
        {
            let mut m = self.tab.m.lock();
            if !m.text_dirty {
                return Ok(());
            }
            m.disk = m.text.clone();
            m.text_dirty = false;
        }
        let ed = self.editor.clone();
        self.spawn(move |e, tab| {
            Box::pin(async move {
                if let Err(err) = super::save_and_read(&e, &tab).await {
                    tab.m.lock().text_dirty = true;
                    post_notice(&ed, "Save failed", err);
                }
            })
        });
        Ok(())
    }

    fn discard(&mut self, _cx: &mut DocCx<'_>) {
        let path = {
            let mut m = self.tab.m.lock();
            m.text = m.disk.clone();
            m.text_dirty = false;
            m.path.clone()
        };
        if !path.is_empty() {
            self.spawn(move |ed, tab| {
                Box::pin(async move {
                    super::load_file(&ed, &tab, &path).await;
                })
            });
        }
    }

    fn on_activate(&mut self, _cx: &mut DocCx<'_>) {
        self.start();
    }

    fn inactive(&mut self, _cx: &mut DocCx<'_>) {
        // the file follows the disk whether or not the tab is up
        self.start();
        self.follow_tick();
    }

    fn on_close(&mut self, _cx: &mut DocCx<'_>) {
        let mut m = self.tab.m.lock();
        if m.view_serial == self.serial {
            m.open = false;
        }
    }

    fn restore_arg(&self) -> Option<String> {
        self.path()
    }

    fn as_any_mut(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }
}

/// The layout Mod > PDA Interface opens, as far as snapshots say it without a file read: the
/// one the module uses, else the project's first.
fn module_layout_now(editor: &Editor) -> Option<String> {
    let session = editor.project_opt()?;
    let manifest = ManifestService::get(&session)?;
    let used = manifest.snapshot().manifest().map(|m| m.pda_layout.clone()).unwrap_or_default();
    if !used.is_empty() {
        return Some(used);
    }
    let (_, project) = Project::of(editor).ok()?;
    let tree = project.tree.snapshot();
    let mut files: Vec<String> = tree
        .files_with_extension("pdaasset")
        .iter()
        .map(RelPath::to_backslash)
        .filter(|rel| !manifest.outside_build(rel))
        .collect();
    files.sort();
    files.into_iter().next()
}

/// Mod > PDA Interface (`ShowModuleLayout`): the module's layout in the tab; a project without
/// one gets one from the template, made the module's.
fn open_module_layout(cx: &mut DocCx<'_>) {
    if cx.editor.project_opt().is_none() {
        return;
    }
    if let Some(rel) = module_layout_now(cx.editor) {
        cx.requests.open(OpenRequest::file("pda", &rel));
        return;
    }
    let ed = cx.editor.clone();
    let _task = cx.editor.exec.spawn(async move {
        let Some(rel) = super::module_layout(&ed).await else {
            ed.notices
                .error("The PDA interface could not be made: the pda folder of the project is not writable.");
            return;
        };
        let mut args = Args::new();
        args.insert("action".into(), Value::String("open".into()));
        args.insert("kind".into(), Value::String("pda".into()));
        args.insert("path".into(), Value::String(rel));
        let reply = ed.call("ui_tab", args, ed.ui_origin()).await;
        if reply.get("ok").and_then(Value::as_bool) != Some(true) {
            let why = reply.get("error").and_then(Value::as_str).unwrap_or("the tab did not open");
            ed.notices.error(format!("PDA Interface: {why}"));
        }
    });
}

/// Plugs the tab and Mod > PDA Interface into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.document("pda", |editor, req| {
        let session = editor.project().map_err(|e| e.message)?;
        let tab = PdaTab::of_session(editor, &session);
        let assets = PdaAssets::of_session(&session);
        let target = match req.path.as_deref().filter(|p| !p.is_empty()) {
            Some(p) => {
                let abs = gz_project::ops::resolve_in_project(&tab.root, p)?.replace('/', "\\");
                if !is_pda_path(&abs) {
                    return Err(format!("{p} is not a PDA interface (.pdaasset)"));
                }
                // a tab the session restores whose file is gone shows "File not found"
                if !Path::new(&abs).is_file() {
                    return Err("File not found".to_owned());
                }
                Target::Path(project_relative(&tab.root, &abs))
            }
            None => Target::Module,
        };
        let serial = {
            let mut m = tab.m.lock();
            m.view_serial += 1;
            m.view_serial
        };
        Ok(Box::new(PdaDoc::new(editor.clone(), tab, assets, serial, target)) as Box<dyn Document>)
    });
    b.command(
        UiCommand::new("COMMAND_MOD_PDA", "Mod", "PDA Interface", Icon::PdaInterface, open_module_layout)
            .enabled(|editor, _| editor.project_opt().is_some()),
    );
}
