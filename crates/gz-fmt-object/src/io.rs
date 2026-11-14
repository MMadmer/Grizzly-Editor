//! Reading and writing helpers shared by the formats of this crate.

use std::borrow::Cow;

use glam::Vec3;
use gz_chunk::{ChunkError, Reader, Writer};
use smol_str::SmolStr;

use crate::{ObjectError, Result};

/// How the strings of a file are stored. Bytes are kept as they are, so a read and a
/// write of the same file are identical.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum TextEncoding {
    #[default]
    Utf8,
    Cp1251,
}

impl TextEncoding {
    /// The bytes of `text` in this encoding.
    #[must_use]
    pub fn encode<'a>(self, text: &'a str) -> Cow<'a, [u8]> {
        match self {
            TextEncoding::Utf8 => Cow::Borrowed(text.as_bytes()),
            TextEncoding::Cp1251 => gz_base::text::utf8_to_cp1251(text).0,
        }
    }
}

/// Decodes strings and remembers whether one of them was not UTF-8.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Text {
    /// Forced encoding; `None` decodes UTF-8 and falls back to cp1251 per string.
    pub force: Option<TextEncoding>,
    pub non_utf8: bool,
}

impl Text {
    pub fn forced(enc: TextEncoding) -> Self {
        Self { force: Some(enc), non_utf8: false }
    }

    pub fn decode(&mut self, raw: &[u8]) -> SmolStr {
        if self.force == Some(TextEncoding::Cp1251) {
            return SmolStr::new(gz_base::text::cp1251_to_utf8(raw));
        }
        match std::str::from_utf8(raw) {
            Ok(s) => SmolStr::new(s),
            Err(_) => {
                self.non_utf8 = true;
                SmolStr::new(gz_base::text::cp1251_to_utf8(raw))
            }
        }
    }

    pub fn r_str(&mut self, r: &mut Reader<'_>) -> Result<SmolStr> {
        let raw = r.r_stringz_raw()?;
        Ok(self.decode(raw))
    }

    /// The encoding a file read with this decoder is written back in.
    #[cfg(test)]
    pub fn encoding(&self) -> TextEncoding {
        match self.force {
            Some(e) => e,
            None if self.non_utf8 => TextEncoding::Cp1251,
            None => TextEncoding::Utf8,
        }
    }
}

pub(crate) fn w_str(w: &mut Writer, text: &str, enc: TextEncoding) {
    w.w_stringz_raw(&enc.encode(text));
}

/// A count read from a file, refused when the bytes left cannot hold that many items.
pub(crate) fn checked_count(r: &Reader<'_>, count: usize, item_size: usize) -> Result<usize> {
    match count.checked_mul(item_size.max(1)) {
        Some(bytes) if bytes <= r.elapsed() => Ok(count),
        _ => Err(ObjectError::Chunk(ChunkError::Eof {
            at: r.tell(),
            wanted: count.saturating_mul(item_size.max(1)),
        })),
    }
}

/// The body of the first chunk with this id; an empty chunk counts as absent, as it does for
/// the engine's `find_chunk`.
pub(crate) fn chunk<'a>(r: &Reader<'a>, id: u32) -> Option<Reader<'a>> {
    r.find_chunk(id).filter(|(body, _)| !body.is_empty()).map(|(body, _)| Reader::new(body))
}

pub(crate) fn need_chunk<'a>(r: &Reader<'a>, id: u32) -> Result<Reader<'a>> {
    chunk(r, id).ok_or(ObjectError::Chunk(ChunkError::Missing(id)))
}

/// `r_chunk(id, &u32)`: the first bytes of a chunk as a little-endian number.
pub(crate) fn chunk_u32(r: &Reader<'_>, id: u32) -> Option<u32> {
    let body = chunk(r, id)?;
    let mut raw = [0u8; 4];
    for (dst, src) in raw.iter_mut().zip(body.data()) {
        *dst = *src;
    }
    Some(u32::from_le_bytes(raw))
}

