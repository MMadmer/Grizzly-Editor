//! Little helpers over raw records: fixed offsets inside a slice whose length was checked.

use glam::Vec3;

use crate::{Aabb, LevelError, Result};

/// Checks that `count` records of `size` bytes fit into `left` bytes before anything is reserved.
pub(crate) fn fits(left: usize, count: u64, size: u64, what: &str) -> Result<usize> {
    let total = count.checked_mul(size).filter(|&t| t <= left as u64);
    match total {
        Some(_) => Ok(count as usize),
        None => Err(LevelError::Invalid(format!(
            "{what}: {count} records of {size} bytes do not fit into the {left} bytes that are left"
        ))),
    }
}

#[inline]
pub(crate) fn u16_at(b: &[u8], at: usize) -> u16 {
    let mut raw = [0u8; 2];
    if let Some(s) = b.get(at..at + 2) {
        raw.copy_from_slice(s);
    }
    u16::from_le_bytes(raw)
}

#[inline]
pub(crate) fn i16_at(b: &[u8], at: usize) -> i16 {
    u16_at(b, at) as i16
}

#[inline]
pub(crate) fn u32_at(b: &[u8], at: usize) -> u32 {
    let mut raw = [0u8; 4];
    if let Some(s) = b.get(at..at + 4) {
        raw.copy_from_slice(s);
    }
    u32::from_le_bytes(raw)
}

#[inline]
pub(crate) fn f32_at(b: &[u8], at: usize) -> f32 {
    f32::from_bits(u32_at(b, at))
}

#[inline]
pub(crate) fn vec3_at(b: &[u8], at: usize) -> Vec3 {
    Vec3::new(f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8))
}

#[inline]
pub(crate) fn aabb_at(b: &[u8], at: usize) -> Aabb {
    Aabb { min: vec3_at(b, at), max: vec3_at(b, at + 12) }
}

/// Every 12 bytes as a vector.
pub(crate) fn read_vec3s(bytes: &[u8]) -> Vec<Vec3> {
    bytes.chunks_exact(12).map(|c| vec3_at(c, 0)).collect()
}

#[inline]
pub(crate) fn put_u16(b: &mut [u8], at: usize, v: u16) {
    if let Some(s) = b.get_mut(at..at + 2) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

#[inline]
pub(crate) fn put_u32(b: &mut [u8], at: usize, v: u32) {
    if let Some(s) = b.get_mut(at..at + 4) {
        s.copy_from_slice(&v.to_le_bytes());
    }
}

#[inline]
pub(crate) fn put_f32(b: &mut [u8], at: usize, v: f32) {
    put_u32(b, at, v.to_bits());
}

#[inline]
pub(crate) fn put_vec3(b: &mut [u8], at: usize, v: Vec3) {
    put_f32(b, at, v.x);
    put_f32(b, at + 4, v.y);
    put_f32(b, at + 8, v.z);
}

/// Appends vectors, 12 bytes each.
pub(crate) fn push_vec3s(out: &mut Vec<u8>, items: &[Vec3]) {
    let start = out.len();
    out.resize(start + items.len() * 12, 0);
    if let Some(dst) = out.get_mut(start..) {
        for (rec, v) in dst.chunks_exact_mut(12).zip(items) {
            put_vec3(rec, 0, *v);
        }
    }
}

pub(crate) fn push_aabb(out: &mut Vec<u8>, b: &Aabb) {
    push_vec3s(out, &[b.min, b.max]);
}

/// The text of a fixed-size name field: up to the first zero byte.
pub(crate) fn fixed_str(b: &[u8]) -> String {
    let end = b.iter().position(|&c| c == 0).unwrap_or(b.len());
    gz_base::text::decode_guess(b.get(..end).unwrap_or_default()).into_owned()
}

/// A name into a fixed-size field: zero padded, cut so that a terminator always fits.
pub(crate) fn push_fixed_str(out: &mut Vec<u8>, text: &str, size: usize) {
    let bytes = text.as_bytes();
    let n = bytes.len().min(size.saturating_sub(1));
    out.extend_from_slice(bytes.get(..n).unwrap_or_default());
    out.resize(out.len() + (size - n), 0);
}
