//! The property grid (docs/UI_DESIGN.md, section 4, "Property grid").
//!
//! - [`PropertyGrid::new(role)`](PropertyGrid::new)`.show(ui, |g| ..)` - a label column and a
//!   value column. The label column's share is kept per role in the layout key `grid.<role>`
//!   (default .40), draggable within 6 em .. width - 8 em, double-click resets; below 18 em the
//!   label goes above its value.
//! - [`Grid::section`] - a full-row header (28 pt) that folds what it holds, with a badge.
//! - [`Grid::row`] - a 26 pt row per [`PropFlags`] (ReadOnly, Required, Invalid, Modified, Mixed,
//!   Inline) at a depth (12 pt per level); the value closure draws in the value column.
//!   [`Grid::trailing`] adds an icon button at the row's end, [`Grid::message`] a sentence under
//!   it, [`Grid::was_reset`] says a Modified row's Reset was pressed, [`Grid::block`] a block the
//!   width of the grid.
//! - [`help_footer`] - the hovered row's label and help, for every grid of a panel.
//! - Values, each starting at the left of its field and reading "Multiple values" in a Mixed
//!   row: [`text_field`], [`number`] / [`integer`] (a drag without steppers, the unit after it,
//!   a click to type), [`switch`], [`combo`], [`color`] (a swatch and hex), [`vec3_field`],
//!   [`pick_field`] (the leaf name with a start ellipsis, browse, clear: [`PickAction`]),
//!   [`path_field`] ([`PathAction`]), [`chips`], [`text_block`], [`read_only`].

use std::ops::BitOr;

use egui::{
    Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Id, Key, Layout, Margin, Rect, Response,
    RichText, Sense, Stroke, StrokeKind, TextEdit, Ui, UiBuilder, Vec2, pos2,
};

use crate::audit;
use crate::button::icon_button_sized;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::layout;
use crate::menu::{MenuItem, menu_item};
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};
use crate::units::em;

/// What a property row is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct PropFlags(u8);

impl PropFlags {
    pub const NONE: Self = Self(0);
    /// The label in the muted colour; the value is shown, not edited.
    pub const READ_ONLY: Self = Self(1);
    /// A dot after the label: the author still has to set it.
    pub const REQUIRED: Self = Self(2);
    /// A bar at the row's left edge; say why with [`Grid::message`].
    pub const INVALID: Self = Self(4);
    /// A Reset icon at the row's end; [`Grid::was_reset`] says it was pressed.
    pub const MODIFIED: Self = Self(8);
    /// The selection differs: the value reads "Multiple values".
    pub const MIXED: Self = Self(16);
    /// A switch: on its label's line even when labels go above values, the box first.
    pub const INLINE: Self = Self(32);

    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0 && other.0 != 0
    }
}

impl BitOr for PropFlags {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

const ROW_H: f32 = metrics::PROPERTY_ROW;
const HEADER_H: f32 = metrics::SECTION_HEADER;
const DEPTH_INDENT: f32 = 12.0;
/// Between the label column and the value.
const LABEL_GAP: f32 = 8.0;
/// A trailing icon button.
const SLOT: f32 = 24.0;
const MULTIPLE: &str = "Multiple values";

/// What a row keeps for the frames after: its trailing slots, the height it took.
#[derive(Clone, Copy, Debug, Default)]
struct RowMem {
    slots: u8,
    height: f32,
}

/// The row being drawn.
struct OpenRow {
    id: Id,
    rect: Rect,
    value_x: f32,
    /// How low what the row holds reaches.
    bottom: f32,
    slots_used: u8,
    reset: bool,
}

/// What the help footer shows: the row hovered last, and when.
#[derive(Clone, Debug, Default)]
struct FooterState {
    label: String,
    help: String,
    frame: u64,
}

fn footer_id() -> Id {
    Id::unique("gz-ui.grid.footer")
}

fn mixed_id() -> Id {
    Id::unique("gz-ui.grid.mixed")
}

/// Whether the value being drawn is in a Mixed row.
fn is_mixed(ui: &Ui) -> bool {
    ui.ctx().data(|d| d.get_temp::<bool>(mixed_id())).unwrap_or(false)
}

/// A property grid of one role.
#[must_use = "a grid does nothing until it is shown: .show(ui, |g| ..)"]
pub struct PropertyGrid<'a> {
    role: &'a str,
    frac: f32,
}

impl<'a> PropertyGrid<'a> {
    /// A grid whose label column is stored as `grid.<role>`.
    pub fn new(role: &'a str) -> Self {
        Self { role, frac: 0.40 }
    }

    /// The label column's share until the author drags it (Level Settings starts at .50).
    pub fn label_frac(mut self, frac: f32) -> Self {
        self.frac = frac;
        self
    }

    pub fn show<R>(self, ui: &mut Ui, add: impl FnOnce(&mut Grid<'_>) -> R) -> R {
        let ctx = ui.ctx().clone();
        let mut key = String::with_capacity(5 + self.role.len());
        key.push_str("grid.");
        key.push_str(if self.role.is_empty() { "default" } else { self.role });
        let frac = layout::get(&ctx, &key, self.frac).clamp(0.1, 0.9);
        let origin = ui.cursor().min;
        let width = ui.available_width().max(0.0);
        let stacked = width < em(18.0);
        let label_w = if stacked { width } else { label_width(width, frac) };
        let id = ui.make_persistent_id(("gz-ui.grid", self.role));
        let mut g = Grid {
            ui,
            id,
            key,
            default_frac: self.frac,
            x0: origin.x,
            x1: origin.x + width,
            label_w,
            stacked,
            top: origin.y,
            y: origin.y,
            row: None,
            hovered: None,
        };
        let r = add(&mut g);
        g.finish();
        r
    }
}

/// The label column for a grid `width` wide at a share: within 6 em .. width - 8 em.
#[must_use]
pub fn label_width(width: f32, frac: f32) -> f32 {
    (width * frac).clamp(em(6.0), em(6.0).max(width - em(8.0)))
}

/// The rows of a property grid being drawn.
pub struct Grid<'u> {
    ui: &'u mut Ui,
    id: Id,
    key: String,
    default_frac: f32,
    x0: f32,
    x1: f32,
    label_w: f32,
    stacked: bool,
    top: f32,
    /// Where the next row starts.
    y: f32,
    row: Option<OpenRow>,
    hovered: Option<(String, String)>,
}

