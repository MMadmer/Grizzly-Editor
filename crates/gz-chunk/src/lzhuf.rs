//! LZHUF, the compression of a chunk whose id carries the compressed flag
//! (`_compressLZ` / `_decompressLZ` of the engine, `XrCore/LzHuf.cpp`).
//!
//! The stream is `u32` unpacked size, then LZSS (4096 byte window, matches of 3..60 bytes) coded
//! with an adaptive Huffman tree. This is the framing of every compressed chunk, of the
//! directory chunk of a game archive and of `IWriter::w_compressed`.
//!
//! - [`decompress`] unpacks a stream; [`decompress_limited`] also refuses a stated size above
//!   the caller's own limit (the game passes the size of the archive file).
//! - [`compress`] packs; the bytes equal the engine's for the same input.
//! - [`max_unpacked_len`] is the sanity limit: what a stream of that many bytes can hold at most.
//!
//! Reading is bounded: the stated size is checked against what the input can produce before
//! anything is reserved, every bit read is counted against the bits that exist, and a stream
//! that asks for more is an error.

use crate::{ChunkError, Result};

/// The window.
const N: usize = 4096;
/// The longest match.
const F: usize = 60;
/// Matches are longer than this.
const THRESHOLD: usize = 2;
const NIL: usize = N;
/// Character codes: 256 bytes and the match lengths.
const N_CHAR: usize = 256 - THRESHOLD + F;
/// Size of the Huffman table.
const T: usize = N_CHAR * 2 - 1;
/// The root.
const R: usize = T - 1;
const MAX_FREQ: u32 = 0x4000;

/// Code lengths of the upper six bits of a match position.
const P_LEN: [u8; 64] = [
    3, 4, 4, 4, 5, 5, 5, 5, 5, 5, 5, 5, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 6, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 7, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8, 8,
];

struct PositionTables {
    p_code: [u8; 64],
    d_code: [u8; 256],
    d_len: [u8; 256],
}

/// `p_code`, `d_code` and `d_len` of the C source follow from `p_len`: the codes are a
/// canonical prefix code in table order.
const fn position_tables() -> PositionTables {
    let mut t = PositionTables { p_code: [0; 64], d_code: [0; 256], d_len: [0; 256] };
    let mut code = 0usize;
    let mut c = 0usize;
    while c < 64 {
        t.p_code[c] = code as u8;
        let span = 1usize << (8 - P_LEN[c]);
        let mut k = 0;
        while k < span {
            t.d_code[code + k] = c as u8;
            t.d_len[code + k] = P_LEN[c];
            k += 1;
        }
        code += span;
        c += 1;
    }
    t
}

const TABLES: PositionTables = position_tables();

/// The adaptive Huffman tree (`freq`, `prnt`, `son`).
struct Huff {
    freq: [u32; T + 1],
    /// Parents; `[T..T + N_CHAR)` map a character to its leaf.
    prnt: [usize; T + N_CHAR],
    son: [usize; T],
}

impl Huff {
    /// `StartHuff`.
    fn new() -> Self {
        let mut h = Huff { freq: [0; T + 1], prnt: [0; T + N_CHAR], son: [0; T] };
        for i in 0..N_CHAR {
            h.freq[i] = 1;
            h.son[i] = i + T;
            h.prnt[i + T] = i;
        }
        let (mut i, mut j) = (0, N_CHAR);
        while j <= R {
            h.freq[j] = h.freq[i] + h.freq[i + 1];
            h.son[j] = i;
            h.prnt[i] = j;
            h.prnt[i + 1] = j;
            i += 2;
            j += 1;
        }
        h.freq[T] = 0xffff;
        h.prnt[R] = 0;
        h
    }

    /// `reconst`: halves the frequencies and rebuilds the tree.
    fn reconst(&mut self) {
        let mut j = 0;
        for i in 0..T {
            if self.son[i] >= T {
                self.freq[j] = self.freq[i].div_ceil(2);
                self.son[j] = self.son[i];
                j += 1;
            }
        }
        let (mut i, mut j) = (0, N_CHAR);
        while j < T {
            let f = self.freq[i] + self.freq[i + 1];
            self.freq[j] = f;
            let mut k = j;
            while k > 0 && f < self.freq[k - 1] {
                k -= 1;
            }
            self.freq.copy_within(k..j, k + 1);
            self.freq[k] = f;
            self.son.copy_within(k..j, k + 1);
            self.son[k] = i;
            i += 2;
            j += 1;
        }
        for i in 0..T {
            let k = self.son[i];
            self.prnt[k] = i;
            if k < T {
                self.prnt[k + 1] = i;
            }
        }
    }

