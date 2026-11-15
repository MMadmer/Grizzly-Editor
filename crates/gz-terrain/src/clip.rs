//! The exact cut of a triangle by a region: a line-by-line port of the game's
//! `XMS::Cut::ClipTriangle` (`src/xrCore/XMS/xms_cut_clip.cpp`, the game's
//! `docs/dead-air/XMS_ARCHITECTURE.md` 8.4), in single precision, with the same constants, the
//! same order of the four lines and the same weights - the editor's preview and the game cut the
//! same triangles the same way. [`clip_mesh`] runs it over a triangle soup.

use glam::{Vec2, Vec3};

/// A point this near a line is inside, metres.
const ON_LINE: f32 = 1e-5;
/// The smallest polygon that counts, square metres.
const MIN_AREA: f32 = 1e-8;
const MAX_POLYGON: usize = 12;
/// What a clip may give at most: the game's `max_points` and `max_triangles`.
pub const MAX_POINTS: usize = 32;
pub const MAX_TRIANGLES: usize = 16;

/// What a region removes: everything inside its box, or the ground near its old heights.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CutKind {
    /// 0: every part inside the box's heights.
    Everything,
    /// 1: every part within `tolerance` of the old ground.
    Ground,
}

impl CutKind {
    #[must_use]
    pub fn id(self) -> u8 {
        match self {
            Self::Everything => 0,
            Self::Ground => 1,
        }
    }
}

/// A region of `overlay.cut`.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub kind: CutKind,
    pub centre: Vec3,
    pub half: Vec3,
    /// Radians about +Y: a point's local coordinates are `p - centre` turned by `-yaw`.
    pub yaw: f32,
    /// Ground: metres.
    pub tolerance: f32,
    /// Ground: the grid, two by two at least.
    pub columns: u32,
    pub rows: u32,
    /// Ground: `columns * rows`, row by row from local -z, each row from local -x.
    pub heights: Vec<f32>,
}

/// What replaces a triangle: points as weights of its three corners (a corner that stays is the
/// exact unit weight) and triangles over the points with the triangle's winding.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClipResult {
    pub weights: Vec<[f32; 3]>,
    pub triangles: Vec<[u8; 3]>,
}

#[derive(Clone, Copy, Default)]
struct Point {
    x: f32,
    y: f32,
    z: f32,
    w: [f32; 3],
}

#[derive(Clone, Copy)]
struct Polygon {
    count: usize,
    p: [Point; MAX_POLYGON],
}

impl Default for Polygon {
    fn default() -> Self {
        Self { count: 0, p: [Point::default(); MAX_POLYGON] }
    }
}

fn lerp(a: &Point, b: &Point, t: f32) -> Point {
    let mut m =
        Point { x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t, z: a.z + (b.z - a.z) * t, w: [0.0; 3] };
    for k in 0..3 {
        m.w[k] = a.w[k] + (b.w[k] - a.w[k]) * t;
    }
    m
}

fn push(poly: &mut Polygon, p: Point) -> bool {
    if poly.count == MAX_POLYGON {
        return false;
    }
    poly.p[poly.count] = p;
    poly.count += 1;
    true
}

/// Splits `poly` by the line where `bound + sign * coordinate` is zero: the part on the side where
/// it is not negative (within [`ON_LINE`]) stays in `poly`, the rest goes to `outside`.
fn split(poly: &mut Polygon, along_z: bool, sign: f32, bound: f32, outside: &mut Polygon) -> bool {
    let mut inside = Polygon::default();
    outside.count = 0;
    for i in 0..poly.count {
        let a = poly.p[i];
        let b = poly.p[(i + 1) % poly.count];
        let da = bound + sign * if along_z { a.z } else { a.x };
        let db = bound + sign * if along_z { b.z } else { b.x };
        let a_in = da >= -ON_LINE;
        let b_in = db >= -ON_LINE;
        if !push(if a_in { &mut inside } else { &mut *outside }, a) {
            return false;
        }
        if a_in != b_in {
            let t = (da / (da - db)).clamp(0.0, 1.0);
            let m = lerp(&a, &b, t);
            if !push(&mut inside, m) || !push(outside, m) {
                return false;
            }
        }
    }
    *poly = inside;
    true
}

fn area(poly: &Polygon) -> f32 {
    let (mut nx, mut ny, mut nz) = (0.0f32, 0.0f32, 0.0f32);
    let o = poly.p[0];
    for i in 1..poly.count.saturating_sub(1) {
        let (ax, ay, az) = (poly.p[i].x - o.x, poly.p[i].y - o.y, poly.p[i].z - o.z);
        let (bx, by, bz) = (poly.p[i + 1].x - o.x, poly.p[i + 1].y - o.y, poly.p[i + 1].z - o.z);
        nx += ay * bz - az * by;
        ny += az * bx - ax * bz;
        nz += ax * by - ay * bx;
    }
    0.5 * (nx * nx + ny * ny + nz * nz).sqrt()
}

