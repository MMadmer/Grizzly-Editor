//! What a texture description says about its texture.

use gz_base::SmolStr;
use gz_chunk::{Reader, Writer};

use crate::dds::DdsEncoding;
use crate::mip::MipFilter;
use crate::{Result, TexError};

pub(crate) const THM_CHUNK_TEXTUREPARAM: u32 = 0x0812;
pub(crate) const THM_CHUNK_TEXTURE_TYPE: u32 = 0x0814;
pub(crate) const THM_CHUNK_DETAIL_EXT: u32 = 0x0815;
pub(crate) const THM_CHUNK_MATERIAL: u32 = 0x0816;
pub(crate) const THM_CHUNK_BUMP: u32 = 0x0817;
pub(crate) const THM_CHUNK_EXT_NORMALMAP: u32 = 0x0818;
pub(crate) const THM_CHUNK_FADE_DELAY: u32 = 0x0819;

/// An enumeration stored as `u32` that keeps a value it does not know, so a file written by a
/// newer editor survives a round trip.
macro_rules! open_enum {
    ($(#[$m:meta])* $name:ident { $($v:ident = $n:literal => $label:expr),* $(,)? }) => {
        $(#[$m])*
        #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
        pub struct $name(pub u32);

        impl $name {
            $(pub const $v: Self = Self($n);)*

            /// The name the editor shows for the value, when it has one.
            #[must_use]
            pub fn label(self) -> Option<&'static str> {
                match self.0 {
                    $($n => $label,)*
                    _ => None,
                }
            }
        }
    };
}

open_enum! {
    /// The texture types.
    TextureType {
        IMAGE = 0 => Some("2D Texture"),
        CUBE_MAP = 1 => Some("Cube Map"),
        BUMP_MAP = 2 => Some("Bump Map"),
        NORMAL_MAP = 3 => Some("Normal Map"),
        TERRAIN = 4 => Some("Terrain"),
    }
}

open_enum! {
    /// The texture formats.
    TextureFormat {
        DXT1 = 0 => Some("DXT1"),
        ADXT1 = 1 => Some("DXT1 Alpha"),
        DXT3 = 2 => Some("DXT3"),
        DXT5 = 3 => Some("DXT5"),
        RGBA4444 = 4 => None,
        RGBA1555 = 5 => Some("16 bit (1:5:5:5)"),
        RGB565 = 6 => Some("16 bit (5:6:5)"),
        RGB = 7 => None,
        RGBA = 8 => Some("32 bit (8:8:8:8)"),
        NVHS = 9 => None,
        NVHU = 10 => None,
        A8 = 11 => Some("8 bit (alpha)"),
        L8 = 12 => Some("8 bit (luminance)"),
        A8L8 = 13 => Some("16 bit (alpha:luminance)"),
        BC4 = 14 => Some("BC4"),
        BC5 = 15 => Some("BC5"),
        BC6 = 16 => Some("BC6"),
        BC7 = 17 => Some("BC7"),
    }
}

open_enum! {
    /// The bump modes.
    BumpMode {
        RESERVED = 0 => None,
        NONE = 1 => Some("None"),
        USE = 2 => Some("Use"),
        USE_PARALLAX = 3 => Some("Use parallax"),
    }
}

open_enum! {
    /// The material kinds.
    TextureMaterial {
        OREN_NAYAR_BLIN = 0 => Some("OrenNayar <-> Blin"),
        BLIN_PHONG = 1 => Some("Blin <-> Phong"),
        PHONG_METAL = 2 => Some("Phong <-> Metal"),
        METAL_OREN_NAYAR = 3 => Some("Metal <-> OrenNayar"),
    }
}

open_enum! {
    /// The mip filter a description names (`kMIPFilter*`).
    MipFilterId {
        BOX = 0 => Some("Box"),
        CUBIC = 1 => Some("Cubic"),
        POINT = 2 => Some("Point"),
        TRIANGLE = 3 => Some("Triangle"),
        QUADRATIC = 4 => Some("Quadratic"),
        ADVANCED = 5 => Some("Advanced"),
        CATROM = 6 => Some("Catrom"),
        MITCHELL = 7 => Some("Mitchell"),
        GAUSSIAN = 8 => Some("Gaussian"),
        SINC = 9 => Some("Sinc"),
        BESSEL = 10 => Some("Bessel"),
        HANNING = 11 => Some("Hanning"),
        HAMMING = 12 => Some("Hamming"),
        BLACKMAN = 13 => Some("Blackman"),
        KAISER = 14 => Some("Kaiser"),
    }
}

bitflags::bitflags! {
    /// The description's flags. Bits the editor does not name are kept.
    #[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
    pub struct TextureFlags: u32 {
        const GENERATE_MIP_MAPS = 1 << 0;
        const BINARY_ALPHA = 1 << 1;
        const ALPHA_BORDER = 1 << 4;
        const COLOR_BORDER = 1 << 5;
        const FADE_TO_COLOR = 1 << 6;
        const FADE_TO_ALPHA = 1 << 7;
        const DITHER_COLOR = 1 << 8;
        const DITHER_EACH_MIP_LEVEL = 1 << 9;
        const DIFFUSE_DETAIL = 1 << 23;
        const IMPLICIT_LIGHTED = 1 << 24;
        const HAS_ALPHA = 1 << 25;
        const BUMP_DETAIL = 1 << 26;
        const _ = !0;
    }
}

/// A texture description. Colours are `0xAARRGGBB` as the file holds them.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureParams {
    pub fmt: TextureFormat,
    pub flags: TextureFlags,
    pub border_color: u32,
    pub fade_color: u32,
    /// Percent of the colour to fade in, 0..100.
    pub fade_amount: u32,
    /// The mip level fading starts at.
    pub fade_delay: u8,
    pub mip_filter: MipFilterId,
    /// The source picture's size.
    pub width: i32,
    pub height: i32,
    pub detail_name: SmolStr,
    pub detail_scale: f32,
    pub ty: TextureType,
    pub material: TextureMaterial,
    pub material_weight: f32,
    pub bump_virtual_height: f32,
    pub bump_mode: BumpMode,
    pub bump_name: SmolStr,
    pub ext_normal_map_name: SmolStr,
}

