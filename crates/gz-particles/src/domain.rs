//! `pDomain` (`particle_core.cpp`): a region of space that positions, velocities and the rest
//! are drawn from or tested against. The file stores domains already built, so only `Within`,
//! `Generate` and the transforms are needed.

use glam::{Mat4, Vec3};

use crate::rand::Rngs;

/// `PDomainEnum`.
pub mod kind {
    pub const POINT: u32 = 0;
    pub const LINE: u32 = 1;
    pub const TRIANGLE: u32 = 2;
    pub const PLANE: u32 = 3;
    pub const BOX: u32 = 4;
    pub const SPHERE: u32 = 5;
    pub const CYLINDER: u32 = 6;
    pub const CONE: u32 = 7;
    pub const BLOB: u32 = 8;
    pub const DISC: u32 = 9;
    pub const RECTANGLE: u32 = 10;
}

/// A domain as the engine holds it: the meaning of the fields depends on the kind.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Domain {
    pub kind: u32,
    pub p1: Vec3,
    pub p2: Vec3,
    pub u: Vec3,
    pub v: Vec3,
    pub radius1: f32,
    pub radius2: f32,
    pub radius1_sqr: f32,
    pub radius2_sqr: f32,
}

impl From<&gz_fmt_xr::Domain> for Domain {
    fn from(d: &gz_fmt_xr::Domain) -> Self {
        Self {
            kind: d.kind,
            p1: d.p1,
            p2: d.p2,
            u: d.u,
            v: d.v,
            radius1: d.radius1,
            radius2: d.radius2,
            radius1_sqr: d.radius1_sqr,
            radius2_sqr: d.radius2_sqr,
        }
    }
}

/// `Fvector::normalize_safe`: left as it is when its length is no number above zero.
pub(crate) fn normalize_safe(v: Vec3) -> Vec3 {
    let m = v.length_squared();
    if m > f32::MIN_POSITIVE { v * (1.0 / m).sqrt() } else { v }
}

/// `Fbox::xform`: the box carried by a matrix, by its three edges.
fn xform_box(min: Vec3, max: Vec3, m: &Mat4) -> (Vec3, Vec3) {
    let vx = m.x_axis.truncate() * (max.x - min.x);
    let vy = m.y_axis.truncate() * (max.y - min.y);
    let vz = m.z_axis.truncate() * (max.z - min.z);
    let mut lo = m.transform_point3(min);
    let mut hi = lo;
    for e in [vx, vy, vz] {
        for a in 0..3 {
            if e[a] < 0.0 {
                lo[a] += e[a];
            } else {
                hi[a] += e[a];
            }
        }
    }
    (lo, hi)
}

impl Domain {
    /// `Within`: whether `pos` is inside; kinds that have no inside say no.
    pub(crate) fn within(&self, pos: Vec3, rng: &mut Rngs) -> bool {
        match self.kind {
            kind::BOX => {
                !(pos.x < self.p1.x
                    || pos.x > self.p2.x
                    || pos.y < self.p1.y
                    || pos.y > self.p2.y
                    || pos.z < self.p1.z
                    || pos.z > self.p2.z)
            }
            kind::PLANE => pos.dot(self.p2) >= -self.radius1,
            kind::SPHERE => {
                let r = (pos - self.p1).length_squared();
                r <= self.radius1_sqr && r >= self.radius2_sqr
            }
            kind::CYLINDER | kind::CONE => {
                let x = pos - self.p1;
                // radius2_sqr holds 1 / (p2 . p2)
                let dist = self.p2.dot(x) * self.radius2_sqr;
                if !(0.0..=1.0).contains(&dist) {
                    return false;
                }
                let r = (x - self.p2 * dist).length_squared();
                if self.kind == kind::CONE {
                    r <= (dist * self.radius1) * (dist * self.radius1)
                        && r >= (dist * self.radius2) * (dist * self.radius2)
                } else {
                    r <= self.radius1_sqr && r >= self.radius2 * self.radius2
                }
            }
            kind::BLOB => {
                let x = pos - self.p1;
                let g = (x.length_squared() * self.radius2_sqr).exp() * self.radius2;
                rng.drand() < g
            }
            _ => false,
        }
    }

