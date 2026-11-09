//! Motions: an object motion (`.anm`) and a skeletal motion (`.skl`, `.skls`, and the motions
//! an `.object` embeds). `motion.cpp`.

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::bone::Bone;
use crate::envelope::Envelope;
use crate::io::{Text, TextEncoding, checked_count, lower, need_chunk, r_term_string, w_str, w_term_string};
use crate::{BONE_NONE, ObjectError, Result};

const CHUNK_OMOTION: u32 = 0x1100;
const CHUNK_SMOTION: u32 = 0x1200;
const OMOTION_VERSION: u16 = 0x0005;
const SMOTION_VERSION: u16 = 0x0007;

/// The six channels of a motion, in the order envelopes are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Channel {
    PositionX = 0,
    PositionY = 1,
    PositionZ = 2,
    RotationH = 3,
    RotationP = 4,
    RotationB = 5,
}

bitflags::bitflags! {
    /// `ESMFlags`.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct SMotionFlags: u8 {
        const FX = 1 << 0;
        const STOP_AT_END = 1 << 1;
        const NO_MIX = 1 << 2;
        const SYNC_PART = 1 << 3;
        const USE_FOOT_STEPS = 1 << 4;
        const ROOT_MOVER = 1 << 5;
        const IDLE = 1 << 6;
        const USE_WEAPON_BONE = 1 << 7;
    }
}

/// `motion_marks`: named time intervals of a motion (foot steps and the like).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MotionMark {
    pub name: SmolStr,
    pub intervals: Vec<(f32, f32)>,
}

impl MotionMark {
    /// Reads a mark; the name is UTF-8 when it is valid UTF-8 and cp1251 otherwise.
    pub fn read(r: &mut Reader<'_>) -> Result<MotionMark> {
        Self::read_with(r, &mut Text::default())
    }

    pub(crate) fn read_with(r: &mut Reader<'_>, text: &mut Text) -> Result<MotionMark> {
        let name = text.decode(r_term_string(r));
        let count = checked_count(r, r.clone().r_u32()? as usize, 8)?;
        r.advance(4)?;
        let mut intervals = Vec::with_capacity(count);
        for _ in 0..count {
            intervals.push((r.r_f32()?, r.r_f32()?));
        }
        Ok(MotionMark { name, intervals })
    }

    pub fn write(&self, w: &mut Writer, enc: TextEncoding) {
        w_term_string(w, &self.name, enc);
        w.w_u32(self.intervals.len() as u32);
        for (a, b) in &self.intervals {
            w.w_f32(*a);
            w.w_f32(*b);
        }
    }
}

/// `COMotion`: the motion of a whole object - six envelopes.
#[derive(Clone, Debug, PartialEq)]
pub struct OMotion {
    pub name: SmolStr,
    pub frame_start: i32,
    pub frame_end: i32,
    pub fps: f32,
    /// Indexed by [`Channel`].
    pub envelopes: [Envelope; 6],
}

impl Default for OMotion {
    fn default() -> Self {
        Self { name: SmolStr::default(), frame_start: 0, frame_end: 0, fps: 30.0, envelopes: Default::default() }
    }
}

impl OMotion {
    /// `COMotion::Load` from a stream positioned at the motion.
    pub(crate) fn read(r: &mut Reader<'_>, text: &mut Text) -> Result<OMotion> {
        let name = text.r_str(r)?;
        let frame_start = r.r_i32()?;
        let frame_end = r.r_i32()?;
        let fps = r.r_f32()?;
        let version = r.r_u16()?;
        let mut envelopes: [Envelope; 6] = Default::default();
        match version {
            0x0003 => {
                for e in &mut envelopes {
                    *e = Envelope::read_v1(r)?;
                }
            }
            0x0004 => {
                // this version stored pitch before heading
                for ch in [0usize, 1, 2, 4, 3, 5] {
                    envelopes[ch] = Envelope::read_v2(r)?;
                }
            }
            OMOTION_VERSION => {
                for e in &mut envelopes {
                    *e = Envelope::read_v2(r)?;
                }
            }
            v => return Err(ObjectError::Version { what: "The object motion", version: u32::from(v) }),
        }
        Ok(OMotion { name, frame_start, frame_end, fps, envelopes })
    }

