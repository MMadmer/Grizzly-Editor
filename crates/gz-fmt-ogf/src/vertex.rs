//! Vertex containers: a D3D FVF or one of the skinned formats, decoded into one neutral vertex.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};

use crate::{Result, invalid};

const FVF_POSITION_MASK: u32 = 0x400E;
const FVF_XYZ: u32 = 0x002;
const FVF_XYZRHW: u32 = 0x004;
const FVF_XYZW: u32 = 0x4002;
const FVF_NORMAL: u32 = 0x010;
const FVF_PSIZE: u32 = 0x020;
const FVF_DIFFUSE: u32 = 0x040;
const FVF_SPECULAR: u32 = 0x080;
const FVF_TEXCOUNT_MASK: u32 = 0xf00;
const FVF_TEXCOUNT_SHIFT: u32 = 8;
const FVF_LASTBETA_UBYTE4: u32 = 0x1000;
const FVF_LASTBETA_D3DCOLOR: u32 = 0x8000;

/// `OGF_SkeletonVertType`, and the plain 1 to 4 the exporter writes.
const SKIN_1L: u32 = 0x1207_1980;
const SKIN_2L: u32 = 2 * 0x1207_1980;
const SKIN_3L: u32 = 4 * 0x1207_1980;
const SKIN_4L: u32 = 5 * 0x1207_1980;

/// One vertex, whatever format it came in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub tangent: Vec3,
    pub binormal: Vec3,
    pub uv: Vec2,
    /// The second texture set (a lightmap).
    pub uv2: Vec2,
    /// `0xAARRGGBB`: the diffuse colour of an FVF that has one.
    pub color: u32,
    pub bones: [u16; 4],
    /// The weight of each bone; they sum to one for a skinned vertex.
    pub weights: [f32; 4],
}

/// How the vertices are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum VertexFormat {
    /// A D3D flexible vertex format.
    Fvf(u32),
    /// A skinned vertex with 1 to 4 links; `type_word` is what the file says.
    Skinned { links: u8, type_word: u32 },
}

/// `OGF_VERTICES`.
#[derive(Clone, Debug, PartialEq)]
pub struct VertexBuffer {
    pub format: VertexFormat,
    pub vertices: Arc<[Vertex]>,
}

/// The size of one FVF vertex, and the floats of each texture set.
fn fvf_layout(fvf: u32) -> Result<(usize, Vec<usize>, usize, bool)> {
    let pos = fvf & FVF_POSITION_MASK;
    // floats of position, then betas
    let (pos_floats, betas) = match pos {
        FVF_XYZ => (3, 0),
        FVF_XYZRHW | FVF_XYZW => (4, 0),
        0x006 => (3, 1),
        0x008 => (3, 2),
        0x00a => (3, 3),
        0x00c => (3, 4),
        0x00e => (3, 5),
        _ => return Err(invalid(format!("vertex format {fvf:#x} has no position"))),
    };
    let last_beta_index = fvf & (FVF_LASTBETA_UBYTE4 | FVF_LASTBETA_D3DCOLOR) != 0;
    let tex_count = ((fvf & FVF_TEXCOUNT_MASK) >> FVF_TEXCOUNT_SHIFT) as usize;
    let tex = (0..tex_count)
        .map(|i| match (fvf >> (16 + i * 2)) & 3 {
            0 => 2,
            1 => 3,
            2 => 4,
            _ => 1,
        })
        .collect::<Vec<_>>();
    let mut size = (pos_floats + betas) * 4;
    if fvf & FVF_NORMAL != 0 {
        size += 12;
    }
    if fvf & FVF_PSIZE != 0 {
        size += 4;
    }
    if fvf & FVF_DIFFUSE != 0 {
        size += 4;
    }
    if fvf & FVF_SPECULAR != 0 {
        size += 4;
    }
    size += tex.iter().sum::<usize>() * 4;
    Ok((size, tex, betas, last_beta_index))
}

