//! Buttons (docs/UI_DESIGN.md, section 4).
//!
//! One Primary per surface: a dialog footer or a toolbar that lays out a second one is a bug of
//! its surface, and a debug build stops on it ([`note_primary`]). The Danger fill goes only on
//! the destructive action; the button that opens the question is [`Kind::GhostDanger`]. A
//! Secondary has a hairline. A toggle that is on (`checked`) - a toolbar button that shows a
//! panel - is a Ghost with a 2 pt accent line under it, never an accent fill. A disabled button
//! says why in its tooltip.

use egui::{
    Align2, Color32, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, Widget,
    WidgetInfo,
};

use crate::audit;
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::text;
use crate::theme::{Palette, metrics, palette};
use crate::units::em;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Kind {
    #[default]
    Secondary,
    /// The one action a surface is for.
    Primary,
    /// The destructive action itself, inside its confirmation.
    Danger,
    /// No fill until hovered: toolbars, row actions.
    Ghost,
    /// A Ghost with error-coloured text: opens a destructive question.
    GhostDanger,
}

struct Look {
    fill: Color32,
    text: Color32,
    font: Font,
    /// The 1 pt outline a Secondary has.
    outline: Option<Color32>,
}

/// Notes a Primary laid out in `scope` (a footer, a toolbar) this frame. One per surface: a
/// second one stops a debug build, naming both. The same button drawn twice in a frame (a sizing
/// pass) counts once.
pub fn note_primary(ctx: &egui::Context, scope: egui::Id, what: &str, label: &str) {
    let frame = ctx.cumulative_frame_nr();
    let id = scope.with("gz-ui.primary");
    let labels = ctx.data_mut(|d| {
        let slot = d.get_temp_mut_or_insert_with::<(u64, Vec<String>)>(id, || (frame, Vec::new()));
        if slot.0 != frame {
            *slot = (frame, Vec::new());
        }
        if !slot.1.iter().any(|l| l == label) {
            slot.1.push(label.to_owned());
        }
        slot.1.clone()
    });
    debug_assert!(
        labels.len() <= 1,
        "two Primary buttons in one {what}: {labels:?} - a surface has one Primary"
    );
}

/// How many Primary buttons `scope` laid out in this frame.
#[must_use]
pub fn primaries(ctx: &egui::Context, scope: egui::Id) -> usize {
    let frame = ctx.cumulative_frame_nr();
    ctx.data(|d| d.get_temp::<(u64, Vec<String>)>(scope.with("gz-ui.primary")))
        .filter(|s| s.0 == frame)
        .map_or(0, |s| s.1.len())
}

/// The line under a toggle that is on.
pub fn toggle_line(painter: &egui::Painter, rect: Rect, color: Color32) {
    let w = (rect.width() - 8.0).max(4.0);
    let line = Rect::from_center_size(
        egui::pos2(rect.center().x, rect.max.y - metrics::TOGGLE_LINE * 0.5),
        Vec2::new(w, metrics::TOGGLE_LINE),
    );
    painter.rect_filled(line, CornerRadius::same(1), color);
}

fn look(p: &Palette, kind: Kind, enabled: bool, hovered: bool, pressed: bool) -> Look {
    if !enabled {
        let fill =
            if matches!(kind, Kind::Ghost | Kind::GhostDanger) { Color32::TRANSPARENT } else { p.input };
        let outline = (kind == Kind::Secondary).then_some(p.border_subtle);
        return Look { fill, text: p.text_disabled, font: Font::Body, outline };
    }
    let pick = |rest: Color32, hover: Color32, press: Color32| {
        if pressed {
            press
        } else if hovered {
            hover
        } else {
            rest
        }
    };
    match kind {
        Kind::Secondary => Look {
            fill: pick(p.button, p.button_hover, p.pressed),
            text: p.text,
            font: Font::Body,
            outline: Some(if hovered { p.border_strong } else { p.border }),
        },
        Kind::Primary => Look {
            fill: pick(p.primary, p.primary_hover, p.primary_pressed),
            text: p.on_fill,
            font: Font::Strong,
            outline: None,
        },
        Kind::Danger => Look {
            fill: pick(p.danger, p.danger_hover, p.danger_pressed),
            text: p.on_fill,
            font: Font::Strong,
            outline: None,
        },
        // on: no fill, the line under it says so
        Kind::Ghost => Look {
            fill: pick(Color32::TRANSPARENT, p.hover, p.pressed),
            text: p.text,
            font: Font::Body,
            outline: None,
        },
        Kind::GhostDanger => Look {
            fill: pick(Color32::TRANSPARENT, p.hover, p.pressed),
            text: p.error,
            font: Font::Body,
            outline: None,
        },
    }
}

