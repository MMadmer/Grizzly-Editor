//! [`Check`] and [`Switch`] - the kit's two on/off controls (docs/UI_DESIGN.md, section 4).
//!
//! - A **check** is a box: "this one of several" - a row of a list, one of a set of tags, a
//!   column of a table. It can be mixed (some of a selection have it).
//! - A **switch** is a track with a knob: a setting that applies at once.
//!
//! One look each, with a label or without, disabled through `Ui::add_enabled_ui` or
//! `.enabled(false, why)` - the reason is the tooltip. The whole row (box and label) is the click
//! target.

use egui::{
    Align2, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo, pos2,
};

use crate::audit;
use crate::font::Font;
use crate::i18n::Tx;
use crate::icons::{self, Icon};
use crate::text::{self, Clip};
use crate::theme::{Palette, metrics, palette};

/// The side of a check's box.
pub const BOX: f32 = 16.0;
/// A switch's track.
pub const TRACK: Vec2 = Vec2::new(28.0, 16.0);

fn focus(ui: &Ui, rect: Rect, radius: u8, p: &Palette) {
    ui.painter().rect_stroke(
        rect.expand(2.0),
        CornerRadius::same(radius + 2),
        Stroke::new(metrics::FOCUS_RING, p.accent_bright),
        StrokeKind::Outside,
    );
}

/// The box of a check painted in `b`: on, off or mixed, hovered, enabled.
pub fn paint_box(ui: &Ui, b: Rect, on: bool, mixed: bool, hovered: bool, enabled: bool) {
    let p = palette(ui.ctx());
    let radius = CornerRadius::same(3);
    let painter = ui.painter();
    if (on || mixed) && enabled {
        painter.rect_filled(b, radius, if hovered { p.primary_hover } else { p.accent });
        icons::draw(painter, if mixed { Icon::Minus } else { Icon::Check }, b.shrink(2.0), p.on_fill);
    } else if on || mixed {
        painter.rect_filled(b, radius, p.text_disabled);
        icons::draw(painter, if mixed { Icon::Minus } else { Icon::Check }, b.shrink(2.0), p.input);
    } else {
        painter.rect(
            b,
            radius,
            if hovered && enabled { p.input_hover } else { p.input },
            Stroke::new(1.0, if hovered && enabled { p.border_strong } else { p.border }),
            StrokeKind::Inside,
        );
    }
}

/// The track and knob of a switch painted in `t`.
pub fn paint_switch(ui: &Ui, t: Rect, on: bool, hovered: bool, enabled: bool) {
    let p = palette(ui.ctx());
    let painter = ui.painter();
    let radius = CornerRadius::same((t.height() * 0.5) as u8);
    let (fill, stroke, knob) = match (on, enabled) {
        (true, true) => (if hovered { p.primary_hover } else { p.accent }, p.accent, p.on_fill),
        (true, false) => (p.text_disabled, p.text_disabled, p.input),
        (false, true) => (
            if hovered { p.input_hover } else { p.input },
            if hovered { p.border_strong } else { p.border },
            p.text_secondary,
        ),
        (false, false) => (p.input, p.border_subtle, p.text_disabled),
    };
    painter.rect(t, radius, fill, Stroke::new(1.0, stroke), StrokeKind::Inside);
    let r = t.height() * 0.5 - 3.0;
    let cx = if on { t.max.x - r - 3.0 } else { t.min.x + r + 3.0 };
    painter.circle_filled(pos2(cx, t.center().y), r, knob);
}

/// A row of `width` (the label's room after the control), the click target of a check or a
/// switch. Returns its rect and response.
fn row(ui: &mut Ui, control_w: f32, label: &str, width: Option<f32>, enabled: bool) -> (Rect, Response) {
    let ctx = ui.ctx().clone();
    let label_w = if label.is_empty() { 0.0 } else { text::measure(&ctx, &Font::Body.id(), label) + 8.0 };
    let w = width.unwrap_or(control_w + label_w).max(control_w);
    ui.allocate_exact_size(
        Vec2::new(w, metrics::FRAME),
        if enabled { Sense::click() } else { Sense::hover() },
    )
}

fn paint_label(ui: &Ui, rect: Rect, x: f32, label: &Tx<'_>, enabled: bool) {
    if label.is_empty() {
        return;
    }
    let p = palette(ui.ctx());
    text::paint(
        ui.painter(),
        &Font::Body.id(),
        pos2(x, rect.center().y),
        Align2::LEFT_CENTER,
        if enabled { p.text } else { p.text_disabled },
        label.borrow(),
        Some((rect.max.x - x).max(0.0)),
        Clip::End,
    );
}

/// A box and its label.
#[must_use = "a check does nothing until it is shown: .show(ui)"]
pub struct Check<'a> {
    value: &'a mut bool,
    label: Tx<'a>,
    mixed: bool,
    enabled: bool,
    why: Tx<'a>,
    width: Option<f32>,
    tip: Tx<'a>,
}

