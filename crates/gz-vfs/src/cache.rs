//! The saved index: what a mount read, with the stamps it read it at, so that the next start
//! checks the stamps instead of reading again.
//!
//! `editor appdata\game_index\<hash of the game root>.bin`, little-endian:
//!
//! ```text
//! "GZGI" u32 version  stringz root-key
//! u32 trees    { stringz key  u32 dirs { stringz rel  u64 mtime
//!                                        u32 files { stringz name  u64 size }
//!                                        u32 subdirs { stringz name } } }
//! u32 archives { stringz path  u64 size  u64 mtime
//!                u32 entries { stringz key  u32 size  u32 packed  u32 offset } }
//! ```
//!
//! A file that does not read back - another version, another root, cut short - is no cache.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_chunk::{ChunkError, Reader, Writer};

use crate::archive::ArchEntry;
use crate::walk::DirRec;

const MAGIC: &[u8; 4] = b"GZGI";
// 2: the game's loose tree keeps tooling names
const VERSION: u32 = 2;

/// One archive as it was read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ArchiveRec {
    pub path: PathBuf,
    pub len: u64,
    pub mtime: u64,
    pub entries: Arc<[ArchEntry]>,
}

/// What a mount read: the loose trees by key (`gamedata`, `modules/<id>`, `mods/<id>`) and the
/// archives in mount order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Records {
    pub trees: Vec<(String, Arc<[DirRec]>)>,
    pub archives: Vec<ArchiveRec>,
}

impl Records {
    pub fn tree(&self, key: &str) -> &[DirRec] {
        self.trees.iter().find(|(k, _)| k == key).map_or(&[], |(_, t)| &**t)
    }

    pub fn archive(&self, path: &Path) -> Option<&ArchiveRec> {
        self.archives.iter().find(|a| a.path.as_os_str().eq_ignore_ascii_case(path.as_os_str()))
    }
}

