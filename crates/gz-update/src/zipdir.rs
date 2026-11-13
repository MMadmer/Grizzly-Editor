//! The archive's directory, read out of the bytes at its end: the end of central directory record,
//! the ZIP64 locator and end record, the central directory, and a local header. Nothing here reads
//! a file or the network - the caller hands the bytes in, from a ranged request or from disk.

const LOCAL_SIG: u32 = 0x0403_4b50;
const CENTRAL_SIG: u32 = 0x0201_4b50;
const END_SIG: u32 = 0x0605_4b50;
const END64_SIG: u32 = 0x0606_4b50;
const LOCATOR64_SIG: u32 = 0x0706_4b50;
const MAX32: u64 = 0xFFFF_FFFF;
/// The end record and the longest comment it may carry, and the ZIP64 locator before it.
pub const TAIL: u64 = 22 + 0xFFFF + 20;
/// The bytes of a ZIP64 end record this reader needs.
pub const END64_LEN: u64 = 56;
pub const STORED: u16 = 0;
pub const DEFLATED: u16 = 8;

/// Where the central directory is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Directory {
    pub offset: u64,
    pub size: u64,
    pub count: u64,
}

/// What the end of the archive said.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum End {
    Found(Directory),
    /// A ZIP64 archive whose end record lies before the bytes given: read [`END64_LEN`] bytes
    /// here and hand them to [`zip64_end`].
    Zip64At(u64),
}

/// One entry of the central directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub flags: u16,
    pub method: u16,
    pub crc: u32,
    pub packed: u64,
    pub size: u64,
    /// Its local header.
    pub offset: u64,
}

impl Entry {
    #[must_use]
    pub fn encrypted(&self) -> bool {
        self.flags & 1 != 0
    }

    /// Stored or deflated, nothing encrypted: what this client unpacks.
    #[must_use]
    pub fn readable(&self) -> bool {
        !self.encrypted() && matches!(self.method, STORED | DEFLATED)
    }
}

fn le16(b: &[u8], at: usize) -> Option<u16> {
    b.get(at..at + 2).map(|x| u16::from_le_bytes([x[0], x[1]]))
}

fn le32(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at + 4).map(|x| u32::from_le_bytes([x[0], x[1], x[2], x[3]]))
}

fn le64(b: &[u8], at: usize) -> Option<u64> {
    b.get(at..at + 8).and_then(|x| x.try_into().ok()).map(u64::from_le_bytes)
}

fn damaged(what: &str) -> String {
    format!("the archive's {what} is damaged")
}

/// Finds the end record in `tail`, the last bytes of an archive of `len` bytes that start at
/// `tail_start`. A comment is allowed; the record is the one whose comment ends the archive.
pub fn find_end(tail: &[u8], tail_start: u64, len: u64) -> Result<End, String> {
    if tail.len() < 22 {
        return Err("the archive is too short to be one".to_owned());
    }
    let at = (0..=tail.len() - 22)
        .rev()
        .find(|&i| {
            le32(tail, i) == Some(END_SIG)
                && le16(tail, i + 20).is_some_and(|c| i + 22 + usize::from(c) == tail.len())
        })
        .ok_or_else(|| "the archive has no end of central directory".to_owned())?;
    let count = u64::from(le16(tail, at + 10).unwrap_or(0));
    let size = u64::from(le32(tail, at + 12).unwrap_or(0));
    let offset = u64::from(le32(tail, at + 16).unwrap_or(0));
    let locator = at.checked_sub(20).filter(|&l| le32(tail, l) == Some(LOCATOR64_SIG));
    let wants64 = count == 0xFFFF || size == MAX32 || offset == MAX32;
    let dir = match locator {
        Some(l) => {
            let end64 = le64(tail, l + 8).ok_or_else(|| damaged("ZIP64 locator"))?;
            if end64.checked_add(END64_LEN).is_none_or(|e| e > len) {
                return Err(damaged("ZIP64 locator"));
            }
            match end64.checked_sub(tail_start).and_then(|o| usize::try_from(o).ok()) {
                Some(o) if o + END64_LEN as usize <= tail.len() => {
                    zip64_end(tail.get(o..o + END64_LEN as usize).unwrap_or(&[]))?
                }
                _ => return Ok(End::Zip64At(end64)),
            }
        }
        None if wants64 => return Err(damaged("ZIP64 locator")),
        None => Directory { offset, size, count },
    };
    if dir.offset.checked_add(dir.size).is_none_or(|e| e > len) || dir.count > dir.size / 46 {
        return Err(damaged("central directory"));
    }
    Ok(End::Found(dir))
}

/// The ZIP64 end record.
pub fn zip64_end(rec: &[u8]) -> Result<Directory, String> {
    if le32(rec, 0) != Some(END64_SIG) {
        return Err(damaged("ZIP64 end record"));
    }
    let field = |o| le64(rec, o).ok_or_else(|| damaged("ZIP64 end record"));
    Ok(Directory { count: field(32)?, size: field(40)?, offset: field(48)? })
}

