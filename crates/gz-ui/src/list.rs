//! Lists and tables (docs/UI_DESIGN.md, section 4, "Lists and tables").
//!
//! - [`list`] - an inset, bordered, scrolling box of rows; its closure gets a [`List`].
//! - [`List::row`] - a 22 pt row with a stable id, selected or not; its content closure draws on
//!   the row, inset 6 pt. [`List::row_actions`] puts row-high squares with 14 pt glyphs on the
//!   last row while it is hovered.
//! - [`nav_row`] per [`NavRowSpec`] - the row of a nav or a page list, in or out of a list: an
//!   icon, an ellipsized label, a status dot or a count, one hover action.
//! - [`table_headers`] per [`Column`] - Small secondary names, centred where asked, sortable.

use egui::{
    Align, Align2, CornerRadius, Frame, Id, Layout, Margin, Rect, Response, Role, ScrollArea, Sense, Stroke,
    Ui, UiBuilder, Vec2, WidgetInfo, pos2,
};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};

/// Whether the list was the last thing clicked: its selection reads focused.
fn active_id(id: Id) -> Id {
    id.with("gz-ui.list.active")
}

/// What a list's keys work from: last frame's rows, and the row the keys asked for.
#[derive(Clone, Debug, Default)]
struct KeyNav {
    rows: Vec<String>,
    selected: Option<usize>,
    target: Option<String>,
    typed: String,
    typed_at: f64,
}

fn nav_id(id: Id) -> Id {
    id.with("gz-ui.list.keys")
}

/// How long typed letters keep adding up to one prefix.
const TYPE_AHEAD_S: f64 = 1.0;
/// The rows Page Up / Page Down move by.
const PAGE_ROWS: usize = 10;

/// The leaf of a stable id, which type-ahead matches: a path's last part, or the id itself.
fn leaf(stable_id: &str) -> &str {
    stable_id.rsplit(['/', '\\']).next().unwrap_or(stable_id)
}

/// The row the keys of this frame move to, from last frame's rows: the arrows, Home / End,
/// Page Up / Down, and type-ahead over the rows' names.
fn key_target(ctx: &egui::Context, nav: &mut KeyNav) -> Option<String> {
    use egui::{Event, Key, Modifiers};
    let n = nav.rows.len();
    if n == 0 {
        return None;
    }
    let cur = nav.selected;
    let take = |k: Key| ctx.input_mut(|i| i.consume_key(Modifiers::NONE, k));
    let last = n - 1;
    let to = if take(Key::ArrowDown) {
        Some(cur.map_or(0, |c| (c + 1).min(last)))
    } else if take(Key::ArrowUp) {
        Some(cur.map_or(last, |c| c.saturating_sub(1)))
    } else if take(Key::Home) {
        Some(0)
    } else if take(Key::End) {
        Some(last)
    } else if take(Key::PageDown) {
        Some(cur.map_or(0, |c| (c + PAGE_ROWS).min(last)))
    } else if take(Key::PageUp) {
        Some(cur.map_or(0, |c| c.saturating_sub(PAGE_ROWS)))
    } else {
        let typed: String = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| if let Event::Text(t) = e { Some(t.clone()) } else { None })
                .collect()
        });
        let typed = typed.trim().to_lowercase();
        if typed.is_empty() {
            None
        } else {
            let now = ctx.input(|i| i.time);
            if now - nav.typed_at > TYPE_AHEAD_S {
                nav.typed.clear();
            }
            nav.typed_at = now;
            nav.typed.push_str(&typed);
            // one letter again and again walks the rows that start with it
            let repeat = nav.typed.chars().all(|c| nav.typed.starts_with(c));
            let prefix =
                if repeat { nav.typed.chars().take(1).collect::<String>() } else { nav.typed.clone() };
            let start = if repeat { cur.map_or(0, |c| c + 1) } else { cur.unwrap_or(0) };
            (0..n)
                .map(|k| (start + k) % n)
                .find(|&k| nav.rows.get(k).is_some_and(|r| leaf(r).to_lowercase().starts_with(&prefix)))
        }
    };
    to.filter(|t| Some(*t) != cur).and_then(|t| nav.rows.get(t).cloned())
}

/// The rows being drawn into a [`list`].
pub struct List<'u> {
    ui: &'u mut Ui,
    id: Id,
    active: bool,
    clicked: bool,
    /// The last row drawn and whether the pointer is on it.
    row: Option<(Rect, bool)>,
    nav: KeyNav,
}

