//! The engine's arithmetic the bulk tools depend on, written the way the engine writes it so a
//! result comes out float for float: vector normalizing, `normalize_safe`, `mknormal`,
//! planes, `snapto`, `fsimilar`, frustum polygon clipping, the ray-triangle tests,
//! the box-triangle test and the 4x3 matrices.

use glam::Vec3;
pub use gz_fmt_level::i_floor;

/// `EPS_S`, `EPS`, `EPS_L` of `vector.h`.
pub const EPS_S: f32 = 0.000_000_1;
pub const EPS: f32 = 0.000_01;
pub const EPS_L: f32 = 0.001;

/// `x*x + y*y + z*z`, left to right.
#[inline]
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// The magnitude of a vector.
#[inline]
#[must_use]
pub fn magnitude(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// Normalizes a vector: no guard.
#[inline]
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let mag = (1.0 / (v.x * v.x + v.y * v.y + v.z * v.z)).sqrt();
    Vec3::new(v.x * mag, v.y * mag, v.z * mag)
}

/// Normalizes a vector safely: a vector too short stays as it is.
#[inline]
#[must_use]
pub fn normalize_safe(v: Vec3) -> Vec3 {
    let m = v.x * v.x + v.y * v.y + v.z * v.z;
    if m > f32::MIN_POSITIVE {
        let s = (1.0 / m).sqrt();
        Vec3::new(v.x * s, v.y * s, v.z * s)
    } else {
        v
    }
}

/// The cross product.
#[inline]
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// `(p1 - p0) x (p2 - p1)`, `normalize_safe`d.
#[inline]
#[must_use]
pub fn mknormal(p0: Vec3, p1: Vec3, p2: Vec3) -> Vec3 {
    normalize_safe(cross(p1 - p0, p2 - p1))
}

/// `a + v * s`.
#[inline]
#[must_use]
pub fn mad(a: Vec3, v: Vec3, s: f32) -> Vec3 {
    Vec3::new(a.x + v.x * s, a.y + v.y * s, a.z + v.z * s)
}

/// Whether two vectors are similar within `eps`.
#[inline]
#[must_use]
pub fn similar(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

/// `fsimilar(a, b, cmp)`.
#[inline]
#[must_use]
pub fn fsimilar(a: f32, b: f32, cmp: f32) -> bool {
    (a - b).abs() < cmp
}

/// `distance_to_sqr`.
#[inline]
#[must_use]
pub fn dist_sqr(a: Vec3, b: Vec3) -> f32 {
    let d = Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z);
    d.x * d.x + d.y * d.y + d.z * d.z
}

/// `snapto`.
#[inline]
#[must_use]
pub fn snapto(value: f32, snap: f32) -> f32 {
    if snap <= 0.0 {
        return value;
    }
    i_floor((value + snap * 0.5) / snap) as f32 * snap
}

/// `deg2rad`.
#[inline]
#[must_use]
pub fn deg2rad(v: f32) -> f32 {
    v * std::f32::consts::PI / 180.0
}

/// The unit normal and `d`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Plane {
    pub n: Vec3,
    pub d: f32,
}

impl Plane {
    /// `build(point, normal)`: the normal normalised (no guard), `d = -n . p`.
    #[must_use]
    pub fn build(p: Vec3, n: Vec3) -> Self {
        let n = normalize(n);
        Self { n, d: -dot(n, p) }
    }

    #[inline]
    #[must_use]
    pub fn classify(&self, v: Vec3) -> f32 {
        dot(self.n, v) + self.d
    }

    /// The point along `dir` on the plane; `None` when the ray runs along
    /// it. The engine returns the point also for a hit behind the start, with a false flag: `.1` says
    /// whether the hit is ahead.
    #[must_use]
    pub fn intersect_ray_point(&self, p: Vec3, dir: Vec3) -> Option<(Vec3, bool)> {
        let numer = self.classify(p);
        let denom = dot(self.n, dir);
        if denom.abs() < EPS_S {
            return None;
        }
        let dist = -(numer / denom);
        Some((mad(p, dir, dist), dist > 0.0 || dist.abs() < EPS_S))
    }
}

