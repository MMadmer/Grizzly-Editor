//! The engine's float math, operation for operation.
//!
//! Exported files hold what these formulas produce, so the order of every multiply and add is
//! the engine's (`_matrix.h`, `_quaternion.h`, `_fbox.h`), not the tidiest one.

use glam::{Quat, Vec3};

pub const EPS_S: f32 = 0.000_000_1;
pub const EPS: f32 = 0.000_010_0;
pub const EPS_L: f32 = 0.001_000_0;

/// `Fvector::similar`.
#[must_use]
pub fn similar(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

/// `Fvector::magnitude`.
#[must_use]
pub fn magnitude(v: Vec3) -> f32 {
    (v.x * v.x + v.y * v.y + v.z * v.z).sqrt()
}

/// `Fvector::crossproduct`.
#[must_use]
pub fn cross(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x)
}

/// `Fvector::mknormal`: the normal of a triangle, left as it is when the triangle has no area.
#[must_use]
pub fn mknormal(p0: Vec3, p1: Vec3, p2: Vec3) -> Vec3 {
    let v01 = Vec3::new(p1.x - p0.x, p1.y - p0.y, p1.z - p0.z);
    let v12 = Vec3::new(p2.x - p1.x, p2.y - p1.y, p2.z - p1.z);
    let mut n = cross(v01, v12);
    let m = n.x * n.x + n.y * n.y + n.z * n.z;
    if m > f32::MIN_POSITIVE {
        let s = (1.0 / m).sqrt();
        n.x *= s;
        n.y *= s;
        n.z *= s;
    }
    n
}

/// What x86 `cvttss2si` gives for a float: truncation, and `i32::MIN` for a NaN or a value
/// that does not fit. Key quantisation depends on it: a bone that does not move along an axis
/// divides zero by zero.
#[must_use]
pub fn cvtt(f: f32) -> i32 {
    if f.is_nan() || f >= 2_147_483_648.0 || f < -2_147_483_648.0 { i32::MIN } else { f as i32 }
}

/// `Fbox`: an axis-aligned box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Aabb {
    /// `invalidate()`: the box that any point shrinks to itself.
    pub const INVALID: Aabb = Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(-f32::MAX) };
    pub const ZERO: Aabb = Aabb { min: Vec3::ZERO, max: Vec3::ZERO };

    pub fn modify(&mut self, p: Vec3) {
        self.min.x = if self.min.x < p.x { self.min.x } else { p.x };
        self.min.y = if self.min.y < p.y { self.min.y } else { p.y };
        self.min.z = if self.min.z < p.z { self.min.z } else { p.z };
        self.max.x = if self.max.x > p.x { self.max.x } else { p.x };
        self.max.y = if self.max.y > p.y { self.max.y } else { p.y };
        self.max.z = if self.max.z > p.z { self.max.z } else { p.z };
    }

    pub fn merge(&mut self, b: &Aabb) {
        self.modify(b.min);
        self.modify(b.max);
    }

    /// `getpoint(index)`: one of the eight corners.
    #[must_use]
    pub fn corner(&self, index: usize) -> Vec3 {
        let (a, b) = (self.min, self.max);
        match index & 7 {
            0 => Vec3::new(a.x, a.y, a.z),
            1 => Vec3::new(a.x, a.y, b.z),
            2 => Vec3::new(b.x, a.y, b.z),
            3 => Vec3::new(b.x, a.y, a.z),
            4 => Vec3::new(a.x, b.y, a.z),
            5 => Vec3::new(a.x, b.y, b.z),
            6 => Vec3::new(b.x, b.y, b.z),
            _ => Vec3::new(b.x, b.y, a.z),
        }
    }

    /// `getsphere`: the centre and the distance from it to `max`.
    #[must_use]
    pub fn sphere(&self) -> (Vec3, f32) {
        let c = Vec3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        );
        let d = Vec3::new(c.x - self.max.x, c.y - self.max.y, c.z - self.max.z);
        (c, (d.x * d.x + d.y * d.y + d.z * d.z).sqrt())
    }
}

/// `Fmatrix`: four rows `i`, `j`, `k`, `c` - the three axes and the translation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [[f32; 4]; 4]);

impl Default for Mat4 {
    fn default() -> Self {
        Self::IDENTITY
    }
}

impl Mat4 {
    pub const IDENTITY: Mat4 =
        Mat4([[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]]);

