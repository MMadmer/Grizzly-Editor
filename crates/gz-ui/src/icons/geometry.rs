//! The geometry of every glyph.
//!
//! Every glyph lives on a 16-unit grid with a 1-unit margin and is stroked 1.5 units wide. The
//! whole set is built once, on the first draw: arcs are tessellated then, fills get their winding,
//! convexity and (when concave) their triangles then, corners that want a round join are found
//! then. A draw is a scale, an offset and - at 20 px and below - a snap to the pixel grid.

use std::sync::OnceLock;

use egui::{Pos2, pos2};

use super::Icon;

pub(super) const GRID: f32 = 16.0;
const DEG: f64 = std::f64::consts::PI / 180.0;
/// A turn sharper than ~20 degrees is a corner.
const JOIN_COS: f32 = 0.94;

/// A point of a curve: never snapped, or the curve wobbles.
pub(super) const PF_FREE: u8 = 1;
/// A corner or an end: covered by a disc when joins are drawn round.
pub(super) const PF_JOIN: u8 = 2;

/// A dashed path is closed by repeating its first point, and keeps the flag.
pub(super) const CLOSED: u8 = 1;
/// A secondary stroke, drawn at a lower alpha.
pub(super) const QUIET: u8 = 2;
pub(super) const DASHED: u8 = 4;
pub(super) const FILLED: u8 = 8;
pub(super) const CONVEX: u8 = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PrimKind {
    Stroke,
    Rect,
    Circle,
    Fill,
}

/// One primitive of a glyph.
#[derive(Clone, Copy, Debug)]
pub(super) struct Prim {
    pub kind: PrimKind,
    pub flags: u8,
    /// A rect's rounding, a circle's radius.
    pub r: f32,
    /// A rect's corners; a circle's centre is `a`.
    pub a: Pos2,
    pub b: Pos2,
    /// A stroke's or a fill's points in `Glyph::pts`.
    pub first: u32,
    pub count: u32,
    /// A concave fill's triangles in `Glyph::tris` (indices relative to `first`).
    pub tri_first: u32,
    pub tri_count: u32,
}

impl Prim {
    fn new(kind: PrimKind, flags: u8) -> Self {
        Self {
            kind,
            flags,
            r: 0.0,
            a: Pos2::ZERO,
            b: Pos2::ZERO,
            first: 0,
            count: 0,
            tri_first: 0,
            tri_count: 0,
        }
    }

    #[must_use]
    pub fn has(&self, flag: u8) -> bool {
        self.flags & flag != 0
    }
}

/// A glyph: its primitives and the points they share.
#[derive(Clone, Debug, Default)]
pub(super) struct Glyph {
    pub prims: Vec<Prim>,
    pub pts: Vec<Pos2>,
    pub pflags: Vec<u8>,
    pub tris: Vec<u16>,
}

impl Glyph {
    /// The points of a stroke or a fill.
    #[must_use]
    pub fn points(&self, p: &Prim) -> (&[Pos2], &[u8]) {
        let a = p.first as usize;
        let b = a + p.count as usize;
        (self.pts.get(a..b).unwrap_or(&[]), self.pflags.get(a..b).unwrap_or(&[]))
    }

    #[must_use]
    pub fn triangles(&self, p: &Prim) -> &[u16] {
        let a = p.tri_first as usize;
        self.tris.get(a..a + p.tri_count as usize).unwrap_or(&[])
    }
}

/// Every glyph, by `Icon as usize`. An immutable cache computed once from the constants below -
/// the one static of the kit, and it holds no state.
static GLYPHS: OnceLock<Vec<Glyph>> = OnceLock::new();

/// The built glyphs.
pub(super) fn glyphs() -> &'static [Glyph] {
    GLYPHS.get_or_init(|| {
        let mut all = vec![Glyph::default(); Icon::ALL.len() + 1];
        for &icon in Icon::ALL {
            if let Some(g) = all.get_mut(icon as usize) {
                let mut b = B::new(g);
                build(icon, &mut b);
            }
        }
        all
    })
}

/// Coordinates are written to two decimals, so an arc that should start where a line ends can
/// miss it by a hundredth: that is one point, or the path steps back over a sliver and a mitre
/// spikes.
fn near(a: Pos2, b: Pos2) -> bool {
    (a.x - b.x).abs() < 0.02 && (a.y - b.y).abs() < 0.02
}

/// The builder the glyphs are written in. Coordinates are grid units, angles degrees clockwise
/// from +x (y grows downward, so 270 is the top).
pub(super) struct B<'g> {
    g: &'g mut Glyph,
    quiet: bool,
    dashed: bool,
    path: Vec<Pos2>,
    flags: Vec<u8>,
}

impl<'g> B<'g> {
    fn new(g: &'g mut Glyph) -> Self {
        Self { g, quiet: false, dashed: false, path: Vec::new(), flags: Vec::new() }
    }

    fn style(&self) -> u8 {
        (if self.quiet { QUIET } else { 0 }) | (if self.dashed { DASHED } else { 0 })
    }

    /// What is added after it is a secondary stroke.
    pub fn quiet(&mut self, on: bool) -> &mut Self {
        self.quiet = on;
        self
    }

    /// What is added after it is dashed.
    pub fn dashed(&mut self, on: bool) -> &mut Self {
        self.dashed = on;
        self
    }

    /// A polyline through x,y pairs.
    pub fn l(&mut self, xy: &[f64]) {
        self.poly(xy, false);
    }

    /// A closed polyline.
    pub fn z(&mut self, xy: &[f64]) {
        self.poly(xy, true);
    }

    /// A filled polygon.
    pub fn f(&mut self, xy: &[f64]) {
        self.fill(xy);
    }

    /// A filled polygon with its outline stroked too, so its corners round like every other.
    pub fn fz(&mut self, xy: &[f64]) {
        self.fill(xy);
        self.poly(xy, true);
    }

    pub fn r(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) {
        self.rect(x0, y0, x1, y1, r, false);
    }

    pub fn rf(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64) {
        self.rect(x0, y0, x1, y1, r, true);
    }

    /// A circle.
    pub fn c(&mut self, cx: f64, cy: f64, r: f64) {
        self.circle(cx, cy, r, false);
    }

    /// A disc.
    pub fn d(&mut self, cx: f64, cy: f64, r: f64) {
        self.circle(cx, cy, r, true);
    }

    /// An arc; `a1` below `a0` runs counter-clockwise.
    pub fn a(&mut self, cx: f64, cy: f64, r: f64, a0: f64, a1: f64) {
        self.m().arc(cx, cy, r, a0, a1).end(false);
    }

    /// An ellipse.
    pub fn e(&mut self, cx: f64, cy: f64, rx: f64, ry: f64) {
        self.m().ellipse(cx, cy, rx, ry, 0.0, 360.0).end(true);
    }

    /// An arrow head: a chevron with its tip at x,y, pointing along `deg`.
    pub fn head(&mut self, x: f64, y: f64, deg: f64, len: f64) {
        let back0 = (deg + 180.0 - 42.0) * DEG;
        let back1 = (deg + 180.0 + 42.0) * DEG;
        self.m_at(x + len * back0.cos(), y + len * back0.sin())
            .to(x, y)
            .to(x + len * back1.cos(), y + len * back1.sin())
            .end(false);
    }

    pub fn arrow(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, head: f64) {
        self.l(&[x0, y0, x1, y1]);
        self.head(x1, y1, (y1 - y0).atan2(x1 - x0) / DEG, head);
    }

    /// An elliptic arc with an arrow head at its `a1` end.
    #[allow(clippy::too_many_arguments)]
    pub fn arc_arrow(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64, head: f64) {
        self.m().ellipse(cx, cy, rx, ry, a0, a1).end(false);
        // the tangent at the end, in the direction of travel
        let a = a1 * DEG;
        let s = if a1 >= a0 { 1.0 } else { -1.0 };
        self.head(
            cx + rx * a.cos(),
            cy + ry * a.sin(),
            (s * ry * a.cos()).atan2(-s * rx * a.sin()) / DEG,
            head,
        );
    }

    /// Starts a path.
    pub fn m(&mut self) -> &mut Self {
        self.path.clear();
        self.flags.clear();
        self
    }

    /// Starts a path at a point.
    pub fn m_at(&mut self, x: f64, y: f64) -> &mut Self {
        self.m().to(x, y)
    }

    pub fn to(&mut self, x: f64, y: f64) -> &mut Self {
        self.push(x, y, 0);
        self
    }

    pub fn arc(&mut self, cx: f64, cy: f64, r: f64, a0: f64, a1: f64) -> &mut Self {
        self.ellipse(cx, cy, r, r, a0, a1)
    }

    pub fn ellipse(&mut self, cx: f64, cy: f64, rx: f64, ry: f64, a0: f64, a1: f64) -> &mut Self {
        let steps = (((a1 - a0).abs() / 10.0).ceil() as i32).max(2);
        for i in 0..=steps {
            let a = (a0 + (a1 - a0) * f64::from(i) / f64::from(steps)) * DEG;
            self.push(cx + rx * a.cos(), cy + ry * a.sin(), PF_FREE);
        }
        self
    }

    fn push(&mut self, x: f64, y: f64, flags: u8) {
        let p = pos2(x as f32, y as f32);
        // a path that meets itself (a line into an arc) keeps one point, snapped unless both are curve
        if let (Some(last), Some(f)) = (self.path.last(), self.flags.last_mut())
            && near(*last, p)
        {
            *f &= flags;
            return;
        }
        self.path.push(p);
        self.flags.push(flags);
    }

