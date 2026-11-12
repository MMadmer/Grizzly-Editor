//! The bounding spheres of `OGF_Base::CalcBounds` (`OGF_Face_Sphere.cpp`): Gaertner's miniball
//! (`Fsphere_compute`, xrCore `_sphere.cpp`), the box's own sphere, and Magic Software's
//! randomised minimum sphere (`Mgc::MinSphere`, `MgcCont3DMinSphere.cpp`), the smallest valid
//! one kept.
//!
//! `MinSphere` permutes the points with the C runtime's `rand()`, seeded with 367 at its first
//! call; every call of the compiler runs on its main thread, so the stream is one sequence over
//! the whole compile. [`MsvcRand`] is that generator; the caller hands it from call to call in
//! the C++ order.

use glam::Vec3;
use gz_lc_light::math::{Aabb, EPS_L, EPS_S, distance, distance_sqr};

/// The UCRT's `rand()`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MsvcRand {
    state: u32,
}

impl MsvcRand {
    /// `srand(seed)`.
    #[must_use]
    pub fn new(seed: u32) -> Self {
        Self { state: seed }
    }

    /// The stream `MinSphere` draws from: `srand(367)`.
    #[must_use]
    pub fn min_sphere() -> Self {
        Self::new(367)
    }

    /// `rand()`.
    pub fn rand(&mut self) -> i32 {
        self.state = self.state.wrapping_mul(214_013).wrapping_add(2_531_011);
        ((self.state >> 16) & 0x7fff) as i32
    }

    /// MSVC's `std::random_shuffle(first, last)`: `std::shuffle` over `rand()` through
    /// `_Rng_from_urng` (15 bits a draw, a value out of the unbiased range drawn again).
    pub fn shuffle<T>(&mut self, list: &mut [T]) {
        for target in 1..list.len() {
            let index = target as u64 + 1;
            let off = loop {
                let (mut ret, mut mask) = (0u64, 0u64);
                while mask < index - 1 {
                    ret = (ret << 15) | self.rand() as u64;
                    mask = (mask << 15) | 0x7fff;
                }
                if ret / index < mask / index || mask % index == index - 1 {
                    break (ret % index) as usize;
                }
            };
            if off != target {
                list.swap(target, off);
            }
        }
    }

    /// The generator after `k` draws, in `log k` steps (the affine map composed by squaring).
    #[must_use]
    pub fn advanced(mut self, mut k: u64) -> Self {
        let (mut a, mut c) = (214_013u32, 2_531_011u32);
        while k > 0 {
            if k & 1 == 1 {
                self.state = self.state.wrapping_mul(a).wrapping_add(c);
            }
            c = c.wrapping_mul(a).wrapping_add(c);
            a = a.wrapping_mul(a);
            k >>= 1;
        }
        self
    }
}

// ------------------------------------------------------------------------------- miniball

const NIL: usize = usize::MAX;

/// `Basis` of the miniball.
struct Basis {
    m: usize,
    q0: Vec3,
    z: [f32; 4],
    f: [f32; 4],
    v: [Vec3; 4],
    a: [Vec3; 4],
    c: [Vec3; 4],
    sqr_r: [f32; 4],
    cur_c: usize,
    cur_sqr_r: f32,
}