fn read_fvf(r: &mut Reader<'_>, fvf: u32) -> Result<Vertex> {
    let (_, tex, betas, last_beta_index) = fvf_layout(fvf)?;
    let mut v = Vertex::default();
    v.position = r.r_vec3()?;
    if matches!(fvf & FVF_POSITION_MASK, FVF_XYZRHW | FVF_XYZW) {
        r.r_f32()?;
    }
    let mut weights = Vec::new();
    for i in 0..betas {
        if i + 1 == betas && last_beta_index {
            let b = r.r_u32()?;
            v.bones = [(b & 0xff) as u16, ((b >> 8) & 0xff) as u16, ((b >> 16) & 0xff) as u16, (b >> 24) as u16];
        } else {
            weights.push(r.r_f32()?);
        }
    }
    for (slot, w) in v.weights.iter_mut().zip(&weights) {
        *slot = *w;
    }
    if fvf & FVF_NORMAL != 0 {
        v.normal = r.r_vec3()?;
    }
    if fvf & FVF_PSIZE != 0 {
        r.r_f32()?;
    }
    if fvf & FVF_DIFFUSE != 0 {
        v.color = r.r_u32()?;
    }
    if fvf & FVF_SPECULAR != 0 {
        r.r_u32()?;
    }
    for (i, n) in tex.iter().enumerate() {
        let mut f = [0.0f32; 4];
        for slot in f.iter_mut().take(*n) {
            *slot = r.r_f32()?;
        }
        match i {
            0 => v.uv = Vec2::new(f[0], f[1]),
            1 => v.uv2 = Vec2::new(f[0], f[1]),
            _ => {}
        }
    }
    Ok(v)
}

fn read_skinned(r: &mut Reader<'_>, links: u8) -> Result<Vertex> {
    let mut v = Vertex::default();
    match links {
        1 => {
            v.position = r.r_vec3()?;
            v.normal = r.r_vec3()?;
            v.tangent = r.r_vec3()?;
            v.binormal = r.r_vec3()?;
            v.uv = r.r_vec2()?;
            v.bones[0] = r.r_u32()? as u16;
            v.weights[0] = 1.0;
        }
        2 => {
            v.bones[0] = r.r_u16()?;
            v.bones[1] = r.r_u16()?;
            v.position = r.r_vec3()?;
            v.normal = r.r_vec3()?;
            v.tangent = r.r_vec3()?;
            v.binormal = r.r_vec3()?;
            // the weight of the second bone: the engine lerps from the first to it
            let w = r.r_f32()?;
            v.weights[0] = 1.0 - w;
            v.weights[1] = w;
            v.uv = r.r_vec2()?;
        }
        n => {
            let n = usize::from(n);
            for b in v.bones.iter_mut().take(n) {
                *b = r.r_u16()?;
            }
            v.position = r.r_vec3()?;
            v.normal = r.r_vec3()?;
            v.tangent = r.r_vec3()?;
            v.binormal = r.r_vec3()?;
            let mut sum = 0.0;
            for w in v.weights.iter_mut().take(n - 1) {
                *w = r.r_f32()?;
                sum += *w;
            }
            v.weights[n - 1] = 1.0 - sum;
            v.uv = r.r_vec2()?;
        }
    }
    Ok(v)
}

impl VertexFormat {
    /// The format a type word names.
    #[must_use]
    pub fn from_word(word: u32, skinned: bool) -> Self {
        let links = match word {
            SKIN_1L => Some(1),
            SKIN_2L => Some(2),
            SKIN_3L => Some(3),
            SKIN_4L => Some(4),
            1..=4 if skinned => Some(word as u8),
            _ => None,
        };
        match links {
            Some(links) => Self::Skinned { links, type_word: word },
            None => Self::Fvf(word),
        }
    }

    /// The bytes of one vertex.
    pub fn stride(self) -> Result<usize> {
        Ok(match self {
            Self::Fvf(fvf) => fvf_layout(fvf)?.0,
            Self::Skinned { links: 1, .. } => 60,
            Self::Skinned { links: 2, .. } => 64,
            Self::Skinned { links: 3, .. } => 70,
            Self::Skinned { .. } => 76,
        })
    }

