//! Real data for the corpus tests of the format crates: the files of a game install and of
//! projects, named by the environment, read only.
//!
//! - `GRIZZLY_TEST_GAME`: a game root (its `gamedata` and the archives of its `database`);
//! - `GRIZZLY_TEST_PROJECTS`: project folders, separated by `;`;
//! - the repository itself ([`repo`]).
//!
//! Without a variable a corpus test prints `skipped: <variable> is not set` and passes.
//! `tools/rust/test.sh` sets both. The game is read here independently of `gz-vfs`, so the
//! corpus test of `gz-vfs` checks it against a second reader.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::Instant;

use gz_chunk::{COMPRESSED, lzhuf, lzo};
use rayon::prelude::*;

pub const GAME_VAR: &str = "GRIZZLY_TEST_GAME";
pub const PROJECTS_VAR: &str = "GRIZZLY_TEST_PROJECTS";

fn var(name: &str) -> Option<String> {
    match std::env::var(name) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => {
            println!("skipped: {name} is not set");
            None
        }
    }
}

/// The game root, or `None` (said) when the variable is not set.
#[must_use]
pub fn game_root() -> Option<PathBuf> {
    var(GAME_VAR).map(PathBuf::from)
}

/// The game, mounted, or `None` (said) when the variable is not set. A game that does not
/// mount is an error of the test.
pub fn game() -> Option<io::Result<Game>> {
    game_root().map(|root| Game::open(&root))
}

/// The project folders that exist, or `None` (said) when the variable is not set.
#[must_use]
pub fn projects() -> Option<Vec<PathBuf>> {
    let list = var(PROJECTS_VAR)?;
    Some(
        list.split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.is_dir())
            .collect(),
    )
}

/// The repository's root (this crate lives in `crates/gz-corpus`).
#[must_use]
pub fn repo() -> PathBuf {
    let here = Path::new(env!("CARGO_MANIFEST_DIR"));
    here.parent().and_then(Path::parent).map_or_else(|| here.to_path_buf(), Path::to_path_buf)
}

/// A path made a key: lower case, `/`, no `.` or `..`.
#[must_use]
pub fn key(path: &str) -> String {
    let mut segs: Vec<String> = Vec::new();
    for seg in path.split(['\\', '/']) {
        match seg {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            s => segs.push(s.to_lowercase()),
        }
    }
    segs.join("/")
}

/// Whether a key has one of the extensions (without the dot, lower case).
#[must_use]
pub fn has_ext(key: &str, exts: &[&str]) -> bool {
    key.rsplit_once('.').is_some_and(|(_, e)| exts.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

#[derive(Clone, Debug)]
enum Src {
    Loose(PathBuf),
    Archive { archive: usize, offset: u32, packed: u32 },
}

/// One file of the game.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The path below `gamedata`, as a [`key`].
    pub key: String,
    pub size: u32,
    src: Src,
}

impl Entry {
    #[must_use]
    pub fn in_archive(&self) -> bool {
        matches!(self.src, Src::Archive { .. })
    }
}

/// A game's files: the archives of `database\` in name order, a later one over an earlier, and
/// the loose `gamedata` over them all. Archived copies that a later source hides stay listed in
/// [`Game::archived`], so every file an archive lists can be read.
pub struct Game {
    pub root: PathBuf,
    archives: Vec<PathBuf>,
    /// The visible files, sorted by key.
    files: Vec<Entry>,
    /// Every archived file, hidden or not.
    archived: Vec<Entry>,
}

fn invalid(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}

/// Chunk 1 of an archive: its directory.
fn directory(path: &Path) -> io::Result<Vec<lzo::ArchiveEntry>> {
    let mut f = File::open(path)?;
    let len = f.metadata()?.len();
    let mut pos = 0u64;
    loop {
        if pos + 8 > len {
            return Err(invalid(format!("{} has no file table", path.display())));
        }
        let mut head = [0u8; 8];
        f.read_exact(&mut head)?;
        pos += 8;
        let id = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        if size > len - pos {
            return Err(invalid(format!("a chunk of {} runs past its end", path.display())));
        }
        if id & !COMPRESSED == 1 {
            let mut body = vec![0u8; usize::try_from(size).map_err(|_| invalid("a huge chunk"))?];
            f.read_exact(&mut body)?;
            if id & COMPRESSED != 0 {
                body = lzhuf::decompress(&body).map_err(|e| invalid(e.to_string()))?;
            }
            return lzo::read_archive_index(&body).map_err(|e| invalid(e.to_string()));
        }
        pos += size;
        f.seek(SeekFrom::Start(pos))?;
    }
}

/// The files below `dir`, recursively, sorted; dot-names and links are left out (the editor's
/// tooling rule).
#[must_use]
pub fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with('.') {
                continue;
            }
            let Ok(t) = e.file_type() else { continue };
            if t.is_symlink() {
                continue;
            }
            if t.is_dir() {
                stack.push(e.path());
            } else if t.is_file() {
                out.push(e.path());
            }
        }
    }
    out.sort();
    out
}

/// The files below `dir` with one of the extensions.
#[must_use]
pub fn walk_ext(dir: &Path, exts: &[&str]) -> Vec<PathBuf> {
    walk(dir).into_iter().filter(|p| has_ext(&p.to_string_lossy(), exts)).collect()
}

