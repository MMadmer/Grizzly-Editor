//! How the scene's values are spelled in its files: positions in metres as `{ x, y, z }`,
//! angles in degrees, colours as `{ r, g, b }` from 0 to 1, numbers as the shortest text that
//! reads back to the same value.

use gz_base::SmolStr;
use gz_luadata::{Table, Value};

use crate::math::Vec3;

/// Why a value of a file is not what its key takes: a sentence after the key.
pub type Bad = String;

/// A number; one the files cannot spell as a number (not a number, infinite) as its word.
#[must_use]
pub fn num(v: f32) -> Value {
    number(shortest(v))
}

/// The shortest decimal that reads back to the very same float, as the double the file then
/// writes (`-392.142578`, not the double's own `-392.142578125`).
#[must_use]
pub fn shortest(v: f32) -> f64 {
    let exact = f64::from(v);
    if !exact.is_finite() || exact.fract() == 0.0 {
        return exact;
    }
    for digits in 1..=9usize {
        if let Ok(d) = format!("{:.*e}", digits - 1, exact).parse::<f64>()
            && (d as f32).to_bits() == v.to_bits()
        {
            return d;
        }
    }
    exact
}

/// A number as the file spells it: `"-0"`, `"nan"`, `"inf"`, `"-inf"` for what a number of
/// the file cannot be (a file's `-0` reads back as 0).
#[must_use]
pub fn number(n: f64) -> Value {
    if n == 0.0 && n.is_sign_negative() {
        Value::String(SmolStr::new_static("-0"))
    } else if n.is_finite() {
        Value::Number(n)
    } else if n.is_nan() {
        Value::String(SmolStr::new_static("nan"))
    } else if n > 0.0 {
        Value::String(SmolStr::new_static("inf"))
    } else {
        Value::String(SmolStr::new_static("-inf"))
    }
}

/// The word of a number the files cannot spell as one.
#[must_use]
pub fn special(s: &str) -> Option<f64> {
    match s {
        "-0" => Some(-0.0),
        "nan" => Some(f64::NAN),
        "inf" => Some(f64::INFINITY),
        "-inf" => Some(f64::NEG_INFINITY),
        _ => None,
    }
}

#[must_use]
pub fn int(v: impl Into<f64>) -> Value {
    Value::Number(v.into())
}

#[must_use]
pub fn text(v: &str) -> Value {
    Value::String(SmolStr::new(v))
}

/// A list of values.
#[must_use]
pub fn list(items: impl IntoIterator<Item = Value>) -> Value {
    Value::Table(Table { arr: items.into_iter().collect(), fields: Vec::new() })
}

#[must_use]
pub fn vec3(v: Vec3) -> Value {
    list([num(v.x), num(v.y), num(v.z)])
}

/// The degrees of an angle in radians: the shortest that read back to the very same radians.
#[must_use]
pub fn degrees(r: f32) -> f64 {
    let exact = f64::from(r).to_degrees();
    if !exact.is_finite() {
        return exact;
    }
    for digits in 1..=17usize {
        let Ok(d) = format!("{:.*e}", digits - 1, exact).parse::<f64>() else { continue };
        if radians(d).to_bits() == r.to_bits() {
            return d;
        }
    }
    exact
}

/// Degrees as the radians the model holds.
#[must_use]
pub fn radians(d: f64) -> f32 {
    d.to_radians() as f32
}

#[must_use]
pub fn angle(r: f32) -> Value {
    number(degrees(r))
}

/// Euler angles in radians as degrees.
#[must_use]
pub fn angles(v: Vec3) -> Value {
    list([angle(v.x), angle(v.y), angle(v.z)])
}

/// `{ r, g, b }`, with the fourth component only when it is not 0.
#[must_use]
pub fn color(c: [f32; 4]) -> Value {
    let mut items = vec![num(c[0]), num(c[1]), num(c[2])];
    if c[3] != 0.0 {
        items.push(num(c[3]));
    }
    list(items)
}

/// An `0xAARRGGBB` colour as `{ r, g, b }` (the bytes over 255), alpha when it is not 0.
#[must_use]
pub fn argb(c: u32) -> Value {
    let f = |s: u32| ((c >> s) & 0xff) as f32 / 255.0;
    color([f(16), f(8), f(0), f(24)])
}

/// A hex string of bytes.
#[must_use]
pub fn hex(bytes: &[u8]) -> Value {
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push_str(&format!("{b:02x}"));
    }
    Value::String(SmolStr::new(s))
}

// ---------------------------------------------------------------- reading

pub fn as_f32(v: &Value) -> Result<f32, Bad> {
    as_f64(v).map(|n| n as f32)
}

pub fn as_f64(v: &Value) -> Result<f64, Bad> {
    match v {
        Value::Number(n) => Ok(*n),
        Value::String(s) => special(s).ok_or_else(|| "takes a number".into()),
        _ => Err("takes a number".into()),
    }
}

/// A whole number in `0..=max`.
pub fn as_uint(v: &Value, max: u64) -> Result<u64, Bad> {
    match v {
        Value::Number(n) if n.fract() == 0.0 && *n >= 0.0 && *n <= max as f64 => Ok(*n as u64),
        Value::Number(_) => Err(format!("takes a whole number from 0 to {max}")),
        _ => Err("takes a number".into()),
    }
}

pub fn as_u32(v: &Value) -> Result<u32, Bad> {
    as_uint(v, u64::from(u32::MAX)).map(|n| n as u32)
}

