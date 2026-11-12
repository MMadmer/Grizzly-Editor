//! States (docs/UI_DESIGN.md, section 4, "Lists and tables" and "Cards and pages").
//!
//! - [`empty_state`] - what an empty list or panel says; fits the height it gets (drops the icon
//!   first, then the margins). Returns whether its action was pressed.
//! - [`badge`], [`status_dot`], [`spinner`], [`busy_line`] - small marks of state.
//! - [`banner`] - a toned bar with a sentence and up to two actions.
//! - [`toast`] queues a note, [`draw_toasts`] draws the queue bottom-right, timed out.
//! - [`tone_icon`] - the icon a tone is shown with.

use std::f32::consts::TAU;

use egui::text::LayoutJob;
use egui::{
    Align, Align2, Area, Context, CornerRadius, Id, Order, Painter, Pos2, Rect, Sense, Shape, Stroke, Ui,
    Vec2, pos2,
};

use crate::button::{Button, Kind, icon_button_sized};
use crate::font::Font;
use crate::icons::{self, Icon};
use crate::text::{self, Clip};
use crate::theme::{Palette, Tone, metrics, palette};
use crate::units::{button_width, em};

/// The icon a tone is shown with.
#[must_use]
pub fn tone_icon(tone: Tone) -> Icon {
    match tone {
        Tone::Muted | Tone::Info => Icon::Info,
        Tone::Success => Icon::Success,
        Tone::Warning => Icon::Warning,
        Tone::Error => Icon::Error,
        Tone::Required => Icon::Required,
    }
}

/// The text colour of a tone on a neutral surface: muted reads secondary.
fn tone_text(p: &Palette, tone: Tone) -> egui::Color32 {
    if tone == Tone::Muted { p.text_secondary } else { p.tone(tone) }
}

/// What an empty list or panel says: an icon, a title, a sentence and an action, centred in the
/// region it gets. A short region drops the icon first, then the margins, and keeps the words.
/// Returns whether the action was pressed.
pub fn empty_state(ui: &mut Ui, icon: Icon, title: &str, text_: &str, action: Option<&str>) -> bool {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let avail = ui.available_size();
    let origin = ui.cursor().min;
    let icon_side = 40.0;
    let wrap = (avail.x - 32.0).min(em(36.0)).max(em(8.0));
    let grows = ui.is_sizing_pass() || !avail.y.is_finite();

    let title_h = if title.is_empty() { 0.0 } else { Font::Title.line() };
    let galley = (!text_.is_empty()).then(|| {
        let mut job = LayoutJob::simple(text_.to_owned(), Font::Body.id(), p.text_secondary, wrap);
        job.halign = Align::Center;
        ctx.fonts_mut(|f| f.layout_job(job))
    });
    let text_h = galley.as_ref().map_or(0.0, |g| g.size().y);
    let button_h = if action.is_some() { metrics::FRAME } else { 0.0 };
    let icon_block = icon_side + 12.0;
    let words = title_h
        + if text_h > 0.0 { 6.0 + text_h } else { 0.0 }
        + if button_h > 0.0 { 16.0 + button_h } else { 0.0 };
    let with_icon = icon != Icon::None && (grows || avail.y >= icon_block + words + 32.0);
    let block = if with_icon { icon_block } else { 0.0 } + words;
    let room_y = if avail.y.is_finite() { avail.y } else { block };
    let margin = if grows { 16.0 } else { ((room_y - block) * 0.5).clamp(0.0, 16.0) };
    let top = origin.y + margin.max((room_y - block) * 0.4);
    let cx = origin.x + avail.x * 0.5;

    let mut y = top;
    if with_icon {
        icons::draw(
            ui.painter(),
            icon,
            Rect::from_min_size(pos2(cx - icon_side * 0.5, y), Vec2::splat(icon_side)),
            p.text_muted,
        );
        y += icon_block;
    }
    if title_h > 0.0 {
        text::paint(
            ui.painter(),
            &Font::Title.id(),
            pos2(cx, y + title_h * 0.5),
            Align2::CENTER_CENTER,
            p.text,
            title,
            Some(avail.x - 16.0),
            Clip::End,
        );
        y += title_h;
    }
    if let Some(galley) = galley {
        y += 6.0;
        // a centred galley is placed by the middle of its lines
        ui.painter().galley(pos2(cx, y), galley, p.text_secondary);
        y += text_h;
    }
    let mut pressed = false;
    if let Some(action) = action {
        y += 16.0;
        let w = button_width(&ctx, action, Kind::Primary, false, 0.0);
        let rect = Rect::from_min_size(pos2(cx - w * 0.5, y), Vec2::new(w, metrics::FRAME));
        pressed = ui.put(rect, Button::new(action).primary()).clicked();
        y += button_h;
    }
    let used = Rect::from_min_size(origin, Vec2::new(avail.x, (y - origin.y + margin).max(0.0)));
    ui.allocate_rect(used, Sense::hover());
    pressed
}

