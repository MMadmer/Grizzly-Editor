//! Cards and pages (docs/UI_DESIGN.md, section 4, "Cards and pages").
//!
//! - [`card`] - a raised box as tall as what it holds, around one part of a surface, with a tone
//!   strip while it asks for attention.
//! - [`page`] - a surface over the whole work area (the Start page): the base colour, 24 x 20 pt
//!   margins, a body that scrolls.
//! - [`page_card`] + [`card_footer`] per [`CardAction`] - a centred card on a page that asks a
//!   question without a modal: status left, buttons right, Enter presses the default.

use egui::{Align2, CornerRadius, Frame, Id, Key, Margin, Modifiers, Popup, Rect, ScrollArea, Sense, Stroke, Ui, UiBuilder, Vec2, pos2};

use crate::audit;
use crate::button::{Button, Kind};
use crate::font::Font;
use crate::text::{self, Clip};
use crate::theme::{Tone, metrics, palette};
use crate::units::{button_width, em};

/// A raised box around one part of a surface, as tall as what it holds, the whole width it gets.
/// `attention` puts a strip in the tone's colour down its left edge.
pub fn card<R>(ui: &mut Ui, id_salt: &str, title: &str, tone: Tone, attention: bool, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    let out = ui.push_id(("gz-ui.card", id_salt), |ui| {
        Frame::new()
            .fill(p.raised)
            .stroke(Stroke::new(1.0, p.border_subtle))
            .corner_radius(CornerRadius::same(metrics::RADIUS_POPUP))
            .inner_margin(Margin::symmetric(12, 10))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                if !title.is_empty() {
                    let color = p.text;
                    text::label(ui, title, Font::Strong, color, Clip::End);
                }
                add(ui)
            })
    });
    let frame = out.inner;
    if attention {
        let r = frame.response.rect;
        let strip = Rect::from_min_max(r.min, pos2(r.min.x + 3.0, r.max.y));
        ui.painter().rect_filled(strip, CornerRadius { nw: metrics::RADIUS_POPUP, sw: metrics::RADIUS_POPUP, ne: 0, se: 0 }, p.tone(tone));
    }
    ui.add_space(4.0);
    frame.inner
}

/// A surface over the whole of `ui`'s rectangle (the work area under the caption): the base
/// colour, 24 x 20 pt margins, a body that scrolls. `title` names it for the audit.
pub fn page<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    let rect = ui.max_rect();
    ui.painter().rect_filled(rect, CornerRadius::ZERO, p.base);
    let ctx = ui.ctx().clone();
    let inner = rect.shrink2(Vec2::new(24.0, 20.0));
    let mut child = ui.new_child(UiBuilder::new().id_salt(("gz-ui.page", title)).max_rect(inner));
    let r = audit::surface(&ctx, title, || {
        ScrollArea::vertical()
            .id_salt(("gz-ui.page.scroll", title))
            .auto_shrink([false, false])
            .show(&mut child, |ui| add(ui))
            .inner
    });
    ui.allocate_rect(rect, Sense::hover());
    r
}

/// What [`page_card`] remembers between frames.
#[derive(Clone, Copy, Debug, Default)]
struct PageCardMem {
    /// Last frame's height: the card is centred by it.
    height: f32,
    last_frame: u64,
    /// Frames in a row it was shown: the key that brought it up must not also answer it.
    shown: u32,
}

fn current_card_id() -> Id {
    Id::unique("gz-ui.page_card.current")
}

/// The card's padding: its footer's line runs across it.
const CARD_PAD: Vec2 = Vec2::new(20.0, 16.0);

/// A card centred on a page, a little above the middle, at most `width` wide, with its title.
/// A card taller than the page makes the page scroll.
pub fn page_card<R>(ui: &mut Ui, id_salt: &str, title: &str, width: f32, add: impl FnOnce(&mut Ui) -> R) -> R {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let id = Id::unique(("gz-ui.page_card", id_salt));
    let frame_nr = ctx.cumulative_frame_nr();
    let mut mem = ctx.data(|d| d.get_temp::<PageCardMem>(id)).unwrap_or_default();
    mem.shown = if mem.last_frame + 1 >= frame_nr && mem.shown > 0 { mem.shown + 1 } else { 1 };
    mem.last_frame = frame_nr;

    let avail = ui.available_size();
    let origin = ui.cursor().min;
    let w = width.min(avail.x);
    let top = if avail.y.is_finite() { ((avail.y - mem.height) * 0.4).max(0.0) } else { 0.0 };
    ui.add_space(top);
    let previous = ctx.data(|d| d.get_temp::<(Id, u32)>(current_card_id()));
    ctx.data_mut(|d| d.insert_temp(current_card_id(), (id, mem.shown)));
    let left = origin.x + (avail.x - w) * 0.5;
    let card_rect = Rect::from_min_size(pos2(left, ui.cursor().min.y), Vec2::new(w, 0.0));
    let mut child = ui.new_child(UiBuilder::new().id_salt(id).max_rect(Rect::from_min_size(card_rect.min, Vec2::new(w, f32::INFINITY))));
    let out = Frame::new()
        .fill(p.raised)
        .stroke(Stroke::new(1.0, p.border_subtle))
        .corner_radius(CornerRadius::same(metrics::RADIUS_POPUP))
        .inner_margin(Margin::symmetric(CARD_PAD.x as i8, CARD_PAD.y as i8))
        .show(&mut child, |ui| {
            ui.set_width(w - CARD_PAD.x * 2.0);
            let color = p.text;
            text::label(ui, title, Font::Title, color, Clip::End);
            ui.add_space(8.0);
            add(ui)
        });
    let used = out.response.rect;
    ui.allocate_rect(used, Sense::hover());
    ctx.data_mut(|d| match previous {
        Some(prev) => {
            d.insert_temp(current_card_id(), prev);
        }
        None => {
            d.remove::<(Id, u32)>(current_card_id());
        }
    });
    mem.height = used.height();
    ctx.data_mut(|d| d.insert_temp(id, mem));
    out.inner
}