fn focus_ring(ui: &Ui, rect: Rect, p: &Palette) {
    ui.painter().rect_stroke(
        rect.expand(1.0),
        CornerRadius::same(metrics::RADIUS + 1),
        Stroke::new(metrics::FOCUS_RING, p.accent_bright),
        StrokeKind::Outside,
    );
}

/// A labelled button.
#[must_use = "a button does nothing until it is added: ui.add(..) or .show(ui)"]
pub struct Button<'a> {
    label: &'a str,
    kind: Kind,
    enabled: bool,
    why: Option<&'a str>,
    label_set: &'a [&'a str],
    icon: Option<Icon>,
    min_size: Vec2,
    checked: bool,
    tip: Option<&'a str>,
}

impl<'a> Button<'a> {
    pub fn new(label: &'a str) -> Self {
        Self {
            label,
            kind: Kind::Secondary,
            enabled: true,
            why: None,
            label_set: &[],
            icon: None,
            min_size: Vec2::new(0.0, metrics::FRAME),
            checked: false,
            tip: None,
        }
    }

    pub fn kind(mut self, kind: Kind) -> Self {
        self.kind = kind;
        self
    }

    pub fn primary(self) -> Self {
        self.kind(Kind::Primary)
    }

    /// Off, with the reason as its tooltip.
    pub fn enabled(mut self, enabled: bool, why: &'a str) -> Self {
        self.enabled = enabled;
        self.why = (!enabled && !why.is_empty()).then_some(why);
        self
    }

    /// Every label this button can show: it keeps the width of the widest, so a label that
    /// changes with the state never moves its neighbours.
    pub fn label_set(mut self, labels: &'a [&'a str]) -> Self {
        self.label_set = labels;
        self
    }

    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    pub fn min_size(mut self, size: Vec2) -> Self {
        self.min_size = size;
        self
    }

    /// A dialog's footer button: at least 88 x 28.
    pub fn dialog(self) -> Self {
        self.min_size(Vec2::new(metrics::DIALOG_BUTTON_W, metrics::DIALOG_BUTTON_H))
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn tip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    pub fn show(self, ui: &mut Ui) -> Response {
        ui.add(self)
    }
}

impl Widget for Button<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let p = palette(ui.ctx());
        let pad = 8.0;
        let gap = 6.0;
        // measured in the widest face a kind can use, so the width does not depend on the state
        let measure_font =
            if matches!(self.kind, Kind::Primary | Kind::Danger) { Font::Strong } else { Font::Body };
        let font_id = measure_font.id();
        let mut text_w = text::width(ui.ctx(), &font_id, self.label);
        for other in self.label_set {
            text_w = text_w.max(text::width(ui.ctx(), &font_id, other));
        }
        let icon_w = if self.icon.is_some() {
            metrics::GLYPH + if self.label.is_empty() { 0.0 } else { gap }
        } else {
            0.0
        };
        let size = Vec2::new(
            (pad * 2.0 + icon_w + text_w).max(self.min_size.x),
            self.min_size.y.max(metrics::FRAME),
        );
        let sense = if self.enabled { Sense::click() } else { Sense::hover() };
        let (rect, mut resp) = ui.allocate_exact_size(size, sense);
        resp.widget_info(|| WidgetInfo::labeled(Role::Button, self.enabled, self.label));

        if ui.is_rect_visible(rect) {
            let l = look(&p, self.kind, self.enabled, resp.hovered(), resp.is_pointer_button_down_on());
            let painter = ui.painter();
            if l.fill != Color32::TRANSPARENT {
                painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS), l.fill);
            }
            if let Some(o) = l.outline {
                painter.rect_stroke(
                    rect,
                    CornerRadius::same(metrics::RADIUS),
                    Stroke::new(1.0, o),
                    StrokeKind::Inside,
                );
            }
            if self.checked && self.enabled {
                toggle_line(painter, rect, p.accent_bright);
            }
            let content_w = icon_w + text::width(ui.ctx(), &l.font.id(), self.label);
            let mut x = rect.center().x - content_w * 0.5;
            if let Some(icon) = self.icon {
                let r = Rect::from_min_size(
                    egui::pos2(x, rect.center().y - metrics::GLYPH * 0.5),
                    Vec2::splat(metrics::GLYPH),
                );
                icons::draw(
                    painter,
                    icon,
                    r,
                    if self.checked && self.enabled { p.accent_bright } else { l.text },
                );
                x += icon_w;
            }
            if !self.label.is_empty() {
                painter.text(
                    egui::pos2(x, rect.center().y),
                    Align2::LEFT_CENTER,
                    self.label,
                    l.font.id(),
                    l.text,
                );
            }
            if resp.has_focus() {
                focus_ring(ui, rect, &p);
            }
        }
        audit::control(ui.ctx(), self.label, self.enabled, self.why, self.tip, rect);
        match (self.enabled, self.why, self.tip) {
            (false, Some(why), _) => resp = resp.on_hover_text(why),
            (_, _, Some(tip)) => resp = resp.on_hover_text(tip),
            _ => {}
        }
        resp
    }
}

