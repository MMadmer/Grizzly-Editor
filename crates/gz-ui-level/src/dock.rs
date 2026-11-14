//! Where the slots of the Level dock go in the work area, and its splitters: side slots keep
//! their pt size, the viewport takes every resize; a slot holds its panels' minimums first and
//! the viewport its 360 x 240, and what still does not fit scrolls inside the panels.

use egui::{Context, Pos2, Rect, Ui, pos2};
use gz_ui::layout as store;
use gz_ui::splitter;

use crate::layout::{
    DockLayout, KEY_BOTTOM, KEY_LEFT, KEY_RIGHT, KEY_RIGHT_TOP, PanelId, SizeClass, Sizes, Slot,
};

/// The gap between two slots (the docking separator).
pub const GAP: f32 = 2.0;
/// The panel tab strip.
pub const STRIP: f32 = gz_ui::metrics::PANEL_TABS;
const VIEWPORT_MIN: (f32, f32) = (360.0, 240.0);

/// A splitter between slots.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Line {
    pub key: &'static str,
    pub vertical: bool,
    pub at: Pos2,
    pub length: f32,
    pub sign: f32,
    pub lo: f32,
    pub hi: f32,
    pub default: f32,
}

/// The rectangles of a frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    pub viewport: Rect,
    pub slots: [Option<Rect>; 4],
    pub lines: Vec<Line>,
}

fn min_of(layout: &DockLayout, s: Slot, horizontal: bool) -> f32 {
    layout
        .panels_in(s)
        .iter()
        .map(|p| if horizontal { p.min_size().0 } else { p.min_size().1 })
        .fold(0.0, f32::max)
}

fn stored(ctx: &Context, key: &str, default: f32) -> f32 {
    store::get(ctx, key, default)
}

/// Lays the slots out in `rect` for this frame.
#[must_use]
pub fn geometry(ctx: &Context, rect: Rect, layout: &DockLayout, class: SizeClass) -> Geometry {
    let d = Sizes::of(class);
    let shown = |s: Slot| layout.slot_shown(s, class);
    let (left, rt, rb, bottom) =
        (shown(Slot::Left), shown(Slot::RightTop), shown(Slot::RightBottom), shown(Slot::Bottom));
    let right = rt || rb;
    let (w, h) = (rect.width(), rect.height());
    let mut lines = Vec::new();

    let lmin = min_of(layout, Slot::Left, true);
    let rmin = min_of(layout, Slot::RightTop, true).max(min_of(layout, Slot::RightBottom, true));
    let right_w = if right {
        let hi = (w - VIEWPORT_MIN.0 - GAP - if left { lmin + GAP } else { 0.0 }).max(rmin);
        stored(ctx, KEY_RIGHT, d.right).clamp(rmin, hi)
    } else {
        0.0
    };
    let left_hi = (w - VIEWPORT_MIN.0 - GAP - if right { right_w + GAP } else { 0.0 }).max(lmin);
    let left_w = if left { stored(ctx, KEY_LEFT, d.left).clamp(lmin, left_hi) } else { 0.0 };

    let x0 = rect.min.x;
    let x1 = rect.max.x;
    let cx0 = if left { x0 + left_w + GAP } else { x0 };
    let cx1 = if right { x1 - right_w - GAP } else { x1 };
    let mut slots = [None; 4];
    if left {
        slots[Slot::Left.index()] = Some(Rect::from_min_max(rect.min, pos2(x0 + left_w, rect.max.y)));
        lines.push(Line {
            key: KEY_LEFT,
            vertical: true,
            at: pos2(x0 + left_w + GAP * 0.5, rect.min.y),
            length: h,
            sign: 1.0,
            lo: lmin,
            hi: left_hi,
            default: d.left,
        });
    }
    if right {
        let hi = (w - VIEWPORT_MIN.0 - GAP - if left { left_w + GAP } else { 0.0 }).max(rmin);
        let col = Rect::from_min_max(pos2(x1 - right_w, rect.min.y), rect.max);
        lines.push(Line {
            key: KEY_RIGHT,
            vertical: true,
            at: pos2(col.min.x - GAP * 0.5, rect.min.y),
            length: h,
            sign: -1.0,
            lo: rmin,
            hi,
            default: d.right,
        });
        match (rt, rb) {
            (true, true) => {
                let tmin = min_of(layout, Slot::RightTop, false) + STRIP;
                let bmin = min_of(layout, Slot::RightBottom, false) + STRIP;
                let default = (h * d.right_top_frac).round();
                let hi = (h - GAP - bmin).max(tmin);
                let top = stored(ctx, KEY_RIGHT_TOP, default).clamp(tmin, hi);
                slots[Slot::RightTop.index()] =
                    Some(Rect::from_min_max(col.min, pos2(col.max.x, col.min.y + top)));
                slots[Slot::RightBottom.index()] =
                    Some(Rect::from_min_max(pos2(col.min.x, col.min.y + top + GAP), col.max));
                lines.push(Line {
                    key: KEY_RIGHT_TOP,
                    vertical: false,
                    at: pos2(col.min.x, col.min.y + top + GAP * 0.5),
                    length: col.width(),
                    sign: 1.0,
                    lo: tmin,
                    hi,
                    default,
                });
            }
            (true, false) => slots[Slot::RightTop.index()] = Some(col),
            _ => slots[Slot::RightBottom.index()] = Some(col),
        }
    }
    let mut viewport = Rect::from_min_max(pos2(cx0, rect.min.y), pos2(cx1.max(cx0), rect.max.y));
    if bottom {
        let bmin = min_of(layout, Slot::Bottom, false) + STRIP;
        let hi = (h - VIEWPORT_MIN.1 - GAP).max(bmin);
        let bh = stored(ctx, KEY_BOTTOM, d.bottom).clamp(bmin, hi);
        let strip =
            Rect::from_min_max(pos2(viewport.min.x, rect.max.y - bh), pos2(viewport.max.x, rect.max.y));
        lines.push(Line {
            key: KEY_BOTTOM,
            vertical: false,
            at: pos2(strip.min.x, strip.min.y - GAP * 0.5),
            length: strip.width(),
            sign: -1.0,
            lo: bmin,
            hi,
            default: d.bottom,
        });
        viewport.max.y = strip.min.y - GAP;
        slots[Slot::Bottom.index()] = Some(strip);
    }
    Geometry { viewport, slots, lines }
}

