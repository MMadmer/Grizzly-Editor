//! The light of a point (`base_color_c`) and its packed form (`base_color`, six `fixed16<16>`).

use glam::Vec3;

use crate::math::{i_floor, lerp, max_f, max3};

/// `base_color_c`: the static light (`rgb`), the hemisphere, the sun and a spare channel the
/// vertex light keeps its translucency in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ColorC {
    pub rgb: Vec3,
    pub hemi: f32,
    pub sun: f32,
    pub tmp: f32,
}

impl ColorC {
    /// `mul`: every channel but `tmp`.
    pub fn mul(&mut self, s: f32) {
        self.rgb = Vec3::new(self.rgb.x * s, self.rgb.y * s, self.rgb.z * s);
        self.hemi *= s;
        self.sun *= s;
    }

    /// `add(base_color_c&)`.
    pub fn add(&mut self, o: &ColorC) {
        self.rgb = Vec3::new(self.rgb.x + o.rgb.x, self.rgb.y + o.rgb.y, self.rgb.z + o.rgb.z);
        self.hemi += o.hemi;
        self.sun += o.sun;
    }

    /// `scale(samples)`: `mul(1 / samples)`.
    pub fn scale(&mut self, samples: u32) {
        self.mul(1.0 / samples as f32);
    }

    /// `max`.
    pub fn max(&mut self, o: &ColorC) {
        self.rgb = max3(self.rgb, o.rgb);
        self.hemi = max_f(self.hemi, o.hemi);
        self.sun = max_f(self.sun, o.sun);
    }

    /// `lerp(A, B, s)`: `tmp` is left as it is.
    pub fn lerp(&mut self, a: &ColorC, b: &ColorC, s: f32) {
        self.rgb = lerp(a.rgb, b.rgb, s);
        let is = 1.0 - s;
        self.hemi = is * a.hemi + s * b.hemi;
        self.sun = is * a.sun + s * b.sun;
    }

    /// The five channels as `float*` indexes them: `r, g, b, hemi, sun`.
    #[must_use]
    pub fn channel(&self, i: usize) -> f32 {
        match i {
            0 => self.rgb.x,
            1 => self.rgb.y,
            2 => self.rgb.z,
            3 => self.hemi,
            4 => self.sun,
            _ => self.tmp,
        }
    }

    pub fn set_channel(&mut self, i: usize, v: f32) {
        match i {
            0 => self.rgb.x = v,
            1 => self.rgb.y = v,
            2 => self.rgb.z = v,
            3 => self.hemi = v,
            4 => self.sun = v,
            _ => self.tmp = v,
        }
    }
}

/// `fixed16<16>::_w`.
#[must_use]
pub fn fixed16_w(a: f32) -> i16 {
    i_floor(a * 32767.0 / 16.0).clamp(-32768, 32767) as i16
}

/// `fixed16<16>::_r`.
#[must_use]
pub fn fixed16_r(v: i16) -> f32 {
    (f32::from(v) / 32767.0) * 16.0
}

/// `base_color`: a [`ColorC`] in twelve bytes, each channel a signed 16-bit fixed point of
/// range 16.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BaseColor {
    pub r: i16,
    pub g: i16,
    pub b: i16,
    pub h: i16,
    pub s: i16,
    pub t: i16,
}

impl BaseColor {
    /// `_set(base_color_c&)`.
    #[must_use]
    pub fn from_c(c: &ColorC) -> Self {
        Self {
            r: fixed16_w(c.rgb.x),
            g: fixed16_w(c.rgb.y),
            b: fixed16_w(c.rgb.z),
            h: fixed16_w(c.hemi),
            s: fixed16_w(c.sun),
            t: fixed16_w(c.tmp),
        }
    }

    /// `_get`.
    #[must_use]
    pub fn get(&self) -> ColorC {
        ColorC {
            rgb: Vec3::new(fixed16_r(self.r), fixed16_r(self.g), fixed16_r(self.b)),
            hemi: fixed16_r(self.h),
            sun: fixed16_r(self.s),
            tmp: fixed16_r(self.t),
        }
    }

    /// `_set(rgb, hemi, sun)`: `t` is kept.
    pub fn set_rgb_hemi_sun(&mut self, rgb: Vec3, hemi: f32, sun: f32) {
        self.r = fixed16_w(rgb.x);
        self.g = fixed16_w(rgb.y);
        self.b = fixed16_w(rgb.z);
        self.h = fixed16_w(hemi);
        self.s = fixed16_w(sun);
    }

    /// The raw twelve bytes, as the C++ writes the struct.
    #[must_use]
    pub fn to_bytes(&self) -> [u8; 12] {
        let mut out = [0u8; 12];
        for (dst, v) in
            out.as_chunks_mut::<2>().0.iter_mut().zip([self.r, self.g, self.b, self.h, self.s, self.t])
        {
            dst.copy_from_slice(&v.to_le_bytes());
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_packed_color_keeps_four_decimals() {
        // 1.0 * 32767 / 16 = 2047.94 -> 2047 -> 2047 / 32767 * 16 = 0.99954
        let c = ColorC { rgb: Vec3::new(1.0, 0.5, 0.0), hemi: 0.25, sun: 2.0, tmp: 0.5 };
        let p = BaseColor::from_c(&c);
        assert_eq!(p.r, 2047);
        assert_eq!(p.s, 4095);
        let back = p.get();
        assert!((back.rgb.x - 0.999_54).abs() < 1e-5);
        assert!((back.tmp - 0.5).abs() < 1e-3);
        // the range clips at 16
        assert_eq!(fixed16_w(100.0), 32767);
        assert_eq!(fixed16_w(-100.0), -32768);
    }
}
