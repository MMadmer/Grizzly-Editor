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

/// The key the game takes a module's files in turn by, wherever the order decides something (two
/// items that change one section, two patches of one key): the path as the game reads it (cp1251),
/// its ASCII letters lower-cased, `/` between folders, compared byte by byte. So `a/k` comes before
/// `a2/k`, and `B` after `a`.
#[must_use]
pub fn game_order_key(path: &str) -> Vec<u8> {
    let (bytes, _) = utf8_to_cp1251(path);
    bytes.iter().map(|&b| if b == b'\\' { b'/' } else { b.to_ascii_lowercase() }).collect()
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

/// Why the system refused a file operation, in the editor's words: the system's own sentence comes in
/// the system's language and with a number, and a message is a sentence for the author. `code` is
/// the Windows error; one the editor has no words for keeps its number, the last resort.
#[must_use]
pub fn os_reason(code: u32) -> String {
    let words = match code {
        0 => "the system gave no reason",
        2 | 3 => "it is not there",
        5 => "access is denied: it is read-only, or another program holds it",
        19 => "the disk is write-protected",
        32 | 33 => "another program holds it open",
        39 | 112 => "the disk is full",
        80 | 183 => "something of that name is already there",
        123 | 161 | 267 => "the name is not one Windows takes",
        145 => "the folder is not empty",
        206 => "the path is too long",
        _ => return format!("the system refused it (system error {code})"),
    };
    words.to_owned()
}

/// [`os_reason`] of an I/O error; one the system gave no code for says its own words.
#[must_use]
pub fn io_reason(e: &std::io::Error) -> String {
    match e.raw_os_error().and_then(|c| u32::try_from(c).ok()) {
        Some(code) => os_reason(code),
        None => match e.kind() {
            std::io::ErrorKind::NotFound => os_reason(2),
            std::io::ErrorKind::PermissionDenied => os_reason(5),
            std::io::ErrorKind::AlreadyExists => os_reason(183),
            _ => e.to_string(),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_games_order_of_a_modules_files() {
        let mut v = vec![
            "items/weapons2/ak.itemasset",
            "items/weapons/ak.itemasset",
            "items/B.itemasset",
            "items/a.itemasset",
        ];
        v.sort_by_key(|p| game_order_key(p));
        assert_eq!(
            v,
            [
                "items/a.itemasset",
                "items/B.itemasset",
                "items/weapons/ak.itemasset",
                "items/weapons2/ak.itemasset"
            ]
        );
        // either slash, any case of the ASCII letters
        assert_eq!(game_order_key("Items\\AK"), game_order_key("items/ak"));
    }

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
