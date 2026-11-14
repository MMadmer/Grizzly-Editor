//! The oriented boxes the exporters fit: Wild Magic's minimal box (a Powell
//! minimisation of the box volume over three angles) for bones, and RAPID's
//! moment box (over an eigen decomposition) for the parts of a breakable object.
//!
//! Float for float: `f32` where the X-Ray SDK has `float`, cosine, sine and square root as
//! the double functions rounded to float, the X-Ray SDK's order of every operation. Where the X-Ray SDK reads
//! memory it never wrote, this writes a value of its own (said at each place).

// the loops index several arrays at once
#![allow(clippy::needless_range_loop)]

use glam::Vec3;
use gz_fmt_object::xmath::cross;

use crate::bones::Obb;

/// `Math<float>::_PI`: `(float)(4.0 * atan(1.0))`.
const PI: f32 = std::f64::consts::PI as f32;
const HALF_PI: f32 = 0.5 * PI;
const MAX_REAL: f32 = f32::MAX;

/// The double function, rounded to float.
fn wcos(v: f32) -> f32 {
    f64::from(v).cos() as f32
}

fn wsin(v: f32) -> f32 {
    f64::from(v).sin() as f32
}

fn wsqrt(v: f32) -> f32 {
    f64::from(v).sqrt() as f32
}

/// A 3x3 matrix of `f32`, row major.
type M3 = [f32; 9];

/// The rotation matrix of an axis and an angle.
fn from_axis_angle(axis: [f32; 3], angle: f32) -> M3 {
    let cs = wcos(angle);
    let sn = wsin(angle);
    let one_minus_cos = 1.0 - cs;
    let x2 = axis[0] * axis[0];
    let y2 = axis[1] * axis[1];
    let z2 = axis[2] * axis[2];
    let xym = axis[0] * axis[1] * one_minus_cos;
    let xzm = axis[0] * axis[2] * one_minus_cos;
    let yzm = axis[1] * axis[2] * one_minus_cos;
    let xsin = axis[0] * sn;
    let ysin = axis[1] * sn;
    let zsin = axis[2] * sn;
    [
        x2 * one_minus_cos + cs,
        xym - zsin,
        xzm + ysin,
        xym + zsin,
        y2 * one_minus_cos + cs,
        yzm - xsin,
        xzm - ysin,
        yzm + xsin,
        z2 * one_minus_cos + cs,
    ]
}

/// `Vector * Matrix`: the row vector times the matrix, each sum started at zero.
fn vec_mul_mat(v: Vec3, m: &M3) -> [f32; 3] {
    let v = [v.x, v.y, v.z];
    let mut out = [0.0f32; 3];
    for (col, o) in out.iter_mut().enumerate() {
        let mut p = 0.0f32;
        for (row, x) in v.iter().enumerate() {
            p += x * m[col + 3 * row];
        }
        *o = p;
    }
    out
}

/// `Matrix * Vector`.
fn mat_mul_vec(m: &M3, v: [f32; 3]) -> [f32; 3] {
    let mut out = [0.0f32; 3];
    for (row, o) in out.iter_mut().enumerate() {
        let mut p = 0.0f32;
        for (col, x) in v.iter().enumerate() {
            p += m[3 * row + col] * x;
        }
        *o = p;
    }
    out
}

/// The rotation of three angles.
fn rotation(angle: &[f32; 3]) -> M3 {
    let cos0 = wcos(angle[0]);
    let sin0 = wsin(angle[0]);
    let cos1 = wcos(angle[1]);
    let sin1 = wsin(angle[1]);
    from_axis_angle([cos0 * sin1, sin0 * sin1, cos1], angle[2])
}

/// The extremes of the points in the rotated frame; the `else if` is kept as in the X-Ray SDK.
fn extremes(points: &[Vec3], rot: &M3) -> ([f32; 3], [f32; 3]) {
    let first = points.first().copied().unwrap_or(Vec3::ZERO);
    let mut lo = vec_mul_mat(first, rot);
    let mut hi = lo;
    for p in points.iter().skip(1) {
        let t = vec_mul_mat(*p, rot);
        for k in 0..3 {
            if t[k] < lo[k] {
                lo[k] = t[k];
            } else if t[k] > hi[k] {
                hi[k] = t[k];
            }
        }
    }
    (lo, hi)
}

