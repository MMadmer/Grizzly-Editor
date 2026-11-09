//! Splitters (docs/UI_DESIGN.md, section 4, "Splitters"): a 1 pt line with a 6 pt hit area,
//! re-clamped every frame so it builds no debt, reset by a double-click, kept in the layout store.
//!
//! - [`splitter`] - sizes the pane before the line; call it where that pane starts.
//! - [`splitter_tail`] - sizes the pane after the line (a side or bottom panel that keeps its size
//!   while the other pane takes every resize).
//! - [`splitter_at`] - a line placed by hand (the graph tab's edges).
//! - [`clamp_head`] / [`clamp_tail`] - the clamping, as pure functions.
//!
//! "Vertical" is the line: a vertical splitter has its panes side by side.

use egui::{CursorIcon, Id, Pos2, Rect, Sense, Stroke, Ui, pos2};

use crate::audit;
use crate::layout;
use crate::theme::palette;

/// What a splitter decided this frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Split {
    /// The size of the pane the splitter sizes, in points.
    pub size: f32,
    /// The author dragged or reset it this frame.
    pub changed: bool,
}

const GRIP: f32 = 3.0;
const TIP: &str = "Drag to resize, double-click to reset";

/// The size of the pane before the line: `stored` clamped so both panes keep their minimum in
/// `total` (the gap between them included). The first pane's minimum wins when both cannot.
#[must_use]
pub fn clamp_head(stored: f32, min_a: f32, min_b: f32, total: f32, gap: f32) -> f32 {
    let max_a = min_a.max(total - gap - min_b);
    stored.clamp(min_a, max_a)
}

/// The size of the pane after the line: `stored` clamped so both keep their minimum in `total`.
/// The tail's minimum wins when both cannot.
#[must_use]
pub fn clamp_tail(stored: f32, min_head: f32, min_tail: f32, total: f32, gap: f32) -> f32 {
    let max_tail = min_tail.max(total - gap - min_head);
    stored.clamp(min_tail, max_tail)
}

/// A value dragged from where the drag began: `start` plus the pointer's travel times `sign`,
/// clamped to `lo..=hi` (`hi` below `lo` reads as `lo`).
#[must_use]
pub fn clamp_at(start: f32, travel: f32, sign: f32, lo: f32, hi: f32) -> f32 {
    (start + sign * travel).clamp(lo, hi.max(lo))
}

fn hit_rect(vertical: bool, at: f32, from: f32, length: f32) -> Rect {
    if vertical {
        Rect::from_min_max(pos2(at - GRIP, from), pos2(at + GRIP, from + length.max(1.0)))
    } else {
        Rect::from_min_max(pos2(from, at - GRIP), pos2(from + length.max(1.0), at + GRIP))
    }
}

fn paint_line(ui: &Ui, vertical: bool, at: f32, from: f32, length: f32, hovered: bool, held: bool) {
    let p = palette(ui.ctx());
    let color = if held {
        p.accent
    } else if hovered {
        p.accent_bright.gamma_multiply(0.6)
    } else {
        p.border_subtle
    };
    let stroke = Stroke::new(if held || hovered { 2.0 } else { 1.0 }, color);
    let (a, b) = if vertical { (pos2(at, from), pos2(at, from + length)) } else { (pos2(from, at), pos2(from + length, at)) };
    ui.painter().line_segment([a, b], stroke);
}

/// The line's interaction: hover cursor, where the pointer grabbed it (kept while dragging).
struct Grab {
    hovered: bool,
    /// Pressed or dragged: how the line is painted.
    held: bool,
    /// Being dragged, with the drag's start known: what moves the value.
    dragging: bool,
    double: bool,
    /// The pointer on the axis, and the pointer at the start of the drag.
    pointer: Option<f32>,
    start: Option<f32>,
}

