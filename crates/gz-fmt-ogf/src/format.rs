//! Chunk ids, the header, the description: `Fmesh.h`, `fmesh.cpp`.

use glam::Vec3;
use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{Result, invalid};

/// `MT`: what a visual is.
#[allow(non_snake_case)]
pub mod MT {
    pub const NORMAL: u8 = 0;
    pub const HIERRARHY: u8 = 1;
    pub const PROGRESSIVE: u8 = 2;
    pub const SKELETON_ANIM: u8 = 3;
    pub const SKELETON_GEOMDEF_PM: u8 = 4;
    pub const SKELETON_GEOMDEF_ST: u8 = 5;
    pub const LOD: u8 = 6;
    pub const TREE_ST: u8 = 7;
    pub const PARTICLE_EFFECT: u8 = 8;
    pub const PARTICLE_GROUP: u8 = 9;
    pub const SKELETON_RIGID: u8 = 10;
    pub const TREE_PM: u8 = 11;
    pub const FLUID_VOLUME: u8 = 12;
}

/// `OGF_Chuncks`.
pub mod chunk {
    pub const HEADER: u32 = 1;
    pub const TEXTURE: u32 = 2;
    pub const VERTICES: u32 = 3;
    pub const INDICES: u32 = 4;
    pub const SWIDATA: u32 = 6;
    pub const VCONTAINER: u32 = 7;
    pub const ICONTAINER: u32 = 8;
    pub const CHILDREN: u32 = 9;
    pub const CHILDREN_L: u32 = 10;
    pub const LODDEF2: u32 = 11;
    pub const TREEDEF2: u32 = 12;
    pub const S_BONE_NAMES: u32 = 13;
    pub const S_MOTIONS: u32 = 14;
    pub const S_SMPARAMS: u32 = 15;
    pub const S_IKDATA: u32 = 16;
    pub const S_USERDATA: u32 = 17;
    pub const S_DESC: u32 = 18;
    pub const S_MOTION_REFS: u32 = 19;
    pub const SWICONTAINER: u32 = 20;
    pub const GCONTAINER: u32 = 21;
    pub const FASTPATH: u32 = 22;
    pub const S_LODS: u32 = 23;
    pub const S_MOTION_REFS2: u32 = 24;
    pub const COLLISION_VERTICES: u32 = 25;
    pub const COLLISION_INDICES: u32 = 26;
}

/// `xrOGF_FormatVersion`.
pub const FORMAT_VERSION: u8 = 4;

/// `ogf_header`: 44 bytes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Header {
    pub format_version: u8,
    pub kind: u8,
    pub shader_id: u16,
    pub bb_min: Vec3,
    pub bb_max: Vec3,
    pub sphere_center: Vec3,
    pub sphere_radius: f32,
}

impl Header {
    pub const SIZE: usize = 44;

    pub(crate) fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self {
            format_version: r.r_u8()?,
            kind: r.r_u8()?,
            shader_id: r.r_u16()?,
            bb_min: r.r_vec3()?,
            bb_max: r.r_vec3()?,
            sphere_center: r.r_vec3()?,
            sphere_radius: r.r_f32()?,
        })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_u8(self.format_version);
        w.w_u8(self.kind);
        w.w_u16(self.shader_id);
        w.w_vec3(self.bb_min);
        w.w_vec3(self.bb_max);
        w.w_vec3(self.sphere_center);
        w.w_f32(self.sphere_radius);
    }

    /// A header of `kind` around a box, its sphere from `Fbox::getsphere`.
    pub(crate) fn around(kind: u8, b: &gz_fmt_object::Aabb) -> Self {
        let (c, r) = b.sphere();
        Self {
            format_version: FORMAT_VERSION,
            kind,
            shader_id: 0,
            bb_min: b.min,
            bb_max: b.max,
            sphere_center: c,
            sphere_radius: r,
        }
    }
}

/// `ogf_desc`: who made the model and when.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Desc {
    pub source_file: SmolStr,
    pub build_name: SmolStr,
    pub build_time: i64,
    pub create_name: SmolStr,
    pub create_time: i64,
    pub modif_name: SmolStr,
    pub modif_time: i64,
    /// `time_t` was four bytes in the files of 32-bit builds; the editor writes eight.
    pub time_size: u8,
}

impl Desc {
    pub(crate) fn read(body: &[u8]) -> Result<Self> {
        Self::parse(body, 8)
            .or_else(|| Self::parse(body, 4))
            .ok_or_else(|| invalid("the model's description is damaged"))
    }

    /// The description with times of `time_size` bytes, when that reads the chunk exactly.
    fn parse(body: &[u8], time_size: u8) -> Option<Self> {
        let mut r = Reader::new(body);
        let t = |r: &mut Reader<'_>| -> Option<i64> {
            if time_size == 8 { r.r_i64().ok() } else { r.r_i32().ok().map(i64::from) }
        };
        let source_file = SmolStr::new(r.r_stringz().ok()?);
        let build_name = SmolStr::new(r.r_stringz().ok()?);
        let build_time = t(&mut r)?;
        let create_name = SmolStr::new(r.r_stringz().ok()?);
        let create_time = t(&mut r)?;
        let modif_name = SmolStr::new(r.r_stringz().ok()?);
        let modif_time = t(&mut r)?;
        r.eof().then_some(Self {
            source_file,
            build_name,
            build_time,
            create_name,
            create_time,
            modif_name,
            modif_time,
            time_size,
        })
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        let t = |w: &mut Writer, v: i64| if self.time_size == 4 { w.w_i32(v as i32) } else { w.w_i64(v) };
        w.w_stringz(&self.source_file);
        w.w_stringz(&self.build_name);
        t(w, self.build_time);
        w.w_stringz(&self.create_name);
        t(w, self.create_time);
        w.w_stringz(&self.modif_name);
        t(w, self.modif_time);
    }
}

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}

const CRC_TABLE: [u32; 256] = crc_table();

/// `crc32` of xrCore: the reflected CRC-32.
#[must_use]
pub fn crc32(data: &[u8]) -> u32 {
    let mut crc = 0xffff_ffffu32;
    for &b in data {
        crc = (crc >> 8) ^ CRC_TABLE[((crc & 0xff) ^ u32::from(b)) as usize];
    }
    crc ^ 0xffff_ffff
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crc32_is_the_standard_one() {
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(crc32(b""), 0);
    }

    #[test]
    fn a_description_knows_its_time_size() {
        for time_size in [4u8, 8] {
            let d = Desc {
                source_file: "a.object".into(),
                build_name: "\\\\pc\\me".into(),
                build_time: 0,
                create_name: "me".into(),
                create_time: 1_000,
                modif_name: "you".into(),
                modif_time: 2_000,
                time_size,
            };
            let mut w = Writer::new();
            d.write(&mut w);
            let b = w.into_bytes();
            assert_eq!(Desc::read(&b).unwrap(), d);
        }
    }
}
