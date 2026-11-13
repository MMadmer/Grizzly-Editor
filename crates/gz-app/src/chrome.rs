//! The frame's drawing (docs/UI_DESIGN.md, section 5): the caption with its menus, tab strip and
//! window buttons, the toolbar, the status bar, the work area, the drawers, the Task Center's
//! card, the Command Palette.
//!
//! Widgets the kit lacks, local here: [`local_submenu`], [`local_caption_button`],
//! [`local_window_button`], [`local_status_chip`].

use std::sync::Arc;
use std::time::Duration;

use egui::containers::menu::SubMenu;
use egui::{
    Align, Align2, Area, Color32, Context, CornerRadius, Frame, Id, Key, Layout, Margin, Modifiers, Order,
    Popup, Pos2, Rect, Response, Sense, Stroke, StrokeKind, TextureHandle, Ui, UiBuilder, Vec2,
    ViewportCommand, pos2,
};
use gz_editor::SmolStr;
use gz_ui::menu::{menu_popup, popup_body};
use gz_ui::text::{self, Clip};
use gz_ui::{
    Button, Font, Icon, Kind, MenuItem, SearchField, Tone, Toolbar, ToolbarStyle, audit, icon_button_sized,
    icons, menu_item, palette, state,
};

use crate::builder::MenuContent;
use crate::contract;
use crate::doc::{DocCx, Drawer, EditAction, OpenRequest, PanelOp};
use crate::palette::{self as pal, PaletteRow};
use crate::shell::{PopupInfo, ShellState, Tab, cx};
use crate::tabs::{self, LEVEL};

pub const CAPTION_H: f32 = 32.0;
pub const TOOLBAR_H: f32 = 36.0;
pub const STATUS_H: f32 = 24.0;
const LOGO: f32 = 20.0;
const BUTTON_W: f32 = 46.0;
const EDGE: f32 = 6.0;
/// Under this width the menus fold into one Menu button.
pub const FOLD_W: f32 = 1100.0;

/// The editor's one icon, the same file the executables carry (`gz_ico::embed`).
const ICON_ICO: &[u8] = include_bytes!("../../../assets/grizzly-editor-icon.ico");

/// The logo as straight RGBA of `side` x `side`: the icon's image drawn for that side, or
/// box-filtered down from the next larger one.
#[must_use]
pub fn logo_rgba(side: u32) -> Option<Vec<u8>> {
    let images = gz_ico::images(ICON_ICO).ok()?;
    let image = gz_ico::for_side(&images, side).filter(gz_ico::IcoImage::is_png)?;
    let mut decoder = png::Decoder::new(std::io::Cursor::new(image.data));
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder.read_info().ok()?;
    let mut buf = vec![0u8; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    if info.color_type != png::ColorType::Rgba || info.bit_depth != png::BitDepth::Eight || side == 0 {
        return None;
    }
    let (w, h) = (info.width as usize, info.height as usize);
    let side = side as usize;
    let mut out = vec![0u8; side * side * 4];
    for y in 0..side {
        for x in 0..side {
            let (x0, x1) = (x * w / side, ((x + 1) * w / side).max(x * w / side + 1));
            let (y0, y1) = (y * h / side, ((y + 1) * h / side).max(y * h / side + 1));
            let mut acc = [0u64; 4];
            let mut n = 0u64;
            for sy in y0..y1.min(h) {
                for sx in x0..x1.min(w) {
                    let at = (sy * w + sx) * 4;
                    if let Some(px) = buf.get(at..at + 4) {
                        let a = u64::from(px[3]);
                        // alpha-weighted, so transparent edges do not darken the picture
                        acc[0] += u64::from(px[0]) * a;
                        acc[1] += u64::from(px[1]) * a;
                        acc[2] += u64::from(px[2]) * a;
                        acc[3] += a;
                        n += 1;
                    }
                }
            }
            let at = (y * side + x) * 4;
            if let Some(px) = out.get_mut(at..at + 4)
                && n > 0
            {
                let a = acc[3].max(1);
                px[0] = (acc[0] / a) as u8;
                px[1] = (acc[1] / a) as u8;
                px[2] = (acc[2] / a) as u8;
                px[3] = (acc[3] / n) as u8;
            }
        }
    }
    Some(out)
}

pub(crate) fn load_logo(ctx: &Context) -> Option<TextureHandle> {
    let side = 64;
    let rgba = logo_rgba(side)?;
    Some(ctx.load_texture(
        "gz-app.logo",
        egui::ColorImage::from_rgba_unmultiplied([side as usize, side as usize], &rgba),
        egui::TextureOptions::LINEAR,
    ))
}

// ---- local widgets --------------------------------------------------------------------------

/// A caption menu's button: 32 pt tall, the label in Body with 8 pt each side, the hover fill
/// while hovered or open.
pub fn local_caption_button(ui: &mut Ui, label: &str, open: bool, enabled: bool) -> Response {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let w = text::width(ui.ctx(), &font, label) + 16.0;
    let h = ui.max_rect().height().min(CAPTION_H);
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, h), sense);
    if ui.is_rect_visible(rect) {
        if enabled && (open || resp.hovered()) {
            ui.painter().rect_filled(
                rect.shrink2(Vec2::new(0.0, 4.0)),
                CornerRadius::same(4),
                if resp.is_pointer_button_down_on() { p.pressed } else { p.hover },
            );
        }
        let color = if enabled { p.text } else { p.text_disabled };
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, label, font, color);
    }
    audit::control(ui.ctx(), label, enabled, None, None, rect);
    resp
}

/// A window button of the caption: 46 x 32, a whisper of a fill on hover, the danger fill
/// under the close cross only.
pub fn local_window_button(ui: &mut Ui, rect: Rect, icon: Icon, tip: &str, danger: bool) -> bool {
    let p = palette(ui.ctx());
    let resp = ui.interact(rect, ui.scope_id().with(("gz-app.window_button", tip)), Sense::click());
    if resp.hovered() {
        let fill = match (danger, resp.is_pointer_button_down_on()) {
            (true, true) => p.danger_pressed,
            (true, false) => p.danger_hover,
            (false, true) => p.pressed,
            (false, false) => p.hover,
        };
        ui.painter().rect_filled(rect, CornerRadius::ZERO, fill);
    }
    let color = if resp.hovered() { p.text } else { p.text_secondary };
    icons::draw(ui.painter(), icon, Rect::from_center_size(rect.center(), Vec2::splat(12.0)), color);
    audit::control(ui.ctx(), tip, true, None, Some(tip), rect);
    resp.on_hover_text(tip).clicked()
}

/// The maximise button: Windows owns its clicks (`HTMAXBUTTON`), so its hover comes from the
/// window's hit test, not from egui.
fn local_max_button(ui: &mut Ui, rect: Rect, icon: Icon, tip: &str, hot: bool) {
    let p = palette(ui.ctx());
    if hot {
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.hover);
    }
    icons::draw(
        ui.painter(),
        icon,
        Rect::from_center_size(rect.center(), Vec2::splat(12.0)),
        if hot { p.text } else { p.text_secondary },
    );
    audit::control(ui.ctx(), tip, true, None, Some(tip), rect);
}

/// A menu row that opens a submenu to its right while hovered: the kit's look, a chevron at
/// the end.
pub fn local_submenu<R>(
    ui: &mut Ui,
    label: &str,
    icon: Icon,
    enabled: bool,
    add: impl FnOnce(&mut Ui) -> R,
) -> Option<R> {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let slot = icon != Icon::None;
    let w = 8.0 + if slot { 24.0 } else { 0.0 } + text::width(ui.ctx(), &font, label) + 24.0 + 16.0;
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let resp = ui.allocate_response(Vec2::new(w, 24.0), sense);
    let rect = resp.rect;
    if ui.is_rect_visible(rect) {
        if enabled && resp.hovered() {
            ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
        }
        let color = if enabled { p.text } else { p.text_disabled };
        let mut x = rect.min.x + 8.0;
        if slot {
            icons::draw(
                ui.painter(),
                icon,
                Rect::from_min_size(pos2(x, rect.center().y - 8.0), Vec2::splat(16.0)),
                p.text_secondary,
            );
            x += 24.0;
        }
        text::paint(
            ui.painter(),
            &font,
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            color,
            label,
            Some((rect.max.x - 24.0 - x).max(0.0)),
            Clip::End,
        );
        icons::draw(
            ui.painter(),
            Icon::ChevronRight,
            Rect::from_min_size(pos2(rect.max.x - 20.0, rect.center().y - 6.0), Vec2::splat(12.0)),
            p.text_secondary,
        );
    }
    audit::control(ui.ctx(), label, enabled, None, None, rect);
    if !enabled {
        return None;
    }
    SubMenu::new().show(ui, &resp, |ui| popup_body(ui, add)).map(|r| r.inner)
}

/// A chip of the status bar: an icon in its tone and a short text in Small.
pub fn local_status_chip(
    ui: &mut Ui,
    icon: Icon,
    tone: Option<Tone>,
    label: &str,
    tip: &str,
    checked: bool,
) -> Response {
    let p = palette(ui.ctx());
    let font = Font::Small.id();
    let text_w = if label.is_empty() { 0.0 } else { text::width(ui.ctx(), &font, label) + 4.0 };
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(6.0 + 14.0 + text_w + 6.0, 20.0), Sense::click());
    if ui.is_rect_visible(rect) {
        if checked || resp.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                if checked { p.selection_focused } else { p.hover },
            );
        }
        let icon_color = tone.map_or(p.text_secondary, |t| p.tone(t));
        icons::draw(
            ui.painter(),
            icon,
            Rect::from_min_size(pos2(rect.min.x + 6.0, rect.center().y - 7.0), Vec2::splat(14.0)),
            icon_color,
        );
        if !label.is_empty() {
            ui.painter().text(
                pos2(rect.min.x + 24.0, rect.center().y),
                Align2::LEFT_CENTER,
                label,
                font,
                p.text_secondary,
            );
        }
    }
    audit::control(ui.ctx(), if label.is_empty() { tip } else { label }, true, None, Some(tip), rect);
    resp.on_hover_text(tip)
}

// ---- the menu bar -----------------------------------------------------------------------------

/// What the caption's menus keep between frames.
#[derive(Default)]
pub(crate) struct MenuBarState {
    pub folded: bool,
    /// The menu open last frame.
    pub open: Option<SmolStr>,
    pub open_now: Option<SmolStr>,
    pub shown: Vec<PopupInfo>,
}

pub(crate) fn menu_id(name: &str) -> Id {
    Id::unique(("gz-app.caption_menu", name))
}

pub(crate) fn fold_id() -> Id {
    Id::unique("gz-app.caption_menus_folded")
}

/// The caption's menus as a document adds its own (Scene, Build, View; Graph): a menu here is a
/// button on the caption, or a submenu of the Menu button while the window is too narrow.
pub struct MenuBar<'u, 's> {
    ui: &'u mut Ui,
    state: &'s mut MenuBarState,
}

impl MenuBar<'_, '_> {
    /// A menu; `add` draws its rows (`gz_ui::menu_item`, [`local_submenu`]) while it is open.
    pub fn menu(&mut self, name: &str, add: impl FnOnce(&mut Ui)) {
        self.menu_enabled(name, true, add);
    }

