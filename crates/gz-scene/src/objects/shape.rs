//! Shapes and what is built on them: sound environments and fog volumes (`EShape.cpp`,
//! `ESound_Environment.cpp`, `ESceneFogVolumeTools.cpp`).

use gz_base::SmolStr;
use gz_chunk::{Reader, Writer};

use crate::SceneError;
use crate::ltx::{IniReader, IniWriter};
use crate::math::{Aabb, Mat4, Transform, Vec3, ray_box};
use crate::objects::simple::Body;

pub const SHAPE_VERSION: u32 = 2;
pub const CF_SPHERE: u8 = 0;
pub const CF_BOX: u8 = 1;
/// `eShapeCommon` / `eShapeLevelBound`.
pub const SHAPE_COMMON: u8 = 0;
pub const SHAPE_LEVEL_BOUND: u8 = 1;

/// `CShapeData::shape_def`: a type and a 64-byte union (a sphere's centre and radius in its
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

    fn set_row(&mut self, r: usize, v: Vec3) {
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

/// `CEditShape`'s own data.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ShapeData {
    pub shapes: Vec<ShapeDef>,
    /// `m_shape_type`.
    pub usage: u8,
}

impl ShapeData {
    /// One box of the identity (`add_box(Fidentity)`): sound environments and fog volumes.
    #[must_use]
    pub fn unit_box() -> Self {
        Self { shapes: vec![ShapeDef::boxed(&Mat4::IDENTITY)], usage: SHAPE_COMMON }
    }

    /// `ComputeBounds`: the local box.
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

    /// `GetBox`: the local box carried by the object's transform.
    #[must_use]
    pub fn world_box(&self, xf: &Transform) -> Option<Aabb> {
        let b = self.local_box();
        b.is_valid().then(|| crate::math::xform_box(&b, &xf.matrix()))
    }

    /// `CEditShape::SetScale`: one sphere scales evenly, one box on each axis, several evenly.
    #[must_use]
    pub fn constrain_scale(&self, v: Vec3) -> Vec3 {
        match self.shapes.as_slice() {
            [s] if s.kind == CF_BOX => v,
            _ => Vec3::splat(v.x),
        }
    }

    /// `CEditShape::RayPick`.
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

    /// `ApplyScale`: the object's scale moved into the shapes.
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

    /// The `shape_*` keys (`CEditShape::LoadLTX` after the common part).
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let count = r.r_u32(s, "shapes_count")?;
        let usage = if version > 1 { r.r_u8(s, "shape_type")? } else { SHAPE_COMMON };
        let mut shapes = Vec::new();
        for i in 0..count.min(1 << 16) {
            let kind = r.r_u8(s, &format!("shape_type_{i}"))?;
            if kind == CF_SPHERE {
                let c = r.r_fvector3(s, &format!("shape_center_{i}"))?;
                let rad = r.r_float(s, &format!("shape_radius_{i}"))?;
                shapes.push(ShapeDef::sphere(c, rad));
            } else {
                let mut d = ShapeDef::boxed(&Mat4::IDENTITY);
                d.kind = kind;
                for (row, key) in ["i", "j", "k", "c"].iter().enumerate() {
                    d.set_row(row, r.r_fvector3(s, &format!("shape_matrix_{key}_{i}"))?);
                }
                shapes.push(d);
            }
        }
        Ok(Self { shapes, usage })
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", SHAPE_VERSION);
        w.w_u32(s, "shapes_count", u32::try_from(self.shapes.len()).unwrap_or(0));
        w.w_u8(s, "shape_type", self.usage);
        for (i, sh) in self.shapes.iter().enumerate() {
            w.w_u8(s, &format!("shape_type_{i}"), sh.kind);
            if sh.kind == CF_SPHERE {
                w.w_fvector3(s, &format!("shape_center_{i}"), sh.center());
                w.w_float(s, &format!("shape_radius_{i}"), sh.radius());
            } else {
                let m = sh.matrix();
                for (row, key) in ["i", "j", "k", "c"].iter().enumerate() {
                    w.w_fvector3(s, &format!("shape_matrix_{key}_{i}"), m.row(row));
                }
            }
        }
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        if r.open_chunk(0)?.is_none() {
            return Ok(Body::Refused("CEditShape: no version".into()));
        }
        let Some(c) = r.open_chunk(1)? else {
            return Ok(Body::Refused("CEditShape: no shapes".into()));
        };
        let mut q = c.reader();
        let n = q.r_u32()? as usize;
        if n.saturating_mul(68) > q.len() {
            return Err(SceneError::load("CEditShape: the shape list is cut short"));
        }
        let mut shapes = Vec::with_capacity(n);
        for _ in 0..n {
            let kind = q.r_u8()?;
            let pad = [q.r_u8()?, q.r_u8()?, q.r_u8()?];
            let mut data = [0.0; 16];
            for d in &mut data {
                *d = q.r_f32()?;
            }
            shapes.push(ShapeDef { kind, pad, data });
        }
        let usage = match r.open_chunk(2)? {
            Some(c) => c.reader().r_u8()?,
            None => SHAPE_COMMON,
        };
        Ok(Body::Read(Self { shapes, usage }))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0, |w| w.w_u16(SHAPE_VERSION as u16));
        w.chunk(1, |w| {
            w.w_u32(u32::try_from(self.shapes.len()).unwrap_or(0));
            for s in &self.shapes {
                w.w_u8(s.kind);
                w.w(&s.pad);
                for d in s.data {
                    w.w_f32(d);
                }
            }
        });
        w.chunk(2, |w| w.w_u8(self.usage));
    }
}

