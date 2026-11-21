//! A form's widgets outside a property grid: what two surfaces drew with copies of their own
//! (docs/UI_DESIGN.md section 8).
//!
//! - [`check`] - the kit's [`crate::Check`] for a form that names its state by an `Id`.
//! - [`hint_field`] - a one-line text field with a hint while it is empty (the kit's
//!   `grid::text_field` has none), named to the audit by its hint; [`row_field`] - the same as wide
//!   as the row it is in.
//! - [`secret_field`] - a one-line field whose text is never drawn (a password, a token).
//! - [`area_field`] - a text of several lines as wide as the row, with its outline and a hint.
//!
//! Each takes the [`Id`] its state lives under (`row_field`, a salt it makes one of): the caller
//! decides whether it is the panel's (`ui.make_persistent_id`) or one the form reaches from outside
//! (`Id::unique`).

use std::fmt::Debug;
use std::hash::Hash;

use egui::{CornerRadius, Id, Margin, Response, Stroke, StrokeKind, TextEdit, Ui, Vec2};

use crate::i18n::Tx;
use crate::units::em;
use crate::{Font, audit, metrics, palette};

/// A box and its label, `width` wide, the whole row a click target; disabled with the `Ui`: the
/// kit's [`crate::Check`]. Returns whether it flipped.
pub fn check<'a>(ui: &mut Ui, id: Id, label: impl Into<Tx<'a>>, value: &'a mut bool, width: f32) -> bool {
    ui.push_id(id, |ui| crate::Check::new(value, label).width(width).show(ui).changed()).inner
}

/// A one-line text field `width` wide, a frame tall, with `hint` while it is empty.
pub fn hint_field<'a>(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: impl Into<Tx<'a>>,
    width: f32,
) -> Response {
    let hint = hint.into().text(ui.ctx());
    let hint: &str = &hint;
    let resp = ui.add(
        TextEdit::singleline(value)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 4))
            .hint_text(hint)
            .desired_width(width)
            .min_size(Vec2::new(width, metrics::FRAME)),
    );
    // the hint is what names a field with no label of its own
    if !hint.is_empty() {
        audit::control(ui.ctx(), hint, true, None, None, resp.rect);
    }
    resp
}

/// A [`hint_field`] as wide as the row it is in (3 em at least). Its state lives under
/// `Id::unique(salt)`, so it keeps the keyboard while the form around it scrolls.
pub fn row_field<'a>(
    ui: &mut Ui,
    salt: impl Hash + Debug,
    value: &mut String,
    hint: impl Into<Tx<'a>>,
) -> Response {
    let width = ui.available_width().max(em(3.0));
    hint_field(ui, Id::unique(salt), value, hint, width)
}

/// A one-line field `width` wide whose text is never drawn (a password, a token, an API key):
/// dots, `hint` while it is empty. `hint` names it to the audit.
pub fn secret_field<'a>(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    hint: impl Into<Tx<'a>>,
    width: f32,
) -> Response {
    let hint = hint.into().text(ui.ctx());
    let hint: &str = &hint;
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

/// A text of several lines as wide as the row it is in, `rows` lines tall at first, with `hint`
/// while it is empty and an outline that reads as one input (the focus ring while it has the keys).
/// `label` names it to the audit, the hint its tip.
pub fn area_field<'a>(
    ui: &mut Ui,
    id: Id,
    value: &mut String,
    label: impl Into<Tx<'a>>,
    hint: impl Into<Tx<'a>>,
    rows: usize,
) -> Response {
    let label = label.into().text(ui.ctx());
    let hint = hint.into().text(ui.ctx());
    let hint: &str = &hint;
    let width = ui.available_width().max(em(3.0));
    let resp = ui.add(
        TextEdit::multiline(value)
            .id(id)
            .font(Font::Body.id())
            .margin(Margin::symmetric(8, 6))
            .hint_text(hint)
            .desired_rows(rows)
            .desired_width(width),
    );
    let p = palette(ui.ctx());
    let stroke = if resp.has_focus() {
        Stroke::new(2.0, p.accent_bright)
    } else if resp.hovered() {
        Stroke::new(1.0, p.border)
    } else {
        Stroke::new(1.0, p.border_subtle)
    };
    ui.painter().rect_stroke(resp.rect, CornerRadius::same(4), stroke, StrokeKind::Inside);
    audit::control(ui.ctx(), &label, true, None, Some(hint), resp.rect);
    resp
}
