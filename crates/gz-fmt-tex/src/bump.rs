//! Bump maps: the two game textures made of one height-and-gloss picture.
//!
//! The source keeps the height in its colour and the gloss in its alpha. The first texture
//! (`name.dds`, DXT5) holds gloss in red and the normal in alpha, blue, green - x, y, z. DXT5
//! loses precision there, so the second (`name#.dds`, DXT5) holds what the first got wrong, as
//! the error around 128 in the same channel layout, and the height for parallax in alpha.

use rayon::prelude::*;

use crate::bc::nv_decode_dxt5;
use crate::dds::{DdsEncoding, build_dds};
use crate::mip::MipFilter;
use crate::params::DdsRequest;
use crate::{Image, Result, TexError};

/// The game textures of a bump map.
#[derive(Clone, Debug, PartialEq)]
pub struct BumpTextures {
    /// `name.dds`.
    pub bump: Vec<u8>,
    /// `name#.dds`.
    pub error: Vec<u8>,
    /// The mean gloss, 0..1.
    pub gloss_power: f32,
    /// The gloss mask is under 10 percent: the editor keeps the textures and reports
    /// "Invalid gloss mask".
    pub low_gloss: bool,
}

fn fpack(v: f32) -> i32 {
    ((((v + 1.0) * 0.5) * 255.0 + 0.5).floor() as i32).clamp(0, 255)
}

fn vunpack(x: i32, y: i32, z: i32) -> [f32; 3] {
    [(x as f32 / 255.0 - 0.5) * 2.0, (y as f32 / 255.0 - 0.5) * 2.0, (z as f32 / 255.0 - 0.5) * 2.0]
}

/// Packs a direction into three bytes: the nearest byte triple is tried against its
/// neighbours two steps around, and the one that points closest to the direction wins.
fn vpack(src: [f32; 3]) -> [u8; 3] {
    let inv = (1.0 / (src[0] * src[0] + src[1] * src[1] + src[2] * src[2])).sqrt();
    let src = [src[0] * inv, src[1] * inv, src[2] * inv];
    let (bx, by, bz) = (fpack(src[0]), fpack(src[1]), fpack(src[2]));
    let mut best = f32::MAX;
    let (mut r, mut g, mut b) = (bx, by, bz);
    let d = 2;
    for x in (bx - d).max(0)..=(bx + d).min(255) {
        for y in (by - d).max(0)..=(by + d).min(255) {
            for z in (bz - d).max(0)..=(bz + d).min(255) {
                let v = vunpack(x, y, z);
                let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
                if (m - 1.0).abs() > 0.03 {
                    continue;
                }
                let v = [v[0] / m, v[1] / m, v[2] / m];
                let e = ((src[0] * v[0] + src[1] * v[1] + src[2] * v[2]) - 1.0).abs();
                if e < best {
                    best = e;
                    (r, g, b) = (x, y, z);
                }
            }
        }
    }
    [r as u8, g as u8, b as u8]
}

/// The normal map of a height field: the height is each pixel's mean colour, the slope comes
/// from the four neighbours, and the picture wraps. The result is x, y, z in red, green, blue.
fn normals_from_height(height_gloss: &[u8], w: usize, h: usize, scale: f32) -> Vec<u8> {
    // AverageRGBToAlpha
    let heights: Vec<u8> = height_gloss
        .as_chunks::<4>()
        .0
        .iter()
        .map(|p| ((f32::from(p[0]) + f32::from(p[1]) + f32::from(p[2])) / 3.0) as u8)
        .collect();
    const BYTE_TO_FLOAT: f32 = 1.0 / 255.0;
    let at = |x: usize, y: usize| f32::from(heights.get(y * w + x).copied().unwrap_or(0)) * BYTE_TO_FLOAT;

    let mut out = vec![0u8; w * h * 4];
    out.par_chunks_mut(w * 4).enumerate().for_each(|(j, row)| {
        let up = (j + 1) % h;
        // the bordered copy the convolution reads takes its row below the first from
        // one row short of the last, so the wrap downwards is off by one; kept
        let down = if j > 0 { j - 1 } else { h.saturating_sub(2) };
        for (i, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let left = if i > 0 { i - 1 } else { w - 1 };
            let right = (i + 1) % w;
            let du = (0.0 + at(left, j) * -0.5) + at(right, j) * 0.5;
            let dv = (0.0 + at(i, up) * 0.5) + at(i, down) * -0.5;
            let du = du * scale;
            let dv = dv * scale;
            let mag = (du * du + dv * dv + 1.0).sqrt();
            let n = vpack([-du / mag, -dv / mag, 1.0 / mag]);
            px.copy_from_slice(&[n[0], n[1], n[2], 0]);
        }
    });
    out
}