    /// Finishes the path.
    pub fn end(&mut self, closed: bool) {
        let mut closed = closed;
        if closed
            && self.path.len() > 2
            && self.path.first().zip(self.path.last()).is_some_and(|(a, b)| near(*a, *b))
        {
            let last = self.flags.pop().unwrap_or(0);
            self.path.pop();
            if let Some(f) = self.flags.first_mut() {
                *f &= last;
            }
        }
        if self.path.len() < 2 {
            self.m();
            return;
        }
        // a dash walker needs a start and an end: a closed dashed path runs back to its start
        let looped = self.dashed && closed;
        if looped {
            if let (Some(p), Some(f)) = (self.path.first().copied(), self.flags.first().copied()) {
                self.path.push(p);
                self.flags.push(f);
            }
            closed = false;
        }

        let n = self.path.len();
        for i in 0..n {
            if closed || (i > 0 && i + 1 < n) {
                let (Some(a), Some(p), Some(c)) =
                    (self.path.get((i + n - 1) % n), self.path.get(i), self.path.get((i + 1) % n))
                else {
                    continue;
                };
                let (ux, uy, vx, vy) = (p.x - a.x, p.y - a.y, c.x - p.x, c.y - p.y);
                let lu = (ux * ux + uy * uy).sqrt();
                let lv = (vx * vx + vy * vy).sqrt();
                if lu < 1e-4 || lv < 1e-4 || (ux * vx + uy * vy) / (lu * lv) > JOIN_COS {
                    continue;
                }
            }
            if let Some(f) = self.flags.get_mut(i) {
                *f |= PF_JOIN;
            }
        }

        let mut prim = Prim::new(PrimKind::Stroke, self.style() | if closed || looped { CLOSED } else { 0 });
        prim.first = self.g.pts.len() as u32;
        prim.count = n as u32;
        self.g.pts.extend_from_slice(&self.path);
        self.g.pflags.extend_from_slice(&self.flags);
        self.g.prims.push(prim);
        self.m();
    }

    pub fn poly(&mut self, xy: &[f64], closed: bool) {
        self.m();
        for [x, y] in xy.as_chunks::<2>().0 {
            self.to(*x, *y);
        }
        self.end(closed);
    }

    pub fn fill(&mut self, xy: &[f64]) {
        let mut pts: Vec<Pos2> =
            xy.as_chunks::<2>().0.iter().map(|[x, y]| pos2(*x as f32, *y as f32)).collect();
        if pts.len() < 3 {
            return;
        }
        // egui fills a convex polygon with anti-aliased edges when it runs clockwise on screen
        if signed_area(&pts) < 0.0 {
            pts.reverse();
        }
        let convex = is_convex(&pts);
        let mut prim = Prim::new(
            PrimKind::Fill,
            (if self.quiet { QUIET } else { 0 }) | FILLED | if convex { CONVEX } else { 0 },
        );
        prim.first = self.g.pts.len() as u32;
        prim.count = pts.len() as u32;
        if !convex {
            let tris = ear_clip(&pts);
            prim.tri_first = self.g.tris.len() as u32;
            prim.tri_count = tris.len() as u32;
            self.g.tris.extend_from_slice(&tris);
        }
        self.g.pflags.extend(std::iter::repeat_n(0u8, pts.len()));
        self.g.pts.extend_from_slice(&pts);
        self.g.prims.push(prim);
    }

    fn rect(&mut self, x0: f64, y0: f64, x1: f64, y1: f64, r: f64, filled: bool) {
        // a dashed one becomes a path
        if self.dashed && !filled {
            self.m()
                .arc(x0 + r, y0 + r, r, 180.0, 270.0)
                .arc(x1 - r, y0 + r, r, 270.0, 360.0)
                .arc(x1 - r, y1 - r, r, 0.0, 90.0)
                .arc(x0 + r, y1 - r, r, 90.0, 180.0)
                .end(true);
            return;
        }
        let mut prim = Prim::new(PrimKind::Rect, self.style() | if filled { FILLED } else { 0 });
        prim.r = r as f32;
        prim.a = pos2(x0 as f32, y0 as f32);
        prim.b = pos2(x1 as f32, y1 as f32);
        self.g.prims.push(prim);
    }

    fn circle(&mut self, cx: f64, cy: f64, r: f64, filled: bool) {
        if self.dashed && !filled {
            self.m().arc(cx, cy, r, 0.0, 360.0).end(true);
            return;
        }
        let mut prim = Prim::new(PrimKind::Circle, self.style() | if filled { FILLED } else { 0 });
        prim.r = r as f32;
        prim.a = pos2(cx as f32, cy as f32);
        self.g.prims.push(prim);
    }
}

/// Twice the signed area: positive when the polygon runs clockwise on screen (y down).
pub(super) fn signed_area(p: &[Pos2]) -> f32 {
    let n = p.len();
    (0..n).filter_map(|i| Some((p.get(i)?, p.get((i + 1) % n)?))).map(|(a, b)| a.x * b.y - b.x * a.y).sum()
}

fn is_convex(p: &[Pos2]) -> bool {
    let n = p.len();
    (0..n).all(|i| match (p.get(i), p.get((i + 1) % n), p.get((i + 2) % n)) {
        (Some(a), Some(b), Some(c)) => (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) >= -1e-4,
        _ => true,
    })
}

/// Triangles of a simple clockwise polygon, by ear clipping: indices into `p`.
pub(super) fn ear_clip(p: &[Pos2]) -> Vec<u16> {
    let mut idx: Vec<u16> = (0..p.len() as u16).collect();
    let mut out = Vec::with_capacity(p.len().saturating_sub(2) * 3);
    let at = |i: u16| p.get(i as usize).copied().unwrap_or(Pos2::ZERO);
    let cross = |a: Pos2, b: Pos2, c: Pos2| (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x);
    let inside = |q: Pos2, a: Pos2, b: Pos2, c: Pos2| {
        cross(a, b, q) >= 0.0 && cross(b, c, q) >= 0.0 && cross(c, a, q) >= 0.0
    };
    let mut guard = p.len() * p.len();
    while idx.len() > 3 && guard > 0 {
        guard -= 1;
        let n = idx.len();
        let mut clipped = false;
        for k in 0..n {
            let (Some(&ia), Some(&ib), Some(&ic)) =
                (idx.get((k + n - 1) % n), idx.get(k), idx.get((k + 1) % n))
            else {
                continue;
            };
            let (a, b, c) = (at(ia), at(ib), at(ic));
            if cross(a, b, c) <= 1e-6 {
                continue;
            }
            let blocked = idx.iter().any(|&j| j != ia && j != ib && j != ic && inside(at(j), a, b, c));
            if blocked {
                continue;
            }
            out.extend_from_slice(&[ia, ib, ic]);
            idx.remove(k);
            clipped = true;
            break;
        }
        if !clipped {
            break;
        }
    }
    if let [a, b, c] = idx[..] {
        out.extend_from_slice(&[a, b, c]);
    }
    out
}

// ------------------------------------------------------------------------------------------------
// shapes more than one glyph is made of
// ------------------------------------------------------------------------------------------------

fn page(b: &mut B<'_>) {
    b.z(&[3.5, 1.5, 9.5, 1.5, 12.5, 4.5, 12.5, 14.5, 3.5, 14.5]);
    b.l(&[9.5, 1.5, 9.5, 4.5, 12.5, 4.5]);
}

/// A page whose lower right corner is left to a badge.
fn page_cut(b: &mut B<'_>) {
    b.l(&[8.0, 14.5, 3.5, 14.5, 3.5, 1.5, 9.5, 1.5, 12.5, 4.5, 12.5, 7.0]);
    b.l(&[9.5, 1.5, 9.5, 4.5, 12.5, 4.5]);
}

fn folder(b: &mut B<'_>) {
    b.z(&[1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 14.5, 4.75, 14.5, 13.5, 1.5, 13.5]);
}

fn clipboard(b: &mut B<'_>) {
    b.r(5.5, 1.5, 10.5, 4.25, 0.75);
    b.m_at(10.5, 3.0)
        .to(11.5, 3.0)
        .arc(11.5, 4.0, 1.0, 270.0, 360.0)
        .to(12.5, 13.5)
        .arc(11.5, 13.5, 1.0, 0.0, 90.0)
        .to(4.5, 14.5)
        .arc(4.5, 13.5, 1.0, 90.0, 180.0)
        .to(3.5, 4.0)
        .arc(4.5, 4.0, 1.0, 180.0, 270.0)
        .to(5.5, 3.0)
        .end(false);
}

fn two_sheets(b: &mut B<'_>) {
    b.r(5.5, 5.5, 14.0, 14.0, 1.5);
    b.m_at(3.5, 10.5)
        .to(3.0, 10.5)
        .arc(3.0, 9.5, 1.0, 90.0, 180.0)
        .to(2.0, 3.0)
        .arc(3.0, 3.0, 1.0, 180.0, 270.0)
        .to(9.5, 2.0)
        .arc(9.5, 3.0, 1.0, 270.0, 360.0)
        .to(10.5, 3.5)
        .end(false);
}

fn corners(b: &mut B<'_>) {
    b.l(&[2.0, 5.5, 2.0, 2.0, 5.5, 2.0]);
    b.l(&[10.5, 2.0, 14.0, 2.0, 14.0, 5.5]);
    b.l(&[14.0, 10.5, 14.0, 14.0, 10.5, 14.0]);
    b.l(&[5.5, 14.0, 2.0, 14.0, 2.0, 10.5]);
}

fn lens(b: &mut B<'_>, cx: f64, cy: f64, r: f64, reach: f64) {
    let d = r * std::f64::consts::FRAC_1_SQRT_2;
    b.c(cx, cy, r);
    b.l(&[cx + d, cy + d, reach, reach]);
}