pub fn as_u16(v: &Value) -> Result<u16, Bad> {
    as_uint(v, u64::from(u16::MAX)).map(|n| n as u16)
}

pub fn as_u8(v: &Value) -> Result<u8, Bad> {
    as_uint(v, u64::from(u8::MAX)).map(|n| n as u8)
}

pub fn as_bool(v: &Value) -> Result<bool, Bad> {
    match v {
        Value::Bool(b) => Ok(*b),
        _ => Err("takes true or false".into()),
    }
}

pub fn as_text(v: &Value) -> Result<SmolStr, Bad> {
    match v {
        Value::String(s) => Ok(s.clone()),
        _ => Err("takes a string".into()),
    }
}

fn numbers<const N: usize>(v: &Value, what: &str) -> Result<[f64; N], Bad> {
    let t = v.as_table().ok_or_else(|| format!("takes {what}"))?;
    if t.arr.len() != N || !t.fields.is_empty() {
        return Err(format!("takes {what}"));
    }
    let mut out = [0.0; N];
    for (o, x) in out.iter_mut().zip(&t.arr) {
        *o = as_f64(x).map_err(|_| format!("takes {what}"))?;
    }
    Ok(out)
}

pub fn as_vec3(v: &Value) -> Result<Vec3, Bad> {
    let [x, y, z] = numbers::<3>(v, "{ x, y, z }")?;
    Ok(Vec3::new(x as f32, y as f32, z as f32))
}

/// `{ x, y, z }` in degrees, as radians.
pub fn as_angles(v: &Value) -> Result<Vec3, Bad> {
    let [x, y, z] = numbers::<3>(v, "{ x, y, z } in degrees")?;
    Ok(Vec3::new(radians(x), radians(y), radians(z)))
}

pub fn as_angle(v: &Value) -> Result<f32, Bad> {
    as_f64(v).map(radians)
}

pub fn as_color(v: &Value) -> Result<[f32; 4], Bad> {
    let t = v.as_table().ok_or("takes { r, g, b }")?;
    let n = t.arr.len();
    if !(n == 3 || n == 4) || !t.fields.is_empty() {
        return Err("takes { r, g, b }".into());
    }
    let mut c = [0.0f32; 4];
    for (o, x) in c.iter_mut().zip(&t.arr) {
        *o = x.as_number().ok_or("takes { r, g, b }")? as f32;
    }
    Ok(c)
}

/// `{ r, g, b[, a] }` as `0xAARRGGBB`: each component times 255, rounded.
pub fn as_argb(v: &Value) -> Result<u32, Bad> {
    let c = as_color(v)?;
    let b = |f: f32| (f.clamp(0.0, 1.0) * 255.0).round() as u32;
    Ok((b(c[3]) << 24) | (b(c[0]) << 16) | (b(c[1]) << 8) | b(c[2]))
}

pub fn as_hex(v: &Value) -> Result<Vec<u8>, Bad> {
    let s = as_text(v)?;
    let b = s.as_bytes();
    if b.len() % 2 != 0 {
        return Err("takes hex digits in pairs".into());
    }
    let digit = |c: u8| (c as char).to_digit(16);
    b.chunks(2)
        .map(|p| match (digit(p[0]), digit(p[1])) {
            (Some(h), Some(l)) => Ok((h * 16 + l) as u8),
            _ => Err("takes hex digits".to_owned()),
        })
        .collect()
}

/// A word of a fixed list, as its number.
pub fn as_word(v: &Value, words: &[(&str, u32)]) -> Result<u32, Bad> {
    let s = as_text(v)?;
    words.iter().find(|(w, _)| w.eq_ignore_ascii_case(&s)).map(|(_, n)| *n).ok_or_else(|| {
        format!(
            "takes one of {}",
            words.iter().map(|(w, _)| format!("\"{w}\"")).collect::<Vec<_>>().join(", ")
        )
    })
}

/// The word of a number of a fixed list; a number the list does not name stays a number.
#[must_use]
pub fn word(n: u32, words: &[(&str, u32)]) -> Value {
    words.iter().find(|(_, x)| *x == n).map_or_else(|| int(n), |(w, _)| text(w))
}

/// A word or, when the list does not name it, a number.
pub fn as_word_or_number(v: &Value, words: &[(&str, u32)]) -> Result<u32, Bad> {
    match v {
        Value::Number(_) => as_u32(v),
        _ => as_word(v, words),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // the X-Ray SDK's own values, short of the constants they stand for
    #[allow(clippy::approx_constant)]
    #[test]
    fn degrees_read_back_to_the_same_radians() {
        for r in [0.0f32, 1.570_796_4, -0.000_022, 3.141_592_7, 0.392_699, 1e-7, -2.5, 6.283_185] {
            let d = degrees(r);
            assert_eq!(radians(d).to_bits(), r.to_bits(), "{r} -> {d}");
        }
        assert_eq!(degrees(std::f32::consts::FRAC_PI_2), 90.0);
        assert_eq!(degrees(std::f32::consts::PI / 8.0), 22.5);
    }

    #[test]
    fn colours_and_hex() {
        assert_eq!(as_color(&color([1.0, 0.5, 0.0, 0.0])).unwrap(), [1.0, 0.5, 0.0, 0.0]);
        assert_eq!(as_argb(&argb(0x80ff_8000)).unwrap(), 0x80ff_8000);
        assert_eq!(as_hex(&hex(&[0, 0xab, 0x10])).unwrap(), vec![0, 0xab, 0x10]);
    }
}
