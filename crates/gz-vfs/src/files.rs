//! [`GameFiles`]: the linked game's files as one immutable index.

use std::io;
use std::path::{Path, PathBuf};

use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::archive::{self, ArchEntry};
use crate::cache::{self, ArchiveRec, Records};
use crate::fsgame;
use crate::index::{FileRef, Index, Listed, Source};
use crate::path::{is_tooling_entry, looks_like_game, normalize};
use crate::walk::{self, DirRec};

/// How many archived files one task reads through one open handle.
const READ_BATCH: usize = 64;

/// Why a game cannot be mounted.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum MountError {
    /// The folder is no game: the reason as `LooksLikeGame` words it.
    #[error("{0}")]
    NotAGame(&'static str),
}

/// Where the saved index lives; `None` mounts without one.
#[derive(Clone, Debug, Default)]
pub struct MountOptions {
    pub cache_dir: Option<PathBuf>,
}

/// What a mount cost: the stamps it checked and what it had to read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MountStats {
    /// A saved index was found and used.
    pub from_cache: bool,
    /// Loose folders whose saved record was checked.
    pub dirs_checked: usize,
    /// Loose folders listed.
    pub dirs_read: usize,
    /// Archives whose saved directory was kept.
    pub archives_reused: usize,
    /// Archives whose directory was read.
    pub archives_read: usize,
    /// The saved index was written.
    pub cache_written: bool,
    /// Why the saved index could not be written.
    pub cache_problem: Option<String>,
}

/// Which folder of the game a module was found in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ModuleRoot {
    /// `modules\`, where modules are installed.
    Modules,
    /// `mods\`, the JSGME folder, which still holds what was installed before the split.
    Mods,
}

impl ModuleRoot {
    pub const ALL: [ModuleRoot; 2] = [ModuleRoot::Modules, ModuleRoot::Mods];

    #[must_use]
    pub fn folder(self) -> &'static str {
        match self {
            ModuleRoot::Modules => "modules",
            ModuleRoot::Mods => "mods",
        }
    }
}

/// A module installed in the game: a folder of its own, listed on its own.
#[derive(Debug)]
pub struct Module {
    pub root: ModuleRoot,
    /// The folder's name, as on disk.
    pub id: String,
    pub dir: PathBuf,
    index: Index,
}

impl Module {
    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.len() == 0
    }

    pub fn iter(&self) -> impl Iterator<Item = FileRef<'_>> + '_ {
        self.index.iter()
    }

    #[must_use]
    pub fn get(&self, path: &str) -> Option<FileRef<'_>> {
        self.index.get(path)
    }

    #[must_use]
    pub fn exists(&self, path: &str) -> bool {
        self.index.get(path).is_some()
    }

    #[must_use]
    pub fn list(&self, dir: &str) -> Vec<Listed<'_>> {
        self.index.list(dir)
    }

    #[must_use]
    pub fn find(&self, dir: &str, ext: &str) -> Vec<FileRef<'_>> {
        self.index.find(dir, ext)
    }

    /// The bytes of a file of the module. Blocking.
    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let f = self.index.get(path).ok_or_else(|| not_found(path))?;
        std::fs::read(walk::dir_path(&self.dir, f.path))
    }
}

/// An archive the game mounts.
#[derive(Clone, Debug)]
pub struct Archive {
    pub path: PathBuf,
    pub files: usize,
}

/// The game's files: `path -> where the bytes are`, loose `gamedata` over the archives, a later
/// archive over an earlier one, as the game mounts them; plus the game's modules as roots of
/// their own. Immutable: a change on disk is a new `GameFiles` ([`GameFiles::refresh`]).
#[derive(Debug)]
pub struct GameFiles {
    root: PathBuf,
    root_key: String,
    game_data: PathBuf,
    archives: Vec<Archive>,
    index: Index,
    modules: Vec<Module>,
    skipped: Vec<(PathBuf, String)>,
    records: Records,
    stats: MountStats,
}

fn not_found(path: &str) -> io::Error {
    io::Error::new(io::ErrorKind::NotFound, format!("the game has no file '{path}'"))
}