impl Grid<'_> {
    /// Whether labels sit above their values (a grid narrower than 18 em).
    #[must_use]
    pub fn stacked(&self) -> bool {
        self.stacked
    }

    fn close_row(&mut self) {
        if let Some(row) = self.row.take() {
            let bottom = row.bottom.max(row.rect.max.y);
            let mem = RowMem { slots: row.slots_used, height: bottom - row.rect.min.y };
            self.ui.ctx().data_mut(|d| d.insert_temp(row.id, mem));
            self.y = bottom;
        }
    }

    /// A full-row header that folds what it holds; `open` is its state until it is clicked.
    /// `body` draws the section's rows while it is open. Returns whether it is open.
    pub fn section(
        &mut self,
        title: &str,
        open: bool,
        badge: Option<&str>,
        body: impl FnOnce(&mut Self),
    ) -> bool {
        self.close_row();
        let p = palette(self.ui.ctx());
        let ctx = self.ui.ctx().clone();
        let rect = Rect::from_min_max(pos2(self.x0, self.y), pos2(self.x1, self.y + HEADER_H));
        let id = self.id.with(("section", title));
        let mut is_open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
        let resp = self.ui.interact(rect, id, Sense::click());
        if resp.clicked() {
            is_open = !is_open;
            ctx.data_mut(|d| d.insert_temp(id, is_open));
        }
        if self.ui.is_rect_visible(rect) {
            let painter = self.ui.painter();
            painter.rect_filled(rect, CornerRadius::ZERO, if resp.hovered() { p.hover } else { p.raised });
            let cy = rect.center().y;
            let chevron =
                Rect::from_min_size(pos2(rect.min.x + 6.0, cy - 6.0), Vec2::splat(metrics::GLYPH_STATUS));
            icons::draw(
                painter,
                if is_open { Icon::ChevronDown } else { Icon::ChevronRight },
                chevron,
                p.text_secondary,
            );
            let mut right = rect.max.x - 8.0;
            if let Some(badge) = badge.filter(|b| !b.is_empty()) {
                let font = Font::Small.id();
                let w = text::width(&ctx, &font, badge) + 12.0;
                let pill = Rect::from_min_max(pos2(right - w, cy - 9.0), pos2(right, cy + 9.0));
                painter.rect_filled(pill, CornerRadius::same(9), p.base);
                painter.text(pos2(pill.min.x + 6.0, cy), Align2::LEFT_CENTER, badge, font, p.text_secondary);
                right = pill.min.x - 6.0;
            }
            let x = rect.min.x + 24.0;
            text::paint(
                painter,
                &Font::Strong.id(),
                pos2(x, cy),
                Align2::LEFT_CENTER,
                p.text,
                title,
                Some((right - x).max(0.0)),
                Clip::End,
            );
        }
        audit::control(&ctx, title, true, None, None, rect);
        self.y = rect.max.y;
        if is_open {
            body(self);
            self.close_row();
        }
        is_open
    }

    /// A property row. `value` draws in the value column, a field-high line that starts at the
    /// left of the column; what it draws below that line makes the row taller. Returns `None`
    /// for a row scrolled out of view (it keeps the height it took when last drawn).
    pub fn row<R>(
        &mut self,
        label: &str,
        help: &str,
        flags: PropFlags,
        depth: u32,
        value: impl FnOnce(&mut Ui) -> R,
    ) -> Option<R> {
        self.close_row();
        let ctx = self.ui.ctx().clone();
        let p = palette(&ctx);
        let inline_value = self.stacked && flags.contains(PropFlags::INLINE);
        let label_h = if self.stacked && !inline_value { Font::Body.line() + 2.0 } else { 0.0 };
        let y = self.y;
        let rect = Rect::from_min_max(pos2(self.x0, y), pos2(self.x1, y + label_h + ROW_H));
        let id = self.id.with(("row", label, depth));
        let mem = ctx.data(|d| d.get_temp::<RowMem>(id)).unwrap_or_default();
        let slots = mem.slots.max(u8::from(flags.contains(PropFlags::MODIFIED)));

        // out of view: keep the height the row took when it was drawn, and skip the work. Never in a
        // sizing pass: a row skipped while its window grows would hold its height back a frame
        let drawn_h = rect.height().max(mem.height);
        if !self.ui.is_sizing_pass()
            && !self.ui.is_rect_visible(Rect::from_min_size(rect.min, Vec2::new(rect.width(), drawn_h)))
        {
            self.y = y + drawn_h;
            return None;
        }

        if self.ui.rect_contains_pointer(rect) && self.hovered.as_ref().is_none_or(|(l, _)| l != label) {
            self.hovered = Some((label.to_owned(), help.to_owned()));
        }
        let painter = self.ui.painter().clone();
        if flags.contains(PropFlags::INVALID) {
            painter.rect_filled(
                Rect::from_min_max(pos2(self.x0, rect.min.y + 3.0), pos2(self.x0 + 2.0, rect.max.y - 3.0)),
                CornerRadius::ZERO,
                p.error,
            );
        }

        // the label: secondary text cut with an ellipsis; its tooltip names it and says what it does
        let indent = DEPTH_INDENT * depth as f32 + 6.0;
        let trailing_w = f32::from(slots) * (SLOT + 2.0);
        let label_x = self.x0 + indent + if inline_value { metrics::FRAME + 6.0 } else { 0.0 };
        let label_right = if inline_value {
            self.x1 - trailing_w
        } else if self.stacked {
            self.x1
        } else {
            self.x0 + self.label_w
        };
        let dot = if flags.contains(PropFlags::REQUIRED) { 10.0 } else { 0.0 };
        let line = Font::Body.line();
        let label_cy = if self.stacked && !inline_value { y + line * 0.5 } else { y + ROW_H * 0.5 };
        let font = Font::Body.id();
        let room = (label_right - label_x - dot).max(0.0);
        let color = if flags.contains(PropFlags::READ_ONLY) { p.text_muted } else { p.text_secondary };
        text::paint(
            &painter,
            &font,
            pos2(label_x, label_cy),
            Align2::LEFT_CENTER,
            color,
            label,
            Some(room),
            Clip::End,
        );
        if flags.contains(PropFlags::REQUIRED) {
            let dot_x = label_x + text::width(&ctx, &font, label).min(room) + 5.0;
            painter.circle_filled(pos2(dot_x, label_cy), 2.5, p.required);
        }
        let label_rect = Rect::from_min_max(
            pos2(label_x, label_cy - line * 0.5),
            pos2(label_right.max(label_x), label_cy + line * 0.5),
        );
        self.ui.interact(label_rect, id.with("label"), Sense::hover()).on_hover_ui(|ui| {
            ui.set_max_width(metrics::TOOLTIP_W - 8.0);
            ui.label(RichText::new(label).font(Font::Strong.id()).color(p.text));
            if !help.is_empty() {
                ui.label(RichText::new(help).font(Font::Body.id()).color(p.text_secondary));
            }
        });
        audit::control(
            &ctx,
            label,
            !flags.contains(PropFlags::READ_ONLY),
            None,
            if help.is_empty() { None } else { Some(help) },
            rect,
        );

        // the value: the rest of the row, less the trailing icons
        let value_x = if self.stacked { self.x0 + indent } else { self.x0 + self.label_w + LABEL_GAP };
        let value_w =
            if inline_value { metrics::FRAME } else { (self.x1 - value_x - trailing_w - 2.0).max(em(3.0)) };
        let value_y = if self.stacked && !inline_value {
            y + label_h + (ROW_H - metrics::FRAME) * 0.5
        } else {
            y + (ROW_H - metrics::FRAME) * 0.5
        };
        let value_rect = Rect::from_min_size(pos2(value_x, value_y), Vec2::new(value_w, metrics::FRAME));
        let mut child = self.ui.new_child(
            UiBuilder::new().id_salt(id).max_rect(value_rect).layout(Layout::left_to_right(Align::Center)),
        );
        let mixed = flags.contains(PropFlags::MIXED);
        if mixed {
            ctx.data_mut(|d| d.insert_temp(mixed_id(), true));
        }
        let r = value(&mut child);
        if mixed {
            ctx.data_mut(|d| d.remove::<bool>(mixed_id()));
        }
        let bottom = child.min_rect().max.y.max(rect.max.y);
        self.row = Some(OpenRow { id, rect, value_x, bottom, slots_used: 0, reset: false });
        if flags.contains(PropFlags::MODIFIED)
            && self.trailing(Icon::Undo, "Reset to default")
            && let Some(row) = self.row.as_mut()
        {
            row.reset = true;
        }
        Some(r)
    }

    /// An icon button at the end of the last row (24 pt, a 14 pt glyph). Returns whether it was
    /// pressed. The row reserves the slots it used the frame before.
    pub fn trailing(&mut self, icon: Icon, tip: &str) -> bool {
        let stacked = self.stacked;
        let x1 = self.x1;
        let Some(row) = self.row.as_mut() else {
            return false;
        };
        let slot = f32::from(row.slots_used);
        row.slots_used = row.slots_used.saturating_add(1);
        let x = x1 - (slot + 1.0) * (SLOT + 2.0);
        let cy = if stacked { row.rect.max.y - 13.0 } else { row.rect.center().y };
        let r = Rect::from_min_size(pos2(x, cy - SLOT * 0.5), Vec2::splat(SLOT));
        let salt = (row.id, "trailing", row.slots_used);
        let mut child = self.ui.new_child(UiBuilder::new().id_salt(salt).max_rect(r));
        icon_button_sized(&mut child, icon, tip, false, true, SLOT, metrics::GLYPH_ROW).clicked()
    }

    /// Whether the Reset of the last row (a Modified one) was pressed.
    #[must_use]
    pub fn was_reset(&self) -> bool {
        self.row.as_ref().is_some_and(|r| r.reset)
    }

    /// A sentence under the last row's value (or across the grid with no row open), in Small
    /// type and the tone's colour: why a value is invalid, what it will do.
    pub fn message(&mut self, tone: Tone, text_: &str) {
        if text_.is_empty() {
            return;
        }
        let ctx = self.ui.ctx().clone();
        let p = palette(&ctx);
        let color = if tone == Tone::Muted { p.text_muted } else { p.tone(tone) };
        let (x, y) = match &self.row {
            Some(row) => (row.value_x, row.bottom.max(row.rect.max.y - 2.0)),
            None => (self.x0 + 6.0, self.y),
        };
        let galley = ctx
            .fonts_mut(|f| f.layout(text_.to_owned(), Font::Small.id(), color, (self.x1 - x).max(em(3.0))));
        let h = galley.size().y;
        self.ui.painter().galley(pos2(x, y), galley, color);
        match self.row.as_mut() {
            Some(row) => row.bottom = y + h + 2.0,
            None => self.y = y + h + 4.0,
        }
    }

    /// Free content the width of the grid under the rows: a text block, a picture.
    pub fn block<R>(&mut self, add: impl FnOnce(&mut Ui) -> R) -> R {
        self.close_row();
        let rect = Rect::from_min_max(pos2(self.x0, self.y + 2.0), pos2(self.x1, f32::INFINITY));
        let salt = (self.id, "block", self.y.to_bits());
        let mut child = self
            .ui
            .new_child(UiBuilder::new().id_salt(salt).max_rect(rect).layout(Layout::top_down(Align::Min)));
        let r = add(&mut child);
        self.y = child.min_rect().max.y + 4.0;
        r
    }

    fn finish(mut self) {
        self.close_row();
        let ctx = self.ui.ctx().clone();
        let p = palette(&ctx);
        let bottom = self.y;
        let width = self.x1 - self.x0;
        // the label column's edge is a grip: its share is stored per role
        if !self.stacked && bottom > self.top && width > 0.0 {
            let edge = self.x0 + self.label_w + LABEL_GAP * 0.5;
            let hit = Rect::from_min_max(pos2(edge - 3.0, self.top), pos2(edge + 3.0, bottom));
            let resp = self.ui.interact(hit, self.id.with("label_edge"), Sense::CLICK | Sense::DRAG);
            let held = resp.dragged();
            if resp.hovered() || held {
                ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
            }
            if held && let Some(pointer) = resp.interact_pointer_pos() {
                let label =
                    (pointer.x - self.x0 - LABEL_GAP * 0.5).clamp(em(6.0), em(6.0).max(width - em(8.0)));
                layout::set(&ctx, &self.key, label / width);
            }
            if resp.double_clicked() {
                layout::set(&ctx, &self.key, self.default_frac);
            }
            if resp.hovered() || held {
                let color = if held { p.accent } else { p.accent_bright.gamma_multiply(0.6) };
                self.ui
                    .painter()
                    .line_segment([pos2(edge, self.top), pos2(edge, bottom)], Stroke::new(1.0, color));
            }
        }
        // several grids may share one footer: the one hovered this frame is what it shows
        if let Some((label, help)) = self.hovered.take() {
            let frame = ctx.cumulative_frame_nr();
            ctx.data_mut(|d| d.insert_temp(footer_id(), FooterState { label, help, frame }));
        }
        self.ui.allocate_rect(
            Rect::from_min_max(pos2(self.x0, self.top), pos2(self.x1, bottom.max(self.top))),
            Sense::hover(),
        );
    }
}

