//! The linked game's files as one gamedata-relative listing: everything inside
//! `database\*.db*` / `*.xdb*` plus the loose `gamedata\` tree, loose beating packed on a
//! duplicate path the way the game resolves it. Read-only: nothing here opens a write handle on
//! the game folder; the one write path is [`DarfIndex::copy_to_project`], into the project.
//!
//! oracle: this is `EditorGameContent`'s private mount. When the game-link area (`gz-vfs`)
//! lands, its archive reader replaces the one here.

use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use gz_chunk::{COMPRESSED, lzhuf, lzo};

use crate::fsys;

/// `Ok` when `game_root` looks like an X-Ray game: the folder, `fsgame.ltx`, and `database\` or
/// `gamedata\`. The reason otherwise, as the C++ worded it.
pub fn looks_like_game(game_root: &str) -> Result<(), &'static str> {
    if game_root.is_empty() {
        return Err("no folder selected");
    }
    let root = Path::new(game_root);
    if !fsys::dir_exists(root) {
        return Err("folder does not exist");
    }
    if !fsys::file_exists(&root.join("fsgame.ltx")) {
        return Err("no fsgame.ltx - not an X-Ray game folder");
    }
    if !fsys::dir_exists(&root.join("database")) && !fsys::dir_exists(&root.join("gamedata")) {
        return Err("neither database\\ nor gamedata\\ is present");
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DarfSource {
    /// Inside the archive with this index in [`DarfIndex::archives`].
    Archive { archive: usize, offset: u32, size_compressed: u32 },
    Loose,
}

/// One file of the game.
#[derive(Clone, Debug)]
pub struct DarfItem {
    /// gamedata-relative, lower case, backslashes.
    pub path: String,
    pub size: u64,
    pub source: DarfSource,
}

impl DarfItem {
    #[must_use]
    pub fn source_name(&self) -> &'static str {
        match self.source {
            DarfSource::Archive { .. } => "archive",
            DarfSource::Loose => "loose",
        }
    }
}

/// The listing of one game folder.
#[derive(Debug)]
pub struct DarfIndex {
    pub root: String,
    pub archives: Vec<PathBuf>,
    /// Sorted by `path`.
    pub items: Vec<DarfItem>,
    /// Archives that could not be read, for the log.
    pub skipped: Vec<String>,
}

/// The body of chunk `id` of a chunk file, read by seeking past the others (an archive is
/// gigabytes; only its directory is wanted). `Ok(None)` when it has no such chunk.
fn read_chunk(file: &mut File, id: u32) -> io::Result<Option<Vec<u8>>> {
    let len = file.metadata()?.len();
    let mut pos = 0u64;
    file.seek(SeekFrom::Start(0))?;
    loop {
        let mut head = [0u8; 8];
        if pos + 8 > len {
            return Ok(None);
        }
        file.read_exact(&mut head)?;
        pos += 8;
        let kind = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
        let size = u64::from(u32::from_le_bytes([head[4], head[5], head[6], head[7]]));
        if size > len - pos {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "a chunk runs past the end of the archive"));
        }
        if kind & !COMPRESSED == id {
            let mut body = vec![0u8; usize::try_from(size).map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "a chunk is too large"))?];
            file.read_exact(&mut body)?;
            if kind & COMPRESSED != 0 {
                body = lzhuf::decompress(&body).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
            }
            return Ok(Some(body));
        }
        pos += size;
        file.seek(SeekFrom::Start(pos))?;
    }
}

fn archive_entries(path: &Path) -> io::Result<Vec<lzo::ArchiveEntry>> {
    let mut file = File::open(path)?;
    let Some(dir) = read_chunk(&mut file, 1)? else {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "no file table"));
    };
    lzo::read_archive_index(&dir).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))
}

