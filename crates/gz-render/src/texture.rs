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

/// A texture's levels, largest first, each half the one before; a cube map holds its six faces
/// one after the other, each with all its levels (the order of a DDS file).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextureData {
    pub width: u32,
    pub height: u32,
    pub format: TexelFormat,
    /// 1, or 6 for a cube map (+X, -X, +Y, -Y, +Z, -Z).
    pub layers: u32,
    pub levels: Vec<Vec<u8>>,
}

/// Row bytes and rows of a level.
pub(crate) fn level_layout(format: TexelFormat, w: u32, h: u32) -> (u32, u32) {
    if format.is_block() {
        (w.div_ceil(4).max(1) * format.unit_bytes(), h.div_ceil(4).max(1))
    } else {
        (w * 4, h)
    }
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
        Ok(Self { width: image.width, height: image.height, format: TexelFormat::Rgba8, layers: 1, levels })
    }

    /// Any picture a texture can be: a DDS (BC1-3 kept as blocks when `bc` - the device samples
    /// them - and the sides are whole blocks; anything else decoded; a cube map with its six
    /// faces), TGA, PNG, JPG or BMP.
    pub fn decode(bytes: &[u8], ext: Option<&str>, bc: bool) -> Result<Self> {
        if bytes.starts_with(b"DDS ") {
            if bc && let Some(t) = dds_blocks(bytes)? {
                return Ok(t);
            }
            let dds = read_dds(bytes)?;
            let cube = dds.info.kind == DdsKind::Cube && dds.layers.len() == 6;
            let mut faces = Vec::with_capacity(if cube { 6 } else { 1 });
            for levels in dds.layers.into_iter().take(if cube { 6 } else { 1 }) {
                faces.push(Self::chain(levels)?);
            }
            let Some(first) = faces.first() else { return Err(invalid("the DDS holds no picture")) };
            let (width, height) = (first.width, first.height);
            if !cube {
                return faces.pop().ok_or_else(|| invalid("the DDS holds no picture"));
            }
            // the faces of a cube are one size with one chain: a short one is made whole
            let n = first.levels.len();
            if faces.iter().any(|f| f.width != width || f.height != height || f.levels.len() != n) {
                return Err(invalid("the faces of the cube map differ in size"));
            }
            let levels = faces.into_iter().flat_map(|f| f.levels).collect();
            return Ok(Self { width, height, format: TexelFormat::Rgba8, layers: 6, levels });
        }
        let loaded = load_image(bytes, ext)?;
        Self::rgba(&loaded.image, true)
    }

    /// One picture's levels as RGBA; a chain that stops short is completed: the sampler expects
    /// every level.
    fn chain(levels: Vec<Image>) -> Result<Self> {
        let Some(top) = levels.first() else { return Err(invalid("the DDS holds no picture")) };
        let (w, h) = (top.width, top.height);
        let whole =
            levels.iter().enumerate().all(|(i, l)| l.width == side(w, i as u32) && l.height == side(h, i as u32));
        if levels.len() == 1 || !whole {
            return Self::rgba(top, true);
        }
        Ok(Self {
            width: w,
            height: h,
            format: TexelFormat::Rgba8,
            layers: 1,
            levels: levels.into_iter().map(|l| l.rgba).collect(),
        })
    }

    /// Levels of each layer.
    #[must_use]
    pub fn mips(&self) -> u32 {
        (self.levels.len() as u32) / self.layers.max(1)
    }

    /// Whether the texture is a cube map.
    #[must_use]
    pub fn is_cube(&self) -> bool {
        self.layers == 6 && self.width == self.height
    }

    /// A checker of two colours, `cell` pixels a square.
    #[must_use]
    pub fn checker(side: u32, cell: u32, a: Color, b: Color) -> Self {
        let side = side.clamp(1, 4096);
        let cell = cell.max(1);
        let mut rgba = Vec::with_capacity((side * side * 4) as usize);
        for y in 0..side {
            for x in 0..side {
                let c = if ((x / cell) + (y / cell)).is_multiple_of(2) { a } else { b };
                rgba.extend_from_slice(&c.0);
            }
        }
        let image = Image { width: side, height: side, rgba };
        Self::rgba(&image, true).unwrap_or_else(|_| Self::solid(a))
    }

    /// One pixel.
    #[must_use]
    pub fn solid(color: Color) -> Self {
        Self { width: 1, height: 1, format: TexelFormat::Rgba8, layers: 1, levels: vec![color.0.to_vec()] }
    }

    /// A cube map of one colour, a pixel a face.
    #[must_use]
    pub fn solid_cube(color: Color) -> Self {
        Self { width: 1, height: 1, format: TexelFormat::Rgba8, layers: 6, levels: vec![color.0.to_vec(); 6] }
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
        if !matches!(self.layers, 1 | 6) || !(self.levels.len() as u32).is_multiple_of(self.layers) {
            return Err(invalid(format!(
                "a texture of {} layers cannot hold {} levels",
                self.layers,
                self.levels.len()
            )));
        }
        if self.layers == 6 && self.width != self.height {
            return Err(invalid("the faces of a cube map are not square"));
        }
        let mips = self.mips().max(1) as usize;
        if self.format.is_block() && (!self.width.is_multiple_of(4) || !self.height.is_multiple_of(4)) {
            return Err(invalid(format!(
                "a block-compressed texture of {} x {} is not whole blocks",
                self.width, self.height
            )));
        }
        for (n, level) in self.levels.iter().enumerate() {
            let i = n % mips;
            let (row, rows) =
                level_layout(self.format, side(self.width, i as u32), side(self.height, i as u32));
            if level.len() != (row * rows) as usize {
                return Err(invalid(format!(
                    "level {i} of the texture holds {} bytes, not {}",
                    level.len(),
                    row * rows
                )));
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
    let layers = match (info.kind, info.layers) {
        (DdsKind::Texture, 1) => 1,
        (DdsKind::Cube, 6) if info.width == info.height => 6,
        _ => return Ok(None),
    };
    if info.depth > 1 || info.width % 4 != 0 || info.height % 4 != 0 {
        return Ok(None);
    }
    // the header is 128 bytes, and 20 more when the pixel format's FourCC is "DX10"
    let at = if bytes.get(84..88) == Some(b"DX10") { 148 } else { 128 };
    let mips = info.mips.max(1);
    let mut levels = Vec::with_capacity((mips * layers) as usize);
    let mut cursor = at;
    // a cube map's faces come one after the other, each with its whole chain
    for _ in 0..layers {
        for mip in 0..mips {
            let (row, rows) = level_layout(format, side(info.width, mip), side(info.height, mip));
            let len = (row * rows) as usize;
            let Some(data) = bytes.get(cursor..cursor + len) else {
                return Err(invalid(format!("the DDS is cut short in level {mip}")));
            };
            levels.push(data.to_vec());
            cursor += len;
        }
    }
    Ok(Some(TextureData { width: info.width, height: info.height, format, layers, levels }))
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
        let bad = TextureData { width: 6, height: 4, format: TexelFormat::Bc1, layers: 1, levels: vec![vec![0; 16]] };
        assert!(bad.check().is_err());
    }

    #[test]
    fn cube_maps_keep_their_faces() {
        let c = TextureData::solid_cube(Color::WHITE);
        c.check().unwrap();
        assert!(c.is_cube());
        assert_eq!(c.mips(), 1);
        // six faces of a 4 x 4 picture with its three levels
        let mut levels = Vec::new();
        for _ in 0..6 {
            levels.extend([vec![0u8; 64], vec![0u8; 16], vec![0u8; 4]]);
        }
        let t = TextureData { width: 4, height: 4, format: TexelFormat::Rgba8, layers: 6, levels };
        t.check().unwrap();
        assert_eq!(t.mips(), 3);
        let short = TextureData { layers: 6, levels: vec![vec![0u8; 64]; 5], ..t.clone() };
        assert!(short.check().is_err());
        let flat = TextureData { width: 8, height: 4, ..t };
        assert!(flat.check().is_err());
    }
}
