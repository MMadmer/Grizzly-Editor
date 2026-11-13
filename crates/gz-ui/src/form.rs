//! A form's widgets outside a property grid: what two surfaces drew with copies of their own
//! (docs/UI_DESIGN.md section 8).
//!
//! - [`section`], [`raised_section`] - the kit's one fold header ([`crate::Section`]) for a form
//!   that names its state by an `Id`; the second shows a count.
//! - [`check`] - the kit's [`crate::Check`] for a form that names its state by an `Id`.
//! - [`hint_field`] - a one-line text field with a hint while it is empty (the kit's
//!   `grid::text_field` has none).
//!
//! Each takes the [`Id`] its state lives under: the caller decides whether it is the panel's
//! (`ui.make_persistent_id`) or one the form reaches from outside (`Id::unique`).

use egui::{Id, Margin, Response, TextEdit, Ui, Vec2};

use crate::{Font, metrics};

/// The key a form's `Id` keeps a section's state under in the layout store.
fn key_of(id: Id) -> String {
    format!("form.{:016x}", id.value())
}

/// A folding header: the kit's [`crate::Section`] under the key of `id`. `open` is its state
/// until it is clicked. Returns whether it is open.
pub fn section(ui: &mut Ui, id: Id, title: &str, open: bool) -> bool {
    let key = key_of(id);
    crate::Section::new(&key, title).default_open(open).show(ui).open
}

/// A folding header with what it holds in a few words after the title: the kit's
/// [`crate::Section`] with a count. Returns whether it is open.
pub fn raised_section(ui: &mut Ui, id: Id, title: &str, open: bool, right: Option<&str>) -> bool {
    let key = key_of(id);
    let mut s = crate::Section::new(&key, title).default_open(open);
    if let Some(r) = right {
        s = s.count(r);
    }
    s.show(ui).open
}

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