    /// `update`: counts a character and keeps the tree ordered.
    fn update(&mut self, c: usize) {
        if self.freq[R] == MAX_FREQ {
            self.reconst();
        }
        let mut c = self.prnt[c + T];
        loop {
            self.freq[c] += 1;
            let k = self.freq[c];
            let mut l = c + 1;
            if k > self.freq[l] {
                // freq[T] is a sentinel no count reaches
                while l < T && k > self.freq[l + 1] {
                    l += 1;
                }
                self.freq[c] = self.freq[l];
                self.freq[l] = k;

                let i = self.son[c];
                self.prnt[i] = l;
                if i < T {
                    self.prnt[i + 1] = l;
                }
                let j = self.son[l];
                self.son[l] = i;
                self.prnt[j] = c;
                if j < T {
                    self.prnt[j + 1] = c;
                }
                self.son[c] = j;
                c = l;
            }
            c = self.prnt[c];
            if c == 0 {
                break;
            }
        }
    }

    /// `EncodeChar`.
    fn encode_char(&mut self, w: &mut BitWriter, c: usize) {
        let mut code: u32 = 0;
        let mut len: u32 = 0;
        let mut k = self.prnt[c + T];
        loop {
            code >>= 1;
            if k & 1 != 0 {
                code += 0x8000;
            }
            len += 1;
            k = self.prnt[k];
            if k == R {
                break;
            }
        }
        w.put_code(len, code);
        self.update(c);
    }

    /// `DecodeChar`.
    fn decode_char(&mut self, r: &mut BitReader<'_>) -> Result<usize> {
        let mut c = self.son[R];
        while c < T {
            c += r.bit()? as usize;
            c = match self.son.get(c) {
                Some(&next) => next,
                None => return Err(ChunkError::Compression("the LZHUF tree is damaged")),
            };
        }
        c -= T;
        if c >= N_CHAR {
            return Err(ChunkError::Compression("the LZHUF tree is damaged"));
        }
        self.update(c);
        Ok(c)
    }
}

/// `PutCode` / `PutFlush`. `putbuf` is 32 bits wide as in the engine's build; only its low 16
/// bits ever reach the output.
struct BitWriter {
    out: Vec<u8>,
    putbuf: u32,
    putlen: u32,
}

impl BitWriter {
    fn put_code(&mut self, len: u32, code: u32) {
        self.putbuf |= code.wrapping_shr(self.putlen);
        self.putlen += len;
        if self.putlen >= 8 {
            self.out.push((self.putbuf >> 8) as u8);
            self.putlen -= 8;
            if self.putlen >= 8 {
                self.out.push(self.putbuf as u8);
                self.putlen -= 8;
                self.putbuf = code.wrapping_shl(len.wrapping_sub(self.putlen));
            } else {
                self.putbuf = self.putbuf.wrapping_shl(8);
            }
        }
    }

    fn flush(&mut self) {
        if self.putlen != 0 {
            self.out.push((self.putbuf >> 8) as u8);
        }
    }
}

/// `EncodePosition`.
fn encode_position(w: &mut BitWriter, pos: usize) {
    let hi = (pos >> 6) & 63;
    w.put_code(u32::from(P_LEN[hi]), u32::from(TABLES.p_code[hi]) << 8);
    w.put_code(6, ((pos & 0x3f) as u32) << 10);
}

/// `GetBit` / `GetByte`. The engine reads zeros past the end of the input and never notices; a
/// stream that uses a bit it does not have is an error here.
struct BitReader<'a> {
    data: &'a [u8],
    pos: usize,
    getbuf: u32,
    getlen: u32,
    /// Bits of real input not consumed yet.
    left: u64,
}

impl<'a> BitReader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0, getbuf: 0, getlen: 0, left: data.len() as u64 * 8 }
    }

    fn fill(&mut self) {
        while self.getlen <= 8 {
            let byte = self.data.get(self.pos).copied().unwrap_or(0);
            self.pos = self.pos.saturating_add(1);
            self.getbuf |= u32::from(byte) << (8 - self.getlen);
            self.getlen += 8;
        }
    }

    fn take(&mut self, bits: u32) -> Result<()> {
        match self.left.checked_sub(u64::from(bits)) {
            Some(left) => {
                self.left = left;
                Ok(())
            }
            None => Err(ChunkError::Compression("the LZHUF stream ends early")),
        }
    }

    fn bit(&mut self) -> Result<u32> {
        self.take(1)?;
        self.fill();
        let v = self.getbuf;
        self.getbuf <<= 1;
        self.getlen -= 1;
        Ok((v & 0x8000) >> 15)
    }

    fn byte(&mut self) -> Result<u32> {
        self.take(8)?;
        self.fill();
        let v = self.getbuf;
        self.getbuf <<= 8;
        self.getlen -= 8;
        Ok((v & 0xff00) >> 8)
    }
}

