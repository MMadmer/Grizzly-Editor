//! A game archive (`.db*`, `.xdb*`): a chunk file whose chunk 1 is the directory of the files
//! inside it. Only the directory is read when the game mounts;
//! a file's bytes are read when it is asked for.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;

use gz_chunk::{COMPRESSED, lzhuf, lzo};

use crate::path::normalize;

/// One file of an archive's directory, its path made a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchEntry {
    pub path: String,
    pub size: u32,
    pub packed: u32,
    pub offset: u32,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// The body of chunk `id`, found by seeking past the others (an archive is gigabytes; only its
/// directory is wanted). `Ok(None)` when it has no such chunk.
fn read_chunk(file: &mut File, len: u64, id: u32) -> io::Result<Option<Vec<u8>>> {
    let mut pos = 0u64;
    file.seek(SeekFrom::Start(0))?;
    loop {
        if pos + 8 > len {
            return Ok(None);
        }
        let mut head = [0u8; 8];
        file.read_exact(&mut head)?;
        pos += 8;
        let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        if size > len - pos {
            return Err(invalid("a chunk runs past the end of the archive"));
        }
        if kind & !COMPRESSED == id {
            let mut body = vec![0u8; usize::try_from(size).map_err(|_| invalid("a chunk is too large"))?];
            file.read_exact(&mut body)?;
            if kind & COMPRESSED != 0 {
                body = lzhuf::decompress(&body).map_err(|e| invalid(e.to_string()))?;
            }
            return Ok(Some(body));
        }
        pos += size;
        file.seek(SeekFrom::Start(pos))?;
    }
}

/// The directory of the archive at `path`. Folder markers (a name ending in a separator) are
/// left out; a later record of one path replaces an earlier one, as `Register` does.
pub(crate) fn read_directory(path: &Path) -> io::Result<Vec<ArchEntry>> {
    let mut file = File::open(path)?;
    let len = file.metadata()?.len();
    let Some(dir) = read_chunk(&mut file, len, 1)? else { return Err(invalid("no file table")) };
    let records = lzo::read_archive_index(&dir).map_err(|e| invalid(e.to_string()))?;
    let mut out: Vec<ArchEntry> = Vec::with_capacity(records.len());
    let mut at: rustc_hash::FxHashMap<String, usize> = rustc_hash::FxHashMap::default();
    for r in records {
        if r.name.is_empty() || r.name.ends_with(['\\', '/']) {
            continue;
        }
        let key = normalize(&r.name);
        if key.is_empty() {
            continue;
        }
        let entry =
            ArchEntry { path: key.clone(), size: r.size_real, packed: r.size_compressed, offset: r.offset };
        match at.get(&key) {
            Some(&i) => {
                if let Some(slot) = out.get_mut(i) {
                    *slot = entry;
                }
            }
            None => {
                at.insert(key, out.len());
                out.push(entry);
            }
        }
    }
    Ok(out)
}

/// An archive opened for reading a run of its files.
pub(crate) struct Open {
    file: File,
    len: u64,
}

impl Open {
    pub fn new(path: &Path) -> io::Result<Self> {
        let file = File::open(path)?;
        let len = file.metadata()?.len();
        Ok(Self { file, len })
    }

    /// The unpacked bytes of the file stored at `offset`, `packed` bytes that unpack to `size`.
    pub fn read(&mut self, offset: u32, packed: u32, size: u32) -> io::Result<Vec<u8>> {
        if u64::from(offset) + u64::from(packed) > self.len {
            return Err(invalid("the archived file runs past the end of its archive"));
        }
        self.file.seek(SeekFrom::Start(u64::from(offset)))?;
        let mut stored = vec![0u8; packed as usize];
        self.file.read_exact(&mut stored)?;
        if packed == size {
            return Ok(stored);
        }
        // LZO1X never expands by more than 255 bytes per input byte: a larger claim is damage,
        // and must not become an allocation
        if u64::from(size) > u64::from(packed) * 256 + 1024 {
            return Err(invalid("an archived file claims more bytes than its packed data can hold"));
        }
        let bytes = lzo::unpack_entry(&stored, size as usize).map_err(|e| invalid(e.to_string()))?;
        Ok(bytes.into_owned())
    }
}