    pub(crate) fn write(&self, w: &mut Writer, enc: TextEncoding) {
        w_str(w, &self.name, enc);
        w.w_i32(self.frame_start);
        w.w_i32(self.frame_end);
        w.w_f32(self.fps);
        w.w_u16(OMOTION_VERSION);
        for e in &self.envelopes {
            e.write(w);
        }
    }

    /// `_Evaluate`: translation and rotation (x = pitch, y = heading, z = bank) at `t` seconds.
    #[must_use]
    pub fn evaluate(&self, t: f32) -> (Vec3, Vec3) {
        evaluate_channels(&self.envelopes, t)
    }

    /// `Length()`: frames, both ends included.
    #[must_use]
    pub fn length(&self) -> i32 {
        self.frame_end.wrapping_sub(self.frame_start).wrapping_add(1)
    }
}

fn evaluate_channels(envs: &[Envelope; 6], t: f32) -> (Vec3, Vec3) {
    let tr = Vec3::new(envs[0].evaluate(t), envs[1].evaluate(t), envs[2].evaluate(t));
    let ry = envs[3].evaluate(t);
    let rx = envs[4].evaluate(t);
    let rz = envs[5].evaluate(t);
    (tr, Vec3::new(rx, ry, rz))
}

/// `st_BoneMotion`: the envelopes of one bone.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BoneMotion {
    pub name: SmolStr,
    /// Bit 0: the rotation is in world space; bit 1: no rotation keys.
    pub flags: u8,
    /// Indexed by [`Channel`].
    pub envelopes: [Envelope; 6],
}

impl BoneMotion {
    pub const WORLD_ORIENT: u8 = 1 << 0;
}

/// `CSMotion`: a skeletal motion.
#[derive(Clone, Debug, PartialEq)]
pub struct SMotion {
    pub name: SmolStr,
    pub frame_start: i32,
    pub frame_end: i32,
    pub fps: f32,
    pub flags: SMotionFlags,
    /// A bone part for a cycle, a bone for an FX; [`BONE_NONE`] for all.
    pub bone_or_part: u16,
    pub speed: f32,
    pub accrue: f32,
    pub falloff: f32,
    pub power: f32,
    pub bone_motions: Vec<BoneMotion>,
    pub marks: Vec<MotionMark>,
}

impl Default for SMotion {
    fn default() -> Self {
        Self {
            name: SmolStr::default(),
            frame_start: 0,
            frame_end: 0,
            fps: 30.0,
            flags: SMotionFlags::empty(),
            bone_or_part: BONE_NONE,
            speed: 1.0,
            accrue: 2.0,
            falloff: 2.0,
            power: 1.0,
            bone_motions: Vec::new(),
            marks: Vec::new(),
        }
    }
}

