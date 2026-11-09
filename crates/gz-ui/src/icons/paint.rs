//! Drawing a built glyph: a scale, an offset and - at 20 px and below - a snap to the pixel grid.
//!
//! egui strokes a path with mitred joins and butt caps, feathered by its anti-aliasing. That is
//! the look small (1-2 px strokes, snapped to pixel centres or edges so they stay crisp); above
//! 2 px of an opaque stroke the line is cut at its corners and every corner and end gets a disc,
//! so joins and caps are round and an acute corner never spikes.

use egui::epaint::{Mesh, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Stroke, pos2};

use super::geometry::{CLOSED, CONVEX, DASHED, FILLED, GRID, Glyph, PF_FREE, PF_JOIN, PrimKind, QUIET};

/// At this size in px and below, points land on pixels.
pub(super) const SNAP_UP_TO: f32 = 20.0;
const STROKE: f32 = 1.5;
/// A secondary stroke's alpha, relative to the colour.
const QUIET_ALPHA: f32 = 0.55;
/// px of stroke above which joins and caps are drawn round.
const ROUND_ABOVE: f32 = 2.0;

/// Where a glyph lands, in pixels.
#[derive(Clone, Copy, Debug)]
pub(super) struct View {
    /// The top-left corner in px.
    pub o: Pos2,
    /// px per grid unit.
    pub u: f32,
    pub snap: bool,
    /// An odd stroke is crisp on a pixel's centre, an even one on its edge.
    pub odd: bool,
}

impl View {
    /// The view of a glyph `side` px wide at `o`, and its stroke width in px.
    #[must_use]
    pub fn new(o: Pos2, side: f32) -> (Self, f32) {
        let u = side / GRID;
        let snap = side <= SNAP_UP_TO;
        let mut t = STROKE * u;
        // small: a whole number of pixels, rounded a little down so 16 px keeps a 1 px line
        if snap {
            t = (t + 0.4).floor();
        }
        let t = t.max(1.0);
        (Self { o, u, snap, odd: snap && (t as i32) & 1 == 1 }, t)
    }

    /// A grid point in px; snapped unless it is a point of a curve.
    #[must_use]
    pub fn at(&self, p: Pos2, free: bool) -> Pos2 {
        let s = pos2(self.o.x + p.x * self.u, self.o.y + p.y * self.u);
        if !self.snap || free { s } else { pos2(snap(s.x, self.odd), snap(s.y, self.odd)) }
    }
}

/// A coordinate in px snapped for a crisp stroke: a pixel's centre for an odd width, its edge
/// for an even one.
#[must_use]
pub(super) fn snap(v: f32, odd: bool) -> f32 {
    if odd { v.floor() + 0.5 } else { (v + 0.5).floor() }
}

/// Snapping moves a corner by up to half a pixel, and the first points of a curve that starts
/// there can then lie behind it: the line doubles back over less than a pixel, and a mitre turns
/// that into a spike. Such a point goes - the one on the shorter leg, so a snapped corner outlives
/// the curve points around it; an open path keeps its ends. Points are in px.
pub(super) fn drop_backtracks(s: &mut Vec<Pos2>, closed: bool) {
    fn doubles_back(a: Pos2, b: Pos2, c: Pos2) -> (bool, f32, f32) {
        let (ux, uy, vx, vy) = (b.x - a.x, b.y - a.y, c.x - b.x, c.y - b.y);
        let ab = ux * ux + uy * uy;
        let bc = vx * vx + vy * vy;
        (bc < 1e-6 || (ux * vx + uy * vy < 0.0 && (ab < 1.0 || bc < 1.0)), ab, bc)
    }
    let n = s.len();
    let mut w = 0usize;
    for i in 0..n {
        let Some(p) = s.get(i).copied() else { break };
        if let Some(slot) = s.get_mut(w) {
            *slot = p;
        }
        w += 1;
        while w >= 3 {
            let (Some(a), Some(b), Some(c)) = (s.get(w - 3).copied(), s.get(w - 2).copied(), s.get(w - 1).copied()) else { break };
            let (back, ab, bc) = doubles_back(a, b, c);
            if !back {
                break;
            }
            if bc > ab || (!closed && i + 1 == n) {
                if let Some(slot) = s.get_mut(w - 2) {
                    *slot = c;
                }
            }
            w -= 1;
        }
    }
    s.truncate(w);
    if closed {
        while s.len() >= 4 {
            let k = s.len();
            let (Some(a), Some(b), Some(c)) = (s.get(k - 2).copied(), s.get(k - 1).copied(), s.first().copied()) else { break };
            if !doubles_back(a, b, c).0 {
                break;
            }
            s.pop();
        }
        if s.len() >= 4
            && let (Some(a), Some(b), Some(c)) = (s.last().copied(), s.first().copied(), s.get(1).copied())
            && doubles_back(a, b, c).0
        {
            s.remove(0);
        }
    }
}