fn comp(v: &Vec3, i: usize) -> f32 {
    match i {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}
fn set_comp(v: &mut Vec3, i: usize, x: f32) {
    match i {
        0 => v.x = x,
        1 => v.y = x,
        _ => v.z = x,
    }
}
fn dotp(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}
fn madv(p: Vec3, d: Vec3, m: f32) -> Vec3 {
    Vec3::new(p.x + d.x * m, p.y + d.y * m, p.z + d.z * m)
}

impl Basis {
    fn new() -> Self {
        let mut b = Self {
            m: 0,
            q0: Vec3::ZERO,
            z: [0.0; 4],
            f: [0.0; 4],
            v: [Vec3::ZERO; 4],
            a: [Vec3::ZERO; 4],
            c: [Vec3::ZERO; 4],
            sqr_r: [0.0; 4],
            cur_c: 0,
            cur_sqr_r: 0.0,
        };
        b.reset();
        b
    }

    fn reset(&mut self) {
        self.m = 0;
        self.c[0] = Vec3::ZERO;
        self.cur_c = 0;
        self.cur_sqr_r = -1.0;
    }

    fn center(&self) -> Vec3 {
        self.c[self.cur_c]
    }

    fn excess(&self, p: Vec3) -> f32 {
        let mut e = -self.cur_sqr_r;
        e += distance_sqr(p, self.center());
        e
    }

    fn pop(&mut self) {
        self.m = self.m.saturating_sub(1);
    }

    fn push(&mut self, p: Vec3) -> bool {
        let m = self.m;
        if m >= 4 {
            return false;
        }
        if m == 0 {
            self.q0 = p;
            self.c[0] = self.q0;
            self.sqr_r[0] = 0.0;
        } else {
            let eps = 1e-16f32;
            self.v[m] = p - self.q0;
            for i in 1..m {
                let mut a = dotp(self.v[i], self.v[m]);
                a *= 2.0 / self.z[i];
                set_comp(&mut self.a[m], i, a);
            }
            for i in 1..m {
                let a = comp(&self.a[m], i);
                self.v[m] = madv(self.v[m], self.v[i], -a);
            }
            self.z[m] = 0.0;
            self.z[m] += dotp(self.v[m], self.v[m]);
            self.z[m] *= 2.0;
            if self.z[m] < eps * self.cur_sqr_r {
                return false;
            }
            let mut e = -self.sqr_r[m - 1];
            e += distance_sqr(p, self.c[m - 1]);
            self.f[m] = e / self.z[m];
            self.c[m] = madv(self.c[m - 1], self.v[m], self.f[m]);
            self.sqr_r[m] = self.sqr_r[m - 1] + e * self.f[m] / 2.0;
        }
        self.cur_c = m;
        self.cur_sqr_r = self.sqr_r[m];
        self.m = m + 1;
        true
    }
}

/// `Miniball` over an `xr_list` of points.
struct Miniball {
    pts: Vec<Vec3>,
    next: Vec<usize>,
    prev: Vec<usize>,
    head: usize,
    b: Basis,
    support_end: usize,
}

impl Miniball {
    fn new(points: &[Vec3]) -> Self {
        let n = points.len();
        let next = (0..n).map(|i| if i + 1 < n { i + 1 } else { NIL }).collect();
        let prev = (0..n).map(|i| if i == 0 { NIL } else { i - 1 }).collect();
        Self {
            pts: points.to_vec(),
            next,
            prev,
            head: if n > 0 { 0 } else { NIL },
            b: Basis::new(),
            support_end: NIL,
        }
    }

    fn nx(&self, i: usize) -> usize {
        self.next.get(i).copied().unwrap_or(NIL)
    }

    fn pt(&self, i: usize) -> Vec3 {
        self.pts.get(i).copied().unwrap_or(Vec3::ZERO)
    }

    /// `move_to_front`.
    fn move_to_front(&mut self, j: usize) {
        if self.support_end == j {
            self.support_end = self.nx(j);
        }
        if j == self.head || j == NIL {
            return;
        }
        let (p, n) = (self.prev.get(j).copied().unwrap_or(NIL), self.nx(j));
        if let Some(slot) = self.next.get_mut(p) {
            *slot = n;
        }
        if let Some(slot) = self.prev.get_mut(n) {
            *slot = p;
        }
        if let Some(slot) = self.prev.get_mut(self.head) {
            *slot = j;
        }
        if let Some(slot) = self.next.get_mut(j) {
            *slot = self.head;
        }
        if let Some(slot) = self.prev.get_mut(j) {
            *slot = NIL;
        }
        self.head = j;
    }

    fn mtf_mb(&mut self, end: usize) {
        self.support_end = self.head;
        if self.b.m == 4 {
            return;
        }
        let mut k = self.head;
        while k != end && k != NIL {
            let j = k;
            k = self.nx(k);
            if self.b.excess(self.pt(j)) > 0.0 && self.b.push(self.pt(j)) {
                self.mtf_mb(j);
                self.b.pop();
                self.move_to_front(j);
            }
        }
    }

    fn max_excess(&self, t: usize, end: usize, pivot: &mut usize) -> f32 {
        let c = self.b.center();
        let sqr_r = self.b.cur_sqr_r;
        let mut max_e = 0.0f32;
        let mut k = t;
        while k != end && k != NIL {
            let mut e = -sqr_r;
            e += distance_sqr(self.pt(k), c);
            if e > max_e {
                max_e = e;
                *pivot = k;
            }
            k = self.nx(k);
        }
        max_e
    }

    fn pivot_mb(&mut self) {
        let mut t = self.nx(self.head);
        self.mtf_mb(t);
        let mut old_sqr_r = 0.0f32;
        loop {
            let mut pivot = self.head;
            let max_e = self.max_excess(t, NIL, &mut pivot);
            if max_e > 0.0 {
                t = self.support_end;
                if t == pivot {
                    t = self.nx(t);
                }
                old_sqr_r = self.b.cur_sqr_r;
                self.b.push(self.pt(pivot));
                self.mtf_mb(self.support_end);
                self.b.pop();
                self.move_to_front(pivot);
            }
            if !(max_e > 0.0 && self.b.cur_sqr_r > old_sqr_r) {
                break;
            }
        }
    }
}

/// `Fsphere_compute`.
#[must_use]
pub fn miniball(points: &[Vec3]) -> (Vec3, f32) {
    let mut mb = Miniball::new(points);
    mb.b.reset();
    mb.support_end = mb.head;
    mb.pivot_mb();
    (mb.b.center(), mb.b.cur_sqr_r.sqrt())
}

// ------------------------------------------------------------------------------- Mgc::MinSphere

const MGC_EPS: f32 = 1e-03;
const ONE_PLUS_EPS: f32 = 1.0 + MGC_EPS;

#[derive(Clone, Copy, Debug)]
struct Sph {
    c: Vec3,
    /// The squared radius while it is computed.
    r: f32,
}

fn sq_len(v: Vec3) -> f32 {
    v.x * v.x + v.y * v.y + v.z * v.z
}
fn scale(s: f32, v: Vec3) -> Vec3 {
    Vec3::new(s * v.x, s * v.y, s * v.z)
}

fn inside(p: Vec3, s: &Sph) -> bool {
    sq_len(p - s.c) <= ONE_PLUS_EPS * s.r
}

fn exact2(p0: Vec3, p1: Vec3) -> Sph {
    Sph { c: scale(0.5, p0 + p1), r: 0.25 * sq_len(p1 - p0) }
}

fn exact3(p0: Vec3, p1: Vec3, p2: Vec3) -> Sph {
    let a = p0 - p2;
    let b = p1 - p2;
    let ada = dotp(a, a);
    let adb = dotp(a, b);
    let bdb = dotp(b, b);
    let det = ada * bdb - adb * adb;
    if det.abs() > MGC_EPS {
        let hid = 0.5 / det;
        let u0 = hid * bdb * (ada - adb);
        let u1 = hid * ada * (bdb - adb);
        let u2 = 1.0 - u0 - u1;
        let c = scale(u0, p0) + scale(u1, p1) + scale(u2, p2);
        let t = scale(u0, a) + scale(u1, b);
        Sph { c, r: sq_len(t) }
    } else {
        Sph { c: Vec3::ZERO, r: f32::MAX }
    }
}

fn exact4(p0: Vec3, p1: Vec3, p2: Vec3, p3: Vec3) -> Sph {
    let e10 = p0 - p3;
    let e20 = p1 - p3;
    let e30 = p2 - p3;
    let mut a = [[0.0f32; 3]; 3];
    a[0][0] = dotp(e10, e10);
    a[0][1] = dotp(e10, e20);
    a[0][2] = dotp(e10, e30);
    a[1][0] = a[0][1];
    a[1][1] = dotp(e20, e20);
    a[1][2] = dotp(e20, e30);
    a[2][0] = a[0][2];
    a[2][1] = a[1][2];
    a[2][2] = dotp(e30, e30);
    let b = [0.5 * a[0][0], 0.5 * a[1][1], 0.5 * a[2][2]];
    let mut inv = [[0.0f32; 3]; 3];
    inv[0][0] = a[1][1] * a[2][2] - a[1][2] * a[2][1];
    inv[0][1] = a[0][2] * a[2][1] - a[0][1] * a[2][2];
    inv[0][2] = a[0][1] * a[1][2] - a[0][2] * a[1][1];
    inv[1][0] = a[1][2] * a[2][0] - a[1][0] * a[2][2];
    inv[1][1] = a[0][0] * a[2][2] - a[0][2] * a[2][0];
    inv[1][2] = a[0][2] * a[1][0] - a[0][0] * a[1][2];
    inv[2][0] = a[1][0] * a[2][1] - a[1][1] * a[2][0];
    inv[2][1] = a[0][1] * a[2][0] - a[0][0] * a[2][1];
    inv[2][2] = a[0][0] * a[1][1] - a[0][1] * a[1][0];
    let det = a[0][0] * inv[0][0] + a[0][1] * inv[1][0] + a[0][2] * inv[2][0];
    if det.abs() > MGC_EPS {
        let id = 1.0 / det;
        for row in &mut inv {
            for v in row.iter_mut() {
                *v *= id;
            }
        }
        let mut u = [0.0f32; 4];
        for r in 0..3 {
            u[r] = 0.0;
            for c in 0..3 {
                u[r] += inv[r][c] * b[c];
            }
        }
        u[3] = 1.0 - u[0] - u[1] - u[2];
        let c = scale(u[0], p0) + scale(u[1], p1) + scale(u[2], p2) + scale(u[3], p3);
        let t = scale(u[0], e10) + scale(u[1], e20) + scale(u[2], e30);
        Sph { c, r: sq_len(t) }
    } else {
        Sph { c: Vec3::ZERO, r: f32::MAX }
    }
}

struct Support {
    n: usize,
    idx: [usize; 4],
}

impl Support {
    fn contains(&self, i: usize, perm: &[Vec3]) -> bool {
        let p = perm.get(i).copied().unwrap_or(Vec3::ZERO);
        self.idx
            .iter()
            .take(self.n)
            .any(|&k| sq_len(p - perm.get(k).copied().unwrap_or(Vec3::ZERO)) < MGC_EPS)
    }
}

fn update1(i: usize, perm: &[Vec3], s: &mut Support) -> Sph {
    let g = |k: usize| perm.get(k).copied().unwrap_or(Vec3::ZERO);
    let m = exact2(g(s.idx[0]), g(i));
    s.n = 2;
    s.idx[1] = i;
    m
}

fn update2(i: usize, perm: &[Vec3], s: &mut Support) -> Sph {
    let g = |k: usize| perm.get(k).copied().unwrap_or(Vec3::ZERO);
    let (p0, p1, p2) = (g(s.idx[0]), g(s.idx[1]), g(i));
    let mut min_r = f32::MAX;
    let mut index = usize::MAX;
    let s0 = exact2(p0, p2);
    if inside(p1, &s0) {
        min_r = s0.r;
        index = 0;
    }
    let s1 = exact2(p1, p2);
    if s1.r < min_r && inside(p0, &s1) {
        index = 1;
    }
    match index {
        0 => {
            s.idx[1] = i;
            s0
        }
        1 => {
            s.idx[0] = i;
            s1
        }
        _ => {
            s.n = 3;
            s.idx[2] = i;
            exact3(p0, p1, p2)
        }
    }
}

fn update3(i: usize, perm: &[Vec3], s: &mut Support) -> Sph {
    let g = |k: usize| perm.get(k).copied().unwrap_or(Vec3::ZERO);
    let (p0, p1, p2, p3) = (g(s.idx[0]), g(s.idx[1]), g(s.idx[2]), g(i));
    let cands = [
        (exact2(p0, p3), vec![p1, p2]),
        (exact2(p1, p3), vec![p0, p2]),
        (exact2(p2, p3), vec![p0, p1]),
        (exact3(p0, p1, p3), vec![p2]),
        (exact3(p0, p2, p3), vec![p1]),
        (exact3(p1, p2, p3), vec![p0]),
    ];
    let mut min_r = f32::MAX;
    let mut index = usize::MAX;
    for (k, (sph, others)) in cands.iter().enumerate() {
        let ok = if k == 0 { true } else { sph.r < min_r };
        if ok && others.iter().all(|&p| inside(p, sph)) {
            min_r = sph.r;
            index = k;
        }
    }
    match index {
        0 => {
            s.n = 2;
            s.idx[1] = i;
        }
        1 => {
            s.n = 2;
            s.idx[0] = i;
        }
        2 => {
            s.n = 2;
            s.idx[0] = s.idx[2];
            s.idx[1] = i;
        }
        3 => s.idx[2] = i,
        4 => s.idx[1] = i,
        5 => s.idx[0] = i,
        _ => {
            s.n = 4;
            s.idx[3] = i;
            return exact4(p0, p1, p2, p3);
        }
    }
    cands.get(index).map_or(Sph { c: Vec3::ZERO, r: f32::MAX }, |c| c.0)
}

fn update4(i: usize, perm: &[Vec3], s: &mut Support) -> Sph {
    let g = |k: usize| perm.get(k).copied().unwrap_or(Vec3::ZERO);
    let (p0, p1, p2, p3, p4) = (g(s.idx[0]), g(s.idx[1]), g(s.idx[2]), g(s.idx[3]), g(i));
    let cands = [
        (exact2(p0, p4), vec![p1, p2, p3]),
        (exact2(p1, p4), vec![p0, p2, p3]),
        (exact2(p2, p4), vec![p0, p1, p3]),
        (exact2(p3, p4), vec![p0, p1, p2]),
        (exact3(p0, p1, p4), vec![p2, p3]),
        (exact3(p0, p2, p4), vec![p1, p3]),
        (exact3(p0, p3, p4), vec![p1, p2]),
        (exact3(p1, p2, p4), vec![p0, p3]),
        (exact3(p1, p3, p4), vec![p0, p2]),
        (exact3(p2, p3, p4), vec![p0, p1]),
        (exact4(p0, p1, p2, p4), vec![p3]),
        (exact4(p0, p1, p3, p4), vec![p2]),
        (exact4(p0, p2, p3, p4), vec![p1]),
        (exact4(p1, p2, p3, p4), vec![p0]),
    ];
    let mut min_r = f32::MAX;
    let mut index = usize::MAX;
    for (k, (sph, others)) in cands.iter().enumerate() {
        let ok = if k == 0 { true } else { sph.r < min_r };
        if ok && others.iter().all(|&p| inside(p, sph)) {
            min_r = sph.r;
            index = k;
        }
    }
    match index {
        0 => {
            s.n = 2;
            s.idx[1] = i;
        }
        1 => {
            s.n = 2;
            s.idx[0] = i;
        }
        2 => {
            s.n = 2;
            s.idx[0] = s.idx[2];
            s.idx[1] = i;
        }
        3 => {
            s.n = 2;
            s.idx[0] = s.idx[3];
            s.idx[1] = i;
        }
        4 => {
            s.n = 3;
            s.idx[2] = i;
        }
        5 => {
            s.n = 3;
            s.idx[1] = i;
        }
        6 => {
            s.n = 3;
            s.idx[1] = s.idx[3];
            s.idx[2] = i;
        }
        7 => {
            s.n = 3;
            s.idx[0] = i;
        }
        8 => {
            s.n = 3;
            s.idx[0] = s.idx[3];
            s.idx[2] = i;
        }
        9 => {
            s.n = 3;
            s.idx[0] = s.idx[3];
            s.idx[1] = i;
        }
        10 => s.idx[3] = i,
        11 => s.idx[2] = i,
        12 => s.idx[1] = i,
        13 => s.idx[0] = i,
        _ => s.n = 5,
    }
    // oracle: with no candidate the C++ reads `akS[-1]`; the restart below throws it away
    cands.get(index).map_or(Sph { c: Vec3::ZERO, r: 0.0 }, |c| c.0)
}

/// `Mgc::MinSphere`: the points permuted with `rand`, the support grown point by point; a
/// support that would need a fifth point starts again with a new permutation.
pub fn min_sphere(points: &[Vec3], rng: &mut MsvcRand) -> (Vec3, f32) {
    if points.is_empty() {
        return (Vec3::ZERO, 0.0);
    }
    let mut restarts = 0usize;
    'restart: loop {
        let mut perm: Vec<Vec3> = points.to_vec();
        for i in (1..perm.len()).rev() {
            let j = (rng.rand() % (i as i32 + 1)) as usize;
            if j != i {
                perm.swap(i, j);
            }
        }
        let mut minimal = Sph { c: perm[0], r: 0.0 };
        let mut supp = Support { n: 1, idx: [0; 4] };
        let mut i = 1usize;
        while i < perm.len() {
            if !supp.contains(i, &perm) && !inside(perm[i], &minimal) {
                let sph = match supp.n {
                    1 => update1(i, &perm, &mut supp),
                    2 => update2(i, &perm, &mut supp),
                    3 => update3(i, &perm, &mut supp),
                    _ => update4(i, &perm, &mut supp),
                };
                if supp.n == 5 {
                    restarts += 1;
                    if restarts < 1000 {
                        continue 'restart;
                    }
                    break 'restart;
                }
                if sph.r > minimal.r {
                    minimal = sph;
                    i = 0;
                    continue;
                }
            }
            i += 1;
        }
        return (minimal.c, minimal.r.sqrt());
    }
    (Vec3::ZERO, f32::MAX)
}