impl SMotion {
    /// `CSMotion::Load` from a stream positioned at the motion.
    pub(crate) fn read(r: &mut Reader<'_>, text: &mut Text) -> Result<SMotion> {
        let mut m = SMotion { name: text.r_str(r)?, ..SMotion::default() };
        m.frame_start = r.r_i32()?;
        m.frame_end = r.r_i32()?;
        m.fps = r.r_f32()?;
        let version = r.r_u16()?;
        match version {
            0x0004 => {
                m.bone_or_part = (r.r_u32()? & 0xffff) as u16;
                m.flags.set(SMotionFlags::FX, r.r_u8()? != 0);
                m.flags.set(SMotionFlags::STOP_AT_END, r.r_u8()? != 0);
                m.read_blend(r)?;
                let count = checked_count(r, r.clone().r_u32()? as usize, 4 + 6 * 12)?;
                r.advance(4)?;
                for i in 0..count {
                    let mut bm = BoneMotion { name: SmolStr::new(i.to_string()), ..BoneMotion::default() };
                    bm.flags = r.r_u32()? as u8;
                    for e in &mut bm.envelopes {
                        *e = Envelope::read_v1(r)?;
                    }
                    m.bone_motions.push(bm);
                }
            }
            0x0005 => {
                m.flags = SMotionFlags::from_bits_retain(r.r_u32()? as u8);
                m.bone_or_part = (r.r_u32()? & 0xffff) as u16;
                m.read_blend(r)?;
                let count = checked_count(r, r.clone().r_u32()? as usize, 1 + 4 + 6 * 12)?;
                r.advance(4)?;
                for _ in 0..count {
                    let mut bm = BoneMotion { name: text.r_str(r)?, ..BoneMotion::default() };
                    bm.flags = r.r_u32()? as u8;
                    for e in &mut bm.envelopes {
                        *e = Envelope::read_v1(r)?;
                    }
                    m.bone_motions.push(bm);
                }
            }
            v if v >= 0x0006 => {
                m.flags = SMotionFlags::from_bits_retain(r.r_u8()?);
                m.bone_or_part = r.r_u16()?;
                m.read_blend(r)?;
                let count = checked_count(r, usize::from(r.clone().r_u16()?), 1 + 1 + 6 * 4)?;
                r.advance(2)?;
                for _ in 0..count {
                    let mut bm = BoneMotion { name: text.r_str(r)?, ..BoneMotion::default() };
                    bm.flags = r.r_u8()?;
                    for e in &mut bm.envelopes {
                        *e = Envelope::read_v2(r)?;
                    }
                    m.bone_motions.push(bm);
                }
            }
            // oracle: the C++ reads nothing for an older motion and carries on into garbage
            v => return Err(ObjectError::Version { what: "The skeletal motion", version: u32::from(v) }),
        }
        if version >= 0x0007 {
            let count = checked_count(r, r.clone().r_u32()? as usize, 5)?;
            r.advance(4)?;
            for _ in 0..count {
                m.marks.push(MotionMark::read_with(r, text)?);
            }
        }
        for bm in &mut m.bone_motions {
            bm.name = lower(&bm.name);
        }
        Ok(m)
    }

    fn read_blend(&mut self, r: &mut Reader<'_>) -> Result<()> {
        self.speed = r.r_f32()?;
        self.accrue = r.r_f32()?;
        self.falloff = r.r_f32()?;
        self.power = r.r_f32()?;
        Ok(())
    }

    /// `CSMotion::Save`.
    pub(crate) fn write(&self, w: &mut Writer, enc: TextEncoding) {
        w_str(w, &self.name, enc);
        w.w_i32(self.frame_start);
        w.w_i32(self.frame_end);
        w.w_f32(self.fps);
        w.w_u16(SMOTION_VERSION);
        w.w_u8(self.flags.bits());
        w.w_u16(self.bone_or_part);
        w.w_f32(self.speed);
        w.w_f32(self.accrue);
        w.w_f32(self.falloff);
        w.w_f32(self.power);
        w.w_u16(self.bone_motions.len() as u16);
        for bm in &self.bone_motions {
            w_str(w, &lower(&bm.name), enc);
            w.w_u8(bm.flags);
            for e in &bm.envelopes {
                e.write(w);
            }
        }
        w.w_u32(self.marks.len() as u32);
        for mark in &self.marks {
            mark.write(w, enc);
        }
    }

    /// `SetName`: motion names are lower case.
    pub fn set_name(&mut self, name: &str) {
        self.name = SmolStr::new(name.to_ascii_lowercase());
    }

    /// `Length()`: frames, both ends included.
    #[must_use]
    pub fn length(&self) -> i32 {
        self.frame_end.wrapping_sub(self.frame_start).wrapping_add(1)
    }

