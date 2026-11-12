//! A form's widgets outside a property grid: what two surfaces drew with copies of their own
//! (docs/UI_DESIGN.md section 8).
//!
//! - [`section`] - a folding header on a row (Preferences, the PDA preview's Details).
//! - [`raised_section`] - a folding header on a raised strip, with what it holds in a pill at its
//!   right (World Rules, the libraries).
//! - [`check`] - a box and its label, the whole row a click target (Mod Settings, World Rules).
//! - [`hint_field`] - a one-line text field with a hint while it is empty (the kit's
//!   `grid::text_field` has none).
//!
//! Each takes the [`Id`] its state lives under: the caller decides whether it is the panel's
//! (`ui.make_persistent_id`) or one the form reaches from outside (`Id::unique`).

use egui::{
    Align2, CornerRadius, Id, Margin, Rect, Response, Sense, Stroke, StrokeKind, TextEdit, Ui, Vec2, pos2,
};

use crate::text::{self, Clip};
use crate::{Font, Icon, icons, metrics, palette};

/// A folding header a row tall: a chevron and the title, the whole row a click target, a hover
/// fill. `open` is its state until it is clicked. Returns whether it is open.
pub fn section(ui: &mut Ui, id: Id, title: &str, open: bool) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let mut is_open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::ROW), Sense::hover());
    let resp = ui.interact(rect, id.with("header"), Sense::click());
    if resp.clicked() {
        is_open = !is_open;
    }
    ctx.data_mut(|d| d.insert_temp(id, is_open));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if resp.hovered() {
            painter.rect_filled(rect, CornerRadius::same(3), p.hover);
        }
        let chevron = Rect::from_min_size(
            pos2(rect.min.x + 4.0, rect.center().y - metrics::GLYPH_STATUS * 0.5),
            Vec2::splat(metrics::GLYPH_STATUS),
        );
        icons::draw(
            painter,
            if is_open { Icon::ChevronDown } else { Icon::ChevronRight },
            chevron,
            p.text_secondary,
        );
        let x = chevron.max.x + 6.0;
        text::paint(
            painter,
            &Font::Strong.id(),
            pos2(x, rect.center().y),
            Align2::LEFT_CENTER,
            p.text,
            title,
            Some((rect.max.x - x).max(0.0)),
            Clip::End,
        );
    }
    crate::audit::control(&ctx, title, true, None, None, rect);
    is_open
}

/// A folding header on a raised strip a section header tall (the C++ `XUI::Section`): a
/// chevron, the title, and at the right what it holds in a few words, in a pill; the whole strip
/// a click target. `open` is its state until it is clicked. Returns whether it is open.
pub fn raised_section(ui: &mut Ui, id: Id, title: &str, open: bool, right: Option<&str>) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let mut is_open = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(open);
    let (rect, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), metrics::SECTION_HEADER), Sense::hover());
    let resp = ui.interact(rect, id.with("header"), Sense::click());
    if resp.clicked() {
        is_open = !is_open;
    }
    ctx.data_mut(|d| d.insert_temp(id, is_open));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            CornerRadius::same(metrics::RADIUS),
            if resp.hovered() { p.hover } else { p.raised },
        );
        let cy = rect.center().y;
        let chevron = Rect::from_min_size(
            pos2(rect.min.x + 6.0, cy - metrics::GLYPH_STATUS * 0.5),
            Vec2::splat(metrics::GLYPH_STATUS),
        );
        icons::draw(
            painter,
            if is_open { Icon::ChevronDown } else { Icon::ChevronRight },
            chevron,
            p.text_secondary,
        );
        let mut end = rect.max.x - 8.0;
        if let Some(right) = right.filter(|r| !r.is_empty()) {
            let font = Font::Small.id();
            let w = (text::width(&ctx, &font, right) + 12.0).min(rect.width() * 0.5);
            let pill = Rect::from_min_max(pos2(end - w, cy - 9.0), pos2(end, cy + 9.0));
            painter.rect_filled(pill, CornerRadius::same(9), p.base);
            text::paint(
                painter,
                &font,
                pos2(pill.min.x + 6.0, cy),
                Align2::LEFT_CENTER,
                p.text_secondary,
                right,
                Some(w - 12.0),
                Clip::End,
            );
            end = pill.min.x - 6.0;
        }
        let x = chevron.max.x + 6.0;
        text::paint(
            painter,
            &Font::Strong.id(),
            pos2(x, cy),
            Align2::LEFT_CENTER,
            p.text,
            title,
            Some((end - x).max(0.0)),
            Clip::End,
        );
    }
    crate::audit::control(&ctx, title, true, None, None, rect);
    is_open
}

/// A box and its label, `width` wide, the whole row a click target; disabled with the `Ui`.
/// Returns whether it flipped.
pub fn check(ui: &mut Ui, id: Id, label: &str, value: &mut bool, width: f32) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let enabled = ui.is_enabled();
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, metrics::FRAME), Sense::hover());
    let resp = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let mut changed = false;
    if enabled && resp.clicked() {
        *value = !*value;
        changed = true;
    }
    if ui.is_rect_visible(rect) {
        let b = Rect::from_center_size(
            pos2(rect.min.x + metrics::FRAME * 0.5, rect.center().y),
            Vec2::splat(16.0),
        );
        let radius = CornerRadius::same(3);
        if *value {
            let fill = if !enabled {
                p.text_disabled
            } else if resp.hovered() {
                p.primary_hover
            } else {
                p.accent
            };
            ui.painter().rect_filled(b, radius, fill);
            icons::draw(ui.painter(), Icon::Check, b.shrink(2.0), p.on_fill);
        } else {
            ui.painter().rect(
                b,
                radius,
                if resp.hovered() && enabled { p.input_hover } else { p.input },
                Stroke::new(1.0, p.border_strong),
                StrokeKind::Inside,
            );
        }
        if !label.is_empty() {
            let x = rect.min.x + metrics::FRAME + 4.0;
            let color = if enabled { p.text } else { p.text_disabled };
            text::paint(
                ui.painter(),
                &Font::Body.id(),
                pos2(x, rect.center().y),
                Align2::LEFT_CENTER,
                color,
                label,
                Some((rect.max.x - x).max(0.0)),
                Clip::End,
            );
        }
    }
    crate::audit::control(&ctx, label, enabled, None, None, rect);
    changed
}

/// A one-line text field `width` wide, a frame tall, with `hint` while it is empty.
pub fn hint_field(ui: &mut Ui, id: Id, value: &mut String, hint: &str, width: f32) -> Response {
    ui.add(
        TextEdit::singleline(value)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    )
}