/// The volume of a box.
fn volume(points: &[Vec3], angle: &[f32; 3]) -> f32 {
    let (lo, hi) = extremes(points, &rotation(angle));
    (hi[0] - lo[0]) * (hi[1] - lo[1]) * (hi[2] - lo[2])
}

/// `Box3`: the centre, the axes, the extents.
struct Box3 {
    center: [f32; 3],
    axis: [[f32; 3]; 3],
    extent: [f32; 3],
}

/// The minimal box for three angles.
fn box_for_angles(points: &[Vec3], angle: &[f32; 3]) -> Box3 {
    let rot = rotation(angle);
    let (lo, hi) = extremes(points, &rot);
    let mid = [0.5 * (hi[0] + lo[0]), 0.5 * (hi[1] + lo[1]), 0.5 * (hi[2] + lo[2])];
    let rng = [0.5 * (hi[0] - lo[0]), 0.5 * (hi[1] - lo[1]), 0.5 * (hi[2] - lo[2])];
    let column = |c: usize| [rot[c], rot[3 + c], rot[6 + c]];
    Box3 { center: mat_mul_vec(&rot, mid), axis: [column(0), column(1), column(2)], extent: rng }
}

/// `Minimize1<float>`: the bracketing line search.
struct Minimize1 {
    max_level: i32,
    max_bracket: i32,
    t_min: f32,
    f_min: f32,
}

impl Minimize1 {
    fn get_minimum(&mut self, f: &mut dyn FnMut(f32) -> f32, t0: f32, t1: f32, t_initial: f32) -> (f32, f32) {
        self.t_min = MAX_REAL;
        self.f_min = MAX_REAL;
        let f0 = f(t0);
        let f_initial = f(t_initial);
        let f1 = f(t1);
        self.search3(f, t0, f0, t_initial, f_initial, t1, f1, self.max_level);
        (self.t_min, self.f_min)
    }

    fn keep(&mut self, t: f32, v: f32) {
        if v < self.f_min {
            self.t_min = t;
            self.f_min = v;
        }
    }

    /// The minimum of a function inside a bracket of three points, searched to a depth.
    #[allow(clippy::too_many_arguments)]
    fn search3(
        &mut self,
        f: &mut dyn FnMut(f32) -> f32,
        t0: f32,
        f0: f32,
        tm: f32,
        fm: f32,
        t1: f32,
        f1: f32,
        level: i32,
    ) {
        self.keep(t0, f0);
        self.keep(tm, fm);
        self.keep(t1, f1);
        if level == 0 {
            return;
        }
        let level = level - 1;
        if (t1 - tm) * (f0 - fm) > (tm - t0) * (fm - f1) {
            self.positive(f, t0, f0, tm, fm, t1, f1, level);
        } else {
            self.nonpositive(f, t0, f0, tm, fm, t1, f1, level);
        }
    }

    /// The minimum of a function between two points, searched to a depth.
    fn search2(&mut self, f: &mut dyn FnMut(f32) -> f32, t0: f32, f0: f32, t1: f32, f1: f32, level: i32) {
        self.keep(t0, f0);
        self.keep(t1, f1);
        if level == 0 {
            return;
        }
        let level = level - 1;
        let tm = 0.5 * (t0 + t1);
        let fm = f(tm);
        if f0 - 2.0 * fm + f1 > 0.0 {
            self.positive(f, t0, f0, tm, fm, t1, f1, level);
        } else {
            self.nonpositive(f, t0, f0, tm, fm, t1, f1, level);
        }
    }

