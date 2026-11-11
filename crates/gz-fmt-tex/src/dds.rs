//! DDS: every layout the game and the editor meet, read into RGBA8; and the files the editor
//! writes.

use rayon::prelude::*;

use crate::bc::{self, AlphaRule, Block};
use crate::mip::{mip_chain, mip_count};
use crate::params::DdsRequest;
use crate::{Image, MAX_SIDE, Result, TexError};

const MAGIC: &[u8; 4] = b"DDS ";
const HEADER_SIZE: usize = 124;

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDSD_MIPMAPCOUNT: u32 = 0x2_0000;
const DDSD_LINEARSIZE: u32 = 0x8_0000;
const DDSD_DEPTH: u32 = 0x80_0000;

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;

const DDSCAPS_TEXTURE: u32 = 0x1000;
const DDSCAPS2_CUBEMAP: u32 = 0x200;
const DDSCAPS2_FACES: u32 = 0xfc00;

const fn fourcc(s: &[u8; 4]) -> u32 {
    u32::from_le_bytes(*s)
}

/// The bits and channel masks of an uncompressed pixel. A mask of zero is a channel the format
/// does not have.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PixelMasks {
    pub bits: u32,
    pub r: u32,
    pub g: u32,
    pub b: u32,
    pub a: u32,
}

/// How a DDS stores its pixels.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DdsFormat {
    Dxt1,
    /// DXT3, and DXT2.
    Dxt3,
    /// DXT5, and DXT4.
    Dxt5,
    /// One channel in blocks (ATI1); read as grey.
    Bc4,
    /// Two channels in blocks (ATI2); read as red and green.
    Bc5,
    /// High dynamic range in blocks; read clamped to 0..1.
    Bc6h {
        signed: bool,
    },
    Bc7,
    /// Uncompressed colour: A8R8G8B8, X8R8G8B8, R8G8B8, R5G6B5, A1R5G5B5, A4R4G4B4 and the like.
    Rgb(PixelMasks),
    /// Luminance in the red mask, alpha when there is a mask for it (L8, A8L8); read as grey.
    Luminance(PixelMasks),
    /// Alpha alone (A8); read as black with that alpha, as the game samples it.
    Alpha(PixelMasks),
    /// Four 16-bit channels.
    Rgba16,
    /// Floating point channels of 16 or 32 bits, one to four of them; read clamped to 0..1.
    Float {
        bits: u8,
        channels: u8,
    },
}

impl DdsFormat {
    /// Whether the format stores an alpha channel. DXT1's 1-bit alpha is a property of single
    /// blocks and shows only in the decoded pixels.
    #[must_use]
    pub fn has_alpha_channel(self) -> bool {
        match self {
            DdsFormat::Dxt3 | DdsFormat::Dxt5 | DdsFormat::Bc7 | DdsFormat::Alpha(_) | DdsFormat::Rgba16 => {
                true
            }
            DdsFormat::Rgb(m) | DdsFormat::Luminance(m) => m.a != 0,
            DdsFormat::Float { channels, .. } => channels == 4,
            _ => false,
        }
    }

    fn block(self) -> Option<Block> {
        Some(match self {
            DdsFormat::Dxt1 => Block::Bc1,
            DdsFormat::Dxt3 => Block::Bc2,
            DdsFormat::Dxt5 => Block::Bc3,
            DdsFormat::Bc4 => Block::Bc4,
            DdsFormat::Bc5 => Block::Bc5,
            DdsFormat::Bc6h { signed } => Block::Bc6h { signed },
            DdsFormat::Bc7 => Block::Bc7,
            _ => return None,
        })
    }

    /// Bytes one level of this size takes in the file.
    fn level_len(self, width: usize, height: usize) -> usize {
        match self {
            DdsFormat::Rgb(m) | DdsFormat::Luminance(m) | DdsFormat::Alpha(m) => {
                width * height * (m.bits as usize / 8)
            }
            DdsFormat::Rgba16 => width * height * 8,
            DdsFormat::Float { bits, channels } => {
                width * height * usize::from(bits / 8) * usize::from(channels)
            }
            other => other.block().map_or(0, |b| bc::level_bytes(b, width, height)),
        }
    }
}

/// What a DDS is.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DdsKind {
    /// One picture, or an array of them.
    Texture,
    /// Cube faces; a layer is a face.
    Cube,
    /// A volume; a layer is a depth slice, and deeper levels have fewer of them.
    Volume,
}

/// A DDS header.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct DdsInfo {
    pub width: u32,
    pub height: u32,
    /// Depth slices of the top level; 1 unless the file is a volume.
    pub depth: u32,
    /// Levels per layer.
    pub mips: u32,
    /// Pictures at the top level: 1, the cube faces, the array elements or the depth slices.
    pub layers: u32,
    pub kind: DdsKind,
    pub format: DdsFormat,
    data_at: usize,
}

