//! LZO1X, the compression of the engine's `rtc_compress` / `rtc_decompress`
//! (`XrCore/rt_compressor.cpp` over LZO 2.10).
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
//! - [`decompress`] - `lzo1x_decompress_safe`: every read and every copy is checked, the output
//!   never grows past the size asked for.
//! - [`compress`] - `lzo1x_1_compress` as the engine's x64 build compiles it, ported statement
//!   for statement so that the bytes are the same.
//! - [`max_compressed_len`] - `rtc_csize`, the worst case of [`compress`].
//! - [`unpack_entry`], [`read_archive_index`], [`ArchiveEntry`] - the archive framing above.
//!
//! Not here: `rtc9` (LZO1X-999 with a preset dictionary, network packets only) and the byte
//! scrambling of Shadow of Chernobyl's own `.db` archives, which the game applies to a
//! directory before LZHUF when an archive has no header chunk.

use std::borrow::Cow;

use crate::{ChunkError, Reader, Result};

const M2_MAX_LEN: usize = 8;
const M2_MAX_OFFSET: usize = 0x0800;
const M3_MAX_LEN: usize = 33;
const M3_MAX_OFFSET: usize = 0x4000;
const M4_MAX_LEN: usize = 9;
const M3_MARKER: usize = 32;
const M4_MARKER: usize = 16;

/// Bits of the compressor's dictionary index.
const D_BITS: u32 = 14;
/// A block the compressor works on at once: its dictionary holds 16-bit offsets.
const BLOCK: usize = 49152;

const INPUT_OVERRUN: ChunkError = ChunkError::Compression("the LZO stream ends early");
const OUTPUT_OVERRUN: ChunkError =
    ChunkError::Compression("the LZO stream unpacks to more than its stated size");
const LOOKBEHIND_OVERRUN: ChunkError = ChunkError::Compression("an LZO match starts before the output");

#[inline]
fn byte(src: &[u8], at: usize) -> Result<usize> {
    src.get(at).map(|&b| usize::from(b)).ok_or(INPUT_OVERRUN)
}

/// The length that follows a zero length field: `255` per zero byte, then a last byte.
fn long_length(src: &[u8], ip: &mut usize, base: usize) -> Result<usize> {
    let mut t = 0usize;
    while byte(src, *ip)? == 0 {
        t = t.saturating_add(255);
        *ip += 1;
    }
    t = t.saturating_add(base + byte(src, *ip)?);
    *ip += 1;
    Ok(t)
}

fn copy_literals(src: &[u8], ip: &mut usize, out: &mut Vec<u8>, n: usize, limit: usize) -> Result<()> {
    if n > limit - out.len() {
        return Err(OUTPUT_OVERRUN);
    }
    let run = ip.checked_add(n).and_then(|end| src.get(*ip..end)).ok_or(INPUT_OVERRUN)?;
    out.extend_from_slice(run);
    *ip += n;
    Ok(())
}

fn copy_match(out: &mut Vec<u8>, offset: usize, n: usize, limit: usize) -> Result<()> {
    if offset == 0 || offset > out.len() {
        return Err(LOOKBEHIND_OVERRUN);
    }
    if n > limit - out.len() {
        return Err(OUTPUT_OVERRUN);
    }
    let start = out.len() - offset;
    // a match may overlap what it writes: the bytes already copied repeat
    let mut left = n;
    while left > 0 {
        let chunk = left.min(out.len() - start);
        out.extend_from_within(start..start + chunk);
        left -= chunk;
    }
    Ok(())
}

enum Step {
    /// The top of the outer loop: a literal run or a match.
    Start,
    /// Right after a literal run.
    AfterLiterals,
    /// A match whose first byte is the value.
    Match(usize),
    /// After a match: the low bits of its second-to-last byte count trailing literals.
    MatchDone,
}

