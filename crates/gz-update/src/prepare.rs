//! The check's second half (docs/UPDATES.md, section 3, "Prepare"): the archive's end and
//! directory read by range, then the record of `update-manifest.txt`, unpacked and checked against
//! its CRC. The same reading works on the whole archive once it is on disk.

use std::fs::File;
use std::future::Future;
use std::io::{Read, Seek, SeekFrom};
use std::path::PathBuf;

use gz_editor::Cancel;
use sha2::{Digest, Sha256};

use crate::manifest::{self, MANIFEST, Manifest, ManifestError};
use crate::net::{self, Net, NetError, Ranged};
use crate::zipdir::{self, DEFLATED, Directory, End, Entry};

#[derive(Debug)]
pub enum PrepError {
    /// The server ignores ranges: the archive is read whole.
    NoRanges,
    /// The release's manifest has a schema this editor does not know.
    Schema(String),
    /// The archive cannot be used, as a sentence.
    Bad(String),
    Net(NetError),
}

impl From<NetError> for PrepError {
    fn from(e: NetError) -> Self {
        Self::Net(e)
    }
}

/// Where the archive's bytes come from.
pub trait Source {
    fn read(&self, start: u64, end: u64) -> impl Future<Output = Result<Vec<u8>, PrepError>> + Send;
}

/// The archive on the server, by range.
pub struct Http<'a> {
    pub net: &'a Net,
    pub src: &'a net::Source,
    pub cancel: &'a Cancel,
}

impl Source for Http<'_> {
    async fn read(&self, start: u64, end: u64) -> Result<Vec<u8>, PrepError> {
        match self.net.range(self.src, start, end, self.cancel).await? {
            Ranged::Whole(_) => Err(PrepError::NoRanges),
            Ranged::Partial(mut resp) => {
                let want = usize::try_from(end - start).unwrap_or(0);
                let mut out = Vec::with_capacity(want.min(64 << 20));
                while out.len() < want {
                    match net::chunk(&mut resp, self.cancel).await? {
                        Some(c) => out.extend_from_slice(c.as_ref()),
                        None => break,
                    }
                }
                if out.len() < want {
                    return Err(PrepError::Net(NetError::Network("the connection was lost".to_owned())));
                }
                out.truncate(want);
                Ok(out)
            }
        }
    }
}

/// The archive on disk.
pub struct Local {
    pub path: PathBuf,
}

impl Source for Local {
    async fn read(&self, start: u64, end: u64) -> Result<Vec<u8>, PrepError> {
        let bad =
            |e: std::io::Error| PrepError::Bad(format!("the downloaded archive could not be read: {e}"));
        let mut f = File::open(&self.path).map_err(bad)?;
        f.seek(SeekFrom::Start(start)).map_err(bad)?;
        let mut out = vec![0u8; usize::try_from(end - start).unwrap_or(0)];
        f.read_exact(&mut out).map_err(bad)?;
        Ok(out)
    }
}

/// What the archive says of itself.
#[derive(Clone, Debug)]
pub struct Prepared {
    pub entries: Vec<Entry>,
    /// Each entry's record, in the order of `entries`.
    pub spans: Vec<(u64, u64)>,
    pub manifest: Manifest,
    pub manifest_bytes: Vec<u8>,
    pub manifest_sha: String,
    /// Every manifest row is in the directory at its size, stored or deflated, nothing encrypted:
    /// the records can be read one by one.
    pub ranged_ok: bool,
}

impl Prepared {
    /// The entry of a manifest path and its record.
    #[must_use]
    pub fn record(&self, path: &str) -> Option<(&Entry, (u64, u64))> {
        let i = self.entries.iter().position(|e| e.name == path)?;
        Some((self.entries.get(i)?, *self.spans.get(i)?))
    }
}

/// Unpacks a whole record held in memory and checks it against its directory entry.
pub fn unpack(rec: &[u8], entry: &Entry) -> Result<Vec<u8>, String> {
    let at = zipdir::local_header_len(rec)?.ok_or_else(|| "the record is cut short".to_owned())?;
    let at = usize::try_from(at).unwrap_or(usize::MAX);
    let packed = rec
        .get(at..at.saturating_add(usize::try_from(entry.packed).unwrap_or(usize::MAX)))
        .ok_or_else(|| "the record is cut short".to_owned())?;
    let data = if entry.method == DEFLATED {
        let mut out = Vec::new();
        flate2::read::DeflateDecoder::new(packed)
            .take(entry.size + 1)
            .read_to_end(&mut out)
            .map_err(|_| "its data does not inflate".to_owned())?;
        out
    } else {
        packed.to_vec()
    };
    if data.len() as u64 != entry.size || crc32fast::hash(&data) != entry.crc {
        return Err("its CRC is wrong".to_owned());
    }
    Ok(data)
}

