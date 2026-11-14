//! Boxes, frusta and rays.

use glam::{Mat4, Vec3, Vec4};

/// An axis-aligned box. [`Aabb::EMPTY`] is the box any point grows into itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::EMPTY
    }
}

impl Aabb {
    pub const EMPTY: Aabb = Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(-f32::MAX) };
    /// The unit box around the origin, -0.5 to 0.5: what a placeholder and a box helper are.
    pub const UNIT: Aabb = Aabb { min: Vec3::splat(-0.5), max: Vec3::splat(0.5) };

    #[must_use]
    pub const fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// The box around points; [`Aabb::EMPTY`] for none.
    #[must_use]
    pub fn from_points(points: impl IntoIterator<Item = Vec3>) -> Self {
        let mut b = Self::EMPTY;
        for p in points {
            b.extend(p);
        }
        b
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.min.x > self.max.x || self.min.y > self.max.y || self.min.z > self.max.z
    }

    pub fn extend(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    #[must_use]
    pub fn union(&self, other: &Aabb) -> Aabb {
        Aabb { min: self.min.min(other.min), max: self.max.max(other.max) }
    }

    #[must_use]
    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    #[must_use]
    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }

    /// The centre and the half diagonal.
    #[must_use]
    pub fn sphere(&self) -> (Vec3, f32) {
        if self.is_empty() {
            return (Vec3::ZERO, 0.0);
        }
        (self.center(), (self.max - self.min).length() * 0.5)
    }

    /// The box around this box moved by `m` (Arvo's method: exact for the corners).
    #[must_use]
    pub fn transformed(&self, m: &Mat4) -> Aabb {
        if self.is_empty() {
            return *self;
        }
        let t = m.w_axis.truncate();
        let mut min = t;
        let mut max = t;
        let axes = [m.x_axis.truncate(), m.y_axis.truncate(), m.z_axis.truncate()];
        let (lo, hi) = (self.min.to_array(), self.max.to_array());
        for (axis, (a, b)) in axes.iter().zip(lo.into_iter().zip(hi)) {
            let e = *axis * a;
            let f = *axis * b;
            min += e.min(f);
            max += e.max(f);
        }
        Aabb { min, max }
    }

    /// The transform that turns [`Aabb::UNIT`] into this box.
    #[must_use]
    pub fn unit_transform(&self) -> Mat4 {
        if self.is_empty() {
            return Mat4::IDENTITY;
        }
        let size = self.size().max(Vec3::splat(1e-4));
        Mat4::from_scale_rotation_translation(size, glam::Quat::IDENTITY, self.center())
    }
}

/// The six planes of a view volume, pointing inside: `n . p + d >= 0` is in.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frustum {
    pub planes: [Vec4; 6],
}

impl Frustum {
    /// The volume a `view_proj` (clip = `view_proj * p`, depth 0 to 1 in either direction) sees.
    #[must_use]
    pub fn from_view_proj(m: &Mat4) -> Self {
        let (r0, r1, r2, r3) = (m.row(0), m.row(1), m.row(2), m.row(3));
        let raw = [r3 + r0, r3 - r0, r3 + r1, r3 - r1, r2, r3 - r2];
        let planes = raw.map(|p| {
            let len = p.truncate().length();
            if len > 0.0 { p / len } else { p }
        });
        Self { planes }
    }

    /// Whether any part of the box may be inside (conservative: a box near a corner of the
    /// volume may pass while it is outside).
    #[must_use]
    pub fn intersects_aabb(&self, b: &Aabb) -> bool {
        if b.is_empty() {
            return false;
        }
        self.planes.iter().all(|p| {
            let n = p.truncate();
            let far = Vec3::select(n.cmpge(Vec3::ZERO), b.max, b.min);
            n.dot(far) + p.w >= 0.0
        })
    }

    #[must_use]
    pub fn intersects_sphere(&self, center: Vec3, radius: f32) -> bool {
        self.planes.iter().all(|p| p.truncate().dot(center) + p.w >= -radius)
    }

    #[must_use]
    pub fn contains_point(&self, point: Vec3) -> bool {
        self.intersects_sphere(point, 0.0)
    }
}

/// A half line.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Ray {
    pub origin: Vec3,
    /// Unit length.
    pub dir: Vec3,
}

impl Ray {
    #[must_use]
    pub fn at(&self, t: f32) -> Vec3 {
        self.origin + self.dir * t
    }

    /// The distance to where the ray enters the box (0 when it starts inside).
    #[must_use]
    pub fn intersect_aabb(&self, b: &Aabb) -> Option<f32> {
        if b.is_empty() {
            return None;
        }
        let inv = self.dir.recip();
        let t0 = (b.min - self.origin) * inv;
        let t1 = (b.max - self.origin) * inv;
        let near = t0.min(t1).max_element().max(0.0);
        let far = t0.max(t1).min_element();
        (near <= far).then_some(near)
    }

    /// The distance to the horizontal plane at `y` (the grid's plane is 0).
    #[must_use]
    pub fn intersect_plane_y(&self, y: f32) -> Option<f32> {
        if self.dir.y.abs() < 1e-8 {
            return None;
        }
        let t = (y - self.origin.y) / self.dir.y;
        (t >= 0.0).then_some(t)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boxes_grow_and_move() {
        let b = Aabb::from_points([Vec3::new(1.0, 2.0, 3.0), Vec3::new(-1.0, 0.0, 5.0)]);
        assert_eq!(b.min, Vec3::new(-1.0, 0.0, 3.0));
        assert_eq!(b.max, Vec3::new(1.0, 2.0, 5.0));
        assert!(Aabb::EMPTY.is_empty());
        let moved = Aabb::UNIT.transformed(&Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0)));
        assert_eq!(moved.center(), Vec3::new(10.0, 0.0, 0.0));
        // a quarter turn about Y keeps a cube a cube
        let turned = Aabb::new(Vec3::new(-2.0, -1.0, -1.0), Vec3::new(2.0, 1.0, 1.0))
            .transformed(&Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2));
        assert!((turned.size() - Vec3::new(2.0, 2.0, 4.0)).abs().max_element() < 1e-5);
        let u = b.unit_transform();
        let back = Aabb::UNIT.transformed(&u);
        assert!(
            (back.min - b.min).abs().max_element() < 1e-5 && (back.max - b.max).abs().max_element() < 1e-5
        );
    }

    #[test]
    fn rays_hit_boxes_and_planes() {
        let r = Ray { origin: Vec3::new(0.0, 0.0, -5.0), dir: Vec3::Z };
        assert_eq!(r.intersect_aabb(&Aabb::UNIT), Some(4.5));
        let miss = Ray { origin: Vec3::new(3.0, 0.0, -5.0), dir: Vec3::Z };
        assert_eq!(miss.intersect_aabb(&Aabb::UNIT), None);
        let down = Ray { origin: Vec3::new(0.0, 10.0, 0.0), dir: -Vec3::Y };
        assert_eq!(down.intersect_plane_y(0.0), Some(10.0));
        assert_eq!(down.intersect_plane_y(20.0), None);
    }
}