/// `DecodePosition`.
fn decode_position(r: &mut BitReader<'_>) -> Result<usize> {
    let mut i = r.byte()?;
    let c = u32::from(TABLES.d_code[(i & 0xff) as usize]) << 6;
    let mut extra = u32::from(TABLES.d_len[(i & 0xff) as usize]) - 2;
    while extra > 0 {
        extra -= 1;
        i = (i << 1) + r.bit()?;
    }
    Ok((c | (i & 0x3f)) as usize)
}

/// The LZSS dictionary: a ring buffer and the binary search trees over its strings.
struct Lzss {
    /// The ring, with its first `F - 1` bytes repeated after the end so that a string can be
    /// compared without wrapping.
    text_buf: Vec<u8>,
    lson: Vec<usize>,
    rson: Vec<usize>,
    dad: Vec<usize>,
    match_position: usize,
    match_length: usize,
}

impl Lzss {
    /// The state of a fresh process plus `InitTree`.
    fn new() -> Self {
        let mut z = Lzss {
            text_buf: vec![0; N + F],
            lson: vec![0; N + 257],
            rson: vec![0; N + 257],
            dad: vec![0; N + 257],
            match_position: 0,
            match_length: 0,
        };
        for root in &mut z.rson[N + 1..=N + 256] {
            *root = NIL;
        }
        for node in &mut z.dad[..N] {
            *node = NIL;
        }
        z
    }

    /// `InsertNode`: adds the string at `r` to its tree and finds the longest match for it.
    fn insert_node(&mut self, r: usize) {
        let mut cmp: i32 = 1;
        let mut p = N + 1 + usize::from(self.text_buf[r]);
        self.rson[r] = NIL;
        self.lson[r] = NIL;
        self.match_length = 0;
        loop {
            if cmp >= 0 {
                if self.rson[p] != NIL {
                    p = self.rson[p];
                } else {
                    self.rson[p] = r;
                    self.dad[r] = p;
                    return;
                }
            } else if self.lson[p] != NIL {
                p = self.lson[p];
            } else {
                self.lson[p] = r;
                self.dad[r] = p;
                return;
            }
            let mut i = 1;
            while i < F {
                cmp = i32::from(self.text_buf[r + i]) - i32::from(self.text_buf[p + i]);
                if cmp != 0 {
                    break;
                }
                i += 1;
            }
            if i > THRESHOLD {
                let distance = (r.wrapping_sub(p) & (N - 1)).wrapping_sub(1) & (N - 1);
                if i > self.match_length {
                    self.match_position = distance;
                    self.match_length = i;
                    if i >= F {
                        break;
                    }
                }
                if i == self.match_length && distance < self.match_position {
                    self.match_position = distance;
                }
            }
        }
        // the string at p is the same for F bytes: r takes its place
        self.dad[r] = self.dad[p];
        self.lson[r] = self.lson[p];
        self.rson[r] = self.rson[p];
        let (left, right, parent) = (self.lson[p], self.rson[p], self.dad[p]);
        self.dad[left] = r;
        self.dad[right] = r;
        if self.rson[parent] == p {
            self.rson[parent] = r;
        } else {
            self.lson[parent] = r;
        }
        self.dad[p] = NIL;
    }

    /// `DeleteNode`.
    fn delete_node(&mut self, p: usize) {
        if self.dad[p] == NIL {
            return;
        }
        let q;
        if self.rson[p] == NIL {
            q = self.lson[p];
        } else if self.lson[p] == NIL {
            q = self.rson[p];
        } else {
            let mut m = self.lson[p];
            if self.rson[m] != NIL {
                while self.rson[m] != NIL {
                    m = self.rson[m];
                }
                let (m_parent, m_left) = (self.dad[m], self.lson[m]);
                self.rson[m_parent] = m_left;
                self.dad[m_left] = m_parent;
                self.lson[m] = self.lson[p];
                let p_left = self.lson[p];
                self.dad[p_left] = m;
            }
            self.rson[m] = self.rson[p];
            let p_right = self.rson[p];
            self.dad[p_right] = m;
            q = m;
        }
        self.dad[q] = self.dad[p];
        let parent = self.dad[p];
        if self.rson[parent] == p {
            self.rson[parent] = q;
        } else {
            self.lson[parent] = q;
        }
        self.dad[p] = NIL;
    }
}

