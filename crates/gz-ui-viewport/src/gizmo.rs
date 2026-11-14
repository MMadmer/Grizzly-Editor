//! The transform gizmo, pure: where it stands, its handles on the screen, which handle the
//! pointer is over, and what a drag of each handle does.
//!
//! The gizmo is drawn a fixed distance along the ray through the selection's pivot, so it keeps
//! one size on the screen whatever the distance; its handles are hit-tested on the screen, in
//! points, the way they are drawn. A drag works on the real pivot: an axis follows the closest
//! point of the axis line to the pointer's ray, a plane the ray's hit on the plane, a ring the
//! angle swept round the pivot on the ring's plane.

use glam::{Mat3, Quat, Vec2, Vec3};
use gz_fmt_object::Mat4 as XMat;
use gz_render::{Aabb, Ray};

/// The transform tool: Select draws no gizmo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum GizmoType {
    #[default]
    None,
    Move,
    Rotate,
    Scale,
}

impl GizmoType {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "select",
            Self::Move => "move",
            Self::Rotate => "rotate",
            Self::Scale => "scale",
        }
    }

    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        match text.trim().to_ascii_lowercase().as_str() {
            "select" | "none" => Some(Self::None),
            "move" => Some(Self::Move),
            "rotate" => Some(Self::Rotate),
            "scale" => Some(Self::Scale),
            _ => None,
        }
    }
}

/// Which axes the gizmo's handles follow: the world's, or the selection's own (its first
/// object's rotation). Scaling always works along an object's own axes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Space {
    #[default]
    World,
    Local,
}

impl Space {
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::World => "world",
            Self::Local => "local",
        }
    }
}

/// The handle under the pointer, or the one a drag holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Status {
    #[default]
    None,
    X,
    Y,
    Z,
    /// A plane handle: the two axes it moves (or scales) along.
    XY,
    XZ,
    YZ,
    /// Move: the centre, free in the view's plane. Rotate: the outer ring, about the view.
    Screen,
    /// Scale: the centre, every axis at once.
    Xyz,
}

impl Status {
    /// Which of the gizmo's three axes the handle works along.
    #[must_use]
    pub fn axes(self) -> [bool; 3] {
        match self {
            Self::X => [true, false, false],
            Self::Y => [false, true, false],
            Self::Z => [false, false, true],
            Self::XY => [true, true, false],
            Self::XZ => [true, false, true],
            Self::YZ => [false, true, true],
            Self::Xyz => [true, true, true],
            Self::None | Self::Screen => [false, false, false],
        }
    }

    /// The one axis of an axis handle or a ring.
    #[must_use]
    pub fn axis(self) -> Option<usize> {
        match self {
            Self::X => Some(0),
            Self::Y => Some(1),
            Self::Z => Some(2),
            _ => None,
        }
    }

    /// The axis a plane handle's plane faces.
    #[must_use]
    pub fn plane_normal(self) -> Option<usize> {
        match self {
            Self::YZ => Some(0),
            Self::XZ => Some(1),
            Self::XY => Some(2),
            _ => None,
        }
    }

    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::X => "x",
            Self::Y => "y",
            Self::Z => "z",
            Self::XY => "xy",
            Self::XZ => "xz",
            Self::YZ => "yz",
            Self::Screen => "screen",
            Self::Xyz => "xyz",
        }
    }
}

/// What the gizmo may do with the selection: what every selected object allows (the AI map's
/// nodes move and turn in their mode).
#[must_use]
pub fn supports_of(state: &gz_scene::SceneState) -> [bool; 3] {
    if state.tools.mode == gz_scene::SceneMode::AiMap {
        return [true, true, false];
    }
    if state.selection.is_empty() {
        return [true; 3];
    }
    let c = state.selection_caps();
    [c.moves, c.rotates, c.scales]
}

/// `value` on the nearest step of `snap` (a step of 0 or less leaves it).
#[must_use]
pub fn snapto(value: f32, snap: f32) -> f32 {
    if snap <= 0.0 {
        return value;
    }
    ((value + snap * 0.5) / snap).floor() * snap
}

/// Where the gizmo of a selection stands: its pivot (one object's, or the middle of the
/// selection's box) and the first object's rotation, which Local space follows.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Anchor {
    pub position: Vec3,
    /// The rotation's axes as rows.
    pub rotation: XMat,
    pub count: usize,
}