    #[must_use]
    pub fn row(&self, r: usize) -> Vec3 {
        let m = &self.0[r & 3];
        Vec3::new(m[0], m[1], m[2])
    }

    pub fn set_row(&mut self, r: usize, v: Vec3) {
        let m = &mut self.0[r & 3];
        m[0] = v.x;
        m[1] = v.y;
        m[2] = v.z;
    }

    /// The translation row.
    #[must_use]
    pub fn c(&self) -> Vec3 {
        self.row(3)
    }

    pub fn set_c(&mut self, v: Vec3) {
        self.set_row(3, v);
    }

    /// `translate()`: identity with a translation.
    #[must_use]
    pub fn translation(v: Vec3) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        m.set_c(v);
        m
    }

    /// `setHPB(h, p, b)`.
    #[must_use]
    pub fn hpb(h: f32, p: f32, b: f32) -> Mat4 {
        let (sh, ch) = (h.sin(), h.cos());
        let (sp, cp) = (p.sin(), p.cos());
        let (sb, cb) = (b.sin(), b.cos());
        let cc = ch * cb;
        let cs = ch * sb;
        let sc = sh * cb;
        let ss = sh * sb;
        Mat4([
            [cc - sp * ss, -cp * sb, sp * cs + sc, 0.0],
            [sp * sc + cs, cp * cb, ss - sp * cc, 0.0],
            [-cp * sh, sp, cp * ch, 0.0],
            [0.0, 0.0, 0.0, 1.0],
        ])
    }

    /// `setXYZi(xyz)`: the game's rotation order, as bones store it.
    #[must_use]
    pub fn xyz_i(r: Vec3) -> Mat4 {
        Mat4::hpb(-r.y, -r.x, -r.z)
    }

    /// `mul(A, B)`: the full product, projection column included.
    #[must_use]
    pub fn mul(a: &Mat4, b: &Mat4) -> Mat4 {
        let (a, b) = (&a.0, &b.0);
        let mut m = [[0.0f32; 4]; 4];
        for (r, row) in m.iter_mut().enumerate() {
            for (c, v) in row.iter_mut().enumerate() {
                *v = a[0][c] * b[r][0] + a[1][c] * b[r][1] + a[2][c] * b[r][2] + a[3][c] * b[r][3];
            }
        }
        Mat4(m)
    }

    /// `mul_43(A, B)`: the product without the projection column.
    #[must_use]
    pub fn mul_43(a: &Mat4, b: &Mat4) -> Mat4 {
        let (a, b) = (&a.0, &b.0);
        let mut m = [[0.0f32; 4]; 4];
        for r in 0..3 {
            for c in 0..3 {
                m[r][c] = a[0][c] * b[r][0] + a[1][c] * b[r][1] + a[2][c] * b[r][2];
            }
        }
        for c in 0..3 {
            m[3][c] = a[0][c] * b[3][0] + a[1][c] * b[3][1] + a[2][c] * b[3][2] + a[3][c];
        }
        m[3][3] = 1.0;
        Mat4(m)
    }

    /// `this.mulA_43(A)`: `A * this`.
    #[must_use]
    pub fn mul_a43(&self, a: &Mat4) -> Mat4 {
        Mat4::mul_43(a, self)
    }

    /// `invert(a)`: the inverse of a 4x3 transform.
    #[must_use]
    pub fn invert_43(a: &Mat4) -> Mat4 {
        let a = &a.0;
        let det = a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1]) - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
            + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
        let d = 1.0 / det;
        let mut m = [[0.0f32; 4]; 4];
        m[0][0] = d * (a[1][1] * a[2][2] - a[1][2] * a[2][1]);
        m[0][1] = -d * (a[0][1] * a[2][2] - a[0][2] * a[2][1]);
        m[0][2] = d * (a[0][1] * a[1][2] - a[0][2] * a[1][1]);
        m[1][0] = -d * (a[1][0] * a[2][2] - a[1][2] * a[2][0]);
        m[1][1] = d * (a[0][0] * a[2][2] - a[0][2] * a[2][0]);
        m[1][2] = -d * (a[0][0] * a[1][2] - a[0][2] * a[1][0]);
        m[2][0] = d * (a[1][0] * a[2][1] - a[1][1] * a[2][0]);
        m[2][1] = -d * (a[0][0] * a[2][1] - a[0][1] * a[2][0]);
        m[2][2] = d * (a[0][0] * a[1][1] - a[0][1] * a[1][0]);
        m[3][0] = -(a[3][0] * m[0][0] + a[3][1] * m[1][0] + a[3][2] * m[2][0]);
        m[3][1] = -(a[3][0] * m[0][1] + a[3][1] * m[1][1] + a[3][2] * m[2][1]);
        m[3][2] = -(a[3][0] * m[0][2] + a[3][1] * m[1][2] + a[3][2] * m[2][2]);
        m[3][3] = 1.0;
        Mat4(m)
    }

    /// `transform_tiny`: a point through the 4x3 part.
    #[must_use]
    pub fn transform_tiny(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0] + m[3][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1] + m[3][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2] + m[3][2],
        )
    }

    /// `transform_dir`: a direction through the 3x3 part.
    #[must_use]
    pub fn transform_dir(&self, v: Vec3) -> Vec3 {
        let m = &self.0;
        Vec3::new(
            v.x * m[0][0] + v.y * m[1][0] + v.z * m[2][0],
            v.x * m[0][1] + v.y * m[1][1] + v.z * m[2][1],
            v.x * m[0][2] + v.y * m[1][2] + v.z * m[2][2],
        )
    }

    /// `mk_xform(Q, V)`: a rotation and a translation.
    #[must_use]
    pub fn from_quat(q: Quat, t: Vec3) -> Mat4 {
        let (xx, yy, zz) = (q.x * q.x, q.y * q.y, q.z * q.z);
        let (xy, xz, yz) = (q.x * q.y, q.x * q.z, q.y * q.z);
        let (wx, wy, wz) = (q.w * q.x, q.w * q.y, q.w * q.z);
        Mat4([
            [1.0 - 2.0 * (yy + zz), 2.0 * (xy - wz), 2.0 * (xz + wy), 0.0],
            [2.0 * (xy + wz), 1.0 - 2.0 * (xx + zz), 2.0 * (yz - wx), 0.0],
            [2.0 * (xz - wy), 2.0 * (yz + wx), 1.0 - 2.0 * (xx + yy), 0.0],
            [t.x, t.y, t.z, 1.0],
        ])
    }

    /// `Fquaternion::set(matrix)`.
    #[must_use]
    pub fn to_quat(&self) -> Quat {
        const TOLERANCE: f32 = 0.1;
        let m = &self.0;
        let (m11, m12, m13) = (m[0][0], m[0][1], m[0][2]);
        let (m21, m22, m23) = (m[1][0], m[1][1], m[1][2]);
        let (m31, m32, m33) = (m[2][0], m[2][1], m[2][2]);
        // the quaternion keeps what it held when no branch produces a good root; it starts at zero here
        let (mut x, mut y, mut z, mut w) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let trace = m11 + m22 + m33;
        if trace > 0.0 {
            let mut s = (trace + 1.0).sqrt();
            w = s * 0.5;
            s = 0.5 / s;
            x = (m32 - m23) * s;
            y = (m13 - m31) * s;
            z = (m21 - m12) * s;
            return Quat::from_xyzw(x, y, z, w);
        }
        let try_a = |x: &mut f32, y: &mut f32, z: &mut f32, w: &mut f32| -> bool {
            let mut s = (m11 - (m22 + m33) + 1.0).sqrt();
            if s > TOLERANCE {
                *x = s * 0.5;
                s = 0.5 / s;
                *w = (m32 - m23) * s;
                *y = (m12 + m21) * s;
                *z = (m13 + m31) * s;
                return true;
            }
            false
        };
        let try_e = |x: &mut f32, y: &mut f32, z: &mut f32, w: &mut f32| -> bool {
            let mut s = (m22 - (m33 + m11) + 1.0).sqrt();
            if s > TOLERANCE {
                *y = s * 0.5;
                s = 0.5 / s;
                *w = (m13 - m31) * s;
                *z = (m23 + m32) * s;
                *x = (m21 + m12) * s;
                return true;
            }
            false
        };
        let try_i = |x: &mut f32, y: &mut f32, z: &mut f32, w: &mut f32| -> bool {
            let mut s = (m33 - (m11 + m22) + 1.0).sqrt();
            if s > TOLERANCE {
                *z = s * 0.5;
                s = 0.5 / s;
                *w = (m21 - m12) * s;
                *x = (m31 + m13) * s;
                *y = (m32 + m23) * s;
                return true;
            }
            false
        };
        #[derive(Clone, Copy)]
        enum Big {
            A,
            E,
            I,
        }
        let biggest = if m11 > m22 {
            if m33 > m11 { Big::I } else { Big::A }
        } else if m33 > m11 {
            Big::I
        } else {
            Big::E
        };
        let _ = match biggest {
            Big::A => try_a(&mut x, &mut y, &mut z, &mut w) || try_i(&mut x, &mut y, &mut z, &mut w) || try_e(&mut x, &mut y, &mut z, &mut w),
            Big::E => try_e(&mut x, &mut y, &mut z, &mut w) || try_i(&mut x, &mut y, &mut z, &mut w) || try_a(&mut x, &mut y, &mut z, &mut w),
            Big::I => try_i(&mut x, &mut y, &mut z, &mut w) || try_a(&mut x, &mut y, &mut z, &mut w) || try_e(&mut x, &mut y, &mut z, &mut w),
        };
        Quat::from_xyzw(x, y, z, w)
    }
}

