//! Shapes and what is built on them: sound environments and fog volumes.

use gz_base::SmolStr;

use crate::math::{Aabb, Mat4, Transform, Vec3, ray_box};

pub const CF_SPHERE: u8 = 0;
pub const CF_BOX: u8 = 1;
/// The shape flags: common and level bound.
pub const SHAPE_COMMON: u8 = 0;
pub const SHAPE_LEVEL_BOUND: u8 = 1;

/// A type and a 64-byte union (a sphere's centre and radius in its
/// first four floats, or a box matrix). The three padding bytes after the type and the unused
/// floats of a sphere are kept, so a binary part writes back byte for byte.
#[derive(Clone, Debug, PartialEq)]
pub struct ShapeDef {
    pub kind: u8,
    pub pad: [u8; 3],
    pub data: [f32; 16],
}

impl ShapeDef {
    #[must_use]
    pub fn sphere(center: Vec3, radius: f32) -> Self {
        let mut data = [0.0; 16];
        data[0] = center.x;
        data[1] = center.y;
        data[2] = center.z;
        data[3] = radius;
        Self { kind: CF_SPHERE, pad: [0; 3], data }
    }

    #[must_use]
    pub fn boxed(m: &Mat4) -> Self {
        let mut data = [0.0; 16];
        for (r, row) in m.0.iter().enumerate() {
            data[r * 4..r * 4 + 4].copy_from_slice(row);
        }
        Self { kind: CF_BOX, pad: [0; 3], data }
    }

    #[must_use]
    pub fn center(&self) -> Vec3 {
        Vec3::new(self.data[0], self.data[1], self.data[2])
    }

    #[must_use]
    pub fn radius(&self) -> f32 {
        self.data[3]
    }

    #[must_use]
    pub fn matrix(&self) -> Mat4 {
        let mut m = Mat4::IDENTITY;
        for (r, row) in m.0.iter_mut().enumerate() {
            row.copy_from_slice(&self.data[r * 4..r * 4 + 4]);
        }
        m
    }

    pub(crate) fn set_row(&mut self, r: usize, v: Vec3) {
        self.data[r * 4] = v.x;
        self.data[r * 4 + 1] = v.y;
        self.data[r * 4 + 2] = v.z;
    }
}

/// `du_box_vertices`: the unit box's corners.
fn unit_corners() -> [Vec3; 8] {
    let h = 0.5;
    std::array::from_fn(|i| {
        Vec3::new(
            if i & 1 == 0 { -h } else { h },
            if i & 2 == 0 { -h } else { h },
            if i & 4 == 0 { -h } else { h },
        )
    })
}

/// The shape object's own data.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShapeData {
    pub shapes: Vec<ShapeDef>,
    /// What the shape is for the game: a common shape or a level bound.
    pub usage: u8,
}

impl ShapeData {
    /// One box of the identity (`add_box(Fidentity)`): sound environments and fog volumes.
    #[must_use]
    pub fn unit_box() -> Self {
        Self { shapes: vec![ShapeDef::boxed(&Mat4::IDENTITY)], usage: SHAPE_COMMON }
    }

    /// The local box.
    #[must_use]
    pub fn local_box(&self) -> Aabb {
        let mut b = Aabb::INVALID;
        for s in &self.shapes {
            if s.kind == CF_SPHERE {
                b.modify(s.center() - Vec3::splat(s.radius()));
                b.modify(s.center() + Vec3::splat(s.radius()));
            } else {
                let m = s.matrix();
                for c in unit_corners() {
                    b.modify(m.transform_tiny(c));
                }
            }
        }
        b
    }

    /// The local box carried by the object's transform.
    #[must_use]
    pub fn world_box(&self, xf: &Transform) -> Option<Aabb> {
        let b = self.local_box();
        b.is_valid().then(|| crate::math::xform_box(&b, &xf.matrix()))
    }

