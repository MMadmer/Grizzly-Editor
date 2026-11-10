//! The release package's ZIP (MOD_UPDATES 2.3): stored or deflated entries, ZIP64 where
//! needed, no encryption, no spanning, UTF-8 names flagged. The writer is the shape minizip
//! gave the C++ editor (made by 63 - the FAT host of spec 6.3, the one that defines UTF-8 names;
//! level 9 raw deflate; ZIP64 for an entry of 0xFF000000 bytes and up). The reader walks the
//! central directory and every local header itself, so the file index states where the data
//! REALLY sits in the finished archive.

use std::fs::File;
use std::io::{self, BufWriter, Read, Seek, SeekFrom, Write};
use std::path::Path;

use flate2::Compression;
use flate2::write::DeflateEncoder;

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
const END64_SIG: u32 = 0x0606_4b50;
const LOCATOR64_SIG: u32 = 0x0706_4b50;
const MADE_BY: u16 = 63;
const UTF8_FLAG: u16 = 1 << 11;
/// minizip's "maximum compression" bit for level 8 and 9.
const MAX_FLAG: u16 = 1 << 1;
const DEFLATED: u16 = 8;
/// An entry of this size and up is written as ZIP64, with room for data deflate fails to shrink.
pub const ZIP64_FROM: u64 = 0xFF00_0000;
const MAX32: u64 = 0xFFFF_FFFF;

/// One entry, compressed: what the writer is handed.
pub struct Packed {
    /// The name inside the archive, UTF-8.
    pub name: String,
    /// MS-DOS date (high half) and time (low half).
    pub dos_time: u32,
    pub crc: u32,
    pub size: u64,
    /// The raw deflate stream.
    pub data: Vec<u8>,
}

struct Central {
    name: String,
    flags: u16,
    zip64: bool,
    dos_time: u32,
    crc: u32,
    size: u64,
    packed: u64,
    offset: u64,
}

/// Raw deflate at level 9 of a whole buffer.
pub fn deflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::with_capacity(data.len() / 2 + 64), Compression::new(9));
    enc.write_all(data)?;
    enc.finish()
}

/// Counts what passes through.
struct Counting<W: Write> {
    inner: W,
    count: u64,
}

impl<W: Write> Write for Counting<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.count += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

/// Writes a package, entry after entry, then its central directory.
pub struct ZipWriter {
    out: BufWriter<File>,
    pos: u64,
    entries: Vec<Central>,
}

fn flags_of(name: &str) -> u16 {
    MAX_FLAG | if name.is_ascii() { 0 } else { UTF8_FLAG }
}

fn u16_len(n: usize, what: &str) -> io::Result<u16> {
    u16::try_from(n).map_err(|_| io::Error::other(format!("{what} is too long for a ZIP entry")))
}

