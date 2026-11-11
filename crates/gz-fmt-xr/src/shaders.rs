//! `shaders.xr` (`CResourceManager::OnDeviceCreate`, `CBlender_DESC`, `CConstant`, `CMatrix`)
//! and `shaders_xrlc.xr` (`Shader_xrLC_LIB`).

use gz_chunk::{Reader, Writer};
use smol_str::SmolStr;

use crate::{Encoding, Result, Text, invalid, numbered};

/// `WaveForm`: a function and its four arguments (20 bytes).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WaveForm {
    pub function: u32,
    pub args: [f32; 4],
}

impl WaveForm {
    fn read(r: &mut Reader<'_>) -> Result<Self> {
        Ok(Self { function: r.r_u32()?, args: [r.r_f32()?, r.r_f32()?, r.r_f32()?, r.r_f32()?] })
    }
}

/// `CMatrix`: a texture coordinate transform.
#[derive(Clone, Debug, PartialEq)]
pub struct MatrixDef {
    pub name: SmolStr,
    pub mode: u32,
    pub tcm: u32,
    pub scale_u: WaveForm,
    pub scale_v: WaveForm,
    pub rotate: WaveForm,
    pub scroll_u: WaveForm,
    pub scroll_v: WaveForm,
}

/// One blender: `CBlender_DESC` and the rest of its chunk, the property stream its class reads.
#[derive(Clone, Debug, PartialEq)]
pub struct Blender {
    /// `CLASS_ID`: eight characters.
    pub class_id: [u8; 8],
    pub name: SmolStr,
    pub computer: SmolStr,
    pub time: u32,
    pub version: u16,
    /// Everything after the 176-byte description.
    pub props: Vec<u8>,
}

impl Blender {
    /// The class id as text (`"BLENDER"`, `"LmBmmD  "`, ...), trailing blanks kept.
    #[must_use]
    pub fn class_name(&self) -> String {
        // CLASS_ID is a u64 whose first character is its highest byte
        let mut b = self.class_id;
        b.reverse();
        b.iter().map(|c| char::from(*c)).collect()
    }
}

/// `shaders.xr`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Shaders {
    /// `CConstant`: name and the R, G, B, A waves.
    pub constants: Vec<(SmolStr, [WaveForm; 4])>,
    pub matrices: Vec<MatrixDef>,
    pub blenders: Vec<Blender>,
    pub encoding: Encoding,
}

const DESC_SIZE: usize = 176;

fn fixed_str(bytes: &[u8], text: &mut Text) -> Result<SmolStr> {
    let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
    let mut with_nul = bytes[..end].to_vec();
    with_nul.push(0);
    text.r_str(&mut Reader::new(&with_nul))
}

impl Shaders {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let r = Reader::new(bytes);
        let mut text = Text::default();
        let mut out = Self::default();
        if let Some(c) = r.open_chunk(0)? {
            let mut cr = c.reader();
            while !cr.eof() {
                let name = text.r_str(&mut cr)?;
                out.constants.push((
                    name,
                    [
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                        WaveForm::read(&mut cr)?,
                    ],
                ));
            }
        }
        if let Some(c) = r.open_chunk(1)? {
            let mut cr = c.reader();
            while !cr.eof() {
                let name = text.r_str(&mut cr)?;
                out.matrices.push(MatrixDef {
                    name,
                    mode: cr.r_u32()?,
                    tcm: cr.r_u32()?,
                    scale_u: WaveForm::read(&mut cr)?,
                    scale_v: WaveForm::read(&mut cr)?,
                    rotate: WaveForm::read(&mut cr)?,
                    scroll_u: WaveForm::read(&mut cr)?,
                    scroll_v: WaveForm::read(&mut cr)?,
                });
            }
        }
        if let Some(c) = r.open_chunk(2)? {
            for b in numbered(&c.reader())? {
                let body = b.bytes();
                if body.len() < DESC_SIZE {
                    return Err(invalid("a blender's description is cut short"));
                }
                let mut d = Reader::new(body);
                let mut class_id = [0u8; 8];
                class_id.copy_from_slice(d.take(8)?);
                let name = fixed_str(d.take(128)?, &mut text)?;
                let computer = fixed_str(d.take(32)?, &mut text)?;
                let time = d.r_u32()?;
                let version = d.r_u16()?;
                out.blenders.push(Blender {
                    class_id,
                    name,
                    computer,
                    time,
                    version,
                    props: body[DESC_SIZE..].to_vec(),
                });
            }
        }
        out.encoding = text.encoding();
        Ok(out)
    }

    #[must_use]
    pub fn blender(&self, name: &str) -> Option<&Blender> {
        self.blenders.iter().find(|b| b.name.eq_ignore_ascii_case(name))
    }
}

