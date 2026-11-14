//! A lightmap layer: a deflector's (or a lightmap page's) texels and their markers, the blits that
//! copy one layer into another, the border growth and the
//! packing into pixels.
//!
//! A marker says how a texel got its light: 255 lit directly, `ref` grown by the border growth at
//! that level, 0 not at all.

use gz_fmt_tex::Image;
use rayon::prelude::*;

use crate::color::{BaseColor, ColorC};
use crate::math::{color_get_a, color_get_b, color_get_g, color_get_r, color_rgba, u8_clr};

/// `BORDER`: the texels a deflector's layer grows by on every side.
pub const BORDER: u32 = 1;

/// `lm_layer`. `width` and `height` are the size the texels are lit at; a layer grown by its
/// border keeps them while `surface` holds the grown texels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LmLayer {
    pub width: u32,
    pub height: u32,
    pub surface: Vec<BaseColor>,
    pub marker: Vec<u8>,
}

impl LmLayer {
    /// `create`: black texels, every marker 0.
    pub fn create(&mut self, w: u32, h: u32) {
        self.width = w;
        self.height = h;
        let n = w as usize * h as usize;
        self.surface.clear();
        self.surface.resize(n, BaseColor::default());
        self.marker.clear();
        self.marker.resize(n, 0);
    }

    #[must_use]
    pub fn new(w: u32, h: u32) -> Self {
        let mut l = Self::default();
        l.create(w, h);
        l
    }

    /// `destroy`.
    pub fn destroy(&mut self) {
        self.width = 0;
        self.height = 0;
        self.surface = Vec::new();
        self.marker = Vec::new();
    }

    /// `Area`: the texels the layer takes in a page, its border included.
    #[must_use]
    pub fn area(&self) -> u32 {
        (self.width + 2 * BORDER) * (self.height + 2 * BORDER)
    }

    /// `Pixel`: r, g, b, sun, hemi of a texel as bytes.
    #[must_use]
    pub fn pixel(&self, id: usize) -> [u8; 5] {
        let c = self.surface.get(id).map(BaseColor::get).unwrap_or_default();
        [u8_clr(c.rgb.x), u8_clr(c.rgb.y), u8_clr(c.rgb.z), u8_clr(c.sun), u8_clr(c.hemi)]
    }

    /// `rgb` and the sun in alpha, `0xAARRGGBB`.
    #[must_use]
    pub fn pack(&self) -> Vec<u32> {
        self.surface
            .par_iter()
            .with_min_len(4096)
            .map(|s| {
                let c = s.get();
                color_rgba(u8_clr(c.rgb.x), u8_clr(c.rgb.y), u8_clr(c.rgb.z), u8_clr(c.sun))
            })
            .collect()
    }

    /// The sun in rgb, the hemisphere in alpha.
    #[must_use]
    pub fn pack_hemi(&self) -> Vec<u32> {
        self.surface
            .par_iter()
            .with_min_len(4096)
            .map(|s| {
                let c = s.get();
                let d = u8_clr(c.sun);
                color_rgba(d, d, d, u8_clr(c.hemi))
            })
            .collect()
    }

    /// `similar`: equal markers, texels equal to the last step.
    #[must_use]
    pub fn similar(&self, o: &LmLayer) -> bool {
        self.marker == o.marker
            && self.surface == o.surface
            && self.width == o.width
            && self.height == o.height
    }
}

/// `0xAARRGGBB` pixels as an [`Image`] (bytes R, G, B, A).
#[must_use]
pub fn image_of(width: u32, height: u32, pixels: &[u32]) -> Image {
    let mut rgba = Vec::with_capacity(pixels.len() * 4);
    for &p in pixels {
        rgba.extend_from_slice(&[
            color_get_r(p) as u8,
            color_get_g(p) as u8,
            color_get_b(p) as u8,
            color_get_a(p) as u8,
        ]);
    }
    Image { width, height, rgba }
}

/// An [`Image`] back to `0xAARRGGBB` pixels.
#[must_use]
pub fn pixels_of(image: &Image) -> Vec<u32> {
    image.rgba.as_chunks::<4>().0.iter().map(|p| color_rgba(p[0], p[1], p[2], p[3])).collect()
}

/// The blit: the texels of `src` (`ss_x` x `ss_y` of them) whose marker is
/// at least `a_ref`, into `dst` (`ds_x` wide) at `(px, py)`. Texels outside `dst` are skipped
/// where X-Ray's compiler asserts.
#[allow(clippy::too_many_arguments)]
pub fn blit(
    dst: &mut LmLayer,
    ds_x: u32,
    ds_y: u32,
    src: &LmLayer,
    ss_x: u32,
    ss_y: u32,
    px: u32,
    py: u32,
    a_ref: u32,
) {
    if ds_x < ss_x + px || ds_y < ss_y + py {
        return;
    }
    for y in 0..ss_y {
        for x in 0..ss_x {
            let s = (y * ss_x + x) as usize;
            let d = ((py + y) * ds_x + px + x) as usize;
            copy_texel(dst, d, src, s, a_ref);
        }
    }
}

