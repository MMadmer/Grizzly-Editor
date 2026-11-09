//! Text that fits: measured, cut with an ellipsis where the design says, never clipped silently.

use std::sync::Arc;

use egui::{Align2, Color32, Context, FontId, Galley, Painter, Pos2, Rect, Response, Sense, Ui, Vec2};

use crate::audit;
use crate::font::Font;
use crate::theme::palette;

/// Where a text that does not fit loses its characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Clip {
    /// Names.
    End,
    /// "title - id", file names with an extension worth keeping.
    Middle,
    /// Paths: the end is what tells them apart.
    Start,
}

const ELLIPSIS: char = '…';

fn layout(ctx: &Context, text: &str, font: &FontId, color: Color32) -> Arc<Galley> {
    ctx.fonts_mut(|f| f.layout_no_wrap(text.to_owned(), font.clone(), color))
}

/// The width of a text in one line.
#[must_use]
pub fn width(ctx: &Context, font: &FontId, text: &str) -> f32 {
    if text.is_empty() {
        return 0.0;
    }
    layout(ctx, text, font, Color32::WHITE).size().x
}

fn cut(chars: &[char], keep: usize, clip: Clip) -> String {
    let mut out = String::with_capacity(keep * 2 + 3);
    match clip {
        Clip::End => {
            out.extend(&chars[..keep]);
            // no space before the ellipsis
            while out.ends_with(' ') {
                out.pop();
            }
            out.push(ELLIPSIS);
        }
        Clip::Start => {
            out.push(ELLIPSIS);
            out.extend(&chars[chars.len() - keep..]);
        }
        Clip::Middle => {
            let head = keep.div_ceil(2);
            let tail = keep - head;
            out.extend(&chars[..head]);
            out.push(ELLIPSIS);
            out.extend(&chars[chars.len() - tail..]);
        }
    }
    out
}

/// The longest form of `text` that fits `max_w`, and whether it was cut.
#[must_use]
pub fn ellipsize(ctx: &Context, font: &FontId, text: &str, max_w: f32, clip: Clip) -> (String, bool) {
    if max_w <= 0.0 || text.is_empty() {
        return (String::new(), !text.is_empty());
    }
    if width(ctx, font, text) <= max_w {
        return (text.to_owned(), false);
    }
    let chars: Vec<char> = text.chars().collect();
    // binary search on the number of characters kept
    let (mut lo, mut hi) = (0usize, chars.len().saturating_sub(1));
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if width(ctx, font, &cut(&chars, mid, clip)) <= max_w {
            lo = mid;
        } else {
            hi = mid - 1;
        }
    }
    (cut(&chars, lo, clip), true)
}

/// A one-line label that takes the width it is given and cuts itself to it. A cut label shows
/// the whole text as its tooltip and reports itself to the audit.
pub fn label(ui: &mut Ui, text: &str, font: Font, color: Color32, clip: Clip) -> Response {
    let font_id = font.id();
    let avail = ui.available_width();
    let (shown, was_cut) = ellipsize(ui.ctx(), &font_id, text, avail, clip);
    let galley = layout(ui.ctx(), &shown, &font_id, color);
    let size = Vec2::new(galley.size().x.min(avail), font.line().max(galley.size().y));
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let pos = Pos2::new(rect.min.x, rect.center().y - galley.size().y * 0.5);
        ui.painter().galley(pos, galley, color);
    }
    if was_cut {
        audit::truncated(ui.ctx(), text, &shown, rect);
        resp = resp.on_hover_text(text);
    }
    resp
}

/// Body text in the secondary colour, cut at its end.
pub fn secondary(ui: &mut Ui, text: &str) -> Response {
    let color = palette(ui.ctx()).text_secondary;
    label(ui, text, Font::Body, color, Clip::End)
}

/// Paints a text at a point, cut to `max_w` when given. Returns whether it was cut. For rows
/// and canvases that lay out by hand; reports a cut to the audit like the kit's own labels.
pub fn paint(painter: &Painter, font: &FontId, pos: Pos2, anchor: Align2, color: Color32, text: &str, max_w: Option<f32>, clip: Clip) -> bool {
    let ctx = painter.ctx();
    let (shown, was_cut) = match max_w {
        Some(w) => ellipsize(ctx, font, text, w, clip),
        None => (text.to_owned(), false),
    };
    let rect = painter.text(pos, anchor, &shown, font.clone(), color);
    if was_cut {
        audit::truncated(ctx, text, &shown, rect);
    }
    was_cut
}

/// The rectangle a text painted at `pos` would cover.
#[must_use]
pub fn bounds(ctx: &Context, font: &FontId, pos: Pos2, anchor: Align2, text: &str) -> Rect {
    let size = layout(ctx, text, font, Color32::WHITE).size();
    anchor.anchor_size(pos, size)
}

/// Prose: sentences for the author, wrapped at 36 em.
pub fn prose(ui: &mut Ui, text: &str) -> Response {
    let max = (Font::Body.size() * 36.0).min(ui.available_width());
    ui.scope(|ui| {
        ui.set_max_width(max);
        ui.add(egui::Label::new(egui::RichText::new(text).font(Font::Body.id())).wrap())
    })
    .inner
}

/// A sentence in a tone's colour, wrapped like prose.
pub fn status(ui: &mut Ui, tone: crate::theme::Tone, text: &str) -> Response {
    let color = palette(ui.ctx()).tone(tone);
    let max = (Font::Body.size() * 36.0).min(ui.available_width());
    ui.scope(|ui| {
        ui.set_max_width(max);
        ui.add(egui::Label::new(egui::RichText::new(text).font(Font::Body.id()).color(color)).wrap())
    })
    .inner
}