/// A small pill: a count, a state word. Muted reads on the raised colour, a tone on its tint.
pub fn badge(ui: &mut Ui, text_: &str, tone: Tone) {
    let p = palette(ui.ctx());
    let font = Font::Small.id();
    let h = 18.0;
    let w = text::width(ui.ctx(), &font, text_) + 12.0;
    let (rect, _) = ui.allocate_exact_size(Vec2::new(w, h), Sense::hover());
    if ui.is_rect_visible(rect) {
        let muted = tone == Tone::Muted;
        ui.painter().rect_filled(rect, CornerRadius::same(9), if muted { p.raised } else { p.tint(tone) });
        ui.painter().text(
            pos2(rect.min.x + 6.0, rect.center().y),
            Align2::LEFT_CENTER,
            text_,
            font,
            tone_text(&p, tone),
        );
    }
}

/// A dot in a tone's colour, with what it means as its tooltip.
pub fn status_dot(ui: &mut Ui, tone: Tone, tip: &str) {
    let p = palette(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(10.0, Font::Body.line()), Sense::hover());
    if ui.is_rect_visible(rect) {
        ui.painter().circle_filled(rect.center(), 4.0, p.tone(tone));
    }
    if !tip.is_empty() {
        resp.on_hover_text(tip);
    }
}

/// How often a spinner asks for its next frame: 30 a second turns it smoothly, and a window
/// that showed one at every frame it could (160 a second) made a level compile beside it 30 %
/// slower (docs/rust/STATE.md, "After phase 8c: the window while a job runs").
pub const SPINNER_FRAME: std::time::Duration = std::time::Duration::from_millis(33);

/// Paints the spinner centred in `rect`: three quarters of a ring, one turn a second. It is the
/// one thing of the UI that animates, so it asks for its next frame ([`SPINNER_FRAME`]).
pub(crate) fn paint_spinner(ctx: &Context, painter: &Painter, rect: Rect) {
    let p = palette(ctx);
    let side = rect.width().min(rect.height());
    let r = side * 0.5 - 1.5;
    let start = (ctx.input(|i| i.time) as f32 % 1.0) * TAU;
    let n = 24;
    let points: Vec<Pos2> = (0..=n)
        .map(|k| {
            let a = start + TAU * 0.75 * k as f32 / n as f32;
            rect.center() + Vec2::new(a.cos(), a.sin()) * r
        })
        .collect();
    painter.add(Shape::line(points, Stroke::new(2.0, p.accent_bright)));
    ctx.request_repaint_after(SPINNER_FRAME);
}

/// A spinner as an item of the row: 16 pt.
pub fn spinner(ui: &mut Ui) {
    let (rect, _) = ui.allocate_exact_size(
        Vec2::new(metrics::GLYPH, Font::Body.line().max(metrics::GLYPH)),
        Sense::hover(),
    );
    if ui.is_rect_visible(rect) {
        let ctx = ui.ctx().clone();
        paint_spinner(&ctx, ui.painter(), Rect::from_center_size(rect.center(), Vec2::splat(metrics::GLYPH)));
    }
}