fn mip_side(side: u32, level: u32) -> u32 {
    (side >> level.min(31)).max(1)
}

impl DdsInfo {
    /// The size of a level.
    #[must_use]
    pub fn level_size(&self, mip: u32) -> (u32, u32) {
        (mip_side(self.width, mip), mip_side(self.height, mip))
    }

    /// Where a level lies in the file: offset, length, width, height.
    fn locate(&self, layer: u32, mip: u32) -> Option<(usize, usize, u32, u32)> {
        if mip >= self.mips {
            return None;
        }
        let len_of = |m: u32| {
            self.format.level_len(mip_side(self.width, m) as usize, mip_side(self.height, m) as usize)
        };
        let (w, h) = self.level_size(mip);
        let mut at = self.data_at;
        match self.kind {
            DdsKind::Volume => {
                if layer >= mip_side(self.depth, mip) {
                    return None;
                }
                for m in 0..mip {
                    at = at.checked_add(len_of(m).checked_mul(mip_side(self.depth, m) as usize)?)?;
                }
                at = at.checked_add(len_of(mip).checked_mul(layer as usize)?)?;
            }
            DdsKind::Texture | DdsKind::Cube => {
                if layer >= self.layers {
                    return None;
                }
                let chain: usize = (0..self.mips).map(len_of).sum();
                at = at.checked_add(chain.checked_mul(layer as usize)?)?;
                for m in 0..mip {
                    at = at.checked_add(len_of(m))?;
                }
            }
        }
        Some((at, len_of(mip), w, h))
    }

    /// Bytes of pixel data the header promises.
    fn data_len(&self) -> Option<usize> {
        let len_of = |m: u32| {
            self.format.level_len(mip_side(self.width, m) as usize, mip_side(self.height, m) as usize)
        };
        let mut total = 0usize;
        for m in 0..self.mips {
            let count = match self.kind {
                DdsKind::Volume => mip_side(self.depth, m),
                _ => self.layers,
            };
            total = total.checked_add(len_of(m).checked_mul(count as usize)?)?;
        }
        Some(total)
    }
}

/// A decoded DDS: `layers[layer][level]`. A volume's deeper slices have fewer levels.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dds {
    pub info: DdsInfo,
    pub layers: Vec<Vec<Image>>,
}

fn u32_at(bytes: &[u8], at: usize) -> u32 {
    let mut raw = [0u8; 4];
    if let Some(src) = bytes.get(at..at + 4) {
        raw.copy_from_slice(src);
    }
    u32::from_le_bytes(raw)
}

fn rgb_masks(bits: u32, r: u32, g: u32, b: u32, a: u32) -> DdsFormat {
    DdsFormat::Rgb(PixelMasks { bits, r, g, b, a })
}

fn dxgi_format(code: u32) -> Result<DdsFormat> {
    Ok(match code {
        2 => DdsFormat::Float { bits: 32, channels: 4 },
        6 => DdsFormat::Float { bits: 32, channels: 3 },
        10 => DdsFormat::Float { bits: 16, channels: 4 },
        11 => DdsFormat::Rgba16,
        16 => DdsFormat::Float { bits: 32, channels: 2 },
        34 => DdsFormat::Float { bits: 16, channels: 2 },
        41 => DdsFormat::Float { bits: 32, channels: 1 },
        54 => DdsFormat::Float { bits: 16, channels: 1 },
        28 | 29 => rgb_masks(32, 0xff, 0xff00, 0xff_0000, 0xff00_0000),
        87 | 91 => rgb_masks(32, 0xff_0000, 0xff00, 0xff, 0xff00_0000),
        88 | 93 => rgb_masks(32, 0xff_0000, 0xff00, 0xff, 0),
        49 => rgb_masks(16, 0xff, 0xff00, 0, 0),
        85 => rgb_masks(16, 0xf800, 0x07e0, 0x001f, 0),
        86 => rgb_masks(16, 0x7c00, 0x03e0, 0x001f, 0x8000),
        115 => rgb_masks(16, 0x0f00, 0x00f0, 0x000f, 0xf000),
        61 => DdsFormat::Luminance(PixelMasks { bits: 8, r: 0xff, g: 0, b: 0, a: 0 }),
        65 => DdsFormat::Alpha(PixelMasks { bits: 8, r: 0, g: 0, b: 0, a: 0xff }),
        70..=72 => DdsFormat::Dxt1,
        73..=75 => DdsFormat::Dxt3,
        76..=78 => DdsFormat::Dxt5,
        79 | 80 => DdsFormat::Bc4,
        82 | 83 => DdsFormat::Bc5,
        94 | 95 => DdsFormat::Bc6h { signed: false },
        96 => DdsFormat::Bc6h { signed: true },
        97..=99 => DdsFormat::Bc7,
        other => return Err(TexError::Unsupported(format!("DXGI format {other}"))),
    })
}