fn tree_key(root: ModuleRoot, id: &str) -> String {
    format!("{}/{}", root.folder(), id.to_lowercase())
}

/// The module folders of `<game>\<root>`: by name as the folder listing gives it (case
/// ignored), tooling and hidden entries left out.
fn module_dirs(game: &Path, root: ModuleRoot) -> Vec<(String, PathBuf)> {
    let Ok(rd) = std::fs::read_dir(game.join(root.folder())) else { return Vec::new() };
    let mut out: Vec<(String, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            if is_tooling_entry(&name) {
                return None;
            }
            // a module may be a junction: what it points at decides
            std::fs::metadata(e.path()).ok().filter(std::fs::Metadata::is_dir)?;
            Some((name, e.path()))
        })
        .collect();
    out.sort_by_key(|(n, _)| n.to_uppercase());
    out
}

fn loose_index(dirs: &[DirRec]) -> Index {
    Index::build(walk::files_of(dirs).map(|(rel, size)| (normalize(&rel), size, Source::Loose)).collect())
}

impl GameFiles {
    /// Mounts the game at `root`, checking a saved index against the disk and reading only
    /// what changed. Blocking; runs its parallel work on the caller's pool.
    pub fn mount(root: &Path, opts: &MountOptions) -> Result<GameFiles, MountError> {
        let root_text = root.to_string_lossy();
        looks_like_game(&root_text).map_err(MountError::NotAGame)?;
        let root_key = normalize(&root_text);
        let saved = opts.cache_dir.as_deref().and_then(|d| cache::load(d, &root_key));
        Ok(Self::build(root, root_key, saved, opts))
    }

    /// The same game mounted again: what this index read is the saved index, checked against the
    /// disk. Blocking.
    pub fn refresh(&self, opts: &MountOptions) -> Result<GameFiles, MountError> {
        looks_like_game(&self.root.to_string_lossy()).map_err(MountError::NotAGame)?;
        Ok(Self::build(&self.root, self.root_key.clone(), Some(self.records.clone()), opts))
    }

    fn build(root: &Path, root_key: String, saved: Option<Records>, opts: &MountOptions) -> GameFiles {
        let mut stats = MountStats { from_cache: saved.is_some(), ..MountStats::default() };
        let saved = saved.unwrap_or_default();
        let fsgame_text = std::fs::read(root.join("fsgame.ltx"))
            .ok()
            .map(|b| gz_base::text::cp1251_to_utf8(&b).into_owned());
        let layout = fsgame::layout(root, fsgame_text.as_deref());

        // the archives: a saved directory is kept while the archive's size and time are
        let paths = fsgame::archives(&layout.archive_dirs);
        let read: Vec<(PathBuf, Result<ArchiveRec, String>, bool)> = paths
            .into_par_iter()
            .map(|path| {
                let Some((len, mtime)) = walk::stamp(&path) else {
                    return (path, Err("cannot be opened".to_owned()), false);
                };
                if let Some(rec) = saved.archive(&path).filter(|r| r.len == len && r.mtime == mtime) {
                    return (path, Ok(rec.clone()), true);
                }
                let rec = archive::read_directory(&path).map(|entries| ArchiveRec {
                    path: path.clone(),
                    len,
                    mtime,
                    entries: entries.into(),
                });
                (path, rec.map_err(|e| e.to_string()), false)
            })
            .collect();
        let mut records = Records::default();
        let mut skipped = Vec::new();
        for (path, rec, reused) in read {
            match rec {
                Ok(rec) => {
                    if reused {
                        stats.archives_reused += 1;
                    } else {
                        stats.archives_read += 1;
                    }
                    records.archives.push(rec);
                }
                Err(why) => skipped.push((path, why)),
            }
        }

        // the loose tree and the modules, all at once
        let mut trees: Vec<(String, PathBuf)> = vec![("gamedata".to_owned(), layout.game_data.clone())];
        let mut module_list: Vec<(ModuleRoot, String, PathBuf)> = Vec::new();
        for r in ModuleRoot::ALL {
            for (id, dir) in module_dirs(root, r) {
                trees.push((tree_key(r, &id), dir.clone()));
                module_list.push((r, id, dir));
            }
        }
        let walked: Vec<(String, walk::Walked)> = trees
            .into_par_iter()
            .map(|(key, dir)| {
                // the loose game tree keeps every name; a module leaves tooling out
                let w = walk::walk(&dir, saved.tree(&key), key != "gamedata");
                (key, w)
            })
            .collect();
        for (key, w) in walked {
            stats.dirs_checked += w.checked;
            stats.dirs_read += w.read;
            records.trees.push((key, w.dirs.into()));
        }

        let index = Self::merge(&records);
        let modules: Vec<Module> = module_list
            .into_par_iter()
            .map(|(r, id, dir)| {
                let index = loose_index(records.tree(&tree_key(r, &id)));
                Module { root: r, id, dir, index }
            })
            .collect();
        let archives = records
            .archives
            .iter()
            .map(|a| Archive { path: a.path.clone(), files: a.entries.len() })
            .collect();

        let changed = !stats.from_cache
            || stats.dirs_read > 0
            || stats.archives_read > 0
            || saved.trees.len() != records.trees.len()
            || saved.archives.len() != records.archives.len();
        if changed && let Some(dir) = &opts.cache_dir {
            match cache::store(dir, &root_key, &records) {
                Ok(()) => stats.cache_written = true,
                Err(e) => {
                    stats.cache_problem =
                        Some(format!("the game index could not be saved in {}: {e}", dir.display()))
                }
            }
        }
        GameFiles {
            root: root.to_path_buf(),
            root_key,
            game_data: layout.game_data,
            archives,
            index,
            modules,
            skipped,
            records,
            stats,
        }
    }

