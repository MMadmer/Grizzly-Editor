//! The transform tools' maths (`Gizmo`, `TUI_CustomControl::Select*`, `MoveProcess`,
//! `ScaleProcess`, `RotateProcess`), pure: where the gizmo stands, which of its parts the mouse
//! is over, and what a drag does to the objects it holds.

use glam::{Vec2, Vec3};
use gz_fmt_object::Mat4 as XMat;
use gz_render::{Aabb, Ray};
use gz_scene::ObjClass;

/// `Gizmo::EType`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GizmoType {
    #[default]
    None,
    Move,
    Rotate,
    Scale,
}

/// `Gizmo::EStatus`: the part under the mouse, or the one a drag holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    None,
    X,
    Y,
    Z,
    /// The scale gizmo's centre.
    Xyz,
}

/// What a class's tool can do with its objects (`IsSupportMove` / `Rotate` / `Scale`).
#[must_use]
pub fn supports(class: ObjClass) -> [bool; 3] {
    match class {
        ObjClass::Particles => [true, false, false],
        ObjClass::Way => [true, false, false],
        ObjClass::AiMap => [true, true, false],
        ObjClass::Portal | ObjClass::Sector | ObjClass::Wallmark | ObjClass::DetailObjects => {
            [false, false, false]
        }
        _ => [true, true, true],
    }
}

/// What the gizmo may do with the selection: what every selected object allows (the AI map's
/// nodes in its mode).
#[must_use]
pub fn supports_of(state: &gz_scene::SceneState) -> [bool; 3] {
    if state.tools.mode == gz_scene::SceneMode::AiMap {
        return supports(ObjClass::AiMap);
    }
    if state.selection.is_empty() {
        return [true; 3];
    }
    let c = state.selection_caps();
    [c.moves, c.rotates, c.scales]
}

/// `snapto`.
#[must_use]
pub fn snapto(value: f32, snap: f32) -> f32 {
    if snap <= 0.0 {
        return value;
    }
    ((value + snap * 0.5) / snap).floor() * snap
}

/// `CHECK_SNAP(R, A, C)`: an amount kept back until it makes a whole step.
#[must_use]
pub fn check_snap(remainder: &mut f32, amount: f32, step: f32) -> f32 {
    *remainder += amount;
    if remainder.abs() >= step {
        let a = snapto(*remainder, step);
        *remainder = 0.0;
        a
    } else {
        0.0
    }
}

/// Where the gizmo of a selection stands (`Gizmo::OnFrame`): one object's pivot and rotation,
/// or the centre of the selection's box with no rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub position: Vec3,
    /// The rotation's axes as rows (`m_RotateMatrix`).
    pub rotation: XMat,
    pub count: usize,
}

impl Anchor {
    #[must_use]
    pub fn of(points: &[(Vec3, Vec3, Option<Aabb>)]) -> Option<Anchor> {
        let (first_pos, first_rot, _) = *points.first()?;
        if points.len() == 1 {
            return Some(Anchor {
                position: first_pos,
                rotation: gz_scene::math::rotation_matrix(first_rot),
                count: 1,
            });
        }
        let mut b = Aabb::EMPTY;
        for (p, _, bb) in points {
            match bb {
                Some(bb) => b = b.union(bb),
                None => b.extend(*p),
            }
        }
        let position = if b.is_empty() { first_pos } else { b.center() };
        Some(Anchor { position, rotation: XMat::IDENTITY, count: points.len() })
    }
}

/// The gizmo's place on screen: 5 m along the ray through the anchor's pixel, so it keeps one
/// size whatever the distance (`m_ScreenPosition`). `None` behind the camera.
#[must_use]
pub fn screen_position(camera: &gz_render::Camera, size: [u32; 2], anchor: Vec3) -> Option<Vec3> {
    // in front of the camera, at any distance
    if (anchor - camera.position).dot(camera.forward) <= 0.0 {
        return None;
    }
    let vp = camera.view_proj(size);
    let px = gz_render::project(&vp, size, anchor)?;
    let ray = gz_render::pick_ray(camera, size, Vec2::new(px.x, px.y))?;
    Some(ray.origin + ray.dir * 5.0)
}

fn axis_box(at: Vec3, axis: usize) -> Aabb {
    let mut min = Vec3::splat(-0.03);
    let mut max = Vec3::splat(0.03);
    min[axis] = 0.0;
    max[axis] = 1.0;
    Aabb::new(at + min, at + max)
}

fn hits(ray: &Ray, b: &Aabb) -> bool {
    ray.intersect_aabb(b).is_some()
}