impl Anchor {
    #[must_use]
    pub fn of(points: &[(Vec3, Vec3, Option<Aabb>)]) -> Option<Anchor> {
        let (first_pos, first_rot, _) = *points.first()?;
        let rotation = gz_scene::math::rotation_matrix(first_rot);
        if points.len() == 1 {
            return Some(Anchor { position: first_pos, rotation, count: 1 });
        }
        let mut b = Aabb::EMPTY;
        for (p, _, bb) in points {
            match bb {
                Some(bb) => b = b.union(bb),
                None => b.extend(*p),
            }
        }
        let position = if b.is_empty() { first_pos } else { b.center() };
        Some(Anchor { position, rotation, count: points.len() })
    }

    /// The gizmo's three axes in the world for a space.
    #[must_use]
    pub fn axes(&self, space: Space) -> [Vec3; 3] {
        match space {
            Space::World => [Vec3::X, Vec3::Y, Vec3::Z],
            Space::Local => {
                let r = |i: usize| self.rotation.row(i).normalize_or(Vec3::AXES[i]);
                [r(0), r(1), r(2)]
            }
        }
    }
}

/// How far along the ray through the pivot's pixel the gizmo is drawn: its handles are one unit
/// long there, so it keeps one size on the screen.
pub const DISTANCE: f32 = 5.0;

/// The gizmo's place for drawing: [`DISTANCE`] along the ray through the anchor's pixel. `None`
/// behind the camera.
#[must_use]
pub fn screen_position(camera: &gz_render::Camera, size: [u32; 2], anchor: Vec3) -> Option<Vec3> {
    if (anchor - camera.position).dot(camera.forward) <= 0.0 {
        return None;
    }
    let vp = camera.view_proj(size);
    let px = gz_render::project(&vp, size, anchor)?;
    let ray = gz_render::pick_ray(camera, size, Vec2::new(px.x, px.y))?;
    Some(ray.origin + ray.dir * DISTANCE)
}

// ---- the handles on the screen ------------------------------------------------------------------

/// Where an arrow's shaft starts (clear of the centre) and its head starts.
const SHAFT_FROM: f32 = 0.2;
const HEAD_FROM: f32 = 0.8;
/// A plane handle's square, along both its axes.
const PLANE_FROM: f32 = 0.24;
const PLANE_TO: f32 = 0.44;
/// The rings' radius, and the outer ring's that turns about the view.
pub const RING: f32 = 0.85;
pub const SCREEN_RING: f32 = 1.05;
/// The centre handle's half side, points.
pub const CENTRE: f32 = 7.0;
/// How near a line the pointer has to be to take it, points.
pub const REACH: f32 = 6.0;

/// The gizmo as the screen shows it, in points: what is drawn and what the pointer is tested
/// against are the same shapes.
#[derive(Clone, Debug, Default)]
pub struct Shapes {
    pub centre: Vec2,
    /// Axis handles: the line from the shaft's start to the tip, and where the head starts.
    pub axes: Vec<(Status, Vec2, Vec2, Vec2)>,
    /// Plane handles: the four corners.
    pub planes: Vec<(Status, [Vec2; 4])>,
    /// Rings: the closed polyline.
    pub rings: Vec<(Status, Vec<Vec2>)>,
    /// The outer ring of Rotate: its centre is `centre`, its radius in points.
    pub screen_ring: Option<f32>,
    /// Scale's centre handle and Move's: a square at the centre.
    pub centre_handle: bool,
}

