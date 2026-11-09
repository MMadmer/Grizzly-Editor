//! `atoi` and `atof` as the C runtime reads a prefix.

fn space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `atoi`: leading blanks, a sign, digits; saturated at the bounds of `int` as MSVC does.
pub(crate) fn atoi(s: &str) -> i32 {
    let b = s.as_bytes();
    let mut i = 0;
    while b.get(i).copied().is_some_and(space) {
        i += 1;
    }
    let mut neg = false;
    if let Some(&c @ (b'+' | b'-')) = b.get(i) {
        neg = c == b'-';
        i += 1;
    }
    let mut v: i64 = 0;
    while let Some(&d) = b.get(i).filter(|d| d.is_ascii_digit()) {
        v = v.saturating_mul(10).saturating_add(i64::from(d - b'0'));
        i += 1;
    }
    let v = if neg { -v } else { v };
    i32::try_from(v).unwrap_or(if v < 0 { i32::MIN } else { i32::MAX })
}

/// `atof`: the longest decimal prefix (`inf`, `nan` included), 0 when there is none.
pub(crate) fn atof(s: &str) -> f64 {
    let b = s.as_bytes();
    let mut i = 0;
    while b.get(i).copied().is_some_and(space) {
        i += 1;
    }
    let start = i;
    if let Some(b'+' | b'-') = b.get(i) {
        i += 1;
    }
    let neg = b.get(start) == Some(&b'-');
    let word = |w: &[u8]| b.get(i..i + w.len()).is_some_and(|x| x.eq_ignore_ascii_case(w));
    if word(b"inf") {
        return if neg { f64::NEG_INFINITY } else { f64::INFINITY };
    }
    if word(b"nan") {
        return f64::NAN;
    }
    let mut digits = 0;
    while b.get(i).is_some_and(u8::is_ascii_digit) {
        i += 1;
        digits += 1;
    }
    if b.get(i) == Some(&b'.') {
        i += 1;
        while b.get(i).is_some_and(u8::is_ascii_digit) {
            i += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return 0.0;
    }
    if let Some(b'e' | b'E') = b.get(i) {
        let mut j = i + 1;
        if let Some(b'+' | b'-') = b.get(j) {
            j += 1;
        }
        let e = j;
        while b.get(j).is_some_and(u8::is_ascii_digit) {
            j += 1;
        }
        if j > e {
            i = j;
        }
    }
    let text = s.get(start..i).unwrap_or("0");
    let text = text.strip_suffix('.').unwrap_or(text);
    text.parse().unwrap_or_else(|_| format!("{text}0").parse().unwrap_or(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_prefixes() {
        assert_eq!(atoi(" -12px"), -12);
        assert_eq!(atoi("x"), 0);
        assert_eq!(atoi("99999999999"), i32::MAX);
        assert_eq!(atof("1.5e2x"), 150.0);
        assert_eq!(atof(".5"), 0.5);
        assert_eq!(atof("5."), 5.0);
        assert_eq!(atof("-"), 0.0);
        assert_eq!(atof("1e"), 1.0);
    }
}