/// A square button that shows an icon. Its name is its tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tip: &str, checked: bool, enabled: bool) -> Response {
    icon_button_sized(ui, icon, tip, checked, enabled, metrics::ICON_BUTTON, metrics::GLYPH)
}

/// An icon button of another size: a row action is as tall as its row with a 14 pt glyph.
pub fn icon_button_sized(
    ui: &mut Ui,
    icon: Icon,
    tip: &str,
    checked: bool,
    enabled: bool,
    side: f32,
    glyph: f32,
) -> Response {
    let p = palette(ui.ctx());
    let sense = if enabled { Sense::click() } else { Sense::hover() };
    let (rect, mut resp) = ui.allocate_exact_size(Vec2::splat(side), sense);
    resp.widget_info(|| WidgetInfo::labeled(Role::Button, enabled, tip));
    if ui.is_rect_visible(rect) {
        let hovered = resp.hovered() && enabled;
        let pressed = resp.is_pointer_button_down_on() && enabled;
        let fill = if pressed {
            p.pressed
        } else if hovered {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        if fill != Color32::TRANSPARENT {
            ui.painter().rect_filled(rect, CornerRadius::same(metrics::RADIUS), fill);
        }
        if checked && enabled {
            toggle_line(ui.painter(), rect, p.accent_bright);
        }
        let color = if !enabled {
            p.text_disabled
        } else if checked {
            p.accent_bright
        } else if hovered {
            p.text
        } else {
            p.text_secondary
        };
        icons::draw(ui.painter(), icon, Rect::from_center_size(rect.center(), Vec2::splat(glyph)), color);
        if resp.has_focus() {
            focus_ring(ui, rect, &p);
        }
    }
    audit::control(ui.ctx(), tip, enabled, None, Some(tip), rect);
    if !tip.is_empty() {
        resp = resp.on_hover_text(tip);
    }
    resp
}

/// One choice out of a few, side by side. Returns whether the choice changed.
pub fn segmented(ui: &mut Ui, current: &mut usize, labels: &[&str], icons_of: Option<&[Icon]>) -> bool {
    let p = palette(ui.ctx());
    let font = Font::Body.id();
    let pad = 10.0;
    let widths: Vec<f32> = labels
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let icon_w = if icons_of.and_then(|ic| ic.get(i)).is_some() {
                metrics::GLYPH + if l.is_empty() { 0.0 } else { 6.0 }
            } else {
                0.0
            };
            pad * 2.0 + icon_w + text::width(ui.ctx(), &font, l)
        })
        .collect();
    let total: f32 = widths.iter().sum();
    // what does not fit side by side becomes the kit's combo of the same choices
    if total > ui.available_width() && labels.len() > 1 {
        let widest = widths.iter().copied().fold(0.0, f32::max);
        let w = (widest + metrics::GLYPH + 16.0).min(ui.available_width()).max(em(4.0));
        let items: Vec<crate::pick::PickItem> = labels
            .iter()
            .enumerate()
            .map(|(i, l)| {
                crate::pick::PickItem::new(i.to_string(), *l)
                    .icon(icons_of.and_then(|ic| ic.get(i)).copied().unwrap_or(Icon::None))
            })
            .collect();
        let current_id = current.to_string();
        let salt = ("gz-ui.segmented", labels.first().copied().unwrap_or(""), labels.len());
        let shown = labels.get(*current).copied().unwrap_or("");
        let list = crate::pick::PickList::new(salt, &items).current(&current_id).without_search();
        return match crate::pick::Combo::new(salt, shown).width(w).show(ui, list) {
            Some(i) if i != *current => {
                *current = i;
                true
            }
            _ => false,
        };
    }
    let (rect, _) = ui.allocate_exact_size(Vec2::new(total, metrics::FRAME), Sense::hover());
    let radius = CornerRadius::same(metrics::RADIUS);
    if ui.is_rect_visible(rect) {
        ui.painter().rect_filled(rect, radius, p.input);
    }
    let mut changed = false;
    let mut x = rect.min.x;
    for (i, label) in labels.iter().enumerate() {
        let cell = Rect::from_min_size(egui::pos2(x, rect.min.y), Vec2::new(widths[i], rect.height()));
        x += widths[i];
        let resp =
            ui.interact(cell, ui.scope_id().with(("segmented", rect.min.x.to_bits(), i)), Sense::click());
        resp.widget_info(|| WidgetInfo::selected(Role::RadioButton, true, *current == i, *label));
        if resp.clicked() && *current != i {
            *current = i;
            changed = true;
        }
        if ui.is_rect_visible(cell) {
            let on = *current == i;
            let painter = ui.painter();
            if on {
                painter.rect_filled(
                    cell.shrink(2.0),
                    CornerRadius::same(metrics::RADIUS - 1),
                    p.selection_focused,
                );
            } else if resp.hovered() {
                painter.rect_filled(cell.shrink(2.0), CornerRadius::same(metrics::RADIUS - 1), p.hover);
            }
            let color = if on { p.text } else { p.text_secondary };
            let mut tx = cell.min.x + pad;
            if let Some(icon) = icons_of.and_then(|ic| ic.get(i)) {
                let r = Rect::from_min_size(
                    egui::pos2(tx, cell.center().y - metrics::GLYPH * 0.5),
                    Vec2::splat(metrics::GLYPH),
                );
                icons::draw(painter, *icon, r, if on { p.accent_bright } else { color });
                tx += metrics::GLYPH + if label.is_empty() { 0.0 } else { 6.0 };
            }
            painter.text(egui::pos2(tx, cell.center().y), Align2::LEFT_CENTER, *label, font.clone(), color);
        }
        audit::control(ui.ctx(), label, true, None, None, cell);
    }
    changed
}

