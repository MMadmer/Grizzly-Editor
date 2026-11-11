//! Mip levels: the resize the editor's texture compressor uses.
//!
//! The arithmetic follows `stb_image_resize` as the compressor calls it - 8-bit linear
//! channels, no alpha weighting, edges clamped - step by step in `f32`, so a level comes out
//! with the same bytes: values go to `v / 255`, are filtered along x and then along y, and come
//! back as `(int)(clamp01(v) * 255 + 0.5)`.

use rayon::prelude::*;

use crate::{Image, Result, checked_len};

/// The filters of the resizer.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum MipFilter {
    /// Mitchell when shrinking, Catmull-Rom when growing.
    Default,
    /// A box whose edges ramp over one pixel; for a halving, the mean of two.
    Box,
    Triangle,
    CubicBSpline,
    CatmullRom,
    Mitchell,
}

fn kernel(filter: MipFilter, x: f32, scale: f32) -> f32 {
    let x = x.abs();
    match filter {
        MipFilter::Box => {
            let halfscale = scale / 2.0;
            let t = 0.5 + halfscale;
            if x >= t {
                0.0
            } else {
                let r = 0.5 - halfscale;
                if x <= r { 1.0 } else { (t - x) / scale }
            }
        }
        MipFilter::Triangle => {
            if x <= 1.0 {
                1.0 - x
            } else {
                0.0
            }
        }
        MipFilter::CubicBSpline => {
            if x < 1.0 {
                (4.0 + x * x * (3.0 * x - 6.0)) / 6.0
            } else if x < 2.0 {
                (8.0 + x * (-12.0 + x * (6.0 - x))) / 6.0
            } else {
                0.0
            }
        }
        MipFilter::CatmullRom => {
            if x < 1.0 {
                1.0 - x * x * (2.5 - 1.5 * x)
            } else if x < 2.0 {
                2.0 - x * (4.0 + x * (0.5 * x - 2.5))
            } else {
                0.0
            }
        }
        // `Default` is resolved before a kernel is asked for
        MipFilter::Mitchell | MipFilter::Default => {
            if x < 1.0 {
                (16.0 + x * x * (21.0 * x - 36.0)) / 18.0
            } else if x < 2.0 {
                (32.0 + x * (-60.0 + x * (36.0 - 7.0 * x))) / 18.0
            } else {
                0.0
            }
        }
    }
}

fn support(filter: MipFilter, scale: f32) -> f32 {
    match filter {
        MipFilter::Box => 0.5 + scale / 2.0,
        MipFilter::Triangle => 1.0,
        _ => 2.0,
    }
}

/// For every output pixel of one axis, the input pixels it is made of, in the order the
/// resizer adds them up.
struct Axis {
    /// `taps[starts[k]..starts[k + 1]]` belong to output pixel `k`.
    starts: Vec<u32>,
    taps: Vec<(u32, f32)>,
}

impl Axis {
    fn of(&self, k: usize) -> &[(u32, f32)] {
        let from = self.starts.get(k).copied().unwrap_or(0) as usize;
        let to = self.starts.get(k + 1).copied().unwrap_or(0) as usize;
        self.taps.get(from..to).unwrap_or(&[])
    }

    /// The input pixels the output pixels `from..to` read: first and one past the last.
    fn span(&self, from: usize, to: usize) -> (usize, usize) {
        let mut lo = usize::MAX;
        let mut hi = 0usize;
        for k in from..to {
            for &(src, _) in self.of(k) {
                lo = lo.min(src as usize);
                hi = hi.max(src as usize + 1);
            }
        }
        if lo > hi { (0, 0) } else { (lo, hi) }
    }
}