    /// One table: the archives in order (a later one replaces a path), the loose tree over them.
    fn merge(records: &Records) -> Index {
        let total = records.archives.iter().map(|a| a.entries.len()).sum::<usize>()
            + records.tree("gamedata").iter().map(|d| d.files.len()).sum::<usize>();
        let mut map: FxHashMap<&str, (u64, Source)> = FxHashMap::default();
        map.reserve(total);
        for (i, a) in records.archives.iter().enumerate() {
            let archive = u32::try_from(i).unwrap_or(u32::MAX);
            for ArchEntry { path, size, packed, offset } in a.entries.iter() {
                map.insert(
                    path.as_str(),
                    (u64::from(*size), Source::Archive { archive, offset: *offset, packed: *packed }),
                );
            }
        }
        let loose: Vec<(String, u64)> =
            walk::files_of(records.tree("gamedata")).map(|(rel, size)| (normalize(&rel), size)).collect();
        for (key, size) in &loose {
            map.insert(key.as_str(), (*size, Source::Loose));
        }
        let items: Vec<(String, u64, Source)> =
            map.into_iter().map(|(k, (size, source))| (k.to_owned(), size, source)).collect();
        Index::build(items)
    }

    /// The game folder.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The loose tree (`$game_data$`, `gamedata\`).
    #[must_use]
    pub fn game_data(&self) -> &Path {
        &self.game_data
    }

    /// The archives in mount order.
    #[must_use]
    pub fn archives(&self) -> &[Archive] {
        &self.archives
    }

    /// Archives that could not be read, with why.
    #[must_use]
    pub fn skipped(&self) -> &[(PathBuf, String)] {
        &self.skipped
    }

    #[must_use]
    pub fn stats(&self) -> &MountStats {
        &self.stats
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.index.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.index.len() == 0
    }

    /// Every file, in key order.
    pub fn iter(&self) -> impl Iterator<Item = FileRef<'_>> + '_ {
        self.index.iter()
    }