/// Reads the header of a DDS.
pub fn read_dds_info(bytes: &[u8]) -> Result<DdsInfo> {
    if bytes.get(..4) != Some(MAGIC.as_slice()) {
        return Err(TexError::NotA("DDS"));
    }
    if bytes.len() < 4 + HEADER_SIZE {
        return Err(TexError::Truncated { need: 4 + HEADER_SIZE - bytes.len() });
    }
    if u32_at(bytes, 4) as usize != HEADER_SIZE {
        return Err(TexError::NotA("DDS"));
    }
    let flags = u32_at(bytes, 8);
    let height = u32_at(bytes, 12);
    let width = u32_at(bytes, 16);
    if width == 0 || height == 0 || width > MAX_SIDE || height > MAX_SIDE {
        return Err(TexError::BadSize { width: u64::from(width), height: u64::from(height) });
    }
    let pf_flags = u32_at(bytes, 80);
    let four = u32_at(bytes, 84);
    let bits = u32_at(bytes, 88);
    let (r, g, b, a) = (u32_at(bytes, 92), u32_at(bytes, 96), u32_at(bytes, 100), u32_at(bytes, 104));
    let caps2 = u32_at(bytes, 112);

    let mut data_at = 4 + HEADER_SIZE;
    let mut array = 1u32;
    let mut cube = caps2 & DDSCAPS2_CUBEMAP != 0 || caps2 & DDSCAPS2_FACES != 0;
    let mut faces = (caps2 & DDSCAPS2_FACES).count_ones();

    let format = if pf_flags & DDPF_FOURCC != 0 {
        match &four.to_le_bytes() {
            b"DXT1" => DdsFormat::Dxt1,
            b"DXT2" | b"DXT3" => DdsFormat::Dxt3,
            b"DXT4" | b"DXT5" => DdsFormat::Dxt5,
            b"ATI1" | b"BC4U" => DdsFormat::Bc4,
            b"ATI2" | b"BC5U" => DdsFormat::Bc5,
            b"DX10" => {
                if bytes.len() < data_at + 20 {
                    return Err(TexError::Truncated { need: data_at + 20 - bytes.len() });
                }
                let format = dxgi_format(u32_at(bytes, data_at))?;
                if u32_at(bytes, data_at + 8) & 0x4 != 0 {
                    cube = true;
                    faces = 6;
                }
                array = u32_at(bytes, data_at + 12).max(1);
                data_at += 20;
                format
            }
            _ => match four {
                36 => DdsFormat::Rgba16,
                111 => DdsFormat::Float { bits: 16, channels: 1 },
                112 => DdsFormat::Float { bits: 16, channels: 2 },
                113 => DdsFormat::Float { bits: 16, channels: 4 },
                114 => DdsFormat::Float { bits: 32, channels: 1 },
                115 => DdsFormat::Float { bits: 32, channels: 2 },
                116 => DdsFormat::Float { bits: 32, channels: 4 },
                _ => {
                    let name: String = four
                        .to_le_bytes()
                        .iter()
                        .map(|&c| if c.is_ascii_graphic() { c as char } else { '?' })
                        .collect();
                    return Err(TexError::Unsupported(format!("DDS pixel format '{name}'")));
                }
            },
        }
    } else {
        if !matches!(bits, 8 | 16 | 24 | 32) {
            return Err(TexError::Unsupported(format!("DDS pixels of {bits} bits")));
        }
        let alpha = if pf_flags & (DDPF_ALPHAPIXELS | DDPF_ALPHA) != 0 { a } else { 0 };
        let masks = PixelMasks { bits, r, g, b, a: alpha };
        if r == 0 && g == 0 && b == 0 {
            if alpha == 0 {
                return Err(TexError::Unsupported("DDS pixels without a channel mask".into()));
            }
            DdsFormat::Alpha(masks)
        } else if pf_flags & DDPF_LUMINANCE != 0
            || (g == 0 && b == 0 && pf_flags & DDPF_RGB == 0 && bits <= 16)
        {
            DdsFormat::Luminance(masks)
        } else {
            DdsFormat::Rgb(masks)
        }
    };

    let volume = flags & DDSD_DEPTH != 0 && !cube;
    let depth = if volume { u32_at(bytes, 24).max(1) } else { 1 };
    if depth > MAX_SIDE {
        return Err(TexError::BadSize { width: u64::from(width), height: u64::from(depth) });
    }
    let possible = mip_count(width, height).max(if volume { mip_count(depth, 1) } else { 1 });
    let mips = if flags & DDSD_MIPMAPCOUNT != 0 { u32_at(bytes, 28).clamp(1, possible) } else { 1 };
    let (kind, layers) = if volume {
        (DdsKind::Volume, depth)
    } else if cube {
        (DdsKind::Cube, array.saturating_mul(if faces == 0 { 6 } else { faces }))
    } else {
        (DdsKind::Texture, array)
    };
    Ok(DdsInfo { width, height, depth, mips, layers, kind, format, data_at })
}