/// Draws the keys of a shortcut as keycaps, right-aligned at `right`. Returns their width.
/// `keys` is what is bound now, as text: "Ctrl+Shift+P".
pub fn keycaps(ui: &Ui, right_center: egui::Pos2, keys: &str, enabled: bool) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    let p = palette(ui.ctx());
    let font = Font::Small.id();
    let h = 16.0;
    let pad = 4.0;
    let gap = 3.0;
    let parts: Vec<&str> = keys.split('+').map(str::trim).filter(|s| !s.is_empty()).collect();
    let widths: Vec<f32> =
        parts.iter().map(|k| (text::width(ui.ctx(), &font, k) + pad * 2.0).max(h)).collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * (parts.len().saturating_sub(1)) as f32;
    let mut x = right_center.x - total;
    let color = if enabled { p.text_secondary } else { p.text_disabled };
    for (part, w) in parts.iter().zip(&widths) {
        let r = Rect::from_min_size(egui::pos2(x, right_center.y - h * 0.5), Vec2::new(*w, h));
        ui.painter().rect(r, CornerRadius::same(3), p.input, Stroke::new(1.0, p.border), StrokeKind::Inside);
        ui.painter().text(r.center(), Align2::CENTER_CENTER, *part, font.clone(), color);
        x += w + gap;
    }
    total
}

/// The width [`keycaps`] takes for these keys.
#[must_use]
pub fn keycaps_width(ui: &Ui, keys: &str) -> f32 {
    if keys.is_empty() {
        return 0.0;
    }
    let font = Font::Small.id();
    let parts = keys.split('+').map(str::trim).filter(|s| !s.is_empty());
    let mut total = 0.0;
    let mut n = 0;
    for k in parts {
        total += (text::width(ui.ctx(), &font, k) + 8.0).max(16.0);
        n += 1;
    }
    total + 3.0 * (n as f32 - 1.0).max(0.0)
}
