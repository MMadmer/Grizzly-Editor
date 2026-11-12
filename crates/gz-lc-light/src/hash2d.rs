//! `hash2D`: a grid over a box of texture space; a cell lists the triangles whose bounds touch
//! it, in the order they were added. Built in two passes into flat arrays, so one grid costs
//! two allocations however many cells it has.

use crate::math::{Box2, i_ceil, i_floor};

/// `hash2D<T, s_X, s_Y>` holding item indices.
#[derive(Clone, Debug)]
pub struct Hash2D {
    sx: usize,
    sy: usize,
    bounds: Box2,
    size: glam::Vec2,
    /// Where each cell's items start in `items`; one more entry than cells.
    starts: Vec<u32>,
    items: Vec<u32>,
}

impl Hash2D {
    /// `initialize` and `add` for every box: `boxes[i]` is item `i`.
    #[must_use]
    pub fn build(sx: usize, sy: usize, bounds: Box2, boxes: &[Box2]) -> Self {
        let mut h = Self {
            sx,
            sy,
            bounds,
            size: glam::Vec2::new(bounds.max.x - bounds.min.x, bounds.max.y - bounds.min.y),
            starts: Vec::new(),
            items: Vec::new(),
        };
        let cells = sx * sy;
        let mut counts = vec![0u32; cells + 1];
        for b in boxes {
            let (x1, x2, y1, y2) = h.span(b);
            for y in y1..=y2 {
                for x in x1..=x2 {
                    if let Some(c) = counts.get_mut(y * sx + x + 1) {
                        *c += 1;
                    }
                }
            }
        }
        for i in 1..counts.len() {
            let prev = counts.get(i - 1).copied().unwrap_or(0);
            if let Some(c) = counts.get_mut(i) {
                *c += prev;
            }
        }
        let total = counts.last().copied().unwrap_or(0) as usize;
        let mut fill = counts.clone();
        let mut items = vec![0u32; total];
        for (i, b) in boxes.iter().enumerate() {
            let (x1, x2, y1, y2) = h.span(b);
            for y in y1..=y2 {
                for x in x1..=x2 {
                    if let Some(at) = fill.get_mut(y * sx + x)
                        && let Some(slot) = items.get_mut(*at as usize)
                    {
                        *slot = i as u32;
                        *at += 1;
                    }
                }
            }
        }
        h.starts = counts;
        h.items = items;
        h
    }

    fn cell_x(&self, x: f32, ceil: bool) -> usize {
        let t = self.sx as f32 * (x - self.bounds.min.x) / self.size.x;
        let i = if ceil { i_ceil(t) } else { i_floor(t) };
        i.clamp(0, self.sx as i32 - 1) as usize
    }

    fn cell_y(&self, y: f32, ceil: bool) -> usize {
        let t = self.sy as f32 * (y - self.bounds.min.y) / self.size.y;
        let i = if ceil { i_ceil(t) } else { i_floor(t) };
        i.clamp(0, self.sy as i32 - 1) as usize
    }

    fn span(&self, b: &Box2) -> (usize, usize, usize, usize) {
        (
            self.cell_x(b.min.x, false),
            self.cell_x(b.max.x, true),
            self.cell_y(b.min.y, false),
            self.cell_y(b.max.y, true),
        )
    }

    /// `query`: the items of the cell a point falls into.
    #[must_use]
    pub fn query(&self, x: f32, y: f32) -> &[u32] {
        let c = self.cell_y(y, false) * self.sx + self.cell_x(x, false);
        let (Some(&a), Some(&b)) = (self.starts.get(c), self.starts.get(c + 1)) else { return &[] };
        self.items.get(a as usize..b as usize).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec2;

    #[test]
    fn a_cell_lists_what_touches_it_in_order() {
        let b = |a: (f32, f32), c: (f32, f32)| Box2 { min: Vec2::new(a.0, a.1), max: Vec2::new(c.0, c.1) };
        let all = b((0.0, 0.0), (1.0, 1.0));
        let h = Hash2D::build(
            4,
            4,
            all,
            &[b((0.0, 0.0), (1.0, 1.0)), b((0.0, 0.0), (0.2, 0.2)), b((0.6, 0.6), (1.0, 1.0))],
        );
        assert_eq!(h.query(0.1, 0.1), &[0, 1]);
        assert_eq!(h.query(0.9, 0.9), &[0, 2]);
        // the ceiling of a box's far side reaches one cell further
        assert_eq!(h.query(0.3, 0.3), &[0, 1]);
        assert_eq!(h.query(0.6, 0.1), &[0]);
    }
}