/// What a stream of `packed_len` bytes can unpack to at most: every code is at least one bit
/// and gives at most 60 bytes.
#[must_use]
pub fn max_unpacked_len(packed_len: usize) -> usize {
    packed_len.saturating_sub(4).saturating_mul(8 * F)
}

/// Unpacks a compressed chunk body.
///
/// The size comes from the stream's header and is refused when it is above
/// [`max_unpacked_len`] of the input, so a damaged header cannot ask for gigabytes.
pub fn decompress(packed: &[u8]) -> Result<Vec<u8>> {
    decompress_limited(packed, usize::MAX)
}

/// [`decompress`] with the caller's own ceiling on the unpacked size.
pub fn decompress_limited(packed: &[u8], limit: usize) -> Result<Vec<u8>> {
    let Some((head, data)) = packed.split_first_chunk::<4>() else {
        return Err(ChunkError::Compression("the LZHUF stream is shorter than its size field"));
    };
    let size = u32::from_le_bytes(*head) as usize;
    if size == 0 {
        return Ok(Vec::new());
    }
    if size > limit || size > max_unpacked_len(packed.len()) {
        return Err(ChunkError::Compression("the LZHUF stream states a size it cannot hold"));
    }

    let mut out = Vec::with_capacity(size);
    let mut huff = Huff::new();
    let mut bits = BitReader::new(data);
    // oracle: the engine leaves the last F bytes of the window as an earlier call left them; a
    // stream its encoder wrote never reads them, so they are zero here.
    let mut window = [0u8; N];
    for b in &mut window[..N - F] {
        *b = 0x20;
    }
    let mut r = N - F;
    while out.len() < size {
        let c = huff.decode_char(&mut bits)?;
        if c < 256 {
            out.push(c as u8);
            window[r] = c as u8;
            r = (r + 1) & (N - 1);
        } else {
            let from = r.wrapping_sub(decode_position(&mut bits)?).wrapping_sub(1) & (N - 1);
            let len = c - 255 + THRESHOLD;
            if len > size - out.len() {
                return Err(ChunkError::Compression("an LZHUF match runs past the stated size"));
            }
            for k in 0..len {
                let b = window[(from + k) & (N - 1)];
                out.push(b);
                window[r] = b;
                r = (r + 1) & (N - 1);
            }
        }
    }
    Ok(out)
}