    /// `Generate`: a point drawn from the domain.
    pub(crate) fn generate(&self, rng: &mut Rngs) -> Vec3 {
        match self.kind {
            kind::POINT | kind::PLANE => self.p1,
            kind::LINE => self.p1 + self.p2 * rng.drand(),
            kind::BOX => {
                let x = self.p1.x + (self.p2.x - self.p1.x) * rng.drand();
                let y = self.p1.y + (self.p2.y - self.p1.y) * rng.drand();
                let z = self.p1.z + (self.p2.z - self.p1.z) * rng.drand();
                Vec3::new(x, y, z)
            }
            kind::TRIANGLE => {
                let r1 = rng.drand();
                let r2 = rng.drand();
                if r1 + r2 < 1.0 {
                    self.p1 + self.u * r1 + self.v * r2
                } else {
                    self.p1 + self.u * (1.0 - r1) + self.v * (1.0 - r2)
                }
            }
            kind::RECTANGLE => {
                let a = rng.drand();
                let b = rng.drand();
                self.p1 + self.u * a + self.v * b
            }
            kind::SPHERE => {
                // `pVector(drand48(), drand48(), drand48())`: MSVC evaluates the arguments last
                // to first
                let z = rng.drand();
                let y = rng.drand();
                let x = rng.drand();
                let dir = normalize_safe(Vec3::new(x, y, z) - Vec3::splat(0.5));
                if self.radius1 == self.radius2 {
                    self.p1 + dir * self.radius1
                } else {
                    let t = rng.drand();
                    self.p1 + dir * (self.radius2 + t * (self.radius1 - self.radius2))
                }
            }
            kind::CYLINDER | kind::CONE => {
                let dist = rng.drand();
                let theta = rng.drand() * 2.0 * std::f32::consts::PI;
                let r = self.radius2 + rng.drand() * (self.radius1 - self.radius2);
                let (mut x, mut y) = (r * theta.cos(), r * theta.sin());
                if self.kind == kind::CONE {
                    x *= dist;
                    y *= dist;
                }
                self.p1 + self.p2 * dist + self.u * x + self.v * y
            }
            kind::BLOB => {
                let x = self.p1.x + rng.nrand(self.radius1);
                let y = self.p1.y + rng.nrand(self.radius1);
                let z = self.p1.z + rng.nrand(self.radius1);
                Vec3::new(x, y, z)
            }
            kind::DISC => {
                let theta = rng.drand() * 2.0 * std::f32::consts::PI;
                let r = self.radius2 + rng.drand() * (self.radius1 - self.radius2);
                self.p1 + self.u * (r * theta.cos()) + self.v * (r * theta.sin())
            }
            _ => Vec3::ZERO,
        }
    }

    /// `transform`: `src` carried by `m` into this domain (the kind is `src`'s).
    pub(crate) fn transformed(src: &Domain, m: &Mat4) -> Domain {
        let mut d = *src;
        match src.kind {
            kind::BOX => {
                let (lo, hi) = xform_box(src.p1, src.p2, m);
                d.p1 = lo;
                d.p2 = hi;
            }
            kind::PLANE => {
                d.p1 = m.transform_point3(src.p1);
                d.p2 = m.transform_vector3(src.p2);
                // radius1 holds the d of the plane's equation
                d.radius1 = -d.p1.dot(d.p2);
            }
            kind::SPHERE | kind::BLOB | kind::POINT => d.p1 = m.transform_point3(src.p1),
            kind::LINE => {
                d.p1 = m.transform_point3(src.p1);
                d.p2 = m.transform_vector3(src.p2);
            }
            kind::CYLINDER | kind::CONE | kind::RECTANGLE | kind::TRIANGLE | kind::DISC => {
                d.p1 = m.transform_point3(src.p1);
                d.p2 = m.transform_vector3(src.p2);
                d.u = m.transform_vector3(src.u);
                d.v = m.transform_vector3(src.v);
            }
            // the engine stops here (NODEFAULT); a kind it does not know stays where it is
            _ => {}
        }
        d
    }