/// The ray-triangle test (one body): Moller-Trumbore with the engine's epsilons; the
/// range is not checked against anything.
#[must_use]
pub fn ray_tri(c: Vec3, d: Vec3, p: &[Vec3; 3], cull: bool) -> Option<f32> {
    let edge1 = p[1] - p[0];
    let edge2 = p[2] - p[0];
    let pvec = cross(d, edge2);
    let det = dot(edge1, pvec);
    if cull {
        if det < EPS {
            return None;
        }
        let tvec = c - p[0];
        let u = dot(tvec, pvec);
        if u < 0.0 || u > det {
            return None;
        }
        let qvec = cross(tvec, edge1);
        let v = dot(d, qvec);
        if v < 0.0 || u + v > det {
            return None;
        }
        let range = dot(edge2, qvec);
        let inv_det = 1.0 / det;
        Some(range * inv_det)
    } else {
        if det > -EPS && det < EPS {
            return None;
        }
        let inv_det = 1.0 / det;
        let tvec = c - p[0];
        let u = dot(tvec, pvec) * inv_det;
        if u < 0.0 || u > 1.0 {
            return None;
        }
        let qvec = cross(tvec, edge1);
        let v = dot(d, qvec) * inv_det;
        if v < 0.0 || u + v > 1.0 {
            return None;
        }
        Some(dot(edge2, qvec) * inv_det)
    }
}

/// The range to the triangle's plane, and whether the hit is inside the triangle.
/// `None` when the ray runs along the plane.
#[must_use]
pub fn ray_tri2(c: Vec3, d: Vec3, p: &[Vec3; 3]) -> (bool, f32) {
    let edge1 = p[1] - p[0];
    let edge2 = p[2] - p[0];
    let pvec = cross(d, edge2);
    let det = dot(edge1, pvec);
    if det.abs() < EPS_S {
        return (false, -1.0);
    }
    let inv_det = 1.0 / det;
    let tvec = c - p[0];
    let u = dot(tvec, pvec) * inv_det;
    let qvec = cross(tvec, edge1);
    let range = dot(edge2, qvec) * inv_det;
    if u < 0.0 || u > 1.0 {
        return (false, range);
    }
    let v = dot(d, qvec) * inv_det;
    if v < 0.0 || u + v > 1.0 {
        return (false, range);
    }
    (true, range)
}

fn plane_box_overlap(normal: Vec3, d: f32, maxbox: Vec3) -> bool {
    let mut vmin = Vec3::ZERO;
    let mut vmax = Vec3::ZERO;
    for q in 0..3 {
        if normal[q] > 0.0 {
            vmin[q] = -maxbox[q];
            vmax[q] = maxbox[q];
        } else {
            vmin[q] = maxbox[q];
            vmax[q] = -maxbox[q];
        }
    }
    if dot(normal, vmin) + d > 0.0 {
        return false;
    }
    dot(normal, vmax) + d >= 0.0
}

fn min_max3(a: f32, b: f32, c: f32) -> (f32, f32) {
    let (mut mn, mut mx) = (a, a);
    if b < mn {
        mn = b;
    }
    if b > mx {
        mx = b;
    }
    if c < mn {
        mn = c;
    }
    if c > mx {
        mx = c;
    }
    (mn, mx)
}