/// `Fsphere::intersect2`: the nearest hit in front.
fn ray_sphere(ray: &Ray, c: Vec3, r: f32) -> Option<f32> {
    let oc = ray.origin - c;
    let b = oc.dot(ray.dir);
    let cc = oc.length_squared() - r * r;
    let disc = b * b - cc;
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    let t0 = -b - s;
    let t1 = -b + s;
    if t0 >= 0.0 {
        Some(t0)
    } else if t1 >= 0.0 {
        Some(t1)
    } else {
        None
    }
}

/// `Fcylinder::intersect`: a capped cylinder round `axis` through `c`, `h` high.
fn ray_cylinder(ray: &Ray, c: Vec3, axis: Vec3, h: f32, r: f32) -> Option<f32> {
    let a = axis.normalize_or_zero();
    if a == Vec3::ZERO {
        return None;
    }
    let half = h * 0.5;
    let mut best: Option<f32> = None;
    let mut keep = |t: f32| {
        if t >= 0.0 && best.is_none_or(|b| t < b) {
            best = Some(t);
        }
    };
    // the side
    let oc = ray.origin - c;
    let d_perp = ray.dir - a * ray.dir.dot(a);
    let o_perp = oc - a * oc.dot(a);
    let qa = d_perp.length_squared();
    if qa > 1e-12 {
        let qb = 2.0 * d_perp.dot(o_perp);
        let qc = o_perp.length_squared() - r * r;
        let disc = qb * qb - 4.0 * qa * qc;
        if disc >= 0.0 {
            let s = disc.sqrt();
            for t in [(-qb - s) / (2.0 * qa), (-qb + s) / (2.0 * qa)] {
                let along = (oc + ray.dir * t).dot(a);
                if along.abs() <= half {
                    keep(t);
                }
            }
        }
    }
    // the caps
    let dn = ray.dir.dot(a);
    if dn.abs() > 1e-9 {
        for side in [-half, half] {
            let t = (side - oc.dot(a)) / dn;
            let p = oc + ray.dir * t;
            if (p - a * p.dot(a)).length_squared() <= r * r {
                keep(t);
            }
        }
    }
    best
}

/// The rotate gizmo's three axes as the rings face them (`m_RotateMatrix`, turned about).
#[must_use]
pub fn rotate_axes(rotation: &XMat) -> [Vec3; 3] {
    let t = |v: Vec3| -rotation.transform_dir(v).normalize_or_zero();
    [t(Vec3::X), t(Vec3::Y), t(Vec3::Z)]
}

/// Which part of the gizmo the ray is over (`Gizmo::OnFrame`). `aimap` leaves out X and Z of
/// the move gizmo and Y of the rotate one, as the AI map's tool does.
#[must_use]
pub fn hover(kind: GizmoType, at: Vec3, rotation: &XMat, ray: &Ray, aimap: bool) -> Status {
    match kind {
        GizmoType::None => Status::None,
        GizmoType::Move => {
            if !aimap && hits(ray, &axis_box(at, 0)) {
                Status::X
            } else if hits(ray, &axis_box(at, 1)) {
                Status::Y
            } else if !aimap && hits(ray, &axis_box(at, 2)) {
                Status::Z
            } else {
                Status::None
            }
        }
        GizmoType::Scale => {
            if hits(ray, &Aabb::new(at - Vec3::splat(0.06), at + Vec3::splat(0.06))) {
                Status::Xyz
            } else if hits(ray, &axis_box(at, 0)) {
                Status::X
            } else if hits(ray, &axis_box(at, 1)) {
                Status::Y
            } else if hits(ray, &axis_box(at, 2)) {
                Status::Z
            } else {
                Status::None
            }
        }
        GizmoType::Rotate => {
            let [x, y, z] = rotate_axes(rotation);
            let mut dist = ray_sphere(ray, at, 0.8).unwrap_or(f32::MAX);
            let mut status = Status::None;
            for (axis, s, on) in [(x, Status::X, true), (y, Status::Y, !aimap), (z, Status::Z, true)] {
                if !on {
                    continue;
                }
                if let Some(t) = ray_cylinder(ray, at, axis, 0.05, 1.0)
                    && dist > t
                {
                    dist = t;
                    status = s;
                }
            }
            status
        }
    }
}

