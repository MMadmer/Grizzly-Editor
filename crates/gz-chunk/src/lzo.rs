//! LZO1X, the compression of the engine's runtime compressor
//! (over LZO 2.10), read only.
//!
//! `rtc` adds no framing of its own: the stream is a bare LZO1X block, without a size and
//! without a checksum. Whoever stores one keeps the two sizes beside it:
//!
//! - a game archive (`.db*`, `.xdb`) keeps them in its directory - chunk 1, LZHUF-compressed
//!   when its id carries the flag - one record per file: `u16 n`, `u32 size_real`,
//!   `u32 size_compressed`, `u32 crc`, `n - 16` bytes of name, `u32 offset`. A file whose two
//!   sizes are equal is stored as it is; any other is one `rtc` block ([`unpack_entry`]). The
//!   `crc` is of the unpacked bytes and nothing in the engine checks it on reading;
//! - a save file is `u32 -1`, `u32 version`, `u32 unpacked size`, then the block.
//!
//! What is here:
//!
//! - [`decompress`] - a block unpacked to exactly the size its owner states. The decoder is
//!   `lzokay-native` (MIT, safe Rust, a port of lzokay - an implementation of the format of its
//!   own): every read and every copy is checked, its output grows only as the stream fills it.
//!   The editor carries no code of the LZO library itself.
//! - [`unpack_entry`], [`read_archive_index`], [`ArchiveEntry`] - the archive framing above.
//!
//! Not here: a compressor - the editor reads the game's archives and writes none (the tests
//! pack their fixtures with the same crate's compressor, a feature only they turn on); `rtc9` (LZO1X-999 with a preset dictionary,
//! network packets only); the byte scrambling of Shadow of Chernobyl's own `.db` archives,
//! which the game applies to a directory before LZHUF when an archive has no header chunk.

use std::borrow::Cow;

use crate::{ChunkError, Reader, Result};

const INPUT_OVERRUN: ChunkError = ChunkError::Compression("the LZO stream ends early");
const OUTPUT_OVERRUN: ChunkError =
    ChunkError::Compression("the LZO stream unpacks to more than its stated size");
const OUTPUT_SHORT: ChunkError =
    ChunkError::Compression("the LZO stream unpacks to less than its stated size");
const LOOKBEHIND_OVERRUN: ChunkError = ChunkError::Compression("an LZO match starts before the output");
const TRAILING: ChunkError = ChunkError::Compression("bytes follow the end of the LZO stream");
const MALFORMED: ChunkError = ChunkError::Compression("the LZO stream is malformed");

/// Unpacks `packed` into exactly `unpacked_len` bytes.
///
/// The stream has to end exactly where the input ends and give exactly that many bytes, the
/// two things the game checks. The stated size is only a limit: nothing is reserved for it
/// that the stream does not fill, so a wrong size costs no memory.
pub fn decompress(packed: &[u8], unpacked_len: usize) -> Result<Vec<u8>> {
    use lzokay_native::Error;
    let mut src = std::io::Cursor::new(packed);
    let out = lzokay_native::decompress_bounded(&mut src, unpacked_len).map_err(|e| match e {
        Error::OutputOverrun => OUTPUT_OVERRUN,
        Error::LookbehindOverrun => LOOKBEHIND_OVERRUN,
        Error::IOError(io) if io.kind() == std::io::ErrorKind::UnexpectedEof => INPUT_OVERRUN,
        _ => MALFORMED,
    })?;
    if src.position() != packed.len() as u64 {
        return Err(TRAILING);
    }
    if out.len() != unpacked_len {
        return Err(OUTPUT_SHORT);
    }
    Ok(out)
}

/// One file of an archive's directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ArchiveEntry {
    /// The path inside the archive, as written (backslashes, the archive's case).
    pub name: String,
    pub size_real: u32,
    pub size_compressed: u32,
    /// CRC-32 of the unpacked bytes; 0 when the packer did not write one.
    pub crc: u32,
    /// Where the stored bytes start in the archive file.
    pub offset: u32,
}

impl ArchiveEntry {
    /// Whether the file is stored as it is.
    #[must_use]
    pub fn is_stored(&self) -> bool {
        self.size_real == self.size_compressed
    }
}

/// Reads the directory of an archive: the body of its chunk 1, already unpacked.
/// Names are cp1251, as every name the game writes.
pub fn read_archive_index(index: &[u8]) -> Result<Vec<ArchiveEntry>> {
    let mut r = Reader::new(index);
    // a record is at least 18 bytes
    let mut out = Vec::with_capacity(index.len() / 40);
    while !r.eof() {
        let size = usize::from(r.r_u16()?);
        if size < 16 {
            return Err(ChunkError::Invalid(format!(
                "an archive directory record of {size} bytes at offset {} is too short",
                r.tell() - 2
            )));
        }
        let mut rec = Reader::new(r.take(size)?);
        let size_real = rec.r_u32()?;
        let size_compressed = rec.r_u32()?;
        let crc = rec.r_u32()?;
        let name = rec.take(size - 16)?;
        let offset = rec.r_u32()?;
        out.push(ArchiveEntry {
            name: gz_base::text::cp1251_to_utf8(name).into_owned(),
            size_real,
            size_compressed,
            crc,
            offset,
        });
    }
    Ok(out)
}

