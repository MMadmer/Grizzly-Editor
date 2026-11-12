//! The engine's arithmetic the compiler's results depend on (`xrCore`'s `Fvector`, `Fmatrix`,
//! `Fbox2`, `CRandom`, `exact_normalize`, `u8_clr`), in its order of operations.

use glam::{Vec2, Vec3};

pub use gz_fmt_level::{Aabb, EPS, EPS_L, EPS_S, i_ceil, i_floor};

/// `PI`, `PI_MUL_2`, `PI_DIV_4`: the engine's float literals round to these.
pub const PI: f32 = std::f32::consts::PI;
pub const PI_MUL_2: f32 = std::f32::consts::TAU;
pub const PI_DIV_4: f32 = std::f32::consts::FRAC_PI_4;

/// `deg2rad`: `val * PI / 180` in float.
#[must_use]
pub fn deg2rad(val: f32) -> f32 {
    val * PI / 180.0
}

/// `_cos(deg2rad(a))`.
#[must_use]
pub fn cos_deg(a: f32) -> f32 {
    deg2rad(a).cos()
}

/// `u8_clr`: `0..1` to a byte, floored.
#[must_use]
pub fn u8_clr(a: f32) -> u8 {
    i_floor(a * 255.0).clamp(0, 255) as u8
}

/// `color_rgba`: `0xAARRGGBB`.
#[must_use]
pub fn color_rgba(r: u8, g: u8, b: u8, a: u8) -> u32 {
    (u32::from(a) << 24) | (u32::from(r) << 16) | (u32::from(g) << 8) | u32::from(b)
}

#[must_use]
pub fn color_get_r(c: u32) -> u32 {
    (c >> 16) & 0xff
}

#[must_use]
pub fn color_get_g(c: u32) -> u32 {
    (c >> 8) & 0xff
}

#[must_use]
pub fn color_get_b(c: u32) -> u32 {
    c & 0xff
}

#[must_use]
pub fn color_get_a(c: u32) -> u32 {
    c >> 24
}

/// `subst_alpha`.
#[must_use]
pub fn subst_alpha(c: u32, a: u8) -> u32 {
    (c & 0x00ff_ffff) | (u32::from(a) << 24)
}

/// `Fvector::dotproduct`, left to right.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// `Fvector::crossproduct`.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// `Fvector::magnitude`.
#[must_use]
pub fn magnitude(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// `Fvector::distance_to_sqr`.
#[must_use]
pub fn distance_sqr(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) + (a.z - b.z) * (a.z - b.z)
}

/// `Fvector::distance_to`.
#[must_use]
pub fn distance(a: Vec3, b: Vec3) -> f32 {
    distance_sqr(a, b).sqrt()
}

/// `Fvector2::distance_to`.
#[must_use]
pub fn distance2(a: Vec2, b: Vec2) -> f32 {
    ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y)).sqrt()
}

/// `Fvector::normalize`: scaled by `sqrt(1 / |v|^2)`.
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let mag = (1.0 / (v.x * v.x + v.y * v.y + v.z * v.z)).sqrt();
    Vec3::new(v.x * mag, v.y * mag, v.z * mag)
}

/// `Fvector::normalize_safe`: left as it is when its length is below the smallest float.
#[must_use]
pub fn normalize_safe(v: Vec3) -> Vec3 {
    let m = v.x * v.x + v.y * v.y + v.z * v.z;
    if m > f32::MIN_POSITIVE {
        let m = (1.0 / m).sqrt();
        Vec3::new(v.x * m, v.y * m, v.z * m)
    } else {
        v
    }
}