/// `<root>\database\*.db*` / `*.xdb*`, not empty, sorted the way the game loads them.
fn collect_archives(root: &Path) -> Vec<PathBuf> {
    let Ok(rd) = std::fs::read_dir(root.join("database")) else { return Vec::new() };
    let mut out: Vec<PathBuf> = rd
        .flatten()
        .filter(|e| e.metadata().is_ok_and(|m| m.is_file() && m.len() > 0))
        .filter(|e| {
            let name = e.file_name().to_string_lossy().to_ascii_lowercase();
            name.rfind('.').and_then(|d| name.get(d..)).is_some_and(|ext| ext.starts_with(".db") || ext.starts_with(".xdb"))
        })
        .map(|e| e.path())
        .collect();
    out.sort();
    out
}

fn scan_loose(base: &Path, rel: &str, out: &mut Vec<DarfItem>) {
    let dir = if rel.is_empty() { base.to_path_buf() } else { base.join(rel) };
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let Ok(name) = e.file_name().into_string() else { continue };
        let sub = if rel.is_empty() { name } else { format!("{rel}\\{name}") };
        let Ok(meta) = e.metadata() else { continue };
        if meta.is_dir() {
            scan_loose(base, &sub, out);
        } else {
            out.push(DarfItem { path: sub.to_lowercase(), size: meta.len(), source: DarfSource::Loose });
        }
    }
}

impl DarfIndex {
    /// Lists the game at `game_root`: its archives' directories and the loose tree. Blocking.
    pub fn mount(game_root: &str) -> Result<DarfIndex, String> {
        looks_like_game(game_root).map_err(str::to_owned)?;
        let root = Path::new(game_root);
        let archives = collect_archives(root);
        let mut items: Vec<DarfItem> = Vec::new();
        let mut skipped = Vec::new();
        let mut packed: std::collections::BTreeMap<String, DarfItem> = std::collections::BTreeMap::new();
        for (i, archive) in archives.iter().enumerate() {
            match archive_entries(archive) {
                Ok(entries) => {
                    for e in entries {
                        // a name ending in a separator is a folder marker
                        if e.name.ends_with('\\') || e.name.ends_with('/') || e.name.is_empty() {
                            continue;
                        }
                        let path = e.name.replace('/', "\\").to_lowercase();
                        // a later archive replaces the entry, as the engine's file table does
                        packed.insert(
                            path.clone(),
                            DarfItem { path, size: u64::from(e.size_real), source: DarfSource::Archive { archive: i, offset: e.offset, size_compressed: e.size_compressed } },
                        );
                    }
                }
                Err(_) => skipped.push(archive.display().to_string()),
            }
        }
        items.extend(packed.into_values());
        let gamedata = root.join("gamedata");
        if fsys::dir_exists(&gamedata) {
            scan_loose(&gamedata, "", &mut items);
        }
        // loose beats packed on a duplicate path
        items.sort_by(|a, b| a.path.cmp(&b.path).then_with(|| (a.source == DarfSource::Loose).cmp(&(b.source == DarfSource::Loose))));
        let mut merged: Vec<DarfItem> = Vec::with_capacity(items.len());
        for it in items {
            match merged.last_mut() {
                Some(last) if last.path == it.path => {
                    if it.source == DarfSource::Loose {
                        *last = it;
                    }
                }
                _ => merged.push(it),
            }
        }
        Ok(DarfIndex { root: game_root.to_owned(), archives, items: merged, skipped })
    }

    /// The item at a gamedata-relative path (any case, either separator).
    #[must_use]
    pub fn find(&self, rel: &str) -> Option<&DarfItem> {
        let key = rel.replace('/', "\\").to_lowercase();
        self.items.binary_search_by(|it| it.path.as_str().cmp(&key)).ok().and_then(|i| self.items.get(i))
    }

