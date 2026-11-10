//! `CImageManager::CreateMergedTexture`: pictures packed into one atlas (the first free place
//! that fits, rotated when only that fits), and the UV remap into it.

use glam::Vec2;
use gz_fmt_tex::{Image, MipFilterId, TextureFlags, TextureFormat, TextureParams, TextureType, make_game_texture};

use crate::{BuildError, Result};

/// One picture of the atlas: its layers (RGBA8 each, the same size).
#[derive(Clone, Debug)]
pub struct SourceImage {
    pub name: String,
    pub w: u32,
    pub h: u32,
    pub layers: Vec<Vec<u8>>,
}

/// The atlas: its layers, and per picture (in the packing order) where it went.
#[derive(Clone, Debug)]
pub struct Merged {
    pub w: u32,
    pub h: u32,
    pub layers: Vec<Vec<u8>>,
    pub offsets: Vec<Vec2>,
    pub scales: Vec<Vec2>,
    pub rotated: Vec<bool>,
    /// A picture's index in the call -> its index in the packing order.
    pub remap: Vec<usize>,
}

/// `_rect_test` + `_rect_place`: the first free place of a `w` x `h` rectangle, scanning rows,
/// then the same rotated.
fn place(mask: &mut [bool], dw: u32, dh: u32, w: u32, h: u32) -> Option<(u32, u32, bool)> {
    let fits = |mask: &[bool], x0: u32, y0: u32, w: u32, h: u32| -> bool {
        if x0 + w > dw || y0 + h > dh {
            return false;
        }
        (0..h).all(|y| (0..w).all(|x| !mask[((y0 + y) * dw + x0 + x) as usize]))
    };
    for (rot, (rw, rh)) in [(false, (w, h)), (true, (h, w))] {
        if rw > dw || rh > dh {
            continue;
        }
        for y in 0..=(dh - rh) {
            for x in 0..=(dw - rw) {
                if mask[(y * dw + x) as usize] {
                    continue;
                }
                if fits(mask, x, y, rw, rh) {
                    for yy in 0..rh {
                        for xx in 0..rw {
                            mask[((y + yy) * dw + x + xx) as usize] = true;
                        }
                    }
                    return Some((x, y, rot));
                }
            }
        }
    }
    None
}

/// One try at `dw` x `dh`.
fn try_merge(layer_cnt: usize, images: &[SourceImage], order: &[usize], dw: u32, dh: u32) -> Option<Merged> {
    let total: u64 = images.iter().map(|i| u64::from(i.w) * u64::from(i.h)).sum();
    if total > u64::from(dw) * u64::from(dh) {
        return None;
    }
    let mut mask = vec![false; (dw * dh) as usize];
    let mut layers = vec![vec![0u8; (dw * dh * 4) as usize]; layer_cnt];
    let (mut offsets, mut scales, mut rotated) = (Vec::new(), Vec::new(), Vec::new());
    for &i in order {
        let img = &images[i];
        let (x0, y0, rot) = place(&mut mask, dw, dh, img.w, img.h)?;
        let (rw, rh) = if rot { (img.h, img.w) } else { (img.w, img.h) };
        offsets.push(Vec2::new(x0 as f32 / dw as f32, y0 as f32 / dh as f32));
        scales.push(Vec2::new(rw as f32 / dw as f32, rh as f32 / dh as f32));
        rotated.push(rot);
        for (k, dst) in layers.iter_mut().enumerate() {
            let src = img.layers.get(k)?;
            for y in 0..img.h {
                for x in 0..img.w {
                    let (dx, dy) = if rot { (x0 + y, y0 + x) } else { (x0 + x, y0 + y) };
                    let s = ((y * img.w + x) * 4) as usize;
                    let d = ((dy * dw + dx) * 4) as usize;
                    dst.get_mut(d..d + 4)?.copy_from_slice(src.get(s..s + 4)?);
                }
            }
        }
    }
    let mut remap = vec![0usize; images.len()];
    for (pos, &i) in order.iter().enumerate() {
        remap[i] = pos;
    }
    Some(Merged { w: dw, h: dh, layers, offsets, scales, rotated, remap })
}

/// `CreateMergedTexture(layer_cnt, ...)` over the size range: the narrower side doubles until
/// the pictures fit. `None` when they do not fit in the largest size.
// oracle: the C++ sorts with `area > || longest edge >`, which is no strict ordering; here the
// pictures are sorted by area, then longest edge, largest first, keeping the order of equals.
#[must_use]
pub fn merge_textures(layer_cnt: usize, images: Vec<SourceImage>, w_min: u32, w_max: u32, h_min: u32, h_max: u32) -> Option<Merged> {
    if images.is_empty() {
        return None;
    }
    let mut order: Vec<usize> = (0..images.len()).collect();
    order.sort_by(|&a, &b| {
        let (ia, ib) = (&images[a], &images[b]);
        (ib.w * ib.h).cmp(&(ia.w * ia.h)).then((ib.w.max(ib.h)).cmp(&ia.w.max(ia.h)))
    });
    let (mut w, mut h) = (w_min, h_min);
    loop {
        if let Some(m) = try_merge(layer_cnt, &images, &order, w, h) {
            return Some(m);
        }
        if w <= h && w < w_max {
            w *= 2;
        } else if h < w && h < h_max {
            h *= 2;
        } else {
            return None;
        }
        if w > w_max || h > h_max {
            return None;
        }
    }
}

/// `MergedTextureRemapUV`.
#[must_use]
pub fn remap_uv(uv: Vec2, offs: Vec2, scale: Vec2, rotated: bool) -> Vec2 {
    if rotated { Vec2::new(scale.x * uv.y + offs.x, scale.y * uv.x + offs.y) } else { Vec2::new(scale.x * uv.x + offs.x, scale.y * uv.y + offs.y) }
}

/// `MakeGameTexture` of a merged layer: 32 bit, mip maps, dithered, the advanced filter.
pub fn game_texture(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>> {
    let image = Image::new(w, h, rgba.to_vec()).map_err(|e| BuildError::new(e.to_string()))?;
    let params = TextureParams {
        fmt: TextureFormat::RGBA,
        ty: TextureType::IMAGE,
        mip_filter: MipFilterId::ADVANCED,
        flags: TextureFlags::DITHER_COLOR | TextureFlags::GENERATE_MIP_MAPS,
        width: w as i32,
        height: h as i32,
        ..TextureParams::default()
    };
    make_game_texture(&image, &params, None).map(|t| t.dds).map_err(|e| BuildError::new(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn img(name: &str, w: u32, h: u32, v: u8) -> SourceImage {
        SourceImage { name: name.into(), w, h, layers: vec![vec![v; (w * h * 4) as usize]] }
    }

    #[test]
    fn pictures_are_packed_and_remapped() {
        let m = merge_textures(1, vec![img("small", 2, 2, 1), img("wide", 4, 2, 2)], 4, 8, 4, 8).unwrap();
        assert_eq!((m.w, m.h), (4, 4));
        // the larger picture goes first, at the origin
        assert_eq!(m.remap, [1, 0]);
        assert_eq!(m.offsets[0], Vec2::ZERO);
        assert_eq!(m.offsets[1], Vec2::new(0.0, 0.5));
        assert_eq!(remap_uv(Vec2::new(1.0, 1.0), m.offsets[1], m.scales[1], false), Vec2::new(0.5, 1.0));
        assert!(merge_textures(1, vec![img("big", 16, 16, 0)], 4, 8, 4, 8).is_none());
    }
}