/// The entries of the central directory.
pub fn directory(cd: &[u8], count: u64) -> Result<Vec<Entry>, String> {
    let mut out = Vec::with_capacity(usize::try_from(count).unwrap_or(0).min(1 << 16));
    let mut at = 0usize;
    let cut = || damaged("central directory");
    for _ in 0..count {
        if le32(cd, at) != Some(CENTRAL_SIG) {
            return Err(cut());
        }
        let f16 = |o: usize| le16(cd, at + o).ok_or_else(cut);
        let f32_ = |o: usize| le32(cd, at + o).ok_or_else(cut);
        let (flags, method, crc) = (f16(8)?, f16(10)?, f32_(16)?);
        let (mut packed, mut size) = (u64::from(f32_(20)?), u64::from(f32_(24)?));
        let (name_len, extra_len, comment_len) =
            (usize::from(f16(28)?), usize::from(f16(30)?), usize::from(f16(32)?));
        let mut offset = u64::from(f32_(42)?);
        let name = cd.get(at + 46..at + 46 + name_len).ok_or_else(cut)?;
        let extra = cd.get(at + 46 + name_len..at + 46 + name_len + extra_len).ok_or_else(cut)?;
        // ZIP64: the fields written as 0xFFFFFFFF follow in this order
        let mut x = 0usize;
        while x + 4 <= extra.len() {
            let (id, n) = (le16(extra, x).unwrap_or(0), usize::from(le16(extra, x + 2).unwrap_or(0)));
            if id == 1 {
                let mut v = x + 4;
                for field in [&mut size, &mut packed, &mut offset] {
                    if *field == MAX32 {
                        *field = le64(extra, v).ok_or_else(cut)?;
                        v += 8;
                    }
                }
            }
            x += 4 + n;
        }
        let name = std::str::from_utf8(name)
            .map_err(|_| "an entry of the archive is not named in UTF-8".to_owned())?;
        out.push(Entry { name: name.to_owned(), flags, method, crc, packed, size, offset });
        at += 46 + name_len + extra_len + comment_len;
    }
    Ok(out)
}

/// Each entry's record: its local header, its data and a data descriptor, up to the next record
/// or the central directory. `None` for an entry whose record overlaps another's.
#[must_use]
pub fn records(entries: &[Entry], dir: &Directory) -> Option<Vec<(u64, u64)>> {
    let mut starts: Vec<u64> = entries.iter().map(|e| e.offset).collect();
    starts.sort_unstable();
    starts.dedup();
    if starts.len() != entries.len() {
        return None;
    }
    entries
        .iter()
        .map(|e| {
            let next = starts.iter().copied().find(|&s| s > e.offset).unwrap_or(dir.offset);
            (e.offset + 30 + e.packed <= next && next <= dir.offset).then_some((e.offset, next))
        })
        .collect()
}

