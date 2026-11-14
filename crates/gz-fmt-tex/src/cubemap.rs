//! Texture Library, Cube Map \ Smaller Copy: a cube strip (six square faces side by side)
//! sampled down to 32 x 32 a face, each pixel the root mean square of a cone of directions of
//! the source.

use rayon::prelude::*;

use crate::{Image, Result, TexError};

/// The face side of the smaller copy.
pub const SMALL_SIDE: u32 = 32;
/// How wide a cone each pixel samples, against the pixel's own angle.
const SAMPLE_FACTOR: f32 = 16.0;
/// `EPS_S` of the engine.
const EPS_S: f32 = 0.000_000_1;
const PI_DIV_2: f32 = std::f32::consts::FRAC_PI_2;

type V3 = [f32; 3];

fn dot(a: V3, b: V3) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn normalize(v: V3) -> V3 {
    let l = dot(v, v).sqrt();
    if l > 0.0 { [v[0] / l, v[1] / l, v[2] / l] } else { v }
}

fn fis_zero(v: f32) -> bool {
    v.abs() < EPS_S
}

/// The heading and pitch of a direction.
fn get_hp(v: V3) -> (f32, f32) {
    let [x, y, z] = v;
    if fis_zero(x) && fis_zero(z) {
        let p = if fis_zero(y) {
            0.0
        } else if y > 0.0 {
            PI_DIV_2
        } else {
            -PI_DIV_2
        };
        return (0.0, p);
    }
    let h = if fis_zero(z) {
        if x > 0.0 { -PI_DIV_2 } else { PI_DIV_2 }
    } else if z < 0.0 {
        -((x / z).atan() - std::f32::consts::PI)
    } else {
        -(x / z).atan()
    };
    let hyp = (x * x + z * z).sqrt();
    let p = if fis_zero(hyp) { if y > 0.0 { PI_DIV_2 } else { -PI_DIV_2 } } else { (y / hyp).atan() };
    (h, p)
}

/// A direction from a heading and a pitch.
fn set_hp(h: f32, p: f32) -> V3 {
    let (sh, ch, sp, cp) = (h.sin(), h.cos(), p.sin(), p.cos());
    [-cp * sh, sp, cp * ch]
}

/// The six planes the faces lie on: `n` and `d`.
fn planes() -> [(V3, f32); 6] {
    let build = |p: V3, n: V3| {
        let n = normalize(n);
        (n, -dot(n, p))
    };
    [
        build([0.5, 0.0, 0.0], [-1.0, 0.0, 0.0]),
        build([-0.5, 0.0, 0.0], [1.0, 0.0, 0.0]),
        build([0.0, 0.5, 0.0], [0.0, -1.0, 0.0]),
        build([0.0, -0.5, 0.0], [0.0, 1.0, 0.0]),
        build([0.0, 0.0, 0.5], [0.0, 0.0, -1.0]),
        build([0.0, 0.0, -0.5], [0.0, 0.0, 1.0]),
    ]
}

/// `vector_from_point`: the direction through a pixel of a face.
fn vector_from_point(side: u32, x: u32, y: u32, width: u32, height: u32) -> V3 {
    let w = x as f32 / (width.saturating_sub(1).max(1)) as f32 * 2.0 - 1.0;
    let h = y as f32 / (height.saturating_sub(1).max(1)) as f32 * 2.0 - 1.0;
    let n = match side {
        0 => [1.0, -h, -w],
        1 => [-1.0, -h, w],
        2 => [w, 1.0, h],
        3 => [w, -1.0, -h],
        4 => [w, -h, 1.0],
        _ => [-w, -h, -1.0],
    };
    normalize(n)
}

/// `xys_from_vector`: the face and the pixel a direction meets.
fn xys_from_vector(planes: &[(V3, f32); 6], n: V3, width: u32, height: u32) -> (u32, u32, u32) {
    let mut t_dist = f32::MAX;
    let mut t_side = 0u32;
    for (side, (pn, pd)) in planes.iter().enumerate() {
        // a ray-distance test from the origin
        let denom = dot(*pn, n);
        if denom.abs() < EPS_S {
            continue;
        }
        let dist = -(pd / denom);
        if (dist > 0.0 || fis_zero(dist)) && dist < t_dist {
            t_dist = dist;
            t_side = side as u32;
        }
    }
    let c = [n[0] * t_dist, n[1] * t_dist, n[2] * t_dist];
    let (x, y) = match t_side {
        0 => (-c[2], -c[1]),
        1 => (c[2], -c[1]),
        2 | 3 => (c[0], c[2]),
        4 => (c[0], -c[1]),
        _ => (-c[0], -c[1]),
    };
    let (x, y) = (x.clamp(-0.5, 0.5), y.clamp(-0.5, 0.5));
    let ux = ((x + 0.5) * (width.saturating_sub(1)) as f32 + 0.5).floor().max(0.0) as u32;
    let uy = ((y + 0.5) * (height.saturating_sub(1)) as f32 + 0.5).floor().max(0.0) as u32;
    (ux, uy, t_side)
}