fn grab(ui: &Ui, id: Id, hit: Rect, vertical: bool, key: &str) -> Grab {
    // not a Tab stop: the line is for the pointer
    let resp = ui.interact(hit, id, Sense::CLICK | Sense::DRAG);
    let hovered = resp.hovered();
    let dragging = resp.dragged();
    let held = dragging || resp.is_pointer_button_down_on();
    if hovered || held {
        ui.ctx().set_cursor_icon(if vertical { CursorIcon::ResizeHorizontal } else { CursorIcon::ResizeVertical });
    }
    let axis = |p: Pos2| if vertical { p.x } else { p.y };
    let pointer = resp.interact_pointer_pos().or_else(|| ui.ctx().pointer_latest_pos()).map(axis);
    let start_id = id.with("grab");
    if resp.drag_started()
        && let Some(at) = pointer
    {
        ui.ctx().data_mut(|d| d.insert_temp(start_id, at));
    }
    let start = if dragging { ui.ctx().data(|d| d.get_temp::<f32>(start_id)) } else { None };
    let double = resp.double_clicked();
    if hovered && !held {
        resp.clone().on_hover_text(TIP);
    }
    audit::control(ui.ctx(), key, true, None, Some(TIP), hit);
    Grab { hovered, held, dragging: dragging && start.is_some(), double, pointer, start }
}

/// A splitter for the pane that starts at the cursor: returns its size, `default` until the
/// author drags it. `min_a` / `min_b` are the panes' minimums, in points.
pub fn splitter(ui: &mut Ui, key: &str, vertical: bool, min_a: f32, min_b: f32, default: f32) -> Split {
    let avail = ui.available_size();
    let start = ui.cursor().min;
    let gap = if vertical { ui.spacing().item_spacing.x } else { ui.spacing().item_spacing.y };
    let total = if vertical { avail.x } else { avail.y };
    let origin = if vertical { start.x } else { start.y };
    let (from, length) = if vertical { (start.y, avail.y) } else { (start.x, avail.x) };
    let ctx = ui.ctx().clone();
    // clamped anew every frame, never written back unless dragged
    let mut size = clamp_head(layout::get(&ctx, key, default), min_a, min_b, total, gap);
    let line = origin + size + gap * 0.5;
    let id = Id::unique(("gz-ui.splitter", key));
    let g = grab(ui, id, hit_rect(vertical, line, from, length), vertical, key);
    let mut changed = false;
    if g.dragging
        && let (Some(pointer), Some(begin)) = (g.pointer, g.start)
    {
        // from where the pointer is, not from how far it moved: past a limit and back, the line follows at once
        let grabbed_at = ctx.data(|d| d.get_temp::<f32>(id.with("offset"))).unwrap_or_else(|| begin - line);
        ctx.data_mut(|d| d.insert_temp(id.with("offset"), grabbed_at));
        let next = clamp_head(pointer - grabbed_at - gap * 0.5 - origin, min_a, min_b, total, gap);
        if next != size {
            size = next;
            layout::set(&ctx, key, size);
            changed = true;
        }
    } else {
        ctx.data_mut(|d| d.remove::<f32>(id.with("offset")));
    }
    if g.double {
        size = clamp_head(default, min_a, min_b, total, gap);
        layout::set(&ctx, key, default);
        changed = true;
    }
    paint_line(ui, vertical, origin + size + gap * 0.5, from, length, g.hovered, g.held);
    Split { size, changed }
}

/// A splitter for the pane that ends where the region ends: returns that pane's size.
pub fn splitter_tail(ui: &mut Ui, key: &str, vertical: bool, min_head: f32, min_tail: f32, default_tail: f32) -> Split {
    let avail = ui.available_size();
    let start = ui.cursor().min;
    let gap = if vertical { ui.spacing().item_spacing.x } else { ui.spacing().item_spacing.y };
    let total = if vertical { avail.x } else { avail.y };
    let end = if vertical { start.x + avail.x } else { start.y + avail.y };
    let (from, length) = if vertical { (start.y, avail.y) } else { (start.x, avail.x) };
    let ctx = ui.ctx().clone();
    let mut tail = clamp_tail(layout::get(&ctx, key, default_tail), min_head, min_tail, total, gap);
    let line = end - tail - gap * 0.5;
    let id = Id::unique(("gz-ui.splitter", key));
    let g = grab(ui, id, hit_rect(vertical, line, from, length), vertical, key);
    let mut changed = false;
    if g.dragging
        && let (Some(pointer), Some(begin)) = (g.pointer, g.start)
    {
        let grabbed_at = ctx.data(|d| d.get_temp::<f32>(id.with("offset"))).unwrap_or_else(|| begin - line);
        ctx.data_mut(|d| d.insert_temp(id.with("offset"), grabbed_at));
        let next = clamp_tail(end - (pointer - grabbed_at) - gap * 0.5, min_head, min_tail, total, gap);
        if next != tail {
            tail = next;
            layout::set(&ctx, key, tail);
            changed = true;
        }
    } else {
        ctx.data_mut(|d| d.remove::<f32>(id.with("offset")));
    }
    if g.double {
        tail = clamp_tail(default_tail, min_head, min_tail, total, gap);
        layout::set(&ctx, key, default_tail);
        changed = true;
    }
    paint_line(ui, vertical, end - tail - gap * 0.5, from, length, g.hovered, g.held);
    Split { size: tail, changed }
}

