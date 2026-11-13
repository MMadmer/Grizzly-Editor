//! Text that fits: measured, cut with an ellipsis where the design says, never clipped silently.
//!
//! [`fit`] is the cut a canvas or a list asks for every frame: it is worked out once per text,
//! face, width and clip and kept, so a frame that draws the same things again neither searches
//! nor allocates. [`ellipsize`] is the search itself, for a one-off.

use std::hash::{BuildHasher, BuildHasherDefault, Hash, Hasher};
use std::sync::Arc;

use egui::{Align2, Color32, Context, FontId, Galley, Id, Painter, Pos2, Rect, Response, Sense, Ui, Vec2};
use rustc_hash::{FxHashMap, FxHasher};

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

/// A text cut to a width by [`fit`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fitted {
    /// What is drawn: the text, or its cut form with an ellipsis.
    pub text: Arc<str>,
    /// It was cut.
    pub cut: bool,
}

/// How long a fitted text nobody asked for is kept, in frames.
const FIT_KEEP: u64 = 600;
/// How often the cache drops what nobody asked for, in frames.
const FIT_SWEEP: u64 = 300;

#[derive(Clone, Default)]
struct FitCache {
    map: FxHashMap<u64, (Fitted, u64)>,
    swept: u64,
    hits: u64,
    misses: u64,
}

#[derive(Clone, Default)]
struct FitHandle(Arc<parking_lot::Mutex<FitCache>>);

fn fit_cache(ctx: &Context) -> FitHandle {
    let id = Id::unique("gz-ui.text.fit");
    ctx.data(|d| d.get_temp::<FitHandle>(id)).unwrap_or_else(|| {
        let made = FitHandle::default();
        ctx.data_mut(|d| d.insert_temp(id, made.clone()));
        made
    })
}

fn fit_key(ctx: &Context, font: &FontId, text: &str, max_w: f32, clip: Clip) -> u64 {
    let mut h: FxHasher = BuildHasherDefault::<FxHasher>::default().build_hasher();
    text.hash(&mut h);
    font.size.to_bits().hash(&mut h);
    font.family.hash(&mut h);
    // a quarter point is finer than any cut can tell apart
    ((max_w * 4.0).round() as i64).hash(&mut h);
    (clip as u8).hash(&mut h);
    ctx.pixels_per_point().to_bits().hash(&mut h);
    h.finish()
}

/// `text` cut to `max_w` the way `clip` says, worked out once and kept: a frame that asks for
/// the same text at the same width again gets the kept answer.
#[must_use]
pub fn fit(ctx: &Context, font: &FontId, text: &str, max_w: f32, clip: Clip) -> Fitted {
    let key = fit_key(ctx, font, text, max_w, clip);
    let frame = ctx.cumulative_frame_nr();
    let cache = fit_cache(ctx);
    {
        let mut c = cache.0.lock();
        if let Some((hit, seen)) = c.map.get_mut(&key) {
            *seen = frame;
            let out = hit.clone();
            c.hits += 1;
            return out;
        }
    }
    let (shown, cut) = ellipsize(ctx, font, text, max_w, clip);
    let out = Fitted { text: Arc::from(shown), cut };
    let mut c = cache.0.lock();
    c.misses += 1;
    c.map.insert(key, (out.clone(), frame));
    if frame.saturating_sub(c.swept) >= FIT_SWEEP {
        c.swept = frame;
        c.map.retain(|_, (_, seen)| frame.saturating_sub(*seen) <= FIT_KEEP);
    }
    out
}

/// How often [`fit`] answered from what it kept and how often it worked a cut out: for tests and
/// `perf.*`.
#[must_use]
pub fn fit_stats(ctx: &Context) -> (u64, u64) {
    let c = fit_cache(ctx).0.lock().clone();
    (c.hits, c.misses)
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
// every argument is one the caller decides at each call; a struct of them would only rename them
#[allow(clippy::too_many_arguments)]
pub fn paint(
    painter: &Painter,
    font: &FontId,
    pos: Pos2,
    anchor: Align2,
    color: Color32,
    text: &str,
    max_w: Option<f32>,
    clip: Clip,
) -> bool {
    let ctx = painter.ctx();
    let Some(w) = max_w else {
        painter.text(pos, anchor, text, font.clone(), color);
        return false;
    };
    let fitted = fit(ctx, font, text, w, clip);
    let rect = painter.text(pos, anchor, &*fitted.text, font.clone(), color);
    if fitted.cut {
        audit::truncated(ctx, text, &fitted.text, rect);
    }
    fitted.cut
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

#[cfg(test)]
mod tests {
    use super::{Clip, fit, fit_stats};
    use crate::font::Font;

    #[test]
    fn a_fitted_text_is_worked_out_once() {
        let ctx = crate::testkit::context();
        let mut cut = None;
        crate::testkit::frame(&ctx, 0.0, Vec::new(), |ui| {
            let font = Font::Body.id();
            let long = "A rather long name of a node that cannot fit";
            let a = fit(ui.ctx(), &font, long, 60.0, Clip::End);
            let (_, misses) = fit_stats(ui.ctx());
            for _ in 0..10 {
                let again = fit(ui.ctx(), &font, long, 60.0, Clip::End);
                assert_eq!(again, a);
            }
            let (hits, misses_after) = fit_stats(ui.ctx());
            assert_eq!(misses_after, misses, "every later ask is a hit");
            assert!(hits >= 10);
            // another width is another cut
            let wider = fit(ui.ctx(), &font, long, 600.0, Clip::End);
            assert!(!wider.cut);
            cut = Some(a.cut && a.text.ends_with('\u{2026}'));
        });
        assert_eq!(cut, Some(true));
    }
}
