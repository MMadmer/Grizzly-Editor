//! The engine's arithmetic the compiler's results depend on (vectors, matrices,
//! 2D boxes, the random generator, the exact normalize and the byte colour conversion), in its order of operations.

use glam::{Vec2, Vec3};
use smallvec::SmallVec;

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

/// The dot product, left to right.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// The cross product.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// The magnitude of a vector.
#[must_use]
pub fn magnitude(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// The squared distance between two points.
#[must_use]
pub fn distance_sqr(a: Vec3, b: Vec3) -> f32 {
    (a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y) + (a.z - b.z) * (a.z - b.z)
}

/// The distance between two points.
#[must_use]
pub fn distance(a: Vec3, b: Vec3) -> f32 {
    distance_sqr(a, b).sqrt()
}

/// The distance between two 2D points.
#[must_use]
pub fn distance2(a: Vec2, b: Vec2) -> f32 {
    ((a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y)).sqrt()
}

/// Scaled by `sqrt(1 / |v|^2)`.
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let mag = (1.0 / (v.x * v.x + v.y * v.y + v.z * v.z)).sqrt();
    Vec3::new(v.x * mag, v.y * mag, v.z * mag)
}

/// Left as it is when its length is below the smallest float.
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
    // the squares are summed in float and only then widened, as the engine does
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

/// The point from barycentric coordinates.
#[must_use]
pub fn from_bary(a: Vec3, b: Vec3, c: Vec3, w: Vec3) -> Vec3 {
    Vec3::new(
        a.x * w.x + b.x * w.y + c.x * w.z,
        a.y * w.x + b.y * w.y + c.y * w.z,
        a.z * w.x + b.z * w.y + c.z * w.z,
    )
}

/// `p + d * m`.
#[must_use]
pub fn mad(p: Vec3, d: Vec3, m: f32) -> Vec3 {
    Vec3::new(p.x + d.x * m, p.y + d.y * m, p.z + d.z * m)
}

/// The normal of a triangle, `normalize_safe`d.
#[must_use]
pub fn mknormal(p0: Vec3, p1: Vec3, p2: Vec3) -> Vec3 {
    normalize_safe(cross(p1 - p0, p2 - p1))
}

/// Linear interpolation of two vectors.
#[must_use]
pub fn lerp(a: Vec3, b: Vec3, t: f32) -> Vec3 {
    let inv = 1.0 - t;
    Vec3::new(a.x * inv + b.x * t, a.y * inv + b.y * t, a.z * inv + b.z * t)
}

/// The maximum of two vectors, component by component, the engine's `_max`.
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

/// The box's centre and the distance from it to `max`.
#[must_use]
pub fn sphere_of(b: &Aabb) -> (Vec3, f32) {
    let c = b.center();
    (c, distance(c, b.max))
}

/// A 2D box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Box2 {
    pub min: Vec2,
    pub max: Vec2,
}

impl Box2 {
    /// `invalidate`.
    pub const INVALID: Box2 = Box2 { min: Vec2::splat(f32::MAX), max: Vec2::splat(-f32::MAX) };

    /// The box of a triangle's three points, the way the deflector makes it.
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

/// Rows `i`, `j`, `k`, `c` (`_11 .. _44`), points as rows multiplied from the left.
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

/// The engine's linear congruential generator. The compiler seeds one per sample from
/// the sample's own data, or - where X-Ray's compiler draws from its one `::Random` in an order that does
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

    /// A random integer.
    pub fn rand_i(&mut self) -> i32 {
        self.hold = self.hold.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.hold >> 16) & 0x7fff
    }

    /// A random float: `0..=1`.
    pub fn rand_f(&mut self) -> f32 {
        self.rand_i() as f32 / 32767.0
    }

    /// `randF(max)`.
    pub fn rand_f_max(&mut self, max: f32) -> f32 {
        self.rand_f() * max
    }

    /// A random direction.
    pub fn dir(&mut self) -> Vec3 {
        let z = self.rand_f_max(PI).cos();
        let a = self.rand_f_max(PI_MUL_2);
        let r = (1.0 - z * z).sqrt();
        let (sa, ca) = (a.sin(), a.cos());
        Vec3::new(r * ca, r * sa, z)
    }

    /// A direction in a cone.
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