/// `Fquaternion::slerp`, with the engine's polynomial arc cosine.
#[must_use]
pub fn slerp(q0: Quat, q1: Quat, tm: f32) -> Quat {
    fn acos_approx(x: f32) -> f32 {
        const C1: f32 = 0.892_399;
        const C3: f32 = 1.693_204;
        const C5: f32 = -3.853_735;
        const C7: f32 = 2.838_933;
        let x2 = x * x;
        let d = x * (C1 + x2 * (C3 + x2 * (C5 + x2 * C7)));
        std::f32::consts::FRAC_PI_2 - d
    }
    let mut cosom = (q0.w * q1.w) + (q0.x * q1.x) + (q0.y * q1.y) + (q0.z * q1.z);
    let sign = if cosom < 0.0 {
        cosom = -cosom;
        -1.0f32
    } else {
        1.0f32
    };
    let (scale0, mut scale1);
    if (1.0 - cosom) > EPS {
        let omega = acos_approx(cosom);
        let i_sinom = 1.0 / omega.sin();
        let t_omega = tm * omega;
        scale0 = (omega - t_omega).sin() * i_sinom;
        scale1 = t_omega.sin() * i_sinom;
    } else {
        scale0 = 1.0 - tm;
        scale1 = tm;
    }
    scale1 *= sign;
    Quat::from_xyzw(
        scale0 * q0.x + scale1 * q1.x,
        scale0 * q0.y + scale1 * q1.y,
        scale0 * q0.z + scale1 * q1.z,
        scale0 * q0.w + scale1 * q1.w,
    )
}

