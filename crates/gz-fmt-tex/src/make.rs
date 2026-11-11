//! What the editor makes of pictures: game textures, a module's thumbnail, a screen picture,
//! the thumbnail's own picture, and the image manager's checks.

use rayon::prelude::*;

use crate::bump::make_bump_textures;
use crate::dds::{DdsEncoding, build_dds};
use crate::mip::MipFilter;
use crate::params::{DdsRequest, TextureParams, TextureType};
use crate::resample::{ImfFilter, imf_resample};
use crate::thm::THUMB_SIDE;
use crate::{Image, Result, TexError, checked_len};

/// The game files of one texture.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GameTexture {
    /// `name.dds`.
    pub dds: Vec<u8>,
    /// `name#.dds`, for a bump map.
    pub bump_error_dds: Option<Vec<u8>>,
    /// A bump map whose gloss mask is under 10 percent: the files are kept and the editor
    /// reports "Invalid gloss mask".
    pub low_gloss: bool,
}

/// Makes the game texture of a source picture (`CImageManager::MakeGameTexture`).
///
/// The picture's own size is used; the size in the description is not consulted. `normal_map`
/// is the description's special normal map, for a bump map that names one.
pub fn make_game_texture(
    source: &Image,
    params: &TextureParams,
    normal_map: Option<&Image>,
) -> Result<GameTexture> {
    source.check()?;
    if params.ty == TextureType::BUMP_MAP {
        let bump = make_bump_textures(source, normal_map, params.bump_virtual_height)?;
        return Ok(GameTexture {
            dds: bump.bump,
            bump_error_dds: Some(bump.error),
            low_gloss: bump.low_gloss,
        });
    }
    Ok(GameTexture {
        dds: build_dds(source, &params.dds_request()?)?,
        bump_error_dds: None,
        low_gloss: false,
    })
}

/// Whether a source picture has a size the game takes: powers of two, or a cube strip six
/// times as wide as it is high (`IsValidSize`).
#[must_use]
pub fn is_valid_size(width: u32, height: u32) -> bool {
    let pow2 = |v: u32| v & v.wrapping_sub(1) == 0;
    if !pow2(height) {
        return false;
    }
    u64::from(height) * 6 == u64::from(width) || pow2(width)
}

/// The 128 x 128 picture a thumbnail stores: the source through the box filter
/// (`MakeThumbnailImage`). RGBA, rows top-down.
pub fn thumbnail_pixels(source: &Image) -> Result<Vec<u8>> {
    Ok(imf_resample(source, THUMB_SIDE, THUMB_SIDE, ImfFilter::Box)?.rgba)
}

const THUMBNAIL_SIDE: u32 = 256;
const THUMBNAIL_LIMIT: usize = 256 << 10;

fn opaque_or_alpha(alpha: bool) -> DdsRequest {
    DdsRequest {
        encoding: if alpha { DdsEncoding::Dxt5 } else { DdsEncoding::Dxt1 },
        mips: true,
        filter: MipFilter::Box,
    }
}