/// The handles of a gizmo of `kind` drawn at `at` (see [`screen_position`]) along `axes`, with
/// `proj` taking a world point to the screen (points). `aimap` keeps what the AI map's nodes take:
/// Y of Move, X and Z of Rotate.
#[must_use]
pub fn shapes(
    kind: GizmoType,
    at: Vec3,
    axes: [Vec3; 3],
    camera_forward: Vec3,
    proj: &dyn Fn(Vec3) -> Option<Vec2>,
    aimap: bool,
) -> Option<Shapes> {
    let centre = proj(at)?;
    let mut s = Shapes { centre, ..Shapes::default() };
    let names = [Status::X, Status::Y, Status::Z];
    let planes = [(Status::XY, 0, 1), (Status::XZ, 0, 2), (Status::YZ, 1, 2)];
    match kind {
        GizmoType::None => return None,
        GizmoType::Move | GizmoType::Scale => {
            for (i, axis) in axes.iter().enumerate() {
                if aimap && i != 1 {
                    continue;
                }
                let (Some(a), Some(h), Some(b)) =
                    (proj(at + *axis * SHAFT_FROM), proj(at + *axis * HEAD_FROM), proj(at + *axis))
                else {
                    continue;
                };
                s.axes.push((names[i], a, b, h));
            }
            if !aimap {
                for (st, i, j) in planes {
                    let (u, v) = (axes[i], axes[j]);
                    let corners = [
                        at + u * PLANE_FROM + v * PLANE_FROM,
                        at + u * PLANE_TO + v * PLANE_FROM,
                        at + u * PLANE_TO + v * PLANE_TO,
                        at + u * PLANE_FROM + v * PLANE_TO,
                    ];
                    let pts: Option<Vec<Vec2>> = corners.iter().map(|c| proj(*c)).collect();
                    if let Some([a, b, c, d]) = pts.as_deref().map(|p| [p[0], p[1], p[2], p[3]]) {
                        s.planes.push((st, [a, b, c, d]));
                    }
                }
            }
            s.centre_handle = !aimap;
        }
        GizmoType::Rotate => {
            for (i, axis) in axes.iter().enumerate() {
                if aimap && i == 1 {
                    continue;
                }
                let u = axis.any_orthonormal_vector();
                let w = axis.cross(u);
                let ring: Vec<Vec2> = (0..=64)
                    .filter_map(|k| {
                        let t = k as f32 / 64.0 * std::f32::consts::TAU;
                        proj(at + (u * t.cos() + w * t.sin()) * RING)
                    })
                    .collect();
                if ring.len() > 2 {
                    s.rings.push((names[i], ring));
                }
            }
            if !aimap {
                // the outer ring's radius: a point of the view's plane that far out
                let side = camera_forward.any_orthonormal_vector();
                s.screen_ring = proj(at + side * SCREEN_RING).map(|p| (p - centre).length());
            }
        }
    }
    Some(s)
}

/// The distance from `p` to the segment `a`..`b`.
#[must_use]
pub fn segment_distance(p: Vec2, a: Vec2, b: Vec2) -> f32 {
    let ab = b - a;
    let len = ab.length_squared();
    let t = if len > 1e-9 { ((p - a).dot(ab) / len).clamp(0.0, 1.0) } else { 0.0 };
    (a + ab * t - p).length()
}

fn inside(p: Vec2, quad: &[Vec2; 4]) -> bool {
    // a convex quad: the point is on one side of every edge
    let mut sign = 0.0f32;
    for i in 0..4 {
        let (a, b) = (quad[i], quad[(i + 1) % 4]);
        let c = (b - a).perp_dot(p - a);
        if c.abs() < 1e-9 {
            continue;
        }
        if sign == 0.0 {
            sign = c.signum();
        } else if c.signum() != sign {
            return false;
        }
    }
    sign != 0.0
}

/// Which handle `p` (points) is over: the centre first, then the planes (small squares that an
/// axis seen end-on may cross), then the axes and the rings (the nearest within [`REACH`]), then
/// Rotate's outer ring.
#[must_use]
pub fn hit(kind: GizmoType, s: &Shapes, p: Vec2) -> Status {
    if s.centre_handle && (p - s.centre).abs().max_element() <= CENTRE + 1.0 {
        return if kind == GizmoType::Scale { Status::Xyz } else { Status::Screen };
    }
    if let Some((st, _)) = s.planes.iter().find(|(_, q)| inside(p, q)) {
        return *st;
    }
    let mut best: Option<(Status, f32)> = None;
    let mut take = |st: Status, d: f32| {
        if d <= REACH && best.is_none_or(|(_, b)| d < b) {
            best = Some((st, d));
        }
    };
    for (st, a, b, _) in &s.axes {
        take(*st, segment_distance(p, *a, *b));
    }
    for (st, ring) in &s.rings {
        let d = ring.windows(2).map(|w| segment_distance(p, w[0], w[1])).fold(f32::MAX, f32::min);
        take(*st, d);
    }
    if let Some((st, _)) = best {
        return st;
    }
    if let Some(r) = s.screen_ring
        && ((p - s.centre).length() - r).abs() <= REACH
    {
        return Status::Screen;
    }
    Status::None
}

