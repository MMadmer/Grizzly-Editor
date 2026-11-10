//! Numbers as the C runtime prints them.
//!
//! The UCRT of Windows 10 2004 and later prints exactly representable values with IEEE
//! round-half-to-even, which is what Rust's fixed-precision formatting does on the exact binary
//! value: `{:.6}` of the `f64` a `float` promotes to IS `%f`.

/// `%f` of a `float`.
#[must_use]
pub fn f(v: f32) -> String {
    format!("{:.6}", f64::from(v))
}

/// `%f,%f,%f`.
#[must_use]
pub fn f3(v: glam::Vec3) -> String {
    format!("{},{},{}", f(v.x), f(v.y), f(v.z))
}

/// `%g` of a `float`: six significant digits, the shorter of fixed and exponent notation, trailing
/// zeros dropped.
#[must_use]
pub fn g(v: f32) -> String {
    let v = f64::from(v);
    if v.is_nan() {
        return if v.is_sign_negative() { "-nan(ind)".to_owned() } else { "nan".to_owned() };
    }
    if v.is_infinite() {
        return if v < 0.0 { "-inf".to_owned() } else { "inf".to_owned() };
    }
    if v == 0.0 {
        return if v.is_sign_negative() { "-0".to_owned() } else { "0".to_owned() };
    }
    // the exponent of the value rounded to six significant digits, as %e would print it
    let e = format!("{v:.5e}");
    let (mant, exp) = e.split_once('e').unwrap_or((&e, "0"));
    let x: i32 = exp.parse().unwrap_or(0);
    if (-4..6).contains(&x) {
        let decimals = usize::try_from(5 - x).unwrap_or(0);
        trim_zeros(&format!("{v:.decimals$}")).to_owned()
    } else {
        format!("{}e{}{:02}", trim_zeros(mant), if x < 0 { '-' } else { '+' }, x.unsigned_abs())
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.') } else { s }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_rounds_half_to_even() {
        assert_eq!(f(1.0), "1.000000");
        assert_eq!(f(-0.0), "-0.000000");
        assert_eq!(f(0.007_812_5), "0.007812");
        assert_eq!(f(12.5), "12.500000");
    }

    #[test]
    fn general_matches_printf() {
        assert_eq!(g(1.0), "1");
        assert_eq!(g(0.5), "0.5");
        assert_eq!(g(100_000.0), "100000");
        assert_eq!(g(1_000_000.0), "1e+06");
        assert_eq!(g(0.0001), "0.0001");
        assert_eq!(g(0.000_01), "1e-05");
        assert_eq!(g(0.3), "0.3");
        assert_eq!(g(123_456_789.0), "1.23457e+08");
    }
}