/// One record of `shaders_xrlc.xr` (144 bytes).
#[derive(Clone, Debug, PartialEq)]
pub struct ShaderXrlc {
    /// The name field as stored: 128 bytes, zero-terminated, what follows kept.
    pub name_raw: [u8; 128],
    pub flags: u32,
    pub vert_translucency: f32,
    pub vert_ambient: f32,
    pub lm_density: f32,
}

impl ShaderXrlc {
    pub const COLLISION: u32 = 1 << 0;
    pub const RENDERING: u32 = 1 << 1;
    pub const OPTIMIZE_UV: u32 = 1 << 2;
    pub const LIGHT_VERTEX: u32 = 1 << 3;
    pub const LIGHT_CAST_SHADOW: u32 = 1 << 4;
    pub const LIGHT_SHARP: u32 = 1 << 5;

    #[must_use]
    pub fn name(&self) -> String {
        let end = self.name_raw.iter().position(|&b| b == 0).unwrap_or(128);
        gz_base::text::decode_guess(&self.name_raw[..end]).into_owned()
    }
}

/// `shaders_xrlc.xr`: a bare array of records.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShadersXrlc {
    pub shaders: Vec<ShaderXrlc>,
}

impl ShadersXrlc {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        if !bytes.len().is_multiple_of(144) {
            return Err(invalid("shaders_xrlc.xr is not a whole number of shaders"));
        }
        let mut r = Reader::new(bytes);
        let mut shaders = Vec::with_capacity(bytes.len() / 144);
        while !r.eof() {
            let mut name_raw = [0u8; 128];
            name_raw.copy_from_slice(r.take(128)?);
            shaders.push(ShaderXrlc {
                name_raw,
                flags: r.r_u32()?,
                vert_translucency: r.r_f32()?,
                vert_ambient: r.r_f32()?,
                lm_density: r.r_f32()?,
            });
        }
        Ok(Self { shaders })
    }

    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(self.shaders.len() * 144);
        for s in &self.shaders {
            w.w(&s.name_raw);
            w.w_u32(s.flags);
            w.w_f32(s.vert_translucency);
            w.w_f32(s.vert_ambient);
            w.w_f32(s.lm_density);
        }
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blenders_constants_and_matrices_are_read() {
        let mut w = Writer::new();
        w.chunk(0, |w| {
            w.w_stringz("c0");
            for _ in 0..4 {
                w.w_u32(1);
                (0..4).for_each(|_| w.w_f32(0.5));
            }
        });
        w.chunk(1, |w| {
            w.w_stringz("m0");
            w.w_u32(2);
            w.w_u32(3);
            for _ in 0..5 {
                w.w_u32(0);
                (0..4).for_each(|_| w.w_f32(0.0));
            }
        });
        w.chunk(2, |w| {
            w.chunk(0, |w| {
                w.w(b"REDNELB ");
                let mut name = [0u8; 128];
                name[..9].copy_from_slice(b"def_shade");
                w.w(&name);
                w.w(&[0u8; 32]);
                w.w_u32(7);
                w.w_u16(3);
                w.w(&[0, 0]);
                w.w(b"props");
            });
        });
        let s = Shaders::read(&w.into_bytes()).unwrap();
        assert_eq!(s.constants[0].1[3].args, [0.5; 4]);
        assert_eq!((s.matrices[0].mode, s.matrices[0].tcm), (2, 3));
        let b = s.blender("DEF_SHADE").unwrap();
        assert_eq!(
            (b.version, b.time, b.props.as_slice(), b.class_name().as_str()),
            (3, 7, &b"props"[..], " BLENDER")
        );
    }

    #[test]
    fn xrlc_shaders_round_trip() {
        let mut name_raw = [0xCDu8; 128];
        name_raw[..8].copy_from_slice(b"default\0");
        let lib = ShadersXrlc {
            shaders: vec![ShaderXrlc {
                name_raw,
                flags: 0x37,
                vert_translucency: 0.5,
                vert_ambient: 0.0,
                lm_density: 1.0,
            }],
        };
        let b = lib.write();
        assert_eq!(b.len(), 144);
        let back = ShadersXrlc::read(&b).unwrap();
        assert_eq!(back, lib);
        assert_eq!(back.shaders[0].name(), "default");
        assert!(ShadersXrlc::read(&b[..100]).is_err());
    }
}