/// The splitters, drawn after the panels so they win the pointer. Answers whether one moved.
pub fn splitters(ui: &mut Ui, g: &Geometry) -> bool {
    let mut changed = false;
    for l in &g.lines {
        changed |=
            splitter::splitter_at(ui, l.key, l.vertical, l.at, l.length, l.sign, l.lo, l.hi, l.default)
                .changed;
    }
    changed
}

/// The panel a slot shows, if any.
#[must_use]
pub fn front(layout: &DockLayout, s: Slot) -> Option<PanelId> {
    layout.front_of(s)
}

/// What a panel's own window came to this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Floated {
    /// Its window's close was pressed: the panel goes back into its slot.
    pub closed: bool,
    /// Where the window stands: x, y, w, h in pt of the screen.
    pub rect: Option<[f32; 4]>,
}

/// A panel in a window of its own (dragged out of the dock):
/// egui's immediate viewport, titled with the panel's name, where it stood last, at least the
/// panel's minimum size. `body` draws into it.
pub fn floating_window(
    ctx: &Context,
    panel: PanelId,
    stood: Option<[f32; 4]>,
    body: impl FnMut(&mut Ui),
) -> Floated {
    let (min_w, min_h) = panel.min_size();
    let mut builder = egui::ViewportBuilder::default()
        .with_title(panel.label())
        .with_min_inner_size([min_w, min_h + STRIP])
        .with_inner_size([min_w + 80.0, min_h + 200.0]);
    if let Some([x, y, w, h]) = stood {
        builder = builder.with_position([x, y]).with_inner_size([w.max(min_w), h.max(min_h)]);
    }
    let id = egui::ViewportId::from_hash_of(("gz-ui-level.float", panel.id()));
    let mut body = body;
    ctx.show_viewport_immediate(id, builder, |ui, _class| {
        let mut out = Floated::default();
        let (close, inner) = ui.input(|i| (i.viewport().close_requested(), i.viewport().inner_rect));
        out.closed = close;
        out.rect = inner.map(|r| [r.min.x, r.min.y, r.width(), r.height()]);
        let full = ui.max_rect();
        let p = gz_ui::palette(ui.ctx());
        ui.painter().rect_filled(full, egui::CornerRadius::ZERO, p.base);
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .id_salt(("gz-ui-level.float.body", panel.id()))
                .max_rect(full.shrink(8.0))
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        body(&mut child);
        out
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_wide_layout_leaves_the_viewport_its_size() {
        let ctx = Context::default();
        let l = DockLayout::built(SizeClass::Wide, None);
        // 1707 x 912 pt less the 92 pt of chrome
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1707.0, 820.0));
        let g = geometry(&ctx, rect, &l, SizeClass::Wide);
        assert!((g.viewport.width() - 1103.0).abs() < 1.0, "{}", g.viewport.width());
        assert!((g.viewport.height() - 578.0).abs() < 1.0, "{}", g.viewport.height());
    }

    #[test]
    fn the_compact_layout_has_no_strip() {
        let ctx = Context::default();
        let l = DockLayout::built(SizeClass::Compact, None);
        let rect = Rect::from_min_size(pos2(0.0, 0.0), egui::vec2(1280.0, 628.0));
        let g = geometry(&ctx, rect, &l, SizeClass::Compact);
        assert!(g.slots[Slot::Bottom.index()].is_none());
        assert!((g.viewport.width() - 736.0).abs() < 1.0, "{}", g.viewport.width());
    }
}