impl List<'_> {
    /// The list's own `Ui`, for what is not a row (a header, an empty state).
    pub fn ui(&mut self) -> &mut Ui {
        self.ui
    }

    /// A row. `stable_id` names it across frames (a path, a key - never an index that shifts).
    /// Returns the row's response: `clicked()` selects it.
    pub fn row(&mut self, stable_id: &str, selected: bool, content: impl FnOnce(&mut Ui)) -> Response {
        let p = palette(self.ui.ctx());
        let width = self.ui.available_width();
        let id = self.id.with(stable_id);
        let (rect, _) = self.ui.allocate_exact_size(Vec2::new(width, metrics::ROW), Sense::hover());
        let mut resp = self.ui.interact(rect, id, Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::ListItem, true, selected, stable_id));
        // the row the keys moved to answers as clicked, so every list selects it the way a
        // click does
        if self.nav.target.as_deref() == Some(stable_id) {
            self.nav.target = None;
            resp.flags |= egui::response::Flags::FAKE_PRIMARY_CLICKED;
            resp.scroll_to_me(None);
        }
        if selected {
            self.nav.selected = Some(self.nav.rows.len());
        }
        self.nav.rows.push(stable_id.to_owned());
        let hovered = self.ui.rect_contains_pointer(rect);
        if self.ui.is_rect_visible(rect) {
            let radius = CornerRadius::same(3);
            if selected {
                self.ui.painter().rect_filled(
                    rect,
                    radius,
                    if self.active { p.selection_focused } else { p.selection },
                );
            } else if hovered {
                self.ui.painter().rect_filled(
                    rect,
                    radius,
                    if resp.is_pointer_button_down_on() { p.pressed } else { p.hover },
                );
            }
            let inner = Rect::from_min_max(pos2(rect.min.x + 6.0, rect.min.y), rect.max);
            let mut child = self.ui.new_child(
                UiBuilder::new().id_salt(id).max_rect(inner).layout(Layout::left_to_right(Align::Center)),
            );
            content(&mut child);
        }
        self.clicked |= resp.clicked();
        self.row = Some((rect, hovered));
        resp
    }

    /// Hover actions on the last row: squares as tall as the row with 14 pt glyphs, flush right.
    /// Returns the index of the one pressed.
    pub fn row_actions(&mut self, icons_: &[Icon], tips: &[&str]) -> Option<usize> {
        let (row, hovered) = self.row?;
        if !hovered {
            return None;
        }
        row_actions_at(self.ui, row, icons_, tips)
    }
}

/// Row actions on any row rectangle: shown by the caller while the row is hovered.
pub fn row_actions_at(ui: &mut Ui, row: Rect, icons_: &[Icon], tips: &[&str]) -> Option<usize> {
    let side = row.height();
    let n = icons_.len();
    let mut clicked = None;
    for (i, icon) in icons_.iter().enumerate() {
        let x = row.max.x - (n - i) as f32 * (side + 2.0);
        let r = Rect::from_min_size(pos2(x, row.min.y), Vec2::splat(side));
        let tip = tips.get(i).copied().unwrap_or("");
        let mut child =
            ui.new_child(UiBuilder::new().id_salt(("gz-ui.row_action", row.min.y.to_bits(), i)).max_rect(r));
        if icon_button_sized(&mut child, *icon, tip, false, true, side, metrics::GLYPH_ROW).clicked() {
            clicked = Some(i);
        }
    }
    clicked
}