/// The band under a panel's grids: the hovered row's label and help, else what it is for.
pub fn help_footer(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let line = Font::Small.line();
    let h = 8.0 * 2.0 + line * 2.0 + 2.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::hover());
    if !ui.is_rect_visible(rect) {
        return;
    }
    let painter = ui.painter();
    painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(1.0, p.border_subtle));
    let state = ctx.data(|d| d.get_temp::<FooterState>(footer_id()));
    let now = ctx.cumulative_frame_nr();
    let font = Font::Small.id();
    let x = rect.min.x + 4.0;
    let w = Some(rect.width() - 8.0);
    match state.filter(|s| s.frame + 1 >= now && !(s.label.is_empty() && s.help.is_empty())) {
        Some(s) => {
            text::paint(
                painter,
                &font,
                pos2(x, rect.min.y + 8.0),
                Align2::LEFT_TOP,
                p.text_secondary,
                &s.label,
                w,
                Clip::End,
            );
            if !s.help.is_empty() {
                text::paint(
                    painter,
                    &font,
                    pos2(x, rect.min.y + 8.0 + line),
                    Align2::LEFT_TOP,
                    p.text_muted,
                    &s.help,
                    w,
                    Clip::End,
                );
            }
        }
        None => {
            text::paint(
                painter,
                &font,
                pos2(x, rect.min.y + 8.0),
                Align2::LEFT_TOP,
                p.text_muted,
                "Point at a property to see what it does.",
                w,
                Clip::End,
            );
        }
    }
}