impl Default for TextureParams {
    /// Clears a description.
    fn default() -> Self {
        Self {
            fmt: TextureFormat::DXT1,
            flags: TextureFlags::GENERATE_MIP_MAPS | TextureFlags::DITHER_COLOR,
            border_color: 0,
            fade_color: 0,
            fade_amount: 0,
            fade_delay: 0,
            mip_filter: MipFilterId::BOX,
            width: 0,
            height: 0,
            detail_name: SmolStr::default(),
            detail_scale: 1.0,
            ty: TextureType::IMAGE,
            material: TextureMaterial::BLIN_PHONG,
            material_weight: 0.0,
            bump_virtual_height: 0.05,
            bump_mode: BumpMode::NONE,
            bump_name: SmolStr::default(),
            ext_normal_map_name: SmolStr::default(),
        }
    }
}

/// What DDS a description asks for.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DdsRequest {
    pub encoding: DdsEncoding,
    /// Whether the file gets every level under the picture.
    pub mips: bool,
    pub filter: MipFilter,
}

impl TextureParams {
    /// Reads the description's chunks from a thumbnail file.
    pub fn read(file: &Reader<'_>) -> Result<Self> {
        let mut p = Self::default();

        let main = file.need_chunk(THM_CHUNK_TEXTUREPARAM)?;
        let mut r = main.reader();
        p.fmt = TextureFormat(r.r_u32()?);
        p.flags = TextureFlags::from_bits_retain(r.r_u32()?);
        p.border_color = r.r_u32()?;
        p.fade_color = r.r_u32()?;
        p.fade_amount = r.r_u32()?;
        p.mip_filter = MipFilterId(r.r_u32()?);
        p.width = r.r_i32()?;
        p.height = r.r_i32()?;

        if let Some(c) = file.open_chunk(THM_CHUNK_TEXTURE_TYPE)? {
            p.ty = TextureType(c.reader().r_u32()?);
        }
        if let Some(c) = file.open_chunk(THM_CHUNK_DETAIL_EXT)? {
            let mut r = c.reader();
            p.detail_name = r.r_stringz()?.into();
            p.detail_scale = r.r_f32()?;
        }
        if let Some(c) = file.open_chunk(THM_CHUNK_MATERIAL)? {
            let mut r = c.reader();
            p.material = TextureMaterial(r.r_u32()?);
            p.material_weight = r.r_f32()?;
        }
        if let Some(c) = file.open_chunk(THM_CHUNK_BUMP)? {
            let mut r = c.reader();
            p.bump_virtual_height = r.r_f32()?;
            p.bump_mode = BumpMode(r.r_u32()?);
            // the retired "autogen" mode reads as no bump
            if p.bump_mode.0 < BumpMode::NONE.0 {
                p.bump_mode = BumpMode::NONE;
            }
            p.bump_name = r.r_stringz()?.into();
        }
        if let Some(c) = file.open_chunk(THM_CHUNK_EXT_NORMALMAP)? {
            p.ext_normal_map_name = c.reader().r_stringz()?.into();
        }
        if let Some(c) = file.open_chunk(THM_CHUNK_FADE_DELAY)? {
            p.fade_delay = c.reader().r_u8()?;
        }
        Ok(p)
    }