    /// A file by its path (any case, either separator, relative to `gamedata`).
    #[must_use]
    pub fn get(&self, path: &str) -> Option<FileRef<'_>> {
        self.index.get(path)
    }

    #[must_use]
    pub fn exists(&self, path: &str) -> bool {
        self.index.get(path).is_some()
    }

    /// Whether a folder holds a file at any depth (`""` is the root).
    #[must_use]
    pub fn is_dir(&self, dir: &str) -> bool {
        self.index.is_dir(dir)
    }

    /// What a folder holds: its sub-folders and its files.
    #[must_use]
    pub fn list(&self, dir: &str) -> Vec<Listed<'_>> {
        self.index.list(dir)
    }

    /// Every file below a folder with an extension (`""`: every file).
    #[must_use]
    pub fn find(&self, dir: &str, ext: &str) -> Vec<FileRef<'_>> {
        self.index.find(dir, ext)
    }

    /// The installed modules, `modules\` first, each root by name.
    #[must_use]
    pub fn modules(&self) -> &[Module] {
        &self.modules
    }

    /// A module by its folder name (any case).
    #[must_use]
    pub fn module(&self, root: ModuleRoot, id: &str) -> Option<&Module> {
        self.modules.iter().find(|m| m.root == root && m.id.eq_ignore_ascii_case(id))
    }

    fn archive_path(&self, archive: u32) -> io::Result<&Path> {
        self.archives
            .get(archive as usize)
            .map(|a| a.path.as_path())
            .ok_or_else(|| io::Error::other("the archive is not mounted"))
    }

    /// The bytes of a file. Blocking.
    pub fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        let f = self.index.get(path).ok_or_else(|| not_found(path))?;
        self.read_ref(f)
    }

    /// The bytes of a file of this index, unpacked. Blocking.
    pub fn read_ref(&self, f: FileRef<'_>) -> io::Result<Vec<u8>> {
        match f.source {
            Source::Loose => std::fs::read(walk::dir_path(&self.game_data, f.path)),
            Source::Archive { archive, offset, packed } => {
                let size =
                    u32::try_from(f.size).map_err(|_| io::Error::other("the archived file is too large"))?;
                archive::Open::new(self.archive_path(archive)?)?.read(offset, packed, size)
            }
        }
    }

    /// The bytes of many files, in the order asked, read in parallel on the caller's pool: the
    /// archived ones in runs through one handle per run, in the order they lie in the archive.
    #[must_use]
    pub fn read_many(&self, paths: &[&str]) -> Vec<io::Result<Vec<u8>>> {
        enum Job {
            Loose(usize),
            Run(u32, Vec<(usize, u32, u32, u32)>),
            Missing(usize),
        }
        let mut jobs: Vec<Job> = Vec::new();
        let mut runs: FxHashMap<u32, Vec<(usize, u32, u32, u32)>> = FxHashMap::default();
        for (i, p) in paths.iter().enumerate() {
            match self.index.get(p) {
                None => jobs.push(Job::Missing(i)),
                Some(FileRef { source: Source::Loose, .. }) => jobs.push(Job::Loose(i)),
                Some(FileRef { source: Source::Archive { archive, offset, packed }, size, .. }) => {
                    runs.entry(archive).or_default().push((
                        i,
                        offset,
                        packed,
                        u32::try_from(size).unwrap_or(u32::MAX),
                    ));
                }
            }
        }
        for (archive, mut run) in runs {
            run.sort_unstable_by_key(|r| r.1);
            for part in run.chunks(READ_BATCH) {
                jobs.push(Job::Run(archive, part.to_vec()));
            }
        }
        let done: Vec<Vec<(usize, io::Result<Vec<u8>>)>> = jobs
            .into_par_iter()
            .map(|job| match job {
                Job::Missing(i) => vec![(i, Err(not_found(paths.get(i).copied().unwrap_or(""))))],
                Job::Loose(i) => vec![(i, paths.get(i).map_or_else(|| Err(not_found("")), |p| self.read(p)))],
                Job::Run(archive, run) => match self.archive_path(archive).and_then(archive::Open::new) {
                    Ok(mut open) => run
                        .into_iter()
                        .map(|(i, offset, packed, size)| (i, open.read(offset, packed, size)))
                        .collect(),
                    Err(e) => run
                        .into_iter()
                        .map(|(i, ..)| (i, Err(io::Error::new(e.kind(), e.to_string()))))
                        .collect(),
                },
            })
            .collect();
        let mut out: Vec<Option<io::Result<Vec<u8>>>> = (0..paths.len()).map(|_| None).collect();
        for (i, r) in done.into_iter().flatten() {
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(r);
            }
        }
        out.into_iter().map(|r| r.unwrap_or_else(|| Err(not_found("")))).collect()
    }
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod tests;
