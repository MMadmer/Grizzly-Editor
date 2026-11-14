//! The engine's arithmetic the formats depend on: its floor, its compressed normal, its box.

use glam::Vec3;

/// `EPS_S` of the engine.
pub const EPS_S: f32 = 0.000_000_1;
/// `EPS` of the engine.
pub const EPS: f32 = 0.000_01;
/// `EPS_L` of the engine.
pub const EPS_L: f32 = 0.001;

/// The engine's floor: a bit trick, kept bit for bit so that values outside `i32` and NaN land
/// where the X-Ray SDK puts them. Shifts mask their count to five bits, as x86 does.
#[must_use]
pub fn i_floor(x: f32) -> i32 {
    let a = x.to_bits() as i32;
    let mut exponent = (127 + 31) - ((a >> 23) & 0xFF);
    let mut r = ((((a as u32) << 8) | (1u32 << 31)).wrapping_shr(exponent as u32)) as i32;
    exponent += 31 - 127;
    let imask = i32::from(((1i32.wrapping_shl(exponent as u32).wrapping_sub(1)) >> 8) & a == 0);
    exponent -= (31 - 127) + 32;
    exponent >>= 31;
    let a = a >> 31;
    r = r.wrapping_sub(imask & a);
    r &= exponent;
    r ^= a;
    r
}

/// The engine's ceiling, the same trick.
#[must_use]
pub fn i_ceil(x: f32) -> i32 {
    let a = x.to_bits() as i32;
    let mut exponent = (127 + 31) - ((a >> 23) & 0xFF);
    let mut r = ((((a as u32) << 8) | (1u32 << 31)).wrapping_shr(exponent as u32)) as i32;
    exponent += 31 - 127;
    let imask = i32::from(((1i32.wrapping_shl(exponent as u32).wrapping_sub(1)) >> 8) & a == 0);
    exponent -= (31 - 127) + 32;
    exponent >>= 31;
    let a = !(a.wrapping_sub(1) >> 31);
    r = r.wrapping_sub(imask & a);
    r &= exponent;
    r ^= a;
    r.wrapping_neg()
}

const PV_XSIGN: u16 = 0x8000;
const PV_YSIGN: u16 = 0x4000;
const PV_ZSIGN: u16 = 0x2000;
const PV_TOP: u16 = 0x1f80;
const PV_BOTTOM: u16 = 0x007f;

/// A direction in 16 bits (three signs, 13 bits of position on the octant).
#[must_use]
pub fn pv_compress(vec: Vec3) -> u16 {
    let mut packed = 0u16;
    let (mut x, mut y, mut z) = (vec.x, vec.y, vec.z);
    if x.is_sign_negative() {
        packed |= PV_XSIGN;
        x = x.abs();
    }
    if y.is_sign_negative() {
        packed |= PV_YSIGN;
        y = y.abs();
    }
    if z.is_sign_negative() {
        packed |= PV_ZSIGN;
        z = z.abs();
    }
    let w = 126.0 / (x + y + z);
    let mut xbits = i_floor(x * w);
    let mut ybits = i_floor(y * w);
    if xbits >= 64 {
        xbits = 127 - xbits;
        ybits = 127 - ybits;
    }
    packed |= xbits.wrapping_shl(7) as u16;
    packed |= ybits as u16;
    packed
}

/// The unit direction of a [`pv_compress`] value.
#[must_use]
pub fn pv_decompress(packed: u16) -> Vec3 {
    let mut xbits = i32::from((packed & PV_TOP) >> 7);
    let mut ybits = i32::from(packed & PV_BOTTOM);
    if xbits + ybits >= 127 {
        xbits = 127 - xbits;
        ybits = 127 - ybits;
    }
    let (fx, fy, fz) = (xbits as f32, ybits as f32, (126 - xbits - ybits) as f32);
    // the engine's lookup table holds exactly this value for every index
    let adjust = 1.0 / (fy * fy + fz * fz + fx * fx).sqrt();
    let mut out = Vec3::new(adjust * fx, adjust * fy, adjust * fz);
    if packed & PV_XSIGN != 0 {
        out.x = -out.x.abs();
    }
    if packed & PV_YSIGN != 0 {
        out.y = -out.y.abs();
    }
    if packed & PV_ZSIGN != 0 {
        out.z = -out.z.abs();
    }
    out
}

/// Normalizes a vector: scaled by `sqrt(1 / |v|^2)`, in that order of operations.
#[must_use]
pub fn normalize(v: Vec3) -> Vec3 {
    let mag = (1.0 / (v.x * v.x + v.y * v.y + v.z * v.z)).sqrt();
    Vec3::new(v.x * mag, v.y * mag, v.z * mag)
}

/// The dot product, summed left to right.
#[must_use]
pub fn dot(a: Vec3, b: Vec3) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z
}