// ------------------------------------------------------------------------------------------------
// Values
// ------------------------------------------------------------------------------------------------

fn field_frame(ui: &Ui, rect: Rect, hovered: bool, focused: bool) {
    let p = palette(ui.ctx());
    let radius = CornerRadius::same(metrics::RADIUS);
    ui.painter().rect_filled(rect, radius, if hovered || focused { p.input_hover } else { p.input });
    if focused {
        ui.painter().rect_stroke(rect, radius, Stroke::new(1.0, p.accent_bright), StrokeKind::Inside);
    }
}

/// A one-line text field, the width it gets.
pub fn text_field(ui: &mut Ui, value: &mut String) -> Response {
    let hint = if is_mixed(ui) { MULTIPLE } else { "" };
    let w = ui.available_width();
    ui.add(
        TextEdit::singleline(value)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(w)
            .min_size(Vec2::new(w, metrics::FRAME)),
    )
}

/// A number as text: at most `decimals` places, no trailing zeros.
#[must_use]
pub fn format_number(value: f64, decimals: usize) -> String {
    let mut s = format!("{value:.decimals$}");
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
    if s == "-0" {
        s.remove(0);
    }
    s
}

/// The number at the start of what was typed: "12.5 m" is 12.5, "1,5" is 1.5.
#[must_use]
pub fn parse_number(typed: &str) -> Option<f64> {
    let t = typed.trim();
    let end = t
        .find(|c: char| !(c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | ',' | 'e' | 'E')))
        .unwrap_or(t.len());
    let num = t.get(..end)?.replace(',', ".");
    num.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// The shared number field: drags by `speed` per point, a click types. Returns the response and