/// Unpacks `packed` into exactly `unpacked_len` bytes.
///
/// The stream has to end exactly where the input ends and give exactly that many bytes, the
/// two things the game checks. Memory grows with what is really unpacked, so a wrong
/// `unpacked_len` cannot reserve more than the stream produces.
pub fn decompress(packed: &[u8], unpacked_len: usize) -> Result<Vec<u8>> {
    let src = packed;
    let limit = unpacked_len;
    let mut out: Vec<u8> = Vec::with_capacity(limit.min(src.len().saturating_mul(8).saturating_add(64)));
    let mut ip = 0usize;
    let mut step = Step::Start;

    let first = byte(src, 0)?;
    if first > 17 {
        ip = 1;
        let t = first - 17;
        if t < 4 {
            copy_literals(src, &mut ip, &mut out, t, limit)?;
            let next = byte(src, ip)?;
            ip += 1;
            step = Step::Match(next);
        } else {
            copy_literals(src, &mut ip, &mut out, t, limit)?;
            step = Step::AfterLiterals;
        }
    }

    loop {
        match step {
            Step::Start => {
                let mut t = byte(src, ip)?;
                ip += 1;
                if t >= 16 {
                    step = Step::Match(t);
                    continue;
                }
                if t == 0 {
                    t = long_length(src, &mut ip, 15)?;
                }
                copy_literals(src, &mut ip, &mut out, t.saturating_add(3), limit)?;
                step = Step::AfterLiterals;
            }
            Step::AfterLiterals => {
                let t = byte(src, ip)?;
                ip += 1;
                if t >= 16 {
                    step = Step::Match(t);
                    continue;
                }
                let offset = (1 + M2_MAX_OFFSET) + (t >> 2) + (byte(src, ip)? << 2);
                ip += 1;
                copy_match(&mut out, offset, 3, limit)?;
                step = Step::MatchDone;
            }
            Step::Match(t) => {
                if t >= 64 {
                    let offset = 1 + ((t >> 2) & 7) + (byte(src, ip)? << 3);
                    ip += 1;
                    copy_match(&mut out, offset, (t >> 5) + 1, limit)?;
                } else if t >= 32 {
                    let mut len = t & 31;
                    if len == 0 {
                        len = long_length(src, &mut ip, 31)?;
                    }
                    let offset = 1 + ((byte(src, ip)? | (byte(src, ip + 1)? << 8)) >> 2);
                    ip += 2;
                    copy_match(&mut out, offset, len.saturating_add(2), limit)?;
                } else if t >= 16 {
                    let high = (t & 8) << 11;
                    let mut len = t & 7;
                    if len == 0 {
                        len = long_length(src, &mut ip, 7)?;
                    }
                    let low = (byte(src, ip)? | (byte(src, ip + 1)? << 8)) >> 2;
                    ip += 2;
                    if high + low == 0 {
                        // the end marker
                        return if ip != src.len() {
                            Err(ChunkError::Compression("the LZO stream ends before its data does"))
                        } else if out.len() != limit {
                            Err(ChunkError::Compression(
                                "the LZO stream unpacks to less than its stated size",
                            ))
                        } else {
                            Ok(out)
                        };
                    }
                    copy_match(&mut out, high + low + 0x4000, len.saturating_add(2), limit)?;
                } else {
                    let offset = 1 + (t >> 2) + (byte(src, ip)? << 2);
                    ip += 1;
                    copy_match(&mut out, offset, 2, limit)?;
                }
                step = Step::MatchDone;
            }
            Step::MatchDone => {
                let t = match ip.checked_sub(2) {
                    Some(at) => byte(src, at)? & 3,
                    None => return Err(INPUT_OVERRUN),
                };
                if t == 0 {
                    step = Step::Start;
                    continue;
                }
                copy_literals(src, &mut ip, &mut out, t, limit)?;
                let next = byte(src, ip)?;
                ip += 1;
                step = Step::Match(next);
            }
        }
    }
}

/// `rtc_csize`: a buffer that [`compress`] never outgrows.
#[must_use]
pub fn max_compressed_len(unpacked_len: usize) -> usize {
    unpacked_len.saturating_add(unpacked_len / 64).saturating_add(16 + 3)
}

#[inline]
fn le32(src: &[u8], at: usize) -> u32 {
    match src.get(at..).and_then(|s| s.first_chunk::<4>()) {
        Some(b) => u32::from_le_bytes(*b),
        None => 0,
    }
}

