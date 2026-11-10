//! Textures on the CPU, ready to upload: BC blocks as the file holds them, or RGBA8 levels.

use gz_fmt_tex::{DdsFormat, DdsKind, Image, MipFilter, load_image, mip_chain, read_dds, read_dds_info};

use crate::color::Color;
use crate::{Result, invalid};

/// How the texels of a [`TextureData`] are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TexelFormat {
    Rgba8,
    /// DXT1.
    Bc1,
    /// DXT3.
    Bc2,
    /// DXT5.
    Bc3,
}

impl TexelFormat {
    pub(crate) fn wgpu(self) -> wgpu::TextureFormat {
        match self {
            Self::Rgba8 => wgpu::TextureFormat::Rgba8Unorm,
            Self::Bc1 => wgpu::TextureFormat::Bc1RgbaUnorm,
            Self::Bc2 => wgpu::TextureFormat::Bc2RgbaUnorm,
            Self::Bc3 => wgpu::TextureFormat::Bc3RgbaUnorm,
        }
    }

    /// Bytes per pixel for RGBA, per 4 x 4 block otherwise.
    #[must_use]
    pub fn unit_bytes(self) -> u32 {
        match self {
            Self::Rgba8 => 4,
            Self::Bc1 => 8,
            Self::Bc2 | Self::Bc3 => 16,
        }
    }

    #[must_use]
    pub fn is_block(self) -> bool {
        self != Self::Rgba8
    }
}

/// A texture's levels, largest first, each half the one before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub format: TexelFormat,
    pub levels: Vec<Vec<u8>>,
}

/// Row bytes and rows of a level.
pub(crate) fn level_layout(format: TexelFormat, w: u32, h: u32) -> (u32, u32) {
    if format.is_block() { (w.div_ceil(4).max(1) * format.unit_bytes(), h.div_ceil(4).max(1)) } else { (w * 4, h) }
}

fn side(v: u32, mip: u32) -> u32 {
    (v >> mip.min(31)).max(1)
}

impl TextureData {
    /// RGBA pixels with every level under them made by a box filter.
    pub fn rgba(image: &Image, mips: bool) -> Result<Self> {
        image.check()?;
        let mut levels = Vec::new();
        levels.push(image.rgba.clone());
        if mips {
            levels.extend(mip_chain(image, MipFilter::Box)?.into_iter().map(|l| l.rgba));
        }
        Ok(Self { width: image.width, height: image.height, format: TexelFormat::Rgba8, levels })
    }

    /// Any picture a texture can be: a DDS (BC1-3 kept as blocks when `bc` - the device samples
    /// them - and the sides are whole blocks; anything else decoded), TGA, PNG, JPG or BMP.
    pub fn decode(bytes: &[u8], ext: Option<&str>, bc: bool) -> Result<Self> {
        if bytes.starts_with(b"DDS ") {
            if bc && let Some(t) = dds_blocks(bytes)? {
                return Ok(t);
            }
            let dds = read_dds(bytes)?;
            let levels = dds.layers.into_iter().next().ok_or_else(|| invalid("the DDS holds no picture"))?;
            let Some(top) = levels.first() else { return Err(invalid("the DDS holds no picture")) };
            // a chain that stops short is completed: the sampler expects every level
            let (w, h) = (top.width, top.height);
            let whole = levels.iter().enumerate().all(|(i, l)| l.width == side(w, i as u32) && l.height == side(h, i as u32));
            if levels.len() == 1 || !whole {
                return Self::rgba(top, true);
            }
            return Ok(Self { width: w, height: h, format: TexelFormat::Rgba8, levels: levels.into_iter().map(|l| l.rgba).collect() });
        }
        let loaded = load_image(bytes, ext)?;
        Self::rgba(&loaded.image, true)
    }

