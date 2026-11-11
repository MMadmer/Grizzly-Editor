//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::{Align2, CornerRadius, Id, Margin, Rect, Response, Sense, Stroke, TextEdit, Ui, Vec2, pos2};
use gz_ui::text::{self, Clip};
use gz_ui::{Font, Icon, icons, metrics, palette};

/// A folding header outside a property grid (the C++ `XUI::Section`): a chevron and the title,
/// the whole row a click target. `open` is its state until it is clicked; the state is kept by
/// `id_salt`. Returns whether it is open.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool) -> bool {
    let ctx = ui.ctx().clone();
    let p = palette(&ctx);
    let id = ui.make_persistent_id(("gz-ui-app.section", id_salt));
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
    gz_ui::audit::control(&ctx, title, true, None, None, rect);
    is_open
}

/// A one-line text field with a hint while it is empty, `width` wide, in the kit's look (the
/// kit's `text_field` has no hint of its own).
pub fn local_text_field(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str, width: f32) -> Response {
    let id = ui.make_persistent_id(("gz-ui-app.text", id_salt));
    let resp = ui.add(
        TextEdit::singleline(value)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    gz_ui::audit::control(ui.ctx(), hint, true, None, None, resp.rect);
    resp
}

/// The id a [`local_text_field`] gives its edit, to ask for the focus.
#[must_use]
pub fn local_text_field_id(ui: &Ui, id_salt: &str) -> Id {
    ui.make_persistent_id(("gz-ui-app.text", id_salt))
}

/// A one-point rule from `x0` to `x1` at `y`, in the subtle border colour.
pub fn local_rule(ui: &Ui, x0: f32, x1: f32, y: f32) {
    if x1 <= x0 {
        return;
    }
    let p = palette(ui.ctx());
    let y = y.round() + 0.5;
    ui.painter().line_segment([pos2(x0, y), pos2(x1, y)], Stroke::new(1.0, p.border_subtle));
}