/// A module's `thumbnail.dds` (`EditorCatalog::MakeThumbnail`): the middle square of the
/// picture averaged down, or spread up, to 256 x 256; DXT1, or DXT5 when a pixel comes out
/// with alpha under 250; every level. The picture must be at least 16 pixels on each side.
pub fn make_thumbnail_dds(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let need = checked_len(width, height)?;
    if rgba.len() != need {
        return Err(TexError::BadPixels { width, height, need, got: rgba.len() });
    }
    if width < 16 || height < 16 {
        return Err(TexError::Invalid(format!(
            "a thumbnail needs a picture of at least 16 x 16 pixels, this one is {width} x {height}"
        )));
    }
    let (w, h) = (width as i64, height as i64);
    let side = w.min(h);
    let (left, top) = ((w - side) / 2, (h - side) / 2);
    let target = THUMBNAIL_SIDE as usize;
    let scale = side as f32 / target as f32;

    let mut data = vec![0u8; target * target * 4];
    let alpha = data
        .par_chunks_mut(target * 4)
        .enumerate()
        .map(|(y, row)| {
            let fy0 = top as f32 + y as f32 * scale;
            let y0 = fy0 as i64;
            let y1 = (y0 + 1).max((fy0 + scale) as i64).min(h);
            let mut alpha = false;
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let fx0 = left as f32 + x as f32 * scale;
                let x0 = fx0 as i64;
                let x1 = (x0 + 1).max((fx0 + scale) as i64).min(w);
                let mut sum = [0u32; 4];
                let mut count = 0u32;
                for sy in y0..y1 {
                    let line =
                        rgba.get((sy * w + x0) as usize * 4..(sy * w + x1) as usize * 4).unwrap_or(&[]);
                    for p in line.as_chunks::<4>().0 {
                        for c in 0..4 {
                            sum[c] += u32::from(p[c]);
                        }
                        count += 1;
                    }
                }
                let count = count.max(1);
                let a = sum[3] / count;
                alpha |= a < 250;
                px.copy_from_slice(&[
                    (sum[0] / count) as u8,
                    (sum[1] / count) as u8,
                    (sum[2] / count) as u8,
                    a as u8,
                ]);
            }
            alpha
        })
        .reduce(|| false, |a, b| a || b);

    let image = Image { width: THUMBNAIL_SIDE, height: THUMBNAIL_SIDE, rgba: data };
    let dds = build_dds(&image, &opaque_or_alpha(alpha))?;
    if !valid_thumbnail_dds(&dds) {
        return Err(TexError::Invalid(
            "the compressor wrote a file the game would not take as a thumbnail".into(),
        ));
    }
    Ok(dds)
}

/// Whether bytes are a thumbnail the game's Mod Browser takes (`EditorCatalog::ValidThumbnail`):
/// a DDS of 256 x 256, DXT1 or DXT5, one level or all nine, of exactly the size those take, and
/// no more than 256 KiB.
#[must_use]
pub fn valid_thumbnail_dds(bytes: &[u8]) -> bool {
    if bytes.len() < 128 || bytes.len() > THUMBNAIL_LIMIT || !bytes.starts_with(b"DDS ") {
        return false;
    }
    let at = |offset: usize| {
        bytes.get(offset..offset + 4).map_or(0, |b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    };
    let (header, height, width, mips, format_size, format_flags) =
        (at(4), at(12), at(16), at(28), at(76), at(80));
    if header != 124 || width != 256 || height != 256 || format_size != 32 || format_flags & 0x4 == 0 {
        return false;
    }
    let block = match bytes.get(84..88) {
        Some(b"DXT1") => 8usize,
        Some(b"DXT5") => 16,
        _ => return false,
    };
    let levels = if mips == 0 { 1 } else { mips };
    if levels != 1 && levels != 9 {
        return false;
    }
    let mut expected = 128usize;
    let mut side = 256usize;
    for _ in 0..levels {
        let blocks = if side >= 4 { side.div_ceil(4) } else { 1 };
        expected += blocks * blocks * block;
        side = if side > 1 { side / 2 } else { 1 };
    }
    bytes.len() == expected
}

/// A screen layer's picture as a DDS (`EditorCatalog::MakeScreenPicture`): the picture
/// stretched, nearest pixel, to the next size that is a multiple of four; DXT1, or DXT5 when a
/// pixel has alpha under 250.
pub fn make_screen_picture_dds(rgba: &[u8], width: u32, height: u32) -> Result<Vec<u8>> {
    let need = checked_len(width, height)?;
    if rgba.len() != need {
        return Err(TexError::BadPixels { width, height, need, got: rgba.len() });
    }
    let (w, h) = (width as usize, height as usize);
    let (tw, th) = ((w + 3) & !3, (h + 3) & !3);
    checked_len(tw as u32, th as u32)?;
    let mut data = vec![0u8; tw * th * 4];
    let alpha = data
        .par_chunks_mut(tw * 4)
        .enumerate()
        .map(|(y, row)| {
            let sy = y * h / th;
            let mut alpha = false;
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                let at = (sy * w + x * w / tw) * 4;
                if let Some(p) = rgba.get(at..at + 4) {
                    alpha |= p[3] < 250;
                    px.copy_from_slice(p);
                }
            }
            alpha
        })
        .reduce(|| false, |a, b| a || b);
    // oracle: the description asks for no mip levels here, and the compressor makes them anyway
    build_dds(&Image { width: tw as u32, height: th as u32, rgba: data }, &opaque_or_alpha(alpha))
}