/// An inset, bordered box of rows that scrolls. `size` is in points per axis: positive is that
/// size, negative is what is left minus it, zero is all the width / as tall as its rows.
pub fn list<R>(ui: &mut Ui, id_salt: &str, size: Vec2, add: impl FnOnce(&mut List<'_>) -> R) -> R {
    let p = palette(ui.ctx());
    let avail = ui.available_size();
    let axis = |pt: f32, room: f32| {
        if pt > 0.0 {
            pt
        } else if pt < 0.0 {
            (room + pt).max(1.0)
        } else {
            room
        }
    };
    let w = axis(size.x, avail.x);
    let h = if size.y == 0.0 { f32::INFINITY } else { axis(size.y, avail.y) };
    let id = ui.make_persistent_id(("gz-ui.list", id_salt));
    let ctx = ui.ctx().clone();
    let active = ctx.data(|d| d.get_temp::<bool>(active_id(id))).unwrap_or(false);
    let mut nav = ctx.data(|d| d.get_temp::<KeyNav>(nav_id(id))).unwrap_or_default();
    let target = if active && !ctx.egui_wants_keyboard_input() { key_target(&ctx, &mut nav) } else { None };
    let (typed, typed_at) = (nav.typed.clone(), nav.typed_at);
    let out = Frame::new()
        .fill(p.inset)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(metrics::RADIUS))
        .inner_margin(Margin::same(4))
        .show(ui, |ui| {
            ui.set_width((w - 10.0).max(1.0));
            ScrollArea::vertical()
                .id_salt(id)
                .max_height(if h.is_finite() { (h - 10.0).max(1.0) } else { f32::INFINITY })
                .auto_shrink([false, size.y == 0.0])
                .show(ui, |ui| {
                    if h.is_finite() {
                        ui.set_min_height((h - 10.0).max(1.0));
                    }
                    // rows touch: their 22 pt is the rhythm
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let mut l = List {
                        ui,
                        id,
                        active,
                        clicked: false,
                        row: None,
                        nav: KeyNav { target, typed, typed_at, ..KeyNav::default() },
                    };
                    let r = add(&mut l);
                    (r, l.clicked, l.nav)
                })
                .inner
        });
    let (r, clicked, mut nav) = out.inner;
    nav.target = None;
    ctx.data_mut(|d| d.insert_temp(nav_id(id), nav));
    let rect = out.response.rect;
    // the selection reads focused from a click on a row until a click elsewhere
    let elsewhere = ctx
        .input(|i| i.pointer.any_click() && i.pointer.interact_pos().is_some_and(|pos| !rect.contains(pos)));
    if clicked != active && (clicked || elsewhere) {
        ctx.data_mut(|d| d.insert_temp(active_id(id), clicked));
    }
    r
}

/// The row of a nav, a page list or a sources pane.
#[derive(Clone, Copy, Debug)]
pub struct NavRowSpec<'a> {
    pub icon: Icon,
    /// The row's tooltip; empty for none (a cut label then shows itself).
    pub tip: &'a str,
    /// A status dot at the right end.
    pub dot: Option<Tone>,
    /// A count at the right end, when there is no dot.
    pub count: Option<&'a str>,
    /// Dimmed: a thing that is there but off.
    pub dim: bool,
    /// The one action shown while the row is hovered; `Icon::None` for none.
    pub action: Icon,
    pub action_tip: &'a str,
}

impl Default for NavRowSpec<'_> {
    fn default() -> Self {
        Self {
            icon: Icon::None,
            tip: "",
            dot: None,
            count: None,
            dim: false,
            action: Icon::None,
            action_tip: "",
        }
    }
}

/// What a nav row was asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NavResponse {
    pub clicked: bool,
    pub action: bool,
}

/// A nav row, 22 pt, the whole width it gets.
pub fn nav_row(
    ui: &mut Ui,
    stable_id: &str,
    label: &str,
    selected: bool,
    spec: &NavRowSpec<'_>,
) -> NavResponse {
    let p = palette(ui.ctx());
    let h = metrics::ROW;
    let width = ui.available_width().max(h * 2.0);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, h), Sense::hover());
    let id = ui.make_persistent_id(("gz-ui.nav_row", stable_id));
    let resp = ui.interact(rect, id, Sense::click());
    resp.widget_info(|| WidgetInfo::selected(Role::ListItem, true, selected, label));
    let row_hovered = ui.rect_contains_pointer(rect);
    let mut out = NavResponse { clicked: resp.clicked(), action: false };
    audit::control(
        ui.ctx(),
        label,
        true,
        None,
        if spec.tip.is_empty() { None } else { Some(spec.tip) },
        rect,
    );
    if !ui.is_rect_visible(rect) {
        return out;
    }
    let ctx = ui.ctx().clone();
    let painter = ui.painter().clone();
    let radius = CornerRadius::same(3);
    if selected {
        painter.rect_filled(
            rect,
            radius,
            if resp.has_focus() || row_hovered { p.selection_focused } else { p.selection },
        );
    } else if row_hovered {
        painter.rect_filled(rect, radius, if resp.is_pointer_button_down_on() { p.pressed } else { p.hover });
    }

    // the right end: the hover action, else a dot or a count
    let mut text_right = rect.max.x - 6.0;
    let show_action = spec.action != Icon::None && row_hovered;
    if show_action {
        text_right = rect.max.x - h - 2.0;
    } else if let Some(tone) = spec.dot {
        painter.circle_filled(pos2(rect.max.x - 10.0, rect.center().y), 3.5, p.tone(tone));
        text_right = rect.max.x - 18.0;
    } else if let Some(count) = spec.count {
        let font = Font::Small.id();
        let cw = text::width(&ctx, &font, count);
        painter.text(
            pos2(rect.max.x - 8.0, rect.center().y),
            Align2::RIGHT_CENTER,
            count,
            font,
            p.text_muted,
        );
        text_right = rect.max.x - 8.0 - cw - 8.0;
    }

    let mut x = rect.min.x + 6.0;
    if spec.icon != Icon::None {
        let color = if selected {
            p.accent_bright
        } else if spec.dim {
            p.text_disabled
        } else {
            p.text_secondary
        };
        icons::draw(
            &painter,
            spec.icon,
            Rect::from_min_size(pos2(x, rect.center().y - metrics::GLYPH * 0.5), Vec2::splat(metrics::GLYPH)),
            color,
        );
        x += metrics::GLYPH + 6.0;
    }
    let color = if spec.dim { p.text_disabled } else { p.text };
    let cut = text::paint(
        &painter,
        &Font::Body.id(),
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        color,
        label,
        Some((text_right - x).max(0.0)),
        Clip::End,
    );
    if !spec.tip.is_empty() {
        resp.on_hover_text(spec.tip);
    } else if cut {
        resp.on_hover_text(label);
    }
    if show_action {
        let r = Rect::from_min_size(pos2(rect.max.x - h, rect.min.y), Vec2::splat(h));
        let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("action")).max_rect(r));
        out.action = icon_button_sized(
            &mut child,
            spec.action,
            spec.action_tip,
            false,
            true,
            h,
            metrics::GLYPH_STATUS,
        )
        .clicked();
    }
    out
}