fn plan(in_size: usize, out_size: usize, filter: MipFilter) -> Axis {
    let scale = out_size as f32 / in_size as f32;
    let growing = scale > 1.0;
    let filter = match filter {
        MipFilter::Default if growing => MipFilter::CatmullRom,
        MipFilter::Default => MipFilter::Mitchell,
        f => f,
    };
    let clamp = |n: i64| n.clamp(0, in_size as i64 - 1) as u32;
    // (output pixel, input pixel, weight), in the order they are found
    let mut entries: Vec<(u32, u32, f32)> = Vec::new();

    if growing {
        let radius = support(filter, 1.0 / scale) * scale;
        for n in 0..out_size {
            let centre = n as f32 + 0.5;
            let lo = (centre - radius) / scale;
            let hi = (centre + radius) / scale;
            let in_centre = centre / scale;
            let mut first = (f64::from(lo) + 0.5).floor() as i64;
            let last = (f64::from(hi) - 0.5).floor() as i64;

            let at = entries.len();
            let mut total = 0f32;
            let mut i = 0i64;
            while i <= last - first {
                let c = kernel(filter, in_centre - ((i + first) as f32 + 0.5), 1.0 / scale);
                // a pixel without weight at the front is skipped; the ones outside the image
                // stay, clamped to the edge
                if i == 0 && c == 0.0 {
                    first += 1;
                    continue;
                }
                entries.push((n as u32, clamp(i + first), c));
                total += c;
                i += 1;
            }
            let norm = 1.0 / total;
            for e in entries.iter_mut().skip(at) {
                e.2 *= norm;
            }
        }
    } else {
        let sup = support(filter, scale);
        let radius = sup / scale;
        let margin = i64::from((f64::from(sup * 2.0 / scale).ceil() as i32) / 2);
        let mut totals = vec![0f32; out_size];
        for n in -margin..in_size as i64 + margin {
            let centre = n as f32 + 0.5;
            let lo = (centre - radius) * scale;
            let hi = (centre + radius) * scale;
            let out_centre = centre * scale;
            let first = (f64::from(lo) + 0.5).floor() as i64;
            let last = (f64::from(hi) - 0.5).floor() as i64;
            for k in first.max(0)..=last.min(out_size as i64 - 1) {
                let c = kernel(filter, (k as f32 + 0.5) - out_centre, scale) * scale;
                if let Some(total) = totals.get_mut(k as usize) {
                    *total += c;
                }
                entries.push((k as u32, clamp(n), c));
            }
        }
        // every output pixel's weights are brought to a sum of one
        for e in &mut entries {
            if let Some(&total) = totals.get(e.0 as usize) {
                e.2 *= 1.0 / total;
            }
        }
    }

    // group by output pixel, keeping the order within one
    let mut starts = vec![0u32; out_size + 1];
    for e in entries.iter().filter(|e| e.2 != 0.0) {
        if let Some(slot) = starts.get_mut(e.0 as usize + 1) {
            *slot += 1;
        }
    }
    for k in 1..starts.len() {
        starts[k] += starts[k - 1];
    }
    let mut next: Vec<u32> = starts.clone();
    let mut taps = vec![(0u32, 0f32); entries.iter().filter(|e| e.2 != 0.0).count()];
    for e in entries.iter().filter(|e| e.2 != 0.0) {
        if let Some(slot) = next.get_mut(e.0 as usize) {
            if let Some(tap) = taps.get_mut(*slot as usize) {
                *tap = (e.1, e.2);
            }
            *slot += 1;
        }
    }
    Axis { starts, taps }
}

fn encode(v: f32) -> u8 {
    let s = v.clamp(0.0, 1.0);
    (f64::from(s * 255.0) + 0.5) as i32 as u8
}

fn unit_table() -> [f32; 256] {
    let mut table = [0f32; 256];
    for (v, slot) in table.iter_mut().enumerate() {
        *slot = v as f32 / 255.0;
    }
    table
}

/// The halving with the box filter: every output pixel is two pixels of two rows, each added
/// with weight one half, in the resizer's order.
fn halve_box(src: &[u8], sw: usize, dw: usize, dh: usize) -> Vec<u8> {
    let unit = unit_table();
    let mut out = vec![0u8; dw * dh * 4];
    out.par_chunks_mut(dw * 4).enumerate().for_each(|(y, row)| {
        let top = src.get(2 * y * sw * 4..).unwrap_or(&[]);
        let bottom = src.get((2 * y + 1) * sw * 4..).unwrap_or(&[]);
        for (x, px) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let a = top.get(x * 8..x * 8 + 8);
            let b = bottom.get(x * 8..x * 8 + 8);
            let (Some(a), Some(b)) = (a, b) else { continue };
            for c in 0..4 {
                let h0 = (0.0 + unit[usize::from(a[c])] * 0.5) + unit[usize::from(a[c + 4])] * 0.5;
                let h1 = (0.0 + unit[usize::from(b[c])] * 0.5) + unit[usize::from(b[c + 4])] * 0.5;
                px[c] = encode((0.0 + h0 * 0.5) + h1 * 0.5);
            }
        }
    });
    out
}