/// the new value when it changed.
fn number_field(ui: &mut Ui, value: f64, speed: f64, unit: &str, decimals: usize) -> (Response, Option<f64>) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width().max(em(3.0));
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::new(w, metrics::FRAME), Sense::click_and_drag());
    let edit_id = resp.id.with("edit");
    let mut out = None;

    let editing = ctx.data(|d| d.get_temp::<String>(edit_id));
    if let Some(mut buf) = editing {
        let first = !ctx.memory(|m| m.has_focus(edit_id))
            && ctx.data(|d| d.get_temp::<bool>(edit_id.with("asked"))).is_none();
        if first {
            ctx.memory_mut(|m| m.request_focus(edit_id));
            ctx.data_mut(|d| d.insert_temp(edit_id.with("asked"), true));
        }
        field_frame(ui, rect, true, true);
        let edit = ui.put(
            rect,
            TextEdit::singleline(&mut buf)
                .id(edit_id)
                .frame(Frame::NONE)
                .margin(Margin::symmetric(8, 4))
                .font(Font::Body.id())
                .desired_width(rect.width()),
        );
        if edit.lost_focus() || (!first && !edit.has_focus()) {
            if !ctx.input(|i| i.key_pressed(Key::Escape))
                && let Some(v) = parse_number(&buf)
                && v != value
            {
                out = Some(v);
                resp.mark_changed();
            }
            ctx.data_mut(|d| {
                d.remove::<String>(edit_id);
                d.remove::<bool>(edit_id.with("asked"));
            });
        } else {
            ctx.data_mut(|d| d.insert_temp(edit_id, buf));
        }
        return (resp, out);
    }

    if resp.hovered() || resp.dragged() {
        ctx.set_cursor_icon(CursorIcon::ResizeHorizontal);
    }
    if resp.dragged() {
        let acc_id = resp.id.with("acc");
        let start = ctx.data(|d| d.get_temp::<f64>(acc_id)).unwrap_or(value);
        let next = start + f64::from(resp.drag_delta().x) * speed;
        ctx.data_mut(|d| d.insert_temp(acc_id, next));
        let shown = (next * 10f64.powi(decimals as i32)).round() / 10f64.powi(decimals as i32);
        if shown != value {
            out = Some(shown);
            resp.mark_changed();
        }
    } else {
        ctx.data_mut(|d| d.remove::<f64>(resp.id.with("acc")));
        if resp.clicked() {
            ctx.data_mut(|d| d.insert_temp(edit_id, format_number(value, decimals)));
        }
    }
    if ui.is_rect_visible(rect) {
        field_frame(ui, rect, resp.hovered(), resp.has_focus());
        let font = Font::Body.id();
        let painter = ui.painter();
        let x = rect.min.x + 8.0;
        if is_mixed(ui) && out.is_none() {
            painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, MULTIPLE, font, p.text_muted);
        } else {
            let shown = format_number(out.unwrap_or(value), decimals);
            // a number is never cut
            let r = painter.text(pos2(x, rect.center().y), Align2::LEFT_CENTER, &shown, font.clone(), p.text);
            if !unit.is_empty() {
                painter.text(
                    pos2(r.max.x + 4.0, rect.center().y),
                    Align2::LEFT_CENTER,
                    unit,
                    font,
                    p.text_muted,
                );
            }
        }
    }
    (resp, out)
}

/// A number dragged without steppers, its unit after it (m, °, fps); a click types it.
pub fn number(ui: &mut Ui, value: &mut f32, speed: f32, unit: &str) -> Response {
    let (resp, out) = number_field(ui, f64::from(*value), f64::from(speed), unit, 3);
    if let Some(v) = out {
        *value = v as f32;
    }
    resp
}

/// A whole number dragged without steppers, its unit after it.
pub fn integer(ui: &mut Ui, value: &mut i32, speed: f32, unit: &str) -> Response {
    let (resp, out) = number_field(ui, f64::from(*value), f64::from(speed), unit, 0);
    if let Some(v) = out {
        *value = v.round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
    }
    resp
}

/// A switch: a box the accent fills when it is on.
pub fn switch(ui: &mut Ui, value: &mut bool) -> Response {
    let p = palette(ui.ctx());
    let mixed = is_mixed(ui);
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(metrics::FRAME), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, *value, ""));
    if resp.clicked() {
        *value = !*value;
        resp.mark_changed();
    }
    if ui.is_rect_visible(rect) {
        let b = Rect::from_center_size(rect.center(), Vec2::splat(16.0));
        let radius = CornerRadius::same(3);
        if *value && !mixed {
            ui.painter().rect_filled(b, radius, if resp.hovered() { p.primary_hover } else { p.accent });
            icons::draw(ui.painter(), Icon::Check, b.shrink(2.0), p.on_fill);
        } else {
            ui.painter().rect(
                b,
                radius,
                if resp.hovered() { p.input_hover } else { p.input },
                Stroke::new(1.0, p.border_strong),
                StrokeKind::Inside,
            );
            if mixed {
                icons::draw(ui.painter(), Icon::Minus, b.shrink(2.0), p.text_secondary);
            }
        }
        if resp.has_focus() {
            ui.painter().rect_stroke(
                b.expand(2.0),
                CornerRadius::same(5),
                Stroke::new(metrics::FOCUS_RING, p.accent_bright),
                StrokeKind::Outside,
            );
        }
    }
    resp
}

/// One of a few named choices. Returns whether the choice changed.
pub fn combo(ui: &mut Ui, id_salt: &str, current: &mut usize, options: &[&str]) -> bool {
    let shown = if is_mixed(ui) { MULTIPLE } else { options.get(*current).copied().unwrap_or("") };
    let mut changed = false;
    let w = ui.available_width();
    let resp = egui::ComboBox::from_id_salt(("gz-ui.combo", id_salt)).selected_text(shown).width(w).show_ui(
        ui,
        |ui| {
            for (i, option) in options.iter().enumerate() {
                if menu_item(ui, option, MenuItem { selected: i == *current, ..MenuItem::default() })
                    && i != *current
                {
                    *current = i;
                    changed = true;
                }
            }
        },
    );
    audit::control(ui.ctx(), shown, true, None, None, resp.response.rect);
    changed
}

/// A colour as `#rrggbb`, or `#rrggbbaa` when it is not opaque.
#[must_use]
pub fn color_hex(c: Color32) -> String {
    let [r, g, b, a] = c.to_srgba_unmultiplied();
    if a == 255 { format!("#{r:02x}{g:02x}{b:02x}") } else { format!("#{r:02x}{g:02x}{b:02x}{a:02x}") }
}