/// A column of [`table_headers`].
#[derive(Clone, Copy, Debug)]
pub struct Column<'a> {
    pub name: &'a str,
    /// Its width in points.
    pub width: f32,
    /// Centred over its cells (a checkbox column).
    pub centred: bool,
    /// Sorted by this column: `Some(true)` ascending.
    pub sorted: Option<bool>,
    /// Clicking the name sorts by it.
    pub sortable: bool,
}

impl<'a> Column<'a> {
    #[must_use]
    pub fn new(name: &'a str, width: f32) -> Self {
        Self { name, width, centred: false, sorted: None, sortable: false }
    }
}

/// The header row of a table: Small secondary names over their columns, 24 pt. Returns the
/// column whose name was clicked, for a sortable table.
pub fn table_headers(ui: &mut Ui, columns: &[Column<'_>]) -> Option<usize> {
    let p = palette(ui.ctx());
    let h = 24.0;
    let total: f32 = columns.iter().map(|c| c.width).sum();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total.max(ui.available_width()), h), Sense::hover());
    let font = Font::Small.id();
    let mut clicked = None;
    let mut x = rect.min.x;
    for (i, c) in columns.iter().enumerate() {
        let cell = Rect::from_min_size(pos2(x, rect.min.y), Vec2::new(c.width, h));
        x += c.width;
        let sort_w = if c.sorted.is_some() { metrics::GLYPH_STATUS + 4.0 } else { 0.0 };
        let room = (c.width - 12.0 - sort_w).max(0.0);
        let name_w = text::width(ui.ctx(), &font, c.name).min(room);
        let tx = if c.centred { cell.center().x - (name_w + sort_w) * 0.5 } else { cell.min.x + 6.0 };
        let mut color = p.text_secondary;
        if c.sortable {
            let resp =
                ui.interact(cell, ui.make_persistent_id(("gz-ui.table_header", i, c.name)), Sense::click());
            if resp.hovered() {
                color = p.text;
            }
            if resp.clicked() {
                clicked = Some(i);
            }
            audit::control(ui.ctx(), c.name, true, None, Some("Sort by this column"), cell);
        }
        if ui.is_rect_visible(cell) {
            text::paint(
                ui.painter(),
                &font,
                pos2(tx, cell.center().y),
                Align2::LEFT_CENTER,
                color,
                c.name,
                Some(room),
                Clip::End,
            );
            if let Some(asc) = c.sorted {
                let r = Rect::from_min_size(
                    pos2(tx + name_w + 4.0, cell.center().y - 6.0),
                    Vec2::splat(metrics::GLYPH_STATUS),
                );
                icons::draw(
                    ui.painter(),
                    if asc { Icon::SortAsc } else { Icon::SortDesc },
                    r,
                    p.text_secondary,
                );
            }
        }
    }
    ui.painter().line_segment(
        [pos2(rect.min.x, rect.max.y - 0.5), pos2(rect.max.x, rect.max.y - 0.5)],
        Stroke::new(1.0, p.border_subtle),
    );
    clicked
}
