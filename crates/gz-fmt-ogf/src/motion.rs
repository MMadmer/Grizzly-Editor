//! Skeletal motions: `OGF_S_SMPARAMS` (bone parts and motion definitions) and `OGF_S_MOTIONS`
//! (quantised keys), embedded in a model or alone in an `.omf`. `motions_value::load`,
//! `CMotionDef::Load`, `motion_marks`, and the sampling of `AnimationKeyCalculate.h`.

use std::sync::Arc;

use glam::{Quat, Vec3};
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::format::chunk;
use crate::{Result, invalid};

pub(crate) const FL_T_KEY_PRESENT: u8 = 1 << 0;
pub(crate) const FL_R_KEY_ABSENT: u8 = 1 << 1;
pub(crate) const FL_T_KEY_16: u8 = 1 << 2;
/// `KEY_QuantI`.
const KEY_QUANT_I: f32 = 1.0 / 32767.0;
/// `SAMPLE_FPS`: the engine plays every motion at thirty keys a second.
pub const SAMPLE_FPS: f32 = 30.0;
/// `xrOGF_SMParamsVersion`.
pub const SMPARAMS_VERSION: u16 = 4;

/// `CPartDef`: a named group of bones; each bone with the key slot it occupies.
#[derive(Clone, Debug, PartialEq)]
pub struct PartDef {
    pub name: SmolStr,
    pub bones: Vec<(SmolStr, u32)>,
}

/// `motion_marks`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionMark {
    pub name: SmolStr,
    pub intervals: Vec<(f32, f32)>,
}

/// One definition of `OGF_S_SMPARAMS`; the four floats are as stored (the engine quantises
/// them to 1/655.35 when it loads them).
#[derive(Clone, Debug, PartialEq)]
pub struct MotionDef {
    pub name: SmolStr,
    pub flags: u32,
    pub bone_or_part: u16,
    pub motion: u16,
    pub speed: f32,
    pub power: f32,
    pub accrue: f32,
    pub falloff: f32,
    /// Version 4 and later.
    pub marks: Vec<MotionMark>,
}

/// Rotation keys: one constant or one per frame, `x, y, z, w` over 32767.
#[derive(Clone, Debug, PartialEq)]
pub enum RotKeys {
    Constant([i16; 4]),
    Keys { crc: u32, keys: Arc<[[i16; 4]]> },
}

/// Translation keys: a constant, or per-frame offsets of 8 or 16 bits around `init` in units
/// of `size`.
#[derive(Clone, Debug, PartialEq)]
pub enum TransKeys {
    Constant(Vec3),
    Keys8 { crc: u32, keys: Arc<[[i8; 3]]>, size: Vec3, init: Vec3 },
    Keys16 { crc: u32, keys: Arc<[[i16; 3]]>, size: Vec3, init: Vec3 },
}

/// One bone's keys of a motion.
#[derive(Clone, Debug, PartialEq)]
pub struct BoneMotion {
    pub flags: u8,
    pub rot: RotKeys,
    pub trans: TransKeys,
}

/// One motion: its name, its length in frames and every bone's keys in key-slot order.
#[derive(Clone, Debug, PartialEq)]
pub struct Motion {
    pub name: SmolStr,
    pub length: u32,
    pub bones: Vec<BoneMotion>,
}

/// A bone's transform at a moment.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneKey {
    pub rotation: Quat,
    pub translation: Vec3,
}

/// Parts, definitions and keys.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionSet {
    /// `None` when the source had no `OGF_S_SMPARAMS`.
    pub params_version: Option<u16>,
    pub parts: Vec<PartDef>,
    pub defs: Vec<MotionDef>,
    pub motions: Vec<Motion>,
}

fn qr(k: [i16; 4]) -> Quat {
    Quat::from_xyzw(f32::from(k[0]) * KEY_QUANT_I, f32::from(k[1]) * KEY_QUANT_I, f32::from(k[2]) * KEY_QUANT_I, f32::from(k[3]) * KEY_QUANT_I)
}