    /// `_Evaluate`: translation and rotation (x = pitch, y = heading, z = bank) of a bone at
    /// `t` seconds. `None` when the motion has no such bone.
    #[must_use]
    pub fn evaluate(&self, bone: usize, t: f32) -> Option<(Vec3, Vec3)> {
        self.bone_motions.get(bone).map(|bm| evaluate_channels(&bm.envelopes, t))
    }

    /// `FindBoneMotion`.
    #[must_use]
    pub fn find_bone_motion(&self, name: &str) -> Option<usize> {
        self.bone_motions.iter().position(|bm| bm.name == name)
    }

    /// `GetMotionFlags`; zero for a bone the motion does not hold.
    #[must_use]
    pub fn bone_flags(&self, bone: usize) -> u8 {
        self.bone_motions.get(bone).map_or(0, |bm| bm.flags)
    }

    /// `SortBonesBySkeleton`: one bone motion per bone, in the skeleton's order. A bone the
    /// motion lacks gets one key from its rest pose; a bone motion the skeleton lacks is dropped.
    pub fn sort_bones_by_skeleton(&mut self, bones: &[Bone]) {
        let mut sorted = Vec::with_capacity(bones.len());
        for bone in bones {
            let found = match self.find_bone_motion(&bone.name) {
                Some(i) => i,
                None => {
                    let mut bm = BoneMotion { name: bone.name.clone(), ..BoneMotion::default() };
                    // oracle: the flags come from the motion's first bone, and heading takes
                    // the bone's x, pitch its y
                    self.bone_motions.push(BoneMotion::default());
                    bm.flags = self.bone_motions[0].flags;
                    // oracle: the C++ reads the bone's animated offset here, which nothing has
                    // set yet on this path; the rest pose is what it holds once the object is up
                    let (o, rot) = (bone.rest_offset, bone.rest_rotate);
                    bm.envelopes[Channel::PositionX as usize].insert_key(0.0, o.x);
                    bm.envelopes[Channel::PositionY as usize].insert_key(0.0, o.y);
                    bm.envelopes[Channel::PositionZ as usize].insert_key(0.0, o.z);
                    bm.envelopes[Channel::RotationH as usize].insert_key(0.0, rot.x);
                    bm.envelopes[Channel::RotationP as usize].insert_key(0.0, rot.y);
                    bm.envelopes[Channel::RotationB as usize].insert_key(0.0, rot.z);
                    let at = self.bone_motions.len() - 1;
                    self.bone_motions[at] = bm;
                    at
                }
            };
            sorted.push(self.bone_motions[found].clone());
        }
        self.bone_motions = sorted;
    }

    /// `CheckBoneCompliance`: gives every bone the motion lacks an empty bone motion.
    pub fn add_missing_bones(&mut self, bones: &[Bone]) {
        for bone in bones {
            if self.find_bone_motion(&bone.name).is_none() {
                // flRKeyAbsent
                self.bone_motions.push(BoneMotion { name: bone.name.clone(), flags: 1 << 1, ..BoneMotion::default() });
            }
        }
    }

    /// `Optimize`: collapses envelopes that hold one value.
    pub fn optimize(&mut self) {
        for bm in &mut self.bone_motions {
            for e in &mut bm.envelopes {
                e.optimize();
            }
        }
    }
}

/// Runs a reader in automatic text mode and again as cp1251 when a string was not UTF-8, so
/// every string of such a file goes through the same code page.
pub(crate) fn with_text<T>(mut read: impl FnMut(&mut Text) -> Result<T>) -> Result<(T, TextEncoding)> {
    let mut text = Text::default();
    let value = read(&mut text)?;
    if !text.non_utf8 {
        return Ok((value, TextEncoding::Utf8));
    }
    let mut text = Text::forced(TextEncoding::Cp1251);
    Ok((read(&mut text)?, TextEncoding::Cp1251))
}