/// `Fsphere::intersect`: the first distance along a unit ray to a sphere.
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

pub const SOUND_ENV_VERSION: u32 = 0x12;

/// `ESoundEnvironment`: a box with the environments inside and outside it.
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

impl SoundEnvData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != SOUND_ENV_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        // oracle: the C++ LTX reader never reads env_inner / env_outer back, so an LTX scene
        // loses them on load; kept that way so both editors answer the same.
        Ok(Body::Read(Self {
            shape: ShapeData::read_ltx(r, s)?,
            inner: SmolStr::default(),
            outer: SmolStr::default(),
        }))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        self.shape.write_ltx(w, s);
        w.w_u32(s, "version", SOUND_ENV_VERSION);
        w.w_string(s, "env_inner", &self.inner);
        w.w_string(s, "env_outer", &self.outer);
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let v = match r.open_chunk(0x1001)? {
            Some(c) => c.reader().r_u16()?,
            None => 0,
        };
        if u32::from(v) != SOUND_ENV_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        let shape = match ShapeData::read_stream(r)? {
            Body::Read(s) => s,
            Body::Refused(w) => return Ok(Body::Refused(w)),
        };
        let Some(c) = r.open_chunk(0x1003)? else {
            return Ok(Body::Refused("ESoundEnvironment: no environments".into()));
        };
        let mut q = c.reader();
        Ok(Body::Read(Self { shape, inner: q.r_stringz()?.into(), outer: q.r_stringz()?.into() }))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        self.shape.write_stream(w);
        w.chunk(0x1001, |w| w.w_u16(SOUND_ENV_VERSION as u16));
        w.chunk(0x1003, |w| {
            w.w_stringz(&self.inner);
            w.w_stringz(&self.outer);
        });
    }
}

pub const FOG_VOL_VERSION: u32 = 2;
/// `fvEmitter` / `fvOcclusion`.
pub const FV_EMITTER: u8 = 0;
pub const FV_OCCLUSION: u8 = 1;

/// `EFogVolume`.
#[derive(Clone, Debug, PartialEq)]
pub struct FogData {
    /// oracle: `EFogVolume` reads and writes through `CCustomObject`, not `CEditShape`, so its
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

impl FogData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let mut d = Self::default();
        if version > 0 {
            d.kind = r.r_u8(s, "folume_type")?;
            d.group_id = r.r_u32(s, "group_id")?;
        }
        if version > 1 && d.kind == FV_EMITTER {
            d.profile = r.r_string(s, "profile")?.into();
        }
        Ok(d)
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", FOG_VOL_VERSION);
        w.w_u8(s, "folume_type", self.kind);
        w.w_u32(s, "group_id", self.group_id);
        if self.kind == FV_EMITTER {
            w.w_string(s, "profile", &self.profile);
        }
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let Some(v) = r.open_chunk(0x1002)? else {
            return Ok(Body::Refused("EFogVolume: no version".into()));
        };
        let version = v.reader().r_u16()?;
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0x1003)? {
            let mut q = c.reader();
            d.kind = q.r_u8()?;
            d.group_id = q.r_u32()?;
            if version > 1 && d.kind == FV_EMITTER {
                d.profile = q.r_stringz()?.into();
            }
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0x1002, |w| w.w_u16(FOG_VOL_VERSION as u16));
        w.chunk(0x1003, |w| {
            w.w_u8(self.kind);
            w.w_u32(self.group_id);
            if self.kind == FV_EMITTER {
                w.w_stringz(&self.profile);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shapes_round_trip_both_ways() {
        let mut s = ShapeData::unit_box();
        s.shapes.push(ShapeDef::sphere(Vec3::new(1.0, 2.0, 3.0), 4.0));
        s.shapes[1].data[9] = 7.0; // garbage of the union a binary file carries
        let mut w = Writer::new();
        s.write_stream(&mut w);
        let bytes = w.into_bytes();
        let Body::Read(back) = ShapeData::read_stream(&Reader::new(&bytes)).unwrap() else { panic!() };
        assert_eq!(back, s);

        let mut w = IniWriter::new();
        s.write_ltx(&mut w, "o");
        let r = IniReader::parse(&w.save(), "t").unwrap();
        let back = ShapeData::read_ltx(&r, "o").unwrap();
        assert_eq!(back.shapes[1].center(), Vec3::new(1.0, 2.0, 3.0));
        assert_eq!(back.shapes[0].matrix(), Mat4::IDENTITY);
    }

    #[test]
    fn a_ray_meets_a_box_shape() {
        let s = ShapeData::unit_box();
        let xf = Transform::at(Vec3::new(0.0, 0.0, 10.0));
        let d = s.ray_pick(&xf, Vec3::ZERO, Vec3::Z, f32::MAX).unwrap();
        assert!((d - 9.5).abs() < 1e-4);
    }
}