    /// One sphere scales evenly, one box on each axis, several evenly.
    #[must_use]
    pub fn constrain_scale(&self, v: Vec3) -> Vec3 {
        match self.shapes.as_slice() {
            [s] if s.kind == CF_BOX => v,
            _ => Vec3::splat(v.x),
        }
    }

    /// The shape's ray pick.
    #[must_use]
    pub fn ray_pick(&self, xf: &Transform, start: Vec3, dir: Vec3, distance: f32) -> Option<f32> {
        let inv = xf.inverse();
        let inv_r = Mat4::invert_43(&crate::math::rotation_matrix(xf.rotation));
        let mut dist = distance;
        for s in &self.shapes {
            if s.kind == CF_SPHERE {
                let sp = inv.transform_tiny(start);
                let d = inv_r.transform_dir(dir);
                if let Some(t) = ray_sphere(sp, d, s.center(), s.radius())
                    && t > 0.0
                    && t < dist
                {
                    dist = t;
                }
            } else {
                let bi = Mat4::invert_43(&s.matrix());
                let s1 = bi.transform_tiny(inv.transform_tiny(start));
                let d1 = bi.transform_dir(inv.transform_dir(dir));
                let unit = Aabb::new(Vec3::splat(-0.5), Vec3::splat(0.5));
                if let Some((t, false)) = ray_box(s1, d1, &unit) {
                    let p = xf.matrix().transform_tiny(s.matrix().transform_tiny(s1 + d1 * t));
                    let d = (p - start).length();
                    if d < dist {
                        dist = d;
                    }
                }
            }
        }
        (dist < distance).then_some(dist)
    }

    /// The object's scale moved into the shapes.
    pub fn apply_scale(&mut self, xf: &mut Transform) {
        let s = crate::math::scale_matrix(xf.scale);
        for sh in &mut self.shapes {
            if sh.kind == CF_SPHERE {
                let c = s.transform_tiny(sh.center());
                sh.data[0] = c.x;
                sh.data[1] = c.y;
                sh.data[2] = c.z;
                sh.data[3] *= xf.scale.x;
            } else {
                let m = Mat4::mul_43(&s, &sh.matrix());
                *sh = ShapeDef { pad: sh.pad, ..ShapeDef::boxed(&m) };
            }
        }
        xf.scale = Vec3::ONE;
    }
}

/// The first distance along a unit ray to a sphere.
fn ray_sphere(start: Vec3, dir: Vec3, c: Vec3, r: f32) -> Option<f32> {
    let q = c - start;
    let cl = q.length();
    let v = q.dot(dir);
    let d = r * r - (cl * cl - v * v);
    if d < 0.0 {
        return None;
    }
    let s = d.sqrt();
    let t = if v - s > 0.0 { v - s } else { v + s };
    (t > 0.0).then_some(t)
}

/// A box with the environments inside and outside it.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundEnvData {
    pub shape: ShapeData,
    pub inner: SmolStr,
    pub outer: SmolStr,
}

impl Default for SoundEnvData {
    fn default() -> Self {
        Self { shape: ShapeData::unit_box(), inner: SmolStr::default(), outer: SmolStr::default() }
    }
}

impl SoundEnvData {}

/// The fog volume kinds: emitter and occlusion.
pub const FV_EMITTER: u8 = 0;
pub const FV_OCCLUSION: u8 = 1;

#[derive(Clone, Debug, PartialEq)]
pub struct FogData {
    /// A fog volume reads and writes through the common object part, not the shape's, so its
    /// shape is never saved: every fog volume is the unit box it is made with.
    pub shape: ShapeData,
    pub kind: u8,
    pub group_id: u32,
    pub profile: SmolStr,
}

impl Default for FogData {
    fn default() -> Self {
        Self {
            shape: ShapeData::unit_box(),
            kind: FV_EMITTER,
            group_id: u32::MAX,
            profile: SmolStr::new_static("environment\\fog\\default.ltx"),
        }
    }
}

impl FogData {}
