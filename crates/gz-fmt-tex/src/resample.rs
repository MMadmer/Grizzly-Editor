//! The engine's own image resampler (`imf_Process`, after Dale Schumacher's filtered image
//! rescaling): the 128 x 128 picture of a thumbnail and the compliance check are made with it.
//!
//! The arithmetic is kept as it is, quirks included: weights are not brought to a sum of one,
//! the picture is rounded to 8 bits between the two passes, and alpha is rounded up twice.

use rayon::prelude::*;

use crate::{Image, Result, TexError, checked_len};

/// `EIMF_Type`.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum ImfFilter {
    /// `imf_filter`: the cubic `2|t|^3 - 3|t|^2 + 1`.
    Hermite,
    Box,
    Triangle,
    Bell,
    BSpline,
    Lanczos3,
    Mitchell,
}

impl ImfFilter {
    fn support(self) -> f32 {
        match self {
            ImfFilter::Hermite | ImfFilter::Triangle => 1.0,
            ImfFilter::Box => 0.5,
            ImfFilter::Bell => 1.5,
            ImfFilter::BSpline | ImfFilter::Mitchell => 2.0,
            ImfFilter::Lanczos3 => 3.0,
        }
    }

    fn weight(self, t: f32) -> f32 {
        match self {
            ImfFilter::Hermite => {
                let t = f64::from(t.abs());
                if t < 1.0 { ((2.0 * t - 3.0) * t * t + 1.0) as f32 } else { 0.0 }
            }
            ImfFilter::Box => {
                if f64::from(t) > -0.5 && f64::from(t) <= 0.5 {
                    1.0
                } else {
                    0.0
                }
            }
            ImfFilter::Triangle => {
                let t = t.abs();
                if t < 1.0 { 1.0 - t } else { 0.0 }
            }
            ImfFilter::Bell => {
                let t = t.abs();
                if t < 0.5 {
                    (0.75 - f64::from(t * t)) as f32
                } else if t < 1.5 {
                    let t = t - 1.5;
                    0.5 * (t * t)
                } else {
                    0.0
                }
            }
            ImfFilter::BSpline => {
                let t = t.abs();
                if t < 1.0 {
                    let tt = t * t;
                    (0.5 * tt * t) - tt + (2.0 / 3.0)
                } else if t < 2.0 {
                    let t = 2.0 - t;
                    (1.0 / 6.0) * (t * t * t)
                } else {
                    0.0
                }
            }
            ImfFilter::Lanczos3 => {
                let t = t.abs();
                if t < 3.0 { sinc(t) * sinc(t / 3.0) } else { 0.0 }
            }
            ImfFilter::Mitchell => {
                const B: f32 = 1.0 / 3.0;
                const C: f32 = 1.0 / 3.0;
                let tt = t * t;
                let t = t.abs();
                if t < 1.0 {
                    let v = ((12.0 - 9.0 * B - 6.0 * C) * (t * tt))
                        + ((-18.0 + 12.0 * B + 6.0 * C) * tt)
                        + (6.0 - 2.0 * B);
                    v / 6.0
                } else if t < 2.0 {
                    let v = ((-B - 6.0 * C) * (t * tt))
                        + ((6.0 * B + 30.0 * C) * tt)
                        + ((-12.0 * B - 48.0 * C) * t)
                        + (8.0 * B + 24.0 * C);
                    v / 6.0
                } else {
                    0.0
                }
            }
        }
    }
}

// the C++ resampler's literal, one ulp from `f32::consts::PI`: the filter weights stay its own
#[allow(clippy::approx_constant)]
fn sinc(x: f32) -> f32 {
    let x = x * 3.141_592_6;
    if x != 0.0 { x.sin() / x } else { 1.0 }
}

/// The source pixels one destination pixel is made of.
type Contrib = Vec<(u32, f32)>;

fn contributions(src: usize, dst: usize, filter: ImfFilter) -> Vec<Contrib> {
    let scale = dst as f32 / src as f32;
    let fwidth = filter.support();
    let mirror = |j: i64| -> u32 {
        let size = src as i64;
        let n = if j < 0 {
            -j
        } else if j >= size {
            (size - j) + size - 1
        } else {
            j
        };
        // oracle: a tap mirrored past the far edge takes the last pixel; one that lands before
        // the first pixel read outside the picture, here it takes the first
        n.clamp(0, size - 1) as u32
    };
    (0..dst)
        .map(|i| {
            let centre = i as f32 / scale;
            let (width, fscale) = if scale < 1.0 { (fwidth / scale, 1.0 / scale) } else { (fwidth, 1.0) };
            let left = f64::from(centre - width).ceil() as f32 as i64;
            let right = f64::from(centre + width).floor() as f32 as i64;
            (left..=right)
                .map(|j| {
                    let t = centre - j as f32;
                    let weight =
                        if scale < 1.0 { filter.weight(t / fscale) / fscale } else { filter.weight(t) };
                    (mirror(j), weight)
                })
                .collect()
        })
        .collect()
}

/// `CC`: round to nearest, clamp to a byte.
fn cc(a: f32) -> u8 {
    (a + 0.5).floor().clamp(0.0, 255.0) as u8
}

fn mix(taps: &[(u32, f32)], mut pixel: impl FnMut(u32) -> [u8; 4]) -> [u8; 4] {
    let mut sum = [0f32; 4];
    for &(at, w) in taps {
        let p = pixel(at);
        for c in 0..4 {
            sum[c] += w * f32::from(p[c]);
        }
    }
    [cc(sum[0]), cc(sum[1]), cc(sum[2]), cc(sum[3] + 0.5)]
}