/// How much of a texture survives being halved and brought back, as the image manager measures
/// it (`CheckCompliance`): the picture goes down through Lanczos and up through the cubic, and
/// the loss is the distance in colour, as percent. The editor packs the pair as
/// `average * 1000 + worst`. A picture one pixel wide or high loses nothing.
pub fn check_compliance(source: &Image, has_alpha: bool) -> Result<(u32, u32)> {
    source.check()?;
    if source.width == 1 || source.height == 1 {
        return Ok((0, 0));
    }
    let small = imf_resample(source, source.width / 2, source.height / 2, ImfFilter::Lanczos3)?;
    let restored = imf_resample(&small, source.width, source.height, ImfFilter::Hermite)?;

    const F: f32 = 1.0 / 255.0;
    let mut difference = 0f32;
    let mut maximal = 0f32;
    // summed in pixel order, in single precision, as the editor does
    for (a, b) in source.rgba.as_chunks::<4>().0.iter().zip(restored.rgba.as_chunks::<4>().0) {
        let d = |c: usize| {
            let v = F * f32::from(a[c]) - F * f32::from(b[c]);
            v * v
        };
        let e = if has_alpha {
            (d(0) + d(1) + d(2) + d(3)).sqrt() * (F * f32::from(a[3]))
        } else {
            (d(0) + d(1) + d(2)).sqrt()
        };
        difference += e;
        maximal = maximal.max(e);
    }
    let norm = if has_alpha { 2.0 } else { 3f32.sqrt() };
    let percent = |v: f32| (v / norm * 100.0).clamp(0.0, 100.0).floor() as u32;
    difference /= (source.rgba.len() / 4) as f32;
    Ok((percent(difference), percent(maximal)))
}

/// One pass of the border fill: every fully transparent pixel next to a pixel more opaque than
/// `reference` takes the mean colour of those neighbours and alpha `reference`.
fn border_pass(pixels: &[u8], w: usize, h: usize, reference: u8) -> Option<Vec<u8>> {
    let mut result = pixels.to_vec();
    let changed = result
        .par_chunks_mut(w * 4)
        .enumerate()
        .map(|(y, row)| {
            let mut changed = false;
            for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                if px[3] != 0 {
                    continue;
                }
                let (mut count, mut sum) = (0u32, [0u32; 3]);
                for (dx, dy) in [(-1i64, -1i64), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                    if nx < 0 || ny < 0 || nx >= w as i64 || ny >= h as i64 {
                        continue;
                    }
                    let at = (ny as usize * w + nx as usize) * 4;
                    if let Some(p) = pixels.get(at..at + 4).filter(|p| p[3] > reference) {
                        for c in 0..3 {
                            sum[c] += u32::from(p[c]);
                        }
                        count += 1;
                    }
                }
                if let Some(n) = std::num::NonZeroU32::new(count) {
                    px.copy_from_slice(&[
                        (sum[0] / n) as u8,
                        (sum[1] / n) as u8,
                        (sum[2] / n) as u8,
                        reference,
                    ]);
                    changed = true;
                }
            }
            changed
        })
        .reduce(|| false, |a, b| a || b);
    changed.then_some(result)
}