/// The old ground's height at a local point: bilinear over the grid, clamped to its border.
fn ground(r: &Region, x: f32, z: f32) -> f32 {
    let (cols, rows) = (r.columns as usize, r.rows as usize);
    let mut u = (x + r.half.x) / (2.0 * r.half.x) * (r.columns - 1) as f32;
    let mut v = (z + r.half.z) / (2.0 * r.half.z) * (r.rows - 1) as f32;
    u = if u < 0.0 {
        0.0
    } else if u > (r.columns - 1) as f32 {
        (r.columns - 1) as f32
    } else {
        u
    };
    v = if v < 0.0 {
        0.0
    } else if v > (r.rows - 1) as f32 {
        (r.rows - 1) as f32
    } else {
        v
    };
    let i = if (u as usize) < cols - 2 { u as usize } else { cols - 2 };
    let j = if (v as usize) < rows - 2 { v as usize } else { rows - 2 };
    let (fu, fv) = (u - i as f32, v - j as f32);
    let h = |c: usize, rr: usize| r.heights.get(rr * cols + c).copied().unwrap_or(0.0);
    (h(i, j) * (1.0 - fu) + h(i + 1, j) * fu) * (1.0 - fv)
        + (h(i, j + 1) * (1.0 - fu) + h(i + 1, j + 1) * fu) * fv
}

fn removed(r: &Region, poly: &Polygon) -> bool {
    for p in &poly.p[..poly.count] {
        let off = match r.kind {
            CutKind::Everything => p.y < r.centre.y - r.half.y || p.y > r.centre.y + r.half.y,
            CutKind::Ground => (p.y - ground(r, p.x, p.z)).abs() > r.tolerance,
        };
        if off {
            return false;
        }
    }
    true
}

fn point_of(out: &mut ClipResult, p: &Point) -> Option<u8> {
    if let Some(i) = out.weights.iter().position(|w| *w == p.w) {
        return u8::try_from(i).ok();
    }
    if out.weights.len() == MAX_POINTS {
        return None;
    }
    out.weights.push(p.w);
    u8::try_from(out.weights.len() - 1).ok()
}

/// The clip of one triangle: `None` keeps it whole and unchanged, `Some` replaces it by what the
/// result holds, which may be no triangle at all.
#[must_use]
pub fn clip_triangle(region: &Region, a: Vec3, b: Vec3, c: Vec3) -> Option<ClipResult> {
    let (hx, hz) = (region.half.x, region.half.z);
    let grid_ok = region.columns >= 2
        && region.rows >= 2
        && region.heights.len() >= region.columns as usize * region.rows as usize;
    if !(hx > 0.0 && hz > 0.0) || (region.kind == CutKind::Ground && !grid_ok) {
        return None;
    }

    // local coordinates: p - centre turned by -yaw about +Y
    let (cs, sn) = (region.yaw.cos(), region.yaw.sin());
    let corners = [a, b, c];
    let mut poly = Polygon { count: 3, ..Polygon::default() };
    for (i, q) in corners.iter().enumerate() {
        let (dx, dz) = (q.x - region.centre.x, q.z - region.centre.z);
        let p = &mut poly.p[i];
        p.x = dx * cs - dz * sn;
        p.y = q.y;
        p.z = dx * sn + dz * cs;
        p.w = [0.0; 3];
        p.w[i] = 1.0;
    }

    let min_x = poly.p[0].x.min(poly.p[1].x.min(poly.p[2].x));
    let max_x = poly.p[0].x.max(poly.p[1].x.max(poly.p[2].x));
    let min_z = poly.p[0].z.min(poly.p[1].z.min(poly.p[2].z));
    let max_z = poly.p[0].z.max(poly.p[1].z.max(poly.p[2].z));
    if max_x < -hx || min_x > hx || max_z < -hz || min_z > hz {
        return None;
    }

    // x = -hx, x = +hx, z = -hz, z = +hz: what lies beyond each line is kept, `poly` ends inside
    let mut kept = [Polygon::default(); 4];
    let [k0, k1, k2, k3] = &mut kept;
    if !split(&mut poly, false, 1.0, hx, k0)
        || !split(&mut poly, false, -1.0, hx, k1)
        || !split(&mut poly, true, 1.0, hz, k2)
        || !split(&mut poly, true, -1.0, hz, k3)
    {
        return None;
    }
    if poly.count < 3 || area(&poly) < MIN_AREA || !removed(region, &poly) {
        return None;
    }

    let mut out = ClipResult::default();
    for k in &kept {
        if k.count < 3 || area(k) < MIN_AREA {
            continue;
        }
        for i in 1..k.count - 1 {
            if out.triangles.len() == MAX_TRIANGLES {
                return None;
            }
            let ids = [
                point_of(&mut out, &k.p[0])?,
                point_of(&mut out, &k.p[i])?,
                point_of(&mut out, &k.p[i + 1])?,
            ];
            out.triangles.push(ids);
        }
    }
    Some(out)
}