/// FNV-1a over the root's key: the cache file of one game.
#[must_use]
pub(crate) fn file_for(dir: &Path, root_key: &str) -> PathBuf {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in root_key.bytes() {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    dir.join(format!("{h:016x}.bin"))
}

pub(crate) fn encode(root_key: &str, r: &Records) -> Vec<u8> {
    let mut w = Writer::new();
    w.w(MAGIC);
    w.w_u32(VERSION);
    w.w_stringz(root_key);
    w.w_u32(len32(r.trees.len()));
    for (key, dirs) in &r.trees {
        w.w_stringz(key);
        w.w_u32(len32(dirs.len()));
        for d in dirs.iter() {
            w.w_stringz(&d.rel);
            w.w_u64(d.mtime);
            w.w_u32(len32(d.files.len()));
            for (name, size) in &d.files {
                w.w_stringz(name);
                w.w_u64(*size);
            }
            w.w_u32(len32(d.dirs.len()));
            for name in &d.dirs {
                w.w_stringz(name);
            }
        }
    }
    w.w_u32(len32(r.archives.len()));
    for a in &r.archives {
        w.w_stringz(&a.path.to_string_lossy());
        w.w_u64(a.len);
        w.w_u64(a.mtime);
        w.w_u32(len32(a.entries.len()));
        for e in a.entries.iter() {
            w.w_stringz(&e.path);
            w.w_u32(e.size);
            w.w_u32(e.packed);
            w.w_u32(e.offset);
        }
    }
    w.into_bytes()
}

fn len32(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A count, checked against the bytes left: each item needs at least `min` of them.
fn count(r: &mut Reader<'_>, min: usize) -> Result<usize, ChunkError> {
    let n = r.r_u32()? as usize;
    if n.saturating_mul(min) > r.rest().len() {
        return Err(ChunkError::Invalid(format!("the saved game index counts {n} records it cannot hold")));
    }
    Ok(n)
}

fn text(r: &mut Reader<'_>) -> Result<String, ChunkError> {
    let raw = r.r_stringz_raw()?;
    String::from_utf8(raw.to_vec())
        .map_err(|_| ChunkError::Invalid("a name of the saved game index is not UTF-8".into()))
}

/// The records of a saved index of `root_key`.
pub(crate) fn decode(bytes: &[u8], root_key: &str) -> Result<Records, ChunkError> {
    let mut r = Reader::new(bytes);
    if r.take(4)? != MAGIC || r.r_u32()? != VERSION {
        return Err(ChunkError::Invalid("not a saved game index of this version".into()));
    }
    if text(&mut r)? != root_key {
        return Err(ChunkError::Invalid("the saved game index is of another game".into()));
    }
    let mut out = Records::default();
    let trees = count(&mut r, 5)?;
    out.trees.reserve(trees);
    for _ in 0..trees {
        let key = text(&mut r)?;
        let n = count(&mut r, 17)?;
        let mut dirs = Vec::with_capacity(n);
        for _ in 0..n {
            let rel = text(&mut r)?;
            let mtime = r.r_u64()?;
            let nf = count(&mut r, 9)?;
            let mut files = Vec::with_capacity(nf);
            for _ in 0..nf {
                let name = text(&mut r)?;
                files.push((name, r.r_u64()?));
            }
            let nd = count(&mut r, 1)?;
            let mut subs = Vec::with_capacity(nd);
            for _ in 0..nd {
                subs.push(text(&mut r)?);
            }
            dirs.push(DirRec { rel, mtime, files, dirs: subs });
        }
        out.trees.push((key, dirs.into()));
    }
    let archives = count(&mut r, 21)?;
    out.archives.reserve(archives);
    for _ in 0..archives {
        let path = PathBuf::from(text(&mut r)?);
        let len = r.r_u64()?;
        let mtime = r.r_u64()?;
        let n = count(&mut r, 13)?;
        let mut entries = Vec::with_capacity(n);
        for _ in 0..n {
            let key = text(&mut r)?;
            entries.push(ArchEntry { path: key, size: r.r_u32()?, packed: r.r_u32()?, offset: r.r_u32()? });
        }
        out.archives.push(ArchiveRec { path, len, mtime, entries: entries.into() });
    }
    if !r.eof() {
        return Err(ChunkError::Invalid("the saved game index has bytes after its end".into()));
    }
    Ok(out)
}

/// The records saved for `root_key` in `dir`, when there are any that read back.
pub(crate) fn load(dir: &Path, root_key: &str) -> Option<Records> {
    let bytes = std::fs::read(file_for(dir, root_key)).ok()?;
    decode(&bytes, root_key).ok()
}

/// Saves the records: a temporary file renamed over the old one, so a reader never meets half.
pub(crate) fn store(dir: &Path, root_key: &str, r: &Records) -> io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let target = file_for(dir, root_key);
    let tmp = target.with_extension(format!("tmp{}", std::process::id()));
    std::fs::write(&tmp, encode(root_key, r))?;
    std::fs::rename(&tmp, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Records {
        Records {
            trees: vec![(
                "gamedata".into(),
                vec![
                    DirRec {
                        rel: String::new(),
                        mtime: 7,
                        files: vec![("a.ltx".into(), 3)],
                        dirs: vec!["Configs".into()],
                    },
                    DirRec {
                        rel: "Configs".into(),
                        mtime: 9,
                        files: vec![("Тест.xml".into(), 12)],
                        dirs: Vec::new(),
                    },
                ]
                .into(),
            )],
            archives: vec![ArchiveRec {
                path: PathBuf::from("D:\\G\\database\\configs.xdb0"),
                len: 100,
                mtime: 5,
                entries: vec![ArchEntry {
                    path: "configs/system.ltx".into(),
                    size: 10,
                    packed: 6,
                    offset: 8,
                }]
                .into(),
            }],
        }
    }

    #[test]
    fn round_trip() {
        let r = sample();
        let bytes = encode("d:/g", &r);
        assert_eq!(decode(&bytes, "d:/g").unwrap(), r);
        assert!(decode(&bytes, "d:/other").is_err());
        assert!(decode(&bytes[..bytes.len() - 1], "d:/g").is_err());
        let mut longer = bytes.clone();
        longer.push(0);
        assert!(decode(&longer, "d:/g").is_err());
    }

    #[test]
    fn hostile_counts_are_refused() {
        let mut w = Writer::new();
        w.w(MAGIC);
        w.w_u32(VERSION);
        w.w_stringz("r");
        w.w_u32(u32::MAX);
        assert!(decode(&w.into_bytes(), "r").is_err());
    }

    #[test]
    fn one_file_per_game() {
        let a = file_for(Path::new("C:\\x"), "d:/games/a");
        let b = file_for(Path::new("C:\\x"), "d:/games/b");
        assert_ne!(a, b);
        assert_eq!(a, file_for(Path::new("C:\\x"), "d:/games/a"));
    }
}