/// Spreads the colour of the opaque parts of a picture into its transparent parts, ring by
/// ring, and leaves the alpha as it was (`CImageManager::ApplyBorders`). Alpha over 200 counts
/// as opaque.
pub fn apply_borders(source: &Image) -> Result<Image> {
    source.check()?;
    let (w, h) = (source.width as usize, source.height as usize);
    let mut pixels = source.rgba.clone();
    for p in pixels.as_chunks_mut::<4>().0 {
        p[3] = if p[3] > 200 { 0xff } else { 0 };
    }
    for reference in (1..=254u8).rev() {
        // a pass that fills nothing leaves nothing for the passes after it
        match border_pass(&pixels, w, h, reference) {
            Some(next) => pixels = next,
            None => break,
        }
    }
    for (p, s) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(source.rgba.as_chunks::<4>().0) {
        p[3] = s[3];
    }
    Ok(Image { width: source.width, height: source.height, rgba: pixels })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dds::{DdsFormat, read_dds_info, read_dds_level};
    use crate::params::TextureFormat;

    fn noise(w: u32, h: u32, alpha: u8) -> Image {
        let mut rgba = Vec::new();
        for i in 0..w * h {
            rgba.extend_from_slice(&[(i * 7 % 256) as u8, (i * 13 % 256) as u8, (i * 29 % 256) as u8, alpha]);
        }
        Image::new(w, h, rgba).unwrap()
    }

    #[test]
    fn sizes_the_game_takes() {
        assert!(is_valid_size(256, 256));
        assert!(is_valid_size(1024, 512));
        assert!(is_valid_size(768, 128));
        assert!(!is_valid_size(300, 256));
        assert!(!is_valid_size(256, 300));
    }

    #[test]
    fn a_thumbnail_is_256_square_with_nine_levels() {
        let src = noise(40, 24, 255);
        let dds = make_thumbnail_dds(&src.rgba, 40, 24).unwrap();
        assert!(valid_thumbnail_dds(&dds));
        let info = read_dds_info(&dds).unwrap();
        assert_eq!((info.width, info.height, info.mips, info.format), (256, 256, 9, DdsFormat::Dxt1));
        // 64 * 64 + 32 * 32 + ... + 1 blocks, then three levels of one block each
        assert_eq!(dds.len(), 128 + (4096 + 1024 + 256 + 64 + 16 + 4 + 1 + 1 + 1) * 8);

        // a transparent pixel anywhere makes it DXT5
        let mut clear = src.clone();
        clear.rgba[3 + 4 * (12 * 40 + 20)] = 0;
        let dds = make_thumbnail_dds(&clear.rgba, 40, 24).unwrap();
        assert_eq!(read_dds_info(&dds).unwrap().format, DdsFormat::Dxt5);
        assert!(valid_thumbnail_dds(&dds));
    }

    #[test]
    fn the_thumbnail_takes_the_middle_square() {
        // left third red, middle third green, right third blue: 48 x 16, the square is green
        let mut rgba = Vec::new();
        for _y in 0..16 {
            for x in 0..48 {
                let c = [[255u8, 0, 0], [0, 255, 0], [0, 0, 255]][x / 16];
                rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
            }
        }
        let dds = make_thumbnail_dds(&rgba, 48, 16).unwrap();
        let top = read_dds_level(&dds, 0, 0).unwrap();
        assert!(top.rgba.as_chunks::<4>().0.iter().all(|p| p[0] < 16 && p[1] > 240 && p[2] < 16));
    }

    #[test]
    fn thumbnails_that_are_not_thumbnails_are_refused() {
        assert!(make_thumbnail_dds(&[0; 15 * 20 * 4], 15, 20).is_err());
        assert!(make_thumbnail_dds(&[0; 10], 16, 16).is_err());
        assert!(!valid_thumbnail_dds(&[]));
        assert!(!valid_thumbnail_dds(b"DDS "));
        let small = noise(64, 64, 255);
        let other = build_dds(&small, &opaque_or_alpha(false)).unwrap();
        assert!(!valid_thumbnail_dds(&other));
        // right header, wrong length
        let mut dds = make_thumbnail_dds(&noise(16, 16, 255).rgba, 16, 16).unwrap();
        dds.push(0);
        assert!(!valid_thumbnail_dds(&dds));
    }

    #[test]
    fn a_screen_picture_grows_to_a_multiple_of_four() {
        let src = noise(10, 6, 255);
        let dds = make_screen_picture_dds(&src.rgba, 10, 6).unwrap();
        let info = read_dds_info(&dds).unwrap();
        assert_eq!((info.width, info.height, info.format), (12, 8, DdsFormat::Dxt1));
        let clear = noise(8, 8, 100);
        let dds = make_screen_picture_dds(&clear.rgba, 8, 8).unwrap();
        assert_eq!(read_dds_info(&dds).unwrap().format, DdsFormat::Dxt5);
        assert!(make_screen_picture_dds(&[1, 2, 3], 8, 8).is_err());
    }

    #[test]
    fn game_textures_follow_the_description() {
        let src = noise(16, 16, 255);
        let mut params = TextureParams::for_new_texture(16, 16, false);
        let out = make_game_texture(&src, &params, None).unwrap();
        assert!(out.bump_error_dds.is_none());
        let info = read_dds_info(&out.dds).unwrap();
        assert_eq!((info.format, info.mips), (DdsFormat::Dxt1, 5));

        params.fmt = TextureFormat::RGBA;
        let out = make_game_texture(&src, &params, None).unwrap();
        assert_eq!(read_dds_level(&out.dds, 0, 0).unwrap(), src);

        params.ty = TextureType::BUMP_MAP;
        let out = make_game_texture(&src, &params, None).unwrap();
        assert!(out.bump_error_dds.is_some());
        assert_eq!(read_dds_info(&out.dds).unwrap().format, DdsFormat::Dxt5);

        params.ty = TextureType::IMAGE;
        params.fmt = TextureFormat::BC7;
        assert!(make_game_texture(&src, &params, None).is_err());
    }

    #[test]
    fn the_thumbnail_picture_is_128_square() {
        let px = thumbnail_pixels(&Image::filled(300, 200, [9, 8, 7, 255]).unwrap()).unwrap();
        assert_eq!(px.len(), 128 * 128 * 4);
        assert!(px.as_chunks::<4>().0.iter().all(|p| p[..3] == [9, 8, 7]));
        assert!(thumbnail_pixels(&Image::filled(1, 200, [0; 4]).unwrap()).is_err());
    }

    #[test]
    fn a_flat_picture_complies_fully() {
        let flat = Image::filled(16, 16, [128, 128, 128, 255]).unwrap();
        assert_eq!(check_compliance(&flat, false).unwrap(), (0, 0));
        assert_eq!(check_compliance(&Image::filled(1, 8, [0; 4]).unwrap(), true).unwrap(), (0, 0));
        // a checkerboard cannot survive a halving
        let mut rgba = Vec::new();
        for y in 0..16 {
            for x in 0..16 {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let (average, worst) = check_compliance(&Image::new(16, 16, rgba).unwrap(), false).unwrap();
        assert!(average > 20 && worst >= average);
    }

    #[test]
    fn borders_spread_colour_and_keep_alpha() {
        // one opaque red pixel in the middle of nothing
        let mut img = Image::filled(5, 5, [0, 0, 0, 0]).unwrap();
        img.rgba[(2 * 5 + 2) * 4..][..4].copy_from_slice(&[200, 10, 10, 255]);
        let out = apply_borders(&img).unwrap();
        for (i, p) in out.rgba.as_chunks::<4>().0.iter().enumerate() {
            assert_eq!(&p[..3], &[200, 10, 10], "pixel {i}");
            assert_eq!(p[3], img.rgba[i * 4 + 3]);
        }
        // nothing opaque: nothing to spread
        let empty = Image::filled(3, 3, [5, 6, 7, 100]).unwrap();
        assert_eq!(apply_borders(&empty).unwrap(), empty);
    }
}