/// Where a channel sits in a pixel: shift and width of the mask's lowest run of bits.
#[derive(Copy, Clone)]
struct Channel {
    mask: u32,
    shift: u32,
    size: u32,
}

impl Channel {
    fn new(mask: u32) -> Self {
        if mask == 0 {
            return Self { mask, shift: 0, size: 0 };
        }
        let shift = mask.trailing_zeros();
        let size = (mask >> shift).trailing_ones();
        Self { mask, shift, size }
    }

    /// The channel of a pixel as 8 bits: narrower values repeat their bits, wider ones drop the
    /// low ones.
    fn get(self, pixel: u32) -> u8 {
        if self.size == 0 {
            return 0;
        }
        let c = (pixel & self.mask) >> self.shift & ((1u64 << self.size) - 1) as u32;
        if self.size >= 8 {
            return (c >> (self.size - 8)) as u8;
        }
        let mut out = 0u32;
        let mut have = 0u32;
        while have < 8 {
            let take = self.size.min(8 - have);
            out = (out << take) | (c >> (self.size - take));
            have += take;
        }
        out as u8
    }
}

fn half_to_f32(h: u16) -> f32 {
    let sign = if h & 0x8000 != 0 { -1.0f32 } else { 1.0 };
    let exp = i32::from((h >> 10) & 31);
    let frac = f32::from(h & 0x3ff);
    let v = match exp {
        0 => frac * 2f32.powi(-24),
        31 => {
            if frac == 0.0 {
                f32::INFINITY
            } else {
                f32::NAN
            }
        }
        e => (1.0 + frac / 1024.0) * 2f32.powi(e - 15),
    };
    sign * v
}

fn unit_to_u8(v: f32) -> u8 {
    // NaN clamps to zero
    (v.clamp(0.0, 1.0) * 255.0 + 0.5) as u8
}

/// Decodes one level. `data` holds at least the level's bytes.
fn decode(format: DdsFormat, data: &[u8], width: usize, height: usize) -> Vec<u8> {
    if let Some(block) = format.block() {
        return bc::decode_level(block, data, width, height);
    }
    let mut out = vec![0u8; width * height * 4];
    let px_bytes = format.level_len(1, 1).max(1);
    let rows = out.par_chunks_mut(width * 4).zip(data.par_chunks(width * px_bytes));
    match format {
        DdsFormat::Rgb(m) | DdsFormat::Luminance(m) | DdsFormat::Alpha(m) => {
            let (r, g, b, a) = (Channel::new(m.r), Channel::new(m.g), Channel::new(m.b), Channel::new(m.a));
            let kind = format;
            rows.for_each(|(dst, src)| {
                for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.chunks_exact(px_bytes)) {
                    let mut raw = [0u8; 4];
                    raw[..px_bytes.min(4)].copy_from_slice(&s[..px_bytes.min(4)]);
                    let v = u32::from_le_bytes(raw);
                    let alpha = if a.size == 0 { 255 } else { a.get(v) };
                    let px = match kind {
                        DdsFormat::Luminance(_) => {
                            let l = r.get(v);
                            [l, l, l, alpha]
                        }
                        DdsFormat::Alpha(_) => [0, 0, 0, alpha],
                        _ => [r.get(v), g.get(v), b.get(v), alpha],
                    };
                    d.copy_from_slice(&px);
                }
            });
        }
        DdsFormat::Rgba16 => rows.for_each(|(dst, src)| {
            for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.as_chunks::<8>().0) {
                // the high byte of each little-endian channel
                d.copy_from_slice(&[s[1], s[3], s[5], s[7]]);
            }
        }),
        DdsFormat::Float { bits, channels } => {
            let step = usize::from(bits / 8);
            let channels = usize::from(channels);
            rows.for_each(|(dst, src)| {
                for (d, s) in dst.as_chunks_mut::<4>().0.iter_mut().zip(src.chunks_exact(px_bytes)) {
                    let mut v = [0f32, 0.0, 0.0, 1.0];
                    for (c, raw) in s.chunks_exact(step).take(channels.min(4)).enumerate() {
                        v[c] = if step == 2 {
                            half_to_f32(u16::from_le_bytes([raw[0], raw[1]]))
                        } else {
                            f32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
                        };
                    }
                    if channels == 1 {
                        v[1] = v[0];
                        v[2] = v[0];
                    }
                    d.copy_from_slice(&[
                        unit_to_u8(v[0]),
                        unit_to_u8(v[1]),
                        unit_to_u8(v[2]),
                        unit_to_u8(v[3]),
                    ]);
                }
            });
        }
        _ => {}
    }
    out
}