/// Dashes measured ALONG the stroke, each one polyline: an arc arrives as many short pieces, and
/// "every other piece" would be noise that gets finer with size. The pattern is stretched to a
/// whole number of dashes: a loop closes on a gap, an open path starts and ends on a dash.
/// Points, width and unit are in points.
fn stroke_dashed(painter: &Painter, pts: &[Pos2], stroke: Stroke, unit: f32, looped: bool) {
    let length: f32 = pts.windows(2).map(|w| if let [a, b] = w { a.distance(*b) } else { 0.0 }).sum();
    let (base_dash, base_gap) = (2.4 * unit, 1.35 * unit);
    let base_period = base_dash + base_gap;
    if length < base_gap {
        return;
    }
    let count = ((if looped { length } else { length + base_gap }) / base_period).round().max(if looped { 3.0 } else { 2.0 });
    let fit = length / if looped { count * base_period } else { count * base_period - base_gap };
    let (dash, period) = (base_dash * fit, base_period * fit);
    let mut phase = 0.0f32;
    let mut run: Vec<Pos2> = Vec::new();
    for w in pts.windows(2) {
        let [a, b] = w else { continue };
        let len = a.distance(*b);
        if len < 1e-4 {
            continue;
        }
        let mut done = 0.0f32;
        while done < len {
            let on = phase < dash;
            let take = ((if on { dash } else { period }) - phase).min(len - done);
            if on {
                let (u0, u1) = (done / len, (done + take) / len);
                if run.is_empty() {
                    run.push(a.lerp(*b, u0));
                }
                run.push(a.lerp(*b, u1));
            }
            done += take;
            phase += take;
            if !run.is_empty() && phase >= dash - 1e-4 {
                painter.add(Shape::line(std::mem::take(&mut run), stroke));
            }
            if phase >= period - 1e-4 {
                phase = 0.0;
            }
        }
    }
    if run.len() >= 2 {
        painter.add(Shape::line(run, stroke));
    }
}

/// Round joins and caps: the line is cut at its corners, the pieces are butted and every corner
/// and end gets a disc. Only for an opaque colour - a translucent disc would show on the line.
fn stroke_round(painter: &Painter, s: &[Pos2], fl: &[u8], closed: bool, stroke: Stroke) {
    let n = s.len();
    let join = |i: usize| fl.get(i).is_some_and(|f| f & PF_JOIN != 0);
    let start = if closed {
        match (0..n).find(|&i| join(i)) {
            Some(i) => i,
            None => {
                painter.add(Shape::closed_line(s.to_vec(), stroke));
                return;
            }
        }
    } else {
        0
    };
    let segments = if closed { n } else { n.saturating_sub(1) };
    let mut run: Vec<Pos2> = Vec::with_capacity(n + 1);
    if let Some(p) = s.get(start) {
        run.push(*p);
    }
    for k in 1..=segments {
        let i = (start + k) % n;
        let Some(p) = s.get(i).copied() else { continue };
        run.push(p);
        if join(i) || k == segments {
            painter.add(Shape::line(std::mem::replace(&mut run, Vec::with_capacity(8)), stroke));
            run.push(p);
        }
    }
    for (i, p) in s.iter().enumerate() {
        if join(i) {
            painter.add(Shape::circle_filled(*p, stroke.width * 0.5, stroke.color));
        }
    }
}