    pub fn menu_enabled(&mut self, name: &str, enabled: bool, add: impl FnOnce(&mut Ui)) {
        if self.state.folded {
            if local_submenu(self.ui, name, Icon::None, enabled, add).is_some() {
                self.state.open_now = Some(SmolStr::new(name));
            }
            return;
        }
        let ctx = self.ui.ctx().clone();
        let id = menu_id(name);
        let open = Popup::is_id_open(&ctx, id);
        let resp = local_caption_button(self.ui, name, open, enabled);
        if enabled {
            if resp.clicked() {
                Popup::toggle_id(&ctx, id);
            } else if resp.hovered() && !open && self.state.open.as_ref().is_some_and(|o| o != name) {
                // a menu of the bar is open: hovering another switches to it
                Popup::open_id(&ctx, id);
            }
        }
        let layer = self.ui.layer_id();
        if menu_popup(&ctx, id, resp.rect, layer, add).is_some() {
            self.state.open_now = Some(SmolStr::new(name));
            let rect = ctx.memory(|m| m.area_rect(id)).unwrap_or(resp.rect);
            self.state.shown.push(PopupInfo {
                id: format!("menu:{name}"),
                title: name.to_owned(),
                window: format!("##Menu_{name}"),
                rect,
            });
        }
    }

    /// The menus are folded into the Menu button.
    #[must_use]
    pub fn folded(&self) -> bool {
        self.state.folded
    }
}

/// The shell's own rows, placed among the contributions by their menu, group and order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Builtin {
    Save,
    OpenProjectFolder,
    SwitchProject,
    Quit,
    Edit(EditAction),
    Palette,
    Preferences,
    Panels,
    Drawers,
    Documents,
    ResetLayout,
    KeyboardShortcuts,
    About,
}

const BUILTINS: [(&str, Option<&str>, i32, i32, Builtin); 19] = [
    ("File", None, 20, 0, Builtin::Save),
    ("File", Some("Project"), 50, 0, Builtin::OpenProjectFolder),
    ("File", Some("Project"), 60, 0, Builtin::SwitchProject),
    ("File", None, 90, 0, Builtin::Quit),
    ("Edit", None, 0, 0, Builtin::Edit(EditAction::Undo)),
    ("Edit", None, 0, 1, Builtin::Edit(EditAction::Redo)),
    ("Edit", None, 10, 0, Builtin::Edit(EditAction::Cut)),
    ("Edit", None, 10, 1, Builtin::Edit(EditAction::Copy)),
    ("Edit", None, 10, 2, Builtin::Edit(EditAction::Paste)),
    ("Edit", None, 10, 3, Builtin::Edit(EditAction::Duplicate)),
    ("Edit", None, 10, 4, Builtin::Edit(EditAction::Delete)),
    ("Edit", None, 20, 0, Builtin::Edit(EditAction::Find)),
    ("Edit", None, 90, 0, Builtin::Palette),
    ("Edit", None, 90, 1, Builtin::Preferences),
    ("Window", None, 0, 0, Builtin::Panels),
    ("Window", None, 50, 0, Builtin::Drawers),
    ("Window", None, 60, 0, Builtin::Documents),
    ("Window", None, 90, 0, Builtin::ResetLayout),
    ("Help", None, 0, 0, Builtin::KeyboardShortcuts),
];

#[derive(Clone, Debug, PartialEq, Eq)]
enum Row {
    Builtin(Builtin),
    Entry(usize),
    Submenu(SmolStr),
}

/// What a row picked asks the shell to do once the menu is drawn.
enum Pick {
    Builtin(Builtin),
    Command(usize),
    Edit(EditAction),
}

fn edit_label(a: EditAction) -> (&'static str, &'static str, Icon, Option<&'static str>) {
    match a {
        EditAction::Undo => ("Undo", "Ctrl+Z", Icon::Undo, Some("COMMAND_UNDO")),
        EditAction::Redo => ("Redo", "Ctrl+Y", Icon::Redo, Some("COMMAND_REDO")),
        EditAction::Cut => ("Cut", "Ctrl+X", Icon::Cut, Some("COMMAND_CUT")),
        EditAction::Copy => ("Copy", "Ctrl+C", Icon::Copy, Some("COMMAND_COPY")),
        EditAction::Paste => ("Paste", "Ctrl+V", Icon::Paste, Some("COMMAND_PASTE")),
        EditAction::Duplicate => ("Duplicate", "Ctrl+D", Icon::Duplicate, None),
        EditAction::Delete => ("Delete", "Del", Icon::Delete, Some("COMMAND_DELETE_SELECTION")),
        EditAction::Find => ("Find", "Ctrl+F", Icon::Search, Some("COMMAND_SHOW_OBJECTLIST")),
    }
}

impl ShellState {
    // ---- the frame -----------------------------------------------------------------------