fn level_of(bytes: &[u8], info: &DdsInfo, layer: u32, mip: u32) -> Result<Image> {
    let (at, len, width, height) = info
        .locate(layer, mip)
        .ok_or_else(|| TexError::Invalid(format!("the DDS has no level {mip} of layer {layer}")))?;
    let end = at.checked_add(len).ok_or(TexError::Truncated { need: usize::MAX })?;
    let data = bytes.get(at..end).ok_or(TexError::Truncated { need: end.saturating_sub(bytes.len()) })?;
    Ok(Image { width, height, rgba: decode(info.format, data, width as usize, height as usize) })
}

/// Decodes one level of one layer, without touching the rest of the file.
pub fn read_dds_level(bytes: &[u8], layer: u32, mip: u32) -> Result<Image> {
    let info = read_dds_info(bytes)?;
    level_of(bytes, &info, layer, mip)
}

/// Decodes every layer and level. The file must hold all the data its header promises.
pub fn read_dds(bytes: &[u8]) -> Result<Dds> {
    let info = read_dds_info(bytes)?;
    // checked before anything is allocated for a level
    let need = info
        .data_len()
        .and_then(|n| n.checked_add(info.data_at))
        .ok_or(TexError::Truncated { need: usize::MAX })?;
    if bytes.len() < need {
        return Err(TexError::Truncated { need: need - bytes.len() });
    }
    let mut layers = Vec::with_capacity(info.layers as usize);
    for layer in 0..info.layers {
        let mut levels = Vec::new();
        for mip in 0..info.mips {
            if info.locate(layer, mip).is_none() {
                break;
            }
            levels.push(level_of(bytes, &info, layer, mip)?);
        }
        layers.push(levels);
    }
    Ok(Dds { info, layers })
}

/// What the editor writes into a DDS.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum DdsEncoding {
    /// DXT1, every pixel opaque.
    Dxt1,
    /// DXT1 with 1-bit alpha: over 127 is opaque.
    Dxt1Alpha,
    Dxt3,
    Dxt5,
    /// The red channel in blocks (ATI1).
    Bc4,
    /// Red and green in blocks (ATI2).
    Bc5,
    /// 32 bits, bytes in the order R, G, B, A: what the editor's "32 bit (8:8:8:8)" is.
    Rgba8,
    /// 32 bits, bytes in the order B, G, R, A: Direct3D's A8R8G8B8.
    Bgra8,
    /// 24 bits, bytes in the order R, G, B.
    Rgb8,
}

impl DdsEncoding {
    fn encode(self, level: &Image) -> Vec<u8> {
        let (w, h) = (level.width as usize, level.height as usize);
        let px = &level.rgba;
        match self {
            DdsEncoding::Dxt1 => bc::encode_level(texpresso::Format::Bc1, AlphaRule::Opaque, px, w, h),
            DdsEncoding::Dxt1Alpha => bc::encode_level(texpresso::Format::Bc1, AlphaRule::Binary, px, w, h),
            DdsEncoding::Dxt3 => bc::encode_level(texpresso::Format::Bc2, AlphaRule::Keep, px, w, h),
            DdsEncoding::Dxt5 => bc::encode_level(texpresso::Format::Bc3, AlphaRule::Keep, px, w, h),
            DdsEncoding::Bc4 => bc::encode_level(texpresso::Format::Bc4, AlphaRule::Keep, px, w, h),
            DdsEncoding::Bc5 => bc::encode_level(texpresso::Format::Bc5, AlphaRule::Keep, px, w, h),
            DdsEncoding::Rgba8 => px.clone(),
            DdsEncoding::Bgra8 => {
                let mut out = px.clone();
                for p in out.as_chunks_mut::<4>().0 {
                    p.swap(0, 2);
                }
                out
            }
            DdsEncoding::Rgb8 => {
                let mut out = Vec::with_capacity(w * h * 3);
                for p in px.as_chunks::<4>().0 {
                    out.extend_from_slice(&p[..3]);
                }
                out
            }
        }
    }

    /// The pixel format block of the header: flags, four-character code, bits, masks.
    fn pixel_format(self) -> [u32; 7] {
        match self {
            DdsEncoding::Dxt1 | DdsEncoding::Dxt1Alpha => [DDPF_FOURCC, fourcc(b"DXT1"), 0, 0, 0, 0, 0],
            DdsEncoding::Dxt3 => [DDPF_FOURCC, fourcc(b"DXT3"), 0, 0, 0, 0, 0],
            DdsEncoding::Dxt5 => [DDPF_FOURCC, fourcc(b"DXT5"), 0, 0, 0, 0, 0],
            DdsEncoding::Bc4 => [DDPF_FOURCC, fourcc(b"ATI1"), 0, 0, 0, 0, 0],
            DdsEncoding::Bc5 => [DDPF_FOURCC, fourcc(b"ATI2"), 0, 0, 0, 0, 0],
            DdsEncoding::Rgba8 => [DDPF_RGB | DDPF_ALPHAPIXELS, 0, 32, 0xff, 0xff00, 0xff_0000, 0xff00_0000],
            DdsEncoding::Bgra8 => [DDPF_RGB | DDPF_ALPHAPIXELS, 0, 32, 0xff_0000, 0xff00, 0xff, 0xff00_0000],
            DdsEncoding::Rgb8 => [DDPF_RGB, 0, 24, 0xff, 0xff00, 0xff_0000, 0],
        }
    }