fn local_header(name: &str, flags: u16, zip64: bool, dos_time: u32, crc: u32, size: u64, packed: u64) -> io::Result<Vec<u8>> {
    let mut h = Vec::with_capacity(30 + name.len() + 20);
    h.extend_from_slice(&LOCAL_SIG.to_le_bytes());
    h.extend_from_slice(&(if zip64 { 45u16 } else { 20 }).to_le_bytes());
    h.extend_from_slice(&flags.to_le_bytes());
    h.extend_from_slice(&DEFLATED.to_le_bytes());
    h.extend_from_slice(&dos_time.to_le_bytes());
    h.extend_from_slice(&crc.to_le_bytes());
    if zip64 {
        h.extend_from_slice(&u32::MAX.to_le_bytes());
        h.extend_from_slice(&u32::MAX.to_le_bytes());
    } else {
        h.extend_from_slice(&(packed.min(MAX32) as u32).to_le_bytes());
        h.extend_from_slice(&(size.min(MAX32) as u32).to_le_bytes());
    }
    h.extend_from_slice(&u16_len(name.len(), "a file name")?.to_le_bytes());
    h.extend_from_slice(&(if zip64 { 20u16 } else { 0 }).to_le_bytes());
    h.extend_from_slice(name.as_bytes());
    if zip64 {
        h.extend_from_slice(&1u16.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(&size.to_le_bytes());
        h.extend_from_slice(&packed.to_le_bytes());
    }
    Ok(h)
}

impl ZipWriter {
    pub fn create(path: &Path) -> io::Result<Self> {
        Ok(Self { out: BufWriter::with_capacity(1 << 20, File::create(path)?), pos: 0, entries: Vec::new() })
    }

    fn put(&mut self, bytes: &[u8]) -> io::Result<()> {
        self.out.write_all(bytes)?;
        self.pos += bytes.len() as u64;
        Ok(())
    }

    /// An entry compressed beforehand.
    pub fn add(&mut self, e: &Packed) -> io::Result<()> {
        let flags = flags_of(&e.name);
        let zip64 = e.size >= ZIP64_FROM || e.data.len() as u64 >= MAX32;
        let offset = self.pos;
        let header = local_header(&e.name, flags, zip64, e.dos_time, e.crc, e.size, e.data.len() as u64)?;
        self.put(&header)?;
        self.put(&e.data)?;
        self.entries.push(Central { name: e.name.clone(), flags, zip64, dos_time: e.dos_time, crc: e.crc, size: e.size, packed: e.data.len() as u64, offset });
        Ok(())
    }

    /// An entry deflated as it is read - a file too large to hold in memory. `feed` sees every
    /// piece of the content (the hash); the header is completed once the data is written.
    /// Answers the bytes read.
    pub fn add_stream(&mut self, name: &str, dos_time: u32, size_hint: u64, src: &mut dyn Read, feed: &mut dyn FnMut(&[u8]) -> io::Result<()>) -> io::Result<u64> {
        let flags = flags_of(name);
        let zip64 = size_hint >= ZIP64_FROM;
        let offset = self.pos;
        let placeholder = local_header(name, flags, zip64, dos_time, 0, 0, 0)?;
        self.put(&placeholder)?;
        self.out.flush()?;
        let mut crc = crc32fast::Hasher::new();
        let mut read = 0u64;
        let packed = {
            let counting = Counting { inner: &mut self.out, count: 0 };
            let mut enc = DeflateEncoder::new(counting, Compression::new(9));
            let mut buf = vec![0u8; 1 << 20];
            loop {
                let n = src.read(&mut buf)?;
                if n == 0 {
                    break;
                }
                let piece = buf.get(..n).unwrap_or(&[]);
                crc.update(piece);
                feed(piece)?;
                enc.write_all(piece)?;
                read += n as u64;
            }
            enc.finish()?.count
        };
        if !zip64 && (read >= ZIP64_FROM || packed >= MAX32) {
            return Err(io::Error::other(format!("{name} grew past the size it was listed with")));
        }
        self.pos += packed;
        let crc = crc.finalize();
        // the header again, now with what was written
        let header = local_header(name, flags, zip64, dos_time, crc, read, packed)?;
        self.out.flush()?;
        let file = self.out.get_mut();
        file.seek(SeekFrom::Start(offset))?;
        file.write_all(&header)?;
        file.seek(SeekFrom::Start(self.pos))?;
        self.entries.push(Central { name: name.to_owned(), flags, zip64, dos_time, crc, size: read, packed, offset });
        Ok(read)
    }

    /// The central directory and its end records; the file is flushed and closed.
    pub fn finish(mut self) -> io::Result<()> {
        let cd_start = self.pos;
        let entries = std::mem::take(&mut self.entries);
        for e in &entries {
            let mut extra: Vec<u8> = Vec::new();
            let size32 = if e.zip64 || e.size >= MAX32 { u32::MAX } else { e.size as u32 };
            let packed32 = if e.zip64 || e.packed >= MAX32 { u32::MAX } else { e.packed as u32 };
            let offset32 = if e.offset >= MAX32 { u32::MAX } else { e.offset as u32 };
            if size32 == u32::MAX {
                extra.extend_from_slice(&e.size.to_le_bytes());
            }
            if packed32 == u32::MAX {
                extra.extend_from_slice(&e.packed.to_le_bytes());
            }
            if offset32 == u32::MAX {
                extra.extend_from_slice(&e.offset.to_le_bytes());
            }
            let mut h = Vec::with_capacity(46 + e.name.len() + extra.len() + 4);
            h.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            h.extend_from_slice(&MADE_BY.to_le_bytes());
            h.extend_from_slice(&(if e.zip64 || !extra.is_empty() { 45u16 } else { 20 }).to_le_bytes());
            h.extend_from_slice(&e.flags.to_le_bytes());
            h.extend_from_slice(&DEFLATED.to_le_bytes());
            h.extend_from_slice(&e.dos_time.to_le_bytes());
            h.extend_from_slice(&e.crc.to_le_bytes());
            h.extend_from_slice(&packed32.to_le_bytes());
            h.extend_from_slice(&size32.to_le_bytes());
            h.extend_from_slice(&u16_len(e.name.len(), "a file name")?.to_le_bytes());
            let extra_len = if extra.is_empty() { 0 } else { extra.len() + 4 };
            h.extend_from_slice(&u16_len(extra_len, "a ZIP64 record")?.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes()); // comment
            h.extend_from_slice(&0u16.to_le_bytes()); // disk
            h.extend_from_slice(&0u16.to_le_bytes()); // internal attributes
            h.extend_from_slice(&0u32.to_le_bytes()); // external attributes
            h.extend_from_slice(&offset32.to_le_bytes());
            h.extend_from_slice(e.name.as_bytes());
            if !extra.is_empty() {
                h.extend_from_slice(&1u16.to_le_bytes());
                h.extend_from_slice(&u16_len(extra.len(), "a ZIP64 record")?.to_le_bytes());
                h.extend_from_slice(&extra);
            }
            self.put(&h)?;
        }
        let cd_size = self.pos - cd_start;
        let count = entries.len() as u64;
        let zip64 = count >= 0xFFFF || cd_start >= MAX32 || cd_size >= MAX32;
        if zip64 {
            let end64 = self.pos;
            let mut r = Vec::with_capacity(76);
            r.extend_from_slice(&END64_SIG.to_le_bytes());
            r.extend_from_slice(&44u64.to_le_bytes());
            r.extend_from_slice(&45u16.to_le_bytes());
            r.extend_from_slice(&45u16.to_le_bytes());
            r.extend_from_slice(&0u32.to_le_bytes());
            r.extend_from_slice(&0u32.to_le_bytes());
            r.extend_from_slice(&count.to_le_bytes());
            r.extend_from_slice(&count.to_le_bytes());
            r.extend_from_slice(&cd_size.to_le_bytes());
            r.extend_from_slice(&cd_start.to_le_bytes());
            r.extend_from_slice(&LOCATOR64_SIG.to_le_bytes());
            r.extend_from_slice(&0u32.to_le_bytes());
            r.extend_from_slice(&end64.to_le_bytes());
            r.extend_from_slice(&1u32.to_le_bytes());
            self.put(&r)?;
        }
        let mut end = Vec::with_capacity(22);
        end.extend_from_slice(&END_SIG.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        let count16 = if zip64 { 0xFFFF } else { count as u16 };
        end.extend_from_slice(&count16.to_le_bytes());
        end.extend_from_slice(&count16.to_le_bytes());
        end.extend_from_slice(&(if zip64 { u32::MAX } else { cd_size as u32 }).to_le_bytes());
        end.extend_from_slice(&(if zip64 { u32::MAX } else { cd_start as u32 }).to_le_bytes());
        end.extend_from_slice(&0u16.to_le_bytes());
        self.put(&end)?;
        self.out.flush()?;
        self.out.get_ref().sync_all()
    }
}

/// An entry as the finished archive holds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stored {
    /// The name, UTF-8 (lossy for bytes that are none).
    pub name: String,
    pub flags: u16,
    pub method: u16,
    pub size: u64,
    pub packed: u64,
    /// The first byte of the stored data: past the LOCAL header's name and extra field.
    pub data_offset: u64,
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn le64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).map(|x| u64::from_le_bytes([x[0], x[1], x[2], x[3], x[4], x[5], x[6], x[7]]))
}