#[inline]
fn le64(src: &[u8], at: usize) -> u64 {
    match src.get(at..).and_then(|s| s.first_chunk::<8>()) {
        Some(b) => u64::from_le_bytes(*b),
        None => 0,
    }
}

/// The header of a literal run of `t` bytes that follows a match (or opens a later block).
fn put_literal_run(out: &mut Vec<u8>, t: usize) {
    if t <= 3 {
        // the count rides in the two spare bits of the match before it
        if let Some(b) = out.len().checked_sub(2).and_then(|at| out.get_mut(at)) {
            *b |= t as u8;
        }
    } else if t <= 18 {
        out.push((t - 3) as u8);
    } else {
        let mut tt = t - 18;
        out.push(0);
        while tt > 255 {
            tt -= 255;
            out.push(0);
        }
        out.push(tt as u8);
    }
}

fn put_long_length(out: &mut Vec<u8>, mut len: usize) {
    while len > 255 {
        len -= 255;
        out.push(0);
    }
    out.push(len as u8);
}

/// `do_compress`: one block of at most [`BLOCK`] bytes at `src[base..base + len]`. `ti` is the
/// count of literals the block before left unwritten; the return is this block's.
fn compress_block(
    src: &[u8],
    base: usize,
    len: usize,
    out: &mut Vec<u8>,
    mut ti: usize,
    dict: &mut [u16],
) -> usize {
    let in_end = base + len;
    let ip_end = in_end.saturating_sub(20);
    let mut ip = base;
    let mut ii = base;

    ip += 4_usize.saturating_sub(ti);
    'literal: loop {
        ip += 1 + ((ip - ii) >> 5);
        loop {
            if ip >= ip_end {
                break 'literal;
            }
            let dv = le32(src, ip);
            let dindex = (0x1824_429d_u32.wrapping_mul(dv) >> (32 - D_BITS)) as usize & ((1 << D_BITS) - 1);
            let Some(slot) = dict.get_mut(dindex) else { break 'literal };
            let m_pos = base + usize::from(*slot);
            *slot = (ip - base) as u16;
            if dv != le32(src, m_pos) {
                continue 'literal;
            }

            // a match
            ii = ii.saturating_sub(ti);
            ti = 0;
            let t = ip - ii;
            if t != 0 {
                put_literal_run(out, t);
                if let Some(run) = src.get(ii..ip) {
                    out.extend_from_slice(run);
                }
            }
            let mut m_len = 4usize;
            {
                let mut v = le64(src, ip + m_len) ^ le64(src, m_pos + m_len);
                let mut at_end = false;
                if v == 0 {
                    loop {
                        m_len += 8;
                        v = le64(src, ip + m_len) ^ le64(src, m_pos + m_len);
                        if ip + m_len >= ip_end {
                            at_end = true;
                            break;
                        }
                        if v != 0 {
                            break;
                        }
                    }
                }
                if !at_end {
                    m_len += (v.trailing_zeros() / 8) as usize;
                }
            }
            let mut m_off = ip - m_pos;
            ip += m_len;
            ii = ip;
            if m_len <= M2_MAX_LEN && m_off <= M2_MAX_OFFSET {
                m_off -= 1;
                out.push((((m_len - 1) << 5) | ((m_off & 7) << 2)) as u8);
                out.push((m_off >> 3) as u8);
            } else if m_off <= M3_MAX_OFFSET {
                m_off -= 1;
                if m_len <= M3_MAX_LEN {
                    out.push((M3_MARKER | (m_len - 2)) as u8);
                } else {
                    out.push(M3_MARKER as u8);
                    put_long_length(out, m_len - M3_MAX_LEN);
                }
                out.push((m_off << 2) as u8);
                out.push((m_off >> 6) as u8);
            } else {
                m_off -= 0x4000;
                if m_len <= M4_MAX_LEN {
                    out.push((M4_MARKER | ((m_off >> 11) & 8) | (m_len - 2)) as u8);
                } else {
                    out.push((M4_MARKER | ((m_off >> 11) & 8)) as u8);
                    put_long_length(out, m_len - M4_MAX_LEN);
                }
                out.push((m_off << 2) as u8);
                out.push((m_off >> 6) as u8);
            }
        }
    }
    in_end.saturating_sub(ii.saturating_sub(ti))
}