/// The colour a hex text names: `#rgb`, `#rrggbb` or `#rrggbbaa`, the `#` optional.
#[must_use]
pub fn parse_hex(text_: &str) -> Option<Color32> {
    let h = text_.trim().trim_start_matches('#');
    let byte = |i: usize| h.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    match h.len() {
        3 => {
            let n = |i: usize| h.get(i..=i).and_then(|s| u8::from_str_radix(s, 16).ok()).map(|v| v * 17);
            Some(Color32::from_rgb(n(0)?, n(1)?, n(2)?))
        }
        6 => Some(Color32::from_rgb(byte(0)?, byte(2)?, byte(4)?)),
        8 => Some(Color32::from_rgba_unmultiplied(byte(0)?, byte(2)?, byte(4)?, byte(6)?)),
        _ => None,
    }
}

/// A colour: a swatch that opens a picker, and its hex.
pub fn color(ui: &mut Ui, value: &mut Color32) -> Response {
    let mixed = is_mixed(ui);
    let mut resp = ui
        .scope(|ui| {
            ui.spacing_mut().interact_size = Vec2::splat(metrics::FRAME);
            egui::color_picker::color_edit_button_srgba(ui, value, egui::color_picker::Alpha::OnlyBlend)
        })
        .inner;
    let id = resp.id.with("hex");
    let focused = ui.ctx().memory(|m| m.has_focus(id));
    let mut buf = if focused {
        ui.ctx().data(|d| d.get_temp::<String>(id)).unwrap_or_else(|| color_hex(*value))
    } else if mixed {
        String::new()
    } else {
        color_hex(*value)
    };
    let w = ui.available_width();
    let edit = ui.add(
        TextEdit::singleline(&mut buf)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(if mixed { MULTIPLE } else { "" })
            .desired_width(w)
            .min_size(Vec2::new(w, metrics::FRAME)),
    );
    if edit.changed()
        && let Some(c) = parse_hex(&buf)
        && c != *value
    {
        *value = c;
        resp.mark_changed();
    }
    if edit.has_focus() {
        ui.ctx().data_mut(|d| d.insert_temp(id, buf));
    } else {
        ui.ctx().data_mut(|d| d.remove::<String>(id));
    }
    resp
}

/// Three numbers tagged X, Y, Z in the axes' colours, side by side from 15 em, stacked below.
/// Returns whether one changed.
pub fn vec3_field(ui: &mut Ui, id_salt: &str, value: &mut [f32; 3], speed: f32) -> bool {
    let p = palette(ui.ctx());
    let width = ui.available_width().max(em(9.0));
    let stacked = width < em(15.0);
    let gap = 4.0;
    let each = if stacked { width } else { (width - gap * 2.0) / 3.0 };
    let tag = 18.0;
    let origin = ui.cursor().min;
    let axes = [("X", p.axis_x), ("Y", p.axis_y), ("Z", p.axis_z)];
    let mut changed = false;
    let mut bottom = origin.y;
    for (i, ((letter, tone), v)) in axes.iter().zip(value.iter_mut()).enumerate() {
        let pos = if stacked {
            pos2(origin.x, origin.y + i as f32 * (metrics::FRAME + gap))
        } else {
            pos2(origin.x + i as f32 * (each + gap), origin.y)
        };
        let cell = Rect::from_min_size(pos, Vec2::new(each, metrics::FRAME));
        let field =
            Rect::from_min_max(pos2(cell.min.x + tag - f32::from(metrics::RADIUS), cell.min.y), cell.max);
        let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui.vec3", id_salt, i)).max_rect(field));
        changed |= number(&mut child, v, speed, "").changed();
        // the tag sits over the field's rounded left corners
        let t = Rect::from_min_size(cell.min, Vec2::new(tag, metrics::FRAME));
        let left = CornerRadius { nw: metrics::RADIUS, sw: metrics::RADIUS, ne: 0, se: 0 };
        ui.painter().rect_filled(t, left, p.raised);
        ui.painter().rect_filled(t, left, tone.gamma_multiply(0.22));
        ui.painter().text(t.center(), Align2::CENTER_CENTER, *letter, Font::Strong.id(), *tone);
        bottom = cell.max.y;
    }
    ui.allocate_rect(Rect::from_min_max(origin, pos2(origin.x + width, bottom)), Sense::hover());
    changed
}

/// What a pick field was asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PickAction {
    #[default]
    None,
    /// Open the picker.
    Browse,
    Clear,
}

/// The leaf of a value that is a path: what names the thing.
#[must_use]
pub fn leaf_name(value: &str) -> &str {
    match value.rfind(['\\', '/']) {
        Some(i) if i + 1 < value.len() => value.get(i + 1..).unwrap_or(value),
        _ => value,
    }
}

/// The longest end of `text` that fits `max_w` behind an ellipsis, measured by `measure`, and
/// whether it was cut. A path keeps its end: that is what tells two apart.
fn fit_start(text_: &str, max_w: f32, measure: &dyn Fn(&str) -> f32) -> (String, bool) {
    if measure(text_) <= max_w {
        return (text_.to_owned(), false);
    }
    let starts: Vec<usize> = text_.char_indices().map(|(i, _)| i).collect();
    let mut shown = String::from("…");
    // fewest characters dropped first: binary search on where the kept end starts
    let (mut lo, mut hi) = (1usize, starts.len());
    while lo < hi {
        let mid = (lo + hi) / 2;
        let tail = starts.get(mid).and_then(|&i| text_.get(i..)).unwrap_or("");
        shown.truncate('…'.len_utf8());
        shown.push_str(tail);
        if measure(&shown) <= max_w { hi = mid } else { lo = mid + 1 }
    }
    let tail = starts.get(lo).and_then(|&i| text_.get(i..)).unwrap_or("");
    shown.truncate('…'.len_utf8());
    shown.push_str(tail);
    (shown, true)
}

