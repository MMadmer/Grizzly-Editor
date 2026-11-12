//! `xrImage_Filter.cpp`: a general kernel filter over float triples with wrapped edges, and the
//! Gaussian blur kernel. Nothing of the compilers calls it; it is kept with the light's image
//! tools.

use glam::Vec3;
use rayon::prelude::*;

/// `imk_blur_gauss`.
pub const BLUR_GAUSS: [[f32; 7]; 7] = [
    [0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0],
    [0.0, 5.0, 18.0, 32.0, 18.0, 5.0, 0.0],
    [0.0, 18.0, 64.0, 100.0, 64.0, 18.0, 0.0],
    [5.0, 32.0, 100.0, 100.0, 100.0, 32.0, 5.0],
    [0.0, 18.0, 64.0, 100.0, 64.0, 18.0, 0.0],
    [0.0, 5.0, 18.0, 32.0, 18.0, 5.0, 0.0],
    [0.0, 0.0, 0.0, 5.0, 0.0, 0.0, 0.0],
];

/// `imf_BuildKernel`: a `(2 * dim + 1)^2` kernel scaled to sum to `norm`.
#[must_use]
pub fn build_kernel(src: &[f32], norm: f32) -> Vec<f32> {
    let sum: f32 = src.iter().fold(0.0, |s, v| s + v);
    let scale = norm / sum;
    src.iter().map(|v| v * scale).collect()
}

/// `imf_ProcessKernel`: every pixel the kernel's weighted sum of its neighbours, the picture
/// wrapped at its edges. Rows run in parallel.
#[must_use]
pub fn process_kernel(src: &[Vec3], w: usize, h: usize, kern: &[f32], dim: usize) -> Vec<Vec3> {
    let mut out = vec![Vec3::ZERO; w * h];
    if w == 0 || h == 0 || src.len() < w * h {
        return out;
    }
    let d = dim as isize;
    out.par_chunks_mut(w).enumerate().for_each(|(y, row)| {
        for (x, dst) in row.iter_mut().enumerate() {
            let mut total = Vec3::ZERO;
            let mut k = kern.iter();
            for j in -d..=d {
                for i in -d..=d {
                    let wrap = |v: isize, n: usize| {
                        if v < 0 {
                            v + n as isize
                        } else if v >= n as isize {
                            v - n as isize
                        } else {
                            v
                        }
                    };
                    let x2 = wrap(x as isize + i, w) as usize;
                    let y2 = wrap(y as isize + j, h) as usize;
                    let kv = k.next().copied().unwrap_or(0.0);
                    let s = src.get(y2 * w + x2).copied().unwrap_or_default();
                    total = Vec3::new(total.x + s.x * kv, total.y + s.y * kv, total.z + s.z * kv);
                }
            }
            *dst = total;
        }
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_blur_keeps_a_flat_picture() {
        let kern = build_kernel(BLUR_GAUSS.as_flattened(), 1.0);
        let sum: f32 = kern.iter().sum();
        assert!((sum - 1.0).abs() < 1e-5);
        let src = vec![Vec3::splat(2.0); 16];
        let out = process_kernel(&src, 4, 4, &kern, 3);
        assert!(out.iter().all(|v| (v.x - 2.0).abs() < 1e-4));
    }
}
