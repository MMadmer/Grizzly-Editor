//! One record of the archive on its way into the stage: its bytes as they arrive are kept raw in
//! `<path>.raw` (a resume continues inside the record) and unpacked into `<path>.part`, which is
//! renamed to `<path>` once its size, CRC and SHA-256 are what the manifest and the directory say.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use flate2::{Decompress, FlushDecompress, Status};
use sha2::{Digest, Sha256};

use crate::manifest::Row;
use crate::zipdir::{self, DEFLATED, Entry};

/// Why a record did not make it into the stage.
#[derive(Debug)]
pub enum RecordError {
    /// The disk: worth another try only after the author freed it.
    Io(io::Error),
    /// The bytes are not what the manifest says: refused for good.
    Mismatch(String),
}

impl From<io::Error> for RecordError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl std::fmt::Display for RecordError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "the stage could not be written: {e}"),
            Self::Mismatch(s) => f.write_str(s),
        }
    }
}

/// The paths of a staged file.
pub struct Paths {
    pub done: PathBuf,
    pub part: PathBuf,
    pub raw: PathBuf,
}

impl Paths {
    #[must_use]
    pub fn of(files: &Path, rel: &str) -> Self {
        let done = files.join(rel.replace('/', "\\"));
        let with = |ext: &str| {
            let mut s = done.clone().into_os_string();
            s.push(ext);
            PathBuf::from(s)
        };
        Self { part: with(".part"), raw: with(".raw"), done }
    }
}

/// A record being received.
pub struct Sink {
    row: Row,
    entry: Entry,
    paths: Paths,
    /// The record's length in the archive.
    len: u64,
    raw: Option<File>,
    part: File,
    head: Vec<u8>,
    data_at: Option<u64>,
    received: u64,
    inflate: Decompress,
    /// The inflater may hold output it had no room for.
    pending: bool,
    ended: bool,
    crc: crc32fast::Hasher,
    sha: Sha256,
    out: u64,
    buf: Vec<u8>,
}

impl Sink {
    /// Opens the record of `row`, replaying what an earlier attempt kept. Answers the sink and how
    /// many of the record's bytes are already there.
    pub fn open(files: &Path, row: &Row, entry: &Entry, len: u64) -> Result<Self, RecordError> {
        let paths = Paths::of(files, &row.path);
        if let Some(dir) = paths.done.parent() {
            fs::create_dir_all(dir)?;
        }
        let part = File::create(&paths.part)?;
        let mut sink = Self {
            row: row.clone(),
            entry: entry.clone(),
            paths,
            len,
            raw: None,
            part,
            head: Vec::new(),
            data_at: None,
            received: 0,
            inflate: Decompress::new(false),
            pending: false,
            ended: false,
            crc: crc32fast::Hasher::new(),
            sha: Sha256::new(),
            out: 0,
            buf: vec![0u8; 256 * 1024],
        };
        if let Ok(mut kept) = File::open(&sink.paths.raw) {
            let mut chunk = vec![0u8; 1 << 20];
            loop {
                let n = kept.read(&mut chunk)?;
                if n == 0 {
                    break;
                }
                let piece =
                    chunk.get(..n.min(usize::try_from(len - sink.received).unwrap_or(n))).unwrap_or(&[]);
                if piece.is_empty() {
                    break;
                }
                if let Err(e) = sink.process(piece) {
                    // what was kept does not unpack: the record starts over
                    drop(kept);
                    let _ = fs::remove_file(&sink.paths.raw);
                    return match e {
                        RecordError::Mismatch(_) => Self::open(files, row, entry, len),
                        io => Err(io),
                    };
                }
            }
        }
        sink.raw = Some(OpenOptions::new().create(true).append(true).open(&sink.paths.raw)?);
        Ok(sink)
    }

    /// The record's bytes already received.
    #[must_use]
    pub fn received(&self) -> u64 {
        self.received
    }

    /// The bytes still to come.
    #[must_use]
    pub fn left(&self) -> u64 {
        self.len - self.received
    }