    /// `transform_dir`: the same without the translation.
    pub(crate) fn transformed_dir(src: &Domain, m: &Mat4) -> Domain {
        let mut r = *m;
        r.w_axis = glam::Vec4::new(0.0, 0.0, 0.0, m.w_axis.w);
        Self::transformed(src, &r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(p: Vec3) -> Domain {
        Domain { kind: kind::POINT, p1: p, ..Domain::default() }
    }

    #[test]
    fn generate_draws_in_the_engines_order() {
        let mut rng = Rngs::default();
        let b = Domain { kind: kind::BOX, p1: Vec3::ZERO, p2: Vec3::new(2.0, 4.0, 8.0), ..Domain::default() };
        let p = b.generate(&mut rng);
        // x, y, z from the first three draws: 41, 18467, 6334
        let f = |v: i32| v as f32 / 32767.0;
        assert!((p - Vec3::new(2.0 * f(41), 4.0 * f(18467), 8.0 * f(6334))).abs().max_element() < 1e-5);
        assert!(b.within(Vec3::new(1.0, 1.0, 1.0), &mut rng));
        assert!(!b.within(Vec3::new(3.0, 1.0, 1.0), &mut rng));
        let mut rng = Rngs::default();
        assert_eq!(point(Vec3::ONE).generate(&mut rng), Vec3::ONE);
        // a point draws nothing
        assert_eq!(rng, Rngs::default());
    }

    #[test]
    fn spheres_cones_and_planes() {
        let mut rng = Rngs::default();
        let s = Domain {
            kind: kind::SPHERE,
            p1: Vec3::ZERO,
            radius1: 2.0,
            radius2: 1.0,
            radius1_sqr: 4.0,
            radius2_sqr: 1.0,
            ..Domain::default()
        };
        for _ in 0..50 {
            let p = s.generate(&mut rng);
            let r = p.length();
            assert!((1.0 - 1e-4..=2.0 + 1e-4).contains(&r), "{r}");
        }
        assert!(s.within(Vec3::new(1.5, 0.0, 0.0), &mut rng));
        assert!(!s.within(Vec3::new(0.5, 0.0, 0.0), &mut rng));
        let plane =
            Domain { kind: kind::PLANE, p1: Vec3::ZERO, p2: Vec3::Y, radius1: 0.0, ..Domain::default() };
        assert!(plane.within(Vec3::new(0.0, 1.0, 0.0), &mut rng));
        assert!(!plane.within(Vec3::new(0.0, -1.0, 0.0), &mut rng));
        // a plane moved up by 3: d follows
        let m = Mat4::from_translation(Vec3::new(0.0, 3.0, 0.0));
        let moved = Domain::transformed(&plane, &m);
        assert!((moved.radius1 + 3.0).abs() < 1e-6);
        assert!(!moved.within(Vec3::new(0.0, 2.0, 0.0), &mut rng));
        // a direction ignores the translation
        let line = Domain { kind: kind::LINE, p1: Vec3::ONE, p2: Vec3::X, ..Domain::default() };
        let d = Domain::transformed_dir(&line, &m);
        assert_eq!(d.p1, Vec3::ONE);
    }

    #[test]
    fn boxes_follow_the_engines_xform() {
        let b = Domain {
            kind: kind::BOX,
            p1: Vec3::new(-1.0, 0.0, -1.0),
            p2: Vec3::new(1.0, 2.0, 1.0),
            ..Domain::default()
        };
        // a quarter turn about y and a move
        let m = Mat4::from_translation(Vec3::new(10.0, 0.0, 0.0))
            * Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let t = Domain::transformed(&b, &m);
        assert!((t.p1 - Vec3::new(9.0, 0.0, -1.0)).abs().max_element() < 1e-5);
        assert!((t.p2 - Vec3::new(11.0, 2.0, 1.0)).abs().max_element() < 1e-5);
    }
}