/// The bytes of an archived file: `stored` is what lies in the archive (`size_compressed`
/// bytes at `offset`), `size_real` what the directory says it unpacks to.
pub fn unpack_entry(stored: &[u8], size_real: usize) -> Result<Cow<'_, [u8]>> {
    if stored.len() == size_real {
        return Ok(Cow::Borrowed(stored));
    }
    decompress(stored, size_real).map(Cow::Owned)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::Writer;

    fn sample(len: usize) -> Vec<u8> {
        let mut seed = 0x9e37_79b9u32;
        (0..len)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                match (i / 211) % 4 {
                    0 => b"[section]:parent\r\nkey = value\r\n"[i % 31],
                    1 => (seed >> 24) as u8,
                    2 => 0,
                    _ => (i % 7) as u8,
                }
            })
            .collect()
    }

    /// A block as another LZO1X compressor writes it.
    fn pack(body: &[u8]) -> Vec<u8> {
        lzokay_native::compress(body).unwrap()
    }

    // Blocks as the engine's compressor (`lzo1x_1_compress`) writes them.
    #[test]
    fn the_engines_blocks() {
        // nothing: the end marker alone
        assert_eq!(decompress(&[17, 0, 0], 0).unwrap(), b"");
        // a first run of up to 238 literals is 17 + its length, then the bytes
        assert_eq!(decompress(&[20, b'a', b'b', b'c', 17, 0, 0], 3).unwrap(), b"abc");
        let mut run = vec![17 + 31];
        run.extend_from_slice(&[7u8; 31]);
        run.extend_from_slice(&[17, 0, 0]);
        assert_eq!(decompress(&run, 31).unwrap(), [7u8; 31]);
        // 40 equal bytes: five literals, a match of 20 at distance 5, the 15 left as a run
        let mut block = vec![5 - 3, b'x', b'x', b'x', b'x', b'x', 32 | (20 - 2), 4 << 2, 0, 15 - 3];
        block.extend_from_slice(&[b'x'; 15]);
        block.extend_from_slice(&[17, 0, 0]);
        assert_eq!(decompress(&block, 40).unwrap(), [b'x'; 40]);
    }

    #[test]
    fn round_trips() {
        for len in [1, 3, 4, 20, 21, 32, 33, 100, 1000, 49151, 49152, 49153, 49180, 120_000] {
            let body = sample(len);
            assert_eq!(decompress(&pack(&body), len).unwrap(), body, "length {len}");
        }
        let zeros = vec![0u8; 200_000];
        let packed = pack(&zeros);
        assert!(packed.len() < 2000);
        assert_eq!(decompress(&packed, zeros.len()).unwrap(), zeros);
    }

    #[test]
    fn damage_is_an_error() {
        let body = sample(4000);
        let packed = pack(&body);
        assert!(decompress(&[], 0).is_err());
        assert!(decompress(&packed, 3999).is_err());
        assert!(decompress(&packed, 4001).is_err());
        assert!(decompress(&packed[..packed.len() - 1], 4000).is_err());
        assert!(decompress(&packed[..packed.len() / 2], 4000).is_err());
        let mut longer = packed.clone();
        longer.push(0);
        assert!(decompress(&longer, 4000).is_err());
        // a match that points before the output
        assert!(decompress(&[18, b'a', 0x40, 0xff, 17, 0, 0], 4).is_err());
        // a stated size the stream does not reach is an error and reserves nothing
        assert!(decompress(&[17, 0, 0], usize::MAX).is_err());
        assert!(decompress(&[17, 0, 0], 1 << 20).is_err());
    }

    #[test]
    fn archive_framing() {
        let mut w = Writer::new();
        for (name, real, packed, offset) in
            [("configs\\system.ltx", 10u32, 10u32, 0u32), ("a.ogf", 4000, 1200, 10)]
        {
            w.w_u16((16 + name.len()) as u16);
            w.w_u32(real);
            w.w_u32(packed);
            w.w_u32(0xdead_beef);
            w.w(name.as_bytes());
            w.w_u32(offset);
        }
        let bytes = w.into_bytes();
        let index = read_archive_index(&bytes).unwrap();
        assert_eq!(index.len(), 2);
        assert_eq!(index[0].name, "configs\\system.ltx");
        assert!(index[0].is_stored());
        assert_eq!((index[1].size_real, index[1].size_compressed, index[1].offset), (4000, 1200, 10));
        assert!(read_archive_index(&bytes[..bytes.len() - 1]).is_err());
        assert!(read_archive_index(&[3, 0, 1, 2, 3]).is_err());

        let body = sample(4000);
        assert!(matches!(unpack_entry(&body, 4000).unwrap(), Cow::Borrowed(_)));
        assert_eq!(&*unpack_entry(&pack(&body), 4000).unwrap(), &body[..]);
    }
}