    /// The next bytes of the record, as they arrive. They are on disk when this returns.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<(), RecordError> {
        let take =
            bytes.get(..bytes.len().min(usize::try_from(self.left()).unwrap_or(bytes.len()))).unwrap_or(&[]);
        if let Some(raw) = &mut self.raw {
            raw.write_all(take)?;
        }
        self.process(take)
    }

    fn mismatch(&self, what: &str) -> RecordError {
        RecordError::Mismatch(format!("{} of the archive does not match its manifest: {what}", self.row.path))
    }

    fn process(&mut self, bytes: &[u8]) -> Result<(), RecordError> {
        let mut bytes = bytes;
        self.received += bytes.len() as u64;
        // the local header first: exactly its bytes, then everything else is data
        while self.data_at.is_none() && !bytes.is_empty() {
            let have = self.head.len() as u64;
            let need = match zipdir::local_header_len(&self.head).map_err(|e| self.mismatch(&e))? {
                None => 30 - have,
                Some(n) => n.saturating_sub(have),
            };
            let (a, rest) = bytes.split_at(bytes.len().min(usize::try_from(need).unwrap_or(bytes.len())));
            self.head.extend_from_slice(a);
            bytes = rest;
            if let Some(n) = zipdir::local_header_len(&self.head).map_err(|e| self.mismatch(&e))?
                && self.head.len() as u64 >= n
            {
                self.data_at = Some(n);
            }
        }
        if self.data_at.is_some() && !bytes.is_empty() {
            self.data(bytes)?;
        }
        Ok(())
    }

    /// Bytes past the local header: the packed data, then whatever follows it (a descriptor).
    fn data(&mut self, bytes: &[u8]) -> Result<(), RecordError> {
        let data_at = self.data_at.unwrap_or(0);
        let consumed = self.received - bytes.len() as u64;
        let data_seen = consumed.saturating_sub(data_at);
        let room = self.entry.packed.saturating_sub(data_seen);
        let packed =
            bytes.get(..bytes.len().min(usize::try_from(room).unwrap_or(bytes.len()))).unwrap_or(&[]);
        if self.entry.method == DEFLATED {
            let mut input = packed;
            while !self.ended && (!input.is_empty() || self.pending) {
                let before = self.inflate.total_in();
                let out_before = self.inflate.total_out();
                let status = self
                    .inflate
                    .decompress(input, &mut self.buf, FlushDecompress::None)
                    .map_err(|_| self.mismatch("its data does not inflate"))?;
                let used = usize::try_from(self.inflate.total_in() - before).unwrap_or(input.len());
                let made = usize::try_from(self.inflate.total_out() - out_before).unwrap_or(0);
                input = input.get(used..).unwrap_or(&[]);
                let produced = self.buf.get(..made).unwrap_or(&[]).to_vec();
                self.emit(&produced)?;
                // a full buffer may leave output inside the inflater: drain it before more input
                self.pending = made == self.buf.len();
                if status == Status::StreamEnd {
                    self.ended = true;
                } else if used == 0 && made == 0 && !input.is_empty() {
                    return Err(self.mismatch("its data does not inflate"));
                }
            }
        } else {
            self.emit(packed)?;
        }
        Ok(())
    }

    fn emit(&mut self, bytes: &[u8]) -> Result<(), RecordError> {
        if bytes.is_empty() {
            return Ok(());
        }
        self.out += bytes.len() as u64;
        if self.out > self.row.size {
            return Err(self.mismatch("it is larger than the manifest says"));
        }
        self.crc.update(bytes);
        self.sha.update(bytes);
        self.part.write_all(bytes)?;
        Ok(())
    }

    /// The record is all there: checks it and moves the file into place. A file that does not
    /// match leaves nothing behind.
    pub fn finish(self) -> Result<(), RecordError> {
        let verdict = if self.out != self.row.size || self.out != self.entry.size {
            Err(self.mismatch(&format!("{} bytes where the manifest says {}", self.out, self.row.size)))
        } else if self.crc.clone().finalize() != self.entry.crc {
            Err(self.mismatch("its CRC is wrong"))
        } else if hex::encode(self.sha.clone().finalize()) != self.row.sha256 {
            Err(self.mismatch("its SHA-256 is not the manifest's"))
        } else {
            Ok(())
        };
        let Self { part, raw, paths, .. } = self;
        part.sync_all()?;
        drop(part);
        drop(raw);
        if let Err(e) = verdict {
            let _ = fs::remove_file(&paths.part);
            let _ = fs::remove_file(&paths.raw);
            return Err(e);
        }
        if paths.done.exists() {
            fs::remove_file(&paths.done)?;
        }
        fs::rename(&paths.part, &paths.done)?;
        let _ = fs::remove_file(&paths.raw);
        Ok(())
    }

    /// Drops what this record holds on disk (a stopped download keeps it; this is for a refusal).
    pub fn discard(self) {
        let Self { part, raw, paths, .. } = self;
        drop(part);
        drop(raw);
        let _ = fs::remove_file(&paths.part);
        let _ = fs::remove_file(&paths.raw);
    }
}