/// Packs a chunk body. The output is byte-equal to the engine's for the same input.
///
/// The size field is 32 bits, as in the engine: a body of 4 GiB or more cannot be packed and
/// would be read back short.
///
/// For a body shorter than 119 bytes the engine's own output depends on what an earlier call
/// left in its static window; this is the output of a process that packs for the first time.
#[must_use]
pub fn compress(body: &[u8]) -> Vec<u8> {
    let mut w = BitWriter { out: Vec::with_capacity(body.len() / 2 + 16), putbuf: 0, putlen: 0 };
    w.out.extend_from_slice(&(body.len() as u32).to_le_bytes());
    if body.is_empty() {
        return w.out;
    }
    let mut huff = Huff::new();
    let mut lz = Lzss::new();
    let mut input = body.iter().copied();

    let mut s = 0usize;
    let mut r = N - F;
    for b in &mut lz.text_buf[..r] {
        *b = 0x20;
    }
    let mut len = 0usize;
    while len < F {
        let Some(c) = input.next() else { break };
        lz.text_buf[r + len] = c;
        len += 1;
    }
    for i in 1..=F {
        lz.insert_node(r - i);
    }
    lz.insert_node(r);
    loop {
        if lz.match_length > len {
            lz.match_length = len;
        }
        if lz.match_length <= THRESHOLD {
            lz.match_length = 1;
            huff.encode_char(&mut w, usize::from(lz.text_buf[r]));
        } else {
            huff.encode_char(&mut w, 255 - THRESHOLD + lz.match_length);
            encode_position(&mut w, lz.match_position);
        }
        let last = lz.match_length;
        let mut i = 0;
        while i < last {
            let Some(c) = input.next() else { break };
            lz.delete_node(s);
            lz.text_buf[s] = c;
            if s < F - 1 {
                lz.text_buf[s + N] = c;
            }
            s = (s + 1) & (N - 1);
            r = (r + 1) & (N - 1);
            lz.insert_node(r);
            i += 1;
        }
        while i < last {
            i += 1;
            lz.delete_node(s);
            s = (s + 1) & (N - 1);
            r = (r + 1) & (N - 1);
            len = len.saturating_sub(1);
            if len != 0 {
                lz.insert_node(r);
            }
        }
        if len == 0 {
            break;
        }
    }
    w.flush();
    w.out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_tables_match_the_c_source() {
        assert_eq!(&TABLES.p_code[..8], &[0x00, 0x20, 0x30, 0x40, 0x50, 0x58, 0x60, 0x68]);
        assert_eq!(TABLES.p_code[12], 0x90);
        assert_eq!(TABLES.p_code[24], 0xC0);
        assert_eq!(TABLES.p_code[48], 0xF0);
        assert_eq!(TABLES.p_code[63], 0xFF);
        assert_eq!(TABLES.d_code[0x1f], 0x00);
        assert_eq!(TABLES.d_code[0x20], 0x01);
        assert_eq!(TABLES.d_code[0x50], 0x04);
        assert_eq!(TABLES.d_code[0xbf], 0x17);
        assert_eq!(TABLES.d_code[0xc0], 0x18);
        assert_eq!(TABLES.d_code[0xff], 0x3f);
        assert_eq!(TABLES.d_len[0x00], 3);
        assert_eq!(TABLES.d_len[0x4f], 4);
        assert_eq!(TABLES.d_len[0x90], 6);
        assert_eq!(TABLES.d_len[0xef], 7);
        assert_eq!(TABLES.d_len[0xf0], 8);
    }

    // Traced through LzHuf.cpp by hand: with the initial tree the code of 'a' (leaf 97) is the
    // nine bits 111101101, of 'b' after 'a' was counted 111101110, of the length code 257 the
    // eight bits 10001101; position 0 is three zero bits and six zero bits.
    #[test]
    fn known_vectors() {
        assert_eq!(compress(b""), [0, 0, 0, 0]);
        assert_eq!(compress(b"a"), [1, 0, 0, 0, 0xF6, 0x80]);
        assert_eq!(compress(b"ab"), [2, 0, 0, 0, 0xF6, 0xFB, 0x80]);
        assert_eq!(compress(b"    "), [4, 0, 0, 0, 0x8D, 0x00, 0x00]);
        assert_eq!(decompress(&[1, 0, 0, 0, 0xF6, 0x80]).unwrap(), b"a");
        assert_eq!(decompress(&[4, 0, 0, 0, 0x8D, 0x00, 0x00]).unwrap(), b"    ");
    }

    fn sample(len: usize) -> Vec<u8> {
        let mut seed = 0x1234_5678u32;
        (0..len)
            .map(|i| {
                seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
                // runs, repeats and noise, so that literals and matches both occur
                match (i / 97) % 3 {
                    0 => b"section_name = value, "[i % 22],
                    1 => (seed >> 24) as u8,
                    _ => 0,
                }
            })
            .collect()
    }

    #[test]
    fn round_trips() {
        for len in [0, 1, 2, 3, 59, 60, 61, 118, 119, 120, 4095, 4096, 4097, 20_000, 70_000] {
            let body = sample(len);
            let packed = compress(&body);
            assert_eq!(decompress(&packed).unwrap(), body, "length {len}");
        }
        // enough input to make the tree rebuild itself
        let long: Vec<u8> = sample(300_000);
        assert_eq!(decompress(&compress(&long)).unwrap(), long);
    }

    #[test]
    fn damage_is_an_error() {
        assert!(decompress(&[]).is_err());
        assert!(decompress(&[1, 0, 0]).is_err());
        // a size the data cannot hold
        assert!(decompress(&[0xff, 0xff, 0xff, 0x7f, 0x8D, 0, 0]).is_err());
        // cut short
        let packed = compress(&sample(5000));
        assert!(decompress(&packed[..packed.len() / 2]).is_err());
        assert!(decompress(&packed[..4]).is_err());
        // the caller's ceiling
        assert!(decompress_limited(&packed, 4999).is_err());
        assert!(decompress_limited(&packed, 5000).is_ok());
        // a match that runs past the stated size
        assert!(decompress(&[3, 0, 0, 0, 0x8D, 0x00, 0x00]).is_err());
    }
}