pub(crate) fn r_f32s(r: &mut Reader<'_>, count: usize) -> Result<Vec<f32>> {
    let bytes = r.take(count.checked_mul(4).ok_or(ChunkError::Eof { at: r.tell(), wanted: usize::MAX })?)?;
    Ok(bytes.as_chunks::<4>().0.iter().map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub(crate) fn r_i32s(r: &mut Reader<'_>, count: usize) -> Result<Vec<i32>> {
    let bytes = r.take(count.checked_mul(4).ok_or(ChunkError::Eof { at: r.tell(), wanted: usize::MAX })?)?;
    Ok(bytes.as_chunks::<4>().0.iter().map(|c| i32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect())
}

pub(crate) fn r_vec3s(r: &mut Reader<'_>, count: usize) -> Result<Vec<Vec3>> {
    let bytes = r.take(count.checked_mul(12).ok_or(ChunkError::Eof { at: r.tell(), wanted: usize::MAX })?)?;
    let f = |c: &[u8]| f32::from_le_bytes([c[0], c[1], c[2], c[3]]);
    Ok(bytes.as_chunks::<12>().0.iter().map(|c| Vec3::new(f(&c[0..4]), f(&c[4..8]), f(&c[8..12]))).collect())
}

/// `w_float_q16`: the engine rounds to the nearest step.
pub(crate) fn w_float_q16(w: &mut Writer, v: f32, min: f32, max: f32) {
    let q = (v - min) / (max - min);
    w.w_u16((q * 65535.0 + 0.5).floor() as i32 as u16);
}

/// `r_string`: a line, with the engine's quirks - the first byte is taken whatever it is, and
/// every terminator byte that follows the line is swallowed, including one that belongs to the
/// next field.
pub(crate) fn r_term_string<'a>(r: &mut Reader<'a>) -> &'a [u8] {
    let data = r.data();
    let start = r.tell();
    let is_term = |b: u8| b == 13 || b == 10;
    let mut pos = start;
    let mut size = 0usize;
    while pos < data.len() {
        pos += 1;
        size += 1;
        if pos < data.len() && is_term(data[pos]) {
            while pos < data.len() && is_term(data[pos]) {
                pos += 1;
            }
            break;
        }
    }
    // `pos` never passes the end, so the seek cannot fail
    let _ = r.seek(pos);
    data.get(start..start + size).unwrap_or(&[])
}

/// `w_string`: the text and `\r\n`.
pub(crate) fn w_term_string(w: &mut Writer, text: &str, enc: TextEncoding) {
    w.w(&enc.encode(text));
    w.w_u8(13);
    w.w_u8(10);
}

/// ASCII lower case, what the CRT's `strlwr` does in the "C" locale.
pub(crate) fn lower(name: &SmolStr) -> SmolStr {
    if name.bytes().any(|b| b.is_ascii_uppercase()) {
        SmolStr::new(name.to_ascii_lowercase())
    } else {
        name.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn line_reader_keeps_the_engine_quirks() {
        let mut r = Reader::new(b"abc\r\n\x02\0\0\0");
        assert_eq!(r_term_string(&mut r), b"abc");
        assert_eq!(r.r_u32().unwrap(), 2);
        // an empty name takes its own terminator as the first character
        let mut r = Reader::new(b"\r\n\x05");
        assert_eq!(r_term_string(&mut r), b"\r");
        assert_eq!(r.r_u8().unwrap(), 5);
        // a count of ten is eaten as a terminator: the X-Ray SDK does the same
        let mut r = Reader::new(b"x\r\n\x0a\0\0\0");
        assert_eq!(r_term_string(&mut r), b"x");
        assert_eq!(r.elapsed(), 3);
    }

    #[test]
    fn quantises_like_the_engine() {
        let mut w = Writer::new();
        w_float_q16(&mut w, 0.0, -32.0, 32.0);
        w_float_q16(&mut w, 32.0, -32.0, 32.0);
        let b = w.into_bytes();
        assert_eq!(u16::from_le_bytes([b[0], b[1]]), 32768);
        assert_eq!(u16::from_le_bytes([b[2], b[3]]), 65535);
        // every step survives a read and a write
        for v in [0u16, 1, 2, 777, 32767, 32768, 65534, 65535] {
            let bytes = v.to_le_bytes();
            let f = Reader::new(&bytes).r_float_q16(-32.0, 32.0).unwrap();
            let mut w = Writer::new();
            w_float_q16(&mut w, f, -32.0, 32.0);
            assert_eq!(w.bytes(), &bytes);
        }
    }

    #[test]
    fn text_falls_back_to_cp1251() {
        let mut t = Text::default();
        assert_eq!(t.decode(b"plain"), "plain");
        assert!(!t.non_utf8);
        assert_eq!(t.decode(&[0xF2, 0xE5, 0xEA, 0xF1, 0xF2]), "текст");
        assert_eq!(t.encoding(), TextEncoding::Cp1251);
        assert_eq!(&*TextEncoding::Cp1251.encode("текст"), &[0xF2, 0xE5, 0xEA, 0xF1, 0xF2]);
    }
}
