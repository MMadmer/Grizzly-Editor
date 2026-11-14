//! What the Start page and the shell's own surfaces draw that no other surface does.

use egui::{Response, Stroke, Ui, pos2};
use gz_ui::palette;

/// The kit's hint field `width` wide, named to the audit by its hint.
pub fn local_text_field(ui: &mut Ui, id_salt: &str, value: &mut String, hint: &str, width: f32) -> Response {
    let id = ui.make_persistent_id(("gz-ui-app.text", id_salt));
    let resp = gz_ui::form::hint_field(ui, id, value, hint, width);
    gz_ui::audit::control(ui.ctx(), hint, true, None, None, resp.rect);
    resp
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
