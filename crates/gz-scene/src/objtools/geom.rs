//! The arithmetic the object tools share: the engine's random numbers, Magic
//! Software's plane fit and 2D convex hull,
//! the library meshes as the sectors
//! and portals see them, and the frustum's polygon test.

use std::sync::Arc;

use glam::{Vec2, Vec4};
use gz_fmt_level::{Bvh, Ray};
use gz_fmt_object::{MeshFlags, ObjectFlags};

use crate::class::ObjClass;
use crate::library::{Library, LibraryObject};
use crate::math::{Aabb, Mat4, Vec3};

/// The engine's `EPS_S`, `EPS`, `EPS_L` (`vector.h`).
pub const EPS_S: f32 = 0.000_000_1;
pub const EPS: f32 = 0.000_01;
pub const EPS_L: f32 = 0.001;
use crate::model::{ObjData, Object, SceneState};

/// `CRandom`: the MSVC `rand` recurrence the engine keeps per generator.
#[derive(Clone, Debug)]
pub struct CRandom {
    hold: i32,
}

impl CRandom {
    #[must_use]
    pub fn new(seed: i32) -> Self {
        Self { hold: seed }
    }

    /// `randI()`: 0..32767.
    pub fn rand_i(&mut self) -> i32 {
        self.hold = self.hold.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.hold >> 16) & 0x7fff
    }

    /// `randI(min, max)`: `min + randI() % (max - min)`.
    pub fn rand_range(&mut self, min: i32, max: i32) -> i32 {
        let span = max - min;
        if span <= 0 {
            return min;
        }
        min + self.rand_i() % span
    }

    /// `randF(max)`.
    pub fn rand_f(&mut self, max: f32) -> f32 {
        self.rand_i() as f32 / 32767.0 * max
    }

    /// A random direction.
    pub fn random_dir(&mut self) -> Vec3 {
        let z = self.rand_f(std::f32::consts::PI).cos();
        let a = self.rand_f(std::f32::consts::TAU);
        let r = (1.0 - z * z).max(0.0).sqrt();
        Vec3::new(r * a.cos(), r * a.sin(), z)
    }

    /// A random direction: the axis bent by a random amount, normalised.
    pub fn random_dir_cone(&mut self, axis: Vec3, angle: f32) -> Vec3 {
        let rnd = self.random_dir();
        let k = self.rand_f(angle.tan());
        normalize(axis + rnd * k)
    }
}

/// No guard, as the engine's (a zero vector stays NaN-free here only by
/// being returned as it is).
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let m = v.x * v.x + v.y * v.y + v.z * v.z;
    if m > f32::MIN_POSITIVE { v * (1.0 / m).sqrt() } else { v }
}

/// `(p1 - p0) x (p2 - p1)`, normalised when it can be.
#[must_use]
pub fn mknormal(p0: Vec3, p1: Vec3, p2: Vec3) -> Vec3 {
    normalize((p1 - p0).cross(p2 - p1))
}