/// Every component closer than `eps`.
#[must_use]
pub fn similar(a: Vec3, b: Vec3, eps: f32) -> bool {
    (a.x - b.x).abs() < eps && (a.y - b.y).abs() < eps && (a.z - b.z).abs() < eps
}

#[inline]
fn min_f(a: f32, b: f32) -> f32 {
    if a < b { a } else { b }
}

#[inline]
fn max_f(a: f32, b: f32) -> f32 {
    if a > b { a } else { b }
}

/// An axis-aligned box with the engine's box behaviour.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aabb {
    pub min: Vec3,
    pub max: Vec3,
}

impl Default for Aabb {
    fn default() -> Self {
        Self::INVALID
    }
}

impl Aabb {
    /// Holds nothing; the first [`Self::modify`] makes it a point.
    pub const INVALID: Aabb = Aabb { min: Vec3::splat(f32::MAX), max: Vec3::splat(-f32::MAX) };

    #[must_use]
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    /// From a centre and half extents.
    #[must_use]
    pub fn from_center(center: Vec3, half: Vec3) -> Self {
        Self {
            min: Vec3::new(center.x - half.x, center.y - half.y, center.z - half.z),
            max: Vec3::new(center.x + half.x, center.y + half.y, center.z + half.z),
        }
    }

    /// The box of some points.
    #[must_use]
    pub fn from_points<'a>(points: impl IntoIterator<Item = &'a Vec3>) -> Self {
        let mut out = Self::INVALID;
        for p in points {
            out.modify(*p);
        }
        out
    }

    /// Grows to hold a point.
    pub fn modify(&mut self, p: Vec3) {
        self.min = Vec3::new(min_f(self.min.x, p.x), min_f(self.min.y, p.y), min_f(self.min.z, p.z));
        self.max = Vec3::new(max_f(self.max.x, p.x), max_f(self.max.y, p.y), max_f(self.max.z, p.z));
    }

    /// Grows to hold another box.
    pub fn merge(&mut self, other: &Aabb) {
        self.modify(other.min);
        self.modify(other.max);
    }

    /// Grows a box.
    pub fn grow(&mut self, by: f32) {
        self.min = Vec3::new(self.min.x - by, self.min.y - by, self.min.z - by);
        self.max = Vec3::new(self.max.x + by, self.max.y + by, self.max.z + by);
    }

    /// The centre of a box.
    #[must_use]
    pub fn center(&self) -> Vec3 {
        Vec3::new(
            (self.min.x + self.max.x) * 0.5,
            (self.min.y + self.max.y) * 0.5,
            (self.min.z + self.max.z) * 0.5,
        )
    }

    /// Half extents the way the editor takes them: `max - center`.
    #[must_use]
    pub fn half_extents(&self) -> Vec3 {
        let c = self.center();
        Vec3::new(self.max.x - c.x, self.max.y - c.y, self.max.z - c.z)
    }

    /// Whether the box holds at least a point.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn floor_and_ceil_agree_with_the_library_on_ordinary_values() {
        // -0.0 is left out on purpose: the trick reads its sign bit and answers -1 / 1
        for &v in
            &[0.0f32, 0.5, -0.5, 1.0, -1.0, 1.5, -1.5, 2.999, -2.999, 126.0, 65535.4, -32767.2, 1e6, -1e6]
        {
            assert_eq!(i_floor(v), v.floor() as i32, "floor of {v}");
            assert_eq!(i_ceil(v), v.ceil() as i32, "ceil of {v}");
        }
    }

    #[test]
    fn a_compressed_normal_comes_back_close() {
        for n in [Vec3::Y, Vec3::NEG_Y, Vec3::new(0.3, 0.9, -0.2), Vec3::new(-0.7, 0.1, 0.7)] {
            let unit = n.normalize();
            let back = pv_decompress(pv_compress(unit));
            assert!((back - unit).length() < 0.03, "{unit:?} came back as {back:?}");
            assert!((back.length() - 1.0).abs() < 1e-4);
        }
        // straight up packs to "y = 126 of 126" and no sign
        assert_eq!(pv_compress(Vec3::Y), 126);
        assert_eq!(pv_compress(Vec3::NEG_Y) & 0x4000, 0x4000);
    }

    #[test]
    fn the_box_starts_empty_and_grows() {
        let mut b = Aabb::INVALID;
        assert!(!b.is_valid());
        b.modify(Vec3::new(1.0, 2.0, 3.0));
        b.modify(Vec3::new(-1.0, 4.0, 0.0));
        assert_eq!(b.min, Vec3::new(-1.0, 2.0, 0.0));
        assert_eq!(b.max, Vec3::new(1.0, 4.0, 3.0));
        assert_eq!(b.center(), Vec3::new(0.0, 3.0, 1.5));
        let again = Aabb::from_center(b.center(), b.half_extents());
        assert_eq!(again, b);
    }
}