impl BoneMotion {
    fn read(r: &mut Reader<'_>, len: usize) -> Result<Self> {
        let flags = r.r_u8()?;
        let r4 = |r: &mut Reader<'_>| -> Result<[i16; 4]> { Ok([r.r_i16()?, r.r_i16()?, r.r_i16()?, r.r_i16()?]) };
        let rot = if flags & FL_R_KEY_ABSENT != 0 {
            RotKeys::Constant(r4(r)?)
        } else {
            let crc = r.r_u32()?;
            if len.saturating_mul(8) > r.elapsed() {
                return Err(invalid("a motion's rotation keys are cut short"));
            }
            RotKeys::Keys { crc, keys: (0..len).map(|_| r4(r)).collect::<Result<Vec<_>>>()?.into() }
        };
        let trans = if flags & FL_T_KEY_PRESENT != 0 {
            let crc = r.r_u32()?;
            if flags & FL_T_KEY_16 != 0 {
                if len.saturating_mul(6) > r.elapsed() {
                    return Err(invalid("a motion's translation keys are cut short"));
                }
                let keys = (0..len).map(|_| Ok([r.r_i16()?, r.r_i16()?, r.r_i16()?])).collect::<Result<Vec<_>>>()?.into();
                TransKeys::Keys16 { crc, keys, size: r.r_vec3()?, init: r.r_vec3()? }
            } else {
                if len.saturating_mul(3) > r.elapsed() {
                    return Err(invalid("a motion's translation keys are cut short"));
                }
                let keys = (0..len).map(|_| Ok([r.r_i8()?, r.r_i8()?, r.r_i8()?])).collect::<Result<Vec<_>>>()?.into();
                TransKeys::Keys8 { crc, keys, size: r.r_vec3()?, init: r.r_vec3()? }
            }
        } else {
            TransKeys::Constant(r.r_vec3()?)
        };
        Ok(Self { flags, rot, trans })
    }

    fn write(&self, w: &mut Writer) {
        w.w_u8(self.flags);
        match &self.rot {
            RotKeys::Constant(k) => k.iter().for_each(|v| w.w_i16(*v)),
            RotKeys::Keys { crc, keys } => {
                w.w_u32(*crc);
                for k in keys.iter() {
                    k.iter().for_each(|v| w.w_i16(*v));
                }
            }
        }
        match &self.trans {
            TransKeys::Constant(t) => w.w_vec3(*t),
            TransKeys::Keys8 { crc, keys, size, init } => {
                w.w_u32(*crc);
                for k in keys.iter() {
                    k.iter().for_each(|v| w.w_i8(*v));
                }
                w.w_vec3(*size);
                w.w_vec3(*init);
            }
            TransKeys::Keys16 { crc, keys, size, init } => {
                w.w_u32(*crc);
                for k in keys.iter() {
                    k.iter().for_each(|v| w.w_i16(*v));
                }
                w.w_vec3(*size);
                w.w_vec3(*init);
            }
        }
    }

    fn rotation(&self, frame: usize) -> Quat {
        match &self.rot {
            RotKeys::Constant(k) => qr(*k),
            RotKeys::Keys { keys, .. } => keys.get(frame % keys.len().max(1)).map_or(Quat::IDENTITY, |k| qr(*k)),
        }
    }

    fn translation(&self, frame: usize) -> Vec3 {
        match &self.trans {
            TransKeys::Constant(t) => *t,
            TransKeys::Keys8 { keys, size, init, .. } => keys.get(frame % keys.len().max(1)).map_or(*init, |k| {
                Vec3::new(f32::from(k[0]) * size.x + init.x, f32::from(k[1]) * size.y + init.y, f32::from(k[2]) * size.z + init.z)
            }),
            TransKeys::Keys16 { keys, size, init, .. } => keys.get(frame % keys.len().max(1)).map_or(*init, |k| {
                Vec3::new(f32::from(k[0]) * size.x + init.x, f32::from(k[1]) * size.y + init.y, f32::from(k[2]) * size.z + init.z)
            }),
        }
    }

