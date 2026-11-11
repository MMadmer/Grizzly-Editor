//! Text-bound sizes. `em(n)` is n times the Body line (16 pt), the unit the design gives widths
//! in (docs/UI_DESIGN.md, section 2: "Em(n), n x the font size", the font size being 16).
//! [`button_width`] is the width a [`crate::Button`] takes, for rows laid out by hand.

use egui::Context;

use crate::button::Kind;
use crate::font::Font;
use crate::text;
use crate::theme::metrics;

/// One em in points: the Body line.
pub const EM: f32 = Font::Body.line();

/// `n` em in points.
#[must_use]
pub const fn em(n: f32) -> f32 {
    n * EM
}

/// The width [`crate::Button`] takes for this label and kind (8 pt padding each side, the label
/// measured in the face its kind draws with), at least `min_w`.
#[must_use]
pub fn button_width(ctx: &Context, label: &str, kind: Kind, icon: bool, min_w: f32) -> f32 {
    let font = if matches!(kind, Kind::Primary | Kind::Danger) { Font::Strong } else { Font::Body };
    let icon_w = if icon { metrics::GLYPH + if label.is_empty() { 0.0 } else { 6.0 } } else { 0.0 };
    (16.0 + icon_w + text::width(ctx, &font.id(), label)).max(min_w)
}