// ---- what a drag does ---------------------------------------------------------------------------

/// The ray's hit on the plane through `point` facing `normal`, in front of the ray.
#[must_use]
pub fn ray_plane(ray: &Ray, point: Vec3, normal: Vec3) -> Option<Vec3> {
    let dn = ray.dir.dot(normal);
    if dn.abs() < 1e-6 {
        return None;
    }
    let t = (point - ray.origin).dot(normal) / dn;
    (t >= 0.0).then(|| ray.origin + ray.dir * t)
}

/// Along the line through `origin` in direction `axis`, the parameter of its point nearest the
/// ray. `None` when the ray runs along the line.
#[must_use]
pub fn axis_param(origin: Vec3, axis: Vec3, ray: &Ray) -> Option<f32> {
    let w = origin - ray.origin;
    let (a, b, c) = (axis.dot(axis), axis.dot(ray.dir), ray.dir.dot(ray.dir));
    let (d, e) = (axis.dot(w), ray.dir.dot(w));
    let den = a * c - b * b;
    if den.abs() < 1e-4 {
        return None;
    }
    Some((b * e - c * d) / den)
}

/// The world delta of a move drag from the press's ray to this one: along an axis, in a plane, or
/// in the view's plane through the pivot.
#[must_use]
pub fn move_delta(
    status: Status,
    pivot: Vec3,
    axes: [Vec3; 3],
    camera_forward: Vec3,
    ray0: &Ray,
    ray: &Ray,
) -> Option<Vec3> {
    if let Some(i) = status.axis() {
        let axis = axes[i];
        let (t0, t1) = (axis_param(pivot, axis, ray0)?, axis_param(pivot, axis, ray)?);
        return Some(axis * (t1 - t0));
    }
    let normal = match status {
        Status::Screen => camera_forward,
        _ => axes[status.plane_normal()?],
    };
    let (a, b) = (ray_plane(ray0, pivot, normal)?, ray_plane(ray, pivot, normal)?);
    Some(b - a)
}

/// Where an object that stood at `start` goes with a move of `delta`. With a grid step: World space
/// puts each moved world axis on the grid; Local space moves in whole steps along the moved axes.
#[must_use]
pub fn moved(
    start: Vec3,
    delta: Vec3,
    status: Status,
    axes: [Vec3; 3],
    space: Space,
    step: Option<f32>,
) -> Vec3 {
    let Some(step) = step.filter(|s| *s > 0.0) else { return start + delta };
    let used = status.axes();
    match space {
        Space::World => {
            let mut p = start + delta;
            for a in 0..3 {
                // the screen handle moves every axis the delta has
                let moves = if status == Status::Screen { delta[a].abs() > 1e-6 } else { used[a] };
                if moves {
                    p[a] = snapto(p[a], step);
                }
            }
            p
        }
        Space::Local => {
            let mut out = start;
            for (a, axis) in axes.iter().enumerate() {
                let along = delta.dot(*axis);
                if used[a] || status == Status::Screen {
                    out += *axis * snapto(along, step);
                }
            }
            out
        }
    }
}

/// The angle a ring drag swept round `pivot` about `axis`, from the press's ray to this one,
/// right-handed about the axis. `None` when the ring is seen edge-on (its plane gives no angle).
#[must_use]
pub fn ring_angle(axis: Vec3, pivot: Vec3, ray0: &Ray, ray: &Ray) -> Option<f32> {
    if ray.dir.dot(axis).abs() < 0.12 || ray0.dir.dot(axis).abs() < 0.12 {
        return None;
    }
    let (a, b) = (ray_plane(ray0, pivot, axis)? - pivot, ray_plane(ray, pivot, axis)? - pivot);
    if a.length_squared() < 1e-8 || b.length_squared() < 1e-8 {
        return None;
    }
    Some(axis.dot(a.cross(b)).atan2(a.dot(b)))
}

/// The angle of a ring seen edge-on: the pointer's travel on the screen across the ring's line,
/// a radian per `radius` points.
#[must_use]
pub fn edge_angle(centre: Vec2, press: Vec2, now: Vec2, radius: f32) -> f32 {
    let d = now - press;
    let out = (press - centre).normalize_or(Vec2::X);
    // the screen's y runs down: a turn to the left is the positive one
    -(out.perp_dot(d) / radius.max(1.0))
}

