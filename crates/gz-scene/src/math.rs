//! The engine's transform arithmetic as the scene uses it.

pub use glam::Vec3;
pub use gz_fmt_level::Aabb;
pub use gz_fmt_object::Mat4;

// The engine's epsilons (`XrCore/vector.h`), re-exported so the workspace has one copy.
pub use gz_fmt_level::{EPS, EPS_L, EPS_S};

/// `FTransformR.setXYZi(-x, -y, -z)`, which is `setHPB(y, x, z)`.
#[must_use]
pub fn rotation_matrix(r: Vec3) -> Mat4 {
    Mat4::hpb(r.y, r.x, r.z)
}

#[must_use]
pub fn scale_matrix(s: Vec3) -> Mat4 {
    let mut m = Mat4::IDENTITY;
    m.0[0][0] = s.x;
    m.0[1][1] = s.y;
    m.0[2][2] = s.z;
    m
}

/// `getHPB` read back as `getXYZ`: the angles `rotation_matrix` builds the matrix from.
#[must_use]
pub fn get_xyz(m: &Mat4) -> Vec3 {
    let (i, j, k) = (m.row(0), m.row(1), m.row(2));
    let cy = (j.y * j.y + i.y * i.y).sqrt();
    let (h, p, b) = if cy > 16.0 * f32::EPSILON {
        (-(k.x.atan2(k.z)), -((-k.y).atan2(cy)), -(i.y.atan2(j.y)))
    } else {
        (-((-i.z).atan2(i.x)), -((-k.y).atan2(cy)), 0.0)
    };
    Vec3::new(p, h, b)
}

/// `CCustomObject::NormalAlign`: the rotation that stands an object on a surface normal.
#[must_use]
pub fn normal_align(up: Vec3, dir: Vec3) -> Vec3 {
    let n = up;
    let mut d = dir;
    if n.z.abs() > 0.99 {
        d = Vec3::X;
    }
    let r = n.cross(d).normalize_or_zero();
    let d = r.cross(n).normalize_or_zero();
    let mut m = Mat4::IDENTITY;
    m.set_row(0, r);
    m.set_row(1, n);
    m.set_row(2, d);
    get_xyz(&m)
}

/// Position, rotation (radians, `x` pitch, `y` heading, `z` bank) and scale, as
/// `CCustomObject` keeps them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Transform {
    pub position: Vec3,
    pub rotation: Vec3,
    pub scale: Vec3,
}

impl Default for Transform {
    fn default() -> Self {
        Self { position: Vec3::ZERO, rotation: Vec3::ZERO, scale: Vec3::ONE }
    }
}

impl Transform {
    #[must_use]
    pub fn at(position: Vec3) -> Self {
        Self { position, ..Self::default() }
    }

    /// `FTransformRP`: rotation then translation.
    #[must_use]
    pub fn matrix_rp(&self) -> Mat4 {
        Mat4::mul(&Mat4::translation(self.position), &rotation_matrix(self.rotation))
    }

    /// `FTransform`: scale, rotation, translation.
    #[must_use]
    pub fn matrix(&self) -> Mat4 {
        Mat4::mul(&self.matrix_rp(), &scale_matrix(self.scale))
    }

    /// `FITransform`.
    #[must_use]
    pub fn inverse(&self) -> Mat4 {
        Mat4::invert_43(&self.matrix())
    }

    /// Every component finite (`_valid`).
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.position.is_finite() && self.rotation.is_finite() && self.scale.is_finite()
    }
}

/// `Fbox::xform`: the box around a box carried by a matrix.
#[must_use]
pub fn xform_box(b: &Aabb, m: &Mat4) -> Aabb {
    if !b.is_valid() {
        return Aabb::INVALID;
    }
    // the C++ way to the bit (a box's bounds are part of the level files): the min corner carried,
    // then each edge vector (a column of the matrix times the box's size) added to the min or the
    // max by its sign
    let edges =
        [m.row(0) * (b.max.x - b.min.x), m.row(1) * (b.max.y - b.min.y), m.row(2) * (b.max.z - b.min.z)];
    let mut min = m.transform_tiny(b.min);
    let mut max = min;
    for e in edges {
        for (axis, v) in [e.x, e.y, e.z].into_iter().enumerate() {
            // `negative`: the sign bit, so -0.0 counts as negative
            let (lo, hi) = (&mut min, &mut max);
            let target = if v.is_sign_negative() { lo } else { hi };
            match axis {
                0 => target.x += v,
                1 => target.y += v,
                _ => target.z += v,
            }
        }
    }
    Aabb::new(min, max)
}

/// The box of a point grown by `r` in every direction.
#[must_use]
pub fn point_box(p: Vec3, r: f32) -> Aabb {
    Aabb::new(p - Vec3::splat(r), p + Vec3::splat(r))
}