/// Reads a `.skl`: one skeletal motion.
pub fn read_skl(bytes: &[u8]) -> Result<SMotion> {
    let file = Reader::new(bytes);
    let body = need_chunk(&file, CHUNK_SMOTION)?;
    with_text(|text| SMotion::read(&mut body.clone(), text)).map(|(m, _)| m)
}

/// Writes a `.skl` in UTF-8.
#[must_use]
pub fn write_skl(motion: &SMotion) -> Vec<u8> {
    let mut w = Writer::new();
    w.chunk(CHUNK_SMOTION, |w| motion.write(w, TextEncoding::Utf8));
    w.into_bytes()
}

/// Reads a `.skls`: a count and that many skeletal motions.
pub fn read_skls(bytes: &[u8]) -> Result<Vec<SMotion>> {
    with_text(|text| {
        let mut r = Reader::new(bytes);
        let count = checked_count(&r, r.clone().r_u32()? as usize, 32)?;
        r.advance(4)?;
        let mut out = Vec::with_capacity(count);
        for _ in 0..count {
            out.push(SMotion::read(&mut r, text)?);
        }
        Ok(out)
    })
    .map(|(m, _)| m)
}

/// Writes a `.skls` in UTF-8.
#[must_use]
pub fn write_skls(motions: &[SMotion]) -> Vec<u8> {
    let mut w = Writer::new();
    w.w_u32(motions.len() as u32);
    for m in motions {
        m.write(&mut w, TextEncoding::Utf8);
    }
    w.into_bytes()
}

/// Reads an `.anm`: one object motion.
pub fn read_anm(bytes: &[u8]) -> Result<OMotion> {
    let file = Reader::new(bytes);
    let body = need_chunk(&file, CHUNK_OMOTION)?;
    with_text(|text| OMotion::read(&mut body.clone(), text)).map(|(m, _)| m)
}

