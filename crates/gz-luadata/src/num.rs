//! Numbers the way the game reads and prints them: LuaJIT's scanner for a literal and for
//! a string used in arithmetic, C's `strtod` for a typed read, C's `%g` for the writer.

/// What LuaJIT's number scanner (`lj_strscan_scan`) made of a text.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Scanned {
    Num(f64),
    /// `1LL`: an FFI 64-bit integer.
    I64(i64),
    /// `1ULL`.
    U64(u64),
    /// `1i`: an FFI complex number, the imaginary part.
    Imag(f64),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Fmt {
    Num,
    Imag,
    Int,
    U32,
    I64,
    U64,
}

fn is_space(c: u8) -> bool {
    matches!(c, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
}

/// `casecmp` of LuaJIT: equal ignoring the case bit.
fn casecmp(c: u8, k: u8) -> bool {
    (c ^ k) & 0xdf == 0
}

/// `ldexp` without an intermediate overflow.
fn scale2(mut n: f64, mut ex: i32) -> f64 {
    while ex > 1000 {
        n *= 2f64.powi(1000);
        ex -= 1000;
    }
    while ex < -1000 {
        n *= 2f64.powi(-1000);
        ex += 1000;
    }
    n * 2f64.powi(ex)
}

/// `strscan_double`: a 64-bit mantissa and a binary exponent as a double, rounded once.
fn mantissa_double(mut x: u64, mut ex2: i32, neg: bool) -> f64 {
    if x & 0xc000_0000_0000_0000 != 0 {
        x = (x >> 2) | (x & 3);
        ex2 += 2;
    }
    if ex2 <= -1075 && x != 0 {
        let b = 63 - x.leading_zeros() as i32;
        if b + ex2 <= -1023 && b + ex2 >= -1075 {
            let shift = (-1075 - ex2) as u32;
            if shift < 62 {
                let rb = 1u64 << shift;
                if x & rb != 0 && x & (rb + rb + rb - 1) != 0 {
                    x = x.wrapping_add(rb + rb);
                }
                x &= !(rb + rb - 1);
            }
        }
    }
    let n = x as i64 as f64;
    scale2(if neg { -n } else { n }, ex2)
}

/// LuaJIT's `lj_strscan_scan`. `cdata` allows the FFI suffixes `LL`, `ULL` and `i`, as the
/// lexer does; a string in arithmetic is scanned without them.
pub(crate) fn strscan(p: &[u8], cdata: bool) -> Option<Scanned> {
    let at = |i: usize| -> u8 { p.get(i).copied().unwrap_or(0) };
    let pe = p.len();
    let mut i = 0usize;
    let mut neg = false;

    if !at(0).is_ascii_digit() {
        while is_space(at(i)) {
            i += 1;
        }
        if at(i) == b'+' || at(i) == b'-' {
            neg = at(i) == b'-';
            i += 1;
        }
        if at(i) >= b'A' {
            let mut v = f64::NAN;
            if casecmp(at(i), b'i') && casecmp(at(i + 1), b'n') && casecmp(at(i + 2), b'f') {
                v = if neg { f64::NEG_INFINITY } else { f64::INFINITY };
                i += 3;
                if casecmp(at(i), b'i')
                    && casecmp(at(i + 1), b'n')
                    && casecmp(at(i + 2), b'i')
                    && casecmp(at(i + 3), b't')
                    && casecmp(at(i + 4), b'y')
                {
                    i += 5;
                }
            } else if casecmp(at(i), b'n') && casecmp(at(i + 1), b'a') && casecmp(at(i + 2), b'n') {
                i += 3;
            }
            while is_space(at(i)) {
                i += 1;
            }
            if at(i) != 0 || i < pe {
                return None;
            }
            return Some(Scanned::Num(v));
        }
    }

    let mut fmt = Fmt::Int;
    let (mut hex, mut bin) = (false, false);
    let mut dp: Option<usize> = None;
    let mut hasdig = false;
    if at(i) <= b'0' {
        if at(i) == b'0' {
            if casecmp(at(i + 1), b'x') {
                hex = true;
                i += 2;
            } else if casecmp(at(i + 1), b'b') {
                bin = true;
                i += 2;
            }
        }
        loop {
            if at(i) == b'0' {
                hasdig = true;
            } else if at(i) == b'.' {
                if dp.is_some() {
                    return None;
                }
                dp = Some(i);
            } else {
                break;
            }
            i += 1;
        }
    }
    let sp = i;
    let mut dig = 0usize;
    loop {
        let c = at(i);
        if (hex && c.is_ascii_hexdigit()) || (!hex && c.is_ascii_digit()) {
            dig += 1;
        } else if c == b'.' {
            if dp.is_some() {
                return None;
            }
            dp = Some(i);
        } else {
            break;
        }
        i += 1;
    }
    if !hasdig && dig == 0 {
        return None;
    }

    let mut ex: i32 = 0;
    if let Some(d) = dp {
        if bin {
            return None;
        }
        fmt = Fmt::Num;
        if dig > 0 {
            let last = i - 1;
            ex = d as i32 - last as i32;
            let mut q = last;
            while ex < 0 && at(q) == b'0' {
                q = q.wrapping_sub(1);
                ex += 1;
                dig -= 1;
            }
            if hex {
                ex *= 4;
            }
        }
    }
    if !bin && casecmp(at(i), if hex { b'p' } else { b'e' }) {
        fmt = Fmt::Num;
        i += 1;
        let mut negx = false;
        if at(i) == b'+' || at(i) == b'-' {
            negx = at(i) == b'-';
            i += 1;
        }
        if !at(i).is_ascii_digit() {
            return None;
        }
        let mut xx = u32::from(at(i) & 15);
        i += 1;
        while at(i).is_ascii_digit() {
            if xx < 65536 {
                xx = xx * 10 + u32::from(at(i) & 15);
            }
            i += 1;
        }
        ex += if negx { -(xx as i32) } else { xx as i32 };
    }
    if at(i) != 0 {
        if casecmp(at(i), b'i') {
            if !cdata {
                return None;
            }
            i += 1;
            fmt = Fmt::Imag;
        } else if fmt == Fmt::Int {
            if casecmp(at(i), b'u') {
                i += 1;
                fmt = Fmt::U32;
            }
            if casecmp(at(i), b'l') {
                i += 1;
                if !casecmp(at(i), b'l') {
                    return None;
                }
                i += 1;
                fmt = if fmt == Fmt::U32 { Fmt::U64 } else { Fmt::I64 };
            }
            if casecmp(at(i), b'u') && (fmt == Fmt::Int || fmt == Fmt::I64) {
                i += 1;
                fmt = if fmt == Fmt::Int { Fmt::U32 } else { Fmt::U64 };
            }
            if fmt == Fmt::U32 || (matches!(fmt, Fmt::I64 | Fmt::U64) && !cdata) {
                return None;
            }
        }
        while is_space(at(i)) {
            i += 1;
        }
        if at(i) != 0 {
            return None;
        }
    }
    if i < pe {
        return None;
    }

    // the significant digits, without the point
    let digits: Vec<u8> =
        p.get(sp..).unwrap_or(&[]).iter().copied().filter(|c| *c != b'.').take(dig).collect();
    let integer = matches!(fmt, Fmt::I64 | Fmt::U64);
    let wrap = |x: u64| -> Scanned {
        let x = if neg { x.wrapping_neg() } else { x };
        if fmt == Fmt::U64 { Scanned::U64(x) } else { Scanned::I64(x as i64) }
    };
    let float = |n: f64| -> Scanned { if fmt == Fmt::Imag { Scanned::Imag(n) } else { Scanned::Num(n) } };

    if hex {
        let mut x: u64 = 0;
        let mut ex2 = ex;
        for (n, &c) in digits.iter().enumerate() {
            if n < 16 {
                let d = if c > b'9' { u64::from(c).wrapping_add(9) } else { u64::from(c) };
                x = (x << 4) + (d & 15);
            } else {
                x |= u64::from(c != b'0');
                ex2 += 4;
            }
        }
        if integer {
            return if dig > 16 { None } else { Some(wrap(x)) };
        }
        return Some(float(mantissa_double(x, ex2, neg)));
    }
    if bin {
        if ex != 0 || dig > 64 {
            return None;
        }
        let mut x: u64 = 0;
        for &c in &digits {
            if c & !1 != b'0' {
                return None;
            }
            x = (x << 1) | u64::from(c & 1);
        }
        if integer {
            return Some(wrap(x));
        }
        return Some(float(mantissa_double(x, 0, neg)));
    }
    if integer {
        if dig > 20 {
            return None;
        }
        let mut x: u128 = 0;
        for &c in &digits {
            x = x * 10 + u128::from(c & 15);
        }
        if x > u128::from(u64::MAX) {
            return None;
        }
        return Some(wrap(x as u64));
    }
    let n = if digits.is_empty() {
        0.0
    } else {
        // the digits are ASCII, so this is always text
        let mut text = String::with_capacity(digits.len() + 8);
        text.extend(digits.iter().map(|&c| char::from(c)));
        text.push('e');
        text.push_str(&ex.to_string());
        text.parse::<f64>().unwrap_or(0.0)
    };
    Some(float(if neg { -n } else { n }))
}

/// A string as Lua's arithmetic reads it (`lj_strscan_num`): a number with nothing but spaces
/// around it.
pub(crate) fn lua_tonumber(text: &[u8]) -> Option<f64> {
    match strscan(text, false) {
        Some(Scanned::Num(n)) => Some(n),
        _ => None,
    }
}

/// C's `strtod` over a whole string: leading spaces are skipped, anything after the number
/// makes it no number.
pub(crate) fn strtod_whole(text: &str) -> Option<f64> {
    let body = text.trim_start_matches(|c: char| c.is_ascii() && is_space(c as u8));
    if body.is_empty() || body.ends_with(|c: char| c.is_ascii() && is_space(c as u8)) {
        return None;
    }
    let unsigned = body.strip_prefix(['+', '-']).unwrap_or(body);
    if unsigned.len() > 2 && unsigned.as_bytes()[0] == b'0' && casecmp(unsigned.as_bytes()[1], b'x') {
        return lua_tonumber(body.as_bytes());
    }
    body.parse::<f64>().ok()
}

/// C's `%.<precision>g` as the Universal CRT prints it.
#[must_use]
pub fn format_g(value: f64, precision: usize) -> String {
    if value.is_nan() {
        return (if value.is_sign_negative() { "-nan(ind)" } else { "nan" }).to_owned();
    }
    if value.is_infinite() {
        return (if value < 0.0 { "-inf" } else { "inf" }).to_owned();
    }
    let p = precision.max(1);
    if value == 0.0 {
        return (if value.is_sign_negative() { "-0" } else { "0" }).to_owned();
    }
    let sci = format!("{:.*e}", p - 1, value);
    let (mantissa, exponent) = sci.split_once('e').unwrap_or((sci.as_str(), "0"));
    let x: i32 = exponent.parse().unwrap_or(0);
    let strip = |s: &str| -> String {
        if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s.to_owned() }
    };
    if x < -4 || x >= p as i32 {
        let mut out = strip(mantissa);
        out.push('e');
        out.push(if x < 0 { '-' } else { '+' });
        out.push_str(&format!("{:02}", x.unsigned_abs()));
        out
    } else {
        let decimals = (p as i32 - 1 - x).max(0) as usize;
        strip(&format!("{value:.decimals$}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn num(text: &str) -> Option<f64> {
        match strscan(text.as_bytes(), true) {
            Some(Scanned::Num(n)) => Some(n),
            _ => None,
        }
    }

    #[test]
    fn scans_what_luajit_scans() {
        assert_eq!(num("0"), Some(0.0));
        assert_eq!(num("12"), Some(12.0));
        assert_eq!(num("1.50"), Some(1.5));
        assert_eq!(num("0.05"), Some(0.05));
        assert_eq!(num(".5"), Some(0.5));
        assert_eq!(num("5."), Some(5.0));
        assert_eq!(num("1e3"), Some(1000.0));
        assert_eq!(num("1E-2"), Some(0.01));
        assert_eq!(num("0x10"), Some(16.0));
        assert_eq!(num("0xA.8p1"), Some(21.0));
        assert_eq!(num("0b101"), Some(5.0));
        assert_eq!(num("1e999"), Some(f64::INFINITY));
        assert_eq!(num("  -7  "), Some(-7.0));
        assert_eq!(num("inf"), Some(f64::INFINITY));
        assert_eq!(num("-Infinity"), Some(f64::NEG_INFINITY));
        assert!(num("nan").is_some_and(f64::is_nan));
        assert_eq!(num("1..2"), None);
        assert_eq!(num("1e"), None);
        assert_eq!(num("1abc"), None);
        assert_eq!(num("0b12"), None);
        assert_eq!(num("."), None);
        assert_eq!(num(""), None);
        assert_eq!(num("1 2"), None);
        assert_eq!(strscan(b"3LL", true), Some(Scanned::I64(3)));
        assert_eq!(strscan(b"3ull", true), Some(Scanned::U64(3)));
        assert_eq!(strscan(b"2i", true), Some(Scanned::Imag(2.0)));
        assert_eq!(strscan(b"3LL", false), None);
        assert_eq!(strscan(b"3u", true), None);
        assert_eq!(strscan(b"3L", true), None);
    }

    #[test]
    fn strtod_takes_whole_strings_only() {
        assert_eq!(strtod_whole("12.5"), Some(12.5));
        assert_eq!(strtod_whole("\t-3"), Some(-3.0));
        assert_eq!(strtod_whole("3 "), None);
        assert_eq!(strtod_whole("3x"), None);
        assert_eq!(strtod_whole("0x1p4"), Some(16.0));
        assert_eq!(strtod_whole(""), None);
    }

    #[test]
    fn prints_like_printf_g() {
        assert_eq!(format_g(0.1, 7), "0.1");
        assert_eq!(format_g(12.5, 7), "12.5");
        assert_eq!(format_g(1234567.0, 7), "1234567");
        assert_eq!(format_g(12345678.0, 7), "1.234568e+07");
        assert_eq!(format_g(0.0001, 7), "0.0001");
        assert_eq!(format_g(0.00001, 7), "1e-05");
        assert_eq!(format_g(1e20, 15), "1e+20");
        assert_eq!(format_g(0.1, 17), "0.10000000000000001");
        assert_eq!(format_g(1.0 / 3.0, 15), "0.333333333333333");
        assert_eq!(format_g(-2.5e-7, 15), "-2.5e-07");
        assert_eq!(format_g(1e100, 15), "1e+100");
        assert_eq!(format_g(f64::INFINITY, 7), "inf");
    }
}