    /// Writes the description's chunks.
    pub fn write(&self, w: &mut Writer) {
        w.chunk(THM_CHUNK_TEXTUREPARAM, |w| {
            w.w_u32(self.fmt.0);
            w.w_u32(self.flags.bits());
            w.w_u32(self.border_color);
            w.w_u32(self.fade_color);
            w.w_u32(self.fade_amount);
            w.w_u32(self.mip_filter.0);
            w.w_i32(self.width);
            w.w_i32(self.height);
        });
        w.chunk(THM_CHUNK_TEXTURE_TYPE, |w| w.w_u32(self.ty.0));
        w.chunk(THM_CHUNK_DETAIL_EXT, |w| {
            w.w_stringz_cp1251(&self.detail_name);
            w.w_f32(self.detail_scale);
        });
        w.chunk(THM_CHUNK_MATERIAL, |w| {
            w.w_u32(self.material.0);
            w.w_f32(self.material_weight);
        });
        w.chunk(THM_CHUNK_BUMP, |w| {
            w.w_f32(self.bump_virtual_height);
            w.w_u32(self.bump_mode.0);
            w.w_stringz_cp1251(&self.bump_name);
        });
        w.chunk(THM_CHUNK_EXT_NORMALMAP, |w| w.w_stringz_cp1251(&self.ext_normal_map_name));
        w.chunk(THM_CHUNK_FADE_DELAY, |w| w.w_u8(self.fade_delay));
    }

    /// Whether the source picture has an alpha channel.
    #[must_use]
    pub fn has_alpha(&self) -> bool {
        self.flags.contains(TextureFlags::HAS_ALPHA)
    }

    /// Whether the game texture's format stores alpha.
    #[must_use]
    pub fn has_alpha_channel(&self) -> bool {
        matches!(
            self.fmt,
            TextureFormat::ADXT1
                | TextureFormat::DXT3
                | TextureFormat::DXT5
                | TextureFormat::RGBA4444
                | TextureFormat::RGBA1555
                | TextureFormat::RGBA
                | TextureFormat::BC7
        )
    }

    /// The description of a source picture seen for the first time: DXT3 with alpha, DXT1
    /// without; a strip six times as wide as it is high is a cube map without mip levels.
    #[must_use]
    pub fn for_new_texture(width: u32, height: u32, has_alpha: bool) -> Self {
        let mut p = Self { width: width as i32, height: height as i32, ..Self::default() };
        p.flags.set(TextureFlags::HAS_ALPHA, has_alpha);
        p.fmt = if has_alpha { TextureFormat::DXT3 } else { TextureFormat::DXT1 };
        if u64::from(height) * 6 == u64::from(width) {
            p.ty = TextureType::CUBE_MAP;
            p.flags.remove(TextureFlags::GENERATE_MIP_MAPS);
        }
        p
    }

