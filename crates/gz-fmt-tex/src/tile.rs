//! The picture a content browser tile draws for an image file.
//!
//! A tile is 48 to 192 pt and the editor runs at any DPI, so the picture is made for the size
//! the tile asks for: a DDS gives its own mip level first (the smallest one at least as large
//! as the tile, decoded alone), then the rest is an alpha-weighted box average down to the
//! tile, so a transparent pixel's colour never bleeds into its neighbours.

use rayon::prelude::*;

use crate::dds::{read_dds_info, read_dds_level};
use crate::load::load_image;
use crate::{Image, Result, TexError};

/// How the tile has to be blended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TileAlpha {
    Opaque,
    /// Every pixel fully on or fully off.
    Cutout,
    Translucent,
}

/// A tile's picture.
#[derive(Clone, Debug, PartialEq)]
pub struct TextureTile {
    /// At most `side` x `side`, the file's aspect kept.
    pub image: Image,
    /// The size of the file's top level.
    pub source_width: u32,
    pub source_height: u32,
    pub alpha: TileAlpha,
}

fn fit(width: u32, height: u32, side: u32) -> (u32, u32) {
    if width <= side && height <= side {
        return (width, height);
    }
    if width >= height {
        (
            side,
            ((u64::from(height) * u64::from(side) + u64::from(width) / 2) / u64::from(width)).max(1) as u32,
        )
    } else {
        (
            ((u64::from(width) * u64::from(side) + u64::from(height) / 2) / u64::from(height)).max(1) as u32,
            side,
        )
    }
}

/// The box average of `src` at `w` x `h`, each colour weighted by its alpha.
fn shrink(src: &Image, w: u32, h: u32) -> Result<Image> {
    if (w, h) == (src.width, src.height) {
        return Ok(src.clone());
    }
    let need = crate::checked_len(w, h)?;
    let (sw, sh) = (src.width as usize, src.height as usize);
    let mut out = vec![0u8; need];
    out.par_chunks_exact_mut(w as usize * 4).enumerate().for_each(|(y, row)| {
        let y0 = y * sh / h as usize;
        let y1 = ((y + 1) * sh / h as usize).max(y0 + 1).min(sh);
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let x0 = x * sw / w as usize;
            let x1 = ((x + 1) * sw / w as usize).max(x0 + 1).min(sw);
            let (mut r, mut g, mut b, mut a, mut n) = (0u64, 0u64, 0u64, 0u64, 0u64);
            for sy in y0..y1 {
                let line = &src.rgba[sy * sw * 4..(sy + 1) * sw * 4];
                for p in line[x0 * 4..x1 * 4].as_chunks::<4>().0 {
                    let pa = u64::from(p[3]);
                    r += u64::from(p[0]) * pa;
                    g += u64::from(p[1]) * pa;
                    b += u64::from(p[2]) * pa;
                    a += pa;
                    n += 1;
                }
            }
            if let Some(na) = std::num::NonZeroU64::new(a) {
                px[0] = ((r + a / 2) / na) as u8;
                px[1] = ((g + a / 2) / na) as u8;
                px[2] = ((b + a / 2) / na) as u8;
            }
            px[3] = ((a + n / 2) / n.max(1)) as u8;
        }
    });
    Image::new(w, h, out)
}

fn alpha_of(img: &Image) -> TileAlpha {
    let mut cut = false;
    for p in img.rgba.as_chunks::<4>().0 {
        match p[3] {
            255 => {}
            0 => cut = true,
            _ => return TileAlpha::Translucent,
        }
    }
    if cut { TileAlpha::Cutout } else { TileAlpha::Opaque }
}

/// The tile picture of an image file (`ext` without its dot, when the bytes alone do not say).
pub fn texture_tile(bytes: &[u8], ext: Option<&str>, side: u32) -> Result<TextureTile> {
    if side == 0 {
        return Err(TexError::BadSize { width: 0, height: 0 });
    }
    let (image, source_width, source_height) = if bytes.starts_with(b"DDS ") {
        let info = read_dds_info(bytes)?;
        // the smallest level that still covers the tile
        let mut level = 0;
        for m in 0..info.mips {
            let (w, h) = info.level_size(m);
            if w.max(h) >= side {
                level = m;
            } else {
                break;
            }
        }
        (read_dds_level(bytes, 0, level)?, info.width, info.height)
    } else {
        let loaded = load_image(bytes, ext)?;
        let (w, h) = (loaded.image.width, loaded.image.height);
        (loaded.image, w, h)
    };
    let (w, h) = fit(image.width, image.height, side);
    let image = shrink(&image, w, h)?;
    let alpha = alpha_of(&image);
    Ok(TextureTile { image, source_width, source_height, alpha })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tile_keeps_the_aspect() {
        assert_eq!(fit(512, 256, 128), (128, 64));
        assert_eq!(fit(100, 400, 64), (16, 64));
        assert_eq!(fit(32, 16, 128), (32, 16));
    }

    #[test]
    fn transparent_colour_does_not_bleed() {
        // a red opaque pixel next to a transparent green one
        let img = Image::new(2, 1, vec![255, 0, 0, 255, 0, 255, 0, 0]).unwrap();
        let s = shrink(&img, 1, 1).unwrap();
        assert_eq!(&s.rgba, &[255, 0, 0, 128]);
        assert_eq!(alpha_of(&s), TileAlpha::Translucent);
        assert_eq!(alpha_of(&Image::filled(2, 2, [1, 2, 3, 255]).unwrap()), TileAlpha::Opaque);
    }

    #[test]
    fn a_zero_side_is_an_error() {
        assert!(texture_tile(b"", None, 0).is_err());
    }
}