/// `Fplane::intersectRayPoint`.
fn ray_plane(ray: &Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let dn = ray.dir.dot(normal);
    if dn.abs() < 1e-9 {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / dn;
    if t < 0.0 {
        return None;
    }
    Some(ray.origin + ray.dir * t)
}

/// A move or scale drag: the plane the C++ intersects for a part, and the axis vector the
/// delta is multiplied by.
fn drag_plane(status: Status, camera_dir: Vec3) -> Option<(Vec3, Vec3)> {
    match status {
        Status::X => Some((Vec3::Y, Vec3::X)),
        Status::Z => Some((Vec3::Y, Vec3::Z)),
        Status::Y => {
            let mut n = -camera_dir;
            n.y = 0.0;
            Some((n.normalize_or_zero(), Vec3::Y))
        }
        Status::Xyz => Some(((-camera_dir).normalize_or_zero(), Vec3::ONE)),
        Status::None => None,
    }
}

/// The world delta of a move drag from the press's ray to this one, and the axis it moves
/// along (`SelectProcess` for `EType::Move`).
#[must_use]
pub fn move_delta(
    status: Status,
    start: Vec3,
    camera_dir: Vec3,
    ray0: &Ray,
    ray: &Ray,
) -> Option<(Vec3, Vec3)> {
    if status == Status::Xyz {
        return None;
    }
    let (normal, vector) = drag_plane(status, camera_dir)?;
    let a = ray_plane(ray0, start, normal)?;
    let b = ray_plane(ray, start, normal)?;
    Some((b - a, vector))
}

/// `MoveProcess` for one object: its position at the press moved by the delta, each moved axis
/// on the grid when the grid snap is on.
#[must_use]
pub fn moved(saved: Vec3, delta: Vec3, vector: Vec3, snap: Option<f32>) -> Vec3 {
    let mut p = saved + delta * vector;
    if let Some(step) = snap {
        for a in 0..3 {
            if vector[a].abs() > gz_scene::math::EPS {
                p[a] = snapto(p[a], step);
            }
        }
    }
    p
}

/// The delta of a scale drag (`SelectProcess` for `EType::Scale`); the centre takes the sum
/// of the view-space delta's components, times 0.7, on every axis.
#[must_use]
pub fn scale_delta(
    status: Status,
    start: Vec3,
    camera: &gz_render::Camera,
    ray0: &Ray,
    ray: &Ray,
) -> Option<(Vec3, Vec3)> {
    let (normal, vector) = drag_plane(status, camera.forward)?;
    let a = ray_plane(ray0, start, normal)?;
    let b = ray_plane(ray, start, normal)?;
    if status == Status::Xyz {
        let view = camera.view_matrix();
        let (va, vb) = (view.transform_point3(a), view.transform_point3(b));
        let d = vb - va;
        return Some((Vec3::splat((d.x + d.y + d.z) * 0.7), vector));
    }
    Some((b - a, vector))
}

/// `ScaleProcess` for one object: a glow takes the dragged component on every axis; steps when
/// the scale snap is on; never zero.
#[must_use]
pub fn scaled(
    saved: Vec3,
    mut delta: Vec3,
    mut vector: Vec3,
    status: Status,
    glow: bool,
    snap: Option<f32>,
) -> Vec3 {
    if glow {
        match status {
            Status::X => delta = Vec3::splat(delta.x),
            Status::Y => delta = Vec3::splat(delta.y),
            Status::Z => delta = Vec3::splat(delta.z),
            _ => {}
        }
        vector = Vec3::ONE;
    }
    let mut s = saved + delta * vector;
    if let Some(step) = snap {
        for a in 0..3 {
            if vector[a].abs() > gz_scene::math::EPS {
                s[a] = snapto(s[a], step);
            }
        }
    }
    for a in 0..3 {
        if s[a].abs() < gz_scene::math::EPS_S {
            s[a] = gz_scene::math::EPS;
        }
    }
    s
}

/// The angle one mouse move turns by (`SelectProcess` for `EType::Rotate`): `dx`, `dy` in
/// pixels, `sr` the tools' rotation sensitivity; `step` the angle snap in degrees when on.
#[must_use]
pub fn rotate_amount(
    status: Status,
    dx: f32,
    dy: f32,
    sr: f32,
    step: Option<f32>,
    remainder: &mut f32,
) -> f32 {
    let amount = sr * dx + -sr * dy;
    let mut f = match status {
        Status::X => -amount,
        Status::Y | Status::Z => amount,
        _ => 0.0,
    };
    if let Some(step) = step {
        f *= (step / 22.0).max(1.0);
    }
    f = -f;
    if let Some(step) = step {
        f = check_snap(remainder, f, step.to_radians());
    }
    f
}

/// `Fmatrix::rotation(axis, angle)`.
#[must_use]
pub fn axis_rotation(axis: Vec3, angle: f32) -> XMat {
    let (s, c) = angle.sin_cos();
    let (x, y, z) = (axis.x, axis.y, axis.z);
    XMat([
        [x * x + (1.0 - x * x) * c, x * y * (1.0 - c) + z * s, x * z * (1.0 - c) - y * s, 0.0],
        [x * y * (1.0 - c) - z * s, y * y + (1.0 - y * y) * c, y * z * (1.0 - c) + x * s, 0.0],
        [x * z * (1.0 - c) + y * s, y * z * (1.0 - c) - x * s, z * z + (1.0 - z * z) * c, 0.0],
        [0.0, 0.0, 0.0, 1.0],
    ])
}

/// `RotateLocal(axis, angle)` and the wrap into a turn: the object's rotation after it.
#[must_use]
pub fn rotate_local(rotation: Vec3, axis: Vec3, angle: f32) -> Vec3 {
    let m = XMat::mul_43(&gz_scene::math::rotation_matrix(rotation), &axis_rotation(axis, angle));
    let r = gz_scene::math::get_xyz(&m);
    let turn = std::f32::consts::TAU;
    Vec3::new(r.x % turn, r.y % turn, r.z % turn)
}

/// The axis a rotate drag turns about.
#[must_use]
pub fn rotate_vector(status: Status) -> Vec3 {
    match status {
        Status::X => Vec3::X,
        Status::Y => Vec3::Y,
        Status::Z => Vec3::Z,
        _ => Vec3::ZERO,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(o: Vec3, d: Vec3) -> Ray {
        Ray { origin: o, dir: d.normalize() }
    }

    #[test]
    fn snapping() {
        assert_eq!(snapto(0.26, 0.5), 0.5);
        assert_eq!(snapto(0.24, 0.5), 0.0);
        assert_eq!(snapto(-0.26, 0.5), -0.5);
        assert_eq!(snapto(3.0, 0.0), 3.0);
        let mut r = 0.0;
        assert_eq!(check_snap(&mut r, 0.3, 1.0), 0.0);
        assert_eq!(check_snap(&mut r, 0.8, 1.0), 1.0);
        assert_eq!(r, 0.0);
    }

    #[test]
    fn the_move_gizmo_is_picked_by_its_shafts() {
        let at = Vec3::new(0.0, 0.0, 5.0);
        let rot = XMat::IDENTITY;
        let x = ray(Vec3::new(0.5, 0.0, 0.0), Vec3::Z);
        assert_eq!(hover(GizmoType::Move, at, &rot, &x, false), Status::X);
        assert_eq!(hover(GizmoType::Move, at, &rot, &x, true), Status::None);
        let y = ray(Vec3::new(0.0, 0.5, 0.0), Vec3::Z);
        assert_eq!(hover(GizmoType::Move, at, &rot, &y, false), Status::Y);
        let miss = ray(Vec3::new(0.5, 0.5, 0.0), Vec3::Z);
        assert_eq!(hover(GizmoType::Move, at, &rot, &miss, false), Status::None);
        assert_eq!(hover(GizmoType::Scale, at, &rot, &ray(Vec3::ZERO, Vec3::Z), false), Status::Xyz);
    }

    #[test]
    fn a_move_drag_follows_the_ground() {
        let r0 = ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -1.0, 1.0));
        let r1 = ray(Vec3::new(2.0, 10.0, 0.0), Vec3::new(0.0, -1.0, 1.0));
        let (d, v) = move_delta(Status::X, Vec3::ZERO, Vec3::Z, &r0, &r1).unwrap_or((Vec3::ZERO, Vec3::ZERO));
        assert!((d.x - 2.0).abs() < 1e-4);
        assert_eq!(v, Vec3::X);
        assert_eq!(moved(Vec3::new(1.0, 1.0, 1.0), d, v, Some(0.5)), Vec3::new(3.0, 1.0, 1.0));
    }

    #[test]
    fn rotations_turn_about_the_local_axis() {
        let r = rotate_local(Vec3::ZERO, Vec3::Y, 0.5);
        assert!((r.y.abs() - 0.5).abs() < 1e-4, "{r}");
        let mut rem = 0.0;
        let step = Some(45.0);
        let a = rotate_amount(Status::Y, 10.0, 0.0, 0.01, step, &mut rem);
        assert_eq!(a, 0.0);
        assert!(rem != 0.0);
        assert_eq!(
            scaled(Vec3::ONE, Vec3::new(-1.0, 0.0, 0.0), Vec3::X, Status::X, false, None).x,
            gz_scene::math::EPS
        );
        assert_eq!(
            scaled(Vec3::ONE, Vec3::new(0.5, 0.0, 0.0), Vec3::X, Status::X, true, None),
            Vec3::splat(1.5)
        );
    }

    #[test]
    fn a_selection_stands_at_its_centre() {
        let one = Anchor::of(&[(Vec3::ONE, Vec3::ZERO, None)]).map(|a| a.position);
        assert_eq!(one, Some(Vec3::ONE));
        let two = Anchor::of(&[(Vec3::ZERO, Vec3::ZERO, None), (Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, None)]);
        assert_eq!(two.map(|a| a.position), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert!(Anchor::of(&[]).is_none());
    }
}