    pub(crate) fn draw_frame(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let screen = ui.max_rect();
        let caption = Rect::from_min_max(screen.min, pos2(screen.max.x, screen.min.y + CAPTION_H));
        let status =
            Rect::from_min_max(pos2(screen.min.x, (screen.max.y - STATUS_H).max(caption.max.y)), screen.max);
        let toolbar = if self.start_page {
            None
        } else {
            Some(Rect::from_min_max(
                pos2(screen.min.x, caption.max.y),
                pos2(screen.max.x, (caption.max.y + TOOLBAR_H).min(status.min.y)),
            ))
        };
        let work = Rect::from_min_max(
            pos2(screen.min.x, toolbar.map_or(caption.max.y, |t| t.max.y)),
            pos2(screen.max.x, status.min.y),
        );
        self.work_rect = work;
        // the kit centres its dialogs in the work area, between the toolbar and the status bar
        gz_ui::dialog::set_work_area(&ctx, work);
        self.status_rect = status;
        ui.painter().rect_filled(screen, CornerRadius::ZERO, p.base);

        self.draw_caption(ui, caption);
        if let Some(t) = toolbar {
            self.draw_toolbar(ui, t);
        }
        self.draw_status(ui, status);
        self.draw_work(ui, work);
        self.apply_requests(&ctx);
        self.draw_drawer(ui);
        self.draw_dialogs(ui);
        self.draw_card(ui);
        self.draw_palette(ui);
        let toast_bottom = self.card_rect.map_or(work.max.y, |r| r.min.y + 8.0);
        let mut toasts = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-app.toasts")
                .max_rect(Rect::from_min_max(work.min, pos2(work.max.x, toast_bottom.max(work.min.y)))),
        );
        state::draw_toasts(&mut toasts);
        self.publish_hit_test(&ctx);
        self.publish_dirty_tabs();
        self.title_sync(&ctx);
    }

    /// Hands the window's hit test what the caption drew, in client pixels: menus, tabs and
    /// buttons take the mouse, the maximise button is `HTMAXBUTTON` (snap layouts), the rest of
    /// the band drags the window, and a restored window resizes from its edges.
    fn publish_hit_test(&self, ctx: &Context) {
        let ppp = ctx.pixels_per_point();
        let px = |r: Rect| gz_win::PxRect {
            x0: (r.min.x * ppp) as i32,
            y0: (r.min.y * ppp) as i32,
            x1: (r.max.x * ppp) as i32,
            y1: (r.max.y * ppp) as i32,
        };
        let c = &self.caption;
        let bar = c.bar;
        let shape = if c.valid && bar.is_positive() {
            let span = |x0: f32, x1: f32| px(Rect::from_min_max(pos2(x0, bar.min.y), pos2(x1, bar.max.y)));
            gz_win::CaptionShape {
                band: Some(px(bar)),
                interactive: vec![
                    span(c.menus_x0, c.menus_end),
                    span(c.buttons_x, c.buttons_x + BUTTON_W),
                    span(c.buttons_x + 2.0 * BUTTON_W, bar.max.x),
                ],
                max_button: Some(span(c.buttons_x + BUTTON_W, c.buttons_x + 2.0 * BUTTON_W)),
                border: (EDGE * ppp).round() as i32,
            }
        } else {
            gz_win::CaptionShape { border: (EDGE * ppp).round() as i32, ..Default::default() }
        };
        self.hit_test.publish(shape);
    }

    fn title_sync(&self, ctx: &Context) {
        let info = crate::project::project_info(&self.editor);
        let title =
            if info.active { format!("{} - Grizzly Editor", info.name) } else { "Grizzly Editor".to_owned() };
        let current = ctx.input(|i| i.viewport().title.clone());
        if current.as_deref() != Some(title.as_str()) {
            ctx.send_viewport_cmd(ViewportCommand::Title(title));
        }
    }

    // ---- the caption ---------------------------------------------------------------------

    fn draw_caption(&mut self, ui: &mut Ui, bar: Rect) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(bar, CornerRadius::ZERO, p.chrome);
        self.caption.bar = bar;
        self.caption.buttons_x = bar.max.x - 3.0 * BUTTON_W;
        let x0 = bar.min.x + 10.0;
        if let Some(logo) = &self.logo {
            let r = Rect::from_min_size(pos2(x0, (bar.center().y - LOGO * 0.5).round()), Vec2::splat(LOGO));
            ui.painter().image(logo.id(), r, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
        }
        self.caption.menus_x0 = x0 + LOGO + 6.0;
        let mut row = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-app.caption")
                .max_rect(Rect::from_min_max(
                    pos2(self.caption.menus_x0, bar.min.y),
                    pos2(self.caption.buttons_x, bar.max.y),
                ))
                .layout(Layout::left_to_right(Align::Center)),
        );
        row.spacing_mut().item_spacing = Vec2::ZERO;
        let mut end = self.caption.menus_x0;
        if self.menus_shown {
            self.folded = bar.width() < FOLD_W;
            let mut mb = std::mem::take(&mut self.menu_bar);
            mb.shown.clear();
            mb.open_now = None;
            if self.folded {
                mb.folded = true;
                let open = Popup::is_id_open(&ctx, fold_id());
                let r = row.allocate_exact_size(Vec2::new(28.0, CAPTION_H), Sense::hover()).0;
                let mut cell = row.new_child(
                    UiBuilder::new().max_rect(Rect::from_center_size(r.center(), Vec2::splat(24.0))),
                );
                if icon_button_sized(&mut cell, Icon::Menu, "Menus", open, true, 24.0, 16.0).clicked() {
                    Popup::toggle_id(&ctx, fold_id());
                }
                let layer = row.layer_id();
                let anchor = Rect::from_min_max(pos2(r.min.x, bar.min.y), pos2(r.max.x, bar.max.y));
                if menu_popup(&ctx, fold_id(), anchor, layer, |ui| self.draw_menus(ui, &ctx, &mut mb))
                    .is_some()
                {
                    let rect = ctx.memory(|m| m.area_rect(fold_id())).unwrap_or(anchor);
                    mb.shown.push(PopupInfo {
                        id: "menu:Menu".to_owned(),
                        title: "Menu".to_owned(),
                        window: "##caption_menus".to_owned(),
                        rect,
                    });
                }
                row.add_space(4.0);
            } else {
                mb.folded = false;
                self.draw_menus(&mut row, &ctx, &mut mb);
            }
            mb.open = mb.open_now.take();
            self.popups_cur.append(&mut mb.shown);
            self.menu_bar = mb;
            end = self.draw_tab_strip(&mut row, bar, self.caption.buttons_x - 48.0);
        } else {
            self.folded = bar.width() < FOLD_W;
            for t in &mut self.tabs {
                t.rect = None;
            }
        }
        self.caption.menus_end = end;

        // the project's name in the band, the first thing to go when the tabs need the room
        let info = crate::project::project_info(&self.editor);
        if info.active {
            let font = Font::Body.id();
            let name = info.name;
            let w = text::width(&ctx, &font, &name);
            let right = self.caption.buttons_x - 12.0;
            if right - w >= end + 48.0 {
                ui.painter().text(
                    pos2((right - w).round(), bar.center().y),
                    Align2::LEFT_CENTER,
                    &name,
                    font,
                    p.text_muted,
                );
            }
        }

        // the free band is HTCAPTION to Windows: the system drags, maximises on a double click
        // and snaps (`publish_hit_test`)

        // the window buttons, flush with the window's edge
        let maximized = ctx.input(|i| i.viewport().maximized == Some(true));
        let buttons_x = self.caption.buttons_x;
        let b = |n: f32| {
            Rect::from_min_size(pos2(buttons_x + n * BUTTON_W, bar.min.y), Vec2::new(BUTTON_W, CAPTION_H))
        };
        if local_window_button(ui, b(0.0), Icon::Minimize, "Minimize", false) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        // HTMAXBUTTON: Windows clicks it (and offers its snap layouts); egui only sees the hover
        // the hit test reports
        let (icon, tip) = if maximized { (Icon::Restore, "Restore") } else { (Icon::Maximize, "Maximize") };
        local_max_button(ui, b(1.0), icon, tip, self.hit_test.max_hot());
        if local_window_button(ui, b(2.0), Icon::Close, "Close", true) {
            self.request_exit(&ctx);
        }
        self.caption.valid = true;
    }

    /// Every menu: File, Edit, the active tab's own, Mod, Libraries, Window, Help.
    fn draw_menus(&mut self, ui: &mut Ui, ctx: &Context, mb: &mut MenuBarState) {
        for name in ["File", "Edit"] {
            self.shell_menu(ui, ctx, mb, name);
        }
        let id = self.active.to_string();
        if let Some(i) = self.find(&id)
            && let Some(tab) = self.tabs.get_mut(i)
            && !tab.failed
            && tab.panicked.is_none()
            && let Some(doc) = tab.doc.as_mut()
            && !doc.missing()
        {
            let mut bar = MenuBar { ui, state: mb };
            doc.menus(&mut bar, &mut cx!(self, ctx));
        }
        for name in ["Mod", "Libraries", "Window", "Help"] {
            self.shell_menu(ui, ctx, mb, name);
        }
    }

    /// The rows of one of the shell's menus, contributions and its own, by group and order.
    fn rows_of(&self, menu: &str, submenu: Option<&str>) -> Vec<(i32, i32, Row)> {
        let mut rows: Vec<(i32, i32, Row)> = Vec::new();
        // a submenu stands where its first row does
        let mut subs: Vec<SmolStr> = Vec::new();
        let mut sub_row = |rows: &mut Vec<(i32, i32, Row)>, name: &str, group: i32, order: i32| {
            if !subs.iter().any(|s| s == name) {
                subs.push(SmolStr::new(name));
                rows.push((group, order, Row::Submenu(SmolStr::new(name))));
            }
        };
        for (m, sub, group, order, b) in BUILTINS {
            if m != menu {
                continue;
            }
            match (sub, submenu) {
                (None, None) => rows.push((group, order, Row::Builtin(b))),
                (Some(s), None) => sub_row(&mut rows, s, group, order),
                (Some(s), Some(want)) if s == want => rows.push((group, order, Row::Builtin(b))),
                _ => {}
            }
        }
        if menu == "Help" && submenu.is_none() {
            rows.push((0, 1, Row::Builtin(Builtin::About)));
        }
        for (i, e) in self.parts.menus.iter().enumerate() {
            if e.menu != menu {
                continue;
            }
            match (e.submenu.as_deref(), submenu) {
                (None, None) => rows.push((e.group, e.order, Row::Entry(i))),
                (Some(s), None) => sub_row(&mut rows, s, e.group, e.order),
                (Some(s), Some(want)) if s == want => rows.push((e.group, e.order, Row::Entry(i))),
                _ => {}
            }
        }
        rows.sort_by_key(|(g, o, _)| (*g, *o));
        rows
    }

    /// Whether a shell menu has anything to show.
    pub(crate) fn menu_has_rows(&self, menu: &str) -> bool {
        !self.rows_of(menu, None).is_empty()
    }

    fn shell_menu(&mut self, ui: &mut Ui, ctx: &Context, mb: &mut MenuBarState, name: &str) {
        if !self.menu_has_rows(name) {
            return;
        }
        let mut picks: Vec<Pick> = Vec::new();
        {
            let mut bar = MenuBar { ui, state: mb };
            bar.menu(name, |ui| self.menu_rows(ui, ctx, name, None, &mut picks));
        }
        for pick in picks {
            self.run_pick(ctx, pick);
        }
    }

    fn menu_rows(
        &mut self,
        ui: &mut Ui,
        ctx: &Context,
        menu: &str,
        submenu: Option<&str>,
        picks: &mut Vec<Pick>,
    ) {
        let rows = self.rows_of(menu, submenu);
        let mut last_group = None;
        for (group, _, row) in rows {
            if last_group.is_some_and(|g| g != group) {
                ui.separator();
            }
            last_group = Some(group);
            match row {
                Row::Builtin(b) => self.builtin_row(ui, ctx, b, picks),
                Row::Entry(i) => self.entry_row(ui, ctx, i, picks),
                Row::Submenu(name) => {
                    local_submenu(ui, &name, Icon::None, true, |ui| {
                        self.menu_rows(ui, ctx, menu, Some(&name), picks)
                    });
                }
            }
        }
    }

    fn entry_row(&mut self, ui: &mut Ui, ctx: &Context, i: usize, picks: &mut Vec<Pick>) {
        let (command, label) = match self.parts.menus.get(i).map(|e| &e.content) {
            Some(MenuContent::Command { command, label }) => (command.clone(), label.clone()),
            Some(MenuContent::Custom(_)) => {
                if let Some(MenuContent::Custom(draw)) = self.parts.menus.get_mut(i).map(|e| &mut e.content) {
                    draw(
                        ui,
                        &mut DocCx {
                            editor: &self.editor,
                            gpu: &self.gpu,
                            ctx,
                            active_tab: &self.active,
                            requests: &mut self.requests,
                        },
                    );
                }
                return;
            }
            None => return,
        };
        let Some(c) = self.command_index(&command) else { return };
        let keys = self.binding(c).map(|k| k.label()).unwrap_or_default();
        let (text, icon) = match self.parts.commands.get(c) {
            Some(cmd) => (label.map_or_else(|| cmd.label.to_string(), |l| l.to_string()), cmd.icon),
            None => return,
        };
        let item = MenuItem { icon, keys: &keys, enabled: self.command_enabled(c), ..MenuItem::default() };
        if menu_item(ui, &text, item) {
            picks.push(Pick::Command(c));
        }
    }

    fn builtin_row(&mut self, ui: &mut Ui, ctx: &Context, b: Builtin, picks: &mut Vec<Pick>) {
        match b {
            Builtin::Save => {
                if menu_item(ui, "Save", MenuItem { icon: Icon::Save, keys: "Ctrl+S", ..MenuItem::default() })
                {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::OpenProjectFolder => {
                let on = self.editor.project_opt().is_some();
                if menu_item(
                    ui,
                    "Open Project Folder",
                    MenuItem { icon: Icon::Reveal, enabled: on, ..MenuItem::default() },
                ) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::SwitchProject => {
                let on = self.editor.project_opt().is_some();
                let resp =
                    menu_item(ui, "Switch Project...", MenuItem { enabled: on, ..MenuItem::default() });
                if resp {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::Quit => {
                let keys = self.keys_of("COMMAND_EXIT", "");
                if menu_item(ui, "Quit", MenuItem { keys: &keys, ..MenuItem::default() }) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::Edit(a) => {
                let (label, fallback, icon, level_command) = edit_label(a);
                let keys = match level_command {
                    Some(id) if self.active == LEVEL => self.keys_of(id, fallback),
                    _ => fallback.to_owned(),
                };
                let on = self.edit(ctx, a, false);
                if menu_item(ui, label, MenuItem { icon, keys: &keys, enabled: on, ..MenuItem::default() }) {
                    picks.push(Pick::Edit(a));
                }
            }
            Builtin::Palette => {
                let keys = self.keys_of("COMMAND_COMMAND_PALETTE", "Ctrl+Shift+P");
                if menu_item(
                    ui,
                    "Command Palette...",
                    MenuItem { icon: Icon::Search, keys: &keys, ..MenuItem::default() },
                ) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::Preferences => {
                let keys = self.keys_of("COMMAND_EDITOR_PREF", "");
                if menu_item(
                    ui,
                    "Preferences",
                    MenuItem { icon: Icon::Settings, keys: &keys, ..MenuItem::default() },
                ) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::Panels => {
                let panels = self.parts.panels.clone();
                for panel in panels {
                    let open = self.level_panel(&panel.id).is_some_and(|p| p.open);
                    if menu_item(
                        ui,
                        &panel.label,
                        MenuItem { icon: panel.icon, selected: open, ..MenuItem::default() },
                    ) {
                        self.requests.panel(&panel.id, if open { PanelOp::Close } else { PanelOp::Focus });
                    }
                }
            }
            Builtin::Drawers => {
                for (which, label, keys, icon) in [
                    (Drawer::Content, "Content Drawer", "Ctrl+Space", Icon::Drawer),
                    (Drawer::Log, "Log Drawer", "Ctrl+`", Icon::Log),
                ] {
                    if !self.has_drawer(which) {
                        continue;
                    }
                    if menu_item(
                        ui,
                        label,
                        MenuItem { icon, keys, selected: self.drawer == Some(which), ..MenuItem::default() },
                    ) {
                        self.requests.toggle_drawer(which);
                    }
                }
            }
            Builtin::Documents => {
                // the open documents, the way Ctrl+Tab walks them
                let list: Vec<(SmolStr, String, bool)> =
                    self.tabs.iter().map(|t| (t.id.clone(), t.label(), t.dirty())).collect();
                for (id, label, dirty) in list {
                    let text = if dirty { format!("{label} *") } else { label };
                    let picked = ui
                        .push_id(id.as_str(), |ui| {
                            menu_item(
                                ui,
                                &text,
                                MenuItem { selected: id == self.active, ..MenuItem::default() },
                            )
                        })
                        .inner;
                    if picked {
                        self.requests.activate(&id);
                    }
                }
            }
            Builtin::ResetLayout => {
                if menu_item(ui, "Reset Layout", MenuItem::default()) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::KeyboardShortcuts => {
                if menu_item(
                    ui,
                    "Keyboard Shortcuts",
                    MenuItem { icon: Icon::Keyboard, ..MenuItem::default() },
                ) {
                    picks.push(Pick::Builtin(b));
                }
            }
            Builtin::About => {
                if menu_item(ui, "About Grizzly Editor", MenuItem { icon: Icon::Info, ..MenuItem::default() })
                {
                    picks.push(Pick::Builtin(b));
                }
            }
        }
    }

    fn run_pick(&mut self, ctx: &Context, pick: Pick) {
        match pick {
            Pick::Command(i) => {
                self.run_command(ctx, i);
            }
            Pick::Edit(a) => {
                self.edit(ctx, a, true);
            }
            Pick::Builtin(b) => match b {
                Builtin::Save => self.save_active(ctx),
                Builtin::OpenProjectFolder => {
                    if let Some(root) = self.editor.project_root() {
                        reveal_folder(&root);
                    }
                }
                Builtin::SwitchProject => self.switch_project(ctx),
                Builtin::Quit => self.request_exit(ctx),
                Builtin::Palette => self.open_palette(""),
                Builtin::Preferences => self.requests.open(OpenRequest::kind("preferences")),
                Builtin::ResetLayout => self.reset_layout(ctx, false),
                Builtin::KeyboardShortcuts => self.requests.open(OpenRequest {
                    arg: Some("Keyboard".to_owned()),
                    ..OpenRequest::kind("preferences")
                }),
                Builtin::About => {
                    if let Some(line) = gz_ui::dialog::notice(
                        ctx,
                        self.asker(),
                        Tone::Info,
                        "About Grizzly Editor",
                        "Grizzly Editor - the level and mod editor of Dead Air Refined.\nAgents drive it over MCP on 127.0.0.1.",
                    ) {
                        self.editor.log.plain(line);
                    }
                }
                Builtin::Edit(_) | Builtin::Panels | Builtin::Drawers | Builtin::Documents => {}
            },
        }
    }

    /// Back to the Start page, after the question about the unsaved tabs.
    fn switch_project(&mut self, ctx: &Context) {
        let dirty: Vec<(String, String)> =
            self.dirty_tabs().into_iter().filter(|(id, _)| id != LEVEL).collect();
        let editor = self.editor.clone();
        let close = move |editor: Arc<gz_editor::Editor>| {
            editor.post(
                "project_close",
                gz_editor::Args::from_iter([("discard".to_owned(), serde_json::Value::Bool(true))]),
            )
        };
        if dirty.is_empty() {
            close(editor);
            return;
        }
        let what: Vec<String> = dirty.iter().map(|(_, l)| l.clone()).collect();
        let ids: Vec<String> = dirty.into_iter().map(|(id, _)| id).collect();
        let queue = self.queue.clone();
        let line = gz_ui::dialog::unsaved_prompt(ctx, self.asker(), "switch_project", &what, move |answer| {
            if answer == gz_ui::Answer::Cancel {
                return;
            }
            queue.post(move |shell, ctx| {
                for id in &ids {
                    if answer == gz_ui::Answer::No {
                        shell.discard_tab(ctx, id);
                    } else if let Err(e) = shell.save_tab(ctx, id) {
                        let _ = gz_ui::dialog::notice(ctx, shell.asker(), Tone::Error, "Save failed", &e);
                        return;
                    }
                }
                close(shell.editor.clone());
            });
        });
        if let Some(line) = line {
            self.editor.log.plain(line);
        }
    }

    pub(crate) fn reset_layout(&mut self, ctx: &Context, all: bool) {
        if all {
            gz_ui::layout::reset_all(ctx);
        }
        if let Some(f) = self.parts.reset_layout.as_mut() {
            f(
                all,
                &mut DocCx {
                    editor: &self.editor,
                    gpu: &self.gpu,
                    ctx,
                    active_tab: &self.active,
                    requests: &mut self.requests,
                },
            );
        }
    }

    // ---- the tab strip -------------------------------------------------------------------

    fn draw_tab_strip(&mut self, row: &mut Ui, bar: Rect, max_x: f32) -> f32 {
        let ctx = row.ctx().clone();
        let p = palette(&ctx);
        let font = Font::Body.id();
        let tab_h = 28.0_f32.min(bar.height());
        let y0 = bar.max.y - tab_h;
        let x_start = row.cursor().min.x + 8.0;
        let avail = (max_x - x_start - tabs::CHEVRON - 4.0).max(0.0);
        let labels: Vec<String> = self.tabs.iter().map(Tab::label).collect();
        let want: Vec<f32> = labels.iter().map(|l| tabs::want_width(text::width(&ctx, &font, l))).collect();
        let active_i = self.find(&self.active);
        let more_font = Font::Small.id();
        let count_extra = (text::width(&ctx, &more_font, "+99") + 16.0 + 16.0 - tabs::CHEVRON).max(0.0);
        let plan = tabs::plan_strip(&want, active_i, avail, count_extra);

        // names that would still cut to the same text are cut in the middle
        let cut_end: Vec<Option<String>> = (0..self.tabs.len())
            .map(|i| {
                let shown = plan.shown.get(i).copied().unwrap_or(false);
                let tab = self.tabs.get(i)?;
                let w = plan.widths.get(i).copied().unwrap_or(0.0);
                let cross = tab.closable() && Some(i) == active_i;
                shown.then(|| {
                    text::ellipsize(
                        &ctx,
                        &font,
                        labels.get(i).map_or("", String::as_str),
                        tabs::text_room(w, cross, tab.dirty()),
                        Clip::End,
                    )
                    .0
                })
            })
            .collect();

        let mut x = x_start;
        let mut activate = None;
        let mut close = None;
        let mut menu = None;
        let pointer = ctx.pointer_latest_pos();
        for i in 0..self.tabs.len() {
            let shown = plan.shown.get(i).copied().unwrap_or(false);
            let Some(tab) = self.tabs.get_mut(i) else { continue };
            if !shown {
                tab.rect = None;
                continue;
            }
            let w = plan.widths.get(i).copied().unwrap_or(tabs::W_MIN);
            let rect =
                Rect::from_min_max(pos2(x.round(), y0.round()), pos2((x + w).round(), bar.max.y.round()));
            x += w + tabs::SPACING;
            tab.rect = Some(rect);
            let id = Id::unique(("gz-app.tab", tab.id.as_str()));
            let resp = row.interact(rect, id, Sense::click_and_drag());
            let active = Some(i) == active_i;
            // the close cross over it takes the hover; the tab still looks hovered
            let hovered = resp.contains_pointer();
            if resp.clicked() {
                activate = Some(tab.id.clone());
            }
            if resp.middle_clicked() && tab.closable() {
                close = Some(tab.id.clone());
            }
            if resp.secondary_clicked() {
                menu = Some(tab.id.clone());
            }
            if resp.dragged() && tab.id != LEVEL {
                self.drag_tab = Some(tab.id.clone());
            }
            let dirty = tab.dirty();
            let slot = Rect::from_min_max(
                pos2(rect.max.x - tabs::PAD_R - tabs::SLOT, rect.min.y + (tab_h - tabs::SLOT) * 0.5),
                pos2(rect.max.x - tabs::PAD_R, rect.min.y + (tab_h + tabs::SLOT) * 0.5),
            );
            let show_close = tab.closable() && (hovered || active);
            let mut slot_hovered = false;
            if show_close {
                let c = row.interact(slot, id.with("close"), Sense::click());
                slot_hovered = c.hovered();
                if c.clicked() {
                    close = Some(tab.id.clone());
                }
            }
            let painter = row.painter();
            if active || hovered {
                painter.rect_filled(
                    rect,
                    CornerRadius { nw: 4, ne: 4, sw: 0, se: 0 },
                    if active { p.base } else { p.hover },
                );
            }
            if active {
                painter.rect_filled(
                    Rect::from_min_max(
                        pos2(rect.min.x + 1.0, rect.min.y),
                        pos2(rect.max.x - 1.0, rect.min.y + 2.0),
                    ),
                    CornerRadius::ZERO,
                    p.accent_bright,
                );
            }
            let icon_rect = Rect::from_min_size(
                pos2((rect.min.x + tabs::PAD_L).round(), (rect.min.y + (tab_h - tabs::ICON) * 0.5).round()),
                Vec2::splat(tabs::ICON),
            );
            icons::draw(
                painter,
                tab.icon,
                icon_rect,
                if active { p.accent_bright } else { p.text_secondary },
            );
            let tx = rect.min.x + tabs::PAD_L + tabs::ICON + tabs::GAP;
            let room = tabs::text_room(rect.width(), show_close, dirty);
            let label = labels.get(i).map_or("", String::as_str);
            let middle = cut_end.iter().enumerate().any(|(j, c)| {
                j != i
                    && c.is_some()
                    && *c == cut_end.get(i).cloned().flatten()
                    && labels.get(j).map(String::as_str) != Some(label)
            });
            let cut = text::paint(
                painter,
                &font,
                pos2(tx, rect.center().y),
                Align2::LEFT_CENTER,
                if active { p.text } else { p.text_secondary },
                label,
                Some(room),
                if middle { Clip::Middle } else { Clip::End },
            );
            if show_close {
                if slot_hovered {
                    painter.rect_filled(slot, CornerRadius::same(3), p.pressed);
                }
                icons::draw(
                    painter,
                    Icon::Close,
                    Rect::from_center_size(slot.center(), Vec2::splat(12.0)),
                    if slot_hovered { p.text } else { p.text_secondary },
                );
            } else if dirty {
                painter.circle_filled(slot.center(), 3.5, if tab.id == LEVEL { p.warning } else { p.text });
            }
            if cut {
                audit::truncated(&ctx, label, &text::ellipsize(&ctx, &font, label, room, Clip::End).0, rect);
            }
            audit::control(&ctx, label, true, None, Some("a major tab"), rect);
            let tip = tab.tooltip();
            let _ = resp.on_hover_text(tip);
        }

        // the drag: over the middle of a neighbour, the two swap; Level keeps the first place
        if let Some(drag) = self.drag_tab.clone() {
            if !ctx.input(|i| i.pointer.primary_down()) {
                self.drag_tab = None;
            } else if let (Some(mx), Some(a)) = (pointer.map(|p| p.x), self.find(&drag)) {
                for b in 1..self.tabs.len() {
                    if b == a {
                        continue;
                    }
                    let Some(mid) = self.tabs.get(b).and_then(|t| t.rect).map(|r| r.center().x) else {
                        continue;
                    };
                    if (a < b && mx > mid) || (a > b && mx < mid) {
                        self.tabs.swap(a, b);
                        self.mark_session_dirty();
                        break;
                    }
                }
            }
        }

        // the chevron: every tab, those under it among them
        let hidden = plan.hidden();
        let overflow = Id::unique("gz-app.tab_overflow");
        let open = Popup::is_id_open(&ctx, overflow);
        let chevron_rect;
        {
            let label = format!("+{hidden}");
            let w =
                if hidden > 0 { text::width(&ctx, &more_font, &label) + 16.0 + 16.0 } else { tabs::CHEVRON };
            let r = Rect::from_min_size(
                pos2((x + 2.0).round(), (y0 + (tab_h - tabs::CHEVRON) * 0.5).round()),
                Vec2::new(w, tabs::CHEVRON),
            );
            chevron_rect = r;
            let tip = match hidden {
                0 => "Open tabs".to_owned(),
                1 => "1 more tab".to_owned(),
                n => format!("{n} more tabs"),
            };
            let resp = row.interact(r, Id::unique("gz-app.tab_overflow_button"), Sense::click());
            let painter = row.painter();
            if open || resp.hovered() {
                painter.rect_filled(
                    r,
                    CornerRadius::same(4),
                    if open { p.selection_focused } else { p.hover },
                );
            }
            if hidden > 0 {
                painter.text(
                    pos2(r.min.x + 6.0, r.center().y),
                    Align2::LEFT_CENTER,
                    &label,
                    more_font.clone(),
                    p.text_secondary,
                );
                icons::draw(
                    painter,
                    Icon::ChevronDown,
                    Rect::from_min_size(pos2(r.max.x - 18.0, r.center().y - 6.0), Vec2::splat(12.0)),
                    p.text_secondary,
                );
            } else {
                icons::draw(
                    painter,
                    Icon::ChevronDown,
                    Rect::from_center_size(r.center(), Vec2::splat(12.0)),
                    p.text_secondary,
                );
            }
            audit::control(&ctx, &tip, true, None, Some(&tip), r);
            if resp.on_hover_text(&tip).clicked() {
                Popup::toggle_id(&ctx, overflow);
            }
            x = r.max.x;
        }
        let layer = row.layer_id();
        let entries: Vec<(SmolStr, String, Icon, bool)> =
            self.tabs.iter().map(|t| (t.id.clone(), t.label(), t.icon, t.dirty())).collect();
        let current = self.active.clone();
        let mut picked = None;
        if menu_popup(&ctx, overflow, chevron_rect, layer, |ui| {
            for (id, label, icon, dirty) in &entries {
                let text = if *dirty { format!("{label} *") } else { label.clone() };
                if ui
                    .push_id(id.as_str(), |ui| {
                        menu_item(
                            ui,
                            &text,
                            MenuItem { icon: *icon, selected: *id == current, ..MenuItem::default() },
                        )
                    })
                    .inner
                {
                    picked = Some(id.clone());
                }
            }
        })
        .is_some()
        {
            let rect = ctx.memory(|m| m.area_rect(overflow)).unwrap_or(chevron_rect);
            self.note_popup("tab_overflow", "Open tabs", "##tab_overflow_popup", rect);
        }
        if let Some(id) = picked {
            activate = Some(id);
        }

        // the tab menu
        let menu_popup_id = Id::unique("gz-app.tab_menu");
        if let Some(id) = menu {
            self.tab_menu = Some(id);
            Popup::open_id(&ctx, menu_popup_id);
        }
        if let Some(target) = self.tab_menu.clone() {
            let found =
                self.find(&target).and_then(|i| self.tabs.get(i)).map(|t| (t.rect, t.closable(), t.path()));
            match found {
                None => {
                    self.tab_menu = None;
                    Popup::close_id(&ctx, menu_popup_id);
                }
                Some((rect, closable, path)) => {
                    let anchor = rect.unwrap_or(chevron_rect);
                    let mut action = None;
                    let has_reveal = self.parts.reveal.is_some();
                    let shown = menu_popup(&ctx, menu_popup_id, anchor, layer, |ui| {
                        if menu_item(
                            ui,
                            "Close",
                            MenuItem { keys: "Ctrl+F4", enabled: closable, ..MenuItem::default() },
                        ) {
                            action = Some(1);
                        }
                        if menu_item(ui, "Close Others", MenuItem::default()) {
                            action = Some(2);
                        }
                        if menu_item(ui, "Close All", MenuItem::default()) {
                            action = Some(3);
                        }
                        if menu_item(ui, "Close Saved", MenuItem::default()) {
                            action = Some(4);
                        }
                        ui.separator();
                        if menu_item(
                            ui,
                            "Copy Path",
                            MenuItem { icon: Icon::Copy, enabled: !path.is_empty(), ..MenuItem::default() },
                        ) {
                            action = Some(5);
                        }
                        if menu_item(
                            ui,
                            "Show in Content Browser",
                            MenuItem {
                                icon: Icon::Folder,
                                enabled: !path.is_empty() && has_reveal,
                                ..MenuItem::default()
                            },
                        ) {
                            action = Some(6);
                        }
                        if menu_item(
                            ui,
                            "Show in Explorer",
                            MenuItem { icon: Icon::Reveal, enabled: !path.is_empty(), ..MenuItem::default() },
                        ) {
                            action = Some(7);
                        }
                    });
                    if shown.is_some() {
                        let r = ctx.memory(|m| m.area_rect(menu_popup_id)).unwrap_or(anchor);
                        self.note_popup(&format!("tab_menu:{target}"), "Tab", "##tab_menu_popup", r);
                    } else if action.is_none() {
                        self.tab_menu = None;
                    }
                    if let Some(a) = action {
                        self.tab_menu = None;
                        self.tab_menu_action(&ctx, &target, &path, a);
                    }
                }
            }
        }

        if let Some(id) = activate {
            self.activate(&ctx, &id);
        }
        if let Some(id) = close {
            self.request_close(&ctx, &id);
        }
        x
    }

    fn tab_menu_action(&mut self, ctx: &Context, target: &str, path: &str, action: u8) {
        let ids: Vec<(SmolStr, bool, bool)> =
            self.tabs.iter().map(|t| (t.id.clone(), t.closable(), t.dirty())).collect();
        match action {
            1 => self.request_close(ctx, target),
            2 => ids
                .iter()
                .filter(|(id, c, _)| *c && id != target)
                .for_each(|(id, _, _)| self.request_close(ctx, id)),
            3 => ids.iter().filter(|(_, c, _)| *c).for_each(|(id, _, _)| self.request_close(ctx, id)),
            4 => ids.iter().filter(|(_, c, d)| *c && !*d).for_each(|(id, _, _)| self.request_close(ctx, id)),
            5 => ctx.copy_text(path.to_owned()),
            6 => {
                if let Some(f) = self.parts.reveal.as_mut() {
                    f(
                        path,
                        &mut DocCx {
                            editor: &self.editor,
                            gpu: &self.gpu,
                            ctx,
                            active_tab: &self.active,
                            requests: &mut self.requests,
                        },
                    );
                }
            }
            7 => {
                let full = match self.editor.project_root() {
                    Some(root) if !std::path::Path::new(path).is_absolute() => root.join(path),
                    _ => std::path::PathBuf::from(path),
                };
                reveal_file(&full);
            }
            _ => {}
        }
    }

    // ---- the toolbar ---------------------------------------------------------------------

    fn draw_toolbar(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.base);
        ui.painter().line_segment(
            [pos2(rect.min.x, rect.max.y - 0.5), pos2(rect.max.x, rect.max.y - 0.5)],
            Stroke::new(1.0, p.border_subtle),
        );
        let inner =
            Rect::from_min_max(pos2(rect.min.x + 8.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y));
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-app.toolbar")
                .max_rect(inner)
                .layout(Layout::left_to_right(Align::Center)),
        );
        let id = self.active.to_string();
        let project = self.editor.project_opt().is_some();
        let on_prefs = self.active == "preferences";
        let i = self.find(&id);
        let ShellState { tabs, parts, editor, gpu, active, requests, .. } = self;
        audit::surface(&ctx, "##toolbar", || {
            let mut tb = Toolbar::begin(&mut child, &id, ToolbarStyle::Row);
            if let Some(tab) = i.and_then(|i| tabs.get_mut(i))
                && !tab.failed
                && tab.panicked.is_none()
                && let Some(doc) = tab.doc.as_mut()
                && !doc.missing()
            {
                doc.toolbar(&mut tb, &mut DocCx { editor, gpu, ctx: &ctx, active_tab: active, requests });
            }
            tb.spring();
            tb.group(100);
            // nothing of a project is edited in the Preferences: they have nothing to build
            if project && !on_prefs {
                for (_, f) in &mut parts.project_toolbar {
                    f(&mut tb, &mut DocCx { editor, gpu, ctx: &ctx, active_tab: active, requests });
                }
            }
            tb.end();
        });
    }

    // ---- the status bar ------------------------------------------------------------------

    fn draw_status(&mut self, ui: &mut Ui, rect: Rect) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(rect, CornerRadius::ZERO, p.chrome);
        ui.painter().line_segment(
            [pos2(rect.min.x, rect.min.y + 0.5), pos2(rect.max.x, rect.min.y + 0.5)],
            Stroke::new(1.0, p.border_subtle),
        );
        let log = self.log_state();
        let small = Font::Small.id();

        // right: the chips, Tasks, the counts, the MCP dot - right to left
        let right_rect =
            Rect::from_min_max(pos2(rect.min.x + 4.0, rect.min.y), pos2(rect.max.x - 8.0, rect.max.y));
        let mut right = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-app.status_right")
                .max_rect(right_rect)
                .layout(Layout::right_to_left(Align::Center)),
        );
        right.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
        local_status_chip(
            &mut right,
            Icon::Mcp,
            Some(Tone::Success),
            "",
            "MCP is on: agents drive the editor over 127.0.0.1.",
            false,
        );
        let log_open = self.drawer == Some(Drawer::Log);
        let warnings_tip =
            format!("{} warnings since the project opened. Opens the Output Log.", log.warnings);
        if local_status_chip(
            &mut right,
            Icon::Warning,
            Some(if log.warnings > 0 { Tone::Warning } else { Tone::Muted }),
            &log.warnings.to_string(),
            &warnings_tip,
            false,
        )
        .clicked()
        {
            self.toggle_drawer(&ctx, Drawer::Log);
        }
        let errors_tip = format!("{} errors since the project opened. Opens the Output Log.", log.errors);
        if local_status_chip(
            &mut right,
            Icon::Error,
            Some(if log.errors > 0 { Tone::Error } else { Tone::Muted }),
            &log.errors.to_string(),
            &errors_tip,
            false,
        )
        .clicked()
        {
            self.toggle_drawer(&ctx, Drawer::Log);
        }
        let running: Vec<gz_editor::JobInfo> =
            self.editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running).collect();
        if !running.is_empty() {
            let label = match running.as_slice() {
                [one] => one.title.clone(),
                many => format!("{} tasks", many.len()),
            };
            let shown = text::ellipsize(&ctx, &small, &label, gz_ui::units::em(18.0), Clip::End).0;
            if local_status_chip(&mut right, Icon::Spinner, None, &shown, &label, self.card_open).clicked() {
                self.card_open = !self.card_open;
            }
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        {
            let ShellState { parts, editor, gpu, active, requests, .. } = self;
            for (_, chip) in parts.status_chips.iter_mut().rev() {
                chip(&mut right, &mut DocCx { editor, gpu, ctx: &ctx, active_tab: active, requests });
            }
        }
        let right_x = right.min_rect().min.x.min(right_rect.max.x);

        // left: the drawers
        let mut left = ui.new_child(
            UiBuilder::new()
                .id_salt("gz-app.status_left")
                .max_rect(Rect::from_min_max(pos2(rect.min.x + 4.0, rect.min.y), pos2(right_x, rect.max.y)))
                .layout(Layout::left_to_right(Align::Center)),
        );
        left.spacing_mut().item_spacing = Vec2::new(4.0, 0.0);
        for (which, label, icon, tip) in [
            (Drawer::Content, "Content Drawer", Icon::Drawer, "Open the Content Browser over this tab"),
            (Drawer::Log, "Output Log", Icon::Log, "Open the Output Log over this tab"),
        ] {
            if !self.has_drawer(which) {
                continue;
            }
            if local_status_chip(
                &mut left,
                icon,
                None,
                label,
                tip,
                self.drawer == Some(which) || (which == Drawer::Log && log_open),
            )
            .clicked()
            {
                self.toggle_drawer(&ctx, which);
            }
        }
        let left_end = left.min_rect().max.x.max(rect.min.x + 4.0) + 4.0;

        // centre: what the tab says; the log's last line fills the gap before it
        let room_x1 = left_end.max(right_x - 12.0);
        let mut status_x = room_x1;
        self.status_centre = None;
        let id = self.active.to_string();
        // the centre is the active document's: the Start page (or a card over it) has none
        if self.menus_shown
            && let Some(i) = self.find(&id)
            && room_x1 - left_end > 24.0
        {
            let want = ctx.data(|d| d.get_temp::<f32>(Id::unique("gz-app.status_centre_w"))).unwrap_or(0.0);
            // centred where the room allows; ImClamp's order: the low bound wins a tie
            let centred = (rect.min.x + (rect.width() - want) * 0.5).round();
            let (lo, hi) = (left_end + 160.0, (room_x1 - want).max(left_end));
            status_x = (if centred < lo { lo } else { centred.min(hi) }).min(room_x1);
            let area = Rect::from_min_max(pos2(status_x, rect.min.y), pos2(room_x1, rect.max.y));
            let mut centre = ui.new_child(
                UiBuilder::new()
                    .id_salt("gz-app.status_centre")
                    .max_rect(area)
                    .layout(Layout::left_to_right(Align::Center)),
            );
            centre.set_clip_rect(area);
            centre.style_mut().override_font_id = Some(small.clone());
            if let Some(tab) = self.tabs.get_mut(i)
                && !tab.failed
                && tab.panicked.is_none()
                && let Some(doc) = tab.doc.as_mut()
                && !doc.missing()
            {
                doc.status(
                    &mut centre,
                    &mut DocCx {
                        editor: &self.editor,
                        gpu: &self.gpu,
                        ctx: &ctx,
                        active_tab: &self.active,
                        requests: &mut self.requests,
                    },
                );
            }
            let used = (centre.min_rect().max.x - status_x).max(0.0);
            ctx.data_mut(|d| d.insert_temp(Id::unique("gz-app.status_centre_w"), used));
            if used <= 0.0 {
                status_x = room_x1;
            } else {
                self.status_centre =
                    Some(Rect::from_min_max(pos2(status_x, rect.min.y), pos2(status_x + used, rect.max.y)));
            }
        }
        let line_x1 = status_x - 16.0;
        if !log.last.is_empty() && line_x1 - left_end > 40.0 {
            text::paint(
                ui.painter(),
                &small,
                pos2(left_end.round(), rect.center().y),
                Align2::LEFT_CENTER,
                p.text_muted,
                &log.last,
                Some(line_x1 - left_end),
                Clip::End,
            );
        }
    }

    // ---- the work area -------------------------------------------------------------------

    fn draw_work(&mut self, ui: &mut Ui, work: Rect) {
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        ui.painter().rect_filled(work, CornerRadius::ZERO, p.base);
        let mut host = ui.new_child(
            UiBuilder::new().id_salt("###doc.host").max_rect(work).layout(Layout::top_down(Align::Min)),
        );
        host.set_clip_rect(work);

        if self.iconsheet {
            audit::surface(&ctx, "###doc.host", || {
                egui::ScrollArea::vertical()
                    .id_salt("gz-app.iconsheet")
                    .auto_shrink([false, false])
                    .show(&mut host, |ui| icons::sheet(ui, 0));
            });
            self.tick_inactive(&ctx);
            return;
        }
        if !self.menus_shown {
            // the Start page, or a card that takes it over (Link Game, Recovery)
            let editor = self.editor.clone();
            let card = self.parts.page_cards.iter().position(|c| (c.wants)(&editor));
            let ShellState { parts, gpu, active, requests, .. } = self;
            let mut cx = DocCx { editor: &editor, gpu, ctx: &ctx, active_tab: active, requests };
            audit::surface(&ctx, contract::WINDOW_START_PAGE, || match (card, parts.start_page.as_mut()) {
                (Some(c), _) => {
                    if let Some(card) = parts.page_cards.get_mut(c) {
                        (card.draw)(&mut host, &mut cx);
                    }
                }
                (None, Some(draw)) => draw(&mut host, &mut cx),
                (None, None) => {
                    gz_ui::card::page(&mut host, contract::WINDOW_START_PAGE, |ui| {
                        let text_ = if editor.project_opt().is_some() {
                            "Link the game this project is for: game_link over MCP."
                        } else {
                            "Open a project with open_project over MCP, or start the editor with -project <folder>."
                        };
                        state::empty_state(ui, Icon::Folder, "No project is open", text_, None);
                    });
                }
            });
            self.tick_inactive(&ctx);
            return;
        }

        let id = self.active.to_string();
        let Some(i) = self.find(&id) else { return };
        let (padded, missing, panicked, loaded, path, label) = match self.tabs.get(i) {
            Some(t) => (
                t.doc.as_ref().is_none_or(|d| d.padded()) || t.missing(),
                t.missing(),
                t.panicked.clone(),
                t.loaded(),
                t.path(),
                t.label(),
            ),
            None => return,
        };
        let body_rect = if padded && !missing { work.shrink(8.0) } else { work };
        let mut body = host.new_child(
            UiBuilder::new()
                .id_salt(("gz-app.doc", id.as_str()))
                .max_rect(body_rect)
                .layout(Layout::top_down(Align::Min)),
        );
        body.set_clip_rect(body_rect);
        audit::surface(&ctx, "###doc.host", || {
            if let Some(text_) = panicked {
                if state::empty_state(
                    &mut body,
                    Icon::Error,
                    "This tab stopped on an internal error",
                    &text_,
                    Some("Close Tab"),
                ) {
                    self.requests.close(&id);
                    if let Some(tab) = self.tabs.get_mut(i) {
                        tab.panicked = None;
                        tab.failed = true;
                    }
                }
            } else if missing {
                state::empty_state(
                    &mut body,
                    Icon::File,
                    "File not found",
                    if path.is_empty() { &label } else { &path },
                    None,
                );
            } else if loaded {
                self.draw_body(&mut body, i);
            }
        });
        self.tick_inactive(&ctx);
    }

    // ---- drawers -------------------------------------------------------------------------

    fn draw_drawer(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        // one host a frame: a drawer whose panel is on screen gives way to it
        if let Some(d) = self.drawer
            && self.drawer_panel_visible(d)
        {
            self.drawer = None;
        }
        let Some(which) = self.drawer else {
            self.drawer_rect = None;
            return;
        };
        if self.start_page || self.work_rect.width() <= 0.0 {
            self.drawer_rect = None;
            return;
        }
        let p = palette(&ctx);
        let work = self.work_rect;
        let h = (work.height() * 0.4).clamp(260.0, 480.0).min(work.height());
        let rect = Rect::from_min_max(pos2(work.min.x, work.max.y - h), work.max);
        self.drawer_rect = Some(rect);
        let mut close = false;
        let ShellState { parts, editor, gpu, active, requests, drawer_focus, .. } = self;
        let Some(desc) = parts.drawers.get_mut(which.index()).and_then(|d| d.as_mut()) else { return };
        let window = which.window();
        let resp = Area::new(Id::unique(window))
            .order(Order::Middle)
            .fixed_pos(rect.min)
            .movable(false)
            .constrain(false)
            .show(&ctx, |ui| {
                let (r, _) = ui.allocate_exact_size(rect.size(), Sense::click());
                ui.painter().rect_filled(r, CornerRadius::ZERO, p.base);
                ui.painter().line_segment([r.left_top(), r.right_top()], Stroke::new(1.0, p.border_strong));
                let head = Rect::from_min_size(r.min, Vec2::new(r.width(), 32.0));
                icons::draw(
                    ui.painter(),
                    desc.icon,
                    Rect::from_min_size(pos2(head.min.x + 8.0, head.center().y - 8.0), Vec2::splat(16.0)),
                    p.text_secondary,
                );
                ui.painter().text(
                    pos2(head.min.x + 32.0, head.center().y),
                    Align2::LEFT_CENTER,
                    desc.label.as_str(),
                    Font::Strong.id(),
                    p.text,
                );
                let cross = Rect::from_min_size(
                    pos2(head.max.x - 16.0 - 24.0, head.center().y - 12.0),
                    Vec2::splat(24.0),
                );
                let mut cell = ui.new_child(UiBuilder::new().max_rect(cross));
                if icon_button_sized(
                    &mut cell,
                    Icon::Close,
                    "Close the drawer (Esc)",
                    false,
                    true,
                    24.0,
                    16.0,
                )
                .clicked()
                {
                    close = true;
                }
                let body_rect = Rect::from_min_max(pos2(r.min.x + 8.0, head.max.y), r.max);
                let mut body = ui.new_child(
                    UiBuilder::new()
                        .id_salt(("gz-app.drawer_body", window))
                        .max_rect(body_rect)
                        .layout(Layout::top_down(Align::Min)),
                );
                body.set_clip_rect(body_rect);
                audit::surface(&ctx, window, || {
                    (desc.body)(
                        &mut body,
                        &mut DocCx { editor, gpu, ctx: &ctx, active_tab: active, requests },
                    )
                });
            });
        *drawer_focus = false;
        let _ = resp;
        // Esc, a click outside it (the status bar's own buttons toggle it), or a drag that leaves it
        let pointer = ctx.pointer_latest_pos();
        let outside = pointer.is_some_and(|m| !rect.contains(m) && !self.status_rect.contains(m));
        if !ctx.egui_wants_keyboard_input()
            && !self.modal_up(&ctx)
            && ctx.input(|i| i.key_pressed(Key::Escape))
        {
            close = true;
        }
        if outside
            && !Popup::is_any_open(&ctx)
            && ctx.input(|i| i.pointer.primary_pressed() || i.pointer.secondary_pressed())
        {
            close = true;
        }
        if outside && egui::DragAndDrop::has_any_payload(&ctx) {
            close = true;
        }
        if close {
            self.drawer = None;
        }
    }

    fn draw_dialogs(&mut self, _ui: &mut Ui) {
        let ctx = _ui.ctx().clone();
        let ShellState { parts, editor, gpu, active, requests, .. } = self;
        // a dialog of the project goes down with it: over the Start page there is none to act on
        let no_project = editor.project_opt().is_none();
        for d in &mut parts.dialogs {
            if no_project
                && d.needs_project
                && let Some((_, name)) = d.window.split_once("###")
                && gz_ui::dialog::open_dialogs(&ctx).iter().any(|o| o.id == name)
            {
                gz_ui::dialog::close_dialog(&ctx, name);
            }
            (d.draw)(_ui, &mut DocCx { editor, gpu, ctx: &ctx, active_tab: active, requests });
        }
    }

    // ---- the Task Center's card ----------------------------------------------------------

    fn draw_card(&mut self, ui: &mut Ui) {
        let ctx = ui.ctx().clone();
        self.card_rect = None;
        let running: Vec<gz_editor::JobInfo> =
            self.editor.jobs.snapshot().into_iter().filter(gz_editor::JobInfo::running).collect();
        // a dialog is the author's work now: the card waits behind it
        if running.is_empty() || !self.card_open || self.modal_up(&ctx) {
            return;
        }
        let p = palette(&ctx);
        let width = 400.0;
        let corner = self.work_rect.right_bottom() - Vec2::splat(16.0);
        let now = crate::tasks::now_ms();
        let mut stop = None;
        let mut hide = false;
        let resp = Area::new(Id::unique(contract::TASK_CARD))
            .order(Order::Foreground)
            .pivot(Align2::RIGHT_BOTTOM)
            .fixed_pos(corner)
            .show(&ctx, |ui| {
                Frame::new()
                    .fill(p.overlay)
                    .stroke(Stroke::new(1.0, p.border_strong))
                    .corner_radius(CornerRadius::same(8))
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_width(width - 28.0);
                        ui.spacing_mut().item_spacing = Vec2::new(8.0, 2.0);
                        let room = width - 28.0;
                        for (n, job) in running.iter().enumerate() {
                            if n > 0 {
                                ui.add_space(6.0);
                                ui.separator();
                                ui.add_space(6.0);
                            }
                            let card = crate::tasks::card_of(&self.editor, job.id).unwrap_or_default();
                            let cancel_requested = self.cancelled.contains(&job.id);
                            if card_row(ui, job, &card, room, n == 0, cancel_requested, now, &mut hide) {
                                stop = Some(job.id);
                            }
                        }
                    });
            });
        self.card_rect = Some(resp.response.rect);
        self.note_popup("tasks", "Tasks", contract::TASK_CARD, resp.response.rect);
        ctx.request_repaint_after(Duration::from_millis(50));
        if hide {
            self.card_open = false;
        }
        if let Some(id) = stop {
            self.job_cancel(id);
        }
    }

    // ---- the Command Palette -------------------------------------------------------------

    /// Every row the palette offers: the open tabs, the Level panels, the drawers, the commands.
    pub(crate) fn palette_rows(&self) -> Vec<PaletteRow> {
        let mut rows = Vec::new();
        for t in &self.tabs {
            let path = t.path();
            rows.push(PaletteRow {
                id: format!("tab:{}", t.id),
                kind: "tab",
                label: t.label(),
                category: "Tabs".to_owned(),
                path: if path.is_empty() { "Tab".to_owned() } else { format!("Tab > {path}") },
                name: format!("tab:{}", t.id),
                shortcut: String::new(),
                icon: if t.icon == Icon::None { Icon::File } else { t.icon },
                command: None,
                score: 0,
            });
        }
        for panel in &self.parts.panels {
            rows.push(PaletteRow {
                id: format!("panel:{}", panel.id),
                kind: "panel",
                label: panel.label.to_string(),
                category: "Panels".to_owned(),
                path: "Window > Panel".to_owned(),
                name: format!("panel:{}", panel.id),
                shortcut: String::new(),
                icon: panel.icon,
                command: None,
                score: 0,
            });
        }
        for (which, label, keys) in
            [(Drawer::Content, "Content Drawer", "Ctrl+Space"), (Drawer::Log, "Log Drawer", "Ctrl+`")]
        {
            if !self.has_drawer(which) {
                continue;
            }
            rows.push(PaletteRow {
                id: format!("drawer:{}", which.id()),
                kind: "drawer",
                label: label.to_owned(),
                category: "Drawers".to_owned(),
                path: "Window > Drawer".to_owned(),
                name: format!("drawer:{}", which.id()),
                shortcut: keys.to_owned(),
                icon: if which == Drawer::Log { Icon::Log } else { Icon::Drawer },
                command: None,
                score: 0,
            });
        }
        for (i, c) in self.parts.commands.iter().enumerate() {
            if !c.in_palette {
                continue;
            }
            rows.push(PaletteRow {
                id: c.id.to_string(),
                kind: "command",
                label: c.label.to_string(),
                category: c.group.to_string(),
                path: format!("{} > {}", c.group, c.label),
                name: c.id.split('.').next().unwrap_or(&c.id).to_owned(),
                shortcut: self.binding(i).map(|k| k.label()).unwrap_or_default(),
                icon: c.icon,
                command: Some(i),
                score: 0,
            });
        }
        rows
    }

    /// Runs a palette row by its id: `panel:<id>`, `drawer:<id>`, `tab:<id>` or a command's.
    pub(crate) fn palette_execute(&mut self, ctx: &Context, id: &str) -> bool {
        if let Some(panel) = id.strip_prefix("panel:") {
            return self.panel(ctx, panel, PanelOp::Focus).is_some();
        }
        if let Some(d) = id.strip_prefix("drawer:") {
            return Drawer::parse(d).is_some_and(|d| self.open_drawer(d));
        }
        if let Some(tab) = id.strip_prefix("tab:") {
            return self.activate(ctx, tab);
        }
        match self.command_index(id) {
            Some(i) if self.parts.commands.get(i).is_some_and(|c| c.in_palette) => self.run_command(ctx, i),
            _ => false,
        }
    }

    fn draw_palette(&mut self, ui: &mut Ui) {
        if !self.palette.open {
            return;
        }
        let ctx = ui.ctx().clone();
        let p = palette(&ctx);
        let work = self.work_rect;
        let width = 720.0_f32.min(work.width() * 0.7);
        let rows = pal::rank(self.palette_rows(), &self.palette.query, 250);
        let count = rows.len();
        if count > 0 {
            self.palette.selected = self.palette.selected.min(count - 1);
        }
        // the field never moves; the list ends under its last row, up to 480 pt in all
        let row_h = 30.0;
        let head_h = 12.0 + 32.0 + 8.0;
        let footer_h = 32.0;
        let list_h = if count == 0 { 168.0 } else { count as f32 * row_h + 14.0 };
        let height = (head_h + row_h + 12.0 + footer_h)
            .max((head_h + list_h + footer_h).min(480.0_f32.min(work.height() - 40.0 - 24.0)));
        let pos = pos2((work.center().x - width * 0.5).round(), work.min.y + 40.0);
        self.palette.rect = Rect::from_min_size(pos, Vec2::new(width, height));

        // keys before the field takes them
        let (esc, enter, up, down, page_up, page_down) = ctx.input_mut(|i| {
            (
                i.consume_key(Modifiers::NONE, Key::Escape),
                i.consume_key(Modifiers::NONE, Key::Enter),
                i.consume_key(Modifiers::NONE, Key::ArrowUp),
                i.consume_key(Modifiers::NONE, Key::ArrowDown),
                i.consume_key(Modifiers::NONE, Key::PageUp),
                i.consume_key(Modifiers::NONE, Key::PageDown),
            )
        });
        if count > 0 {
            let s = self.palette.selected;
            let moved = if down {
                Some((s + 1) % count)
            } else if up {
                Some((s + count - 1) % count)
            } else if page_down {
                Some((s + 10).min(count - 1))
            } else if page_up {
                Some(s.saturating_sub(10))
            } else {
                None
            };
            if let Some(m) = moved {
                self.palette.selected = m;
                self.palette.scroll_to_selected = true;
            }
        }
        let mut run = if enter && count > 0 { Some(self.palette.selected) } else { None };
        let id = Id::unique(contract::PALETTE_ID);
        let area =
            Area::new(id).kind(egui::UiKind::Modal).order(Order::Foreground).fixed_pos(pos).constrain(false);
        let query_before = self.palette.query.clone();
        let focus = std::mem::take(&mut self.palette.focus);
        let scroll_to = std::mem::take(&mut self.palette.scroll_to_selected);
        let selected = self.palette.selected;
        let mut query = std::mem::take(&mut self.palette.query);
        let mut clicked = None;
        let modal =
            egui::Modal::new(id).area(area).backdrop_color(p.dim).frame(Frame::NONE).show(&ctx, |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
                ui.painter().rect(
                    rect,
                    CornerRadius::same(8),
                    p.overlay,
                    Stroke::new(1.0, p.border_strong),
                    StrokeKind::Inside,
                );
                audit::surface(&ctx, contract::PALETTE_WINDOW, || {
                    let field =
                        Rect::from_min_size(rect.min + Vec2::new(12.0, 12.0), Vec2::new(width - 24.0, 32.0));
                    let mut field_ui = ui.new_child(
                        UiBuilder::new()
                            .id_salt("gz-app.palette_field")
                            .max_rect(field)
                            .layout(Layout::left_to_right(Align::Center)),
                    );
                    let edit_id = field_ui.make_persistent_id(("gz-ui.search", "palette")).with("edit");
                    if focus {
                        ctx.memory_mut(|m| m.request_focus(edit_id));
                    }
                    SearchField::new(&mut query)
                        .hint("Run a command, open a panel or go to a tab")
                        .id_salt("palette")
                        .width(width - 24.0)
                        .show(&mut field_ui);
                    let line_y = field.max.y + 8.0;
                    ui.painter().line_segment(
                        [pos2(rect.min.x, line_y), pos2(rect.max.x, line_y)],
                        Stroke::new(1.0, p.border_subtle),
                    );
                    let list = Rect::from_min_max(
                        pos2(rect.min.x + 6.0, line_y + 6.0),
                        pos2(rect.max.x - 6.0, rect.max.y - footer_h - 6.0),
                    );
                    let mut list_ui = ui.new_child(
                        UiBuilder::new()
                            .id_salt("gz-app.palette_list")
                            .max_rect(list)
                            .layout(Layout::top_down_justified(Align::Min)),
                    );
                    if rows.is_empty() {
                        state::empty_state(
                            &mut list_ui,
                            Icon::Search,
                            "No matches",
                            "No command, panel or tab has these words in its name.",
                            None,
                        );
                    } else {
                        egui::ScrollArea::vertical()
                            .id_salt("gz-app.palette_scroll")
                            .auto_shrink([false, false])
                            .max_height(list.height())
                            .show_rows(&mut list_ui, row_h, rows.len(), |ui, range| {
                                ui.spacing_mut().item_spacing = Vec2::ZERO;
                                for i in range {
                                    let Some(row) = rows.get(i) else { continue };
                                    let resp = palette_row(ui, row, i == selected, row_h);
                                    if i == selected && scroll_to {
                                        resp.scroll_to_me(None);
                                    }
                                    if resp.clicked() {
                                        clicked = Some(i);
                                    }
                                }
                            });
                    }
                    // the footer: how many, and the keys
                    let footer = Rect::from_min_max(pos2(rect.min.x, rect.max.y - footer_h), rect.max);
                    ui.painter().line_segment(
                        [footer.left_top(), footer.right_top()],
                        Stroke::new(1.0, p.border_subtle),
                    );
                    let small = Font::Small.id();
                    let found = if count == 1 { "1 result".to_owned() } else { format!("{count} results") };
                    ui.painter().text(
                        pos2(footer.min.x + 12.0, footer.center().y),
                        Align2::LEFT_CENTER,
                        found,
                        small.clone(),
                        p.text_muted,
                    );
                    let mut x = footer.max.x - 12.0;
                    for (keys, what) in [("Esc", "Close"), ("Enter", "Run")] {
                        x -= text::width(&ctx, &small, what);
                        ui.painter().text(
                            pos2(x, footer.center().y),
                            Align2::LEFT_CENTER,
                            what,
                            small.clone(),
                            p.text_muted,
                        );
                        x -= 6.0;
                        let kw = gz_ui::keycaps(ui, pos2(x, footer.center().y), keys, true);
                        x -= kw + 16.0;
                    }
                });
            });
        self.palette.query = query;
        if self.palette.query != query_before {
            self.palette.selected = 0;
            self.palette.scroll_to_selected = true;
        }
        if clicked.is_some() {
            run = clicked;
        }
        if esc || modal.should_close() && run.is_none() {
            self.palette.open = false;
            return;
        }
        if let Some(i) = run {
            self.palette.open = false;
            if let Some(row) = rows.get(i) {
                let executed = match row.command {
                    Some(c) => self.run_command(&ctx, c),
                    None => self.palette_execute(&ctx, &row.id),
                };
                if !executed {
                    state::toast(&ctx, Tone::Warning, &format!("{} is not available here.", row.label));
                }
            }
        }
    }

    // ---- ui_popup ------------------------------------------------------------------------

    /// Puts a popup up to be looked at, the way a click does; the one up before goes. `Err`
    /// says why it cannot.
    pub(crate) fn show_popup(&mut self, ctx: &Context, show: &str) -> Result<(), String> {
        if show == "none" {
            Popup::close_all(ctx);
            self.tab_menu = None;
            return Ok(());
        }
        let (kind, arg) = show.split_once(':').unwrap_or((show, ""));
        let active = self.active.to_string();
        match kind {
            "menu" => {
                if !contract::MENUS.contains(&arg) {
                    return Err(format!("no menu '{arg}' (ui_contract lists the menus)"));
                }
                let graph = active.starts_with("quest:") || active.starts_with("beh:");
                if arg == "Graph" && !graph {
                    return Err("the Graph menu belongs to a graph tab: ui_tab activate one first".to_owned());
                }
                if matches!(arg, "Scene" | "Build" | "View") && active != LEVEL {
                    return Err(format!("the {arg} menu belongs to the Level tab: ui_tab action=activate id=level first"));
                }
                if arg == "Menu" && !self.folded {
                    return Err("the menus are folded into the Menu button only while the window is narrower than 1100 pt".to_owned());
                }
                if !self.menus_shown {
                    return Err("the Start page has no menus".to_owned());
                }
                if matches!(arg, "File" | "Edit" | "Mod" | "Libraries" | "Window" | "Help") && !self.menu_has_rows(arg) {
                    return Err(format!("the {arg} menu has no rows in this shell"));
                }
                Popup::close_all(ctx);
                self.tab_menu = None;
                if self.folded {
                    Popup::open_id(ctx, fold_id());
                } else {
                    Popup::open_id(ctx, menu_id(arg));
                }
                Ok(())
            }
            "tab_menu" => {
                if self.find(arg).is_none() {
                    return Err(format!("no tab '{arg}'"));
                }
                Popup::close_all(ctx);
                self.tab_menu = Some(SmolStr::new(arg));
                Popup::open_id(ctx, Id::unique("gz-app.tab_menu"));
                Ok(())
            }
            "tab_overflow" => {
                Popup::close_all(ctx);
                Popup::open_id(ctx, Id::unique("gz-app.tab_overflow"));
                Ok(())
            }
            "tasks" => {
                if !self.editor.jobs.any_running() {
                    return Err("no job runs now: the tasks chip and its card are up only while one does (progress lists them)".to_owned());
                }
                self.card_open = true;
                Ok(())
            }
            "rebuild" if self.parts.status_chips.is_empty() => Err("nothing needs rebuilding: the Rebuild chip is up only while the scene has changes the level builders have not seen".to_owned()),
            "toolbar_split" if arg.is_empty() => Err("toolbar_split:<id> names the split button (build_mod on every tab)".to_owned()),
            "cb_context" if arg.is_empty() => Err("cb_context:<path> names an entry as content_browser_selection lists it".to_owned()),
            "viewport" if !contract::VIEWPORT_MENUS.contains(&arg) => Err(format!("no viewport menu '{arg}' (ui_contract lists them)")),
            "toolbar_overflow" | "toolbar_split" | "cb_context" | "cb_add" | "cb_view" | "cb_favorites" | "cb_grid" | "outliner_filters" | "outliner_more" | "viewport" | "rebuild" => {
                if matches!(kind, "outliner_filters" | "outliner_more" | "viewport") {
                    self.activate(ctx, LEVEL);
                }
                if kind.starts_with("cb_") && !self.has_drawer(Drawer::Content) {
                    return Err("no Content Browser is installed in this shell".to_owned());
                }
                Popup::close_all(ctx);
                self.tab_menu = None;
                let name = if kind == "toolbar_overflow" && arg.is_empty() { format!("toolbar_overflow:{}", self.active) } else { show.to_owned() };
                gz_ui::menu::show_popup(ctx, &name);
                Ok(())
            }
            _ => Err(format!("unknown popup '{show}' (ui_contract lists them)")),
        }
    }
}

/// One row of the Task Center's card; `true` when its Stop was pressed.
#[allow(clippy::too_many_arguments)]
fn card_row(
    ui: &mut Ui,
    job: &gz_editor::JobInfo,
    card: &crate::TaskCard,
    width: f32,
    first: bool,
    cancel_requested: bool,
    now: u64,
    hide: &mut bool,
) -> bool {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let small = Font::Small.id();
    // the title, its spinner, and the chevron that hides the card
    let (head, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let mut spin = ui.new_child(
        UiBuilder::new()
            .max_rect(Rect::from_min_size(pos2(head.min.x, head.center().y - 8.0), Vec2::splat(16.0))),
    );
    state::spinner(&mut spin);
    let hide_w = if first { 24.0 } else { 0.0 };
    text::paint(
        ui.painter(),
        &Font::Strong.id(),
        pos2(head.min.x + 24.0, head.center().y),
        Align2::LEFT_CENTER,
        p.text,
        &job.title,
        Some((width - 24.0 - hide_w).max(0.0)),
        Clip::End,
    );
    if first {
        let mut cell = ui.new_child(
            UiBuilder::new()
                .max_rect(Rect::from_min_size(pos2(head.max.x - 24.0, head.min.y), Vec2::splat(24.0))),
        );
        if icon_button_sized(
            &mut cell,
            Icon::ChevronDown,
            "Hide. The job goes on, and the chip in the status bar brings this back.",
            false,
            true,
            24.0,
            16.0,
        )
        .clicked()
        {
            *hide = true;
        }
    }
    if !card.subtitle.is_empty() {
        small_line(ui, &card.subtitle, p.text_secondary, width, Clip::Middle);
    }
    ui.add_space(6.0);
    card_steps(ui, card, width);
    ui.add_space(6.0);
    card_bar(ui, job.fraction, width);
    ui.add_space(2.0);
    // what it is doing, and for how long: "Copying files        00:42 · about 01:08 left"
    let mut time = crate::tasks::format_elapsed(now.saturating_sub(job.started_ms));
    if card.eta_ms > 0 {
        time.push_str(&format!(" · about {} left", crate::tasks::format_elapsed(card.eta_ms)));
    }
    let time_w = text::width(&ctx, &small, &time);
    let (line, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
    text::paint(
        ui.painter(),
        &small,
        pos2(line.min.x, line.center().y),
        Align2::LEFT_CENTER,
        p.text,
        &job.stage,
        Some((width - time_w - 12.0).max(0.0)),
        Clip::End,
    );
    ui.painter().text(
        pos2(line.max.x - time_w, line.center().y),
        Align2::LEFT_CENTER,
        &time,
        small.clone(),
        p.text_muted,
    );
    // how far: "38% · 2.1 of 4.6 GB · 3412 of 7146 files"
    if let Some(f) = job.fraction {
        let mut numbers = format!("{:.0}%", f * 100.0);
        if !card.numbers.is_empty() {
            numbers.push_str("  ·  ");
            numbers.push_str(&card.numbers);
        }
        small_line(ui, &numbers, p.text_secondary, width, Clip::End);
    }
    if !card.file.is_empty() {
        small_line(ui, &card.file, p.text_muted, width, Clip::Middle);
    }
    ui.add_space(4.0);
    let label = if cancel_requested {
        "Stopping...".to_owned()
    } else if card.stop_label.is_empty() {
        "Stop".to_owned()
    } else {
        card.stop_label.clone()
    };
    let w = gz_ui::units::button_width(&ctx, &label, Kind::Secondary, false, 0.0);
    let (row, _) = ui.allocate_exact_size(Vec2::new(width, 24.0), Sense::hover());
    let tip = if card.stop_tip.is_empty() { "It stops at the next step." } else { card.stop_tip.as_str() };
    let stopped = ui
        .put(
            Rect::from_min_size(pos2(row.max.x - w, row.min.y), Vec2::new(w, 24.0)),
            Button::new(&label).enabled(!cancel_requested, "It stops at the next step.").tip(tip),
        )
        .clicked();
    stopped && !cancel_requested
}

fn small_line(ui: &mut Ui, text_: &str, color: Color32, width: f32, clip: Clip) {
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
    let shown_cut = text::paint(
        ui.painter(),
        &Font::Small.id(),
        pos2(r.min.x, r.center().y),
        Align2::LEFT_CENTER,
        color,
        text_,
        Some(width),
        clip,
    );
    if shown_cut {
        ui.interact(r, ui.scope_id().with(("gz-app.small_line", text_)), Sense::hover()).on_hover_text(text_);
    }
}

/// o Check --- O Copy --- . Install: where the job is on its way.
fn card_steps(ui: &mut Ui, card: &crate::TaskCard, width: f32) {
    if card.steps.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let small = Font::Small.id();
    let (dot, gap) = (3.0, 5.0);
    let labels: f32 = card.steps.iter().map(|s| text::width(&ctx, &small, s) + dot * 2.0 + gap).sum();
    let count = card.steps.len();
    let link = if count > 1 { ((width - labels) / (count - 1) as f32).max(6.0) } else { 0.0 };
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 18.0), Sense::hover());
    let cy = r.center().y;
    let mut x = r.min.x;
    for (i, name) in card.steps.iter().enumerate() {
        let i = i as i32;
        let (done, now) = (i < card.step, i == card.step);
        let tone = if done || now { p.accent } else { p.text_muted };
        let c = pos2(x + dot, cy);
        if done || now {
            ui.painter().circle_filled(c, dot, tone);
        } else {
            ui.painter().circle_stroke(c, dot - 0.5, Stroke::new(1.0, tone));
        }
        if now {
            ui.painter().circle_stroke(c, dot + 2.0, Stroke::new(1.0, p.accent.gamma_multiply(0.45)));
        }
        x += dot * 2.0 + gap;
        let color = if now {
            p.text
        } else if done {
            p.text_secondary
        } else {
            p.text_muted
        };
        ui.painter().text(pos2(x, cy), Align2::LEFT_CENTER, name, small.clone(), color);
        x += text::width(&ctx, &small, name);
        if (i as usize) + 1 < count {
            let (a, b) = (x + gap, x + link - gap);
            if b > a {
                ui.painter().line_segment(
                    [pos2(a, cy), pos2(b, cy)],
                    Stroke::new(1.0, if done { p.accent.gamma_multiply(0.6) } else { p.border_strong }),
                );
            }
            x += link;
        }
    }
}

/// A 6 pt track: filled as far as the job has come, or crossed by a light while nobody can tell
/// how long it takes.
fn card_bar(ui: &mut Ui, fraction: Option<f32>, width: f32) {
    let p = palette(ui.ctx());
    let (r, _) = ui.allocate_exact_size(Vec2::new(width, 6.0), Sense::hover());
    let radius = CornerRadius::same(3);
    ui.painter().rect_filled(r, radius, p.inset);
    match fraction {
        Some(f) => {
            let fill = (f.clamp(0.0, 1.0) * width).max(if f > 0.0 { 6.0 } else { 0.0 });
            if fill > 0.0 {
                ui.painter().rect_filled(Rect::from_min_size(r.min, Vec2::new(fill, 6.0)), radius, p.accent);
            }
        }
        None => {
            let period = 1.4;
            let t = ui.input(|i| i.time) as f32;
            let phase = (t % period) / period;
            let eased = phase * phase * (3.0 - 2.0 * phase);
            let light = width * 0.32;
            let x = r.min.x - light + (width + light) * eased;
            let lit = Rect::from_min_size(pos2(x, r.min.y), Vec2::new(light, 6.0)).intersect(r);
            if lit.width() > 0.0 {
                ui.painter().rect_filled(lit, radius, p.accent);
            }
        }
    }
}

/// One row of the palette: the icon, the name, where it lives (cut at its start), the keys.
fn palette_row(ui: &mut Ui, row: &PaletteRow, selected: bool, row_h: f32) -> Response {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), row_h), Sense::click());
    if ui.is_rect_visible(rect) {
        if selected || resp.hovered() {
            ui.painter().rect_filled(
                rect,
                CornerRadius::same(4),
                if selected { p.selection_focused } else { p.hover },
            );
        }
        icons::draw(
            ui.painter(),
            row.icon,
            Rect::from_min_size(pos2(rect.min.x + 10.0, rect.center().y - 8.0), Vec2::splat(16.0)),
            if selected { p.accent_bright } else { p.text_secondary },
        );
        let keys_w = gz_ui::keycaps_width(ui, &row.shortcut);
        let right = rect.max.x - 10.0 - if keys_w > 0.0 { keys_w + 12.0 } else { 0.0 };
        let name_x = rect.min.x + 36.0;
        let body = Font::Body.id();
        let parent = pal::parent(row);
        let room = (right - name_x).max(0.0);
        let name_w = text::width(&ctx, &body, &row.label).min(if parent.is_empty() {
            room
        } else {
            (room * 0.6).max(room - gz_ui::units::em(8.0))
        });
        text::paint(
            ui.painter(),
            &body,
            pos2(name_x, rect.center().y),
            Align2::LEFT_CENTER,
            p.text,
            &row.label,
            Some(name_w + 0.5),
            Clip::End,
        );
        let path_x = name_x + name_w + 12.0;
        if !parent.is_empty() && right - path_x > 24.0 {
            text::paint(
                ui.painter(),
                &Font::Small.id(),
                pos2(path_x, rect.center().y),
                Align2::LEFT_CENTER,
                p.text_muted,
                parent,
                Some(right - path_x),
                Clip::Start,
            );
        }
        if keys_w > 0.0 {
            gz_ui::keycaps(ui, pos2(rect.max.x - 10.0, rect.center().y), &row.shortcut, true);
        }
    }
    audit::control(&ctx, &row.label, true, None, Some(&row.path), rect);
    resp.on_hover_text(format!("{}\n{}", row.path, row.id))
}

/// Opens a folder in Explorer.
pub(crate) fn reveal_folder(path: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(path).spawn();
}

/// Shows a file selected in Explorer.
pub(crate) fn reveal_file(path: &std::path::Path) {
    let mut arg = std::ffi::OsString::from("/select,");
    arg.push(path.as_os_str());
    let _ = std::process::Command::new("explorer.exe").arg(arg).spawn();
}