    /// Bytes per pixel of the uncompressed encodings.
    fn pixel_bytes(self) -> Option<u32> {
        match self {
            DdsEncoding::Rgba8 | DdsEncoding::Bgra8 => Some(4),
            DdsEncoding::Rgb8 => Some(3),
            _ => None,
        }
    }
}

fn encode_refs(levels: &[&Image], encoding: DdsEncoding) -> Result<Vec<u8>> {
    let base = levels.first().ok_or_else(|| TexError::Invalid("a DDS needs at least one level".into()))?;
    for (i, level) in levels.iter().enumerate() {
        level.check()?;
        let want = (mip_side(base.width, i as u32), mip_side(base.height, i as u32));
        if (level.width, level.height) != want {
            return Err(TexError::Invalid(format!(
                "level {i} is {} x {}, a level under {} x {} is {} x {}",
                level.width, level.height, base.width, base.height, want.0, want.1
            )));
        }
    }
    if levels.len() as u32 > mip_count(base.width, base.height) {
        return Err(TexError::Invalid(format!(
            "{} x {} has no {} levels",
            base.width,
            base.height,
            levels.len()
        )));
    }

    // levels side by side, and each level's blocks side by side inside it
    let data: Vec<Vec<u8>> = levels.par_iter().map(|level| encoding.encode(level)).collect();

    let mut flags = DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PIXELFORMAT;
    let mut pitch = 0u32;
    if levels.len() > 1 {
        flags |= DDSD_MIPMAPCOUNT;
    }
    if let Some(bytes) = encoding.pixel_bytes() {
        flags |= DDSD_LINEARSIZE;
        pitch = bytes * base.width;
    }
    let mut out = Vec::with_capacity(4 + HEADER_SIZE + data.iter().map(Vec::len).sum::<usize>());
    out.extend_from_slice(MAGIC);
    let mut word = |v: u32| out.extend_from_slice(&v.to_le_bytes());
    word(HEADER_SIZE as u32);
    word(flags);
    word(base.height);
    word(base.width);
    word(pitch);
    word(0);
    word(if levels.len() > 1 { levels.len() as u32 } else { 0 });
    for _ in 0..11 {
        word(0);
    }
    word(32);
    for v in encoding.pixel_format() {
        word(v);
    }
    word(DDSCAPS_TEXTURE);
    for _ in 0..4 {
        word(0);
    }
    for level in &data {
        out.extend_from_slice(level);
    }
    Ok(out)
}

/// Compresses the given levels and writes them as one DDS. The first level is the picture;
/// each following one must be half the one before, as [`mip_chain`] makes them.
pub fn encode_dds(levels: &[Image], encoding: DdsEncoding) -> Result<Vec<u8>> {
    let refs: Vec<&Image> = levels.iter().collect();
    encode_refs(&refs, encoding)
}