fn eye_shape(b: &mut B<'_>) {
    // two arcs of a circle 7.17 in radius, meeting at the corners of the eye
    b.m().arc(8.0, 11.03, 7.17, 205.0, 335.0).arc(8.0, 4.97, 7.17, 25.0, 155.0).end(true);
}

/// Head and shoulders, the shoulders a rounded box.
fn bust(b: &mut B<'_>, cx: f64, head_y: f64, head_r: f64, top: f64, half: f64, bottom: f64) {
    let r = half.min(2.5);
    b.c(cx, head_y, head_r);
    b.m_at(cx - half, bottom)
        .to(cx - half, top + r)
        .arc(cx - half + r, top + r, r, 180.0, 270.0)
        .to(cx + half - r, top)
        .arc(cx + half - r, top + r, r, 270.0, 360.0)
        .to(cx + half, bottom)
        .end(false);
}

fn regular(b: &mut B<'_>, cx: f64, cy: f64, r: f64, n: i32, deg0: f64) {
    let xy: Vec<f64> = (0..n)
        .flat_map(|i| {
            let a = (deg0 + f64::from(i) * 360.0 / f64::from(n)) * DEG;
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    b.poly(&xy, true);
}

fn star(b: &mut B<'_>, cx: f64, cy: f64, ro: f64, ri: f64, n: i32, filled: bool) {
    let xy: Vec<f64> = (0..n * 2)
        .flat_map(|i| {
            let a = (-90.0 + f64::from(i) * 180.0 / f64::from(n)) * DEG;
            let r = if i & 1 == 1 { ri } else { ro };
            [cx + r * a.cos(), cy + r * a.sin()]
        })
        .collect();
    if filled {
        b.fill(&xy);
    }
    b.poly(&xy, true);
}

fn gear(b: &mut B<'_>, cx: f64, cy: f64, ro: f64, ri: f64, n: i32) {
    let p = 180.0 / f64::from(n);
    let xy: Vec<f64> = (0..n)
        .flat_map(|i| {
            let base = -90.0 + f64::from(i) * 2.0 * p;
            // flank, the tooth's top, flank; the valley to the next tooth is a chord
            let a = [base - 0.8 * p, base - 0.42 * p, base + 0.42 * p, base + 0.8 * p];
            let r = [ri, ro, ro, ri];
            a.into_iter().zip(r).flat_map(move |(a, r)| [cx + r * (a * DEG).cos(), cy + r * (a * DEG).sin()])
        })
        .collect();
    b.poly(&xy, true);
}

/// The angle of x,y seen from cx,cy, in [0, 360).
fn angle_of(cx: f64, cy: f64, x: f64, y: f64) -> f64 {
    let a = (y - cy).atan2(x - cx) / DEG;
    if a < 0.0 { a + 360.0 } else { a }
}

/// Where two circles meet, the upper point of the two.
fn meet(x1: f64, y1: f64, r1: f64, x2: f64, y2: f64, r2: f64) -> (f64, f64) {
    let (dx, dy) = (x2 - x1, y2 - y1);
    let d = (dx * dx + dy * dy).sqrt();
    let a = (r1 * r1 - r2 * r2 + d * d) / (2.0 * d);
    let h = (r1 * r1 - a * a).max(0.0).sqrt();
    let (mx, my) = (x1 + a * dx / d, y1 + a * dy / d);
    let (px, py) = (-dy / d * h, dx / d * h);
    if my - py < my + py { (mx - px, my - py) } else { (mx + px, my + py) }
}

/// A cloud: three bumps on a flat base.
fn cloud(b: &mut B<'_>) {
    let (lx, ly, lr, tx, ty, tr, rx, ry, rr) = (5.0, 8.0, 2.5, 8.75, 6.25, 3.5, 12.0, 8.5, 2.0);
    let (ax, ay) = meet(lx, ly, lr, tx, ty, tr);
    let (bx, by) = meet(tx, ty, tr, rx, ry, rr);
    let t0 = angle_of(tx, ty, ax, ay);
    let mut t1 = angle_of(tx, ty, bx, by);
    if t1 < t0 {
        t1 += 360.0;
    }
    b.m()
        .arc(lx, ly, lr, 90.0, angle_of(lx, ly, ax, ay))
        .arc(tx, ty, tr, t0, t1)
        .arc(rx, ry, rr, angle_of(rx, ry, bx, by), 450.0)
        .end(true);
}

/// The editor's own asset kinds keep the shapes they were drawn with in a unit square.
fn u(v: f64) -> f64 {
    1.0 + 14.0 * v
}

// ------------------------------------------------------------------------------------------------
// the glyphs
// ------------------------------------------------------------------------------------------------

#[allow(clippy::cognitive_complexity)]
fn build(icon: Icon, b: &mut B<'_>) {
    match icon {
        Icon::None => {}
        // chrome ------------------------------------------------------------------
        Icon::Minimize => {
            b.l(&[3.5, 8.0, 12.5, 8.0]);
        }
        Icon::Maximize => {
            b.r(3.5, 3.5, 12.5, 12.5, 1.0);
        }
        Icon::Restore => {
            b.r(3.0, 5.5, 10.5, 13.0, 1.0);
            b.l(&[5.5, 5.5, 5.5, 3.0, 13.0, 3.0, 13.0, 10.5, 10.5, 10.5]);
        }
        Icon::Close => {
            b.l(&[4.0, 4.0, 12.0, 12.0]);
            b.l(&[12.0, 4.0, 4.0, 12.0]);
        }
        Icon::ChevronUp => {
            b.l(&[4.0, 10.0, 8.0, 6.0, 12.0, 10.0]);
        }
        Icon::ChevronDown => {
            b.l(&[4.0, 6.0, 8.0, 10.0, 12.0, 6.0]);
        }
        Icon::ChevronLeft => {
            b.l(&[10.0, 4.0, 6.0, 8.0, 10.0, 12.0]);
        }
        Icon::ChevronRight => {
            b.l(&[6.0, 4.0, 10.0, 8.0, 6.0, 12.0]);
        }
        // the one filled triangle of the chrome: that is how a menu caret reads
        Icon::Caret => {
            b.f(&[4.5, 6.25, 11.5, 6.25, 8.0, 10.25]);
        }
        Icon::More => {
            b.d(3.5, 8.0, 1.25);
            b.d(8.0, 8.0, 1.25);
            b.d(12.5, 8.0, 1.25);
        }
        Icon::Overflow => {
            b.l(&[3.5, 4.5, 7.0, 8.0, 3.5, 11.5]);
            b.l(&[8.5, 4.5, 12.0, 8.0, 8.5, 11.5]);
        }
        Icon::Menu => {
            b.l(&[2.5, 4.0, 13.5, 4.0]);
            b.l(&[2.5, 8.0, 13.5, 8.0]);
            b.l(&[2.5, 12.0, 13.5, 12.0]);
        }
        Icon::Dot => {
            b.d(8.0, 8.0, 2.25);
        }
        Icon::Grip => {
            for y in [3.5, 8.0, 12.5] {
                b.d(6.0, y, 1.15);
                b.d(10.0, y, 1.15);
            }
        }
        Icon::Pin => {
            b.l(&[5.0, 2.0, 11.0, 2.0]);
            b.l(&[6.0, 2.0, 6.0, 6.5, 3.5, 9.5, 12.5, 9.5, 10.0, 6.5, 10.0, 2.0]);
            b.l(&[8.0, 9.5, 8.0, 14.5]);
        }
        Icon::External => {
            b.l(&[7.0, 3.5, 3.5, 3.5, 3.5, 12.5, 12.5, 12.5, 12.5, 9.0]);
            b.l(&[7.5, 8.5, 13.5, 2.5]);
            b.l(&[9.5, 2.5, 13.5, 2.5, 13.5, 6.5]);
        }
        Icon::SidebarLeft => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.l(&[6.0, 2.5, 6.0, 13.5]);
        }
        Icon::SidebarRight => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.l(&[10.0, 2.5, 10.0, 13.5]);
        }
        Icon::PanelBottom => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.l(&[1.5, 9.5, 14.5, 9.5]);
        }
        // actions -----------------------------------------------------------------
        Icon::New => {
            page(b);
            b.l(&[8.0, 7.5, 8.0, 12.5]);
            b.l(&[5.5, 10.0, 10.5, 10.0]);
        }
        Icon::Open => {
            folder(b);
            b.arrow(8.0, 11.75, 8.0, 7.0, 2.75);
        }
        Icon::Save => {
            b.z(&[2.5, 2.5, 11.0, 2.5, 13.5, 5.0, 13.5, 13.5, 2.5, 13.5]);
            b.l(&[5.0, 2.5, 5.0, 5.5, 10.0, 5.5, 10.0, 2.5]);
            b.l(&[4.5, 13.5, 4.5, 9.0, 11.5, 9.0, 11.5, 13.5]);
        }
        Icon::SaveAll => {
            b.z(&[1.5, 4.5, 9.0, 4.5, 11.5, 7.0, 11.5, 14.5, 1.5, 14.5]);
            b.l(&[4.0, 4.5, 4.0, 7.25, 8.0, 7.25, 8.0, 4.5]);
            b.l(&[3.5, 14.5, 3.5, 10.5, 9.5, 10.5, 9.5, 14.5]);
            b.l(&[4.5, 4.5, 4.5, 1.5, 12.0, 1.5, 14.5, 4.0, 14.5, 11.5, 11.5, 11.5]);
        }
        Icon::Reload => {
            b.m().arc(8.0, 8.0, 6.0, 0.0, 315.0).to(14.0, 5.33).end(false);
            b.l(&[14.0, 2.0, 14.0, 5.33, 10.67, 5.33]);
        }
        Icon::Undo => {
            b.m_at(2.75, 6.0).to(9.5, 6.0).arc(9.5, 9.5, 3.5, 270.0, 450.0).to(7.0, 13.0).end(false);
            b.l(&[6.0, 2.75, 2.75, 6.0, 6.0, 9.25]);
        }
        Icon::Redo => {
            b.m_at(13.25, 6.0).to(6.5, 6.0).arc(6.5, 9.5, 3.5, 270.0, 90.0).to(9.0, 13.0).end(false);
            b.l(&[10.0, 2.75, 13.25, 6.0, 10.0, 9.25]);
        }
        Icon::Cut => {
            b.c(4.5, 12.0, 2.0);
            b.c(11.5, 12.0, 2.0);
            b.l(&[5.63, 10.35, 11.0, 2.5]);
            b.l(&[10.37, 10.35, 5.0, 2.5]);
        }
        Icon::Copy => {
            two_sheets(b);
        }
        Icon::Paste => {
            clipboard(b);
        }
        Icon::Duplicate => {
            two_sheets(b);
            b.l(&[9.75, 7.75, 9.75, 11.75]);
            b.l(&[7.75, 9.75, 11.75, 9.75]);
        }
        Icon::Rename => {
            b.z(&[11.0, 2.5, 13.5, 5.0, 5.5, 13.0, 2.5, 13.5, 3.0, 10.5]);
            b.l(&[9.5, 4.0, 12.0, 6.5]);
            b.l(&[9.0, 13.5, 13.5, 13.5]);
        }
        Icon::Delete => {
            b.l(&[2.5, 4.0, 13.5, 4.0]);
            b.l(&[6.0, 4.0, 6.0, 2.5, 10.0, 2.5, 10.0, 4.0]);
            b.l(&[3.75, 4.0, 4.5, 14.0, 11.5, 14.0, 12.25, 4.0]);
            b.l(&[6.75, 7.0, 6.75, 11.0]);
            b.l(&[9.25, 7.0, 9.25, 11.0]);
        }
        Icon::Plus => {
            b.l(&[8.0, 3.0, 8.0, 13.0]);
            b.l(&[3.0, 8.0, 13.0, 8.0]);
        }
        Icon::Minus => {
            b.l(&[3.0, 8.0, 13.0, 8.0]);
        }
        Icon::Clear => {
            b.c(8.0, 8.0, 6.25);
            b.l(&[5.75, 5.75, 10.25, 10.25]);
            b.l(&[10.25, 5.75, 5.75, 10.25]);
        }
        Icon::Check => {
            b.l(&[3.0, 8.5, 6.5, 12.0, 13.0, 4.5]);
        }
        Icon::Search => {
            lens(b, 7.0, 7.0, 4.75, 14.0);
        }
        Icon::Filter => {
            b.z(&[2.0, 3.0, 14.0, 3.0, 9.5, 8.5, 9.5, 13.5, 6.5, 12.0, 6.5, 8.5]);
        }
        Icon::Sort => {
            b.arrow(5.0, 2.5, 5.0, 13.5, 3.0);
            b.arrow(11.0, 13.5, 11.0, 2.5, 3.0);
        }
        Icon::SortAsc => {
            b.arrow(4.0, 2.5, 4.0, 13.5, 3.0);
            b.l(&[8.0, 4.0, 10.0, 4.0]);
            b.l(&[8.0, 8.0, 12.0, 8.0]);
            b.l(&[8.0, 12.0, 14.0, 12.0]);
        }
        Icon::SortDesc => {
            b.arrow(4.0, 2.5, 4.0, 13.5, 3.0);
            b.l(&[8.0, 4.0, 14.0, 4.0]);
            b.l(&[8.0, 8.0, 12.0, 8.0]);
            b.l(&[8.0, 12.0, 10.0, 12.0]);
        }
        Icon::Settings => {
            gear(b, 8.0, 8.0, 6.5, 4.75, 8);
            b.c(8.0, 8.0, 2.25);
        }
        Icon::Eye => {
            eye_shape(b);
            b.d(8.0, 8.0, 2.25);
        }
        Icon::EyeOff => {
            eye_shape(b);
            b.d(8.0, 8.0, 2.25);
            b.l(&[2.5, 2.5, 13.5, 13.5]);
        }
        Icon::Lock => {
            b.r(3.0, 7.5, 13.0, 14.0, 1.5);
            b.m_at(5.25, 7.5).to(5.25, 5.0).arc(8.0, 5.0, 2.75, 180.0, 360.0).to(10.75, 7.5).end(false);
            b.d(8.0, 10.75, 1.0);
        }
        Icon::Unlock => {
            b.r(3.0, 7.5, 13.0, 14.0, 1.5);
            b.m_at(5.25, 7.5).to(5.25, 5.0).arc(8.0, 5.0, 2.75, 180.0, 345.0).end(false);
            b.d(8.0, 10.75, 1.0);
        }
        Icon::Link => {
            b.m_at(6.0, 11.33).to(4.67, 11.33).arc(4.67, 8.0, 3.33, 90.0, 270.0).to(6.0, 4.67).end(false);
            b.m_at(10.0, 4.67).to(11.33, 4.67).arc(11.33, 8.0, 3.33, 270.0, 450.0).to(10.0, 11.33).end(false);
            b.l(&[5.33, 8.0, 10.67, 8.0]);
        }
        Icon::Unlink => {
            b.m_at(5.5, 11.0).to(4.25, 11.0).arc(4.25, 8.0, 3.0, 90.0, 270.0).to(5.5, 5.0).end(false);
            b.m_at(10.5, 5.0).to(11.75, 5.0).arc(11.75, 8.0, 3.0, 270.0, 450.0).to(10.5, 11.0).end(false);
            b.l(&[8.0, 2.0, 8.0, 3.5]);
            b.l(&[8.0, 12.5, 8.0, 14.0]);
        }
        Icon::Star => {
            star(b, 8.0, 8.5, 6.75, 3.0, 5, false);
        }
        Icon::StarFilled => {
            star(b, 8.0, 8.5, 6.75, 3.0, 5, true);
        }
        Icon::Bookmark => {
            b.z(&[4.0, 2.0, 12.0, 2.0, 12.0, 14.0, 8.0, 10.75, 4.0, 14.0]);
        }
        Icon::BookmarkFilled => {
            b.fz(&[4.0, 2.0, 12.0, 2.0, 12.0, 14.0, 8.0, 10.75, 4.0, 14.0]);
        }
        Icon::BookmarkPrev => {
            b.z(&[7.0, 2.5, 13.0, 2.5, 13.0, 13.5, 10.0, 11.0, 7.0, 13.5]);
            b.l(&[4.5, 5.5, 2.0, 8.0, 4.5, 10.5]);
        }
        Icon::BookmarkNext => {
            b.z(&[3.0, 2.5, 9.0, 2.5, 9.0, 13.5, 6.0, 11.0, 3.0, 13.5]);
            b.l(&[11.5, 5.5, 14.0, 8.0, 11.5, 10.5]);
        }
        Icon::History => {
            b.m().arc(8.0, 8.0, 6.0, 180.0, -135.0).to(2.0, 5.33).end(false);
            b.l(&[2.0, 2.0, 2.0, 5.33, 5.33, 5.33]);
            b.l(&[8.0, 4.67, 8.0, 8.0, 10.67, 9.33]);
        }
        Icon::Back => {
            b.l(&[13.0, 8.0, 3.0, 8.0]);
            b.l(&[7.5, 3.5, 3.0, 8.0, 7.5, 12.5]);
        }
        Icon::Forward => {
            b.l(&[3.0, 8.0, 13.0, 8.0]);
            b.l(&[8.5, 3.5, 13.0, 8.0, 8.5, 12.5]);
        }
        // to the parent folder: a corner arrow, so it never reads as Move Up
        Icon::Up => {
            b.l(&[9.33, 6.0, 6.0, 2.67, 2.67, 6.0]);
            b.m_at(13.33, 13.33).to(8.67, 13.33).arc(8.67, 10.67, 2.67, 90.0, 180.0).to(6.0, 2.67).end(false);
        }
        Icon::Home => {
            b.l(&[2.0, 7.5, 8.0, 2.5, 14.0, 7.5]);
            b.l(&[3.5, 6.25, 3.5, 14.0, 12.5, 14.0, 12.5, 6.25]);
            b.l(&[6.5, 14.0, 6.5, 10.0, 9.5, 10.0, 9.5, 14.0]);
        }
        // Show in Explorer: the folder, and a magnifier in its corner finding the file there
        Icon::Reveal => {
            b.l(&[6.75, 13.5, 1.5, 13.5, 1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 14.5, 4.75, 14.5, 7.0]);
            lens(b, 10.75, 10.75, 2.5, 14.5);
        }
        Icon::Info => {
            b.c(8.0, 8.0, 6.25);
            b.l(&[8.0, 7.25, 8.0, 11.25]);
            b.d(8.0, 4.9, 1.0);
        }
        Icon::Help => {
            b.c(8.0, 8.0, 6.25);
            b.m().arc(8.0, 6.5, 2.1, 200.0, 380.0).to(8.0, 9.0).end(false);
            b.d(8.0, 11.5, 1.0);
        }
        Icon::Keyboard => {
            b.r(1.5, 4.0, 14.5, 12.0, 1.5);
            for x in [4.25, 6.75, 9.25, 11.75] {
                b.d(x, 6.75, 0.8);
            }
            b.l(&[5.0, 9.5, 11.0, 9.5]);
        }
        Icon::ExpandAll => {
            b.l(&[4.5, 6.0, 8.0, 2.5, 11.5, 6.0]);
            b.l(&[4.5, 10.0, 8.0, 13.5, 11.5, 10.0]);
        }
        Icon::CollapseAll => {
            b.l(&[4.5, 2.5, 8.0, 6.0, 11.5, 2.5]);
            b.l(&[4.5, 13.5, 8.0, 10.0, 11.5, 13.5]);
        }
        Icon::MoveUp => {
            b.arrow(8.0, 13.5, 8.0, 2.5, 4.5);
        }
        Icon::MoveDown => {
            b.arrow(8.0, 2.5, 8.0, 13.5, 4.5);
        }
        Icon::Globe => {
            b.c(8.0, 8.0, 6.25);
            b.e(8.0, 8.0, 2.75, 6.25);
            b.l(&[1.75, 8.0, 14.25, 8.0]);
        }
        Icon::Key => {
            b.c(5.0, 11.0, 3.0);
            b.l(&[7.12, 8.88, 13.5, 2.5]);
            b.l(&[11.0, 5.0, 13.0, 7.0]);
            b.l(&[9.25, 6.75, 10.75, 8.25]);
        }
        // the sides are arcs tangent to the straight upper sides, meeting in a point at the foot
        Icon::Shield => {
            b.m_at(8.0, 1.5)
                .to(13.5, 3.5)
                .to(13.5, 8.0)
                .arc(6.91, 8.0, 6.59, 0.0, 80.5)
                .arc(9.09, 8.0, 6.59, 99.5, 180.0)
                .to(2.5, 3.5)
                .end(true);
        }
        Icon::Tag => {
            b.z(&[2.0, 2.0, 8.5, 2.0, 14.25, 7.75, 7.75, 14.25, 2.0, 8.5]);
            b.d(5.25, 5.25, 1.1);
        }
        Icon::Picture => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.c(5.5, 6.0, 1.5);
            b.l(&[1.5, 12.0, 6.0, 8.0, 9.0, 11.0, 11.0, 9.0, 14.5, 12.5]);
        }
        Icon::Terminal => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.l(&[4.5, 6.0, 6.5, 8.0, 4.5, 10.0]);
            b.l(&[8.0, 10.5, 11.0, 10.5]);
        }
        Icon::Refresh => {
            b.m().arc(8.0, 8.0, 6.0, 180.0, 315.0).to(14.0, 5.33).end(false);
            b.l(&[14.0, 2.0, 14.0, 5.33, 10.67, 5.33]);
            b.m().arc(8.0, 8.0, 6.0, 0.0, 135.0).to(2.0, 10.67).end(false);
            b.l(&[5.33, 10.67, 2.0, 10.67, 2.0, 14.0]);
        }
        // status ------------------------------------------------------------------
        Icon::Error => {
            regular(b, 8.0, 8.0, 6.5, 8, 22.5);
            b.l(&[8.0, 4.75, 8.0, 8.75]);
            b.d(8.0, 11.0, 1.0);
        }
        Icon::Warning => {
            b.z(&[8.0, 2.0, 14.5, 13.5, 1.5, 13.5]);
            b.l(&[8.0, 6.25, 8.0, 9.5]);
            b.d(8.0, 11.5, 0.9);
        }
        Icon::Success => {
            b.c(8.0, 8.0, 6.25);
            b.l(&[5.0, 8.25, 7.25, 10.5, 11.0, 6.0]);
        }
        Icon::Required => {
            b.l(&[8.0, 2.5, 8.0, 13.5]);
            b.l(&[3.24, 5.25, 12.76, 10.75]);
            b.l(&[3.24, 10.75, 12.76, 5.25]);
        }
        // still, the way a screenshot shows it; the kit turns it
        Icon::Spinner => {
            b.quiet(true).c(8.0, 8.0, 5.5);
            b.quiet(false).a(8.0, 8.0, 5.5, 270.0, 400.0);
        }
        Icon::Dirty => {
            b.d(8.0, 8.0, 3.25);
        }
        // build and run -----------------------------------------------------------
        // the collision form is triangles
        Icon::BuildCollision => {
            b.z(&[8.0, 2.0, 14.5, 13.5, 1.5, 13.5]);
            b.z(&[4.75, 7.75, 11.25, 7.75, 8.0, 13.5]);
        }
        // the AI map is a floor of cells, seen in perspective
        Icon::BuildAIMap => {
            b.z(&[5.0, 3.0, 11.0, 3.0, 14.5, 13.0, 1.5, 13.0]);
            b.l(&[7.0, 3.0, 5.83, 13.0]);
            b.l(&[9.0, 3.0, 10.17, 13.0]);
            b.l(&[3.6, 7.0, 12.4, 7.0]);
        }
        Icon::BuildGameGraph => {
            b.c(8.0, 3.5, 2.0);
            b.c(3.5, 12.0, 2.0);
            b.c(12.5, 12.0, 2.0);
            b.l(&[7.06, 5.27, 4.44, 10.23]);
            b.l(&[8.94, 5.27, 11.56, 10.23]);
            b.l(&[5.5, 12.0, 10.5, 12.0]);
        }
        Icon::BuildAll => {
            b.z(&[8.0, 2.0, 14.5, 5.25, 8.0, 8.5, 1.5, 5.25]);
            b.l(&[1.5, 8.25, 8.0, 11.5, 14.5, 8.25]);
            b.l(&[1.5, 11.25, 8.0, 14.5, 14.5, 11.25]);
        }
        Icon::Play => {
            b.fz(&[4.5, 2.5, 13.0, 8.0, 4.5, 13.5]);
        }
        Icon::PlayGame => {
            b.r(1.5, 2.0, 14.5, 11.0, 1.5);
            b.l(&[8.0, 11.0, 8.0, 14.0]);
            b.l(&[5.0, 14.0, 11.0, 14.0]);
            b.fz(&[6.75, 4.5, 10.25, 6.5, 6.75, 8.5]);
        }
        Icon::PlayClean => {
            b.z(&[3.0, 3.0, 10.5, 8.25, 3.0, 13.5]);
            b.l(&[12.5, 1.5, 12.5, 5.5]);
            b.l(&[10.5, 3.5, 14.5, 3.5]);
        }
        Icon::Stop => {
            b.rf(3.5, 3.5, 12.5, 12.5, 1.5);
            b.r(3.5, 3.5, 12.5, 12.5, 1.5);
        }
        Icon::ReloadConfigs => {
            page_cut(b);
            b.arc_arrow(11.5, 11.5, 2.75, 2.75, 180.0, 450.0, 2.25);
        }
        Icon::GameFolder => {
            folder(b);
            b.fz(&[6.5, 7.0, 10.5, 9.25, 6.5, 11.5]);
        }
        // a mod is a piece that fits the game: a puzzle piece
        Icon::BuildMod => {
            b.m_at(2.5, 4.5)
                .to(6.33, 4.5)
                .arc(8.0, 3.6, 1.9, 151.7, 388.3)
                .to(11.5, 4.5)
                .to(11.5, 7.58)
                .arc(12.4, 9.25, 1.9, 241.7, 478.3)
                .to(11.5, 14.0)
                .to(2.5, 14.0)
                .end(true);
        }
        Icon::Package => {
            b.r(1.5, 2.5, 14.5, 5.5, 1.0);
            b.l(&[2.5, 5.5, 2.5, 13.5, 13.5, 13.5, 13.5, 5.5]);
            b.l(&[6.5, 8.5, 9.5, 8.5]);
        }
        Icon::Publish => {
            b.arrow(8.0, 11.0, 8.0, 2.0, 4.5);
            b.l(&[2.0, 10.0, 2.0, 14.0, 14.0, 14.0, 14.0, 10.0]);
        }
        Icon::Releases => {
            b.z(&[1.5, 3.5, 6.75, 3.5, 11.5, 8.25, 7.0, 12.75, 1.5, 7.25]);
            b.d(4.25, 6.25, 1.0);
            b.l(&[9.25, 3.5, 14.0, 8.25, 9.5, 12.75]);
        }
        // viewport ----------------------------------------------------------------
        Icon::Select => {
            b.z(&[3.0, 3.0, 13.5, 7.25, 9.0, 9.0, 7.25, 13.5]);
        }
        // dropped onto the ground
        Icon::Place => {
            b.arrow(8.0, 1.5, 8.0, 9.0, 3.5);
            b.e(8.0, 12.0, 5.5, 2.0);
        }
        Icon::Move => {
            b.l(&[8.0, 1.5, 8.0, 14.5]);
            b.l(&[1.5, 8.0, 14.5, 8.0]);
            b.l(&[6.0, 3.5, 8.0, 1.5, 10.0, 3.5]);
            b.l(&[6.0, 12.5, 8.0, 14.5, 10.0, 12.5]);
            b.l(&[3.5, 6.0, 1.5, 8.0, 3.5, 10.0]);
            b.l(&[12.5, 6.0, 14.5, 8.0, 12.5, 10.0]);
        }
        // turning about a pivot: a circle that stops short of itself around a dot - the dot is what
        // keeps it apart from Reload
        Icon::Rotate => {
            b.arc_arrow(8.0, 8.0, 5.5, 5.5, 135.0, 405.0, 2.75);
            b.d(8.0, 8.0, 1.5);
        }
        Icon::Scale => {
            b.l(&[8.0, 2.0, 2.0, 2.0, 2.0, 14.0, 14.0, 14.0, 14.0, 8.0]);
            b.l(&[9.33, 10.0, 6.0, 10.0, 6.0, 6.67]);
            b.l(&[10.67, 2.0, 14.0, 2.0, 14.0, 5.33]);
            b.l(&[14.0, 2.0, 6.0, 10.0]);
        }
        Icon::SnapGrid => {
            b.l(&[5.5, 1.5, 5.5, 14.5]);
            b.l(&[10.5, 1.5, 10.5, 14.5]);
            b.l(&[1.5, 5.5, 14.5, 5.5]);
            b.l(&[1.5, 10.5, 14.5, 10.5]);
        }
        Icon::SnapAngle => {
            b.l(&[14.0, 13.5, 2.0, 13.5, 10.5, 3.5]);
            b.a(2.0, 13.5, 6.5, -49.6, 0.0);
        }
        // a small box growing into a larger one
        Icon::SnapScale => {
            b.z(&[2.0, 9.0, 7.0, 9.0, 7.0, 14.0, 2.0, 14.0]);
            b.l(&[2.0, 6.0, 2.0, 2.0, 14.0, 2.0, 14.0, 14.0, 10.0, 14.0]);
            b.l(&[7.5, 8.5, 11.5, 4.5]);
            b.l(&[8.5, 4.5, 11.5, 4.5, 11.5, 7.5]);
        }
        Icon::SnapObject => {
            b.m_at(3.0, 2.5)
                .to(3.0, 8.5)
                .arc(8.0, 8.5, 5.0, 180.0, 0.0)
                .to(13.0, 2.5)
                .to(9.5, 2.5)
                .to(9.5, 8.5)
                .arc(8.0, 8.5, 1.5, 0.0, 180.0)
                .to(6.5, 2.5)
                .end(true);
            b.l(&[3.0, 5.5, 6.5, 5.5]);
            b.l(&[9.5, 5.5, 13.0, 5.5]);
        }
        Icon::FrameAll => {
            corners(b);
            b.r(5.0, 5.0, 11.0, 11.0, 1.0);
        }
        Icon::FrameSelection => {
            corners(b);
            b.d(8.0, 8.0, 2.25);
        }
        // the view volume: widening from the eye, or parallel
        Icon::Perspective => {
            b.z(&[4.5, 6.0, 14.0, 2.5, 14.0, 13.5, 4.5, 10.0]);
            b.d(2.0, 8.0, 1.1);
        }
        Icon::Ortho => {
            b.z(&[4.5, 4.0, 14.0, 4.0, 14.0, 12.0, 4.5, 12.0]);
            b.d(2.0, 8.0, 1.1);
        }
        Icon::Lit => {
            b.c(8.0, 8.0, 3.0);
            for i in 0..8 {
                let a = f64::from(i) * 45.0 * DEG;
                b.l(&[8.0 + 5.0 * a.cos(), 8.0 + 5.0 * a.sin(), 8.0 + 6.75 * a.cos(), 8.0 + 6.75 * a.sin()]);
            }
        }
        Icon::Unlit => {
            b.c(8.0, 8.0, 3.0);
            for i in 0..8 {
                let a = f64::from(i) * 45.0 * DEG;
                b.d(8.0 + 5.75 * a.cos(), 8.0 + 5.75 * a.sin(), 0.9);
            }
        }
        Icon::Wireframe => {
            b.quiet(true).z(&[5.0, 2.0, 14.0, 2.0, 14.0, 11.0, 5.0, 11.0]);
            b.l(&[2.0, 14.0, 5.0, 11.0]);
            b.quiet(false).z(&[2.0, 5.0, 11.0, 5.0, 11.0, 14.0, 2.0, 14.0]);
            b.l(&[2.0, 5.0, 5.0, 2.0]);
            b.l(&[11.0, 5.0, 14.0, 2.0]);
            b.l(&[11.0, 14.0, 14.0, 11.0]);
        }
        // a bone the way a rig shows one: a joint, a long diamond, a joint
        Icon::Bones => {
            b.c(4.0, 12.0, 1.75);
            b.c(12.5, 3.5, 1.25);
            b.z(&[5.24, 10.76, 8.18, 10.51, 11.62, 4.38, 5.5, 7.83]);
        }
        Icon::Stats => {
            b.l(&[2.0, 2.0, 2.0, 14.0, 14.0, 14.0]);
            b.l(&[5.33, 11.33, 5.33, 9.33]);
            b.l(&[8.67, 11.33, 8.67, 3.33]);
            b.l(&[12.0, 11.33, 12.0, 6.0]);
        }
        Icon::ViewOptions => {
            b.l(&[2.0, 4.5, 8.5, 4.5]);
            b.c(10.5, 4.5, 2.0);
            b.l(&[12.5, 4.5, 14.0, 4.5]);
            b.l(&[2.0, 11.5, 3.5, 11.5]);
            b.c(5.5, 11.5, 2.0);
            b.l(&[7.5, 11.5, 14.0, 11.5]);
        }
        // scene object classes ----------------------------------------------------
        Icon::SceneObject => {
            b.z(&[8.0, 1.5, 14.0, 4.75, 14.0, 11.25, 8.0, 14.5, 2.0, 11.25, 2.0, 4.75]);
            b.l(&[2.0, 4.75, 8.0, 8.0, 14.0, 4.75]);
            b.l(&[8.0, 8.0, 8.0, 14.5]);
        }
        Icon::Light => {
            b.m_at(6.0, 12.0).to(4.82, 9.68).arc(8.0, 6.5, 4.5, 135.0, 405.0).to(10.0, 12.0).end(false);
            b.l(&[6.0, 12.0, 10.0, 12.0]);
            b.l(&[6.5, 14.25, 9.5, 14.25]);
        }
        Icon::SoundSource => {
            b.z(&[2.0, 6.0, 5.0, 6.0, 9.0, 2.5, 9.0, 13.5, 5.0, 10.0, 2.0, 10.0]);
            b.a(9.0, 8.0, 3.0, -45.0, 45.0);
            b.a(9.0, 8.0, 5.5, -45.0, 45.0);
        }
        // sound that fills a space: waves to both sides
        Icon::SoundEnv => {
            b.d(8.0, 8.0, 1.25);
            b.a(8.0, 8.0, 3.0, -50.0, 50.0);
            b.a(8.0, 8.0, 3.0, 130.0, 230.0);
            b.a(8.0, 8.0, 5.75, -50.0, 50.0);
            b.a(8.0, 8.0, 5.75, 130.0, 230.0);
        }
        Icon::Glow => {
            b.z(&[8.0, 1.5, 9.5, 6.5, 14.5, 8.0, 9.5, 9.5, 8.0, 14.5, 6.5, 9.5, 1.5, 8.0, 6.5, 6.5]);
        }
        Icon::Shape => {
            b.c(5.5, 10.5, 3.75);
            b.r(8.0, 2.0, 14.0, 8.0, 1.0);
        }
        Icon::Spawn => {
            b.m().arc(8.0, 6.5, 5.0, 150.0, 390.0).to(8.0, 14.5).end(true);
            b.c(8.0, 6.5, 1.75);
        }
        Icon::Way => {
            b.c(4.0, 12.67, 2.0);
            b.c(12.0, 3.33, 2.0);
            b.m_at(6.0, 12.67)
                .to(11.67, 12.67)
                .arc(11.67, 10.33, 2.33, 90.0, -90.0)
                .to(4.33, 8.0)
                .arc(4.33, 5.67, 2.33, 90.0, 270.0)
                .to(10.0, 3.33)
                .end(false);
        }
        Icon::Sector => {
            b.z(&[2.0, 5.0, 7.0, 2.0, 14.0, 4.0, 13.0, 12.5, 6.5, 14.0, 2.0, 11.0]);
        }
        Icon::Portal => {
            b.e(8.0, 8.0, 3.75, 6.5);
            b.quiet(true).e(8.0, 8.0, 1.75, 4.25);
        }
        Icon::Group => {
            b.dashed(true).r(1.5, 1.5, 14.5, 14.5, 2.0);
            b.dashed(false).r(4.0, 4.0, 8.5, 8.5, 1.0);
            b.r(7.5, 7.5, 12.0, 12.0, 1.0);
        }
        Icon::Particles => {
            b.d(4.0, 11.0, 1.5);
            b.d(8.0, 5.0, 1.1);
            b.d(12.0, 9.0, 1.3);
            b.d(10.5, 13.5, 0.9);
            b.d(3.5, 4.0, 0.9);
            b.d(13.0, 3.0, 0.8);
        }
        Icon::DetailObjects => {
            b.l(&[1.5, 14.0, 14.5, 14.0]);
            b.l(&[4.0, 14.0, 2.5, 7.0]);
            b.l(&[6.5, 14.0, 6.5, 4.0]);
            b.l(&[9.0, 14.0, 10.5, 6.5]);
            b.l(&[11.5, 14.0, 13.5, 9.0]);
        }
        // nodes on a lattice
        Icon::AIMap => {
            b.quiet(true);
            for v in [3.5, 8.0, 12.5] {
                b.l(&[3.5, v, 12.5, v]);
                b.l(&[v, 3.5, v, 12.5]);
            }
            b.quiet(false);
            for y in [3.5, 8.0, 12.5] {
                for x in [3.5, 8.0, 12.5] {
                    b.d(x, y, 1.2);
                }
            }
        }
        // a splat on a wall
        Icon::Wallmark => {
            b.r(1.5, 1.5, 14.5, 14.5, 1.5);
            star(b, 8.0, 8.0, 4.5, 2.25, 7, false);
        }
        Icon::FogVolume => {
            cloud(b);
            b.l(&[3.0, 12.5, 13.0, 12.5]);
            b.l(&[5.5, 14.5, 10.5, 14.5]);
        }
        // files -------------------------------------------------------------------
        Icon::Folder => {
            folder(b);
        }
        Icon::FolderOpen => {
            b.l(&[1.5, 13.5, 1.5, 3.0, 6.0, 3.0, 7.5, 4.75, 13.0, 4.75, 13.0, 7.0]);
            b.z(&[1.5, 13.5, 4.0, 7.0, 14.5, 7.0, 12.0, 13.5]);
        }
        Icon::File => {
            page(b);
        }
        Icon::FileTexture => {
            page(b);
            b.d(6.5, 7.5, 1.0);
            b.l(&[5.0, 12.5, 7.0, 10.0, 8.5, 11.5, 9.5, 10.5, 11.0, 12.5]);
        }
        Icon::FileModel => {
            page(b);
            b.z(&[8.0, 7.0, 10.5, 8.25, 10.5, 11.25, 8.0, 12.5, 5.5, 11.25, 5.5, 8.25]);
            b.l(&[5.5, 8.25, 8.0, 9.5, 10.5, 8.25]);
            b.l(&[8.0, 9.5, 8.0, 12.5]);
        }
        Icon::FileSound => {
            page(b);
            b.c(6.75, 11.75, 1.25);
            b.l(&[8.0, 11.75, 8.0, 7.0, 10.25, 8.0]);
        }
        Icon::FileScript => {
            page(b);
            b.l(&[
                7.0, 7.25, 6.25, 7.25, 5.75, 7.75, 5.75, 9.25, 5.0, 9.75, 5.75, 10.25, 5.75, 11.75, 6.25,
                12.25, 7.0, 12.25,
            ]);
            b.l(&[
                9.0, 7.25, 9.75, 7.25, 10.25, 7.75, 10.25, 9.25, 11.0, 9.75, 10.25, 10.25, 10.25, 11.75,
                9.75, 12.25, 9.0, 12.25,
            ]);
        }
        // keys and values: sliders, not lines of text
        Icon::FileConfig => {
            page(b);
            b.l(&[5.5, 8.5, 10.5, 8.5]);
            b.d(9.0, 8.5, 1.1);
            b.l(&[5.5, 11.75, 10.5, 11.75]);
            b.d(7.0, 11.75, 1.1);
        }
        Icon::FileXml => {
            page(b);
            b.l(&[6.5, 8.0, 5.0, 9.75, 6.5, 11.5]);
            b.l(&[9.5, 8.0, 11.0, 9.75, 9.5, 11.5]);
            b.l(&[8.75, 7.5, 7.25, 12.0]);
        }
        Icon::FileText => {
            page(b);
            b.l(&[5.5, 8.0, 10.5, 8.0]);
            b.l(&[5.5, 10.25, 10.5, 10.25]);
            b.l(&[5.5, 12.5, 8.5, 12.5]);
        }
        Icon::FileArchive => {
            page(b);
            b.l(&[6.75, 3.0, 6.75, 4.0]);
            b.l(&[6.75, 5.5, 6.75, 6.5]);
            b.r(5.75, 8.25, 7.75, 11.25, 1.0);
        }
        Icon::Level => {
            b.z(&[1.5, 3.5, 5.5, 2.0, 10.5, 3.5, 14.5, 2.0, 14.5, 12.5, 10.5, 14.0, 5.5, 12.5, 1.5, 14.0]);
            b.l(&[5.5, 2.0, 5.5, 12.5]);
            b.l(&[10.5, 3.5, 10.5, 14.0]);
        }
        // a quest is a flow: one node splitting into two
        Icon::Quest => {
            b.r(u(0.28), u(0.03), u(0.72), u(0.23), 0.77);
            b.r(u(0.02), u(0.63), u(0.44), u(0.83), 0.77);
            b.r(u(0.56), u(0.63), u(0.98), u(0.83), 0.77);
            b.quiet(true).l(&[u(0.50), u(0.23), u(0.50), u(0.44)]);
            b.l(&[u(0.23), u(0.63), u(0.23), u(0.44), u(0.77), u(0.44), u(0.77), u(0.63)]);
        }
        // a behaviour graph moves people: a route with its waypoints and an arrow at the end
        Icon::Behaviour => {
            b.l(&[u(0.07), u(0.82), u(0.30), u(0.44), u(0.55), u(0.63), u(0.753), u(0.286)]);
            // fatter than the line they sit on, or they read as kinks and not as places
            b.d(u(0.07), u(0.82), 1.3);
            b.d(u(0.30), u(0.44), 1.3);
            b.d(u(0.55), u(0.63), 1.3);
            b.f(&[u(0.86), u(0.12), u(0.826), u(0.330), u(0.672), u(0.242)]);
        }
        // a person: the head and the shoulders. The template is the same person dashed, because it
        // never becomes anybody in the game.
        Icon::Character => {
            b.c(u(0.5), u(0.3), 0.185 * 14.0);
            b.a(u(0.5), u(0.98), 0.36 * 14.0, 180.0, 360.0);
        }
        Icon::CharacterTemplate => {
            b.dashed(true).a(u(0.5), u(0.3), 0.185 * 14.0, 0.0, 360.0);
            b.a(u(0.5), u(0.98), 0.36 * 14.0, 180.0, 360.0);
        }
        // screens you switch between: the device's screen, a card on it, the tab bar at its foot
        Icon::PdaInterface => {
            b.r(u(0.04), u(0.14), u(0.96), u(0.86), 0.10 * 14.0);
            b.r(u(0.20), u(0.30), u(0.80), u(0.52), 0.04 * 14.0);
            b.quiet(true).l(&[u(0.04), u(0.64), u(0.96), u(0.64)]);
            b.quiet(false);
            for x in [0.27, 0.50, 0.73] {
                b.d(u(x), u(0.75), 0.055 * 14.0);
            }
        }
        // graph editor ------------------------------------------------------------
        Icon::Flow => {
            b.r(2.0, 2.0, 7.0, 7.0, 1.0);
            b.r(9.0, 9.0, 14.0, 14.0, 1.0);
            b.l(&[4.5, 7.0, 4.5, 11.5, 9.0, 11.5]);
        }
        Icon::Wait => {
            b.l(&[3.5, 1.5, 12.5, 1.5]);
            b.l(&[3.5, 14.5, 12.5, 14.5]);
            b.l(&[4.5, 1.5, 4.5, 4.0, 8.0, 8.0, 4.5, 12.0, 4.5, 14.5]);
            b.l(&[11.5, 1.5, 11.5, 4.0, 8.0, 8.0, 11.5, 12.0, 11.5, 14.5]);
        }
        Icon::Condition => {
            b.z(&[8.0, 1.5, 14.5, 8.0, 8.0, 14.5, 1.5, 8.0]);
        }
        Icon::Variable => {
            b.a(7.5, 8.0, 6.0, 128.0, 232.0);
            b.a(8.5, 8.0, 6.0, -52.0, 52.0);
            b.l(&[5.75, 5.75, 10.25, 10.25]);
            b.l(&[10.25, 5.75, 5.75, 10.25]);
        }
        Icon::SpawnNpc => {
            bust(b, 6.0, 4.67, 2.67, 10.0, 4.5, 14.0);
            b.l(&[12.5, 5.25, 12.5, 9.25]);
            b.l(&[10.5, 7.25, 14.5, 7.25]);
        }
        Icon::Dialog => {
            b.z(&[2.0, 2.5, 14.0, 2.5, 14.0, 11.0, 6.0, 11.0, 2.0, 14.0]);
        }
        // an item is loot: a gem
        Icon::Item => {
            b.z(&[4.0, 2.25, 12.0, 2.25, 14.5, 6.0, 8.0, 14.25, 1.5, 6.0]);
            b.l(&[7.33, 2.25, 5.5, 6.0, 8.0, 14.25, 10.5, 6.0, 8.67, 2.25]);
            b.l(&[1.5, 6.0, 14.5, 6.0]);
        }
        Icon::PdaMsg => {
            b.r(1.5, 3.0, 14.5, 13.0, 1.5);
            b.l(&[1.5, 4.75, 8.0, 9.0, 14.5, 4.75]);
        }
        Icon::World => {
            b.z(&[1.5, 13.5, 6.0, 6.0, 8.5, 10.0, 10.5, 7.5, 14.5, 13.5]);
            b.d(11.75, 3.5, 1.5);
        }
        Icon::Relation => {
            b.l(&[2.0, 5.0, 14.0, 5.0]);
            b.l(&[11.0, 2.0, 14.0, 5.0, 11.0, 8.0]);
            b.l(&[14.0, 11.0, 2.0, 11.0]);
            b.l(&[5.0, 8.0, 2.0, 11.0, 5.0, 14.0]);
        }
        Icon::Squad => {
            bust(b, 8.0, 4.25, 2.25, 8.5, 3.25, 14.0);
            b.c(3.0, 6.0, 1.5);
            b.m_at(1.5, 13.0).to(1.5, 11.0).arc(3.0, 11.0, 1.5, 180.0, 270.0).to(3.75, 9.5).end(false);
            b.c(13.0, 6.0, 1.5);
            b.m_at(14.5, 13.0).to(14.5, 11.0).arc(13.0, 11.0, 1.5, 360.0, 270.0).to(12.25, 9.5).end(false);
        }
        // an order sends somebody somewhere
        Icon::Order => {
            b.z(&[2.0, 7.5, 14.0, 2.0, 8.5, 14.0, 7.0, 9.0]);
        }
        Icon::Timer => {
            b.c(8.0, 9.0, 5.5);
            b.l(&[8.0, 9.0, 8.0, 6.0]);
            b.l(&[6.5, 1.75, 9.5, 1.75]);
            b.l(&[8.0, 1.75, 8.0, 3.5]);
        }
        Icon::Trigger => {
            b.z(&[9.0, 1.5, 3.0, 9.0, 8.0, 9.0, 7.0, 14.5, 13.0, 7.0, 8.0, 7.0]);
        }
        // the final state of a state chart
        Icon::End => {
            b.c(8.0, 8.0, 6.0);
            b.d(8.0, 8.0, 3.0);
        }
        Icon::Task => {
            b.r(2.0, 2.0, 14.0, 14.0, 2.0);
            b.l(&[5.0, 8.0, 7.25, 10.25, 11.0, 6.0]);
        }
        // a role is a part somebody is cast in: an ID card
        Icon::Role => {
            b.r(1.5, 3.0, 14.5, 13.0, 1.5);
            b.c(5.5, 7.0, 1.5);
            b.a(5.5, 11.5, 2.5, 180.0, 360.0);
            b.l(&[9.5, 6.5, 12.5, 6.5]);
            b.l(&[9.5, 9.5, 12.5, 9.5]);
        }
        Icon::Minimap => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.r(8.0, 7.0, 12.5, 11.5, 0.5);
        }
        Icon::AutoLayout => {
            b.r(2.0, 2.0, 7.0, 7.0, 1.0);
            b.r(9.0, 2.0, 14.0, 7.0, 1.0);
            b.r(2.0, 9.0, 7.0, 14.0, 1.0);
            b.r(9.0, 9.0, 14.0, 14.0, 1.0);
        }
        Icon::Validate => {
            clipboard(b);
            b.l(&[5.5, 9.5, 7.25, 11.25, 10.5, 7.75]);
        }
        Icon::FindInProject => {
            page_cut(b);
            lens(b, 10.5, 10.75, 2.5, 14.25);
        }
        Icon::ZoomIn => {
            lens(b, 7.0, 7.0, 5.0, 14.0);
            b.l(&[7.0, 4.75, 7.0, 9.25]);
            b.l(&[4.75, 7.0, 9.25, 7.0]);
        }
        Icon::ZoomOut => {
            lens(b, 7.0, 7.0, 5.0, 14.0);
            b.l(&[4.75, 7.0, 9.25, 7.0]);
        }
        Icon::ZoomFit => {
            corners(b);
            lens(b, 7.5, 7.5, 2.5, 11.0);
        }
        // "1:1"
        Icon::Zoom100 => {
            b.l(&[3.0, 5.0, 4.75, 3.5, 4.75, 12.5]);
            b.d(8.0, 6.5, 0.9);
            b.d(8.0, 10.5, 0.9);
            b.l(&[9.75, 5.0, 11.5, 3.5, 11.5, 12.5]);
        }
        // other -------------------------------------------------------------------
        Icon::Person => {
            b.c(8.0, 8.0, 6.5);
            b.c(8.0, 6.67, 2.0);
            b.m_at(4.67, 13.77)
                .to(4.67, 12.67)
                .arc(6.0, 12.67, 1.33, 180.0, 270.0)
                .to(10.0, 11.33)
                .arc(10.0, 12.67, 1.33, 270.0, 360.0)
                .to(11.33, 13.77)
                .end(false);
        }
        // a faction is a banner
        Icon::Community => {
            b.l(&[3.0, 14.5, 3.0, 2.0]);
            b.l(&[3.0, 2.5, 13.5, 2.5, 11.0, 5.5, 13.5, 8.5, 3.0, 8.5]);
        }
        Icon::Rank => {
            b.l(&[3.0, 6.0, 8.0, 2.5, 13.0, 6.0]);
            b.l(&[3.0, 10.0, 8.0, 6.5, 13.0, 10.0]);
            b.l(&[3.0, 14.0, 8.0, 10.5, 13.0, 14.0]);
        }
        Icon::Money => {
            b.r(1.5, 4.0, 14.5, 12.0, 1.5);
            b.c(8.0, 8.0, 2.0);
            b.d(4.25, 8.0, 0.8);
            b.d(11.75, 8.0, 0.8);
        }
        // a starting kit is a backpack
        Icon::Kit => {
            b.r(2.75, 4.0, 13.25, 14.5, 2.5);
            b.m_at(6.0, 4.0)
                .to(6.0, 3.0)
                .arc(7.0, 3.0, 1.0, 180.0, 270.0)
                .to(9.0, 2.0)
                .arc(9.0, 3.0, 1.0, 270.0, 360.0)
                .to(10.0, 4.0)
                .end(false);
            b.l(&[5.5, 7.0, 10.5, 7.0]);
            b.l(&[5.5, 14.5, 5.5, 10.5, 10.5, 10.5, 10.5, 14.5]);
        }
        Icon::Area => {
            b.dashed(true).z(&[2.0, 5.0, 6.5, 2.0, 14.0, 3.5, 13.5, 12.5, 5.0, 14.0, 2.0, 10.5]);
            b.dashed(false).d(8.0, 8.0, 1.5);
        }
        // a level changer: through a door
        Icon::Transition => {
            b.m_at(10.0, 2.0)
                .to(12.5, 2.0)
                .arc(12.5, 3.5, 1.5, 270.0, 360.0)
                .to(14.0, 12.5)
                .arc(12.5, 12.5, 1.5, 0.0, 90.0)
                .to(10.0, 14.0)
                .end(false);
            b.l(&[2.0, 8.0, 10.0, 8.0]);
            b.l(&[7.0, 5.0, 10.0, 8.0, 7.0, 11.0]);
        }
        Icon::PlayerStart => {
            bust(b, 8.0, 3.75, 2.0, 7.5, 3.5, 12.5);
            b.quiet(true).e(8.0, 12.5, 6.0, 2.0);
        }
        // a game mode is a way to play: a gamepad
        Icon::Mode => {
            b.r(1.5, 4.5, 14.5, 12.5, 3.5);
            b.l(&[4.0, 8.5, 7.0, 8.5]);
            b.l(&[5.5, 7.0, 5.5, 10.0]);
            b.d(10.5, 7.5, 0.9);
            b.d(12.0, 9.5, 0.9);
        }
        Icon::Relations => {
            b.c(6.0, 4.67, 2.67);
            b.m_at(10.67, 14.0)
                .to(10.67, 12.67)
                .arc(8.0, 12.67, 2.67, 0.0, -90.0)
                .to(4.17, 10.0)
                .arc(4.17, 12.67, 2.67, 270.0, 180.0)
                .to(1.5, 14.0)
                .end(false);
            b.a(10.0, 4.67, 2.67, -76.0, 76.0);
            b.m_at(14.5, 14.0).to(14.5, 12.67).arc(12.5, 12.67, 2.0, 0.0, -80.0).end(false);
        }
        Icon::Exclude => {
            b.c(8.0, 8.0, 6.25);
            b.l(&[3.58, 3.58, 12.42, 12.42]);
        }
        Icon::Aspect => {
            b.r(1.5, 3.0, 14.5, 13.0, 1.5);
            b.l(&[4.0, 7.5, 4.0, 5.5, 6.5, 5.5]);
            b.l(&[12.0, 8.5, 12.0, 10.5, 9.5, 10.5]);
        }
        Icon::Language => {
            b.l(&[3.33, 5.33, 7.33, 9.33]);
            b.l(&[2.67, 9.33, 6.67, 5.33, 8.0, 3.33]);
            b.l(&[1.5, 3.33, 9.33, 3.33]);
            b.l(&[4.67, 1.5, 5.33, 1.5]);
            b.l(&[14.5, 14.5, 11.33, 8.0, 8.0, 14.5]);
            b.l(&[9.33, 12.0, 13.33, 12.0]);
        }
        Icon::Tasks => {
            b.l(&[2.0, 4.67, 3.33, 6.0, 6.0, 3.33]);
            b.l(&[2.0, 11.33, 3.33, 12.67, 6.0, 10.0]);
            b.l(&[8.67, 4.0, 14.0, 4.0]);
            b.l(&[8.67, 8.0, 14.0, 8.0]);
            b.l(&[8.67, 12.0, 14.0, 12.0]);
        }
        // the agents' connection: a plug
        Icon::Mcp => {
            b.l(&[5.5, 1.5, 5.5, 4.5]);
            b.l(&[10.5, 1.5, 10.5, 4.5]);
            b.m_at(3.5, 4.5).to(12.5, 4.5).to(12.5, 7.0).arc(8.0, 7.0, 4.5, 0.0, 180.0).end(true);
            b.l(&[8.0, 11.5, 8.0, 14.5]);
        }
        Icon::Log => {
            b.l(&[2.0, 3.0, 14.0, 3.0]);
            b.l(&[2.0, 6.33, 11.0, 6.33]);
            b.l(&[2.0, 9.67, 14.0, 9.67]);
            b.l(&[2.0, 13.0, 8.0, 13.0]);
        }
        Icon::Problems => {
            b.z(&[4.5, 2.0, 7.5, 7.5, 1.5, 7.5]);
            b.l(&[10.0, 4.75, 14.5, 4.75]);
            b.l(&[1.5, 10.75, 14.5, 10.75]);
            b.l(&[1.5, 14.0, 10.0, 14.0]);
        }
        Icon::Drawer => {
            b.r(1.5, 2.5, 14.5, 13.5, 1.5);
            b.l(&[1.5, 10.5, 14.5, 10.5]);
            b.l(&[5.5, 7.5, 8.0, 5.0, 10.5, 7.5]);
        }
    }
}