    /// What picking a type in the editor resets.
    pub fn apply_type_defaults(&mut self) {
        let processing = TextureFlags::BINARY_ALPHA
            | TextureFlags::ALPHA_BORDER
            | TextureFlags::COLOR_BORDER
            | TextureFlags::FADE_TO_COLOR
            | TextureFlags::FADE_TO_ALPHA
            | TextureFlags::DITHER_COLOR
            | TextureFlags::DITHER_EACH_MIP_LEVEL
            | TextureFlags::BUMP_DETAIL;
        match self.ty {
            TextureType::BUMP_MAP => self.flags.remove(TextureFlags::GENERATE_MIP_MAPS),
            TextureType::NORMAL_MAP => {
                self.flags.remove(processing | TextureFlags::IMPLICIT_LIGHTED);
                self.flags.insert(TextureFlags::GENERATE_MIP_MAPS);
                self.mip_filter = MipFilterId::KAISER;
                self.fmt = TextureFormat::RGBA;
            }
            TextureType::TERRAIN => {
                self.flags.remove(processing | TextureFlags::GENERATE_MIP_MAPS);
                self.flags.insert(TextureFlags::IMPLICIT_LIGHTED);
                self.fmt = TextureFormat::DXT1;
            }
            _ => {}
        }
    }

    /// The estimate of video memory the editor shows. `frames` is the length of the texture's
    /// `.seq` animation, 0 or 1 without one.
    #[must_use]
    pub fn memory_usage(&self, frames: u32) -> u32 {
        let mut mem = (self.width as u32).wrapping_mul(self.height as u32).wrapping_mul(4);
        if self.flags.contains(TextureFlags::GENERATE_MIP_MAPS) {
            mem = mem.wrapping_mul(3) / 2;
        }
        match self.fmt {
            TextureFormat::DXT1 | TextureFormat::ADXT1 => mem /= 6,
            TextureFormat::DXT3 | TextureFormat::DXT5 => mem /= 4,
            TextureFormat::RGBA4444 | TextureFormat::RGBA1555 | TextureFormat::RGB565 => mem /= 2,
            _ => {}
        }
        mem.wrapping_mul(frames.max(1))
    }

    /// The DDS the editor makes for this description.
    ///
    /// Only Box, Triangle and Kaiser pick a filter of their own (Kaiser is Catmull-Rom); every
    /// other name is the resizer's default.
    pub fn dds_request(&self) -> Result<DdsRequest> {
        let encoding = match self.fmt {
            TextureFormat::DXT1 => DdsEncoding::Dxt1,
            TextureFormat::ADXT1 => DdsEncoding::Dxt1Alpha,
            TextureFormat::DXT3 => DdsEncoding::Dxt3,
            TextureFormat::DXT5 => DdsEncoding::Dxt5,
            TextureFormat::BC4 => DdsEncoding::Bc4,
            TextureFormat::BC5 => DdsEncoding::Bc5,
            TextureFormat::BC6 | TextureFormat::BC7 => {
                return Err(TexError::Unsupported(format!(
                    "the editor has no {} compressor",
                    self.fmt.label().unwrap_or("BC6/BC7")
                )));
            }
            TextureFormat::RGB => DdsEncoding::Rgb8,
            // RGBA, and every format the compressor does not know, is stored as 32 bit
            _ => DdsEncoding::Rgba8,
        };
        let filter = match self.mip_filter {
            MipFilterId::BOX => MipFilter::Box,
            MipFilterId::TRIANGLE => MipFilter::Triangle,
            MipFilterId::KAISER => MipFilter::CatmullRom,
            _ => MipFilter::Default,
        };
        // the game textures of the library were made with the whole mip chain whatever the
        // description's mip switch says, and with neither the fade nor the border options
        Ok(DdsRequest { encoding, mips: true, filter })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_clear() {
        let p = TextureParams::default();
        assert_eq!(p.flags.bits(), 0x101);
        assert_eq!(p.bump_mode, BumpMode::NONE);
        assert_eq!(p.material, TextureMaterial::BLIN_PHONG);
        assert_eq!(p.detail_scale, 1.0);
        assert!(!p.has_alpha());
        assert!(!p.has_alpha_channel());
    }

    #[test]
    fn round_trips_through_chunks() {
        let p = TextureParams {
            fmt: TextureFormat::DXT5,
            flags: TextureFlags::from_bits_retain(0x8200_0101),
            border_color: 0x11223344,
            fade_color: 0xaabbccdd,
            fade_amount: 50,
            fade_delay: 3,
            mip_filter: MipFilterId::KAISER,
            width: 1024,
            height: 512,
            detail_name: "detail\\detail_grnd_asphalt".into(),
            detail_scale: 6.5,
            ty: TextureType::BUMP_MAP,
            material: TextureMaterial::PHONG_METAL,
            material_weight: 0.25,
            bump_virtual_height: 0.02,
            bump_mode: BumpMode::USE_PARALLAX,
            bump_name: "wall\\wall_bump".into(),
            ext_normal_map_name: "wall\\wall_nmap".into(),
        };
        let mut w = Writer::new();
        p.write(&mut w);
        let bytes = w.into_bytes();
        assert_eq!(TextureParams::read(&Reader::new(&bytes)).unwrap(), p);
        // unknown flag bits survive
        assert_eq!(p.flags.bits() & 0x8000_0000, 0x8000_0000);
    }

    #[test]
    fn old_files_keep_defaults_for_missing_chunks() {
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_TEXTUREPARAM, |w| {
            for v in [2u32, 1, 0, 0, 0, 0, 64, 64] {
                w.w_u32(v);
            }
        });
        w.chunk(THM_CHUNK_BUMP, |w| {
            w.w_f32(0.1);
            w.w_u32(0);
            w.w_stringz("x");
        });
        let bytes = w.into_bytes();
        let p = TextureParams::read(&Reader::new(&bytes)).unwrap();
        assert_eq!(p.fmt, TextureFormat::DXT3);
        assert_eq!(p.ty, TextureType::IMAGE);
        assert_eq!(p.bump_mode, BumpMode::NONE);
        assert_eq!(p.bump_name, "x");
        assert_eq!(p.fade_delay, 0);
    }