/// `SphereValid`: every point within the sphere grown by `EPS_L`.
fn valid(points: &[Vec3], c: Vec3, r: f32) -> bool {
    let r = r + EPS_L;
    points.iter().all(|p| distance_sqr(c, *p) <= r * r + EPS_S)
}

/// What `CalcBounds` leaves in an `OGF_Base`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Bounds {
    pub bbox: Aabb,
    pub center: Vec3,
    pub radius: f32,
}

/// `OGF_Base::CalcBounds` over a node's geometry.
pub fn calc_bounds(points: &[Vec3], rng: &mut MsvcRand) -> Bounds {
    let (c1, r1) = miniball(points);
    let b1 = valid(points, c1, r1);

    let mut bbox = Aabb::INVALID;
    for p in points {
        bbox.modify(*p);
    }
    bbox.grow(EPS_L);
    let c2 = bbox.center();
    let mut r2 = -1.0f32;
    for p in points {
        let d = distance_sqr(c2, *p);
        if d > r2 {
            r2 = d;
        }
    }
    let _ = distance;
    let r2 = r2.abs().sqrt();
    let b2 = valid(points, c2, r2);
    let _ = b2;

    let (c3, r3) = min_sphere(points, rng);
    let b3 = valid(points, c3, r3);

    let (center, radius) = if b1 && r1 < r2 {
        if b3 && r3 < r1 { (c3, r3) } else { (c1, r1) }
    } else if b3 && r3 < r2 {
        (c3, r3)
    } else {
        (c2, r2)
    };
    Bounds { bbox, center, radius }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_c_runtime_sequence() {
        let mut r = MsvcRand::new(1);
        // the first values of rand() without srand on the UCRT
        assert_eq!([r.rand(), r.rand(), r.rand()], [41, 18467, 6334]);
    }

    #[test]
    fn the_spheres_of_a_cube_hold_its_corners() {
        let mut pts = Vec::new();
        for x in [0.0, 2.0] {
            for y in [0.0, 2.0] {
                for z in [0.0, 2.0] {
                    pts.push(Vec3::new(x, y, z));
                }
            }
        }
        let (c, r) = miniball(&pts);
        assert!((c - Vec3::ONE).length() < 1e-5);
        assert!((r - 3f32.sqrt()).abs() < 1e-5);
        let mut rng = MsvcRand::min_sphere();
        let (c, r) = min_sphere(&pts, &mut rng);
        assert!((c - Vec3::ONE).length() < 1e-3, "{c:?}");
        assert!((r - 3f32.sqrt()).abs() < 1e-3, "{r}");
        let b = calc_bounds(&pts, &mut rng);
        assert!(b.radius <= 3f32.sqrt() + 1e-3);
        assert_eq!(b.bbox.min, Vec3::splat(-EPS_L));
    }
}