/// The scale factors (along the object's own three axes) of a scale drag: the pointer's travel
/// along the handle's axis on the screen over the axis's length there; the centre grows every
/// axis with the travel up and to the right.
#[must_use]
pub fn scale_factors(status: Status, s: &Shapes, press: Vec2, now: Vec2) -> Vec3 {
    let d = now - press;
    let along = |st: Status| {
        s.axes.iter().find(|(x, ..)| *x == st).map_or(0.0, |(_, _, tip, _)| {
            let dir = *tip - s.centre;
            let len = dir.length().max(1.0);
            d.dot(dir / len) / len
        })
    };
    let f = match status {
        Status::Xyz => {
            let t = (d.x - d.y) / 120.0;
            return Vec3::splat(1.0 + t);
        }
        Status::X => Vec3::new(along(Status::X), 0.0, 0.0),
        Status::Y => Vec3::new(0.0, along(Status::Y), 0.0),
        Status::Z => Vec3::new(0.0, 0.0, along(Status::Z)),
        st if st.plane_normal().is_some() => {
            let [x, y, z] = st.axes();
            let t = [(x, Status::X), (y, Status::Y), (z, Status::Z)]
                .iter()
                .filter(|(on, _)| *on)
                .map(|(_, a)| along(*a))
                .sum::<f32>()
                * 0.5;
            Vec3::new(if x { t } else { 0.0 }, if y { t } else { 0.0 }, if z { t } else { 0.0 })
        }
        _ => Vec3::ZERO,
    };
    Vec3::ONE + f
}

/// An object's scale after a scale drag: each factor applied, on the step when one is given, never
/// zero. A glow scales evenly.
#[must_use]
pub fn scaled(start: Vec3, factors: Vec3, glow: bool, step: Option<f32>) -> Vec3 {
    let factors = if glow {
        let f = [factors.x, factors.y, factors.z].into_iter().find(|f| (f - 1.0).abs() > 1e-6).unwrap_or(1.0);
        Vec3::splat(f)
    } else {
        factors
    };
    let mut s = start * factors;
    for a in 0..3 {
        if (factors[a] - 1.0).abs() > 1e-6
            && let Some(step) = step
        {
            s[a] = snapto(s[a], step);
        }
        if s[a].abs() < gz_scene::math::EPS_S {
            s[a] = gz_scene::math::EPS_S.copysign(if start[a] < 0.0 { -1.0 } else { 1.0 });
        }
    }
    s
}

/// An object's rotation (radians, the scene's order) as a rotation of the world.
#[must_use]
pub fn to_quat(rotation: Vec3) -> Quat {
    let m = gz_scene::math::rotation_matrix(rotation);
    Quat::from_mat3(&Mat3::from_cols(m.row(0), m.row(1), m.row(2))).normalize()
}

/// A rotation of the world as an object's rotation.
#[must_use]
pub fn from_quat(q: Quat) -> Vec3 {
    let m = Mat3::from_quat(q.normalize());
    let mut x = XMat::IDENTITY;
    x.set_row(0, m.x_axis);
    x.set_row(1, m.y_axis);
    x.set_row(2, m.z_axis);
    gz_scene::math::get_xyz(&x)
}