const DXT5_WITH_MIPS: DdsRequest =
    DdsRequest { encoding: DdsEncoding::Dxt5, mips: true, filter: MipFilter::Box };

/// Makes the two textures of a bump map.
///
/// `height_gloss` is the source picture. `normal_map`, when given, replaces the normals derived
/// from the height; it must be of the same size. `virtual_height` is the description's
/// `bump_virtual_height`, in metres.
///
/// the editor measures the error against a decompression of the first texture that
/// reads no blocks at all - the file it has just written lacks the linear-size flag its
/// decompressor relies on - so its `#` texture is made from whatever the memory held. Here the
/// first level is decompressed as that code means to.
pub fn make_bump_textures(
    height_gloss: &Image,
    normal_map: Option<&Image>,
    virtual_height: f32,
) -> Result<BumpTextures> {
    height_gloss.check()?;
    let (w, h) = (height_gloss.width as usize, height_gloss.height as usize);
    let source = &height_gloss.rgba;

    // stage 0: the normals, x y z in red green blue
    let normals = match normal_map {
        Some(nm) => {
            nm.check()?;
            if (nm.width, nm.height) != (height_gloss.width, height_gloss.height) {
                return Err(TexError::Invalid(format!(
                    "the special normal map is {}x{}; it should be {}x{}",
                    nm.width, nm.height, height_gloss.width, height_gloss.height
                )));
            }
            nm.rgba.clone()
        }
        None => normals_from_height(source, w, h, virtual_height * 200.0),
    };

    // gloss into red, the normal turned around into alpha, blue, green
    let mut gloss_power = 0f32;
    let mut packed = vec![0u8; w * h * 4];
    for ((d, n), s) in packed
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(normals.as_chunks::<4>().0)
        .zip(source.as_chunks::<4>().0)
    {
        // summed in pixel order, in single precision, as the editor does
        gloss_power += f32::from(s[3]) / 255.0;
        d.copy_from_slice(&[s[3].saturating_add(1), n[2], n[1], n[0]]);
    }
    gloss_power /= (w * h) as f32;
    let packed = Image { width: height_gloss.width, height: height_gloss.height, rgba: packed };
    let bump = build_dds(&packed, &DXT5_WITH_MIPS)?;

    // stage 1: what the compression lost
    let level_at = 128;
    let unpacked = nv_decode_dxt5(bump.get(level_at..).unwrap_or(&[]), w, h);

    // the height, rescaled by the virtual height towards 1 and centred on 127
    let mut h_scale = (virtual_height / 0.05).powf(0.75);
    if h_scale > 1.0 {
        h_scale = h_scale.sqrt();
    }
    let heights: Vec<u8> = source
        .par_chunks_exact(4)
        .map(|p| (f32::from(p[0]) * h_scale + 0.000_000_1).floor() as i32 as u8)
        .collect();
    let (mut sum, mut lo, mut hi) = (0u32, 255u32, 0u32);
    for &v in &heights {
        let v = u32::from(v);
        sum = sum.wrapping_add(v);
        lo = lo.min(v);
        hi = hi.max(v);
    }
    let median = (9 * (sum / (w * h) as u32) + (hi.wrapping_sub(lo) / 2).wrapping_add(lo)) / 10;
    let correction = 127 - median as i32;

    let err = |orig: u8, ucomp: u8| (128 + 2 * (i32::from(orig) - i32::from(ucomp)) / 3) as u8;
    let mut error = vec![0u8; w * h * 4];
    error
        .par_chunks_exact_mut(4)
        .zip(packed.rgba.par_chunks_exact(4))
        .zip(unpacked.par_chunks_exact(4))
        .zip(heights.par_iter())
        .for_each(|(((d, o), u), &height)| {
            let parallax = (i32::from(height) + correction).clamp(0, 255) as u8;
            // the difference of each channel, turned around like the first texture: the error of
            // its alpha into red, of its blue into green, of its green into blue
            d.copy_from_slice(&[err(o[3], u[3]), err(o[2], u[2]), err(o[1], u[1]), parallax]);
        });
    let error = Image { width: height_gloss.width, height: height_gloss.height, rgba: error };
    let error = build_dds(&error, &DXT5_WITH_MIPS)?;

    Ok(BumpTextures { bump, error, gloss_power, low_gloss: gloss_power < 0.1 })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds::{read_dds_info, read_dds_level};

    #[test]
    fn directions_pack_to_bytes_that_point_the_same_way() {
        // zero lies between two byte values; either is as near
        let up = vpack([0.0, 0.0, 1.0]);
        assert!(matches!(up, [127 | 128, 127 | 128, 255]), "{up:?}");
        let n = vpack([0.6, 0.0, 0.8]);
        let v = vunpack(i32::from(n[0]), i32::from(n[1]), i32::from(n[2]));
        let m = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        assert!(((0.6 * v[0] + 0.8 * v[2]) / m - 1.0).abs() < 1e-3);
        assert_eq!(fpack(-1.0), 0);
        assert_eq!(fpack(1.0), 255);
        assert_eq!(fpack(5.0), 255);
    }

    #[test]
    fn a_flat_height_field_has_straight_normals() {
        let src = vec![100u8; 8 * 8 * 4];
        let n = normals_from_height(&src, 8, 8, 10.0);
        assert!(n.as_chunks::<4>().0.iter().all(|p| matches!(p, [127 | 128, 127 | 128, 255, 0])));
        assert!(n.as_chunks::<4>().0.iter().all(|p| p == &n[..4]));
        // a single row and a single column do not read outside the picture
        assert_eq!(normals_from_height(&src[..32], 8, 1, 10.0).len(), 32);
        assert_eq!(normals_from_height(&src[..32], 1, 8, 10.0).len(), 32);
    }

    #[test]
    fn a_slope_leans_the_normal_against_it() {
        // height rises with x
        let mut src = Vec::new();
        for _y in 0..4 {
            for x in 0..4u8 {
                src.extend_from_slice(&[x * 40, x * 40, x * 40, 255]);
            }
        }
        let n = normals_from_height(&src, 4, 4, 10.0);
        // an inner pixel: the normal's x points down the slope
        let inner = &n[(4 + 1) * 4..][..4];
        assert!(inner[0] < 120);
        assert!(matches!(inner[1], 127 | 128));
    }

    #[test]
    fn both_textures_are_dxt5_with_every_level() {
        let mut rgba = Vec::new();
        for y in 0..8u8 {
            for x in 0..8u8 {
                rgba.extend_from_slice(&[x * 30, x * 30, x * 30, 200 + y]);
            }
        }
        let src = Image::new(8, 8, rgba).unwrap();
        let out = make_bump_textures(&src, None, 0.05).unwrap();
        for dds in [&out.bump, &out.error] {
            let info = read_dds_info(dds).unwrap();
            assert_eq!((info.width, info.height, info.mips), (8, 8, 4));
            assert_eq!(info.format, crate::dds::DdsFormat::Dxt5);
        }
        assert!(!out.low_gloss);
        assert!(out.gloss_power > 0.78 && out.gloss_power < 0.82);
        // the gloss travels in red, one above the source's alpha
        let bump = read_dds_level(&out.bump, 0, 0).unwrap();
        assert!(bump.rgba[0].abs_diff(201) < 12);
        // the error texture stays around 128 and carries the height in alpha
        let error = read_dds_level(&out.error, 0, 0).unwrap();
        assert!(error.rgba.as_chunks::<4>().0.iter().all(|p| p[0].abs_diff(128) < 90));
    }

    #[test]
    fn a_dull_source_is_reported_and_a_wrong_normal_map_refused() {
        let src = Image::filled(4, 4, [50, 50, 50, 10]).unwrap();
        let out = make_bump_textures(&src, None, 0.05).unwrap();
        assert!(out.low_gloss);

        let nm = Image::filled(8, 4, [127, 127, 255, 0]).unwrap();
        assert!(make_bump_textures(&src, Some(&nm), 0.05).is_err());
        let nm = Image::filled(4, 4, [127, 127, 255, 0]).unwrap();
        assert!(make_bump_textures(&src, Some(&nm), 0.05).is_ok());
    }
}