    /// `Dequantize`: the key at `t` seconds, from the two frames around it.
    #[must_use]
    pub fn sample(&self, t: f32) -> BoneKey {
        let time = (t * SAMPLE_FPS).max(0.0);
        let frame = time.floor() as usize;
        let delta = time - frame as f32;
        let rotation = match &self.rot {
            RotKeys::Constant(k) => qr(*k),
            RotKeys::Keys { .. } => {
                gz_fmt_object::xmath::slerp(self.rotation(frame), self.rotation(frame + 1), delta.clamp(0.0, 1.0))
            }
        };
        let translation = match &self.trans {
            TransKeys::Constant(t) => *t,
            _ => gz_fmt_object::xmath::lerp(self.translation(frame), self.translation(frame + 1), delta),
        };
        BoneKey { rotation, translation }
    }
}

impl MotionMark {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        let name = SmolStr::new(r.r_line());
        let n = r.r_u32()? as usize;
        if n.saturating_mul(8) > r.elapsed() {
            return Err(invalid("a motion mark is cut short"));
        }
        Ok(Self { name, intervals: (0..n).map(|_| Ok((r.r_f32()?, r.r_f32()?))).collect::<Result<Vec<_>>>()? })
    }

    fn write(&self, w: &mut Writer) {
        w.w_line(&self.name);
        w.w_u32(self.intervals.len() as u32);
        for (a, b) in &self.intervals {
            w.w_f32(*a);
            w.w_f32(*b);
        }
    }
}

impl MotionSet {
    /// Reads the two chunks. `bone_count` is the model's; `None` takes it from the parts.
    pub fn read_chunks(params: Option<&[u8]>, motions: Option<&[u8]>, bone_count: Option<usize>) -> Result<Self> {
        let mut set = MotionSet::default();
        if let Some(body) = params {
            let mut r = Reader::new(body);
            let version = r.r_u16()?;
            if version > SMPARAMS_VERSION {
                return Err(invalid(format!("the motion parameters have version {version}; the game reads up to 4")));
            }
            set.params_version = Some(version);
            let parts = usize::from(r.r_u16()?);
            for _ in 0..parts {
                let name = SmolStr::new(r.r_stringz()?);
                let n = usize::from(r.r_u16()?);
                if n.saturating_mul(5) > r.elapsed() {
                    return Err(invalid("a bone part is cut short"));
                }
                let bones = (0..n).map(|_| Ok((SmolStr::new(r.r_stringz()?), r.r_u32()?))).collect::<Result<Vec<_>>>()?;
                set.parts.push(PartDef { name, bones });
            }
            let defs = usize::from(r.r_u16()?);
            for _ in 0..defs {
                let name = SmolStr::new(r.r_stringz()?);
                let flags = r.r_u32()?;
                let mut d = MotionDef {
                    name,
                    flags,
                    bone_or_part: r.r_u16()?,
                    motion: r.r_u16()?,
                    speed: r.r_f32()?,
                    power: r.r_f32()?,
                    accrue: r.r_f32()?,
                    falloff: r.r_f32()?,
                    marks: Vec::new(),
                };
                if version >= 4 {
                    let n = r.r_u32()? as usize;
                    if n.saturating_mul(5) > r.elapsed() {
                        return Err(invalid("a motion's marks are cut short"));
                    }
                    for _ in 0..n {
                        d.marks.push(MotionMark::read(&mut r)?);
                    }
                }
                set.defs.push(d);
            }
        }
        if let Some(body) = motions {
            let bones = bone_count.unwrap_or_else(|| set.parts.iter().map(|p| p.bones.len()).sum());
            let r = Reader::new(body);
            let count = r.sub(0)?.map(|mut c| c.r_u32()).transpose()?.unwrap_or(0) as usize;
            if count >= 0x3FFF {
                return Err(invalid(format!("the model states {count} motions")));
            }
            for i in 0..count {
                let mut m = r.sub(i as u32 + 1)?.ok_or_else(|| invalid(format!("motion {i} is missing")))?;
                let name = SmolStr::new(m.r_stringz()?);
                let length = m.r_u32()?;
                // a flag, one rotation and one translation at least
                if bones.saturating_mul(21) > m.elapsed() {
                    return Err(invalid(format!("motion '{name}' is shorter than its bones")));
                }
                let bones = (0..bones).map(|_| BoneMotion::read(&mut m, length as usize)).collect::<Result<Vec<_>>>()?;
                set.motions.push(Motion { name, length, bones });
            }
        }
        Ok(set)
    }