/// Whether a triangle meets a box given by its centre
/// and extents; `class3` adds the nine edge axes.
#[must_use]
pub fn tri_box(center: Vec3, extents: Vec3, t: &[Vec3; 3], class3: bool) -> bool {
    let v0x = t[0].x - center.x;
    let v1x = t[1].x - center.x;
    let v2x = t[2].x - center.x;
    let (mn, mx) = min_max3(v0x, v1x, v2x);
    if mn > extents.x || mx < -extents.x {
        return false;
    }
    let v0y = t[0].y - center.y;
    let v1y = t[1].y - center.y;
    let v2y = t[2].y - center.y;
    let (mn, mx) = min_max3(v0y, v1y, v2y);
    if mn > extents.y || mx < -extents.y {
        return false;
    }
    let v0z = t[0].z - center.z;
    let v1z = t[1].z - center.z;
    let v2z = t[2].z - center.z;
    let (mn, mx) = min_max3(v0z, v1z, v2z);
    if mn > extents.z || mx < -extents.z {
        return false;
    }
    let v0 = Vec3::new(v0x, v0y, v0z);
    let v1 = Vec3::new(v1x, v1y, v1z);
    let v2 = Vec3::new(v2x, v2y, v2z);
    let e0 = v1 - v0;
    let e1 = v2 - v1;
    let normal = cross(e0, e1);
    let d = -dot(normal, v0);
    if !plane_box_overlap(normal, d, extents) {
        return false;
    }
    if !class3 {
        return true;
    }
    let ex = extents;
    let test = |mn: f32, mx: f32, rad: f32| -> bool {
        let (mn, mx) = if mn > mx { (mx, mn) } else { (mn, mx) };
        !(mn > rad || mx < -rad)
    };
    // X01 / Y02 / Z12 and their kin, as the macros spell them
    let x01 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(a * v0.y - b * v0.z, a * v2.y - b * v2.z, fa * ex.y + fb * ex.z)
    };
    let x2 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(a * v0.y - b * v0.z, a * v1.y - b * v1.z, fa * ex.y + fb * ex.z)
    };
    let y02 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(b * v0.z - a * v0.x, b * v2.z - a * v2.x, fa * ex.x + fb * ex.z)
    };
    let y1 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(b * v0.z - a * v0.x, b * v1.z - a * v1.x, fa * ex.x + fb * ex.z)
    };
    let z12 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(a * v1.x - b * v1.y, a * v2.x - b * v2.y, fa * ex.x + fb * ex.y)
    };
    let z0 = |a: f32, b: f32, fa: f32, fb: f32| {
        test(a * v0.x - b * v0.y, a * v1.x - b * v1.y, fa * ex.x + fb * ex.y)
    };
    let (fey0, fez0, fex0) = (e0.y.abs(), e0.z.abs(), e0.x.abs());
    if !x01(e0.z, e0.y, fez0, fey0) || !y02(e0.z, e0.x, fez0, fex0) || !z12(e0.y, e0.x, fey0, fex0) {
        return false;
    }
    let (fey1, fez1, fex1) = (e1.y.abs(), e1.z.abs(), e1.x.abs());
    if !x01(e1.z, e1.y, fez1, fey1) || !y02(e1.z, e1.x, fez1, fex1) || !z0(e1.y, e1.x, fey1, fex1) {
        return false;
    }
    let e2 = t[0] - t[2];
    let (fey2, fez2, fex2) = (e2.y.abs(), e2.z.abs(), e2.x.abs());
    x2(e2.z, e2.y, fez2, fey2) && y1(e2.z, e2.x, fez2, fex2) && z12(e2.y, e2.x, fey2, fex2)
}

/// A frustum's planes: unit normals; inside is
/// where `classify` is negative.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Frustum {
    pub planes: Vec<Plane>,
}

impl Frustum {
    /// Each plane scaled to a unit normal.
    #[must_use]
    pub fn from_planes(planes: &[Plane]) -> Self {
        Self { planes: planes.iter().map(|p| unit(p.n, p.d)).collect() }
    }

    /// `CreateFromMatrix(M, FRUSTUM_P_LRTB)`: left, right, top, bottom.
    #[must_use]
    pub fn from_matrix_lrtb(m: &M43) -> Self {
        let c = |r: usize, k: usize| m.0[r][k];
        let mk = |sign: f32, k: usize| {
            unit(
                Vec3::new(
                    -(c(0, 3) + sign * c(0, k)),
                    -(c(1, 3) + sign * c(1, k)),
                    -(c(2, 3) + sign * c(2, k)),
                ),
                -(c(3, 3) + sign * c(3, k)),
            )
        };
        // left: _14 + _11; right: _14 - _11; top: _14 - _12; bottom: _14 + _12
        Self { planes: vec![mk(1.0, 0), mk(-1.0, 0), mk(-1.0, 1), mk(1.0, 1)] }
    }