/// A value picked from a list or a browser: the leaf name with a start ellipsis (the whole value
/// in the tooltip), a chevron, and a clear cross when `clearable` and set.
pub fn pick_field(ui: &mut Ui, id_salt: &str, value: &str, clearable: bool) -> PickAction {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let w = ui.available_width().max(em(6.0));
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, metrics::FRAME), Sense::hover());
    let id = ui.make_persistent_id(("gz-ui.pick", id_salt));
    let resp = ui.interact(rect, id, Sense::click());
    let mut action = if resp.clicked() { PickAction::Browse } else { PickAction::None };
    let has_value = !value.is_empty();
    let chevron = 20.0;
    let clear = if clearable && has_value { 20.0 } else { 0.0 };
    field_frame(ui, rect, resp.hovered(), resp.has_focus());
    let font = Font::Body.id();
    let room = (rect.width() - 16.0 - chevron - clear).max(0.0);
    let pos = pos2(rect.min.x + 8.0, rect.center().y);
    if is_mixed(ui) {
        ui.painter().text(pos, Align2::LEFT_CENTER, MULTIPLE, font, p.text_muted);
    } else if has_value {
        let leaf = leaf_name(value);
        let (shown, cut) = fit_start(leaf, room, &|s| text::width(&ctx, &font, s));
        ui.painter().text(pos, Align2::LEFT_CENTER, &shown, font, p.text);
        if cut {
            audit::truncated(&ctx, value, &shown, rect);
        }
    } else {
        ui.painter().text(pos, Align2::LEFT_CENTER, "None", font, p.text_muted);
    }
    let ch = Rect::from_min_size(
        pos2(rect.max.x - chevron + 4.0, rect.center().y - 6.0),
        Vec2::splat(metrics::GLYPH_STATUS),
    );
    icons::draw(ui.painter(), Icon::ChevronDown, ch, if resp.hovered() { p.text } else { p.text_secondary });
    audit::control(&ctx, id_salt, true, None, None, rect);
    if clear > 0.0 {
        let r = Rect::from_min_size(
            pos2(rect.max.x - chevron - clear, rect.center().y - clear * 0.5),
            Vec2::splat(clear),
        );
        let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("clear")).max_rect(r));
        if icon_button_sized(&mut child, Icon::Clear, "Clear", false, true, clear, metrics::GLYPH_STATUS)
            .clicked()
        {
            action = PickAction::Clear;
        }
    }
    if has_value {
        resp.on_hover_text(value);
    }
    action
}

/// What a path field was asked to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PathAction {
    #[default]
    None,
    /// The text was typed.
    Edited,
    /// Open the system's file or folder browser (the caller owns it).
    Browse,
}

/// A path: its end shown with a start ellipsis, a click to type it, a browse button.
pub fn path_field(ui: &mut Ui, id_salt: &str, value: &mut String, folder: bool) -> PathAction {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let button = metrics::ICON_BUTTON;
    let gap = 6.0;
    let w = (ui.available_width() - button - gap).max(em(4.0));
    let id = ui.make_persistent_id(("gz-ui.path", id_salt));
    let edit_id = id.with("edit");
    let mut action = PathAction::None;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, metrics::FRAME), Sense::hover());
    let editing = ctx.data(|d| d.get_temp::<bool>(edit_id)).unwrap_or(false);
    if editing {
        field_frame(ui, rect, true, true);
        let resp = ui.put(
            rect,
            TextEdit::singleline(value)
                .id(edit_id)
                .frame(Frame::NONE)
                .margin(Margin::symmetric(8, 4))
                .font(Font::Body.id())
                .desired_width(rect.width()),
        );
        if resp.changed() {
            action = PathAction::Edited;
        }
        if resp.lost_focus()
            || (!resp.has_focus() && ctx.data(|d| d.get_temp::<bool>(edit_id.with("asked"))).is_some())
        {
            ctx.data_mut(|d| {
                d.remove::<bool>(edit_id);
                d.remove::<bool>(edit_id.with("asked"));
            });
        } else if !resp.has_focus() {
            resp.request_focus();
            ctx.data_mut(|d| d.insert_temp(edit_id.with("asked"), true));
        }
    } else {
        let resp = ui.interact(rect, id, Sense::click());
        if resp.clicked() {
            ctx.data_mut(|d| d.insert_temp(edit_id, true));
        }
        field_frame(ui, rect, resp.hovered(), resp.has_focus());
        let font = Font::Body.id();
        let pos = pos2(rect.min.x + 8.0, rect.center().y);
        if value.is_empty() {
            ui.painter().text(
                pos,
                Align2::LEFT_CENTER,
                if folder { "No folder selected" } else { "No file selected" },
                font,
                p.text_muted,
            );
        } else {
            let (shown, cut) = fit_start(value, rect.width() - 16.0, &|s| text::width(&ctx, &font, s));
            ui.painter().text(pos, Align2::LEFT_CENTER, &shown, font, p.text);
            if cut {
                audit::truncated(&ctx, value, &shown, rect);
            }
            resp.on_hover_text(value.as_str());
        }
    }
    let r = Rect::from_min_size(pos2(rect.max.x + gap, rect.min.y), Vec2::splat(button));
    let mut child = ui.new_child(UiBuilder::new().id_salt(id.with("browse")).max_rect(r));
    let (icon, tip) =
        if folder { (Icon::FolderOpen, "Browse for a folder") } else { (Icon::Open, "Browse for a file") };
    if icon_button_sized(&mut child, icon, tip, false, true, button, metrics::GLYPH).clicked() {
        action = PathAction::Browse;
    }
    ui.allocate_rect(r, Sense::hover());
    action
}