/// Makes the levels a request asks for and writes the DDS.
pub fn build_dds(image: &Image, request: &DdsRequest) -> Result<Vec<u8>> {
    image.check()?;
    let chain = if request.mips { mip_chain(image, request.filter)? } else { Vec::new() };
    let mut refs: Vec<&Image> = Vec::with_capacity(chain.len() + 1);
    refs.push(image);
    refs.extend(chain.iter());
    encode_refs(&refs, request.encoding)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mip::MipFilter;

    fn gradient(w: u32, h: u32) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[
                    (x * 255 / w.max(2)) as u8,
                    (y * 255 / h.max(2)) as u8,
                    40,
                    255 - (x * 9) as u8,
                ]);
            }
        }
        Image::new(w, h, rgba).unwrap()
    }

    /// A header with the given pixel format and no data.
    fn header(width: u32, height: u32, flags: u32, mips: u32, pf: [u32; 7], caps2: u32) -> Vec<u8> {
        let mut out = MAGIC.to_vec();
        let mut words = vec![124, flags, height, width, 0, 0, mips];
        words.extend([0; 11]);
        words.push(32);
        words.extend(pf);
        words.extend([0x1000, caps2, 0, 0, 0]);
        for w in words {
            out.extend_from_slice(&w.to_le_bytes());
        }
        assert_eq!(out.len(), 128);
        out
    }

    #[test]
    fn the_written_header_is_the_editors() {
        let img = gradient(8, 8);
        let dds =
            build_dds(&img, &DdsRequest { encoding: DdsEncoding::Dxt1, mips: true, filter: MipFilter::Box })
                .unwrap();
        // 8, 4, 2, 1: 4 + 1 + 1 + 1 blocks
        assert_eq!(dds.len(), 128 + 7 * 8);
        assert_eq!(&dds[..8], b"DDS \x7c\0\0\0");
        assert_eq!(u32_at(&dds, 8), 0x2_1007);
        assert_eq!((u32_at(&dds, 12), u32_at(&dds, 16)), (8, 8));
        assert_eq!(u32_at(&dds, 20), 0);
        assert_eq!(u32_at(&dds, 28), 4);
        assert_eq!(u32_at(&dds, 76), 32);
        assert_eq!(u32_at(&dds, 80), 4);
        assert_eq!(&dds[84..88], b"DXT1");
        assert_eq!(u32_at(&dds, 108), 0x1000);
        assert!(dds[32..76].iter().all(|&b| b == 0));

        let info = read_dds_info(&dds).unwrap();
        assert_eq!((info.width, info.height, info.mips, info.layers), (8, 8, 4, 1));
        assert_eq!((info.kind, info.format), (DdsKind::Texture, DdsFormat::Dxt1));
        assert!(!info.format.has_alpha_channel());
    }

    #[test]
    fn uncompressed_files_round_trip_exactly() {
        let img = gradient(5, 3);
        for encoding in [DdsEncoding::Rgba8, DdsEncoding::Bgra8] {
            let dds = encode_dds(std::slice::from_ref(&img), encoding).unwrap();
            assert_eq!(dds.len(), 128 + 5 * 3 * 4);
            // no levels: no count, no flag; the pitch is a row
            assert_eq!(u32_at(&dds, 8), 0x8_1007);
            assert_eq!(u32_at(&dds, 20), 20);
            assert_eq!(u32_at(&dds, 28), 0);
            let back = read_dds(&dds).unwrap();
            assert_eq!(back.layers.len(), 1);
            assert_eq!(back.layers[0], vec![img.clone()]);
            assert!(back.info.format.has_alpha_channel());
        }
        // the editor's 32 bit files keep the bytes as R, G, B, A
        let dds = encode_dds(std::slice::from_ref(&img), DdsEncoding::Rgba8).unwrap();
        assert_eq!(&dds[128..132], &img.rgba[..4]);
        assert_eq!(u32_at(&dds, 92), 0xff);

        let dds = encode_dds(std::slice::from_ref(&img), DdsEncoding::Rgb8).unwrap();
        assert_eq!(dds.len(), 128 + 5 * 3 * 3);
        let back = read_dds_level(&dds, 0, 0).unwrap();
        assert!(
            back.rgba
                .as_chunks::<4>()
                .0
                .iter()
                .zip(img.rgba.as_chunks::<4>().0)
                .all(|(a, b)| a[..3] == b[..3] && a[3] == 255)
        );
    }

    #[test]
    fn compressed_levels_are_found_one_by_one() {
        let img = gradient(16, 8);
        let dds =
            build_dds(&img, &DdsRequest { encoding: DdsEncoding::Dxt5, mips: true, filter: MipFilter::Box })
                .unwrap();
        let info = read_dds_info(&dds).unwrap();
        assert_eq!(info.mips, 5);
        assert_eq!(info.level_size(3), (2, 1));
        let all = read_dds(&dds).unwrap();
        assert_eq!(all.layers[0].len(), 5);
        for mip in 0..5 {
            assert_eq!(read_dds_level(&dds, 0, mip).unwrap(), all.layers[0][mip as usize]);
        }
        assert!(read_dds_level(&dds, 0, 5).is_err());
        assert!(read_dds_level(&dds, 1, 0).is_err());
        // the top level stays close to the picture
        let top = &all.layers[0][0];
        let worst = top.rgba.iter().zip(&img.rgba).map(|(a, b)| a.abs_diff(*b)).max().unwrap();
        assert!(worst < 64, "{worst}");
    }

    #[test]
    fn sixteen_bit_and_single_channel_layouts_decode() {
        // R5G6B5: pure red, then pure green
        let mut dds = header(2, 1, 0x1007, 0, [DDPF_RGB, 0, 16, 0xf800, 0x07e0, 0x001f, 0], 0);
        dds.extend_from_slice(&[0x00, 0xf8, 0xe0, 0x07]);
        assert_eq!(read_dds_level(&dds, 0, 0).unwrap().rgba, [255, 0, 0, 255, 0, 255, 0, 255]);

        // A1R5G5B5: opaque blue, transparent white
        let mut dds =
            header(2, 1, 0x1007, 0, [DDPF_RGB | DDPF_ALPHAPIXELS, 0, 16, 0x7c00, 0x03e0, 0x001f, 0x8000], 0);
        dds.extend_from_slice(&[0x1f, 0x80, 0xff, 0x7f]);
        assert_eq!(read_dds_level(&dds, 0, 0).unwrap().rgba, [0, 0, 255, 255, 255, 255, 255, 0]);

        // X8R8G8B8: the fourth byte is not alpha
        let mut dds = header(1, 1, 0x1007, 0, [DDPF_RGB, 0, 32, 0xff_0000, 0xff00, 0xff, 0], 0);
        dds.extend_from_slice(&[10, 20, 30, 0]);
        assert_eq!(read_dds_level(&dds, 0, 0).unwrap().rgba, [30, 20, 10, 255]);

        // L8 and A8
        let mut dds = header(1, 1, 0x1007, 0, [DDPF_LUMINANCE, 0, 8, 0xff, 0, 0, 0], 0);
        dds.push(77);
        assert_eq!(read_dds_level(&dds, 0, 0).unwrap().rgba, [77, 77, 77, 255]);
        let mut dds = header(1, 1, 0x1007, 0, [DDPF_ALPHA, 0, 8, 0, 0, 0, 0xff], 0);
        dds.push(99);
        let info = read_dds_info(&dds).unwrap();
        assert!(matches!(info.format, DdsFormat::Alpha(_)));
        assert_eq!(read_dds_level(&dds, 0, 0).unwrap().rgba, [0, 0, 0, 99]);
    }

    #[test]
    fn cubes_and_volumes_are_recognised() {
        let mut dds = header(4, 4, 0x1007, 0, [DDPF_FOURCC, fourcc(b"DXT1"), 0, 0, 0, 0, 0], 0xfe00);
        dds.extend_from_slice(&[0u8; 6 * 8]);
        let cube = read_dds(&dds).unwrap();
        assert_eq!((cube.info.kind, cube.info.layers), (DdsKind::Cube, 6));
        assert_eq!(cube.layers.len(), 6);

        // a 2 x 2 x 2 volume with two levels: two slices, then one
        let mut dds = header(
            2,
            2,
            0x1007 | DDSD_DEPTH | DDSD_MIPMAPCOUNT,
            2,
            [DDPF_RGB, 0, 32, 0xff_0000, 0xff00, 0xff, 0],
            0x20_0000,
        );
        dds[24..28].copy_from_slice(&2u32.to_le_bytes());
        dds.extend_from_slice(&[1u8; 2 * 16 + 4]);
        let volume = read_dds(&dds).unwrap();
        assert_eq!((volume.info.kind, volume.info.depth, volume.info.layers), (DdsKind::Volume, 2, 2));
        assert_eq!(volume.layers[0].len(), 2);
        assert_eq!(volume.layers[1].len(), 1);
    }

    #[test]
    fn hostile_headers_are_errors() {
        assert_eq!(read_dds_info(b"PNG "), Err(TexError::NotA("DDS")));
        assert!(matches!(read_dds_info(b"DDS \x7c\0\0\0"), Err(TexError::Truncated { .. })));

        let pf = [DDPF_FOURCC, fourcc(b"DXT1"), 0, 0, 0, 0, 0];
        // a size past the limit
        assert!(matches!(
            read_dds_info(&header(1 << 20, 4, 0x1007, 0, pf, 0)),
            Err(TexError::BadSize { .. })
        ));
        assert!(matches!(read_dds_info(&header(0, 4, 0x1007, 0, pf, 0)), Err(TexError::BadSize { .. })));
        // the data the header promises is not there: nothing is allocated for it
        let big = header(16384, 16384, 0x1007, 0, pf, 0);
        assert!(matches!(read_dds(&big), Err(TexError::Truncated { .. })));
        assert!(matches!(read_dds_level(&big, 0, 0), Err(TexError::Truncated { .. })));
        // more levels than the size has are clamped
        let many = header(4, 4, 0x1007 | DDSD_MIPMAPCOUNT, 400, pf, 0);
        assert_eq!(read_dds_info(&many).unwrap().mips, 3);
        // formats the reader does not know
        let odd = header(4, 4, 0x1007, 0, [DDPF_FOURCC, fourcc(b"XYZ1"), 0, 0, 0, 0, 0], 0);
        assert!(matches!(read_dds_info(&odd), Err(TexError::Unsupported(_))));
        let bits = header(4, 4, 0x1007, 0, [DDPF_RGB, 0, 12, 0xf00, 0xf0, 0xf, 0], 0);
        assert!(matches!(read_dds_info(&bits), Err(TexError::Unsupported(_))));
    }

    #[test]
    fn levels_must_form_a_chain() {
        let a = gradient(8, 8);
        let b = gradient(3, 3);
        assert!(encode_dds(&[a.clone(), b], DdsEncoding::Dxt1).is_err());
        assert!(encode_dds(&[], DdsEncoding::Dxt1).is_err());
        let broken = Image { width: 8, height: 8, rgba: vec![0; 5] };
        assert!(encode_dds(&[broken], DdsEncoding::Rgba8).is_err());
        assert_eq!(half_to_f32(0x3c00), 1.0);
        assert_eq!(half_to_f32(0xc000), -2.0);
        assert_eq!(half_to_f32(0), 0.0);
    }
}