/// A button of a [`card_footer`].
#[derive(Clone, Copy, Debug)]
pub struct CardAction<'a> {
    pub label: &'a str,
    pub kind: Kind,
    /// Enter presses it (never a Danger one).
    pub is_default: bool,
    pub enabled: bool,
    /// Why it is off: its tooltip.
    pub why: &'a str,
}

impl<'a> CardAction<'a> {
    #[must_use]
    pub fn new(label: &'a str, kind: Kind) -> Self {
        Self { label, kind, is_default: false, enabled: true, why: "" }
    }

    #[must_use]
    pub fn default_action(mut self) -> Self {
        self.is_default = true;
        self
    }

    #[must_use]
    pub fn enabled(mut self, enabled: bool, why: &'a str) -> Self {
        self.enabled = enabled;
        self.why = why;
        self
    }
}

/// The footer of a [`page_card`]: a line across the card, the status on the left in what the
/// buttons leave, the buttons on the right (at least 88 x 28). Returns the index pressed; Enter
/// presses the default while nothing else has the keys.
pub fn card_footer(ui: &mut Ui, status: &str, tone: Tone, actions: &[CardAction<'_>]) -> Option<usize> {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let gap = 8.0;
    let h = metrics::DIALOG_BUTTON_H;
    ui.add_space(8.0);
    let line_y = ui.cursor().min.y.round();
    let max = ui.max_rect();
    ui.painter().line_segment(
        [pos2(max.min.x - CARD_PAD.x, line_y), pos2(max.max.x + CARD_PAD.x, line_y)],
        Stroke::new(1.0, p.border_subtle),
    );
    ui.add_space(12.0);

    let widths: Vec<f32> = actions.iter().map(|a| button_width(&ctx, a.label, a.kind, false, metrics::DIALOG_BUTTON_W)).collect();
    let total: f32 = widths.iter().sum::<f32>() + gap * actions.len().saturating_sub(1) as f32;
    let room = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(room, h), Sense::hover());
    let status_w = room - total - 12.0;
    if !status.is_empty() && status_w > em(2.0) {
        let color = if tone == Tone::Muted { p.text_secondary } else { p.tone(tone) };
        let cut = text::paint(ui.painter(), &Font::Body.id(), pos2(row.min.x, row.center().y), Align2::LEFT_CENTER, color, status, Some(status_w), Clip::End);
        if cut {
            ui.interact(Rect::from_min_size(row.min, Vec2::new(status_w, h)), ui.make_persistent_id("gz-ui.card_footer.status"), Sense::hover()).on_hover_text(status);
        }
    }
    let mut clicked = None;
    let mut x = row.min.x + (room - total).max(0.0);
    for (i, (a, w)) in actions.iter().zip(&widths).enumerate() {
        let r = Rect::from_min_size(pos2(x, row.min.y), Vec2::new(*w, h));
        let button = Button::new(a.label).kind(a.kind).enabled(a.enabled, a.why).dialog();
        if ui.put(r, button).clicked() {
            clicked = Some(i);
        }
        x += w + gap;
    }

    // Enter presses the default - never a Danger one, and never while a field, a popup or a
    // dialog has the keys, nor on the frame the card came up
    let shown = ctx.data(|d| d.get_temp::<(Id, u32)>(current_card_id())).map_or(2, |(_, n)| n);
    let keys_free = shown > 1
        && ctx.memory(|m| m.focused().is_none() && m.top_modal_layer().is_none())
        && !Popup::is_any_open(&ctx);
    if clicked.is_none()
        && keys_free
        && let Some(i) = actions.iter().position(|a| a.is_default && a.enabled && a.kind != Kind::Danger)
        && ctx.input_mut(|inp| inp.consume_key(Modifiers::NONE, Key::Enter))
    {
        clicked = Some(i);
    }
    clicked
}