/// A corner of a triangle of a soup: its position and texture coordinate.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Corner {
    pub pos: Vec3,
    pub uv: Vec2,
}

/// A triangle that stays: the index of the triangle of the soup it is (a part of) and its corners.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Piece {
    pub source: usize,
    pub corners: [Corner; 3],
    /// It is a part of its triangle, not the triangle as it was.
    pub cut: bool,
}

fn mix(c: &[Corner; 3], w: [f32; 3]) -> Corner {
    // a unit weight is the corner itself, bit for bit
    if let Some(k) = w.iter().position(|x| *x == 1.0)
        && w.iter().filter(|x| **x == 0.0).count() == 2
    {
        return c[k];
    }
    Corner {
        pos: c[0].pos * w[0] + c[1].pos * w[1] + c[2].pos * w[2],
        uv: c[0].uv * w[0] + c[1].uv * w[1] + c[2].uv * w[2],
    }
}

/// Every triangle of a soup through every region in order, each region to what the earlier ones
/// left: the triangles that stay, in the soup's order, every attribute interpolated linearly.
#[must_use]
pub fn clip_mesh(regions: &[Region], tris: &[[Corner; 3]]) -> Vec<Piece> {
    let mut out = Vec::with_capacity(tris.len());
    let mut work: Vec<([Corner; 3], bool)> = Vec::new();
    let mut next: Vec<([Corner; 3], bool)> = Vec::new();
    for (source, t) in tris.iter().enumerate() {
        work.clear();
        work.push((*t, false));
        for r in regions {
            next.clear();
            for (c, cut) in &work {
                match clip_triangle(r, c[0].pos, c[1].pos, c[2].pos) {
                    None => next.push((*c, *cut)),
                    Some(res) => {
                        for tri in &res.triangles {
                            let w = |k: usize| res.weights[usize::from(tri[k])];
                            next.push(([mix(c, w(0)), mix(c, w(1)), mix(c, w(2))], true));
                        }
                    }
                }
            }
            std::mem::swap(&mut work, &mut next);
        }
        out.extend(work.iter().map(|(corners, cut)| Piece { source, corners: *corners, cut: *cut }));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game's test region (`tools/qa/cuts/cut_clip_test.cpp`): 20 x 8 m turned a quarter about
    /// +Y, so it spans |x| <= 4, |z| <= 10 in the world; the old ground is flat at 0 and the band
    /// is half a metre.
    fn ground_region() -> Region {
        Region {
            kind: CutKind::Ground,
            centre: Vec3::ZERO,
            half: Vec3::new(10.0, 5.0, 4.0),
            yaw: 1.570_796_4,
            tolerance: 0.5,
            columns: 2,
            rows: 2,
            heights: vec![0.0; 4],
        }
    }

    fn place(t: &[Vec3; 3], w: [f32; 3]) -> Vec3 {
        t[0] * w[0] + t[1] * w[1] + t[2] * w[2]
    }

    fn area_y(p: Vec3, q: Vec3, r: Vec3) -> f32 {
        0.5 * ((q.z - p.z) * (r.x - p.x) - (q.x - p.x) * (r.z - p.z))
    }

    #[test]
    fn straddling_the_edge_the_part_beyond_stays_with_its_winding() {
        let t = [Vec3::new(2.0, 0.0, -2.0), Vec3::new(8.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 2.0)];
        let out = clip_triangle(&ground_region(), t[0], t[1], t[2]).expect("replaced");
        assert_eq!(out.triangles.len(), 1);
        let p: Vec<Vec3> = out.triangles[0].iter().map(|i| place(&t, out.weights[usize::from(*i)])).collect();
        assert!(p.iter().all(|q| q.x >= 4.0 - 1e-5), "{p:?}");
        let a = area_y(p[0], p[1], p[2]);
        assert!((a.abs() - 16.0 / 3.0).abs() < 1e-4, "{a}");
        assert_eq!(a > 0.0, area_y(t[0], t[1], t[2]) > 0.0);
    }

    #[test]
    fn inside_and_on_the_old_ground_it_is_gone() {
        let y = 0.2;
        let out = clip_triangle(
            &ground_region(),
            Vec3::new(-1.0, y, -1.0),
            Vec3::new(1.0, y, -1.0),
            Vec3::new(-1.0, y, 1.0),
        )
        .expect("replaced");
        assert!(out.triangles.is_empty());
    }

    #[test]
    fn outside_the_rectangle_it_is_kept_whole() {
        assert!(
            clip_triangle(
                &ground_region(),
                Vec3::new(20.0, 0.0, 20.0),
                Vec3::new(22.0, 0.0, 20.0),
                Vec3::new(20.0, 0.0, 22.0)
            )
            .is_none()
        );
    }

    #[test]
    fn off_the_band_it_is_kept_whole() {
        let y = 2.0;
        assert!(
            clip_triangle(
                &ground_region(),
                Vec3::new(-1.0, y, -1.0),
                Vec3::new(1.0, y, -1.0),
                Vec3::new(-1.0, y, 1.0)
            )
            .is_none()
        );
    }

    #[test]
    fn a_corner_of_the_rectangle_inside_the_triangle_is_a_mix_of_all_three() {
        // an unturned 2 x 2 m rectangle; a big triangle round its corner (1, 1)
        let r = Region { half: Vec3::new(1.0, 5.0, 1.0), yaw: 0.0, ..ground_region() };
        let t = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(4.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 4.0)];
        let out = clip_triangle(&r, t[0], t[1], t[2]).expect("replaced");
        let corner = out
            .weights
            .iter()
            .find(|w| w.iter().all(|x| *x > 0.0 && *x < 1.0))
            .copied()
            .expect("a point of all three corners");
        let at = place(&t, corner);
        assert!((at.x - 1.0).abs() < 1e-5 && (at.z - 1.0).abs() < 1e-5, "{at:?}");
        // what stays is the triangle less the square's quarter [0,1] x [0,1]
        let kept: f32 = out
            .triangles
            .iter()
            .map(|tri| {
                let p: Vec<Vec3> = tri.iter().map(|i| place(&t, out.weights[usize::from(*i)])).collect();
                area_y(p[0], p[1], p[2]).abs()
            })
            .sum();
        assert!((kept - (8.0 - 1.0)).abs() < 1e-4, "{kept}");
        // and every triangle keeps the winding of the original
        let sign = area_y(t[0], t[1], t[2]) > 0.0;
        for tri in &out.triangles {
            let p: Vec<Vec3> = tri.iter().map(|i| place(&t, out.weights[usize::from(*i)])).collect();
            assert_eq!(area_y(p[0], p[1], p[2]) > 0.0, sign);
        }
    }

    #[test]
    fn a_yaw_of_thirty_degrees_turns_the_rectangle() {
        let yaw = 30f32.to_radians();
        let r = Region { half: Vec3::new(2.0, 5.0, 1.0), yaw, ..ground_region() };
        // a point local (1.5, 0): world d.x = x' cos + z' sin, d.z = -x' sin + z' cos
        let world =
            |x: f32, z: f32| Vec3::new(x * yaw.cos() + z * yaw.sin(), 0.0, -x * yaw.sin() + z * yaw.cos());
        let small = |c: Vec3| {
            [
                c + Vec3::new(-0.05, 0.0, -0.05),
                c + Vec3::new(0.05, 0.0, -0.05),
                c + Vec3::new(-0.05, 0.0, 0.05),
            ]
        };
        // inside the turned rectangle: removed
        let t = small(world(1.5, 0.0));
        assert!(clip_triangle(&r, t[0], t[1], t[2]).is_some_and(|o| o.triangles.is_empty()));
        // a world point an unturned box of the same size holds lies beyond the turned one's z edge
        let beyond = small(Vec3::new(1.85, 0.0, 0.85));
        assert!(beyond.iter().all(|p| p.x.abs() <= 2.0 && p.z.abs() <= 1.0));
        assert!(clip_triangle(&r, beyond[0], beyond[1], beyond[2]).is_none());
    }

    #[test]
    fn a_soup_keeps_its_order_and_interpolates_the_coordinates() {
        let c = |x: f32, z: f32| Corner { pos: Vec3::new(x, 0.0, z), uv: Vec2::new(x / 10.0, z / 10.0) };
        let tris = [
            [c(20.0, 20.0), c(22.0, 20.0), c(20.0, 22.0)],
            [c(2.0, -2.0), c(8.0, 0.0), c(2.0, 2.0)],
            [c(-1.0, -1.0), c(1.0, -1.0), c(-1.0, 1.0)],
        ];
        let out = clip_mesh(&[ground_region()], &tris);
        assert_eq!(out.len(), 2);
        assert_eq!((out[0].source, out[0].cut), (0, false));
        assert_eq!(out[0].corners, tris[0]);
        assert_eq!((out[1].source, out[1].cut), (1, true));
        for k in out[1].corners {
            assert!((k.uv.x - k.pos.x / 10.0).abs() < 1e-6 && (k.uv.y - k.pos.z / 10.0).abs() < 1e-6);
        }
    }
}
