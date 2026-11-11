//! The loose trees (`gamedata\`, each module's folder): walked in parallel, one record per
//! folder, and walked again only where a folder's write time moved.
//!
//! A folder's write time moves when an entry is added, removed or renamed in it - which is
//! what changes a listing. A file rewritten in place keeps it; the index reads loose files from
//! the disk anyway, so only that file's listed size can be behind.

use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::UNIX_EPOCH;

use rayon::prelude::*;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::path::is_tooling_entry;

/// How deep a tree is followed: a folder link that points above itself must end somewhere.
const MAX_DEPTH: usize = 48;

/// One folder as it was listed: its write time, its files with their sizes, its folders. Names
/// as on disk; `rel` is the names from the tree's base joined by `/` (`""` for the base).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DirRec {
    pub rel: String,
    pub mtime: u64,
    pub files: Vec<(String, u64)>,
    pub dirs: Vec<String>,
}

/// A tree and what walking it cost.
pub(crate) struct Walked {
    pub dirs: Vec<DirRec>,
    /// Folders whose saved record was checked against the disk.
    pub checked: usize,
    /// Folders listed.
    pub read: usize,
}

/// The write time of a file or folder in 100 ns units since 1970; 0 when unknown.
pub(crate) fn mtime(meta: &Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| u64::try_from(d.as_nanos() / 100).unwrap_or(u64::MAX))
}

/// `(size, write time)` of a path, following links.
pub(crate) fn stamp(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.len(), mtime(&m)))
}

#[cfg(windows)]
fn hidden(meta: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    meta.file_attributes() & 0x2 != 0
}

#[cfg(not(windows))]
fn hidden(_meta: &Metadata) -> bool {
    false
}

pub(crate) fn dir_path(base: &Path, rel: &str) -> PathBuf {
    if rel.is_empty() {
        return base.to_path_buf();
    }
    let mut p = base.to_path_buf();
    for seg in rel.split('/') {
        p.push(seg);
    }
    p
}

fn child(rel: &str, name: &str) -> String {
    if rel.is_empty() { name.to_owned() } else { format!("{rel}/{name}") }
}

/// Lists one folder. `None` when it cannot be listed (gone, no access). Hidden entries are
/// skipped as the engine skips them (`_A_HIDDEN`), and tooling with `skip_tooling` (a module, as
/// XMS skips it; the game's own `gamedata` lists it, as the C++ editor's DARF content did).
pub(crate) fn scan(base: &Path, rel: &str, skip_tooling: bool) -> Option<DirRec> {
    let dir = dir_path(base, rel);
    // the stamp is taken BEFORE the listing: a change while listing makes the record stale
    let mtime = mtime(&std::fs::metadata(&dir).ok()?);
    let rd = std::fs::read_dir(&dir).ok()?;
    let mut files = Vec::new();
    let mut dirs = Vec::new();
    for e in rd.flatten() {
        let Ok(name) = e.file_name().into_string() else { continue };
        if skip_tooling && is_tooling_entry(&name) {
            continue;
        }
        let Ok(own) = e.metadata() else { continue };
        if hidden(&own) {
            continue;
        }
        // a link is what it points at (a module may be a junction into a working copy)
        let meta = if own.file_type().is_symlink() {
            match std::fs::metadata(e.path()) {
                Ok(m) => m,
                Err(_) => continue,
            }
        } else {
            own
        };
        if meta.is_dir() {
            dirs.push(name);
        } else if meta.is_file() {
            files.push((name, meta.len()));
        }
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    dirs.sort();
    Some(DirRec { rel: rel.to_owned(), mtime, files, dirs })
}

/// Walks the tree at `base`. A folder of `saved` whose write time is unchanged keeps its record
/// (one `metadata` call, all of them in parallel); any other folder is listed again. Runs on
/// the caller's pool.
pub(crate) fn walk(base: &Path, saved: &[DirRec], skip_tooling: bool) -> Walked {
    let by_rel: FxHashMap<&str, &DirRec> = saved.iter().map(|d| (d.rel.as_str(), d)).collect();
    let valid: FxHashSet<&str> = saved
        .par_iter()
        .filter(|d| {
            std::fs::metadata(dir_path(base, &d.rel)).is_ok_and(|m| m.is_dir() && mtime(&m) == d.mtime)
        })
        .map(|d| d.rel.as_str())
        .collect();
    let read = AtomicUsize::new(0);
    let cx = Cx { base, by_rel: &by_rel, valid: &valid, read: &read, skip_tooling };
    let dirs = cx.visit(String::new(), 0);
    Walked { dirs, checked: saved.len(), read: read.into_inner() }
}

struct Cx<'a> {
    base: &'a Path,
    by_rel: &'a FxHashMap<&'a str, &'a DirRec>,
    valid: &'a FxHashSet<&'a str>,
    read: &'a AtomicUsize,
    skip_tooling: bool,
}

impl Cx<'_> {
    fn visit(&self, rel: String, depth: usize) -> Vec<DirRec> {
        let rec = match self.by_rel.get(rel.as_str()) {
            Some(&saved) if self.valid.contains(rel.as_str()) => saved.clone(),
            _ => {
                self.read.fetch_add(1, Ordering::Relaxed);
                match scan(self.base, &rel, self.skip_tooling) {
                    Some(r) => r,
                    None => return Vec::new(),
                }
            }
        };
        if depth >= MAX_DEPTH || rec.dirs.is_empty() {
            return vec![rec];
        }
        let subs: Vec<String> = rec.dirs.iter().map(|d| child(&rel, d)).collect();
        let nested: Vec<Vec<DirRec>> = subs.into_par_iter().map(|s| self.visit(s, depth + 1)).collect();
        let mut out = Vec::with_capacity(1 + nested.iter().map(Vec::len).sum::<usize>());
        out.push(rec);
        for n in nested {
            out.extend(n);
        }
        out
    }
}

/// Every file of a tree: `(rel path, size)` with the names as on disk.
pub(crate) fn files_of(dirs: &[DirRec]) -> impl Iterator<Item = (String, u64)> + '_ {
    dirs.iter().flat_map(|d| d.files.iter().map(move |(name, size)| (child(&d.rel, name), *size)))
}