    /// The polygon cut to the inside of every plane; `None` once fewer than three
    /// points are left.
    #[must_use]
    pub fn clip_poly(&self, poly: &[Vec3]) -> Option<Vec<Vec3>> {
        let mut src: Vec<Vec3> = poly.to_vec();
        let mut dst: Vec<Vec3> = Vec::with_capacity(poly.len() + 8);
        for p in &self.planes {
            dst.clear();
            let cls: Vec<f32> =
                src.iter().map(|v| p.classify(*v)).chain(src.first().map(|v| p.classify(*v))).collect();
            if let Some(first) = src.first().copied() {
                src.push(first);
            }
            for j in 0..src.len().saturating_sub(1) {
                let (a, b) = (src[j], src[j + 1]);
                if similar(a, b, EPS_S) {
                    continue;
                }
                let (ca, cb) = (cls[j], cls[j + 1]);
                let cut = |dst: &mut Vec<Vec3>| {
                    let d = b - a;
                    let denum = dot(p.n, d);
                    if denum != 0.0 {
                        let t = -ca / denum;
                        dst.push(mad(a, d, t));
                    }
                };
                if ca.is_sign_negative() {
                    dst.push(a);
                    if cb.is_sign_positive() {
                        cut(&mut dst);
                    }
                } else if cb.is_sign_negative() {
                    cut(&mut dst);
                }
            }
            if dst.len() < 3 {
                return None;
            }
            std::mem::swap(&mut src, &mut dst);
        }
        Some(src)
    }
}

/// A plane scaled to a unit normal (`denom = 1 / |n|`).
fn unit(n: Vec3, d: f32) -> Plane {
    let denom = 1.0 / magnitude(n);
    Plane { n: Vec3::new(n.x * denom, n.y * denom, n.z * denom), d: d * denom }
}

/// Rows `i`, `j`, `k`, `c` (`m[r][k]` is `_{r+1}{k+1}`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct M43(pub [[f32; 4]; 4]);