/// `exact_normalize`: in double, with a fallback for tiny vectors. `false` (and `(0, 1, 0)`) for
/// a zero vector.
#[must_use]
pub fn exact_normalize(v: Vec3) -> (Vec3, bool) {
    // the squares are summed in float and only then widened, as the C++ does
    let sqr = f64::from(v.x * v.x + v.y * v.y + v.z * v.z);
    if sqr > f64::from(1.192_092_9e-5_f32) {
        let l = 1.0 / sqr.sqrt();
        let out =
            Vec3::new((f64::from(v.x) * l) as f32, (f64::from(v.y) * l) as f32, (f64::from(v.z) * l) as f32);
        return (out, true);
    }
    let (mut a0, mut a1, mut a2) = (f64::from(v.x), f64::from(v.y), f64::from(v.z));
    let (aa0, aa1, aa2) = (a0.abs(), a1.abs(), a2.abs());
    let z_largest = |a0: f64, a1: f64, a2: f64| {
        let (a0, a1) = (a0 / aa2, a1 / aa2);
        let l = 1.0 / (a0 * a0 + a1 * a1 + 1.0).sqrt();
        Vec3::new((a0 * l) as f32, (a1 * l) as f32, l.copysign(a2) as f32)
    };
    if aa1 > aa0 {
        if aa2 > aa1 {
            return (z_largest(a0, a1, a2), true);
        }
        a0 /= aa1;
        a2 /= aa1;
        let l = 1.0 / (a0 * a0 + a2 * a2 + 1.0).sqrt();
        return (Vec3::new((a0 * l) as f32, l.copysign(a1) as f32, (a2 * l) as f32), true);
    }
    if aa2 > aa0 {
        return (z_largest(a0, a1, a2), true);
    }
    if aa0 <= 0.0 {
        return (Vec3::Y, false);
    }
    a1 /= aa0;
    a2 /= aa0;
    let l = 1.0 / (a1 * a1 + a2 * a2 + 1.0).sqrt();
    (Vec3::new(l.copysign(a0) as f32, (a1 * l) as f32, (a2 * l) as f32), true)
}

/// `Fvector::from_bary`.
#[must_use]
pub fn from_bary(a: Vec3, b: Vec3, c: Vec3, w: Vec3) -> Vec3 {
    Vec3::new(
        a.x * w.x + b.x * w.y + c.x * w.z,
        a.y * w.x + b.y * w.y + c.y * w.z,
        a.z * w.x + b.z * w.y + c.z * w.z,
    )
}

/// `Fvector::mad(p, d, m)`: `p + d * m`.
#[must_use]
pub fn mad(p: Vec3, d: Vec3, m: f32) -> Vec3 {
    Vec3::new(p.x + d.x * m, p.y + d.y * m, p.z + d.z * m)
}

/// `Fvector::mknormal`: the normal of a triangle, `normalize_safe`d.
#[must_use]
pub fn mknormal(p0: Vec3, p1: Vec3, p2: Vec3) -> Vec3 {
    normalize_safe(cross(p1 - p0, p2 - p1))
}

/// `Fvector::lerp`.
#[must_use]
pub fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let inv = 1.0 - t;
    Vec3::new(a.x * inv + b.x * t, a.y * inv + b.y * t, a.z * inv + b.z * t)
}

/// `Fvector::max`, component by component, the engine's `_max`.
#[must_use]
pub fn max3(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(max_f(a.x, b.x), max_f(a.y, b.y), max_f(a.z, b.z))
}

/// `_max`: `a > b ? a : b`.
#[must_use]
pub fn max_f(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// `_min`.
#[must_use]
pub fn min_f(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

/// `Fbox::getsphere`: the box's centre and the distance from it to `max`.
#[must_use]
pub fn sphere_of(b: &Aabb) -> (Vec3, f32) {
    let c = b.center();
    (c, distance(c, b.max))
}

/// `Fbox2`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub min: Vec2,
    pub max: Vec2,
}

impl Box2 {
    /// `invalidate`.
    pub const INVALID: Box2 = Box2 { min: Vec2::splat(f32::MAX), max: Vec2::splat(-f32::MAX) };

    /// The box of a triangle's three points, the way `CDeflector::Bounds` makes it.
    #[must_use]
    pub fn of_tri(uv: &[Vec2; 3]) -> Self {
        let mut b = Box2 { min: uv[0], max: uv[0] };
        b.modify(uv[1]);
        b.modify(uv[2]);
        b
    }

    pub fn modify(&mut self, p: Vec2) {
        self.min = Vec2::new(min_f(self.min.x, p.x), min_f(self.min.y, p.y));
        self.max = Vec2::new(max_f(self.max.x, p.x), max_f(self.max.y, p.y));
    }

    pub fn merge(&mut self, o: &Box2) {
        self.min = Vec2::new(min_f(self.min.x, o.min.x), min_f(self.min.y, o.min.y));
        self.max = Vec2::new(max_f(self.max.x, o.max.x), max_f(self.max.y, o.max.y));
    }
}

/// `Fmatrix`: rows `i`, `j`, `k`, `c` (`_11 .. _44`), points as rows multiplied from the left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Matrix {
    pub m: [[f32; 4]; 4],
}