/// A rounded rectangle as a closed path through snapped corners, `r` in points.
fn rounded_rect(a: Pos2, b: Pos2, r: f32) -> Vec<Pos2> {
    let r = r.min((b.x - a.x).abs() * 0.5).min((b.y - a.y).abs() * 0.5).max(0.0);
    if r < 0.25 {
        return vec![a, pos2(b.x, a.y), b, pos2(a.x, b.y)];
    }
    let steps = (r.ceil() as usize).clamp(2, 8);
    let mut out = Vec::with_capacity((steps + 1) * 4);
    let corners = [(pos2(a.x + r, a.y + r), 180.0f32), (pos2(b.x - r, a.y + r), 270.0), (pos2(b.x - r, b.y - r), 0.0), (pos2(a.x + r, b.y - r), 90.0)];
    for (c, a0) in corners {
        for k in 0..=steps {
            let ang = (a0 + 90.0 * k as f32 / steps as f32).to_radians();
            out.push(pos2(c.x + r * ang.cos(), c.y + r * ang.sin()));
        }
    }
    out
}

/// Draws a glyph `side` points wide with its top-left corner at `min` (points).
pub(super) fn draw_glyph(painter: &Painter, g: &Glyph, min: Pos2, side: f32, color: Color32) {
    let ppp = painter.pixels_per_point();
    let side_px = side * ppp;
    if side_px < 2.0 || color.a() == 0 {
        return;
    }
    let (v, t) = View::new(pos2(min.x * ppp, min.y * ppp), side_px);
    let to_pt = |p: Pos2| pos2(p.x / ppp, p.y / ppp);
    let quiet = color.gamma_multiply(QUIET_ALPHA);
    let round = t > ROUND_ABOVE && color.a() == 255;
    let width = t / ppp;

    for p in &g.prims {
        let q = p.has(QUIET);
        let c = if q { quiet } else { color };
        let stroke = Stroke::new(width, c);
        match p.kind {
            PrimKind::Stroke => {
                let (src, fl) = g.points(p);
                let mut s: Vec<Pos2> = src.iter().zip(fl).map(|(pt, f)| v.at(*pt, f & PF_FREE != 0)).collect();
                let closed = p.has(CLOSED);
                let dashed = p.has(DASHED);
                // a snapped glyph is at most 20 px, where no stroke is drawn round: the flags need not
                // follow the points that go. A dashed loop already ends on its first point.
                if v.snap {
                    drop_backtracks(&mut s, closed && !dashed);
                }
                if s.len() < 2 {
                    continue;
                }
                for pt in &mut s {
                    *pt = to_pt(*pt);
                }
                if dashed {
                    stroke_dashed(painter, &s, stroke, v.u / ppp, closed);
                } else if round && !q && !v.snap {
                    stroke_round(painter, &s, fl, closed, stroke);
                } else if closed {
                    painter.add(Shape::closed_line(s, PathStroke::from(stroke)));
                } else {
                    painter.add(Shape::line(s, PathStroke::from(stroke)));
                }
            }
            PrimKind::Rect => {
                let a = to_pt(v.at(p.a, false));
                let b = to_pt(v.at(p.b, false));
                let path = rounded_rect(a, b, p.r * v.u / ppp);
                if p.has(FILLED) {
                    painter.add(Shape::convex_polygon(path, c, Stroke::NONE));
                } else {
                    painter.add(Shape::closed_line(path, stroke));
                }
            }
            PrimKind::Circle => {
                let centre = to_pt(v.at(p.a, false));
                let r = p.r * v.u / ppp;
                if p.has(FILLED) {
                    painter.add(Shape::circle_filled(centre, r, c));
                } else {
                    painter.add(Shape::circle_stroke(centre, r, stroke));
                }
            }
            PrimKind::Fill => {
                let (src, _) = g.points(p);
                if p.has(CONVEX) {
                    let pts: Vec<Pos2> = src.iter().map(|pt| to_pt(v.at(*pt, false))).collect();
                    painter.add(Shape::convex_polygon(pts, c, Stroke::NONE));
                } else {
                    // egui fills only convex paths: a concave fill goes as the triangles built for it;
                    // its outline stroke (every concave fill has one) gives it its edge
                    let mut mesh = Mesh::default();
                    for pt in src {
                        mesh.colored_vertex(to_pt(v.at(*pt, false)), c);
                    }
                    for tri in g.triangles(p).chunks_exact(3) {
                        if let [a, b, cc] = tri {
                            mesh.add_triangle(u32::from(*a), u32::from(*b), u32::from(*cc));
                        }
                    }
                    painter.add(Shape::mesh(mesh));
                }
            }
        }
    }
}