/// A spinner and what is being done, on one line.
pub fn busy_line(ui: &mut Ui, text_: &str) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        spinner(ui);
        let color = palette(ui.ctx()).text;
        text::label(ui, text_, Font::Body, color, Clip::End);
    });
}

/// A toned bar across the row: an icon, a sentence that wraps, up to two actions on the right.
/// Returns the index of the action pressed.
pub fn banner(ui: &mut Ui, tone: Tone, text_: &str, a1: Option<&str>, a2: Option<&str>) -> Option<usize> {
    let p = palette(ui.ctx());
    let ctx = ui.ctx().clone();
    let origin = ui.cursor().min;
    let width = ui.available_width().max(em(8.0));
    let pad = 10.0;
    let icon = metrics::GLYPH;
    let w1 = a1.map_or(0.0, |a| button_width(&ctx, a, Kind::Secondary, false, 0.0));
    let w2 = a2.map_or(0.0, |a| button_width(&ctx, a, Kind::Secondary, false, 0.0));
    let buttons = w1 + w2 + if w1 > 0.0 && w2 > 0.0 { 6.0 } else { 0.0 };
    let text_x = origin.x + pad + icon + 8.0;
    let wrap =
        (origin.x + width - pad - if buttons > 0.0 { buttons + 12.0 } else { 0.0 } - text_x).max(em(6.0));
    let galley = ctx.fonts_mut(|f| f.layout(text_.to_owned(), Font::Body.id(), p.text, wrap));
    let h = galley.size().y.max(metrics::FRAME) + pad * 2.0;
    let rect = Rect::from_min_size(origin, Vec2::new(width, h));
    ui.allocate_rect(rect, Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(rect, CornerRadius::same(metrics::RADIUS_POPUP), p.tint(tone));
        let strip = Rect::from_min_max(rect.min, pos2(rect.min.x + 3.0, rect.max.y));
        painter.rect_filled(
            strip,
            CornerRadius { nw: metrics::RADIUS_POPUP, sw: metrics::RADIUS_POPUP, ne: 0, se: 0 },
            p.tone(tone),
        );
        let glyph = Rect::from_min_size(
            pos2(origin.x + pad, rect.min.y + pad + (Font::Body.line() - icon) * 0.5),
            Vec2::splat(icon),
        );
        icons::draw(painter, tone_icon(tone), glyph, p.tone(tone));
        painter.galley(pos2(text_x, rect.min.y + pad), galley, p.text);
    }
    let mut clicked = None;
    let by = rect.center().y - metrics::FRAME * 0.5;
    let mut x = rect.max.x - pad - buttons;
    if let Some(a) = a1 {
        if ui.put(Rect::from_min_size(pos2(x, by), Vec2::new(w1, metrics::FRAME)), Button::new(a)).clicked() {
            clicked = Some(0);
        }
        x += w1 + 6.0;
    }
    if let Some(a) = a2
        && ui.put(Rect::from_min_size(pos2(x, by), Vec2::new(w2, metrics::FRAME)), Button::new(a)).clicked()
    {
        clicked = Some(1);
    }
    clicked
}

/// A queued note.
#[derive(Clone, Debug)]
struct ToastItem {
    tone: Tone,
    text: String,
    /// When it was first drawn, in seconds of the context's clock; `None` until then.
    born: Option<f64>,
    serial: u64,
}

#[derive(Clone, Debug, Default)]
struct Toasts {
    items: Vec<ToastItem>,
    serial: u64,
}

fn toasts_id() -> Id {
    Id::unique("gz-ui.toasts")
}

/// The most notes on screen at once: a fifth pushes the oldest out.
const MAX_TOASTS: usize = 4;
const TOAST_W: f32 = metrics::TOOLTIP_W;

/// Queues a note for [`draw_toasts`]. It stays 6 seconds, an error 10.
pub fn toast(ctx: &Context, tone: Tone, text_: &str) {
    if text_.is_empty() {
        return;
    }
    ctx.data_mut(|d| {
        let t = d.get_temp_mut_or_default::<Toasts>(toasts_id());
        t.serial += 1;
        let serial = t.serial;
        t.items.push(ToastItem { tone, text: text_.to_owned(), born: None, serial });
        while t.items.len() > MAX_TOASTS {
            t.items.remove(0);
        }
    });
    ctx.request_repaint();
}

