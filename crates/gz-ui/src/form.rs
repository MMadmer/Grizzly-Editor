//! A form's widgets outside a property grid: what two surfaces drew with copies of their own
//! (docs/UI_DESIGN.md section 8).
//!
//! - [`check`] - the kit's [`crate::Check`] for a form that names its state by an `Id`.
//! - [`hint_field`] - a one-line text field with a hint while it is empty (the kit's
//!   `grid::text_field` has none).
//! - [`secret_field`] - a one-line field whose text is never drawn (a password, a token).
//!
//! Each takes the [`Id`] its state lives under: the caller decides whether it is the panel's
//! (`ui.make_persistent_id`) or one the form reaches from outside (`Id::unique`).

use egui::{Id, Margin, Response, TextEdit, Ui, Vec2};

use crate::{Font, audit, metrics};

/// A box and its label, `width` wide, the whole row a click target; disabled with the `Ui`: the
/// kit's [`crate::Check`]. Returns whether it flipped.
pub fn check(ui: &mut Ui, id: Id, label: &str, value: &mut bool, width: f32) -> bool {
    ui.push_id(id, |ui| crate::Check::new(value, label).width(width).show(ui).changed()).inner
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

/// A one-line field `width` wide whose text is never drawn (a password, a token, an API key):
/// dots, `hint` while it is empty. `hint` names it to the audit.
pub fn secret_field(ui: &mut Ui, id: Id, value: &mut String, hint: &str, width: f32) -> Response {
    let resp = ui.add(
        TextEdit::singleline(value)
            .id(id)
            .password(true)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    audit::control(ui.ctx(), hint, true, None, None, resp.rect);
    resp
}