impl<'a> Check<'a> {
    pub fn new(value: &'a mut bool, label: impl Into<Tx<'a>>) -> Self {
        Self {
            value,
            label: label.into(),
            mixed: false,
            enabled: true,
            why: Tx::EMPTY,
            width: None,
            tip: Tx::EMPTY,
        }
    }

    /// Some of what it stands for have it and some do not: a dash in the box. A click sets it.
    pub fn mixed(mut self, mixed: bool) -> Self {
        self.mixed = mixed;
        self
    }

    /// Off, with the reason as its tooltip.
    pub fn enabled(mut self, enabled: bool, why: impl Into<Tx<'a>>) -> Self {
        self.enabled = enabled;
        self.why = why.into();
        self
    }

    /// The row's width: the label is cut to it. Left out: as wide as the label.
    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn tip(mut self, tip: impl Into<Tx<'a>>) -> Self {
        self.tip = tip.into();
        self
    }

    /// Draws it; the response is `changed()` when it flipped.
    pub fn show(self, ui: &mut Ui) -> Response {
        let enabled = self.enabled && ui.is_enabled();
        let ctx = ui.ctx().clone();
        let (label, why_text, tip_text) = (self.label.get(&ctx), self.why.get(&ctx), self.tip.get(&ctx));
        let (rect, mut resp) = row(ui, metrics::FRAME, &label, self.width, enabled);
        resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, *self.value, label.as_ref()));
        if enabled && resp.clicked() {
            *self.value = if self.mixed { true } else { !*self.value };
            resp.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let b = Rect::from_center_size(
                pos2(rect.min.x + metrics::FRAME * 0.5, rect.center().y),
                Vec2::splat(BOX),
            );
            paint_box(ui, b, *self.value, self.mixed, resp.hovered(), enabled);
            if resp.has_focus() {
                focus(ui, b, 3, &palette(ui.ctx()));
            }
            paint_label(ui, rect, rect.min.x + metrics::FRAME + 4.0, &self.label, enabled);
        }
        let why = (!enabled && !why_text.is_empty()).then_some(why_text.as_ref());
        let tip: &str = if !enabled && !why_text.is_empty() { &why_text } else { &tip_text };
        audit::control(&ctx, &label, enabled, why, (!tip.is_empty()).then_some(tip), rect);
        if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
    }
}

/// A switch and its label: a setting that applies at once.
#[must_use = "a switch does nothing until it is shown: .show(ui)"]
pub struct Switch<'a> {
    value: &'a mut bool,
    label: Tx<'a>,
    enabled: bool,
    why: Tx<'a>,
    width: Option<f32>,
    tip: Tx<'a>,
}

impl<'a> Switch<'a> {
    pub fn new(value: &'a mut bool, label: impl Into<Tx<'a>>) -> Self {
        Self { value, label: label.into(), enabled: true, why: Tx::EMPTY, width: None, tip: Tx::EMPTY }
    }

    pub fn enabled(mut self, enabled: bool, why: impl Into<Tx<'a>>) -> Self {
        self.enabled = enabled;
        self.why = why.into();
        self
    }

    pub fn width(mut self, width: f32) -> Self {
        self.width = Some(width);
        self
    }

    pub fn tip(mut self, tip: impl Into<Tx<'a>>) -> Self {
        self.tip = tip.into();
        self
    }

    /// Draws it; the response is `changed()` when it flipped.
    pub fn show(self, ui: &mut Ui) -> Response {
        let enabled = self.enabled && ui.is_enabled();
        let control = TRACK.x + 4.0;
        let ctx = ui.ctx().clone();
        let (label, why_text, tip_text) = (self.label.get(&ctx), self.why.get(&ctx), self.tip.get(&ctx));
        let (rect, mut resp) = row(ui, control, &label, self.width, enabled);
        resp.widget_info(|| WidgetInfo::selected(Role::CheckBox, enabled, *self.value, label.as_ref()));
        if enabled && resp.clicked() {
            *self.value = !*self.value;
            resp.mark_changed();
        }
        if ui.is_rect_visible(rect) {
            let t = Rect::from_min_size(pos2(rect.min.x + 2.0, rect.center().y - TRACK.y * 0.5), TRACK);
            paint_switch(ui, t, *self.value, resp.hovered(), enabled);
            if resp.has_focus() {
                focus(ui, t, 8, &palette(ui.ctx()));
            }
            paint_label(ui, rect, rect.min.x + control + 6.0, &self.label, enabled);
        }
        let why = (!enabled && !why_text.is_empty()).then_some(why_text.as_ref());
        let tip: &str = if !enabled && !why_text.is_empty() { &why_text } else { &tip_text };
        audit::control(&ctx, &label, enabled, why, (!tip.is_empty()).then_some(tip), rect);
        if tip.is_empty() { resp } else { resp.on_hover_text(tip) }
    }
}