// ------------------------------------------------------------------------------------------ welding

/// A cell table for [`greedy_weld`]: open addressing over the runs of a list sorted by cell.
struct CellTable {
    keys: Vec<[i64; 3]>,
    /// `(start, end)` of a cell's run; `start == end` for a free slot.
    runs: Vec<(u32, u32)>,
    mask: usize,
}

impl CellTable {
    fn slot(key: &[i64; 3], mask: usize) -> usize {
        let mut h = (key[0] as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        h ^= (key[1] as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
        h ^= (key[2] as u64).wrapping_mul(0x1656_67B1_9E37_79F9);
        ((h ^ (h >> 29)) as usize) & mask
    }

    fn new(sorted: &[([i64; 3], u32)]) -> Self {
        let mut count = 0usize;
        let mut i = 0;
        while let Some(&(k, _)) = sorted.get(i) {
            count += 1;
            i += sorted.get(i..).map_or(1, |rest| rest.iter().take_while(|e| e.0 == k).count().max(1));
        }
        let cap = (count * 2).next_power_of_two().max(16);
        let mut t = Self { keys: vec![[0; 3]; cap], runs: vec![(0, 0); cap], mask: cap - 1 };
        let mut i = 0usize;
        while let Some(&(k, _)) = sorted.get(i) {
            let len = sorted.get(i..).map_or(1, |rest| rest.iter().take_while(|e| e.0 == k).count().max(1));
            let mut s = Self::slot(&k, t.mask);
            while t.runs.get(s).is_some_and(|r| r.0 != r.1) {
                s = (s + 1) & t.mask;
            }
            if let (Some(key), Some(run)) = (t.keys.get_mut(s), t.runs.get_mut(s)) {
                *key = k;
                *run = (i as u32, (i + len) as u32);
            }
            i += len;
        }
        t
    }

    fn get(&self, key: &[i64; 3]) -> (u32, u32) {
        let mut s = Self::slot(key, self.mask);
        loop {
            match (self.runs.get(s), self.keys.get(s)) {
                (Some(&r), Some(k)) if r.0 != r.1 => {
                    if k == key {
                        return r;
                    }
                }
                _ => return (0, 0),
            }
            s = (s + 1) & self.mask;
        }
    }
}

/// The weld every welding loop of the compiler makes, for a whole list at once: point `i` goes to the
/// first point before it that was kept and that `pred(kept, point)` accepts, and is kept when there is
/// none. Answers for every point the point it went to (itself when kept); `None` when `stop` said so.
///
/// `pred` must hold only for points closer than `radius` on every axis, must hold for a finite point
/// and itself, and must refuse every point that is not finite: such a point is always kept and never
/// taken. Equal points go where the first of them went, so the search is made once per distinct point,
/// in parallel; only the choice between the kept ones is made in order.
///
/// `same`, when given, names for every point an earlier one it is known to equal word for word (or
/// itself): the corners of one vertex. Only the points that name themselves are sorted to find the
/// equal ones; a list that does not keep that promise is not used.
pub fn greedy_weld<P>(
    points: &[Vec3],
    same: Option<&[u32]>,
    radius: f32,
    pred: P,
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<Vec<u32>>
where
    P: Fn(Vec3, Vec3) -> bool + Sync,
{
    use rayon::prelude::*;
    let n = points.len();
    let at = |i: u32| points.get(i as usize).copied().unwrap_or(Vec3::NAN);
    let same = same.filter(|s| {
        s.len() == n
            && s.par_iter().enumerate().all(|(i, &r)| {
                (r as usize) <= i
                    && s.get(r as usize) == Some(&r)
                    && point_words(at(r)) == point_words(at(i as u32))
            })
    });
    let root = |i: u32| same.is_none_or(|s| s.get(i as usize) == Some(&i));
    // the first of every set of equal points
    let mut by_bits: Vec<([u32; 3], u32)> = (0..n as u32)
        .into_par_iter()
        .filter_map(|i| {
            let p = at(i);
            (p.is_finite() && root(i)).then(|| (point_words(p), i))
        })
        .collect();
    by_bits.par_sort_unstable();
    let mut first = vec![u32::MAX; n];
    let mut run_word = None;
    let mut run_first = 0u32;
    for &(w, i) in &by_bits {
        if run_word != Some(w) {
            run_word = Some(w);
            run_first = i;
        }
        if let Some(slot) = first.get_mut(i as usize) {
            *slot = run_first;
        }
    }
    drop(by_bits);
    if let Some(s) = same {
        for (i, &r) in s.iter().enumerate() {
            if r as usize != i {
                let f = first.get(r as usize).copied().unwrap_or(u32::MAX);
                if let Some(slot) = first.get_mut(i) {
                    *slot = f;
                }
            }
        }
    }
    if stop() {
        return None;
    }
    let distinct: Vec<u32> =
        (0..n as u32).into_par_iter().filter(|&i| first.get(i as usize) == Some(&i)).collect();
    let mut rank = vec![u32::MAX; n];
    for (r, &i) in distinct.iter().enumerate() {
        if let Some(slot) = rank.get_mut(i as usize) {
            *slot = r as u32;
        }
    }
    // the distinct points by cells twice the radius wide: a point within the radius of another lies
    // in one of the 27 cells around it
    let size = 2.0 * f64::from(radius);
    let cell = |p: Vec3| {
        [
            (f64::from(p.x) / size).floor() as i64,
            (f64::from(p.y) / size).floor() as i64,
            (f64::from(p.z) / size).floor() as i64,
        ]
    };
    let mut by_cell: Vec<([i64; 3], u32)> =
        distinct.par_iter().enumerate().map(|(r, &i)| (cell(at(i)), r as u32)).collect();
    by_cell.par_sort_unstable();
    let table = CellTable::new(&by_cell);
    // what each distinct point may go to: the earlier distinct points `pred` accepts, in order
    let candidates: Vec<SmallVec<[u32; 2]>> = distinct
        .par_iter()
        .enumerate()
        .with_min_len(1024)
        .map(|(r, &i)| {
            let mut out = SmallVec::new();
            if r % 1024 == 0 && stop() {
                return out;
            }
            let v = at(i);
            let c = cell(v);
            for dx in -1i64..=1 {
                for dy in -1i64..=1 {
                    for dz in -1i64..=1 {
                        let key = [c[0].saturating_add(dx), c[1].saturating_add(dy), c[2].saturating_add(dz)];
                        let (s, e) = table.get(&key);
                        for &(_, r2) in by_cell.get(s as usize..e as usize).unwrap_or_default() {
                            // a run holds its points in rank order
                            if r2 as usize >= r {
                                break;
                            }
                            if let Some(&j) = distinct.get(r2 as usize)
                                && pred(at(j), v)
                            {
                                out.push(r2);
                            }
                        }
                    }
                }
            }
            out.sort_unstable();
            out
        })
        .collect();
    if stop() {
        return None;
    }
    // the choice in order: a distinct point goes to its first candidate that was kept
    let mut to_rank: Vec<u32> = (0..distinct.len() as u32).collect();
    for (r, cands) in candidates.iter().enumerate() {
        let kept = cands.iter().copied().find(|&c| to_rank.get(c as usize) == Some(&c));
        if let (Some(k), Some(slot)) = (kept, to_rank.get_mut(r)) {
            *slot = k;
        }
    }
    Some(
        (0..n)
            .into_par_iter()
            .map(|i| {
                let Some(&f) = first.get(i) else { return i as u32 };
                if f == u32::MAX {
                    return i as u32;
                }
                let r = rank.get(f as usize).copied().unwrap_or(u32::MAX);
                to_rank.get(r as usize).and_then(|&t| distinct.get(t as usize)).copied().unwrap_or(i as u32)
            })
            .collect(),
    )
}

/// The cells of `gz_fmt_level::CollectorPacked`: a 24 x 16 x 24 grid over a box given up front.
#[derive(Clone, Copy, Debug)]
pub struct PackedGrid {
    min: Vec3,
    scale: Vec3,
    eps: Vec3,
}

impl PackedGrid {
    const N: [u32; 3] = [24, 16, 24];

    #[must_use]
    pub fn new(bb: &Aabb) -> Self {
        let scale = Vec3::new(bb.max.x - bb.min.x, bb.max.y - bb.min.y, bb.max.z - bb.min.z);
        let half_cell = |s: f32, n: u32| {
            let e = s / n as f32 / 2.0;
            if e < EPS_L { e } else { EPS_L }
        };
        Self {
            min: bb.min,
            scale,
            eps: Vec3::new(
                half_cell(scale.x, Self::N[0]),
                half_cell(scale.y, Self::N[1]),
                half_cell(scale.z, Self::N[2]),
            ),
        }
    }

    /// The floor into a `u32`, clamped from above, as the X-Ray SDK takes it.
    fn cell(&self, v: Vec3) -> [u32; 3] {
        let one =
            |val: f32, min: f32, scale: f32, n: u32| (i_floor((val - min) / scale * n as f32) as u32).min(n);
        [
            one(v.x, self.min.x, self.scale.x, Self::N[0]),
            one(v.y, self.min.y, self.scale.y, Self::N[1]),
            one(v.z, self.min.z, self.scale.z, Self::N[2]),
        ]
    }

    /// Whether the collector welds point `v` to the earlier vertex `p`: within `EPS_L` on every axis,
    /// and `v`'s cell one of those `p` was put into.
    #[must_use]
    pub fn welds(&self, p: Vec3, v: Vec3) -> bool {
        if !((p.x - v.x).abs() < EPS_L && (p.y - v.y).abs() < EPS_L && (p.z - v.z).abs() < EPS_L) {
            return false;
        }
        let c = self.cell(v);
        let a = self.cell(p);
        let e = self.cell(Vec3::new(p.x + self.eps.x, p.y + self.eps.y, p.z + self.eps.z));
        (0..3).all(|k| c[k] == a[k] || c[k] == e[k])
    }
}

/// Points added in order with, for every one, the first point added for the same vertex: what
/// [`weld_packed`] takes as `same`. A vertex is a number within one mesh, so a new mesh (or the same
/// one placed again) begins afresh.
#[derive(Clone, Debug, Default)]
pub struct Corners {
    pub points: Vec<Vec3>,
    pub same: Vec<u32>,
    seen: Vec<u32>,
}

impl Corners {
    #[must_use]
    pub fn with_capacity(points: usize) -> Self {
        Self { points: Vec::with_capacity(points), same: Vec::with_capacity(points), seen: Vec::new() }
    }

    /// The vertices that follow are another mesh's.
    pub fn begin(&mut self) {
        self.seen.clear();
    }

    /// The point of vertex `v` (the same `p` every time).
    pub fn push(&mut self, v: u32, p: Vec3) {
        let i = self.points.len() as u32;
        let v = v as usize;
        if self.seen.len() <= v {
            self.seen.resize(v + 1, u32::MAX);
        }
        let first = match self.seen.get_mut(v) {
            Some(slot) if *slot == u32::MAX => {
                *slot = i;
                i
            }
            Some(slot) => *slot,
            None => i,
        };
        self.points.push(p);
        self.same.push(first);
    }
}

/// What `gz_fmt_level::CollectorPacked` makes of a list of points added in order, three a face:
/// the welded vertices and every point's vertex. `None` when `stop` said so. `same` as
/// [`greedy_weld`] takes it.
pub fn weld_packed(
    bb: &Aabb,
    points: &[Vec3],
    same: Option<&[u32]>,
    stop: &(dyn Fn() -> bool + Sync),
) -> Option<(Vec<Vec3>, Vec<u32>)> {
    let grid = PackedGrid::new(bb);
    let to = greedy_weld(points, same, EPS_L, |p, v| grid.welds(p, v), stop)?;
    let mut index = vec![0u32; points.len()];
    let mut verts = Vec::new();
    for (i, (&t, &p)) in to.iter().zip(points).enumerate() {
        let out = if t as usize == i {
            verts.push(p);
            (verts.len() - 1) as u32
        } else {
            // the vertex an earlier point made
            index.get(t as usize).copied().unwrap_or(0)
        };
        if let Some(slot) = index.get_mut(i) {
            *slot = out;
        }
    }
    Some((verts, index))
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

    /// The weld of a whole list is the packed collector's, point by point: lattice points nudged
    /// within and past the epsilon and across the cells, repeats, points outside the box and
    /// points that are not finite.
    #[test]
    fn the_packed_weld_is_the_collectors() {
        let mut s = 0x2545_F491_4F6C_DD1Du64;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        for (round, bb) in [
            Aabb { min: Vec3::splat(-10.0), max: Vec3::splat(10.0) },
            Aabb { min: Vec3::new(-3.0, -0.01, -3.0), max: Vec3::new(3.0, 0.0, 3.0) },
        ]
        .into_iter()
        .enumerate()
        {
            let mut points: Vec<Vec3> = Vec::new();
            for n in 0..30_000 {
                let mut q = || {
                    let lattice = (next() % 9) as f32 * 0.5 - 2.0;
                    let nudge = ((next() >> 40) as f32 / (1u64 << 24) as f32 - 0.5) * 0.004;
                    (lattice + nudge) * if round == 1 { 0.3 } else { 1.0 }
                };
                let p = match n % 11 {
                    0 if !points.is_empty() => points[(next() % points.len() as u64) as usize],
                    1 => Vec3::new(q(), f32::NAN, q()),
                    2 if n % 3 == 0 => Vec3::new(f32::INFINITY, q(), q()),
                    3 => Vec3::new(q(), q(), q()) + Vec3::splat(0.0009),
                    _ => Vec3::new(q(), q(), q()),
                };
                points.push(p);
            }
            let mut c = gz_fmt_level::CollectorPacked::new(&bb, 0, 0);
            for t in points.as_chunks::<3>().0 {
                c.add_face_d(t[0], t[1], t[2], 0, 0);
            }
            let (verts, index) = weld_packed(&bb, &points, None, &|| false).unwrap();
            // the same with every repeat named
            let mut seen: std::collections::HashMap<[u32; 3], u32> = std::collections::HashMap::new();
            let same: Vec<u32> = points
                .iter()
                .enumerate()
                .map(|(i, p)| *seen.entry(point_words(*p)).or_insert(i as u32))
                .collect();
            let (v2, i2) = weld_packed(&bb, &points, Some(&same), &|| false).unwrap();
            let bits = |v: &[Vec3]| v.iter().map(|p| point_words(*p)).collect::<Vec<_>>();
            assert_eq!((bits(&v2), i2), (bits(&verts), index.clone()), "round {round}, the repeats named");
            let words = |v: &[Vec3]| v.iter().map(|p| point_words(*p)).collect::<Vec<_>>();
            assert_eq!(words(&verts), words(c.verts()), "round {round}");
            let expect: Vec<u32> = c.tris().iter().flat_map(|t| t.verts).collect();
            assert_eq!(index, expect, "round {round}");
            assert!(verts.len() < points.len() * 9 / 10, "round {round}: nothing was welded");
        }
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
