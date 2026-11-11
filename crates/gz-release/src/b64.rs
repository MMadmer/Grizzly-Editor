//! The text forms of keys and signatures (MOD_CATALOG 3): base64url without padding, lower
//! case hex.

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// base64url without padding (`B64u`).
#[must_use]
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = chunk.get(1).copied().map(u32::from);
        let b2 = chunk.get(2).copied().map(u32::from);
        let v = (b0 << 16) | (b1.unwrap_or(0) << 8) | b2.unwrap_or(0);
        let digit =
            |shift: u32| char::from(ALPHABET.get(((v >> shift) & 63) as usize).copied().unwrap_or(b'A'));
        out.push(digit(18));
        out.push(digit(12));
        if b1.is_some() {
            out.push(digit(6));
        }
        if b2.is_some() {
            out.push(digit(0));
        }
    }
    out
}

/// Reads base64url or standard base64, padded or not, line breaks and spaces skipped
/// (`UnB64u`). `None` for any other character. Bits left over at the end are dropped.
#[must_use]
pub fn decode(text: &str) -> Option<Vec<u8>> {
    let mut out = Vec::with_capacity(text.len() * 3 / 4);
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in text.bytes() {
        let v = match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            b'=' | b' ' | b'\r' | b'\n' => continue,
            _ => return None,
        };
        buffer = (buffer << 6) | u32::from(v);
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push(((buffer >> bits) & 0xFF) as u8);
        }
        buffer &= 0xFFFF;
    }
    Some(out)
}

/// Lower case hex.
#[must_use]
pub fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push(char::from(DIGITS[usize::from(b >> 4)]));
        out.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips() {
        for n in 0..70usize {
            let data: Vec<u8> = (0..n).map(|i| (i * 37 + 11) as u8).collect();
            let text = encode(&data);
            assert!(!text.contains('='));
            assert_eq!(decode(&text).as_deref(), Some(&data[..]));
        }
        assert_eq!(encode(b"\xfb\xff"), "-_8");
        assert_eq!(decode("+/8=").as_deref(), Some(&b"\xfb\xff"[..]));
        assert_eq!(decode("ab\r\ncd").as_deref(), decode("abcd").as_deref());
        assert!(decode("ab*").is_none());
        assert_eq!(hex(&[0x00, 0xab, 0x7f]), "00ab7f");
    }
}