/// The rectangle of a glyph drawn into `rect`: its largest centred square.
#[must_use]
pub(super) fn square_in(rect: Rect) -> Rect {
    let side = rect.width().min(rect.height());
    Rect::from_center_size(rect.center(), egui::vec2(side, side))
}

#[cfg(test)]
mod tests {
    use super::{View, drop_backtracks, snap};
    use egui::pos2;

    #[test]
    fn odd_strokes_snap_to_centres_even_to_edges() {
        assert_eq!(snap(3.2, true), 3.5);
        assert_eq!(snap(3.9, true), 3.5);
        assert_eq!(snap(3.2, false), 3.0);
        assert_eq!(snap(3.6, false), 4.0);
    }

    #[test]
    fn a_16_px_glyph_keeps_a_1_px_line() {
        let (v, t) = View::new(pos2(0.0, 0.0), 16.0);
        assert!(v.snap);
        assert_eq!(t, 1.0);
        assert!(v.odd);
        let (v, t) = View::new(pos2(0.0, 0.0), 20.0);
        assert!(v.snap);
        assert_eq!(t, 2.0);
        assert!(!v.odd);
        let (v, t) = View::new(pos2(0.0, 0.0), 48.0);
        assert!(!v.snap);
        assert_eq!(t, 4.5);
    }

    #[test]
    fn curve_points_are_never_snapped() {
        let (v, _) = View::new(pos2(10.0, 10.0), 16.0);
        assert_eq!(v.at(pos2(3.3, 3.3), false), pos2(13.5, 13.5));
        assert_eq!(v.at(pos2(3.3, 3.3), true), pos2(13.3, 13.3));
    }

    #[test]
    fn a_point_that_doubles_back_goes() {
        // a corner at 5,5, then a curve point snapped behind it, then onwards
        let mut s = vec![pos2(0.0, 5.0), pos2(5.0, 5.0), pos2(4.6, 5.0), pos2(4.6, 9.0)];
        drop_backtracks(&mut s, false);
        assert_eq!(s.len(), 3);
        assert_eq!(s.first(), Some(&pos2(0.0, 5.0)));
        assert_eq!(s.last(), Some(&pos2(4.6, 9.0)));
        // nothing to drop on a clean path
        let mut clean = vec![pos2(0.0, 0.0), pos2(4.0, 0.0), pos2(4.0, 4.0)];
        drop_backtracks(&mut clean, false);
        assert_eq!(clean.len(), 3);
        // a repeated point goes too
        let mut twice = vec![pos2(0.0, 0.0), pos2(4.0, 0.0), pos2(4.0, 0.0), pos2(4.0, 4.0)];
        drop_backtracks(&mut twice, false);
        assert_eq!(twice.len(), 3);
    }
}