/// The length of a local header (where the data begins), once its first 30 bytes are there.
pub fn local_header_len(head: &[u8]) -> Result<Option<u64>, String> {
    if head.len() < 30 {
        return Ok(None);
    }
    if le32(head, 0) != Some(LOCAL_SIG) {
        return Err(damaged("local header"));
    }
    let n = u64::from(le16(head, 26).unwrap_or(0));
    let m = u64::from(le16(head, 28).unwrap_or(0));
    Ok(Some(30 + n + m))
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod tests {
    use super::*;

    /// A stored archive, made the way any zip writer makes one, with ZIP64 records when `zip64`.
    pub(crate) fn archive(files: &[(&str, &[u8])], comment: &[u8], zip64: bool) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data) in files {
            let offset = out.len() as u32;
            let crc = crc32fast::hash(data);
            out.extend_from_slice(&LOCAL_SIG.to_le_bytes());
            out.extend_from_slice(&[20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&crc.to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(data.len() as u32).to_le_bytes());
            out.extend_from_slice(&(name.len() as u16).to_le_bytes());
            out.extend_from_slice(&3u16.to_le_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(b"xyz");
            out.extend_from_slice(data);
            central.extend_from_slice(&CENTRAL_SIG.to_le_bytes());
            central.extend_from_slice(&[63, 0, 20, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&crc.to_le_bytes());
            if zip64 {
                central.extend_from_slice(&u32::MAX.to_le_bytes());
                central.extend_from_slice(&u32::MAX.to_le_bytes());
            } else {
                central.extend_from_slice(&(data.len() as u32).to_le_bytes());
                central.extend_from_slice(&(data.len() as u32).to_le_bytes());
            }
            central.extend_from_slice(&(name.len() as u16).to_le_bytes());
            central.extend_from_slice(&(if zip64 { 28u16 } else { 0 }).to_le_bytes());
            central.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            central.extend_from_slice(&(if zip64 { u32::MAX } else { offset }).to_le_bytes());
            central.extend_from_slice(name.as_bytes());
            if zip64 {
                central.extend_from_slice(&1u16.to_le_bytes());
                central.extend_from_slice(&24u16.to_le_bytes());
                central.extend_from_slice(&(data.len() as u64).to_le_bytes());
                central.extend_from_slice(&(data.len() as u64).to_le_bytes());
                central.extend_from_slice(&u64::from(offset).to_le_bytes());
            }
        }
        let cd_start = out.len() as u64;
        out.extend_from_slice(&central);
        let count = files.len() as u64;
        if zip64 {
            let end64 = out.len() as u64;
            out.extend_from_slice(&END64_SIG.to_le_bytes());
            out.extend_from_slice(&44u64.to_le_bytes());
            out.extend_from_slice(&[45, 0, 45, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&count.to_le_bytes());
            out.extend_from_slice(&(central.len() as u64).to_le_bytes());
            out.extend_from_slice(&cd_start.to_le_bytes());
            out.extend_from_slice(&LOCATOR64_SIG.to_le_bytes());
            out.extend_from_slice(&0u32.to_le_bytes());
            out.extend_from_slice(&end64.to_le_bytes());
            out.extend_from_slice(&1u32.to_le_bytes());
        }
        out.extend_from_slice(&END_SIG.to_le_bytes());
        out.extend_from_slice(&[0, 0, 0, 0]);
        let c16 = if zip64 { 0xFFFF } else { count as u16 };
        out.extend_from_slice(&c16.to_le_bytes());
        out.extend_from_slice(&c16.to_le_bytes());
        out.extend_from_slice(&(if zip64 { u32::MAX } else { central.len() as u32 }).to_le_bytes());
        out.extend_from_slice(&(if zip64 { u32::MAX } else { cd_start as u32 }).to_le_bytes());
        out.extend_from_slice(&(comment.len() as u16).to_le_bytes());
        out.extend_from_slice(comment);
        out
    }

    fn read(zip: &[u8], tail_len: usize) -> Result<Vec<Entry>, String> {
        let len = zip.len() as u64;
        let start = zip.len().saturating_sub(tail_len);
        let dir = match find_end(&zip[start..], start as u64, len)? {
            End::Found(d) => d,
            End::Zip64At(at) => zip64_end(&zip[at as usize..at as usize + END64_LEN as usize])?,
        };
        directory(&zip[dir.offset as usize..(dir.offset + dir.size) as usize], dir.count)
    }

    #[test]
    fn the_directory_reads_with_and_without_zip64_and_a_comment() {
        let files: [(&str, &[u8]); 3] = [("a.txt", b"alpha"), ("dir/b.bin", &[7u8; 300]), ("c", b"")];
        for zip64 in [false, true] {
            for comment in [&b""[..], b"a comment"] {
                let zip = archive(&files, comment, zip64);
                let entries = read(&zip, TAIL as usize).unwrap();
                assert_eq!(entries.len(), 3);
                assert_eq!(entries[1].name, "dir/b.bin");
                assert_eq!(entries[1].size, 300);
                assert!(entries.iter().all(Entry::readable));
                let len = zip.len() as u64;
                let dir = match find_end(&zip, 0, len).unwrap() {
                    End::Found(d) => d,
                    End::Zip64At(_) => unreachable!(),
                };
                let spans = records(&entries, &dir).unwrap();
                // a record runs from its header to the next one, the last to the directory
                assert_eq!(spans[0].1, spans[1].0);
                assert_eq!(spans[2].1, dir.offset);
                let head = &zip[spans[1].0 as usize..];
                let at = local_header_len(head).unwrap().unwrap() as usize;
                assert_eq!(&zip[spans[1].0 as usize + at..spans[1].0 as usize + at + 300], &[7u8; 300][..]);
                // a tail too short for the ZIP64 end record asks for it
                if zip64 {
                    assert!(matches!(
                        find_end(
                            &zip[zip.len() - 22 - comment.len() - 20..],
                            len - 22 - comment.len() as u64 - 20,
                            len
                        )
                        .unwrap(),
                        End::Zip64At(_)
                    ));
                }
            }
        }
    }

    #[test]
    fn a_wrong_signature_is_refused() {
        let mut zip = archive(&[("a.txt", b"alpha")], b"", false);
        let len = zip.len();
        assert!(find_end(&zip[..len - 1], 0, len as u64 - 1).is_err());
        let cd_at = zip.windows(4).position(|w| w == CENTRAL_SIG.to_le_bytes()).unwrap();
        zip[cd_at] = b'X';
        assert_eq!(read(&zip, TAIL as usize).unwrap_err(), "the archive's central directory is damaged");
        assert!(local_header_len(b"PK\x01\x02................................").is_err());
        assert_eq!(local_header_len(b"PK\x03\x04").unwrap(), None);
        // an encrypted entry or another method is not readable
        let e = Entry { name: "x".into(), flags: 1, method: 8, crc: 0, packed: 0, size: 0, offset: 0 };
        assert!(!e.readable());
        assert!(!Entry { flags: 0, method: 14, ..e }.readable());
    }
}