/// Resamples a picture (`imf_Process`). Both pictures must be at least 2 pixels on each side.
pub fn imf_resample(src: &Image, width: u32, height: u32, filter: ImfFilter) -> Result<Image> {
    src.check()?;
    let len = checked_len(width, height)?;
    if width < 2 || height < 2 || src.width < 2 || src.height < 2 {
        return Err(TexError::Invalid(format!(
            "the resampler needs at least 2 pixels on a side: {} x {} into {width} x {height}",
            src.width, src.height
        )));
    }
    let (sw, sh, dw, dh) = (src.width as usize, src.height as usize, width as usize, height as usize);

    // along x, into a picture as wide as the result and as high as the source
    let xs = contributions(sw, dw, filter);
    let mut tmp = vec![0u8; dw * sh * 4];
    tmp.par_chunks_mut(dw * 4).zip(src.rgba.par_chunks(sw * 4)).for_each(|(dst, row)| {
        for (px, taps) in dst.as_chunks_mut::<4>().0.iter_mut().zip(&xs) {
            let p = mix(taps, |at| {
                let i = at as usize * 4;
                row.get(i..i + 4).map_or([0; 4], |s| [s[0], s[1], s[2], s[3]])
            });
            px.copy_from_slice(&p);
        }
    });

    // along y
    let ys = contributions(sh, dh, filter);
    let mut out = vec![0u8; len];
    out.par_chunks_mut(dw * 4).zip(ys.par_iter()).for_each(|(dst, taps)| {
        for (x, px) in dst.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let p = mix(taps, |at| {
                let i = (at as usize * dw + x) * 4;
                tmp.get(i..i + 4).map_or([0; 4], |s| [s[0], s[1], s[2], s[3]])
            });
            px.copy_from_slice(&p);
        }
    });
    Ok(Image { width, height, rgba: out })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_picture_resampled_with_the_box_stays_flat() {
        let src = Image::filled(256, 256, [10, 200, 30, 255]).unwrap();
        let out = imf_resample(&src, 128, 128, ImfFilter::Box).unwrap();
        assert_eq!((out.width, out.height), (128, 128));
        // two taps of weight one half each
        assert!(out.rgba.as_chunks::<4>().0.iter().all(|p| *p == [10, 200, 30, 255]));
    }

    #[test]
    fn a_box_halving_averages_neighbours() {
        // columns 0, 100, 0, 100 ...
        let mut rgba = Vec::new();
        for _y in 0..4 {
            for x in 0..4 {
                let v = if x % 2 == 0 { 0 } else { 100 };
                rgba.extend_from_slice(&[v, v, v, 255]);
            }
        }
        let src = Image::new(4, 4, rgba).unwrap();
        let out = imf_resample(&src, 2, 2, ImfFilter::Box).unwrap();
        // the box of a halving covers the pixel under the centre and the one before it (at the
        // left edge its mirror image): one dark and one light column each time
        assert_eq!(out.rgba[0], 50);
        assert_eq!(out.rgba[4], 50);
    }

    #[test]
    fn the_same_size_is_the_picture_itself() {
        let mut rgba = Vec::new();
        for i in 0..9u8 {
            rgba.extend_from_slice(&[i * 20, 255 - i * 20, i, 200]);
        }
        let src = Image::new(3, 3, rgba).unwrap();
        for filter in [ImfFilter::Box, ImfFilter::Triangle, ImfFilter::Hermite] {
            assert_eq!(imf_resample(&src, 3, 3, filter).unwrap().rgba[..3], src.rgba[..3]);
        }
    }

    #[test]
    fn every_filter_has_its_full_weight_at_the_centre_and_none_past_its_support() {
        for filter in [ImfFilter::Hermite, ImfFilter::Box, ImfFilter::Triangle, ImfFilter::Lanczos3] {
            assert!((filter.weight(0.0) - 1.0).abs() < 1e-6, "{filter:?}");
        }
        for filter in [
            ImfFilter::Hermite,
            ImfFilter::Box,
            ImfFilter::Triangle,
            ImfFilter::Bell,
            ImfFilter::BSpline,
            ImfFilter::Lanczos3,
            ImfFilter::Mitchell,
        ] {
            assert_eq!(filter.weight(filter.support() + 0.25), 0.0, "{filter:?}");
        }
        assert!((ImfFilter::Bell.weight(0.0) - 0.75).abs() < 1e-6);
        assert!((ImfFilter::BSpline.weight(0.0) - 2.0 / 3.0).abs() < 1e-6);
        assert!((ImfFilter::Mitchell.weight(0.0) - 16.0 / 18.0).abs() < 1e-6);
    }

    #[test]
    fn tiny_pictures_are_refused() {
        let one = Image::filled(1, 4, [0; 4]).unwrap();
        assert!(imf_resample(&one, 4, 4, ImfFilter::Box).is_err());
        let ok = Image::filled(4, 4, [0; 4]).unwrap();
        assert!(imf_resample(&ok, 1, 4, ImfFilter::Box).is_err());
        assert!(imf_resample(&ok, 4, 0, ImfFilter::Box).is_err());
        // a wide filter on a two pixel picture mirrors far outside it and still works
        let two = Image::filled(2, 2, [9, 9, 9, 9]).unwrap();
        assert!(imf_resample(&two, 2, 2, ImfFilter::Lanczos3).is_ok());
    }
}