    #[test]
    fn a_missing_or_short_main_chunk_is_an_error() {
        assert!(TextureParams::read(&Reader::new(&[])).is_err());
        let mut w = Writer::new();
        w.chunk(THM_CHUNK_TEXTUREPARAM, |w| w.w_u32(0));
        assert!(TextureParams::read(&Reader::new(&w.into_bytes())).is_err());
    }

    #[test]
    fn new_textures_and_type_changes() {
        let p = TextureParams::for_new_texture(768, 128, true);
        assert_eq!(p.ty, TextureType::CUBE_MAP);
        assert_eq!(p.fmt, TextureFormat::DXT3);
        assert!(!p.flags.contains(TextureFlags::GENERATE_MIP_MAPS));
        assert!(p.has_alpha());

        let mut n = TextureParams { ty: TextureType::NORMAL_MAP, ..TextureParams::default() };
        n.apply_type_defaults();
        assert_eq!(n.fmt, TextureFormat::RGBA);
        assert_eq!(n.mip_filter, MipFilterId::KAISER);
        assert!(!n.flags.contains(TextureFlags::DITHER_COLOR));
    }

    #[test]
    fn requests_and_memory() {
        let mut p = TextureParams { width: 256, height: 256, ..TextureParams::default() };
        assert_eq!(p.memory_usage(0), 256 * 256 * 4 * 3 / 2 / 6);
        let r = p.dds_request().unwrap();
        assert_eq!((r.encoding, r.mips, r.filter), (DdsEncoding::Dxt1, true, MipFilter::Box));
        p.fmt = TextureFormat::A8;
        p.mip_filter = MipFilterId::GAUSSIAN;
        let r = p.dds_request().unwrap();
        assert_eq!((r.encoding, r.filter), (DdsEncoding::Rgba8, MipFilter::Default));
        p.fmt = TextureFormat::BC7;
        assert!(matches!(p.dds_request(), Err(TexError::Unsupported(_))));
        assert_eq!(TextureFormat(99).label(), None);
        assert_eq!(TextureFormat::DXT5.label(), Some("DXT5"));
    }
}