impl Default for Matrix {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Matrix {
    pub const IDENTITY: Matrix = Matrix {
        m: [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]],
    };

    #[must_use]
    pub fn from_rows(m: [[f32; 4]; 4]) -> Self {
        Self { m }
    }

    /// `transform_tiny`.
    #[must_use]
    pub fn transform_tiny(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0] + m[3][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1] + m[3][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2] + m[3][2],
        )
    }

    /// `transform_dir`.
    #[must_use]
    pub fn transform_dir(&self, v: Vec3) -> Vec3 {
        let m = &self.m;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2],
        )
    }

    /// `translate_over(0, 0, 0)`: the translation row cleared.
    #[must_use]
    pub fn without_translation(&self) -> Self {
        let mut out = *self;
        out.m[3][0] = 0.0;
        out.m[3][1] = 0.0;
        out.m[3][2] = 0.0;
        out
    }

    /// `transpose`.
    #[must_use]
    pub fn transposed(&self) -> Self {
        let a = &self.m;
        let mut m = [[0.0f32; 4]; 4];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, v) in row.iter_mut().enumerate() {
                *v = a[j][i];
            }
        }
        Self { m }
    }

    /// `invert`: the 4x3 inverse.
    #[must_use]
    pub fn inverted_43(&self) -> Self {
        let a = &self.m;
        let (a11, a12, a13) = (a[0][0], a[0][1], a[0][2]);
        let (a21, a22, a23) = (a[1][0], a[1][1], a[1][2]);
        let (a31, a32, a33) = (a[2][0], a[2][1], a[2][2]);
        let (a41, a42, a43) = (a[3][0], a[3][1], a[3][2]);
        let det =
            a11 * (a22 * a33 - a23 * a32) - a12 * (a21 * a33 - a23 * a31) + a13 * (a21 * a32 - a22 * a31);
        let d = 1.0 / det;
        let m11 = d * (a22 * a33 - a23 * a32);
        let m12 = -d * (a12 * a33 - a13 * a32);
        let m13 = d * (a12 * a23 - a13 * a22);
        let m21 = -d * (a21 * a33 - a23 * a31);
        let m22 = d * (a11 * a33 - a13 * a31);
        let m23 = -d * (a11 * a23 - a13 * a21);
        let m31 = d * (a21 * a32 - a22 * a31);
        let m32 = -d * (a11 * a32 - a12 * a31);
        let m33 = d * (a11 * a22 - a12 * a21);
        let m41 = -(a41 * m11 + a42 * m21 + a43 * m31);
        let m42 = -(a41 * m12 + a42 * m22 + a43 * m32);
        let m43 = -(a41 * m13 + a42 * m23 + a43 * m33);
        Self { m: [[m11, m12, m13, 0.0], [m21, m22, m23, 0.0], [m31, m32, m33, 0.0], [m41, m42, m43, 1.0]] }
    }

    /// `build_camera`.
    #[must_use]
    pub fn camera(from: Vec3, at: Vec3, world_up: Vec3) -> Self {
        let view = normalize(at - from);
        let d = dot(world_up, view);
        let up = normalize(Vec3::new(view.x * -d, view.y * -d, view.z * -d) + world_up);
        let right = cross(up, view);
        Self {
            m: [
                [right.x, up.x, view.x, 0.0],
                [right.y, up.y, view.y, 0.0],
                [right.z, up.z, view.z, 0.0],
                [-dot(from, right), -dot(from, up), -dot(from, view), 1.0],
            ],
        }
    }
}

/// `CRandom`: the engine's linear congruential generator. The compiler seeds one per sample from
/// the sample's own data, or - where the C++ draws from its one `::Random` in an order that does
/// not depend on threads (the MU models' own light) - jumps to the draw that sample takes; it
/// never shares one between threads.
#[derive(Clone, Copy, Debug)]
pub struct XrRandom {
    hold: i32,
}

