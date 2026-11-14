use std::borrow::Cow;

use glam::{Quat, Vec2, Vec3, Vec4};

use crate::{COMPRESSED, ChunkError, Result, lzhuf};

/// A cursor over bytes, with the engine's reader vocabulary.
#[derive(Clone, Debug)]
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

/// A chunk's body: borrowed from the file, or owned when it had to be decompressed.
#[derive(Clone, Debug)]
pub struct Chunk<'a>(Cow<'a, [u8]>);

impl<'a> Chunk<'a> {
    /// A chunk over bytes as they are stored.
    #[must_use]
    pub fn borrowed(bytes: &'a [u8]) -> Self {
        Self(Cow::Borrowed(bytes))
    }

    #[must_use]
    pub fn reader(&self) -> Reader<'_> {
        Reader::new(&self.0)
    }

    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_owned(self) -> Vec<u8> {
        self.0.into_owned()
    }
}

macro_rules! read_le {
    ($($name:ident -> $t:ty),* $(,)?) => {$(
        #[inline]
        pub fn $name(&mut self) -> Result<$t> {
            const N: usize = std::mem::size_of::<$t>();
            let bytes = self.take(N)?;
            let mut raw = [0u8; N];
            raw.copy_from_slice(bytes);
            Ok(<$t>::from_le_bytes(raw))
        }
    )*};
}

impl<'a> Reader<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    #[must_use]
    pub fn data(&self) -> &'a [u8] {
        self.data
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.data.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    #[must_use]
    pub fn tell(&self) -> usize {
        self.pos
    }

    /// Bytes left after the cursor.
    #[must_use]
    pub fn elapsed(&self) -> usize {
        self.data.len() - self.pos
    }

    #[must_use]
    pub fn eof(&self) -> bool {
        self.pos >= self.data.len()
    }

    pub fn seek(&mut self, pos: usize) -> Result<()> {
        if pos > self.data.len() {
            return Err(ChunkError::Eof { at: self.data.len(), wanted: pos - self.data.len() });
        }
        self.pos = pos;
        Ok(())
    }

    pub fn advance(&mut self, n: usize) -> Result<()> {
        self.take(n).map(|_| ())
    }

    /// The bytes from the cursor to the end, without moving.
    #[must_use]
    pub fn rest(&self) -> &'a [u8] {
        &self.data[self.pos..]
    }

    /// The next `n` bytes.
    #[inline]
    pub fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).filter(|&e| e <= self.data.len());
        match end {
            Some(end) => {
                let out = &self.data[self.pos..end];
                self.pos = end;
                Ok(out)
            }
            None => Err(ChunkError::Eof { at: self.pos, wanted: n - self.elapsed().min(n) }),
        }
    }

    read_le! {
        r_u8 -> u8, r_u16 -> u16, r_u32 -> u32, r_u64 -> u64,
        r_i8 -> i8, r_i16 -> i16, r_i32 -> i32, r_i64 -> i64,
        r_f32 -> f32, r_f64 -> f64,
    }

    pub fn r_bool(&mut self) -> Result<bool> {
        Ok(self.r_u8()? != 0)
    }

    pub fn r_vec2(&mut self) -> Result<Vec2> {
        Ok(Vec2::new(self.r_f32()?, self.r_f32()?))
    }

    pub fn r_vec3(&mut self) -> Result<Vec3> {
        Ok(Vec3::new(self.r_f32()?, self.r_f32()?, self.r_f32()?))
    }

    pub fn r_vec4(&mut self) -> Result<Vec4> {
        Ok(Vec4::new(self.r_f32()?, self.r_f32()?, self.r_f32()?, self.r_f32()?))
    }

    /// A quaternion stored x, y, z, w.
    pub fn r_quat(&mut self) -> Result<Quat> {
        Ok(Quat::from_xyzw(self.r_f32()?, self.r_f32()?, self.r_f32()?, self.r_f32()?))
    }

    /// A 4x4 matrix, row after row as the engine's matrix lies in memory.
    pub fn r_matrix(&mut self) -> Result<[[f32; 4]; 4]> {
        let mut m = [[0.0f32; 4]; 4];
        for row in &mut m {
            for v in row {
                *v = self.r_f32()?;
            }
        }
        Ok(m)
    }

    /// A float stored as `u16` over `[min, max]`.
    pub fn r_float_q16(&mut self, min: f32, max: f32) -> Result<f32> {
        let v = self.r_u16()?;
        Ok(f32::from(v) * (max - min) / 65535.0 + min)
    }

    /// A float stored as `u8` over `[min, max]`. The divisor is the engine's.
    pub fn r_float_q8(&mut self, min: f32, max: f32) -> Result<f32> {
        let v = self.r_u8()?;
        Ok(f32::from(v) / 255.0001 * (max - min) + min)
    }

    /// The bytes of a zero-terminated string, without the terminator.
    pub fn r_stringz_raw(&mut self) -> Result<&'a [u8]> {
        let rest = self.rest();
        match rest.iter().position(|&b| b == 0) {
            Some(n) => {
                self.pos += n + 1;
                Ok(&rest[..n])
            }
            None => Err(ChunkError::Unterminated),
        }
    }

    /// A zero-terminated string. UTF-8 when it is valid UTF-8 (the editor's own files), cp1251
    /// otherwise (the game's).
    pub fn r_stringz(&mut self) -> Result<String> {
        Ok(gz_base::text::decode_guess(self.r_stringz_raw()?).into_owned())
    }

    /// A text line up to `\r\n` or `\n`; the terminator is consumed.
    pub fn r_line_raw(&mut self) -> &'a [u8] {
        let rest = self.rest();
        let end = rest.iter().position(|&b| b == b'\n' || b == b'\r').unwrap_or(rest.len());
        let mut next = end;
        while next < rest.len() && (rest[next] == b'\n' || rest[next] == b'\r') {
            next += 1;
            // one terminator only: "\r\n" or a lone "\n"
            if rest[next - 1] == b'\n' {
                break;
            }
        }
        self.pos += next;
        &rest[..end]
    }

    pub fn r_line(&mut self) -> String {
        gz_base::text::decode_guess(self.r_line_raw()).into_owned()
    }

    /// Every top-level chunk from the start, in file order.
    #[must_use]
    pub fn chunks(&self) -> ChunkIter<'a> {
        ChunkIter { data: self.data, pos: 0 }
    }

    /// The raw body of the first chunk with this id, and whether it is compressed.
    #[must_use]
    pub fn find_chunk(&self, id: u32) -> Option<(&'a [u8], bool)> {
        self.chunks().find(|c| c.0 == id).map(|c| (c.2, c.1))
    }

    /// The body of a chunk, decompressed when it has to be. `None` when the chunk is absent.
    pub fn open_chunk(&self, id: u32) -> Result<Option<Chunk<'a>>> {
        match self.find_chunk(id) {
            None => Ok(None),
            Some((body, false)) => Ok(Some(Chunk(Cow::Borrowed(body)))),
            Some((body, true)) => Ok(Some(Chunk(Cow::Owned(lzhuf::decompress(body)?)))),
        }
    }

    /// Like [`Self::open_chunk`], for a chunk the format requires.
    pub fn need_chunk(&self, id: u32) -> Result<Chunk<'a>> {
        self.open_chunk(id)?.ok_or(ChunkError::Missing(id))
    }

    /// A sub-reader over an uncompressed chunk, borrowing the file. `None` when absent.
    pub fn sub(&self, id: u32) -> Result<Option<Reader<'a>>> {
        match self.find_chunk(id) {
            None => Ok(None),
            Some((body, false)) => Ok(Some(Reader::new(body))),
            Some((_, true)) => {
                Err(ChunkError::Invalid(format!("chunk {id:#x} is compressed; open it with open_chunk")))
            }
        }
    }
}