/// Writes an `.anm` in UTF-8.
#[must_use]
pub fn write_anm(motion: &OMotion) -> Vec<u8> {
    let mut w = Writer::new();
    w.chunk(CHUNK_OMOTION, |w| motion.write(w, TextEncoding::Utf8));
    w.into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::envelope::{Key, shape};
    use gz_chunk::ChunkError;

    fn sample() -> SMotion {
        let mut m = SMotion { frame_start: 0, frame_end: 30, ..SMotion::default() };
        m.set_name("Walk_FWD");
        m.flags = SMotionFlags::STOP_AT_END | SMotionFlags::ROOT_MOVER;
        m.bone_or_part = 2;
        let mut bm = BoneMotion { name: "root".into(), flags: 1, ..BoneMotion::default() };
        bm.envelopes[0].keys.push(Key { time: 0.0, value: 1.0, shape: shape::LINEAR, ..Key::default() });
        bm.envelopes[0].keys.push(Key { time: 1.0, value: 3.0, shape: shape::LINEAR, ..Key::default() });
        bm.envelopes[3].keys.push(Key { time: 0.0, value: 0.5, shape: shape::STEPPED, ..Key::default() });
        m.bone_motions.push(bm);
        m.marks.push(MotionMark { name: "left".into(), intervals: vec![(0.1, 0.2), (0.6, 0.7)] });
        m
    }

    #[test]
    fn skl_round_trips() {
        let m = sample();
        assert_eq!(m.name, "walk_fwd");
        let bytes = write_skl(&m);
        let back = read_skl(&bytes).unwrap();
        // key parameters are stored in 16-bit steps, so the first read settles them
        assert_eq!(write_skl(&back), bytes);
        assert_eq!(read_skl(&write_skl(&back)).unwrap(), back);
        assert_eq!((back.name.as_str(), back.flags, back.bone_or_part), ("walk_fwd", m.flags, 2));
        assert_eq!(back.marks, m.marks);
        assert_eq!(back.bone_motions[0].flags, 1);
        // heading comes back in y
        let (t, r) = back.evaluate(0, 0.5).unwrap();
        assert_eq!(t, Vec3::new(2.0, 0.0, 0.0));
        assert_eq!(r, Vec3::new(0.0, 0.5, 0.0));
        assert!(back.evaluate(4, 0.0).is_none());
    }

    #[test]
    fn skls_round_trips() {
        let a = sample();
        let mut b = sample();
        b.set_name("idle");
        b.marks.clear();
        let bytes = write_skls(&[a, b]);
        let back = read_skls(&bytes).unwrap();
        assert_eq!(back.len(), 2);
        assert_eq!(back[1].name, "idle");
        assert_eq!(write_skls(&back), bytes);
        assert!(read_skls(&[0xff, 0xff, 0xff, 0x7f]).is_err());
        assert!(read_skls(&bytes[..bytes.len() - 3]).is_err());
    }

    #[test]
    fn anm_round_trips_and_old_versions_are_named() {
        let mut m = OMotion { name: "door".into(), frame_end: 10, ..OMotion::default() };
        m.envelopes[1].insert_key(0.0, 2.0);
        m.envelopes[4].insert_key(0.0, 0.25);
        let bytes = write_anm(&m);
        let back = read_anm(&bytes).unwrap();
        assert_eq!(write_anm(&back), bytes);
        assert_eq!(back.name, "door");
        assert_eq!(back.length(), 11);
        let (t, r) = back.evaluate(5.0);
        assert_eq!(t.y, 2.0);
        assert_eq!(r.x, 0.25);

        let mut w = Writer::new();
        w.open_chunk(CHUNK_OMOTION);
        w.w_stringz("x");
        w.w_u32(0);
        w.w_u32(0);
        w.w_f32(30.0);
        w.w_u16(2);
        let old = w.into_bytes();
        assert!(matches!(read_anm(&old), Err(ObjectError::Version { version: 2, .. })));
        assert!(matches!(read_anm(b"junk"), Err(ObjectError::Chunk(ChunkError::Missing(CHUNK_OMOTION)))));
    }

    #[test]
    fn version_five_names_bones_and_lowers_them() {
        let mut w = Writer::new();
        w.w_stringz("old");
        w.w_u32(1);
        w.w_u32(5);
        w.w_f32(25.0);
        w.w_u16(5);
        w.w_u32(u32::from(SMotionFlags::FX.bits()));
        w.w_u32(0x0001_0003);
        for v in [1.0f32, 2.0, 3.0, 4.0] {
            w.w_f32(v);
        }
        w.w_u32(1);
        w.w_stringz("Bip01");
        w.w_u32(1);
        for _ in 0..6 {
            w.w_i32(1);
            w.w_i32(1);
            w.w_u32(0);
        }
        let bytes = w.into_bytes();
        let m = SMotion::read(&mut Reader::new(&bytes), &mut Text::default()).unwrap();
        assert_eq!(m.bone_or_part, 3);
        assert_eq!(m.flags, SMotionFlags::FX);
        assert_eq!((m.speed, m.accrue, m.falloff, m.power), (1.0, 2.0, 3.0, 4.0));
        assert_eq!(m.bone_motions[0].name, "bip01");
        assert!(m.marks.is_empty());
    }

    #[test]
    fn bones_follow_the_skeleton() {
        let mut m = sample();
        let bones = vec![
            Bone { name: "spine".into(), rest_offset: Vec3::new(1.0, 2.0, 3.0), ..Bone::default() },
            Bone { name: "root".into(), ..Bone::default() },
        ];
        m.sort_bones_by_skeleton(&bones);
        assert_eq!(m.bone_motions.len(), 2);
        assert_eq!(m.bone_motions[0].name, "spine");
        assert_eq!(m.bone_motions[0].flags, 1);
        assert_eq!(m.bone_motions[0].envelopes[1].evaluate(0.0), 2.0);
        assert_eq!(m.bone_motions[1].name, "root");
    }
}