/// `Fbox::Pick2`: where a ray enters a box. `None` when it misses, `Some((t, false))` from
/// outside, `Some((0, true))` when the origin is inside.
#[must_use]
pub fn ray_box(origin: Vec3, dir: Vec3, b: &Aabb) -> Option<(f32, bool)> {
    if !b.is_valid() {
        return None;
    }
    let inside = (0..3).all(|a| origin[a] >= b.min[a] && origin[a] <= b.max[a]);
    if inside {
        return Some((0.0, true));
    }
    let mut t0 = 0.0_f32;
    let mut t1 = f32::MAX;
    for a in 0..3 {
        if dir[a].abs() < 1e-20 {
            if origin[a] < b.min[a] || origin[a] > b.max[a] {
                return None;
            }
            continue;
        }
        let inv = 1.0 / dir[a];
        let (mut near, mut far) = ((b.min[a] - origin[a]) * inv, (b.max[a] - origin[a]) * inv);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some((t0, false))
}

/// The sphere test of the point-like classes (lights, sounds, spawns): the distance along the
/// ray to the nearest approach when the ray passes within `radius` and starts outside it.
#[must_use]
pub fn ray_point(origin: Vec3, dir: Vec3, center: Vec3, radius: f32) -> Option<f32> {
    let ray2 = center - origin;
    let d = ray2.dot(dir);
    if d <= 0.0 {
        return None;
    }
    let d2 = ray2.length();
    if (d2 * d2 - d * d) < radius * radius && d > radius { Some(d) } else { None }
}

/// `ClipTriangleToRect` of the drop to ground: a triangle clipped to an XZ rectangle, heights
/// kept (Sutherland-Hodgman, at most 15 points).
#[must_use]
pub fn clip_triangle_xz(tri: &[Vec3; 3], x0: f32, x1: f32, z0: f32, z1: f32) -> Vec<Vec3> {
    let mut src: Vec<Vec3> = tri.to_vec();
    for plane in 0..4 {
        if src.is_empty() {
            break;
        }
        let mut dst: Vec<Vec3> = Vec::with_capacity(16);
        let n = src.len();
        for i in 0..n {
            let cur = src[i];
            let nxt = src[(i + 1) % n];
            let (cd, nd) = match plane {
                0 => (cur.x - x0, nxt.x - x0),
                1 => (x1 - cur.x, x1 - nxt.x),
                2 => (cur.z - z0, nxt.z - z0),
                _ => (z1 - cur.z, z1 - nxt.z),
            };
            let (ci, ni) = (cd >= 0.0, nd >= 0.0);
            if ci && dst.len() < 15 {
                dst.push(cur);
            }
            if ci != ni && dst.len() < 15 {
                let t = cd / (cd - nd);
                dst.push(cur.lerp(nxt, t));
            }
        }
        src = dst;
    }
    src
}

/// `Fvector::mknormal`: the normal of a triangle, unit length (zero for a degenerate one).
#[must_use]
pub fn tri_normal(a: Vec3, b: Vec3, c: Vec3) -> Vec3 {
    (b - a).cross(c - a).normalize_or_zero()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn euler_angles_read_back() {
        let r = Vec3::new(0.3, -1.2, 0.4);
        let back = get_xyz(&rotation_matrix(r));
        assert!((back - r).abs().max_element() < 1e-4, "{back:?}");
    }

    #[test]
    fn matrix_carries_points() {
        let t =
            Transform { position: Vec3::new(1.0, 2.0, 3.0), rotation: Vec3::ZERO, scale: Vec3::splat(2.0) };
        let p = t.matrix().transform_tiny(Vec3::ONE);
        assert!((p - Vec3::new(3.0, 4.0, 5.0)).length() < 1e-5);
        let back = t.inverse().transform_tiny(p);
        assert!((back - Vec3::ONE).length() < 1e-5);
    }

    #[test]
    fn rays_enter_boxes() {
        let b = Aabb::new(Vec3::splat(-1.0), Vec3::splat(1.0));
        let (t, inside) = ray_box(Vec3::new(-5.0, 0.0, 0.0), Vec3::X, &b).unwrap();
        assert!(!inside && (t - 4.0).abs() < 1e-5);
        assert!(ray_box(Vec3::new(-5.0, 3.0, 0.0), Vec3::X, &b).is_none());
        assert_eq!(ray_box(Vec3::ZERO, Vec3::X, &b), Some((0.0, true)));
    }

    #[test]
    fn clipping_keeps_heights() {
        let tri = [Vec3::new(-2.0, 0.0, -2.0), Vec3::new(2.0, 2.0, -2.0), Vec3::new(0.0, 1.0, 2.0)];
        let poly = clip_triangle_xz(&tri, -1.0, 1.0, -1.0, 1.0);
        assert!(!poly.is_empty());
        assert!(poly.iter().all(|p| p.x >= -1.0 - 1e-5 && p.x <= 1.0 + 1e-5));
    }
}