/// Packs `body` into one LZO1X block (`rtc_compress`).
///
/// A port of `lzo1x_1_compress` of LZO 2.10 in the form the engine's x64 build takes: the
/// deterministic variant (16-bit dictionary offsets, blocks of 49152 bytes, the dictionary
/// cleared per block) with matches extended eight bytes at a time.
#[must_use]
pub fn compress(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(max_compressed_len(body.len()).min(body.len() / 2 + 64));
    let mut dict = vec![0u16; 1 << D_BITS];
    let mut ip = 0usize;
    let mut left = body.len();
    let mut t = 0usize;

    while left > 20 {
        let ll = left.min(BLOCK);
        // oracle: the C guards a pointer sum against overflow with a test that is also true
        // while the sum adds nothing, so 21..31 bytes with nothing pending are stored
        if (t + ll) >> 5 == 0 {
            break;
        }
        dict.fill(0);
        t = compress_block(body, ip, ll, &mut out, t, &mut dict);
        ip += ll;
        left -= ll;
    }
    t += left;

    if t > 0 {
        if out.is_empty() && t <= 238 {
            out.push((17 + t) as u8);
        } else {
            put_literal_run(&mut out, t);
        }
        if let Some(tail) = body.len().checked_sub(t).and_then(|at| body.get(at..)) {
            out.extend_from_slice(tail);
        }
    }
    out.extend_from_slice(&[(M4_MARKER | 1) as u8, 0, 0]);
    out
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

    #[test]
    fn short_inputs_are_stored() {
        assert_eq!(compress(b""), [17, 0, 0]);
        assert_eq!(compress(b"abc"), [20, b'a', b'b', b'c', 17, 0, 0]);
        // 21..31 bytes never reach the matcher
        let body = [7u8; 31];
        let packed = compress(&body);
        assert_eq!(packed.len(), 1 + 31 + 3);
        assert_eq!(packed[0], 17 + 31);
        assert_eq!(decompress(&packed, 31).unwrap(), body);
    }

    // Traced through lzo1x_c.ch by hand: 40 equal bytes. The first probe is at offset 5, its
    // dictionary slot is empty and so points at offset 0, which matches: five literals, then a
    // match at distance 5 that the eight-byte compare carries to 20 bytes (it stops once it
    // is inside the block's last 20 bytes), then the 15 bytes left as a run.
    #[test]
    fn known_vector() {
        let body = [b'x'; 40];
        let mut want = vec![5 - 3, b'x', b'x', b'x', b'x', b'x', 32 | (20 - 2), 4 << 2, 0, 15 - 3];
        want.extend_from_slice(&[b'x'; 15]);
        want.extend_from_slice(&[17, 0, 0]);
        assert_eq!(compress(&body), want);
        assert_eq!(decompress(&want, 40).unwrap(), body);
    }

    #[test]
    fn round_trips() {
        for len in [0, 1, 3, 4, 20, 21, 32, 33, 100, 1000, 49151, 49152, 49153, 49180, 120_000] {
            let body = sample(len);
            let packed = compress(&body);
            assert!(packed.len() <= max_compressed_len(len), "length {len}");
            assert_eq!(decompress(&packed, len).unwrap(), body, "length {len}");
        }
        let zeros = vec![0u8; 200_000];
        let packed = compress(&zeros);
        assert!(packed.len() < 2000);
        assert_eq!(decompress(&packed, zeros.len()).unwrap(), zeros);
    }

    #[test]
    fn damage_is_an_error() {
        let body = sample(4000);
        let packed = compress(&body);
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
        // a huge stated size reserves nothing by itself
        assert!(decompress(&[17, 0, 0], usize::MAX).is_err());
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
        assert_eq!(&*unpack_entry(&compress(&body), 4000).unwrap(), &body[..]);
    }
}
