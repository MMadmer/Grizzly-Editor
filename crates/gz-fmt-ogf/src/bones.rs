//! Bone data of a skeleton: `OGF_S_BONE_NAMES` and `OGF_S_IKDATA` (`SkeletonCustom.cpp`,
//! `CBone::ExportOGF`, `SJointIKData::Export` / `Import`).

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use gz_fmt_object::BoneShape;
use smol_str::SmolStr;

use crate::Result;

/// `Fobb`: three axes, the centre, the half sizes (60 bytes).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Obb {
    pub rotate: [Vec3; 3],
    pub translate: Vec3,
    pub halfsize: Vec3,
}

impl Default for Obb {
    fn default() -> Self {
        Self { rotate: [Vec3::X, Vec3::Y, Vec3::Z], translate: Vec3::ZERO, halfsize: Vec3::ZERO }
    }
}

impl Obb {
    /// `Fobb::invalidate`.
    pub const INVALID: Obb =
        Obb { rotate: [Vec3::X, Vec3::Y, Vec3::Z], translate: Vec3::ZERO, halfsize: Vec3::ZERO };

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            rotate: [r.r_vec3()?, r.r_vec3()?, r.r_vec3()?],
            translate: r.r_vec3()?,
            halfsize: r.r_vec3()?,
        })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        for a in self.rotate {
            w.w_vec3(a);
        }
        w.w_vec3(self.translate);
        w.w_vec3(self.halfsize);
    }
}

/// One record of `OGF_S_BONE_NAMES`.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneName {
    pub name: SmolStr,
    /// Empty for the root.
    pub parent: SmolStr,
    pub obb: Obb,
}

/// One record of `OGF_S_IKDATA`, in the file's form: the limits are already turned for ODE
/// (`min = -limit.y`, `max = -limit.x`).
#[derive(Clone, Debug, PartialEq)]
pub struct BoneIk {
    pub version: u32,
    pub game_mtl: SmolStr,
    pub shape: BoneShape,
    pub joint_type: u32,
    /// Per axis: min, max, spring, damping.
    pub limits: [[f32; 4]; 3],
    pub spring_factor: f32,
    pub damping_factor: f32,
    pub flags: u32,
    pub break_force: f32,
    pub break_torque: f32,
    /// Version 1 and later.
    pub friction: f32,
    pub rest_rotate: Vec3,
    pub rest_offset: Vec3,
    pub mass: f32,
    pub center_of_mass: Vec3,
}

/// `OGF_IKDATA_VERSION`.
pub const IKDATA_VERSION: u32 = 1;

impl BoneIk {
    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        let version = r.r_u32()?;
        let game_mtl = SmolStr::new(r.r_stringz()?);
        let shape = BoneShape::read(r).map_err(|e| crate::invalid(e.to_string()))?;
        let joint_type = r.r_u32()?;
        let mut limits = [[0.0f32; 4]; 3];
        for l in &mut limits {
            for v in l.iter_mut() {
                *v = r.r_f32()?;
            }
        }
        let spring_factor = r.r_f32()?;
        let damping_factor = r.r_f32()?;
        let flags = r.r_u32()?;
        let break_force = r.r_f32()?;
        let break_torque = r.r_f32()?;
        // `Import(F, vers)`: the version word is read as u16
        let friction = if version as u16 > 0 { r.r_f32()? } else { 0.0 };
        Ok(Self {
            version,
            game_mtl,
            shape,
            joint_type,
            limits,
            spring_factor,
            damping_factor,
            flags,
            break_force,
            break_torque,
            friction,
            rest_rotate: r.r_vec3()?,
            rest_offset: r.r_vec3()?,
            mass: r.r_f32()?,
            center_of_mass: r.r_vec3()?,
        })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_u32(self.version);
        w.w_stringz(&self.game_mtl);
        self.shape.write(w);
        w.w_u32(self.joint_type);
        for l in &self.limits {
            for v in l {
                w.w_f32(*v);
            }
        }
        w.w_f32(self.spring_factor);
        w.w_f32(self.damping_factor);
        w.w_u32(self.flags);
        w.w_f32(self.break_force);
        w.w_f32(self.break_torque);
        if self.version as u16 > 0 {
            w.w_f32(self.friction);
        }
        w.w_vec3(self.rest_rotate);
        w.w_vec3(self.rest_offset);
        w.w_f32(self.mass);
        w.w_vec3(self.center_of_mass);
    }

    /// `CBone::ExportOGF` of a source bone.
    #[must_use]
    pub fn from_bone(b: &gz_fmt_object::Bone) -> Self {
        let ik = &b.ik;
        let mut limits = [[0.0f32; 4]; 3];
        for (out, l) in limits.iter_mut().zip(&ik.limits) {
            *out = [-l.limit.y, -l.limit.x, l.spring_factor, l.damping_factor];
        }
        Self {
            version: IKDATA_VERSION,
            game_mtl: b.game_mtl.clone(),
            shape: b.shape,
            joint_type: ik.kind,
            limits,
            spring_factor: ik.spring_factor,
            damping_factor: ik.damping_factor,
            flags: ik.flags,
            break_force: ik.break_force,
            break_torque: ik.break_torque,
            friction: ik.friction,
            rest_rotate: b.rest_rotate,
            rest_offset: b.rest_offset,
            mass: b.mass,
            center_of_mass: b.center_of_mass,
        }
    }
}

/// `OGF_S_BONE_NAMES`.
pub(crate) fn read_names(body: &[u8]) -> Result<Vec<BoneName>> {
    let mut r = Reader::new(body);
    let n = r.r_u32()? as usize;
    // a name, a parent and 60 bytes at least
    if n.saturating_mul(62) > r.elapsed() {
        return Err(crate::invalid(format!("the model states {n} bones and cannot hold them")));
    }
    (0..n)
        .map(|_| {
            Ok(BoneName {
                name: SmolStr::new(r.r_stringz()?),
                parent: SmolStr::new(r.r_stringz()?),
                obb: Obb::read(&mut r)?,
            })
        })
        .collect()
}

pub(crate) fn write_names(names: &[BoneName], w: &mut Writer) {
    w.w_u32(names.len() as u32);
    for b in names {
        w.w_stringz(&b.name);
        w.w_stringz(&b.parent);
        b.obb.write(w);
    }
}

/// `OGF_S_IKDATA`: one record per bone.
pub(crate) fn read_ik(body: &[u8], bones: usize) -> Result<Vec<BoneIk>> {
    let mut r = Reader::new(body);
    if bones.saturating_mul(BoneShape::SIZE) > r.elapsed() {
        return Err(crate::invalid("the model's bone data is shorter than its bones"));
    }
    (0..bones).map(|_| BoneIk::read(&mut r)).collect()
}