/// How many bytes of `row`'s record an earlier attempt kept.
#[must_use]
pub fn kept(files: &Path, rel: &str) -> u64 {
    fs::metadata(Paths::of(files, rel).raw).map_or(0, |m| m.len())
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::zipdir::tests::archive;
    use crate::zipdir::{End, directory, find_end, records};

    fn deflated_record(name: &str, data: &[u8]) -> (Vec<u8>, Entry) {
        use flate2::Compression;
        use flate2::write::DeflateEncoder;
        let mut enc = DeflateEncoder::new(Vec::new(), Compression::new(6));
        enc.write_all(data).unwrap();
        let packed = enc.finish().unwrap();
        let mut rec = Vec::new();
        rec.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        rec.extend_from_slice(&[20, 0, 0, 0, 8, 0, 0, 0, 0, 0]);
        rec.extend_from_slice(&crc32fast::hash(data).to_le_bytes());
        rec.extend_from_slice(&(packed.len() as u32).to_le_bytes());
        rec.extend_from_slice(&(data.len() as u32).to_le_bytes());
        rec.extend_from_slice(&(name.len() as u16).to_le_bytes());
        rec.extend_from_slice(&0u16.to_le_bytes());
        rec.extend_from_slice(name.as_bytes());
        rec.extend_from_slice(&packed);
        let entry = Entry {
            name: name.into(),
            flags: 0,
            method: 8,
            crc: crc32fast::hash(data),
            packed: packed.len() as u64,
            size: data.len() as u64,
            offset: 0,
        };
        (rec, entry)
    }

    fn row(path: &str, data: &[u8]) -> Row {
        Row { sha256: hex::encode(Sha256::digest(data)), size: data.len() as u64, path: path.into() }
    }

    #[test]
    fn a_record_cut_anywhere_resumes_inside_it() {
        let data: Vec<u8> = (0..200_000u32).map(|i| (i % 251) as u8 ^ (i / 997) as u8).collect();
        let (rec, entry) = deflated_record("docs/c.dat", &data);
        let r = row("docs/c.dat", &data);
        for cut in [1usize, 17, 29, 30, 41, rec.len() / 2, rec.len() - 1] {
            let dir = tempfile::tempdir().unwrap();
            let mut a = Sink::open(dir.path(), &r, &entry, rec.len() as u64).unwrap();
            a.feed(&rec[..cut]).unwrap();
            drop(a);
            assert_eq!(kept(dir.path(), "docs/c.dat"), cut as u64);
            let mut b = Sink::open(dir.path(), &r, &entry, rec.len() as u64).unwrap();
            assert_eq!(b.received(), cut as u64);
            b.feed(&rec[cut..]).unwrap();
            assert_eq!(b.left(), 0);
            b.finish().unwrap();
            assert_eq!(fs::read(dir.path().join("docs").join("c.dat")).unwrap(), data);
            assert!(!Paths::of(dir.path(), "docs/c.dat").raw.exists());
        }
    }

    #[test]
    fn a_record_that_is_not_the_manifests_file_is_refused() {
        let zip = archive(&[("a.txt", b"alpha"), ("b.txt", b"beta")], b"", false);
        let len = zip.len() as u64;
        let End::Found(dir) = find_end(&zip, 0, len).unwrap() else { unreachable!() };
        let entries =
            directory(&zip[dir.offset as usize..(dir.offset + dir.size) as usize], dir.count).unwrap();
        let spans = records(&entries, &dir).unwrap();
        let (s, e) = spans[0];
        let files = tempfile::tempdir().unwrap();
        // the manifest wants other bytes of the same size
        let wrong = row("a.txt", b"ALPHA");
        let mut sink = Sink::open(files.path(), &wrong, &entries[0], e - s).unwrap();
        sink.feed(&zip[s as usize..e as usize]).unwrap();
        let err = sink.finish().unwrap_err();
        assert_eq!(
            err.to_string(),
            "a.txt of the archive does not match its manifest: its SHA-256 is not the manifest's"
        );
        assert!(!files.path().join("a.txt").exists());
        assert!(!Paths::of(files.path(), "a.txt").part.exists());
        // a stored record of the right file passes
        let mut sink = Sink::open(files.path(), &row("a.txt", b"alpha"), &entries[0], e - s).unwrap();
        sink.feed(&zip[s as usize..e as usize]).unwrap();
        sink.finish().unwrap();
        assert_eq!(fs::read(files.path().join("a.txt")).unwrap(), b"alpha");
    }
}