/// The texts of the notes queued or on screen, oldest first: what MCP answers of them.
#[must_use]
pub fn toast_texts(ctx: &Context) -> Vec<String> {
    ctx.data(|d| d.get_temp::<Toasts>(toasts_id()))
        .map(|t| t.items.into_iter().map(|i| i.text).collect())
        .unwrap_or_default()
}

/// Draws the queued notes stacked in the bottom-right corner of `ui`'s rectangle (the work area)
/// above everything else, up to 360 pt wide, each with a cross to dismiss it.
pub fn draw_toasts(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    let now = ctx.input(|i| i.time);
    let list = ctx.data_mut(|d| {
        let t = d.get_temp_mut_or_default::<Toasts>(toasts_id());
        for item in &mut t.items {
            item.born.get_or_insert(now);
        }
        t.items.retain(|item| {
            now - item.born.unwrap_or(now) <= if item.tone == Tone::Error { 10.0 } else { 6.0 }
        });
        t.items.clone()
    });
    if list.is_empty() {
        return;
    }
    // the next expiry needs a frame of its own
    let next = list
        .iter()
        .map(|item| (if item.tone == Tone::Error { 10.0 } else { 6.0 }) - (now - item.born.unwrap_or(now)))
        .fold(f64::MAX, f64::min);
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(next.max(0.05)));

    let p = palette(&ctx);
    let corner = ui.max_rect().right_bottom() - Vec2::splat(16.0);
    let area = Area::new(Id::unique("gz-ui.toasts.area"))
        .order(Order::Foreground)
        .pivot(Align2::RIGHT_BOTTOM)
        .fixed_pos(corner)
        .interactable(true);
    let layer = area.layer();
    let mut dismissed = None;
    area.show(&ctx, |ui| {
        ui.spacing_mut().item_spacing = Vec2::new(6.0, 6.0);
        ui.set_width(TOAST_W);
        for item in &list {
            ui.push_id(item.serial, |ui| {
                let inner_w = TOAST_W - 24.0;
                let text_w = inner_w - 22.0 - 24.0;
                let galley =
                    ui.ctx().fonts_mut(|f| f.layout(item.text.clone(), Font::Body.id(), p.text, text_w));
                let h = galley.size().y.max(metrics::GLYPH) + 20.0;
                let (rect, _) = ui.allocate_exact_size(Vec2::new(TOAST_W, h), Sense::hover());
                let painter = ui.painter();
                painter.rect(
                    rect,
                    CornerRadius::same(metrics::RADIUS_POPUP),
                    p.overlay,
                    Stroke::new(1.0, p.border_strong),
                    egui::StrokeKind::Inside,
                );
                let glyph = Rect::from_min_size(
                    pos2(rect.min.x + 12.0, rect.min.y + 10.0 + (Font::Body.line() - metrics::GLYPH) * 0.5),
                    Vec2::splat(metrics::GLYPH),
                );
                icons::draw(painter, tone_icon(item.tone), glyph, p.tone(item.tone));
                painter.galley(pos2(rect.min.x + 12.0 + 22.0, rect.min.y + 10.0), galley, p.text);
                let close = Rect::from_min_size(
                    pos2(rect.max.x - 12.0 - 20.0 + 4.0, rect.min.y + 8.0),
                    Vec2::splat(20.0),
                );
                let mut child = ui.new_child(egui::UiBuilder::new().id_salt("dismiss").max_rect(close));
                if icon_button_sized(
                    &mut child,
                    Icon::Close,
                    "Dismiss",
                    false,
                    true,
                    20.0,
                    metrics::GLYPH_STATUS,
                )
                .clicked()
                {
                    dismissed = Some(item.serial);
                }
            });
        }
    });
    ctx.move_to_top(layer);
    if let Some(serial) = dismissed {
        ctx.data_mut(|d| {
            d.get_temp_mut_or_default::<Toasts>(toasts_id()).items.retain(|t| t.serial != serial)
        });
    }
}