fn bad(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

fn read_at(file: &mut File, at: u64, len: usize) -> io::Result<Vec<u8>> {
    file.seek(SeekFrom::Start(at))?;
    let mut buf = vec![0u8; len];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

/// Every entry of a finished archive, in central directory order, with the data offset read
/// from its local header.
pub fn read_layout(path: &Path) -> io::Result<Vec<Stored>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    if len < 22 {
        return Err(bad("too short for a ZIP archive"));
    }
    let tail_len = len.min(22 + 0xFFFF + 20);
    let tail = read_at(&mut file, len - tail_len, tail_len as usize)?;
    let end_at = (0..=tail.len().saturating_sub(22)).rev().find(|&i| le32(&tail, i) == Some(END_SIG)).ok_or_else(|| bad("no end of central directory"))?;
    let mut count = u64::from(le16(&tail, end_at + 10).unwrap_or(0));
    let mut cd_size = u64::from(le32(&tail, end_at + 12).unwrap_or(0));
    let mut cd_start = u64::from(le32(&tail, end_at + 16).unwrap_or(0));
    if count == 0xFFFF || cd_size == MAX32 || cd_start == MAX32 {
        let loc = end_at.checked_sub(20).ok_or_else(|| bad("no ZIP64 locator"))?;
        if le32(&tail, loc) != Some(LOCATOR64_SIG) {
            return Err(bad("no ZIP64 locator"));
        }
        let end64 = le64(&tail, loc + 8).ok_or_else(|| bad("no ZIP64 locator"))?;
        if end64.checked_add(56).is_none_or(|e| e > len) {
            return Err(bad("the ZIP64 end record lies outside the file"));
        }
        let rec = read_at(&mut file, end64, 56)?;
        if le32(&rec, 0) != Some(END64_SIG) {
            return Err(bad("no ZIP64 end record"));
        }
        count = le64(&rec, 32).unwrap_or(0);
        cd_size = le64(&rec, 40).unwrap_or(0);
        cd_start = le64(&rec, 48).unwrap_or(0);
    }
    if cd_start.checked_add(cd_size).is_none_or(|e| e > len) || count > cd_size / 46 {
        return Err(bad("the central directory lies outside the file"));
    }
    let cd = read_at(&mut file, cd_start, cd_size as usize)?;
    let mut out = Vec::with_capacity(count as usize);
    let mut at = 0usize;
    for _ in 0..count {
        if le32(&cd, at) != Some(CENTRAL_SIG) {
            return Err(bad("a central directory entry is damaged"));
        }
        let field16 = |o: usize| le16(&cd, at + o).ok_or_else(|| bad("a central directory entry is cut short"));
        let field32 = |o: usize| le32(&cd, at + o).ok_or_else(|| bad("a central directory entry is cut short"));
        let (flags, method) = (field16(8)?, field16(10)?);
        let (mut packed, mut size) = (u64::from(field32(20)?), u64::from(field32(24)?));
        let (name_len, extra_len, comment_len) = (usize::from(field16(28)?), usize::from(field16(30)?), usize::from(field16(32)?));
        let mut offset = u64::from(field32(42)?);
        let name_bytes = cd.get(at + 46..at + 46 + name_len).ok_or_else(|| bad("a central directory entry is cut short"))?;
        let extra = cd.get(at + 46 + name_len..at + 46 + name_len + extra_len).ok_or_else(|| bad("a central directory entry is cut short"))?;
        // ZIP64: the fields written as 0xFFFFFFFF follow in this order
        let mut x = 0usize;
        while x + 4 <= extra.len() {
            let (id, n) = (le16(extra, x).unwrap_or(0), usize::from(le16(extra, x + 2).unwrap_or(0)));
            if id == 1 {
                let mut v = x + 4;
                let mut take = |field: &mut u64| {
                    if *field == MAX32
                        && let Some(value) = le64(extra, v)
                    {
                        *field = value;
                        v += 8;
                    }
                };
                take(&mut size);
                take(&mut packed);
                take(&mut offset);
            }
            x += 4 + n;
        }
        at += 46 + name_len + extra_len + comment_len;

        if offset.checked_add(30).is_none_or(|e| e > len) {
            return Err(bad("a local header lies outside the file"));
        }
        let local = read_at(&mut file, offset, 30)?;
        if le32(&local, 0) != Some(LOCAL_SIG) {
            return Err(bad("a local header is damaged"));
        }
        let data_offset = offset + 30 + u64::from(le16(&local, 26).unwrap_or(0)) + u64::from(le16(&local, 28).unwrap_or(0));
        if data_offset.checked_add(packed).is_none_or(|e| e > len) {
            return Err(bad("an entry's data lies outside the file"));
        }
        out.push(Stored { name: String::from_utf8_lossy(name_bytes).into_owned(), flags, method, size, packed, data_offset });
    }
    Ok(out)
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn written_entries_read_back() {
        let dir = std::env::temp_dir().join(format!("gz-release-zip-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("t.zip");
        let a = b"hello hello hello hello".to_vec();
        let b: Vec<u8> = (0..5000u32).map(|i| (i * 7) as u8).collect();
        let mut w = ZipWriter::create(&path).unwrap();
        let mut crc = crc32fast::Hasher::new();
        crc.update(&a);
        w.add(&Packed { name: "modules/od/a.txt".into(), dos_time: 0x0021_0000, crc: crc.finalize(), size: a.len() as u64, data: deflate(&a).unwrap() }).unwrap();
        let mut seen = 0usize;
        let n = w.add_stream("modules/od/\u{43f}/b.bin", 0x0021_0000, b.len() as u64, &mut Cursor::new(b.clone()), &mut |p| {
            seen += p.len();
            Ok(())
        });
        assert_eq!(n.unwrap(), b.len() as u64);
        assert_eq!(seen, b.len());
        w.finish().unwrap();

        let layout = read_layout(&path).unwrap();
        assert_eq!(layout.len(), 2);
        let bytes = std::fs::read(&path).unwrap();
        for (entry, content) in layout.iter().zip([&a, &b]) {
            assert_eq!(entry.method, 8);
            assert_eq!(entry.size, content.len() as u64);
            let data = &bytes[entry.data_offset as usize..(entry.data_offset + entry.packed) as usize];
            let mut out = Vec::new();
            flate2::read::DeflateDecoder::new(data).read_to_end(&mut out).unwrap();
            assert_eq!(&out, content);
        }
        assert_eq!(layout[0].flags & UTF8_FLAG, 0);
        assert_eq!(layout[1].flags & UTF8_FLAG, UTF8_FLAG);
        assert_eq!(layout[1].name, "modules/od/\u{43f}/b.bin");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