/// Reads the archive's end, directory and manifest.
pub async fn prepare<S: Source>(src: &S, len: u64) -> Result<Prepared, PrepError> {
    let tail_len = len.min(zipdir::TAIL);
    let tail_start = len - tail_len;
    let tail = src.read(tail_start, len).await?;
    let dir: Directory = match zipdir::find_end(&tail, tail_start, len).map_err(PrepError::Bad)? {
        End::Found(d) => d,
        End::Zip64At(at) => {
            zipdir::zip64_end(&src.read(at, at + zipdir::END64_LEN).await?).map_err(PrepError::Bad)?
        }
    };
    if dir.offset.checked_add(dir.size).is_none_or(|e| e > len) {
        return Err(PrepError::Bad("the archive's central directory is damaged".to_owned()));
    }
    let cd = if dir.offset >= tail_start {
        let from = usize::try_from(dir.offset - tail_start).unwrap_or(0);
        tail.get(from..from + usize::try_from(dir.size).unwrap_or(0)).map(<[u8]>::to_vec).unwrap_or_default()
    } else {
        src.read(dir.offset, dir.offset + dir.size).await?
    };
    let entries = zipdir::directory(&cd, dir.count).map_err(PrepError::Bad)?;
    let spans = zipdir::records(&entries, &dir)
        .ok_or_else(|| PrepError::Bad("the archive's records overlap".to_owned()))?;
    let i = entries
        .iter()
        .position(|e| e.name == MANIFEST)
        .ok_or_else(|| PrepError::Bad(format!("the archive holds no {MANIFEST}")))?;
    let (entry, (a, b)) = (&entries[i], spans[i]);
    if !entry.readable() || entry.size > 64 << 20 {
        return Err(PrepError::Bad(format!("the archive's {MANIFEST} cannot be unpacked")));
    }
    let rec = src.read(a, b).await?;
    let bytes = unpack(&rec, entry).map_err(|e| PrepError::Bad(format!("the archive's {MANIFEST}: {e}")))?;
    let manifest = match manifest::parse(&bytes) {
        Ok(m) => m,
        Err(ManifestError::UnknownSchema(s)) => return Err(PrepError::Schema(s)),
        Err(ManifestError::Invalid(s)) => return Err(PrepError::Bad(s)),
    };
    let ranged_ok = manifest
        .rows
        .iter()
        .all(|row| entries.iter().any(|e| e.name == row.path && e.size == row.size && e.readable()));
    let manifest_sha = hex::encode(Sha256::digest(&bytes));
    Ok(Prepared { entries, spans, manifest, manifest_bytes: bytes, manifest_sha, ranged_ok })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::manifest::SCHEMA;
    use crate::zipdir::tests::archive;

    struct Mem(Vec<u8>, std::sync::Mutex<Vec<(u64, u64)>>);

    impl Source for Mem {
        async fn read(&self, start: u64, end: u64) -> Result<Vec<u8>, PrepError> {
            self.1.lock().unwrap().push((start, end));
            Ok(self.0[start as usize..end as usize].to_vec())
        }
    }

    fn manifest_for(files: &[(&str, &[u8])]) -> String {
        let mut t = format!("schema={SCHEMA}\nversion=0.7.1\n");
        for (p, d) in files {
            t.push_str(&format!("{}\t{}\t{p}\n", hex::encode(Sha256::digest(d)), d.len()));
        }
        t
    }

    #[test]
    fn the_manifest_comes_out_of_the_archives_end() {
        let big = vec![3u8; 200_000];
        let files: Vec<(&str, &[u8])> = vec![("a.txt", b"alpha"), ("big.bin", &big)];
        let m = manifest_for(&files);
        let mut all = files.clone();
        all.push((MANIFEST, m.as_bytes()));
        for zip64 in [false, true] {
            let zip = archive(&all, b"", zip64);
            let mem = Mem(zip.clone(), Default::default());
            let p = futures::executor::block_on(prepare(&mem, zip.len() as u64)).unwrap();
            assert!(p.ranged_ok);
            assert_eq!(p.manifest.rows.len(), 2);
            assert_eq!(p.manifest_bytes, m.as_bytes());
            let (e, (s, _)) = p.record("big.bin").unwrap();
            assert_eq!(e.size, 200_000);
            assert!(s > 0);
            // the tail, then the manifest's record: nothing of the files
            let reads = mem.1.lock().unwrap().clone();
            assert_eq!(reads.len(), 2, "{reads:?}");
            assert_eq!(reads[1], p.record(MANIFEST).unwrap().1);
        }
    }

    #[test]
    fn an_archive_that_does_not_describe_itself_is_read_whole_or_refused() {
        // the manifest wants a size the directory does not hold
        let m = manifest_for(&[("a.txt", b"alphabet")]);
        let zip = archive(&[("a.txt", b"alpha"), (MANIFEST, m.as_bytes())], b"", false);
        let p = futures::executor::block_on(prepare(&Mem(zip.clone(), Default::default()), zip.len() as u64))
            .unwrap();
        assert!(!p.ranged_ok);
        // an unknown schema is its own answer
        let zip = archive(&[(MANIFEST, b"schema=grizzly-editor.update/9\nversion=1.0.0\n")], b"", false);
        assert!(matches!(
            futures::executor::block_on(prepare(&Mem(zip.clone(), Default::default()), zip.len() as u64)),
            Err(PrepError::Schema(_))
        ));
        let zip = archive(&[("a.txt", b"alpha")], b"", false);
        assert!(matches!(
            futures::executor::block_on(prepare(&Mem(zip.clone(), Default::default()), zip.len() as u64)),
            Err(PrepError::Bad(_))
        ));
    }
}