/// `Fvector::lerp`.
#[must_use]
pub fn lerp(p1: Vec3, p2: Vec3, t: f32) -> Vec3 {
    let invt = 1.0 - t;
    Vec3::new(p1.x * invt + p2.x * t, p1.y * invt + p2.y * t, p1.z * invt + p2.z * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inverse_undoes_a_transform() {
        let m = {
            let mut m = Mat4::xyz_i(Vec3::new(0.3, -0.7, 1.1));
            m.set_c(Vec3::new(1.0, 2.0, 3.0));
            m
        };
        let i = Mat4::invert_43(&m);
        let p = Vec3::new(0.5, -4.0, 2.5);
        let back = i.transform_tiny(m.transform_tiny(p));
        assert!(similar(back, p, 1e-4));
        let id = Mat4::mul_43(&m, &i);
        assert!(similar(id.c(), Vec3::ZERO, 1e-4));
    }

    #[test]
    fn quaternion_round_trips_a_rotation() {
        let m = Mat4::hpb(0.4, 0.2, -0.9);
        let q = m.to_quat();
        let back = Mat4::from_quat(q, Vec3::ZERO);
        for r in 0..3 {
            assert!(similar(back.row(r), m.row(r), 1e-4));
        }
        assert!((slerp(q, q, 0.5).w - q.w).abs() < 1e-5);
    }

    #[test]
    fn box_sphere_and_truncation() {
        let mut b = Aabb::INVALID;
        b.modify(Vec3::new(-1.0, -1.0, -1.0));
        b.modify(Vec3::new(1.0, 1.0, 1.0));
        let (c, r) = b.sphere();
        assert_eq!(c, Vec3::ZERO);
        assert!((r - 3.0f32.sqrt()).abs() < 1e-6);
        assert_eq!(cvtt(0.0 / 0.0f32.max(0.0)), i32::MIN);
        assert_eq!(cvtt(-2.9), -2);
    }
}