/// Whether two vectors are similar within `EPS_L`.
#[must_use]
pub fn similar(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

/// The centre and the distance from it to `max`.
#[must_use]
pub fn box_sphere(b: &Aabb) -> (Vec3, f32) {
    let c = b.center();
    (c, c.distance(b.max))
}

/// A box transformed by a matrix, from its eight corners.
#[must_use]
pub fn xform_corners(min: Vec3, max: Vec3, m: &Mat4) -> Aabb {
    let mut out = Aabb::INVALID;
    for i in 0..8 {
        let c = match i {
            0 => Vec3::new(min.x, min.y, min.z),
            1 => Vec3::new(min.x, min.y, max.z),
            2 => Vec3::new(max.x, min.y, max.z),
            3 => Vec3::new(max.x, min.y, min.z),
            4 => Vec3::new(min.x, max.y, min.z),
            5 => Vec3::new(min.x, max.y, max.z),
            6 => Vec3::new(max.x, max.y, max.z),
            _ => Vec3::new(max.x, max.y, min.z),
        };
        out.modify(m.transform_tiny(c));
    }
    out
}

/// Whether two boxes intersect.
#[must_use]
pub fn boxes_meet(a: &Aabb, b: &Aabb) -> bool {
    !(a.max.x < b.min.x
        || a.max.y < b.min.y
        || a.max.z < b.min.z
        || a.min.x > b.max.x
        || a.min.y > b.max.y
        || a.min.z > b.max.z)
}

/// Whether a box contains a point.
#[must_use]
pub fn box_holds(b: &Aabb, p: Vec3) -> bool {
    p.x >= b.min.x && p.x <= b.max.x && p.y >= b.min.y && p.y <= b.max.y && p.z >= b.min.z && p.z <= b.max.z
}

// ---- Mgc::Eigen and the plane fit -------------------------------------------------------------

/// The tridiagonal reduction of a 3x3 matrix.
fn tridiagonal3(m: &mut [[f32; 3]; 3], diag: &mut [f32; 3], subd: &mut [f32; 3]) {
    let (m00, mut m01, mut m02, m11, m12, m22) = (m[0][0], m[0][1], m[0][2], m[1][1], m[1][2], m[2][2]);
    diag[0] = m00;
    subd[2] = 0.0;
    if m02 != 0.0 {
        let length = (m01 * m01 + m02 * m02).sqrt();
        let inv = 1.0 / length;
        m01 *= inv;
        m02 *= inv;
        let q = 2.0 * m01 * m12 + m02 * (m22 - m11);
        diag[1] = m11 + m02 * q;
        diag[2] = m22 - m02 * q;
        subd[0] = length;
        subd[1] = m12 - m01 * q;
        *m = [[1.0, 0.0, 0.0], [0.0, m01, m02], [0.0, m02, -m01]];
    } else {
        diag[1] = m11;
        diag[2] = m22;
        subd[0] = m01;
        subd[1] = m12;
        *m = [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
}

/// The QL algorithm for a 3x3 matrix.
fn ql_algorithm(diag: &mut [f32; 3], subd: &mut [f32; 3], m: &mut [[f32; 3]; 3]) -> bool {
    const SIZE: usize = 3;
    const MAX_ITER: usize = 32;
    for i0 in 0..SIZE {
        let mut i1 = 0;
        while i1 < MAX_ITER {
            let mut i2 = i0;
            while i2 + 2 <= SIZE {
                let tmp = diag[i2].abs() + diag[i2 + 1].abs();
                if subd[i2].abs() + tmp == tmp {
                    break;
                }
                i2 += 1;
            }
            if i2 == i0 {
                break;
            }
            let mut g = (diag[i0 + 1] - diag[i0]) / (2.0 * subd[i0]);
            let mut r = (g * g + 1.0).sqrt();
            g = if g < 0.0 {
                diag[i2] - diag[i0] + subd[i0] / (g - r)
            } else {
                diag[i2] - diag[i0] + subd[i0] / (g + r)
            };
            let (mut sin, mut cos, mut p) = (1.0f32, 1.0f32, 0.0f32);
            let mut i3 = i2 as isize - 1;
            while i3 >= i0 as isize {
                let k = i3 as usize;
                let mut f = sin * subd[k];
                let b = cos * subd[k];
                if f.abs() >= g.abs() {
                    cos = g / f;
                    r = (cos * cos + 1.0).sqrt();
                    subd[k + 1] = f * r;
                    sin = 1.0 / r;
                    cos *= sin;
                } else {
                    sin = f / g;
                    r = (sin * sin + 1.0).sqrt();
                    subd[k + 1] = g * r;
                    cos = 1.0 / r;
                    sin *= cos;
                }
                g = diag[k + 1] - p;
                r = (diag[k] - g) * sin + 2.0 * b * cos;
                p = sin * r;
                diag[k + 1] = g + p;
                g = cos * r - b;
                for row in m.iter_mut() {
                    f = row[k + 1];
                    row[k + 1] = sin * row[k] + cos * f;
                    row[k] = cos * row[k] - sin * f;
                }
                i3 -= 1;
            }
            diag[i0] -= p;
            subd[i0] = g;
            subd[i2] = 0.0;
            i1 += 1;
        }
        if i1 == MAX_ITER {
            return false;
        }
    }
    true
}

/// Sorts the eigenvalues in decreasing order.
fn decreasing_sort(val: &mut [f32; 3], vec: &mut [[f32; 3]; 3]) {
    for i0 in 0..2 {
        let mut i1 = i0;
        let mut max = val[i1];
        for (i2, v) in val.iter().enumerate().skip(i0 + 1) {
            if *v > max {
                i1 = i2;
                max = *v;
            }
        }
        if i1 != i0 {
            val[i1] = val[i0];
            val[i0] = max;
            for row in vec.iter_mut() {
                row.swap(i0, i1);
            }
        }
    }
}

/// The points' average and the unit normal of the least-squares plane
/// (the eigenvector of the smallest eigenvalue).
#[must_use]
pub fn plane_fit(points: &[Vec3]) -> (Vec3, Vec3) {
    let Some(first) = points.first() else { return (Vec3::ZERO, Vec3::Y) };
    let mut offset = *first;
    for p in &points[1..] {
        offset += *p;
    }
    offset *= 1.0 / points.len() as f32;
    let (mut xx, mut xy, mut xz, mut yy, mut yz, mut zz) = (0.0f32, 0.0, 0.0, 0.0, 0.0, 0.0);
    for p in points {
        let d = *p - offset;
        xx += d.x * d.x;
        xy += d.x * d.y;
        xz += d.x * d.z;
        yy += d.y * d.y;
        yz += d.y * d.z;
        zz += d.z * d.z;
    }
    let mut m = [[xx, xy, xz], [xy, yy, yz], [xz, yz, zz]];
    let mut diag = [0.0f32; 3];
    let mut subd = [0.0f32; 3];
    tridiagonal3(&mut m, &mut diag, &mut subd);
    ql_algorithm(&mut diag, &mut subd, &mut m);
    decreasing_sort(&mut diag, &mut m);
    (offset, Vec3::new(m[0][2], m[1][2], m[2][2]))
}

/// A plane from a point and a normal: the normal normalised, `d = -n . p`.
#[must_use]
pub fn plane_build(p: Vec3, n: Vec3) -> Vec4 {
    let n = normalize(n);
    Vec4::new(n.x, n.y, n.z, -n.dot(p))
}

// ---- Mgc::ConvexHull2D ------------------------------------------------------------------------

const COL_EPSILON: f32 = 1e-06;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Order {
    Positive,
    Negative,
    CollinearLeft,
    CollinearRight,
    CollinearContain,
}

/// Where `p` lies against the line through `q0` and `q1`.
fn collinear_test(p: Vec2, q0: Vec2, q1: Vec2) -> Order {
    let d = q1 - q0;
    let a = p - q0;
    let ddd = d.dot(d);
    let ada = a.dot(a);
    let det = d.x * a.y - d.y * a.x;
    let relative = det * det - COL_EPSILON * ddd * ada;
    if relative > 0.0 {
        if det > 0.0 {
            return Order::Positive;
        } else if det < 0.0 {
            return Order::Negative;
        }
    }
    let dda = d.dot(a);
    if dda < 0.0 {
        return Order::CollinearLeft;
    }
    if dda > ddd {
        return Order::CollinearRight;
    }
    Order::CollinearContain
}

#[derive(Clone, Copy, Debug)]
struct Sv {
    v: Vec2,
    index: usize,
}

fn merge_linear(hull: &mut Vec<Sv>, planar: &mut bool, p: Sv) {
    let (Some(q0), Some(q1)) = (hull.first().copied(), hull.get(1).copied()) else { return };
    match collinear_test(p.v, q0.v, q1.v) {
        Order::Positive => {
            *planar = true;
            hull.push(q1);
            hull[1] = q0;
            hull[0] = p;
        }
        Order::Negative => {
            *planar = true;
            hull.push(q0);
            hull[0] = p;
        }
        Order::CollinearLeft => hull[0] = p,
        Order::CollinearRight => hull[1] = p,
        Order::CollinearContain => {}
    }
}

fn merge_planar(hull: &mut Vec<Sv>, p: Sv) {
    let size = hull.len();
    if size < 2 {
        return;
    }
    // counterclockwise: the last visible vertex
    let (mut iu, mut i) = (0usize, 1usize);
    let mut steps = 0;
    loop {
        if i == size {
            i = 0;
        }
        match collinear_test(p.v, hull[iu].v, hull[i].v) {
            Order::Negative => {
                iu = i;
                i += 1;
                steps += 1;
                if steps > size {
                    return;
                }
            }
            Order::Positive | Order::CollinearLeft => break,
            _ => return,
        }
    }
    // clockwise
    let mut il = 0usize;
    let mut j = size as isize - 1;
    let mut found = false;
    while j >= 0 {
        let k = j as usize;
        match collinear_test(p.v, hull[k].v, hull[il].v) {
            Order::Negative => {
                il = k;
                j -= 1;
            }
            Order::Positive | Order::CollinearRight => {
                found = true;
                break;
            }
            _ => return,
        }
    }
    if !found {
        return;
    }
    let mut tmp = vec![p];
    let mut n = 0;
    loop {
        tmp.push(hull[iu]);
        if iu == il || n > size {
            break;
        }
        iu += 1;
        if iu == size {
            iu = 0;
        }
        n += 1;
    }
    if tmp.len() > 2 {
        *hull = tmp;
    }
}

/// `ConvexHull2D(points).ByIncremental(); RemoveCollinear();`: the hull's point indices,
/// counterclockwise.
#[must_use]
pub fn convex_hull(points: &[Vec2]) -> Vec<usize> {
    if points.is_empty() {
        return Vec::new();
    }
    let mut sorted: Vec<Sv> = points.iter().enumerate().map(|(index, v)| Sv { v: *v, index }).collect();
    sorted.sort_by(|a, b| a.v.x.total_cmp(&b.v.x).then(a.v.y.total_cmp(&b.v.y)));
    sorted.dedup_by(|a, b| a.v == b.v);
    let mut planar = false;
    let mut hull = vec![sorted[0]];
    for p in sorted.iter().skip(1) {
        if hull.len() == 1 {
            hull.push(*p);
        } else if planar {
            merge_planar(&mut hull, *p);
        } else {
            merge_linear(&mut hull, &mut planar, *p);
        }
    }
    let idx: Vec<usize> = hull.iter().map(|s| s.index).collect();
    remove_collinear(points, idx)
}

fn remove_collinear(points: &[Vec2], idx: Vec<usize>) -> Vec<usize> {
    let q = idx.len();
    if q <= 2 {
        return idx;
    }
    let at = |i: usize| points.get(idx[i]).copied().unwrap_or(Vec2::ZERO);
    let mut out = Vec::with_capacity(q);
    let (mut i0, mut i2) = (q - 1, 1);
    for (i1, &keep) in idx.iter().enumerate() {
        if matches!(collinear_test(at(i0), at(i1), at(i2)), Order::Positive | Order::Negative) {
            out.push(keep);
        }
        i0 = i1;
        i2 += 1;
        if i2 == q {
            i2 = 0;
        }
    }
    out
}

// ---- the library meshes -----------------------------------------------------------------------

/// Static in either sense: no sound occluder, no dynamic object, no HOM.
#[must_use]
pub fn is_static_any(lo: &LibraryObject) -> bool {
    let f = lo.object.flags;
    !f.contains(ObjectFlags::SOUND_OCCLUDER)
        && !f.contains(ObjectFlags::DYNAMIC)
        && !f.contains(ObjectFlags::HOM)
}

/// Static and used more than once.
#[must_use]
pub fn is_mu_static(lo: &LibraryObject) -> bool {
    is_static_any(lo) && lo.object.flags.contains(ObjectFlags::MULTIPLE_USAGE)
}

/// Finds a mesh by name: case-insensitive.
#[must_use]
pub fn mesh_index(lo: &LibraryObject, name: &str) -> Option<usize> {
    lo.object.meshes.iter().position(|m| m.name.eq_ignore_ascii_case(name))
}

/// The faces of a mesh: two-sided surfaces count twice, `materials\occ` not at all.
#[must_use]
pub fn face_count(lo: &LibraryObject, mesh: usize) -> usize {
    let Some(m) = lo.object.meshes.get(mesh) else { return 0 };
    m.surf_faces
        .iter()
        .map(|sf| {
            let surf = lo.object.surfaces.get(sf.surface as usize);
            if surf.is_some_and(|s| s.game_mtl == "materials\\occ") {
                0
            } else if surf.is_some_and(|s| s.flags.contains(gz_fmt_object::SurfaceFlags::TWO_SIDED)) {
                sf.faces.len() * 2
            } else {
                sf.faces.len()
            }
        })
        .sum()
}

/// A mesh's box carried by the object's transform.
#[must_use]
pub fn mesh_world_box(lo: &LibraryObject, mesh: usize, world: &Mat4) -> Option<Aabb> {
    let m = lo.object.meshes.get(mesh)?;
    Some(xform_corners(m.bbox.min, m.bbox.max, world))
}

/// Every face of a mesh in world space (each vertex carried by the transform).
#[must_use]
pub fn mesh_world_faces(lo: &LibraryObject, mesh: usize, world: &Mat4) -> Vec<[Vec3; 3]> {
    let Some(m) = lo.object.meshes.get(mesh) else { return Vec::new() };
    let at = |i: u32| m.points.get(i as usize).map_or(Vec3::ZERO, |p| world.transform_tiny(*p));
    m.faces.iter().map(|f| [at(f.v[0].point), at(f.v[1].point), at(f.v[2].point)]).collect()
}

/// The picking tree of one mesh in object space; `None` for a hidden or empty mesh.
#[must_use]
pub fn mesh_bvh(lo: &LibraryObject, mesh: usize) -> Option<Bvh> {
    let m = lo.object.meshes.get(mesh)?;
    if !m.flags.contains(MeshFlags::VISIBLE) {
        return None;
    }
    let tris: Vec<[u32; 3]> = m.faces.iter().map(|f| [f.v[0].point, f.v[1].point, f.v[2].point]).collect();
    Bvh::build(&m.points, &tris).ok()
}

/// A mesh's tree with the object's inverse transform: whether the ray hits it closer than `range`
/// (nearest hit, culling).
#[must_use]
pub fn mesh_ray_hits(bvh: &Bvh, inv: &Mat4, start: Vec3, dir: Vec3, range: f32) -> bool {
    let mut ray = Ray::new(inv.transform_tiny(start), inv.transform_dir(dir), range);
    ray.cull_back = true;
    bvh.ray_nearest(&ray).is_some_and(|h| h.t < range)
}

/// Which mesh of a library object a picking tree's triangle belongs to: the tree holds the
/// visible meshes' faces one mesh after another.
#[must_use]
pub fn mesh_of_triangle(lo: &LibraryObject, tri: u32) -> Option<usize> {
    let mut first = 0u32;
    for (i, m) in lo.object.meshes.iter().enumerate() {
        if !m.flags.contains(MeshFlags::VISIBLE) {
            continue;
        }
        let n = u32::try_from(m.faces.len()).unwrap_or(u32::MAX);
        if tri < first.saturating_add(n) {
            return Some(i);
        }
        first = first.saturating_add(n);
    }
    None
}

/// A scene object hit by a ray with the mesh under the hit.
#[derive(Clone, Debug)]
pub struct MeshHit {
    pub object: Arc<Object>,
    pub mesh: usize,
    pub mesh_name: String,
    pub t: f32,
}

/// The nearest visible scene object of a visible tool that a ray meets within its range, with
/// its mesh.
#[must_use]
pub fn pick_mesh(state: &SceneState, lib: &Library, start: Vec3, dir: Vec3, range: f32) -> Option<MeshHit> {
    let tool_visible = state.tools.part(ObjClass::SceneObject).visible;
    if !tool_visible {
        return None;
    }
    let mut best: Option<MeshHit> = None;
    for o in state.of_class(ObjClass::SceneObject) {
        if !o.flags.visible {
            continue;
        }
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib.cached(&d.reference) else { continue };
        let Some(bvh) = &lo.bvh else { continue };
        let world = o.world();
        let inv = Mat4::invert_43(&world);
        let reach = best.as_ref().map_or(range, |b| b.t);
        let mut ray = Ray::new(inv.transform_tiny(start), inv.transform_dir(dir), reach);
        ray.cull_back = true;
        let Some(hit) = bvh.ray_nearest(&ray) else { continue };
        if hit.t >= reach {
            continue;
        }
        let Some(mesh) = mesh_of_triangle(&lo, hit.tri) else { continue };
        let mesh_name = lo.object.meshes.get(mesh).map(|m| m.name.to_string()).unwrap_or_default();
        best = Some(MeshHit { object: o.clone(), mesh, mesh_name, t: hit.t });
    }
    best
}

// ---- the frustum ------------------------------------------------------------------------------

/// Some part of the polygon is inside every plane (a point is inside
/// when `n . p + w >= 0`). The polygon is clipped plane by plane and fails once fewer than three
/// points are left.
#[must_use]
pub fn poly_inside(planes: &[Vec4], poly: &[Vec3]) -> bool {
    let mut src: Vec<Vec3> = poly.to_vec();
    for p in planes {
        if src.len() < 3 {
            return false;
        }
        let n = p.truncate();
        let cls: Vec<f32> = src.iter().map(|v| n.dot(*v) + p.w).collect();
        let mut dst: Vec<Vec3> = Vec::with_capacity(src.len() + 2);
        for j in 0..src.len() {
            let k = (j + 1) % src.len();
            let (a, b) = (src[j], src[k]);
            if similar(a, b, EPS_S) {
                continue;
            }
            let (ca, cb) = (cls[j], cls[k]);
            if ca >= 0.0 {
                dst.push(a);
                if cb < 0.0 {
                    let d = b - a;
                    let denum = n.dot(d);
                    if denum != 0.0 {
                        dst.push(a + d * (-ca / denum));
                    }
                }
            } else if cb >= 0.0 {
                let d = b - a;
                let denum = n.dot(d);
                if denum != 0.0 {
                    dst.push(a + d * (-ca / denum));
                }
            }
        }
        src = dst;
    }
    src.len() >= 3
}

/// Whether the frustum picks the object, as far as the box tells: the object's world box meets every plane.
#[must_use]
pub fn box_in_frustum(planes: &[Vec4], b: &Aabb) -> bool {
    b.is_valid()
        && planes.iter().all(|p| {
            let n = p.truncate();
            let far = Vec3::new(
                if n.x >= 0.0 { b.max.x } else { b.min.x },
                if n.y >= 0.0 { b.max.y } else { b.min.y },
                if n.z >= 0.0 { b.max.z } else { b.min.z },
            );
            n.dot(far) + p.w >= 0.0
        })
}

/// A visible mesh with a face inside the frustum.
#[must_use]
pub fn mesh_in_frustum(lo: &LibraryObject, mesh: usize, world: &Mat4, planes: &[Vec4]) -> bool {
    let Some(m) = lo.object.meshes.get(mesh) else { return false };
    if !m.flags.contains(MeshFlags::VISIBLE) {
        return false;
    }
    mesh_world_faces(lo, mesh, world).iter().any(|f| poly_inside(planes, f))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_random_numbers_are_the_engines() {
        // MSVC rand() with srand(1): 41, 18467, 6334
        let mut r = CRandom::new(1);
        assert_eq!([r.rand_i(), r.rand_i(), r.rand_i()], [41, 18467, 6334]);
    }

    #[test]
    fn a_plane_fit_finds_the_floor() {
        let pts = [
            Vec3::new(0.0, 2.0, 0.0),
            Vec3::new(1.0, 2.0, 0.0),
            Vec3::new(1.0, 2.0, 1.0),
            Vec3::new(0.0, 2.0, 1.0),
        ];
        let (o, n) = plane_fit(&pts);
        assert!((o - Vec3::new(0.5, 2.0, 0.5)).length() < 1e-5);
        assert!((n.y.abs() - 1.0).abs() < 1e-5, "{n:?}");
    }

    #[test]
    fn a_hull_drops_inner_and_collinear_points() {
        let pts = [
            Vec2::new(0.0, 0.0),
            Vec2::new(1.0, 0.0),
            Vec2::new(2.0, 0.0),
            Vec2::new(2.0, 2.0),
            Vec2::new(0.0, 2.0),
            Vec2::new(1.0, 1.0),
            Vec2::new(0.0, 0.0),
        ];
        let mut h = convex_hull(&pts);
        h.sort_unstable();
        assert_eq!(h, vec![0, 2, 3, 4]);
    }

    #[test]
    fn a_triangle_across_a_plane_is_inside() {
        let planes = [Vec4::new(1.0, 0.0, 0.0, 0.0)];
        assert!(poly_inside(
            &planes,
            &[Vec3::new(-1.0, 0.0, 0.0), Vec3::new(1.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0)]
        ));
        assert!(!poly_inside(
            &planes,
            &[Vec3::new(-3.0, 0.0, 0.0), Vec3::new(-1.0, 0.0, 0.0), Vec3::new(-2.0, 1.0, 0.0)]
        ));
    }
}