    /// The branch for a quadratic fit with a positive second derivative at the midpoint.
    #[allow(clippy::too_many_arguments)]
    fn positive(
        &mut self,
        f: &mut dyn FnMut(f32) -> f32,
        t0: f32,
        f0: f32,
        tm: f32,
        fm: f32,
        t1: f32,
        f1: f32,
        level: i32,
    ) {
        if f1 > f0 {
            if fm >= f0 {
                self.search2(f, t0, f0, tm, fm, level);
            } else {
                self.bracketed(f, t0, f0, tm, fm, t1, f1, level);
            }
        } else if f1 < f0 {
            if fm >= f1 {
                self.search2(f, tm, fm, t1, f1, level);
            } else {
                self.bracketed(f, t0, f0, tm, fm, t1, f1, level);
            }
        } else {
            self.search2(f, t0, f0, tm, fm, level);
            self.search2(f, tm, fm, t1, f1, level);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn nonpositive(
        &mut self,
        f: &mut dyn FnMut(f32) -> f32,
        t0: f32,
        f0: f32,
        tm: f32,
        fm: f32,
        t1: f32,
        f1: f32,
        level: i32,
    ) {
        if f1 > f0 {
            self.search2(f, t0, f0, tm, fm, level);
        } else if f1 < f0 {
            self.search2(f, tm, fm, t1, f1, level);
        } else {
            self.search2(f, t0, f0, tm, fm, level);
            self.search2(f, tm, fm, t1, f1, level);
        }
    }

    /// Parabolic steps inside a bracket.
    #[allow(clippy::too_many_arguments)]
    fn bracketed(
        &mut self,
        f: &mut dyn FnMut(f32) -> f32,
        mut t0: f32,
        mut f0: f32,
        mut tm: f32,
        mut fm: f32,
        mut t1: f32,
        mut f1: f32,
        level: i32,
    ) {
        const EPS: f32 = 1e-08;
        const TOL: f32 = 1e-04;
        for _ in 0..self.max_bracket {
            self.keep(tm, fm);
            if (t1 - t0).abs() <= 2.0 * TOL * tm.abs() + EPS {
                break;
            }
            let dt0 = t0 - tm;
            let dt1 = t1 - tm;
            let df0 = f0 - fm;
            let df1 = f1 - fm;
            let tmp0 = dt0 * df1;
            let tmp1 = dt1 * df0;
            let denom = tmp1 - tmp0;
            if denom.abs() < EPS {
                return;
            }
            let tv = tm + 0.5 * (dt1 * tmp1 - dt0 * tmp0) / denom;
            let fv = f(tv);
            if tv < tm {
                if fv < fm {
                    t1 = tm;
                    f1 = fm;
                    tm = tv;
                    fm = fv;
                } else {
                    t0 = tv;
                    f0 = fv;
                }
            } else if tv > tm {
                if fv < fm {
                    t0 = tm;
                    f0 = fm;
                    tm = tv;
                    fm = fv;
                } else {
                    t1 = tv;
                    f1 = fv;
                }
            } else {
                // the search covers both halves and then goes on with the same bracket
                self.search2(f, t0, f0, tm, fm, level);
                self.search2(f, tm, fm, t1, f1, level);
            }
        }
    }
}

/// Powell's direction set over a one-dimensional minimisation, in three dimensions.
///
/// The directions are pointers into one storage in the X-Ray SDK, and the cycling at the end of a
/// pass moves the pointers without refreshing the last one: from the second pass on a
/// direction is used twice and an older conjugate one is lost. `slot` / `conj` keep that.
fn minimize_n(
    f: &dyn Fn(&[f32; 3]) -> f32,
    t0: &[f32; 3],
    t1: &[f32; 3],
    t_initial: &[f32; 3],
    max_iterations: usize,
    line: &mut Minimize1,
) -> ([f32; 3], f32) {
    let mut f_curr = f(t_initial);
    let mut t_save = *t_initial;
    let mut t_curr = *t_initial;
    let mut storage = [[0.0f32; 3]; 4];
    let mut slot = [0usize, 1, 2, 3];
    let mut conj = slot[3];
    for (i, s) in slot.iter().take(3).enumerate() {
        storage[*s][i] = 1.0;
    }

    let domain = |t_curr: &[f32; 3], d: &[f32; 3]| -> (f32, f32) {
        let mut l0 = -MAX_REAL;
        let mut l1 = MAX_REAL;
        for i in 0..3 {
            let mut b0 = t0[i] - t_curr[i];
            let mut b1 = t1[i] - t_curr[i];
            if d[i] > 0.0 {
                let inv = 1.0 / d[i];
                b0 *= inv;
                if b0 > l0 {
                    l0 = b0;
                }
                b1 *= inv;
                if b1 < l1 {
                    l1 = b1;
                }
            } else if d[i] < 0.0 {
                let inv = 1.0 / d[i];
                b0 *= inv;
                if b0 < l1 {
                    l1 = b0;
                }
                b1 *= inv;
                if b1 > l0 {
                    l0 = b1;
                }
            }
        }
        if l0 > 0.0 {
            l0 = 0.0;
        }
        if l1 < 0.0 {
            l1 = 0.0;
        }
        (l0, l1)
    };
    let mut along = |t_curr: &mut [f32; 3], d: [f32; 3], f_curr: &mut f32| {
        let (l0, l1) = domain(t_curr, &d);
        let base = *t_curr;
        let mut lf = |t: f32| f(&[base[0] + t * d[0], base[1] + t * d[1], base[2] + t * d[2]]);
        let (l_min, f_min) = line.get_minimum(&mut lf, l0, l1, 0.0);
        *f_curr = f_min;
        for j in 0..3 {
            t_curr[j] += l_min * d[j];
        }
    };

    for _ in 0..max_iterations {
        for s in slot.iter().take(3) {
            along(&mut t_curr, storage[*s], &mut f_curr);
        }
        let mut length = 0.0f32;
        for i in 0..3 {
            storage[conj][i] = t_curr[i] - t_save[i];
            length += storage[conj][i] * storage[conj][i];
        }
        length = wsqrt(length);
        if length < 1e-06 {
            break;
        }
        let inv = 1.0 / length;
        for i in 0..3 {
            storage[conj][i] *= inv;
        }
        along(&mut t_curr, storage[conj], &mut f_curr);
        conj = slot[0];
        slot.copy_within(1..4, 0);
        t_save = t_curr;
    }
    (t_curr, f_curr)
}

/// `MinBox3<float>`: a grid of 64 angle triples narrows the search, Powell's method finishes it.
fn min_box3(points: &[Vec3]) -> Box3 {
    let a0 = [0.0f32; 3];
    let a1 = [PI, HALF_PI, PI];
    let vol = |a: &[f32; 3]| volume(points, a);
    let mut min_volume = MAX_REAL;
    // the start is uninitialised in the X-Ray SDK when no sample is below FLT_MAX
    let mut initial = [0.0f32; 3];
    const STEPS: i32 = 3;
    let inv_max = 1.0f32 / STEPS as f32;
    for i0 in 0..=STEPS {
        let x = a0[0] + i0 as f32 * (a1[0] - a0[0]) * inv_max;
        for i1 in 0..=STEPS {
            let y = a0[1] + i1 as f32 * (a1[1] - a0[1]) * inv_max;
            for i2 in 0..=STEPS {
                let z = a0[2] + i2 as f32 * (a1[2] - a0[2]) * inv_max;
                let v = vol(&[x, y, z]);
                if v < min_volume {
                    min_volume = v;
                    initial = [x, y, z];
                }
            }
        }
    }
    let mut line = Minimize1 { max_level: 8, max_bracket: 8, t_min: 0.0, f_min: 0.0 };
    let (best, _) = minimize_n(&vol, &a0, &a1, &initial, 32, &mut line);
    box_for_angles(points, &best)
}

/// `B.m_rotate.i.crossproduct(j, k); j.crossproduct(k, i)`.
fn orthogonalise(b: &mut Obb) {
    b.rotate[0] = cross(b.rotate[1], b.rotate[2]);
    b.rotate[1] = cross(b.rotate[2], b.rotate[0]);
}

fn v3(a: [f32; 3]) -> Vec3 {
    Vec3::new(a[0], a[1], a[2])
}

/// The minimal box of the points; an invalid box for fewer than three.
#[must_use]
pub fn min_box(points: &[Vec3]) -> Obb {
    if points.len() < 3 {
        return Obb::INVALID;
    }
    let b = min_box3(points);
    // a box whose volume is not below FLT_MAX stays as it was declared in the X-Ray SDK
    // (uninitialised); its own box stands in here
    let mut obb = Obb { rotate: b.axis.map(v3), translate: v3(b.center), halfsize: v3(b.extent) };
    orthogonalise(&mut obb);
    obb
}

/// A 3x3 matrix, `m[row][col]`; the rows are the axes `i`, `j`, `k`.
type F33 = [[f32; 3]; 3];

/// The Jacobi rotations, the eigenvalues and the eigenvectors as columns.
// after 50 sweeps without convergence the X-Ray SDK returns without writing the vectors (the
// box keeps what its memory held); the vectors reached stand in here
fn meigen(mut a: F33) -> (F33, [f32; 3]) {
    let mut v: F33 = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    let mut b = [a[0][0], a[1][1], a[2][2]];
    let mut d = b;
    let mut z = [0.0f32; 3];
    fn rot(m: &mut F33, s: f32, tau: f32, i: usize, j: usize, k: usize, l: usize) {
        let g = m[i][j];
        let h = m[k][l];
        m[i][j] = g - s * (h + g * tau);
        m[k][l] = h + s * (g - h * tau);
    }
    // (p, q) pairs in the order of the X-Ray SDK's blocks, with the rotations each applies
    struct Block {
        p: usize,
        q: usize,
        rot_a: [(usize, usize, usize, usize); 1],
        rot_v: [(usize, usize, usize, usize); 3],
    }
    let blocks = [
        Block { p: 0, q: 1, rot_a: [(0, 2, 1, 2)], rot_v: [(0, 0, 0, 1), (1, 0, 1, 1), (2, 0, 2, 1)] },
        Block { p: 0, q: 2, rot_a: [(0, 1, 1, 2)], rot_v: [(0, 0, 0, 2), (1, 0, 1, 2), (2, 0, 2, 2)] },
        Block { p: 1, q: 2, rot_a: [(0, 1, 0, 2)], rot_v: [(0, 1, 0, 2), (1, 1, 1, 2), (2, 1, 2, 2)] },
    ];
    for i in 0..50 {
        let mut sm = 0.0f32;
        sm += a[0][1].abs();
        sm += a[0][2].abs();
        sm += a[1][2].abs();
        if sm == 0.0 {
            return (v, d);
        }
        let tresh = if i < 3 { 0.2 * sm / (3.0 * 3.0) } else { 0.0 };
        for bl in &blocks {
            let (p, q) = (bl.p, bl.q);
            let g = 100.0 * a[p][q].abs();
            if i > 3 && d[p].abs() + g == d[p].abs() && d[q].abs() + g == d[q].abs() {
                a[p][q] = 0.0;
            } else if a[p][q].abs() > tresh {
                let mut h = d[q] - d[p];
                let t = if h.abs() + g == h.abs() {
                    a[p][q] / h
                } else {
                    let theta = 0.5 * h / a[p][q];
                    let t = 1.0 / (theta.abs() + (1.0 + theta * theta).sqrt());
                    if theta < 0.0 { -t } else { t }
                };
                let c = 1.0 / (1.0 + t * t).sqrt();
                let s = t * c;
                let tau = s / (1.0 + c);
                h = t * a[p][q];
                z[p] -= h;
                z[q] += h;
                d[p] -= h;
                d[q] += h;
                a[p][q] = 0.0;
                for &(i0, j0, k0, l0) in &bl.rot_a {
                    rot(&mut a, s, tau, i0, j0, k0, l0);
                }
                for &(i0, j0, k0, l0) in &bl.rot_v {
                    rot(&mut v, s, tau, i0, j0, k0, l0);
                }
            }
        }
        for k in 0..3 {
            b[k] += z[k];
        }
        d = b;
        z = [0.0; 3];
    }
    (v, d)
}

/// `eigen_and_sort1`: the eigenvector of the largest eigenvalue to column 0.
fn eigen_and_sort1(cov: F33) -> F33 {
    let (mut e, evals) = meigen(cov);
    let swap = |e: &mut F33, c: usize| {
        for row in e.iter_mut() {
            row.swap(0, c);
        }
    };
    if evals[2] > evals[0] {
        if evals[2] > evals[1] {
            swap(&mut e, 2);
        } else {
            swap(&mut e, 1);
        }
    } else {
        let first_is_largest = evals[0] > evals[1];
        if !first_is_largest {
            swap(&mut e, 1);
        }
    }
    e
}

/// `moment`: a triangle's area, centroid and area-weighted second-order components.
#[derive(Clone, Copy)]
struct Moment {
    area: f32,
    m: [f32; 3],
    s: F33,
}

/// `compute_moment`.
fn compute_moment(p: Vec3, q: Vec3, r: Vec3) -> Moment {
    let u = Vec3::new(q.x - p.x, q.y - p.y, q.z - p.z);
    let v = Vec3::new(r.x - p.x, r.y - p.y, r.z - p.z);
    let w = cross(u, v);
    let area = 0.5 * (w.x * w.x + w.y * w.y + w.z * w.z).sqrt();
    let m = [(p.x + q.x + r.x) / 3.0, (p.y + q.y + r.y) / 3.0, (p.z + q.z + r.z) / 3.0];
    let (p, q, r) = ([p.x, p.y, p.z], [q.x, q.y, q.z], [r.x, r.y, r.z]);
    let mut s: F33 = [[0.0; 3]; 3];
    if area == 0.0 {
        // a triangle without area: the centroid and the vertices' plain second-order sums
        for (i, j) in [(0, 0), (0, 1), (0, 2), (1, 1), (1, 2), (2, 2)] {
            s[i][j] = p[i] * p[j] + q[i] * q[j] + r[i] * r[j];
        }
    } else {
        for (i, j) in [(0, 0), (0, 1), (1, 1), (0, 2), (1, 2), (2, 2)] {
            s[i][j] = area * (9.0 * m[i] * m[j] + p[i] * p[j] + q[i] * q[j] + r[i] * r[j]) / 12.0;
        }
    }
    s[2][1] = s[1][2];
    s[1][0] = s[0][1];
    s[2][0] = s[0][2];
    Moment { area, m, s }
}

/// The box along the covariance of the triangles `points` holds three by
/// three; an invalid box for none.
#[must_use]
pub fn rapid_box(points: &[Vec3]) -> Obb {
    let tris = points.len() / 3;
    if tris < 1 || points.len() < 3 {
        return Obb::INVALID;
    }
    // `compute_moments`: the smallest non-zero area stands in for the triangles without one
    let mut moments: Vec<Moment> = Vec::with_capacity(tris);
    let mut a_min = 0.0f32;
    let mut zero = false;
    for t in points.as_chunks::<3>().0 {
        let m = compute_moment(t[0], t[1], t[2]);
        if m.area == 0.0 {
            zero = true;
        } else if a_min == 0.0 || m.area < a_min {
            a_min = m.area;
        }
        moments.push(m);
    }
    if zero {
        if a_min == 0.0 {
            a_min = 1.0;
        }
        for m in &mut moments {
            if m.area == 0.0 {
                m.area = a_min;
            }
        }
    }
    // `accum_moment` over all of them
    let mut acc_m = [0.0f32; 3];
    let mut acc_s: F33 = [[0.0; 3]; 3];
    let mut acc_a = 0.0f32;
    for b in &moments {
        for k in 0..3 {
            acc_m[k] += b.m[k] * b.area;
        }
        for i in 0..3 {
            for j in 0..3 {
                acc_s[i][j] += b.s[i][j];
            }
        }
        acc_a += b.area;
    }
    let mut cov: F33 = [[0.0; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            cov[i][j] = acc_s[i][j] - acc_m[i] * acc_m[j] / acc_a;
        }
    }
    let rot = eigen_and_sort1(cov);
    let mtxv = |v: Vec3| -> [f32; 3] {
        [
            rot[0][0] * v.x + rot[1][0] * v.y + rot[2][0] * v.z,
            rot[0][1] * v.x + rot[1][1] * v.y + rot[2][1] * v.z,
            rot[0][2] * v.x + rot[1][2] * v.y + rot[2][2] * v.z,
        ]
    };
    let c0 = mtxv(points[0]);
    let mut lo = c0;
    let mut hi = c0;
    for p in points {
        let c = mtxv(*p);
        for k in 0..3 {
            if c[k] < lo[k] {
                lo[k] = c[k];
            } else if c[k] > hi[k] {
                hi[k] = c[k];
            }
        }
    }
    let c = [(lo[0] + hi[0]) * 0.5, (lo[1] + hi[1]) * 0.5, (lo[2] + hi[2]) * 0.5];
    let translate = Vec3::new(
        c[0] * rot[0][0] + c[1] * rot[0][1] + c[2] * rot[0][2],
        c[0] * rot[1][0] + c[1] * rot[1][1] + c[2] * rot[1][2],
        c[0] * rot[2][0] + c[1] * rot[2][1] + c[2] * rot[2][2],
    );
    let halfsize = Vec3::new((hi[0] - lo[0]) * 0.5, (hi[1] - lo[1]) * 0.5, (hi[2] - lo[2]) * 0.5);
    // RAPID keeps the axes as columns; the engine's box as rows
    let t = |r: usize| Vec3::new(rot[0][r], rot[1][r], rot[2][r]);
    let mut obb = Obb { rotate: [t(0), t(1), t(2)], translate, halfsize };
    orthogonalise(&mut obb);
    obb
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    #[test]
    fn an_axis_aligned_brick_fits_itself() {
        // the corners of a 2 x 1 x 0.5 brick: the minimum is the brick itself
        let mut pts = Vec::new();
        for x in [0.0, 2.0] {
            for y in [0.0, 1.0] {
                for z in [0.0, 0.5] {
                    pts.push(Vec3::new(x, y, z));
                }
            }
        }
        let b = min_box(&pts);
        let mut h = [b.halfsize.x, b.halfsize.y, b.halfsize.z];
        h.sort_by(f32::total_cmp);
        assert!(close(h[0], 0.25, 1e-4) && close(h[1], 0.5, 1e-4) && close(h[2], 1.0, 1e-4), "{h:?}");
        assert!(b.translate.abs_diff_eq(Vec3::new(1.0, 0.5, 0.25), 1e-4));
        // the axes stay a right-handed orthonormal frame
        let [i, j, k] = b.rotate;
        assert!(
            close(i.dot(j), 0.0, 1e-5) && close(j.dot(k), 0.0, 1e-5) && close(i.cross(j).dot(k), 1.0, 1e-4)
        );
    }

    #[test]
    fn the_grid_start_gives_a_unit_cube_its_volume() {
        // the eight corners of a unit cube: the first grid sample (0, 0, 0) is the identity
        // rotation of axis (0, 0, 1) and already gives volume 1; nothing beats it
        let pts: Vec<Vec3> =
            (0..8).map(|i| Vec3::new((i & 1) as f32, ((i >> 1) & 1) as f32, ((i >> 2) & 1) as f32)).collect();
        assert_eq!(volume(&pts, &[0.0, 0.0, 0.0]), 1.0);
        let b = min_box(&pts);
        assert!(close(b.halfsize.x * b.halfsize.y * b.halfsize.z, 0.125, 1e-5));
    }

    #[test]
    fn fewer_than_three_points_give_the_invalid_box() {
        assert_eq!(min_box(&[Vec3::ZERO, Vec3::X]), Obb::INVALID);
        assert_eq!(rapid_box(&[Vec3::ZERO, Vec3::X]), Obb::INVALID);
    }

    #[test]
    fn the_axis_angle_rotation_is_wild_magics() {
        // a quarter turn about z sends x to y (row-major entries, the vector on the left)
        let m = from_axis_angle([0.0, 0.0, 1.0], HALF_PI);
        let x = vec_mul_mat(Vec3::X, &m);
        assert!(close(x[0], 0.0, 1e-6) && close(x[1], -1.0, 1e-6));
        let y = mat_mul_vec(&m, [1.0, 0.0, 0.0]);
        assert!(close(y[1], 1.0, 1e-6));
    }

    #[test]
    fn a_flat_triangle_pair_gets_its_plane_from_rapid() {
        // two triangles of the square (0..2, 0..1) in z = 0: the largest spread is along x
        let pts = [
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(2.0, 1.0, 0.0),
            Vec3::new(0.0, 1.0, 0.0),
        ];
        let b = rapid_box(&pts);
        assert!(b.translate.abs_diff_eq(Vec3::new(1.0, 0.5, 0.0), 1e-5));
        let mut h = [b.halfsize.x, b.halfsize.y, b.halfsize.z];
        h.sort_by(f32::total_cmp);
        assert!(close(h[0], 0.0, 1e-6) && close(h[1], 0.5, 1e-5) && close(h[2], 1.0, 1e-5), "{h:?}");
    }

    #[test]
    fn jacobi_diagonalises_a_symmetric_matrix() {
        let (v, d) = meigen([[2.0, 1.0, 0.0], [1.0, 2.0, 0.0], [0.0, 0.0, 5.0]]);
        let mut e = d;
        e.sort_by(f32::total_cmp);
        assert!(close(e[0], 1.0, 1e-5) && close(e[1], 3.0, 1e-5) && close(e[2], 5.0, 1e-5));
        // the third axis is untouched
        assert!(close(v[2][2], 1.0, 1e-6));
    }
}