    /// A checker of two colours, `cell` pixels a square.
    #[must_use]
    pub fn checker(side: u32, cell: u32, a: Color, b: Color) -> Self {
        let side = side.clamp(1, 4096);
        let cell = cell.max(1);
        let mut rgba = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let c = if ((x / cell) + (y / cell)) % 2 == 0 { a } else { b };
                rgba.extend_from_slice(&c.0);
            }
        }
        let image = Image { width: side, height: side, rgba };
        Self::rgba(&image, true).unwrap_or_else(|_| Self::solid(a))
    }

    /// One pixel.
    #[must_use]
    pub fn solid(color: Color) -> Self {
        Self { width: 1, height: 1, format: TexelFormat::Rgba8, levels: vec![color.0.to_vec()] }
    }

    #[must_use]
    pub fn bytes(&self) -> usize {
        self.levels.iter().map(Vec::len).sum()
    }

    /// Whether every level has the size and the bytes its place in the chain asks for.
    pub fn check(&self) -> Result<()> {
        if self.width == 0 || self.height == 0 || self.levels.is_empty() {
            return Err(invalid("the texture has no pixels"));
        }
        if self.format.is_block() && (self.width % 4 != 0 || self.height % 4 != 0) {
            return Err(invalid(format!("a block-compressed texture of {} x {} is not whole blocks", self.width, self.height)));
        }
        for (i, level) in self.levels.iter().enumerate() {
            let (row, rows) = level_layout(self.format, side(self.width, i as u32), side(self.height, i as u32));
            if level.len() != (row * rows) as usize {
                return Err(invalid(format!("level {i} of the texture holds {} bytes, not {}", level.len(), row * rows)));
            }
        }
        Ok(())
    }

    pub(crate) fn level_size(&self, mip: u32) -> (u32, u32) {
        (side(self.width, mip), side(self.height, mip))
    }
}

/// The blocks of a BC1-3 DDS as the file holds them, or `None` when it is not one the GPU
/// can take as it is.
fn dds_blocks(bytes: &[u8]) -> Result<Option<TextureData>> {
    let info = read_dds_info(bytes)?;
    let format = match info.format {
        DdsFormat::Dxt1 => TexelFormat::Bc1,
        DdsFormat::Dxt3 => TexelFormat::Bc2,
        DdsFormat::Dxt5 => TexelFormat::Bc3,
        _ => return Ok(None),
    };
    if info.kind != DdsKind::Texture || info.layers != 1 || info.depth > 1 || info.width % 4 != 0 || info.height % 4 != 0 {
        return Ok(None);
    }
    // the header is 128 bytes, and 20 more when the pixel format's FourCC is "DX10"
    let at = if bytes.get(84..88) == Some(b"DX10") { 148 } else { 128 };
    let mut levels = Vec::with_capacity(info.mips.max(1) as usize);
    let mut cursor = at;
    for mip in 0..info.mips.max(1) {
        let (row, rows) = level_layout(format, side(info.width, mip), side(info.height, mip));
        let len = (row * rows) as usize;
        let Some(data) = bytes.get(cursor..cursor + len) else {
            return Err(invalid(format!("the DDS is cut short in level {mip}")));
        };
        levels.push(data.to_vec());
        cursor += len;
    }
    Ok(Some(TextureData { width: info.width, height: info.height, format, levels }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_tex::{DdsEncoding, encode_dds};

    fn picture(side: u32) -> Image {
        let mut rgba = Vec::new();
        for i in 0..side * side {
            rgba.extend_from_slice(&[(i * 7) as u8, (i * 3) as u8, 200, 255]);
        }
        Image::new(side, side, rgba).unwrap()
    }

    #[test]
    fn bc_blocks_pass_through() {
        let base = picture(16);
        let mut levels = vec![base.clone()];
        levels.extend(mip_chain(&base, MipFilter::Box).unwrap());
        let dds = encode_dds(&levels, DdsEncoding::Dxt5).unwrap();
        let t = TextureData::decode(&dds, Some("dds"), true).unwrap();
        assert_eq!(t.format, TexelFormat::Bc3);
        assert_eq!(t.levels.len(), 5);
        assert_eq!(t.levels[0].len(), 4 * 4 * 16);
        // the 2 x 2 and 1 x 1 levels are one block each
        assert_eq!(t.levels[4].len(), 16);
        t.check().unwrap();
        // the same file without BC support is decoded
        let rgba = TextureData::decode(&dds, Some("dds"), false).unwrap();
        assert_eq!(rgba.format, TexelFormat::Rgba8);
        assert_eq!(rgba.levels[0].len(), 16 * 16 * 4);
        rgba.check().unwrap();
    }

    #[test]
    fn pictures_get_a_mip_chain() {
        let t = TextureData::rgba(&picture(8), true).unwrap();
        assert_eq!(t.levels.iter().map(Vec::len).collect::<Vec<_>>(), vec![256, 64, 16, 4]);
        let c = TextureData::checker(64, 8, Color::WHITE, Color::BLACK);
        assert_eq!(c.levels.len(), 7);
        assert_eq!(&c.levels[0][..4], &[255, 255, 255, 255]);
        assert_eq!(&c.levels[0][8 * 4..8 * 4 + 4], &[0, 0, 0, 255]);
        let bad = TextureData { width: 6, height: 4, format: TexelFormat::Bc1, levels: vec![vec![0; 16]] };
        assert!(bad.check().is_err());
    }
}