    #[must_use]
    pub fn word(self) -> u32 {
        match self {
            Self::Fvf(f) => f,
            Self::Skinned { type_word, .. } => type_word,
        }
    }
}

impl VertexBuffer {
    /// `OGF_VERTICES`: the type word, the count, the vertices. A skinned model's geometry
    /// (`skinned`) reads the type words 1 to 4 as link counts.
    pub(crate) fn read(body: &[u8], skinned: bool) -> Result<Self> {
        let mut r = Reader::new(body);
        let format = VertexFormat::from_word(r.r_u32()?, skinned);
        let count = r.r_u32()? as usize;
        let stride = format.stride()?;
        if count.saturating_mul(stride) > r.elapsed() {
            return Err(invalid(format!("the model states {count} vertices of {stride} bytes and holds {} bytes", r.elapsed())));
        }
        let mut vertices = Vec::with_capacity(count);
        for _ in 0..count {
            let mut one = Reader::new(r.take(stride)?);
            vertices.push(match format {
                VertexFormat::Fvf(fvf) => read_fvf(&mut one, fvf)?,
                VertexFormat::Skinned { links, .. } => read_skinned(&mut one, links)?,
            });
        }
        Ok(Self { format, vertices: Arc::from(vertices) })
    }

    /// Writes the static exporter's position, normal and UV (`XYZ | NORMAL | TEX1`).
    pub(crate) fn write_static(vertices: &[(Vec3, Vec3, Vec2)], w: &mut Writer) {
        w.w_u32(FVF_XYZ | FVF_NORMAL | (1 << FVF_TEXCOUNT_SHIFT));
        w.w_u32(vertices.len() as u32);
        for (p, n, uv) in vertices {
            w.w_vec3(*p);
            w.w_vec3(*n);
            w.w_f32(uv.x);
            w.w_f32(uv.y);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_static_buffer_reads_back() {
        let mut w = Writer::new();
        VertexBuffer::write_static(&[(Vec3::ONE, Vec3::Y, Vec2::new(0.5, 0.25))], &mut w);
        let b = w.into_bytes();
        let vb = VertexBuffer::read(&b, false).unwrap();
        assert_eq!(vb.format, VertexFormat::Fvf(0x112));
        assert_eq!(vb.format.stride().unwrap(), 32);
        assert_eq!((vb.vertices[0].position, vb.vertices[0].normal, vb.vertices[0].uv), (Vec3::ONE, Vec3::Y, Vec2::new(0.5, 0.25)));
    }

    #[test]
    fn skinned_formats_have_the_engine_sizes() {
        assert_eq!(VertexFormat::from_word(SKIN_3L, false).stride().unwrap(), 70);
        assert_eq!(VertexFormat::from_word(2, true), VertexFormat::Skinned { links: 2, type_word: 2 });
        assert_eq!(VertexFormat::from_word(2, false), VertexFormat::Fvf(2));
        // a two-link vertex: the weight is the second bone's
        let mut w = Writer::new();
        w.w_u32(2);
        w.w_u32(1);
        w.w_u16(3);
        w.w_u16(5);
        for _ in 0..12 {
            w.w_f32(0.0);
        }
        w.w_f32(0.25);
        w.w_f32(0.0);
        w.w_f32(0.0);
        let v = VertexBuffer::read(&w.into_bytes(), true).unwrap().vertices[0];
        assert_eq!((v.bones[0], v.bones[1], v.weights[0], v.weights[1]), (3, 5, 0.75, 0.25));
    }

    #[test]
    fn a_count_the_chunk_cannot_hold_is_an_error() {
        let mut w = Writer::new();
        w.w_u32(0x112);
        w.w_u32(1_000_000);
        assert!(VertexBuffer::read(&w.into_bytes(), false).is_err());
        assert!(fvf_layout(0x110).is_err());
    }
}