/// A splitter whose line the caller placed: `line` is its start, `length` how long it runs.
/// The value grows by the pointer's travel times `sign` (+1 when the pane lies before the line,
/// -1 after it), within `lo..=hi`.
pub fn splitter_at(ui: &mut Ui, key: &str, vertical: bool, line: Pos2, length: f32, sign: f32, lo: f32, hi: f32, default: f32) -> Split {
    let ctx = ui.ctx().clone();
    let (at, from) = if vertical { (line.x, line.y) } else { (line.y, line.x) };
    let id = Id::unique(("gz-ui.splitter", key));
    let g = grab(ui, id, hit_rect(vertical, at, from, length), vertical, key);
    let stored = layout::get(&ctx, key, default);
    let mut size = stored.clamp(lo, hi.max(lo));
    let mut changed = false;
    if g.dragging
        && let (Some(pointer), Some(begin)) = (g.pointer, g.start)
    {
        let value_id = id.with("value");
        let start_value = ctx.data(|d| d.get_temp::<f32>(value_id)).unwrap_or(size);
        ctx.data_mut(|d| d.insert_temp(value_id, start_value));
        let next = clamp_at(start_value, pointer - begin, sign, lo, hi);
        if next != stored {
            layout::set(&ctx, key, next);
            changed = true;
        }
        size = next;
    } else {
        ctx.data_mut(|d| d.remove::<f32>(id.with("value")));
    }
    if g.double {
        layout::set(&ctx, key, default);
        size = default.clamp(lo, hi.max(lo));
        changed = true;
    }
    paint_line(ui, vertical, at, from, length, g.hovered, g.held);
    Split { size, changed }
}

#[cfg(test)]
mod tests {
    use super::{clamp_at, clamp_head, clamp_tail};

    #[test]
    fn head_keeps_both_minimums() {
        // 600 wide, 8 gap: the head may take 600 - 8 - 200 = 392 at most
        assert_eq!(clamp_head(500.0, 100.0, 200.0, 600.0, 8.0), 392.0);
        assert_eq!(clamp_head(50.0, 100.0, 200.0, 600.0, 8.0), 100.0);
        assert_eq!(clamp_head(250.0, 100.0, 200.0, 600.0, 8.0), 250.0);
    }

    #[test]
    fn head_minimum_wins_when_both_cannot() {
        assert_eq!(clamp_head(300.0, 100.0, 200.0, 150.0, 8.0), 100.0);
    }

    #[test]
    fn tail_keeps_both_minimums() {
        assert_eq!(clamp_tail(500.0, 400.0, 220.0, 900.0, 8.0), 492.0);
        assert_eq!(clamp_tail(100.0, 400.0, 220.0, 900.0, 8.0), 220.0);
        // a window narrower than both: the tail keeps its minimum, the head scrolls
        assert_eq!(clamp_tail(300.0, 400.0, 220.0, 500.0, 8.0), 220.0);
    }

    #[test]
    fn clamping_builds_no_debt() {
        // the stored value is clamped for this frame only: a wider region gets it back
        let stored = 500.0;
        assert_eq!(clamp_head(stored, 100.0, 200.0, 600.0, 8.0), 392.0);
        assert_eq!(clamp_head(stored, 100.0, 200.0, 1000.0, 8.0), 500.0);
    }

    #[test]
    fn dragging_by_hand_follows_the_pointer() {
        assert_eq!(clamp_at(280.0, 20.0, 1.0, 220.0, 480.0), 300.0);
        assert_eq!(clamp_at(280.0, 20.0, -1.0, 220.0, 480.0), 260.0);
        assert_eq!(clamp_at(280.0, 500.0, 1.0, 220.0, 480.0), 480.0);
        // past a limit and back, the value follows at once
        assert_eq!(clamp_at(280.0, -10.0, 1.0, 220.0, 480.0), 270.0);
        assert_eq!(clamp_at(280.0, 0.0, 1.0, 300.0, 100.0), 300.0);
    }
}