/// Output rows a band of the general path makes at once: enough to share the horizontal pass,
/// small enough that the intermediate rows of a band stay a few megabytes.
const BAND: usize = 32;

fn resize_general(src: &[u8], sw: usize, sh: usize, dw: usize, dh: usize, filter: MipFilter) -> Vec<u8> {
    let unit = unit_table();
    let xs = plan(sw, dw, filter);
    let ys = plan(sh, dh, filter);
    let mut out = vec![0u8; dw * dh * 4];
    out.par_chunks_mut(dw * 4 * BAND).enumerate().for_each(|(band, rows)| {
        let y0 = band * BAND;
        let y1 = y0 + rows.len() / (dw * 4);
        let (s0, s1) = ys.span(y0, y1);

        // the source rows this band reads, filtered along x
        let mut wide = vec![0f32; (s1 - s0) * dw * 4];
        for (line, sy) in wide.chunks_exact_mut(dw * 4).zip(s0..s1) {
            let Some(src_row) = src.get(sy * sw * 4..(sy + 1) * sw * 4) else { continue };
            for (k, px) in line.as_chunks_mut::<4>().0.iter_mut().enumerate() {
                for &(sx, w) in xs.of(k) {
                    if let Some(p) = src_row.get(sx as usize * 4..sx as usize * 4 + 4) {
                        for c in 0..4 {
                            px[c] += unit[usize::from(p[c])] * w;
                        }
                    }
                }
            }
        }

        // then along y
        let mut acc = vec![0f32; dw * 4];
        for (row, k) in rows.chunks_exact_mut(dw * 4).zip(y0..y1) {
            acc.fill(0.0);
            for &(sy, w) in ys.of(k) {
                let at = (sy as usize - s0) * dw * 4;
                if let Some(line) = wide.get(at..at + dw * 4) {
                    for (a, &v) in acc.iter_mut().zip(line) {
                        *a += v * w;
                    }
                }
            }
            for (d, &v) in row.iter_mut().zip(&acc) {
                *d = encode(v);
            }
        }
    });
    out
}

/// Resizes a picture with the texture compressor's resizer.
pub fn resize(src: &Image, width: u32, height: u32, filter: MipFilter) -> Result<Image> {
    src.check()?;
    checked_len(width, height)?;
    let (sw, sh, dw, dh) = (src.width as usize, src.height as usize, width as usize, height as usize);
    let rgba = if filter == MipFilter::Box && sw == dw * 2 && sh == dh * 2 {
        halve_box(&src.rgba, sw, dw, dh)
    } else {
        resize_general(&src.rgba, sw, sh, dw, dh, filter)
    };
    Ok(Image { width, height, rgba })
}

/// Levels a texture of this size has, the picture itself included.
#[must_use]
pub fn mip_count(width: u32, height: u32) -> u32 {
    32 - width.max(height).max(1).leading_zeros()
}