    /// `OGF_S_MOTIONS`.
    pub(crate) fn write_motions(&self, w: &mut Writer) {
        w.chunk(0, |w| w.w_u32(self.motions.len() as u32));
        for (i, m) in self.motions.iter().enumerate() {
            w.chunk(i as u32 + 1, |w| {
                w.w_stringz(&m.name);
                w.w_u32(m.length);
                for b in &m.bones {
                    b.write(w);
                }
            });
        }
    }

    /// `OGF_S_SMPARAMS`.
    pub(crate) fn write_params(&self, w: &mut Writer) {
        let version = self.params_version.unwrap_or(SMPARAMS_VERSION);
        w.w_u16(version);
        w.w_u16(self.parts.len() as u16);
        for p in &self.parts {
            w.w_stringz(&p.name);
            w.w_u16(p.bones.len() as u16);
            for (name, idx) in &p.bones {
                w.w_stringz(name);
                w.w_u32(*idx);
            }
        }
        w.w_u16(self.defs.len() as u16);
        for d in &self.defs {
            w.w_stringz(&d.name);
            w.w_u32(d.flags);
            w.w_u16(d.bone_or_part);
            w.w_u16(d.motion);
            w.w_f32(d.speed);
            w.w_f32(d.power);
            w.w_f32(d.accrue);
            w.w_f32(d.falloff);
            if version >= 4 {
                w.w_u32(d.marks.len() as u32);
                for m in &d.marks {
                    m.write(w);
                }
            }
        }
    }

    /// An `.omf`: the keys, then the parameters, as `PrepareOMF` writes them.
    #[must_use]
    pub fn to_omf(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(chunk::S_MOTIONS, |w| self.write_motions(w));
        w.chunk(chunk::S_SMPARAMS, |w| self.write_params(w));
        w.into_bytes()
    }

    /// One motion at `t` seconds: a key per bone, in key-slot order.
    #[must_use]
    pub fn pose(&self, motion: usize, t: f32) -> Option<Vec<BoneKey>> {
        let m = self.motions.get(motion)?;
        Some(m.bones.iter().map(|b| b.sample(t)).collect())
    }

    /// For every key slot the model's bone, found by name as the engine finds it.
    #[must_use]
    pub fn remap(&self, bone_names: &[SmolStr]) -> Vec<Option<u16>> {
        let slots = self.motions.first().map_or(bone_names.len(), |m| m.bones.len());
        let mut out = vec![None; slots];
        for p in &self.parts {
            for (name, idx) in &p.bones {
                let found = bone_names.iter().position(|b| b.eq_ignore_ascii_case(name)).and_then(|i| u16::try_from(i).ok());
                if let Some(slot) = out.get_mut(*idx as usize) {
                    *slot = found;
                }
            }
        }
        out
    }

    /// The motion a definition names.
    #[must_use]
    pub fn find(&self, name: &str) -> Option<usize> {
        self.motions.iter().position(|m| m.name.eq_ignore_ascii_case(name))
    }
}