impl Game {
    /// Mounts a game: every archive directory read in parallel, the loose tree walked.
    pub fn open(root: &Path) -> io::Result<Self> {
        let mut archives: Vec<PathBuf> = std::fs::read_dir(root.join("database"))?
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.is_file()
                    && p.extension().is_some_and(|e| {
                        let e = e.to_string_lossy().to_ascii_lowercase();
                        e.starts_with("db") || e.starts_with("xdb")
                    })
            })
            .collect();
        archives.sort();
        let dirs: Vec<io::Result<Vec<lzo::ArchiveEntry>>> =
            archives.par_iter().map(|a| directory(a)).collect();
        let mut archived = Vec::new();
        for (i, d) in dirs.into_iter().enumerate() {
            for r in d? {
                if r.name.is_empty() || r.name.ends_with(['\\', '/']) {
                    continue;
                }
                let key = key(&r.name);
                if !key.is_empty() {
                    let src = Src::Archive { archive: i, offset: r.offset, packed: r.size_compressed };
                    archived.push(Entry { key, size: r.size_real, src });
                }
            }
        }
        let mut by_key: std::collections::BTreeMap<String, Entry> = std::collections::BTreeMap::new();
        for e in &archived {
            by_key.insert(e.key.clone(), e.clone());
        }
        let data = root.join("gamedata");
        for p in walk(&data) {
            let Ok(rel) = p.strip_prefix(&data) else { continue };
            let k = key(&rel.to_string_lossy());
            let size = std::fs::metadata(&p).map_or(0, |m| u32::try_from(m.len()).unwrap_or(u32::MAX));
            by_key.insert(k.clone(), Entry { key: k, size, src: Src::Loose(p) });
        }
        Ok(Self { root: root.to_path_buf(), archives, files: by_key.into_values().collect(), archived })
    }

    /// The files the game sees, sorted by key.
    #[must_use]
    pub fn files(&self) -> &[Entry] {
        &self.files
    }

    /// Every file an archive lists, hidden by a later source or not.
    #[must_use]
    pub fn archived(&self) -> &[Entry] {
        &self.archived
    }

    /// The visible files with one of the extensions, sorted by key.
    #[must_use]
    pub fn with_ext(&self, exts: &[&str]) -> Vec<&Entry> {
        self.files.iter().filter(|e| has_ext(&e.key, exts)).collect()
    }

    #[must_use]
    pub fn get(&self, path: &str) -> Option<&Entry> {
        let k = key(path);
        self.files.binary_search_by(|e| e.key.as_str().cmp(&k)).ok().and_then(|i| self.files.get(i))
    }

    /// The bytes of a file.
    pub fn read(&self, e: &Entry) -> io::Result<Vec<u8>> {
        match &e.src {
            Src::Loose(p) => std::fs::read(p),
            Src::Archive { archive, offset, packed } => {
                let path = self.archives.get(*archive).ok_or_else(|| invalid("no such archive"))?;
                let mut f = File::open(path)?;
                f.seek(SeekFrom::Start(u64::from(*offset)))?;
                let mut stored = vec![0u8; *packed as usize];
                f.read_exact(&mut stored)?;
                if *packed == e.size {
                    return Ok(stored);
                }
                lzo::unpack_entry(&stored, e.size as usize)
                    .map(std::borrow::Cow::into_owned)
                    .map_err(|err| invalid(format!("{}: {err}", e.key)))
            }
        }
    }

    /// The first `n` bytes of a file (all of it when shorter). A stored archive entry is read
    /// only that far; a packed one is unpacked whole.
    pub fn read_head(&self, e: &Entry, n: usize) -> io::Result<Vec<u8>> {
        match &e.src {
            Src::Archive { archive, offset, packed } if *packed == e.size => {
                let path = self.archives.get(*archive).ok_or_else(|| invalid("no such archive"))?;
                let mut f = File::open(path)?;
                f.seek(SeekFrom::Start(u64::from(*offset)))?;
                let mut head = vec![0u8; n.min(*packed as usize)];
                f.read_exact(&mut head)?;
                Ok(head)
            }
            Src::Loose(p) => {
                let mut f = File::open(p)?;
                let mut head = Vec::with_capacity(n);
                f.by_ref().take(n as u64).read_to_end(&mut head)?;
                Ok(head)
            }
            Src::Archive { .. } => {
                let mut all = self.read(e)?;
                all.truncate(n);
                Ok(all)
            }
        }
    }

    /// The bytes of a visible file by its path.
    pub fn read_path(&self, path: &str) -> io::Result<Vec<u8>> {
        let e = self.get(path).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, path.to_owned()))?;
        self.read(e)
    }
}

/// Every `n`th item, by the order given (sort first: the choice is then deterministic).
#[must_use]
pub fn stride<T>(items: Vec<T>, n: usize) -> Vec<T> {
    let n = n.max(1);
    items.into_iter().enumerate().filter(|(i, _)| i % n == 0).map(|(_, t)| t).collect()
}

/// Runs `check` over the items in parallel and collects the failures as `name: why`.
pub fn check_all<T: Sync>(
    items: &[T],
    name: impl Fn(&T) -> String + Sync,
    check: impl Fn(&T) -> Result<(), String> + Sync,
) -> Vec<String> {
    let mut failed: Vec<String> =
        items.par_iter().filter_map(|t| check(t).err().map(|why| format!("{}: {why}", name(t)))).collect();
    failed.sort();
    failed
}

/// Ends a corpus run: prints what was checked in how long, and fails with up to 20 failures.
pub fn finish(what: &str, checked: usize, started: Instant, failed: &[String]) {
    println!(
        "{what}: {checked} checked in {:.2} s, {} failed",
        started.elapsed().as_secs_f64(),
        failed.len()
    );
    let shown: Vec<&str> = failed.iter().take(20).map(String::as_str).collect();
    assert!(failed.is_empty(), "{what}: {} of {checked} failed:\n{}", failed.len(), shown.join("\n"));
}
