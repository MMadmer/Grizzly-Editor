//! Widgets the kit lacks, local to this crate until a second surface wants them
//! (docs/UI_DESIGN.md, section 4: "A widget the kit lacks is written as a local one").

use egui::{Id, Response, Stroke, Ui, pos2};
use gz_ui::palette;

/// A folding header outside a property grid (the C++ `XUI::Section`): a chevron and the title,
/// the whole row a click target. `open` is its state until it is clicked; the state is kept by
/// `id_salt`. Returns whether it is open.
pub fn local_section(ui: &mut Ui, id_salt: &str, title: &str, open: bool) -> bool {
    let id = ui.make_persistent_id(("gz-ui-app.section", id_salt));
    gz_ui::form::section(ui, id, title, open)
}

/// A one-line text field with a hint while it is empty, `width` wide, in the kit's look (the
/// kit's `text_field` has no hint of its own).
pub fn local_text_field(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str, width: f32) -> Response {
    let id = ui.make_persistent_id(("gz-ui-app.text", id_salt));
    let resp = gz_ui::form::hint_field(ui, id, value, hint, width);
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