/// An `.omf` file.
pub fn read_omf(bytes: &[u8]) -> Result<MotionSet> {
    let r = Reader::new(bytes);
    let params = r.sub(chunk::S_SMPARAMS)?;
    let motions = r.sub(chunk::S_MOTIONS)?;
    if params.is_none() {
        return Err(invalid("the motions have no bone parts (OGF_S_SMPARAMS); the game refuses such a file"));
    }
    MotionSet::read_chunks(params.as_ref().map(Reader::data), motions.as_ref().map(Reader::data), None)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_set() -> MotionSet {
        let keys16: Arc<[[i16; 3]]> = Arc::from(vec![[-32767, 0, 0], [32767, 0, 0]]);
        MotionSet {
            params_version: Some(4),
            parts: vec![PartDef { name: "default".into(), bones: vec![("root".into(), 0), ("arm".into(), 1)] }],
            defs: vec![MotionDef {
                name: "idle".into(),
                flags: 0,
                bone_or_part: 0,
                motion: 0,
                speed: 1.0,
                power: 1.0,
                accrue: 2.0,
                falloff: 2.0,
                marks: vec![MotionMark { name: "step".into(), intervals: vec![(0.1, 0.2)] }],
            }],
            motions: vec![Motion {
                name: "idle".into(),
                length: 2,
                bones: vec![
                    BoneMotion {
                        flags: FL_T_KEY_PRESENT | FL_T_KEY_16,
                        rot: RotKeys::Keys { crc: 0, keys: Arc::from(vec![[0, 0, 0, 32767], [32767, 0, 0, 0]]) },
                        trans: TransKeys::Keys16 { crc: 0, keys: keys16, size: Vec3::new(1.0 / 32767.0, 0.0, 0.0), init: Vec3::new(1.0, 0.0, 0.0) },
                    },
                    BoneMotion { flags: FL_R_KEY_ABSENT, rot: RotKeys::Constant([0, 0, 0, 32767]), trans: TransKeys::Constant(Vec3::Y) },
                ],
            }],
        }
    }

    #[test]
    fn an_omf_writes_reads_and_writes_the_same_bytes() {
        let set = sample_set();
        let omf = set.to_omf();
        let back = read_omf(&omf).unwrap();
        assert_eq!(back, set);
        assert_eq!(back.to_omf(), omf);
    }

    #[test]
    fn keys_decode_as_the_engine_samples_them() {
        let set = sample_set();
        // frame 0: the init minus one unit, rotation identity
        let p0 = set.pose(0, 0.0).unwrap();
        assert!((p0[0].translation.x - 0.0).abs() < 1e-6);
        assert_eq!(p0[0].rotation, Quat::from_xyzw(0.0, 0.0, 0.0, 1.0));
        // half way between frame 0 and frame 1 (1/60 s): x is lerped to 1, the rotation slerped
        let p = set.pose(0, 0.5 / 30.0).unwrap();
        assert!((p[0].translation.x - 1.0).abs() < 1e-5);
        let half = std::f32::consts::FRAC_1_SQRT_2;
        assert!((p[0].rotation.x - half).abs() < 1e-4 && (p[0].rotation.w - half).abs() < 1e-4);
        // frame 2 wraps to frame 0
        assert!((set.pose(0, 2.0 / 30.0).unwrap()[0].translation.x).abs() < 1e-6);
        // the constant bone
        assert_eq!(p[1].translation, Vec3::Y);
        assert_eq!(set.remap(&["Root".into(), "arm".into()]), vec![Some(0), Some(1)]);
    }

    #[test]
    fn a_cut_motion_is_an_error() {
        let omf = sample_set().to_omf();
        assert!(read_omf(&omf[..omf.len() - 30]).is_err());
        let mut w = Writer::new();
        w.chunk(chunk::S_MOTIONS, |_| {});
        assert!(read_omf(&w.into_bytes()).is_err());
    }
}