    /// The bytes of an item. Blocking.
    pub fn read(&self, item: &DarfItem) -> io::Result<Vec<u8>> {
        match item.source {
            DarfSource::Loose => std::fs::read(Path::new(&self.root).join("gamedata").join(&item.path)),
            DarfSource::Archive { archive, offset, size_compressed } => {
                let path = self.archives.get(archive).ok_or_else(|| io::Error::other("the archive is not mounted"))?;
                let mut file = File::open(path)?;
                let len = file.metadata()?.len();
                if u64::from(offset) + u64::from(size_compressed) > len {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, "the archived file runs past the end of its archive"));
                }
                let size_real = usize::try_from(item.size).map_err(|_| io::Error::other("the archived file is too large"))?;
                file.seek(SeekFrom::Start(u64::from(offset)))?;
                let mut stored = vec![0u8; usize::try_from(size_compressed).map_err(|_| io::Error::other("the archived file is too large"))?];
                file.read_exact(&mut stored)?;
                let bytes = lzo::unpack_entry(&stored, size_real).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?;
                Ok(bytes.into_owned())
            }
        }
    }

    /// Copies an item to `<project>\gamedata\<its path>`. `Ok(target)`, or `Err((target,
    /// existed, reason))` with the C++ reasons.
    pub fn copy_to_project(&self, project_root: &str, rel: &str, overwrite: bool) -> Result<(String, bool), (String, bool, &'static str)> {
        let Some(item) = self.find(rel) else { return Err((String::new(), false, "unknown file")) };
        let target = format!("{project_root}\\gamedata\\{}", item.path);
        let existed = fsys::file_exists(Path::new(&target));
        if existed && !overwrite {
            return Err((target, true, "already exists"));
        }
        let Ok(data) = self.read(item) else { return Err((target, existed, "can't read the source file")) };
        if let Some(parent) = Path::new(&target).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        fsys::clear_readonly(Path::new(&target));
        let Ok(mut file) = File::create(&target) else { return Err((target, existed, "can't create the target file")) };
        if io::Write::write_all(&mut file, &data).is_err() {
            return Err((target, existed, "write failed"));
        }
        Ok((target, existed))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn archive_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        // body first (offsets count from the file start: 8 bytes of the chunk header before it)
        let mut body = Vec::new();
        let mut table = Vec::new();
        let mut offset = 8u32;
        for (name, bytes) in entries {
            body.extend_from_slice(bytes);
            let mut rec = Vec::new();
            rec.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            rec.extend_from_slice(&(bytes.len() as u32).to_le_bytes());
            rec.extend_from_slice(&0u32.to_le_bytes());
            rec.extend_from_slice(name.as_bytes());
            rec.extend_from_slice(&offset.to_le_bytes());
            table.extend_from_slice(&(rec.len() as u16).to_le_bytes());
            table.extend_from_slice(&rec);
            offset += bytes.len() as u32;
        }
        let mut out = Vec::new();
        out.extend_from_slice(&0u32.to_le_bytes());
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(&1u32.to_le_bytes());
        out.extend_from_slice(&(table.len() as u32).to_le_bytes());
        out.extend_from_slice(&table);
        out
    }

    #[test]
    fn lists_archives_and_loose_files() {
        let dir = std::env::temp_dir().join(format!("gz-project-darf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("database")).unwrap();
        std::fs::create_dir_all(dir.join("gamedata\\configs")).unwrap();
        std::fs::write(dir.join("fsgame.ltx"), "").unwrap();
        std::fs::write(dir.join("database\\a.db0"), archive_with(&[("configs\\", b""), ("configs\\System.ltx", b"packed"), ("meshes\\a.ogf", b"ogf")])).unwrap();
        std::fs::write(dir.join("gamedata\\configs\\system.ltx"), "loose").unwrap();

        let root = dir.to_str().unwrap();
        let idx = DarfIndex::mount(root).unwrap();
        let paths: Vec<&str> = idx.items.iter().map(|i| i.path.as_str()).collect();
        assert_eq!(paths, ["configs\\system.ltx", "meshes\\a.ogf"]);
        let sys = idx.find("CONFIGS/system.ltx").unwrap();
        assert_eq!(sys.source, DarfSource::Loose);
        assert_eq!(idx.read(sys).unwrap(), b"loose");
        let ogf = idx.find("meshes\\a.ogf").unwrap();
        assert_eq!(idx.read(ogf).unwrap(), b"ogf");
        assert!(idx.find("nothing").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn game_folders() {
        assert_eq!(looks_like_game(""), Err("no folder selected"));
        assert_eq!(looks_like_game("Z:\\surely\\not\\here"), Err("folder does not exist"));
    }
}