/// An object turned by `angle` about the world direction `axis` through `pivot`: its rotation and
/// its position.
#[must_use]
pub fn turned(rotation: Vec3, position: Vec3, pivot: Vec3, axis: Vec3, angle: f32) -> (Vec3, Vec3) {
    let q = Quat::from_axis_angle(axis.normalize_or(Vec3::Y), angle);
    (from_quat(q * to_quat(rotation)), pivot + q * (position - pivot))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ray(o: Vec3, d: Vec3) -> Ray {
        Ray { origin: o, dir: d.normalize() }
    }

    /// A camera 10 m back along -Z looking at the origin, its screen 100 pt a metre.
    fn proj(p: Vec3) -> Option<Vec2> {
        let z = p.z + 10.0;
        (z > 0.1).then(|| Vec2::new(400.0 + p.x * 1000.0 / z, 300.0 - p.y * 1000.0 / z))
    }

    #[test]
    fn steps_snap() {
        assert_eq!(snapto(0.26, 0.5), 0.5);
        assert_eq!(snapto(0.24, 0.5), 0.0);
        assert_eq!(snapto(-0.26, 0.5), -0.5);
        assert_eq!(snapto(3.0, 0.0), 3.0);
        assert!((snapto(37.0_f32, 15.0) - 30.0).abs() < 1e-4);
    }

    #[test]
    fn every_handle_of_the_move_gizmo_is_hit_where_it_is_drawn() {
        let s = shapes(GizmoType::Move, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::Z, &proj, false)
            .unwrap_or_default();
        let c = s.centre;
        assert_eq!(hit(GizmoType::Move, &s, c), Status::Screen, "the centre moves in the view's plane");
        // halfway along X, a few points off the line
        assert_eq!(hit(GizmoType::Move, &s, c + Vec2::new(60.0, 3.0)), Status::X);
        assert_eq!(hit(GizmoType::Move, &s, c + Vec2::new(-2.0, -60.0)), Status::Y);
        // the XY square sits between the two arrows
        assert_eq!(hit(GizmoType::Move, &s, c + Vec2::new(34.0, -34.0)), Status::XY);
        assert_eq!(hit(GizmoType::Move, &s, c + Vec2::new(70.0, 70.0)), Status::None);
        // the AI map's nodes take Y alone
        let a = shapes(GizmoType::Move, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::Z, &proj, true)
            .unwrap_or_default();
        assert_eq!(hit(GizmoType::Move, &a, c + Vec2::new(60.0, 3.0)), Status::None);
        assert_eq!(hit(GizmoType::Move, &a, c + Vec2::new(-2.0, -60.0)), Status::Y);
    }

    #[test]
    fn the_rotate_gizmo_takes_its_rings_and_the_outer_ring() {
        let s = shapes(GizmoType::Rotate, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::Z, &proj, false)
            .unwrap_or_default();
        let c = s.centre;
        // Z faces the camera: its ring is a circle of 85 pt; X and Y are seen edge-on, as lines
        assert_eq!(hit(GizmoType::Rotate, &s, c + Vec2::new(60.1, -60.1)), Status::Z);
        assert_eq!(hit(GizmoType::Rotate, &s, c + Vec2::new(40.0, 2.0)), Status::Y);
        // the outer ring, about the view
        let r = s.screen_ring.unwrap_or(0.0);
        assert!((r - 105.0).abs() < 1.0, "{r}");
        assert_eq!(hit(GizmoType::Rotate, &s, c + Vec2::new(0.0, r + 2.0)), Status::Screen);
        assert_eq!(hit(GizmoType::Rotate, &s, c + Vec2::new(30.0, 30.0)), Status::None);
    }

    #[test]
    fn a_scale_gizmo_has_its_centre_for_every_axis() {
        let s = shapes(GizmoType::Scale, Vec3::ZERO, [Vec3::X, Vec3::Y, Vec3::Z], Vec3::Z, &proj, false)
            .unwrap_or_default();
        assert_eq!(hit(GizmoType::Scale, &s, s.centre), Status::Xyz);
        // 50 pt along X's 100 pt: one and a half
        let f =
            scale_factors(Status::X, &s, s.centre + Vec2::new(100.0, 0.0), s.centre + Vec2::new(150.0, 0.0));
        assert!((f - Vec3::new(1.5, 1.0, 1.0)).length() < 1e-4, "{f}");
        assert_eq!(scaled(Vec3::ONE, f, false, Some(0.25)), Vec3::new(1.5, 1.0, 1.0));
        assert_eq!(scaled(Vec3::ONE, f, true, None), Vec3::splat(1.5));
    }

    #[test]
    fn an_axis_drag_follows_the_axis_alone() {
        let r0 = ray(Vec3::new(0.0, 10.0, -10.0), Vec3::new(0.0, -1.0, 1.0));
        let r1 = ray(Vec3::new(2.0, 10.0, -10.0), Vec3::new(0.0, -1.0, 1.0));
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        let d = move_delta(Status::X, Vec3::ZERO, axes, Vec3::Z, &r0, &r1).unwrap_or(Vec3::ZERO);
        assert!((d - Vec3::new(2.0, 0.0, 0.0)).length() < 1e-3, "{d}");
        let p = moved(Vec3::new(1.0, 1.0, 1.0), d, Status::X, axes, Space::World, Some(0.5));
        assert_eq!(p, Vec3::new(3.0, 1.0, 1.0));
    }

    #[test]
    fn a_plane_drag_stays_in_its_plane() {
        let axes = [Vec3::X, Vec3::Y, Vec3::Z];
        // looking down at the ground: the XZ square slides on it
        let r0 = ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.1, -1.0, 0.1));
        let r1 = ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.3, -1.0, -0.2));
        let d = move_delta(Status::XZ, Vec3::ZERO, axes, Vec3::NEG_Y, &r0, &r1).unwrap_or(Vec3::ZERO);
        assert!(d.y.abs() < 1e-5 && d.x > 1.0 && d.z < -1.0, "{d}");
        // in Local space a turned object's plane is its own
        let q = Quat::from_rotation_y(std::f32::consts::FRAC_PI_4);
        let local = [q * Vec3::X, q * Vec3::Y, q * Vec3::Z];
        let d = move_delta(Status::XZ, Vec3::ZERO, local, Vec3::NEG_Y, &r0, &r1).unwrap_or(Vec3::ZERO);
        assert!(d.y.abs() < 1e-5);
        let p = moved(Vec3::ZERO, d, Status::XZ, local, Space::Local, Some(1.0));
        assert!(p.dot(local[1]).abs() < 1e-5, "a local step keeps it in the plane: {p}");
    }

    #[test]
    fn a_ring_drag_turns_by_the_angle_swept_and_snaps() {
        // looking down at the Y ring: a quarter turn of the pointer round the pivot
        let r0 = ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(1.0, -10.0, 0.0));
        let r1 = ray(Vec3::new(0.0, 10.0, 0.0), Vec3::new(0.0, -10.0, -1.0));
        let a = ring_angle(Vec3::Y, Vec3::ZERO, &r0, &r1).unwrap_or(0.0);
        assert!((a - std::f32::consts::FRAC_PI_2).abs() < 1e-4, "{a}");
        assert!((snapto(a.to_degrees() + 4.0, 15.0) - 90.0).abs() < 1e-3);
        // edge-on there is no angle
        let flat = ray(Vec3::new(0.0, 0.0, -10.0), Vec3::Z);
        assert!(ring_angle(Vec3::Y, Vec3::ZERO, &flat, &flat).is_none());
        // a ring seen edge-on turns with the pointer's travel across it
        let e = edge_angle(Vec2::ZERO, Vec2::new(50.0, 0.0), Vec2::new(50.0, -50.0), 50.0);
        assert!((e - 1.0).abs() < 1e-4, "{e}");
    }

    #[test]
    fn a_turn_about_a_world_axis_turns_the_object_and_its_place() {
        let q = to_quat(Vec3::new(0.3, -0.7, 0.2));
        let back = to_quat(from_quat(q));
        assert!(q.dot(back).abs() > 0.9999, "the scene's rotations round-trip");
        let (r, p) =
            turned(Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, Vec3::Y, std::f32::consts::FRAC_PI_2);
        assert!((p - Vec3::new(0.0, 0.0, -2.0)).length() < 1e-4, "{p}");
        let x = to_quat(r) * Vec3::X;
        assert!((x - Vec3::new(0.0, 0.0, -1.0)).length() < 1e-4, "{x}");
    }

    #[test]
    fn a_selection_stands_at_its_centre() {
        let one = Anchor::of(&[(Vec3::ONE, Vec3::ZERO, None)]).map(|a| a.position);
        assert_eq!(one, Some(Vec3::ONE));
        let two = Anchor::of(&[(Vec3::ZERO, Vec3::ZERO, None), (Vec3::new(2.0, 0.0, 0.0), Vec3::ZERO, None)]);
        assert_eq!(two.map(|a| a.position), Some(Vec3::new(1.0, 0.0, 0.0)));
        assert!(Anchor::of(&[]).is_none());
        let turned = Anchor::of(&[(Vec3::ZERO, Vec3::new(0.0, std::f32::consts::FRAC_PI_2, 0.0), None)]);
        let axes = turned.map(|a| a.axes(Space::Local)).unwrap_or([Vec3::ZERO; 3]);
        assert!(axes[0].dot(Vec3::X).abs() < 1e-4, "a turned object's own X is not the world's");
    }
}
