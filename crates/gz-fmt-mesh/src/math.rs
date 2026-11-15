//! The change of handedness and the bridge from `glam`'s matrices to the game's.

use gz_fmt_object::glam::{Mat3, Mat4, Quat, Vec3};
#[cfg(test)]
use gz_fmt_object::xmath;

/// A point or a direction of a right-handed source in the game's left-handed space.
pub(crate) fn mirror(v: Vec3) -> Vec3 {
    Vec3::new(v.x, v.y, -v.z)
}

/// A rotation of a right-handed source in the game's space: `S * R * S` as a quaternion.
pub(crate) fn mirror_quat(q: Quat) -> Quat {
    Quat::from_xyzw(-q.x, -q.y, q.z, q.w)
}

/// A transform of a right-handed source in the game's space: `S * M * S`.
pub(crate) fn mirror_mat(m: Mat4) -> Mat4 {
    let s = Mat4::from_scale(Vec3::new(1.0, 1.0, -1.0));
    s * m * s
}

/// The rotation and the translation of a transform, its scale dropped. A transform that holds
/// no rotation (a zero scale, a value that is not a number) gives the identity rotation.
pub(crate) fn rigid(m: &Mat4) -> (Quat, Vec3) {
    let t = m.w_axis.truncate();
    let mut x = m.x_axis.truncate();
    let mut y = m.y_axis.truncate();
    // a mirroring transform keeps its rotation; the mirror is dropped with the scale
    if m.determinant() < 0.0 {
        x = -x;
    }
    let x = x.normalize_or_zero();
    y = (y - x * x.dot(y)).normalize_or_zero();
    let z = x.cross(y);
    let r = Mat3::from_cols(x, y, z);
    let q = Quat::from_mat3(&r);
    if !q.is_finite() || x == Vec3::ZERO || y == Vec3::ZERO {
        return (Quat::IDENTITY, if t.is_finite() { t } else { Vec3::ZERO });
    }
    (q.normalize(), t)
}

/// A rigid transform of the game's space as the game's matrix: rows are the images of the axes.
#[cfg(test)]
pub(crate) fn to_xray(q: Quat, t: Vec3) -> xmath::Mat4 {
    let r = Mat3::from_quat(q);
    let row = |v: Vec3| [v.x, v.y, v.z, 0.0];
    xmath::Mat4([row(r.x_axis), row(r.y_axis), row(r.z_axis), [t.x, t.y, t.z, 1.0]])
}

/// The angles a bone stores (`rest_rotate`, and a motion's rotation channels) for a rotation:
/// `xmath::Mat4::xyz_i(angles)` gives it back. Of the two sets of angles that give one rotation,
/// the one nearer `near` is taken, every angle brought within half a turn of it.
pub(crate) fn angles(q: Quat, near: Option<Vec3>) -> Vec3 {
    use std::f32::consts::PI;
    let r = Mat3::from_quat(q);
    // `xmath::Mat4::hpb(h, p, b)`: row 2 is (-cos p sin h, sin p, cos p cos h), row 0 starts
    // (.., -cos p sin b, ..), row 1 (.., cos p cos b, ..); rows are the images of the axes
    let (x, y, z) = (r.x_axis, r.y_axis, r.z_axis);
    let p = z.y.clamp(-1.0, 1.0).asin();
    let (h, b) = if p.cos() > 1e-5 { ((-z.x).atan2(z.z), (-x.y).atan2(y.y)) } else { (x.z.atan2(x.x), 0.0) };
    // `xyz_i(r)` is `hpb(-r.y, -r.x, -r.z)`
    let first = Vec3::new(-p, -h, -b);
    let Some(near) = near else { return first };
    let second = Vec3::new(-(PI - p), -(h + PI), -(b + PI));
    let wrap = |a: f32, n: f32| a + ((n - a) / (2.0 * PI)).round() * 2.0 * PI;
    let bring = |v: Vec3| Vec3::new(wrap(v.x, near.x), wrap(v.y, near.y), wrap(v.z, near.z));
    let (a, b) = (bring(first), bring(second));
    if (a - near).length_squared() <= (b - near).length_squared() { a } else { b }
}

/// Of two rigid transforms, `parent^-1 * child`.
pub(crate) fn relative(parent: (Quat, Vec3), child: (Quat, Vec3)) -> (Quat, Vec3) {
    let inv = parent.0.inverse();
    ((inv * child.0).normalize(), inv * (child.1 - parent.1))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn same(a: &xmath::Mat4, b: &xmath::Mat4) -> bool {
        a.0.iter().flatten().zip(b.0.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-4)
    }

    #[test]
    fn angles_give_the_rotation_back_through_the_games_formula() {
        let samples = [
            Quat::IDENTITY,
            Quat::from_rotation_y(0.7),
            Quat::from_rotation_x(-1.2),
            Quat::from_rotation_z(2.9),
            Quat::from_euler(gz_fmt_object::glam::EulerRot::XYZ, 0.3, -0.8, 1.9),
            Quat::from_euler(gz_fmt_object::glam::EulerRot::YXZ, 1.0, std::f32::consts::FRAC_PI_2, 0.4),
        ];
        for q in samples {
            let want = to_xray(q, Vec3::new(1.0, 2.0, 3.0));
            let mut got = xmath::Mat4::xyz_i(angles(q, None));
            got.set_c(Vec3::new(1.0, 2.0, 3.0));
            assert!(same(&got, &want), "{q:?}");
            // the other set of angles is the same rotation
            let other = angles(q, Some(angles(q, None) + Vec3::splat(3.0)));
            assert!(same(&xmath::Mat4::xyz_i(other), &xmath::Mat4::xyz_i(angles(q, None))));
        }
        // the game's matrix moves a point the way the rotation does
        let q = Quat::from_rotation_y(0.5);
        let m = to_xray(q, Vec3::ZERO);
        assert!((m.transform_tiny(Vec3::X) - q * Vec3::X).length() < 1e-6);
    }

    #[test]
    fn angles_follow_the_last_frame_across_a_turn() {
        let near = Vec3::new(0.0, -3.1, 0.0);
        let a = angles(Quat::from_rotation_y(3.13), Some(near));
        assert!((a - near).length() < 0.1, "{a:?}");
    }

    #[test]
    fn mirroring_keeps_rotations_rotations() {
        let m = Mat4::from_rotation_translation(Quat::from_rotation_x(0.4), Vec3::new(1.0, 2.0, 3.0));
        let g = mirror_mat(m);
        let (q, t) = rigid(&g);
        assert!((t - Vec3::new(1.0, 2.0, -3.0)).length() < 1e-6);
        let p = Vec3::new(0.2, 0.5, 0.9);
        assert!((q * mirror(p) + t - mirror(m.transform_point3(p))).length() < 1e-5);
        assert!(
            (mirror_quat(Quat::from_rotation_x(0.4)) * mirror(p) - mirror(Quat::from_rotation_x(0.4) * p))
                .length()
                < 1e-6
        );
        // scale and a mirror are dropped, the rotation stays
        let (q2, _) = rigid(&(g * Mat4::from_scale(Vec3::new(-3.0, 3.0, 3.0))));
        assert!(q2.angle_between(q) < 1e-4);
        assert_eq!(rigid(&Mat4::ZERO).0, Quat::IDENTITY);
    }
}