/// `blit_r`: the same, rotated: row `y` of `src` becomes column `y` of `dst`.
#[allow(clippy::too_many_arguments)]
pub fn blit_r(
    dst: &mut LmLayer,
    ds_x: u32,
    ds_y: u32,
    src: &LmLayer,
    ss_x: u32,
    ss_y: u32,
    px: u32,
    py: u32,
    a_ref: u32,
) {
    if ds_x < ss_y + px || ds_y < ss_x + py {
        return;
    }
    for y in 0..ss_y {
        for x in 0..ss_x {
            let s = (y * ss_x + x) as usize;
            let d = ((py + x) * ds_x + px + y) as usize;
            copy_texel(dst, d, src, s, a_ref);
        }
    }
}

fn copy_texel(dst: &mut LmLayer, d: usize, src: &LmLayer, s: usize, a_ref: u32) {
    let (Some(&sc), Some(&sm)) = (src.surface.get(s), src.marker.get(s)) else { return };
    if u32::from(sm) >= a_ref
        && let (Some(dc), Some(dm)) = (dst.surface.get_mut(d), dst.marker.get_mut(d))
    {
        *dc = sc;
        *dm = sm;
    }
}

/// `lblit`: [`blit`] with the layers' own sizes.
pub fn lblit(dst: &mut LmLayer, src: &LmLayer, px: u32, py: u32, a_ref: u32) {
    let (dw, dh, sw, sh) = (dst.width, dst.height, src.width, src.height);
    blit(dst, dw, dh, src, sw, sh, px, py, a_ref);
}

/// Every unlit texel next to texels marked above `r` takes their mean and the
/// marker `r`. Every texel reads the layer as it was before the pass (as X-Ray's compiler does
/// through its saved lines), so the rows are grown in parallel. Answers whether a texel
/// grew.
pub fn apply_borders(lm: &mut LmLayer, r: u32) -> bool {
    let (w, h) = (lm.width as usize, lm.height as usize);
    if w == 0 || h == 0 || lm.surface.len() < w * h || lm.marker.len() < w * h {
        return false;
    }
    let src_s = lm.surface.clone();
    let src_m = lm.marker.clone();
    lm.surface
        .par_chunks_mut(w)
        .zip(lm.marker.par_chunks_mut(w))
        .take(h)
        .enumerate()
        .map(|(y, (srow, mrow))| {
            let mut any = false;
            for x in 0..w {
                if src_m.get(y * w + x) != Some(&0) {
                    continue;
                }
                let mut sum = ColorC::default();
                let mut count = 0u32;
                // X-Ray's order: the row above, left, right, the row below
                let take = |dx: isize, dy: isize, sum: &mut ColorC, count: &mut u32| {
                    let (nx, ny) = (x as isize + dx, y as isize + dy);
                    if nx < 0 || ny < 0 || nx >= w as isize || ny >= h as isize {
                        return;
                    }
                    let id = ny as usize * w + nx as usize;
                    if src_m.get(id).is_some_and(|&m| u32::from(m) > r)
                        && let Some(c) = src_s.get(id)
                    {
                        sum.add(&c.get());
                        *count += 1;
                    }
                };
                for (dx, dy) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
                    take(dx, dy, &mut sum, &mut count);
                }
                if count > 0 {
                    sum.scale(count);
                    if let (Some(s), Some(m)) = (srow.get_mut(x), mrow.get_mut(x)) {
                        *s = BaseColor::from_c(&sum);
                        *m = r as u8;
                    }
                    any = true;
                }
            }
            any
        })
        .reduce(|| false, |a, b| a || b)
}

#[cfg(test)]
mod tests {
    use super::*;
    use glam::Vec3;

    #[test]
    fn the_border_grows_ring_by_ring_with_the_mean_of_the_lit() {
        // a 5 x 1 row: two lit texels at the left end
        let mut lm = LmLayer::new(5, 1);
        let lit = |v: f32| BaseColor::from_c(&ColorC { rgb: Vec3::splat(v), hemi: v, sun: v, tmp: 0.0 });
        lm.surface[0] = lit(1.0);
        lm.surface[1] = lit(0.5);
        lm.marker[0] = 255;
        lm.marker[1] = 255;
        assert!(apply_borders(&mut lm, 254));
        // texel 2 sees texel 1 alone; texel 3 saw nothing lit before this pass
        assert_eq!(lm.marker, vec![255, 255, 254, 0, 0]);
        assert_eq!(lm.surface[2], BaseColor::from_c(&lit(0.5).get()));
        assert!(apply_borders(&mut lm, 253));
        assert_eq!(lm.marker, vec![255, 255, 254, 253, 0]);
        // a pass at a level the ring is not above grows nothing
        assert!(!apply_borders(&mut lm, 254));
    }

    #[test]
    fn a_rotated_blit_turns_rows_into_columns() {
        let mut src = LmLayer::new(3, 2);
        for i in 0..6 {
            src.marker[i] = 255;
            src.surface[i].r = i as i16;
        }
        let mut dst = LmLayer::new(4, 4);
        blit_r(&mut dst, 4, 4, &src, 3, 2, 1, 0, 254);
        // source (x = 2, y = 1) lands at (1 + 1, 0 + 2)
        assert_eq!(dst.surface[2 * 4 + 2].r, 5);
        assert_eq!(dst.marker.iter().filter(|&&m| m == 255).count(), 6);
    }
}