/// Pills of names, wrapping before one that would pass the row's end; `removable` gives each a
/// cross. Returns the index whose cross was pressed.
pub fn chips(ui: &mut Ui, id_salt: &str, items: &[&str], removable: bool) -> Option<usize> {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let h = metrics::ROW;
    let gap = 4.0;
    let font = Font::Small.id();
    let origin = ui.cursor().min;
    let right = origin.x + ui.available_width();
    let mut x = origin.x;
    let mut y = origin.y;
    let mut removed = None;
    for (i, item) in items.iter().enumerate() {
        let close = if removable { 16.0 } else { 0.0 };
        let w = 8.0 + text::width(&ctx, &font, item) + if removable { 4.0 + close } else { 0.0 } + 8.0;
        if x > origin.x && x + w > right {
            x = origin.x;
            y += h + gap;
        }
        let rect = Rect::from_min_size(pos2(x, y), Vec2::new(w, h));
        x += w + gap;
        if !ui.is_rect_visible(rect) {
            continue;
        }
        ui.painter().rect(
            rect,
            CornerRadius::same(11),
            p.raised,
            Stroke::new(1.0, p.border_subtle),
            StrokeKind::Inside,
        );
        ui.painter().text(
            pos2(rect.min.x + 8.0, rect.center().y),
            Align2::LEFT_CENTER,
            *item,
            font.clone(),
            p.text,
        );
        if removable {
            let r = Rect::from_min_size(
                pos2(rect.max.x - 6.0 - close, rect.center().y - close * 0.5),
                Vec2::splat(close),
            );
            let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui.chip", id_salt, i)).max_rect(r));
            if icon_button_sized(&mut child, Icon::Close, "Remove", false, true, close, 10.0).clicked() {
                removed = Some(i);
            }
        }
    }
    let bottom = if items.is_empty() { origin.y } else { y + h };
    ui.allocate_rect(Rect::from_min_max(origin, pos2(right.max(origin.x), bottom)), Sense::hover());
    removed
}

/// A label above a multi-line text as wide as the column, `lines` lines tall.
pub fn text_block(ui: &mut Ui, label: &str, help: &str, value: &mut String, lines: usize) -> Response {
    let p = palette(ui.ctx());
    ui.horizontal(|ui| {
        ui.add_space(6.0);
        let resp = text::label(ui, label, Font::Body, p.text_secondary, Clip::End);
        if !help.is_empty() {
            resp.on_hover_text(help);
        }
    });
    let w = ui.available_width().max(1.0);
    let resp = ui.add(
        TextEdit::multiline(value)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .desired_rows(lines.max(1))
            .desired_width(w),
    );
    audit::control(ui.ctx(), label, true, None, if help.is_empty() { None } else { Some(help) }, resp.rect);
    ui.add_space(6.0);
    resp
}

/// A value that is shown, not edited: selectable secondary text, its first line only (the whole
/// of it in the tooltip). A path passes `Clip::Start` and keeps its end.
pub fn read_only(ui: &mut Ui, value: &str, clip: Clip) {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let first = value.lines().next().unwrap_or("");
    let font = Font::Body.id();
    let room = (ui.available_width() - 16.0).max(0.0);
    let fits = text::width(&ctx, &font, first) <= room && first.len() == value.len();
    if fits || clip == Clip::End {
        let resp = ui.add(
            egui::Label::new(RichText::new(first).font(font.clone()).color(p.text_secondary))
                .truncate()
                .selectable(true),
        );
        if !fits {
            audit::truncated(&ctx, value, first, resp.rect);
            resp.on_hover_text(value);
        }
    } else {
        let resp = text::label(ui, first, Font::Body, p.text_secondary, clip);
        resp.on_hover_text(value);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        PropFlags, color_hex, fit_start, format_number, label_width, leaf_name, parse_hex, parse_number,
    };
    use egui::Color32;

    #[test]
    fn flags_combine() {
        let f = PropFlags::REQUIRED | PropFlags::INVALID;
        assert!(f.contains(PropFlags::REQUIRED));
        assert!(f.contains(PropFlags::INVALID));
        assert!(!f.contains(PropFlags::MIXED));
        assert!(!f.contains(PropFlags::NONE));
    }

    #[test]
    fn label_column_stays_within_its_limits() {
        // 400 wide at .40 is 160; 6 em = 96, width - 8 em = 272
        assert_eq!(label_width(400.0, 0.40), 160.0);
        assert_eq!(label_width(400.0, 0.10), 96.0);
        assert_eq!(label_width(400.0, 0.90), 272.0);
        // too narrow for both limits: the 6 em floor wins
        assert_eq!(label_width(150.0, 0.40), 96.0);
    }

    #[test]
    fn numbers_read_and_print() {
        assert_eq!(format_number(1.5, 3), "1.5");
        assert_eq!(format_number(2.0, 3), "2");
        assert_eq!(format_number(-0.0001, 3), "0");
        assert_eq!(format_number(12.0, 0), "12");
        assert_eq!(parse_number("12.5 m"), Some(12.5));
        assert_eq!(parse_number(" 1,5"), Some(1.5));
        assert_eq!(parse_number("-3°"), Some(-3.0));
        assert_eq!(parse_number("fps"), None);
    }

    #[test]
    fn hex_round_trips() {
        assert_eq!(color_hex(Color32::from_rgb(0x7a, 0x71, 0xbd)), "#7a71bd");
        assert_eq!(parse_hex("#7a71bd"), Some(Color32::from_rgb(0x7a, 0x71, 0xbd)));
        assert_eq!(parse_hex("fff"), Some(Color32::from_rgb(255, 255, 255)));
        assert_eq!(parse_hex("#12345"), None);
        assert_eq!(parse_hex("#zzzzzz"), None);
    }

    #[test]
    fn the_leaf_names_the_thing() {
        assert_eq!(leaf_name("meshes\\actors\\stalker.ogf"), "stalker.ogf");
        assert_eq!(leaf_name("a/b/c"), "c");
        assert_eq!(leaf_name("plain"), "plain");
        assert_eq!(leaf_name("ends\\"), "ends\\");
    }

    #[test]
    fn a_pick_field_cuts_at_the_start() {
        // one point per character
        let measure = |s: &str| s.chars().count() as f32;
        assert_eq!(fit_start("stalker.ogf", 20.0, &measure), ("stalker.ogf".to_owned(), false));
        let (shown, cut) = fit_start("very_long_model_name.ogf", 10.0, &measure);
        assert!(cut);
        assert_eq!(shown, "…_name.ogf");
        assert!(shown.chars().count() <= 10);
        let (shown, _) = fit_start("абвгдежзик", 4.0, &measure);
        assert_eq!(shown, "…зик");
        let (shown, _) = fit_start("abc", 0.0, &measure);
        assert_eq!(shown, "…");
    }
}
