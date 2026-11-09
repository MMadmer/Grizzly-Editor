//! Text at the border with the game: the editor is UTF-8, the game's files are cp1251.
//! Every conversion is explicit; nothing goes through the system code page.

use std::borrow::Cow;

use encoding_rs::WINDOWS_1251;

/// Decodes cp1251 bytes. Every byte maps to a character, so this cannot fail.
#[must_use]
pub fn cp1251_to_utf8(bytes: &[u8]) -> Cow<'_, str> {
    WINDOWS_1251.decode_without_bom_handling(bytes).0
}

/// Encodes for the game. The flag says a character had no cp1251 form and became `?`.
#[must_use]
pub fn utf8_to_cp1251(text: &str) -> (Cow<'_, [u8]>, bool) {
    if text.is_ascii() {
        return (Cow::Borrowed(text.as_bytes()), false);
    }
    let mut out = Vec::with_capacity(text.len());
    let mut lossy = false;
    let mut buf = [0u8; 4];
    for ch in text.chars() {
        if ch.is_ascii() {
            out.push(ch as u8);
            continue;
        }
        let (enc, _, bad) = WINDOWS_1251.encode(ch.encode_utf8(&mut buf));
        if bad {
            out.push(b'?');
            lossy = true;
        } else {
            out.extend_from_slice(&enc);
        }
    }
    (Cow::Owned(out), lossy)
}

/// Whether the game, which reads names through cp1251, can open a file of this name.
#[must_use]
pub fn fits_cp1251(text: &str) -> bool {
    text.is_ascii() || !utf8_to_cp1251(text).1
}

/// Decodes a file of unknown encoding: UTF-8 (with or without a BOM) when it is valid UTF-8,
/// cp1251 otherwise.
#[must_use]
pub fn decode_guess(bytes: &[u8]) -> Cow<'_, str> {
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    match std::str::from_utf8(body) {
        Ok(s) => Cow::Borrowed(s),
        Err(_) => cp1251_to_utf8(body),
    }
}

/// ASCII-insensitive equality, the comparison the engine's `stricmp` makes.
#[must_use]
pub fn eq_ci(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

/// Splits a comma separated list the way the engine's `_GetItem` does: items trimmed, empty
/// items kept out.
pub fn split_list(text: &str) -> impl Iterator<Item = &str> {
    text.split(',').map(str::trim).filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_cyrillic() {
        let (bytes, lossy) = utf8_to_cp1251("Привет, Зона");
        assert!(!lossy);
        assert_eq!(cp1251_to_utf8(&bytes), "Привет, Зона");
    }

    #[test]
    fn marks_what_cp1251_lacks() {
        let (bytes, lossy) = utf8_to_cp1251("a→b");
        assert!(lossy);
        assert_eq!(&*bytes, b"a?b");
        assert!(!fits_cp1251("★"));
        assert!(fits_cp1251("Ёж»"));
    }

    #[test]
    fn guesses_encoding() {
        assert_eq!(decode_guess("\u{feff}текст".as_bytes()), "текст");
        assert_eq!(decode_guess(&[0xF2, 0xE5, 0xEA, 0xF1, 0xF2]), "текст");
    }
}