/// Walks sibling chunks: `(id without the flag, compressed, body)`. Stops at the first header
/// that does not fit, the way the engine stops at the end of a stream.
#[derive(Clone, Debug)]
pub struct ChunkIter<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Iterator for ChunkIter<'a> {
    type Item = (u32, bool, &'a [u8]);

    fn next(&mut self) -> Option<Self::Item> {
        let head = self.data.get(self.pos..self.pos + 8)?;
        let id = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u32::from_le_bytes([head[4], head[5], head[6], head[7]]) as usize;
        let start = self.pos + 8;
        let body = self.data.get(start..start.checked_add(size)?)?;
        self.pos = start + size;
        Some((id & !COMPRESSED, id & COMPRESSED != 0, body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Writer;

    #[test]
    fn reads_what_the_writer_wrote() {
        let mut w = Writer::new();
        w.open_chunk(1);
        w.w_u32(7);
        w.w_stringz("имя");
        w.close_chunk();
        w.open_chunk(2);
        w.open_chunk(10);
        w.w_f32(1.5);
        w.close_chunk();
        w.close_chunk();
        let bytes = w.into_bytes();

        let r = Reader::new(&bytes);
        let mut c1 = r.sub(1).unwrap().unwrap();
        assert_eq!(c1.r_u32().unwrap(), 7);
        assert_eq!(c1.r_stringz().unwrap(), "имя");
        assert!(c1.eof());
        let c2 = r.sub(2).unwrap().unwrap();
        assert_eq!(c2.sub(10).unwrap().unwrap().r_f32().unwrap(), 1.5);
        assert!(r.sub(3).unwrap().is_none());
        assert_eq!(r.chunks().count(), 2);
    }

    #[test]
    fn a_short_file_is_an_error_not_a_panic() {
        let mut r = Reader::new(&[1, 2, 3]);
        assert!(matches!(r.r_u32(), Err(ChunkError::Eof { .. })));
        assert_eq!(r.tell(), 0);
        assert!(Reader::new(&[1, 0, 0, 0, 200, 0, 0, 0, 9]).chunks().next().is_none());
        assert!(matches!(Reader::new(b"abc").r_stringz_raw(), Err(ChunkError::Unterminated)));
    }

    #[test]
    fn lines_end_at_either_terminator() {
        let mut r = Reader::new(b"one\r\ntwo\n\nfour");
        assert_eq!(r.r_line(), "one");
        assert_eq!(r.r_line(), "two");
        assert_eq!(r.r_line(), "");
        assert_eq!(r.r_line(), "four");
        assert!(r.eof());
    }
}
