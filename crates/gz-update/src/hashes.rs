//! The SHA-256 of installed files, cached by path, size and write time in the editor's home
//! (`update\hashes.ltx`): a check compares every managed file, and hashing the programs again on
//! every start would cost seconds for nothing.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};

/// A file's hash, as it was read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Known {
    size: u64,
    mtime: u128,
    sha: [u8; 32],
}

pub struct HashCache {
    path: PathBuf,
    map: Mutex<HashMap<String, Known>>,
    dirty: Mutex<bool>,
}

/// The SHA-256 of a whole file, in lower-case hex.
pub fn sha256_file(path: &Path) -> io::Result<(u64, String)> {
    let mut f = File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut size = 0u64;
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(buf.get(..n).unwrap_or(&[]));
        size += n as u64;
    }
    Ok((size, hex::encode(h.finalize())))
}

fn stamp(path: &Path) -> io::Result<(u64, u128)> {
    let meta = std::fs::metadata(path)?;
    let mtime = meta.modified()?.duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    Ok((meta.len(), mtime))
}

impl HashCache {
    /// The cache kept at `path`; a file that cannot be read is an empty cache.
    #[must_use]
    pub fn load(path: PathBuf) -> Self {
        let mut map = HashMap::new();
        if let Ok(text) = std::fs::read_to_string(&path) {
            for line in text.lines() {
                let Some(rest) = line.strip_prefix("file = ") else { continue };
                let mut parts = rest.splitn(4, '|');
                let (Some(size), Some(mtime), Some(sha), Some(rel)) =
                    (parts.next(), parts.next(), parts.next(), parts.next())
                else {
                    continue;
                };
                let mut bytes = [0u8; 32];
                if let (Ok(size), Ok(mtime), Ok(())) =
                    (size.parse(), mtime.parse(), hex::decode_to_slice(sha, &mut bytes))
                {
                    map.insert(rel.to_owned(), Known { size, mtime, sha: bytes });
                }
            }
        }
        Self { path, map: Mutex::new(map), dirty: Mutex::new(false) }
    }

    /// The size and SHA-256 of `root/rel`, or `None` when there is no such file. A file whose size
    /// and write time are what the cache saw is not read again.
    pub fn of(&self, root: &Path, rel: &str) -> io::Result<Option<(u64, String)>> {
        let path = root.join(rel);
        let (size, mtime) = match stamp(&path) {
            Ok(s) => s,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(e) => return Err(e),
        };
        let key = format!("{}|{}", root.display(), rel).to_lowercase();
        if let Some(k) = self.map.lock().get(&key).copied()
            && k.size == size
            && k.mtime == mtime
        {
            return Ok(Some((size, hex::encode(k.sha))));
        }
        let (read, sha) = sha256_file(&path)?;
        // a file that changed while it was read is not cached
        if read == size && stamp(&path).is_ok_and(|s| s == (size, mtime)) {
            let mut bytes = [0u8; 32];
            if hex::decode_to_slice(&sha, &mut bytes).is_ok() {
                self.map.lock().insert(key, Known { size, mtime, sha: bytes });
                *self.dirty.lock() = true;
            }
        }
        Ok(Some((read, sha)))
    }

    /// Writes the cache when it learned something.
    pub fn save(&self) {
        if !std::mem::take(&mut *self.dirty.lock()) {
            return;
        }
        let mut text = String::from(
            "; the SHA-256 of installed files by size and write time: a cache, safe to delete\n",
        );
        let map = self.map.lock();
        let mut keys: Vec<&String> = map.keys().collect();
        keys.sort();
        for k in keys {
            if let Some(v) = map.get(k) {
                text.push_str(&format!("file = {}|{}|{}|{}\n", v.size, v.mtime, hex::encode(v.sha), k));
            }
        }
        drop(map);
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.path.with_extension("tmp");
        if std::fs::write(&tmp, text).is_ok() && std::fs::rename(&tmp, &self.path).is_err() {
            let _ = std::fs::remove_file(&tmp);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn a_file_is_read_again_only_when_it_changed() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), b"one").unwrap();
        let cache_file = dir.path().join("home").join("hashes.ltx");
        let cache = HashCache::load(cache_file.clone());
        let (size, sha) = cache.of(dir.path(), "a.txt").unwrap().unwrap();
        assert_eq!(size, 3);
        assert_eq!(sha, hex::encode(Sha256::digest(b"one")));
        assert_eq!(cache.of(dir.path(), "missing.txt").unwrap(), None);
        cache.save();
        let again = HashCache::load(cache_file);
        assert_eq!(again.map.lock().len(), 1);
        assert_eq!(again.of(dir.path(), "a.txt").unwrap().unwrap().1, sha);
        std::fs::write(dir.path().join("a.txt"), b"two!").unwrap();
        assert_eq!(again.of(dir.path(), "a.txt").unwrap().unwrap().1, hex::encode(Sha256::digest(b"two!")));
    }
}