impl XrRandom {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self { hold: seed }
    }

    /// A generator seeded from some words (a position's bits, an index): the same words, the
    /// same sequence, on any thread.
    #[must_use]
    pub fn from_words(words: &[u32]) -> Self {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &w in words {
            h ^= u64::from(w);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
            h ^= h >> 29;
        }
        Self { hold: (h ^ (h >> 32)) as i32 }
    }

    /// The generator `n` draws further on: what `n` calls of [`Self::rand_i`] leave, in
    /// O(log n) (the step `x * a + c` composed with itself).
    #[must_use]
    pub fn advanced(self, mut n: u64) -> Self {
        let (mut a, mut c) = (214_013u32, 2_531_011u32);
        let (mut acc_a, mut acc_c) = (1u32, 0u32);
        while n > 0 {
            if n & 1 == 1 {
                acc_a = acc_a.wrapping_mul(a);
                acc_c = acc_c.wrapping_mul(a).wrapping_add(c);
            }
            c = c.wrapping_mul(a).wrapping_add(c);
            a = a.wrapping_mul(a);
            n >>= 1;
        }
        Self { hold: (self.hold as u32).wrapping_mul(acc_a).wrapping_add(acc_c) as i32 }
    }

    /// `randI`.
    pub fn rand_i(&mut self) -> i32 {
        self.hold = self.hold.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.hold >> 16) & 0x7fff
    }

    /// `randF`: `0..=1`.
    pub fn rand_f(&mut self) -> f32 {
        self.rand_i() as f32 / 32767.0
    }

    /// `randF(max)`.
    pub fn rand_f_max(&mut self, max: f32) -> f32 {
        self.rand_f() * max
    }

    /// `Fvector::random_dir`.
    pub fn dir(&mut self) -> Vec3 {
        let z = self.rand_f_max(PI).cos();
        let a = self.rand_f_max(PI_MUL_2);
        let r = (1.0 - z * z).sqrt();
        let (sa, ca) = (a.sin(), a.cos());
        Vec3::new(r * ca, r * sa, z)
    }

    /// `Fvector::random_dir(axis, angle)`: a direction in a cone.
    pub fn dir_cone(&mut self, axis: Vec3, angle: f32) -> Vec3 {
        let rnd = self.dir();
        let s = self.rand_f_max(angle.tan());
        normalize(mad(axis, rnd, s))
    }
}

/// The words of a point, for [`XrRandom::from_words`].
#[must_use]
pub fn point_words(p: Vec3) -> [u32; 3] {
    [p.x.to_bits(), p.y.to_bits(), p.z.to_bits()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_normalize_of_ordinary_and_tiny_vectors() {
        let (n, ok) = exact_normalize(Vec3::new(3.0, 0.0, 4.0));
        assert!(ok);
        assert_eq!(n, Vec3::new(0.6, 0.0, 0.8));
        // below the threshold the largest component decides; the result is still of unit length
        let (n, ok) = exact_normalize(Vec3::new(0.0, 1e-4, 0.0));
        assert!(ok);
        assert_eq!(n, Vec3::Y);
        assert_eq!(exact_normalize(Vec3::ZERO), (Vec3::Y, false));
    }

    #[test]
    fn the_engine_generator_repeats_the_msvc_sequence() {
        // the first values of rand() after srand(1)
        let mut r = XrRandom::new(1);
        assert_eq!([r.rand_i(), r.rand_i(), r.rand_i()], [41, 18467, 6334]);
        // a jump lands where as many draws do
        let mut stepped = XrRandom::new(7);
        for _ in 0..1000 {
            stepped.rand_i();
        }
        assert_eq!(XrRandom::new(7).advanced(1000).rand_i(), stepped.rand_i());
        assert_eq!(XrRandom::new(7).advanced(0).rand_i(), XrRandom::new(7).rand_i());
        let mut a = XrRandom::from_words(&[1, 2, 3]);
        let mut b = XrRandom::from_words(&[1, 2, 3]);
        assert_eq!(a.dir(), b.dir());
        assert!((magnitude(a.dir_cone(Vec3::Y, 0.5)) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn a_camera_looks_along_its_axis() {
        let m = Matrix::camera(Vec3::Y, Vec3::ZERO, Vec3::X);
        // a point below the eye lands straight ahead
        let p = m.transform_tiny(Vec3::new(0.0, -2.0, 0.0));
        assert!((p.z - 3.0).abs() < 1e-6 && p.x.abs() < 1e-6 && p.y.abs() < 1e-6);
        let inv = m.inverted_43();
        assert!((inv.transform_tiny(p) - Vec3::new(0.0, -2.0, 0.0)).length() < 1e-5);
    }
}