/// Every level under a picture, largest first, down to 1 x 1; each is made from the one above.
///
/// oracle: the compressor asks for a level of height (or width) zero once the shorter side has
/// reached one pixel, the resize fails and the level keeps whatever the memory held. Here such
/// a level is one pixel on that side and filtered like the rest.
pub fn mip_chain(base: &Image, filter: MipFilter) -> Result<Vec<Image>> {
    base.check()?;
    let count = mip_count(base.width, base.height);
    let mut levels: Vec<Image> = Vec::with_capacity(count.saturating_sub(1) as usize);
    for _ in 1..count {
        let above = levels.last().unwrap_or(base);
        let level = resize(above, (above.width / 2).max(1), (above.height / 2).max(1), filter)?;
        levels.push(level);
    }
    Ok(levels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(w: u32, h: u32) -> Image {
        let mut rgba = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let v = if (x + y) % 2 == 0 { 255 } else { 0 };
                rgba.extend_from_slice(&[v, (x * 16) as u8, (y * 16) as u8, 255]);
            }
        }
        Image::new(w, h, rgba).unwrap()
    }

    #[test]
    fn counts_levels() {
        assert_eq!(mip_count(256, 256), 9);
        assert_eq!(mip_count(1024, 512), 11);
        assert_eq!(mip_count(1, 1), 1);
        assert_eq!(mip_count(5, 3), 3);
    }

    #[test]
    fn a_box_halving_is_the_mean_of_four() {
        let src = Image::new(2, 2, vec![10, 0, 0, 255, 20, 0, 0, 255, 30, 0, 0, 255, 41, 0, 0, 255]).unwrap();
        let out = resize(&src, 1, 1, MipFilter::Box).unwrap();
        // (10 + 20 + 30 + 41) / 4 = 25.25
        assert_eq!(out.rgba, [25, 0, 0, 255]);
        // the general path agrees with the fast one on a halving
        let big = checker(8, 8);
        let fast = halve_box(&big.rgba, 8, 4, 4);
        let general = resize_general(&big.rgba, 8, 8, 4, 4, MipFilter::Box);
        assert_eq!(fast, general);
    }

    #[test]
    fn a_box_halving_takes_two_pixels_each() {
        let axis = plan(8, 4, MipFilter::Box);
        for k in 0..4 {
            assert_eq!(axis.of(k), &[(2 * k as u32, 0.5), (2 * k as u32 + 1, 0.5)]);
        }
        // the same size is the picture itself
        let same = plan(3, 3, MipFilter::Box);
        for k in 0..3 {
            assert_eq!(same.of(k), &[(k as u32, 1.0)]);
        }
    }

    #[test]
    fn weights_sum_to_one_for_every_filter_and_direction() {
        for filter in [
            MipFilter::Default,
            MipFilter::Box,
            MipFilter::Triangle,
            MipFilter::CubicBSpline,
            MipFilter::CatmullRom,
            MipFilter::Mitchell,
        ] {
            for (from, to) in [(16usize, 8usize), (5, 2), (7, 3), (4, 9), (1, 4)] {
                // the box filter is for shrinking only
                if filter == MipFilter::Box && to > from {
                    continue;
                }
                let axis = plan(from, to, filter);
                for k in 0..to {
                    let sum: f32 = axis.of(k).iter().map(|t| t.1).sum();
                    assert!((sum - 1.0).abs() < 1e-4, "{filter:?} {from}->{to} pixel {k}: {sum}");
                    assert!(axis.of(k).iter().all(|t| (t.0 as usize) < from));
                }
            }
        }
    }

    #[test]
    fn a_flat_picture_stays_flat() {
        let src = Image::filled(13, 7, [200, 100, 50, 25]).unwrap();
        for filter in [MipFilter::Default, MipFilter::Triangle, MipFilter::CatmullRom] {
            let out = resize(&src, 6, 3, filter).unwrap();
            assert!(out.rgba.as_chunks::<4>().0.iter().all(|p| *p == [200, 100, 50, 25]), "{filter:?}");
        }
        let up = resize(&src, 20, 9, MipFilter::Default).unwrap();
        assert!(up.rgba.as_chunks::<4>().0.iter().all(|p| *p == [200, 100, 50, 25]));
    }

    #[test]
    fn the_chain_goes_down_to_one_pixel() {
        let chain = mip_chain(&checker(8, 2), MipFilter::Box).unwrap();
        let sizes: Vec<(u32, u32)> = chain.iter().map(|l| (l.width, l.height)).collect();
        assert_eq!(sizes, [(4, 1), (2, 1), (1, 1)]);
        assert!(mip_chain(&checker(1, 1), MipFilter::Box).unwrap().is_empty());
        // a checker averages to mid grey
        assert_eq!(chain[0].rgba[0], 128);
    }

    #[test]
    fn bad_sizes_are_errors() {
        let src = checker(4, 4);
        assert!(resize(&src, 0, 4, MipFilter::Box).is_err());
        assert!(resize(&src, crate::MAX_SIDE + 1, 4, MipFilter::Box).is_err());
        let broken = Image { width: 4, height: 4, rgba: vec![0; 3] };
        assert!(resize(&broken, 2, 2, MipFilter::Box).is_err());
        assert!(mip_chain(&broken, MipFilter::Box).is_err());
    }
}