/// The byte offset of a pixel of a face in a strip of `width`-wide faces.
fn offset(width: u32, side: u32, x: u32, y: u32) -> usize {
    ((width * side + y * width * 6 + x) as usize) * 4
}

/// The smaller copy of a cube strip: `src` is six `h` x `h` faces side by side (`h` a power of
/// two, at least 32), the copy six 32 x 32 faces. Any other size is refused with
/// "Texture '<name>' - invalid size: [w, h]": the error carries the size.
pub fn simplify(src: &Image) -> Result<Image> {
    src.check()?;
    let (wf, h) = (src.width, src.height);
    let w = wf / 6;
    let small_wf = 6 * SMALL_SIDE;
    if !h.is_power_of_two() || u64::from(h) * 6 != u64::from(wf) || wf < small_wf || h < SMALL_SIDE {
        return Err(TexError::Invalid(format!("invalid size: [{wf}, {h}]")));
    }
    let planes = planes();
    let (dw, dh) = (SMALL_SIDE, SMALL_SIDE);
    let d_size = SAMPLE_FACTOR * w as f32 / dw as f32;
    let t_angle = SAMPLE_FACTOR * PI_DIV_2 / dw as f32;
    let d_angle = t_angle / d_size;
    let h_angle = t_angle / 2.0;
    let pixels: Vec<[u8; 4]> = (0..6 * dh * dw)
        .into_par_iter()
        .map(|i| {
            let side = i / (dh * dw);
            let y = (i / dw) % dh;
            let x = i % dw;
            let normal = vector_from_point(side, x, y, dw, dh);
            let (src_h, src_p) = get_hp(normal);
            let mut sum = [0.0f32; 4];
            let mut ds = 0u32;
            let mut p = src_p - h_angle;
            while p < src_p + h_angle {
                let mut hh = src_h - h_angle;
                while hh < src_h + h_angle {
                    let dir = set_hp(hh, p);
                    let (ux, uy, s) = xys_from_vector(&planes, dir, w, h);
                    let at = offset(w, s, ux.min(w - 1), uy.min(h - 1));
                    if let Some(px) = src.rgba.get(at..at + 4) {
                        for (acc, c) in sum.iter_mut().zip(px) {
                            let v = f32::from(*c) / 255.0;
                            *acc += v * v;
                        }
                    }
                    ds += 1;
                    hh += d_angle;
                }
                p += d_angle;
            }
            let mut out = [0u8; 4];
            if ds > 0 {
                for (o, s) in out.iter_mut().zip(sum) {
                    // `color_rgba_f`: floor, clamped
                    *o = ((s / ds as f32).sqrt() * 255.0).floor().clamp(0.0, 255.0) as u8;
                }
            }
            out
        })
        .collect();
    // the faces side by side: the pixel (side, x, y) of the strip
    let mut rgba = vec![0u8; (small_wf * dh * 4) as usize];
    for (i, px) in pixels.into_iter().enumerate() {
        let i = i as u32;
        let (side, y, x) = (i / (dh * dw), (i / dw) % dh, i % dw);
        let at = offset(dw, side, x, y);
        if let Some(slot) = rgba.get_mut(at..at + 4) {
            slot.copy_from_slice(&px);
        }
    }
    Image::new(small_wf, dh, rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_flat_cube_stays_flat() {
        let src = Image::filled(6 * 32, 32, [200, 100, 50, 255]);
        let small = src.and_then(|s| simplify(&s));
        let Ok(small) = small else { return };
        assert_eq!((small.width, small.height), (192, 32));
        // the RMS of one colour is that colour, floored
        assert!(small.rgba.chunks(4).all(|p| (i32::from(p[0]) - 200).abs() <= 1 && p[3] >= 254));
    }

    #[test]
    fn other_sizes_are_refused() {
        let src = Image::filled(100, 32, [0, 0, 0, 255]);
        assert!(src.is_ok_and(|s| simplify(&s).is_err()));
    }

    #[test]
    fn directions_meet_their_face() {
        let pl = planes();
        assert_eq!(xys_from_vector(&pl, [1.0, 0.0, 0.0], 32, 32).2, 0);
        assert_eq!(xys_from_vector(&pl, [0.0, -1.0, 0.0], 32, 32).2, 3);
        assert_eq!(xys_from_vector(&pl, [0.0, 0.0, -1.0], 32, 32).2, 5);
    }
}
