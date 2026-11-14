//! What stands behind a picture.

use egui::{CornerRadius, Painter, Pos2, Rect, Vec2, pos2};

use crate::theme::palette;

/// The checkerboard a picture with transparency is judged against: the cells of `cell` points
/// that lie inside `clip`, lined up with `origin` so they move with the picture. Only what is in
/// view is painted - a zoomed-in texture is millions of cells, its canvas a few hundred.
pub fn checker(painter: &Painter, origin: Pos2, clip: Rect, cell: f32) {
    let cell = cell.max(2.0);
    if clip.width() <= 0.0 || clip.height() <= 0.0 {
        return;
    }
    let p = palette(painter.ctx());
    painter.rect_filled(clip, CornerRadius::ZERO, p.input);
    let col0 = ((clip.min.x - origin.x) / cell).floor() as i64;
    let row0 = ((clip.min.y - origin.y) / cell).floor() as i64;
    let mut row = row0;
    while origin.y + row as f32 * cell < clip.max.y {
        let mut col = col0 + ((col0 + row) & 1);
        while origin.x + col as f32 * cell < clip.max.x {
            let a = pos2(origin.x + col as f32 * cell, origin.y + row as f32 * cell);
            let r = Rect::from_min_max(a.max(clip.min), (a + Vec2::splat(cell)).min(clip.max));
            if r.width() > 0.0 && r.height() > 0.0 {
                painter.rect_filled(r, CornerRadius::ZERO, p.hover);
            }
            col += 2;
        }
        row += 1;
    }
}