impl M43 {
    pub const IDENTITY: M43 =
        M43([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    /// `build_camera(from, at, up)`.
    #[must_use]
    pub fn camera(from: Vec3, at: Vec3, world_up: Vec3) -> Self {
        let view = normalize(at - from);
        let dp = dot(world_up, view);
        let up = normalize(Vec3::new(view.x * -dp, view.y * -dp, view.z * -dp) + world_up);
        let right = cross(up, view);
        M43([
            [right.x, up.x, view.x, 0.0],
            [right.y, up.y, view.y, 0.0],
            [right.z, up.z, view.z, 0.0],
            [-dot(from, right), -dot(from, up), -dot(from, view), 1.0],
        ])
    }

    /// Rotation about Z.
    #[must_use]
    pub fn rotate_z(a: f32) -> Self {
        let (s, c) = (a.sin(), a.cos());
        M43([[c, s, 0.0, 0.0], [-s, c, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// Rotation about X.
    #[must_use]
    pub fn rotate_x(a: f32) -> Self {
        let (s, c) = (a.sin(), a.cos());
        M43([[1.0, 0.0, 0.0, 0.0], [0.0, c, s, 0.0], [0.0, -s, c, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// `scale`.
    #[must_use]
    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        M43([[x, 0.0, 0.0, 0.0], [0.0, y, 0.0, 0.0], [0.0, 0.0, z, 0.0], [0.0, 0.0, 0.0, 1.0]])
    }

    /// `mul_43(A, B)`.
    #[must_use]
    pub fn mul_43(a: &M43, b: &M43) -> Self {
        let (a, b) = (&a.0, &b.0);
        let mut m = [[0.0f32; 4]; 4];
        for r in 0..3 {
            for k in 0..3 {
                m[r][k] = a[0][k] * b[r][0] + a[1][k] * b[r][1] + a[2][k] * b[r][2];
            }
        }
        for k in 0..3 {
            m[3][k] = a[0][k] * b[3][0] + a[1][k] * b[3][1] + a[2][k] * b[3][2] + a[3][k];
        }
        m[3][3] = 1.0;
        M43(m)
    }

    /// `mulA_43(A)`: `this = A * this`.
    #[must_use]
    pub fn mul_a_43(&self, a: &M43) -> Self {
        M43::mul_43(a, self)
    }

    #[must_use]
    pub fn transform_tiny(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0] + m[3][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1] + m[3][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2] + m[3][2],
        )
    }

    #[must_use]
    pub fn transform_dir(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2],
        )
    }
}

/// Where a ray meets a box and whether its origin is inside (the point is the
/// origin then); `None` for a miss.
#[must_use]
pub fn box_pick2(min: Vec3, max: Vec3, start: Vec3, dir: Vec3) -> Option<(Vec3, bool)> {
    crate::math::ray_box(start, dir, &gz_fmt_level::Aabb::new(min, max))
        .map(|(t, inside)| (start + dir * t, inside))
}

/// The CRT `rand` stream (`srand` / `rand`): the MSVC recurrence.
#[derive(Clone, Copy, Debug)]
pub struct CrtRand {
    hold: u32,
}

impl CrtRand {
    #[must_use]
    pub fn seeded(seed: u32) -> Self {
        Self { hold: seed }
    }

    /// `rand()`: 0..=32767.
    pub fn rand(&mut self) -> u32 {
        self.hold = self.hold.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.hold >> 16) & 0x7fff
    }

    /// `_Rng_from_urng<ptrdiff_t, legacy_rand_urbg>(index)`: MSVC's draw of `[0, index)` from a
    /// 15-bit generator, rejection included.
    pub fn below(&mut self, index: u64) -> u64 {
        if index == 0 {
            return 0;
        }
        loop {
            let (mut ret, mut mask) = (0u64, 0u64);
            while mask < index - 1 {
                ret = (ret << 14) << 1;
                ret |= u64::from(self.rand());
                mask = (mask << 14) << 1;
                mask |= 0x7fff;
            }
            if ret / index < mask / index || mask % index == index - 1 {
                return ret % index;
            }
        }
    }

    /// `std::shuffle(first, last, legacy_rand_urbg{})` as the MSVC library shuffles.
    pub fn shuffle<T>(&mut self, items: &mut [T]) {
        for target in 1..items.len() {
            let off = self.below(target as u64 + 1) as usize;
            if off != target {
                items.swap(target, off);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_crt_stream_and_its_shuffle() {
        let mut r = CrtRand::seeded(1);
        assert_eq!([r.rand(), r.rand(), r.rand()], [41, 18467, 6334]);
        let mut a = [0u8, 1, 2, 3];
        CrtRand::seeded(7).shuffle(&mut a);
        let mut sorted = a;
        sorted.sort_unstable();
        assert_eq!(sorted, [0, 1, 2, 3]);
    }

    #[test]
    fn a_square_clipped_by_a_slab() {
        // inside is x <= 1 and x >= -1
        let f = Frustum::from_planes(&[Plane { n: Vec3::X, d: -1.0 }, Plane { n: Vec3::NEG_X, d: -1.0 }]);
        let poly = [
            Vec3::new(-3.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 0.0),
            Vec3::new(3.0, 0.0, 1.0),
            Vec3::new(-3.0, 0.0, 1.0),
        ];
        let out = f.clip_poly(&poly).unwrap();
        assert_eq!(out.len(), 4);
        assert!(out.iter().all(|p| p.x.abs() <= 1.0 + 1e-6));
        assert!(
            f.clip_poly(&[Vec3::new(5.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 0.0), Vec3::new(6.0, 0.0, 1.0)])
                .is_none()
        );
    }

    #[test]
    fn a_triangle_through_a_box() {
        let t = [Vec3::new(-5.0, 0.0, -5.0), Vec3::new(5.0, 0.0, -5.0), Vec3::new(0.0, 0.0, 5.0)];
        assert!(tri_box(Vec3::ZERO, Vec3::splat(0.5), &t, true));
        assert!(!tri_box(Vec3::new(0.0, 2.0, 0.0), Vec3::splat(0.5), &t, true));
    }

    #[test]
    fn rays_meet_triangles() {
        let t = [Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 1.0), Vec3::new(1.0, 0.0, 0.0)];
        let r = ray_tri(Vec3::new(0.2, 5.0, 0.2), Vec3::NEG_Y, &t, false);
        assert!((r.unwrap() - 5.0).abs() < 1e-5);
        let (inside, range) = ray_tri2(Vec3::new(3.0, 5.0, 3.0), Vec3::NEG_Y, &t);
        assert!(!inside && (range - 5.0).abs() < 1e-5);
    }
}
