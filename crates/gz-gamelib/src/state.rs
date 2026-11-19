//! What the library was made from: for every file of the game a description was made of, the
//! stamp it had then and the row that was made, so a sync finds what changed without opening a
//! file and writes a catalog again without reading the game.

use std::collections::BTreeMap;
use std::path::Path;

use gz_vfs::{FileRef, GameFiles, Source};
use rustc_hash::FxHashMap;

/// The first line of `state.txt`; another one is a state this editor does not read (the
/// library is made again).
const HEAD: &str = "grizzly game library state 1";

/// What one file of the game was described from and as.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub stamp: String,
    /// The file's row of its catalog, as Lua data on one line; empty for a kind without rows.
    pub row: String,
}

/// The state of a library.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct State {
    /// Per kind: the version of what its descriptions state ([`crate::kinds`]).
    pub versions: BTreeMap<String, u32>,
    /// Per kind, per game path.
    pub files: BTreeMap<String, FxHashMap<String, Entry>>,
    /// What is made from several files at once (a level, the particle list): its stamp.
    pub groups: BTreeMap<String, String>,
}

impl State {
    /// Reads `state.txt`; a file that is missing, of another version or broken is an empty
    /// state - the library is then made again.
    #[must_use]
    pub fn read(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else { return Self::default() };
        let mut lines = text.lines();
        if lines.next() != Some(HEAD) {
            return Self::default();
        }
        let mut out = Self::default();
        for line in lines {
            let mut cols = line.splitn(5, '\t');
            match (cols.next(), cols.next(), cols.next(), cols.next(), cols.next()) {
                (Some("v"), Some(kind), Some(version), _, _) => {
                    if let Ok(v) = version.parse() {
                        out.versions.insert(kind.to_owned(), v);
                    }
                }
                (Some("g"), Some(group), Some(stamp), _, _) => {
                    out.groups.insert(group.to_owned(), stamp.to_owned());
                }
                (Some("f"), Some(kind), Some(path), Some(stamp), row) => {
                    out.files.entry(kind.to_owned()).or_default().insert(
                        path.to_owned(),
                        Entry { stamp: stamp.to_owned(), row: row.unwrap_or("").to_owned() },
                    );
                }
                _ => {}
            }
        }
        out
    }

    /// The state as text: the same state gives the same bytes.
    #[must_use]
    pub fn to_text(&self) -> String {
        let mut out = String::from(HEAD);
        out.push('\n');
        for (kind, v) in &self.versions {
            out.push_str(&format!("v\t{kind}\t{v}\n"));
        }
        for (group, stamp) in &self.groups {
            out.push_str(&format!("g\t{group}\t{stamp}\n"));
        }
        for (kind, files) in &self.files {
            let mut paths: Vec<&String> = files.keys().collect();
            paths.sort_unstable();
            for p in paths {
                let e = &files[p];
                out.push_str(&format!("f\t{kind}\t{p}\t{}\t{}\n", e.stamp, e.row));
            }
        }
        out
    }

    /// How many files of a kind are described.
    #[must_use]
    pub fn count(&self, kind: &str) -> usize {
        self.files.get(kind).map_or(0, FxHashMap::len)
    }
}

/// The stamps of a game's files: what tells that a file is another file now.
pub struct Stamps {
    /// Per archive of the index: its name, length and write time as one word.
    archives: Vec<String>,
    game_data: std::path::PathBuf,
}

fn time_of(meta: &std::fs::Metadata) -> u64 {
    meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs())
}

impl Stamps {
    /// Looks at every archive once. Blocking.
    #[must_use]
    pub fn of(files: &GameFiles) -> Self {
        let archives = files
            .archives()
            .iter()
            .map(|a| {
                let name = a.path.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
                match std::fs::metadata(&a.path) {
                    Ok(m) => format!("{name}:{}:{}", m.len(), time_of(&m)),
                    Err(_) => name,
                }
            })
            .collect();
        Self { archives, game_data: files.game_data().to_path_buf() }
    }

    /// A file's stamp: an archived one by its archive and its place in it, a loose one by its
    /// length and write time (one look at the disk). Blocking for a loose file.
    #[must_use]
    pub fn of_file(&self, f: FileRef<'_>) -> String {
        match f.source {
            Source::Archive { archive, offset, packed } => {
                let a = self.archives.get(archive as usize).map_or("?", String::as_str);
                format!("a:{a}:{offset}:{packed}:{}", f.size)
            }
            Source::Loose => match std::fs::metadata(self.game_data.join(f.path)) {
                Ok(m) => format!("l:{}:{}", m.len(), time_of(&m)),
                Err(_) => format!("l:{}", f.size),
            },
        }
    }
}

/// One word for several: a group's stamp from the stamps of what it is made from.
#[must_use]
pub fn join_stamps(parts: &[(String, String)]) -> String {
    let mut sorted: Vec<&(String, String)> = parts.iter().collect();
    sorted.sort();
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for (name, stamp) in sorted {
        for b in name.bytes().chain([0]).chain(stamp.bytes()).chain([0]) {
            h = (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    format!("{h:016x}:{}", parts.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_state_reads_back_and_a_strange_file_is_an_empty_state() {
        let mut s = State::default();
        s.versions.insert("model".into(), 3);
        s.groups.insert("level:l01_escape".into(), "abc:4".into());
        s.files.entry("texture".into()).or_default().insert(
            "textures/wood/a.dds".into(),
            Entry { stamp: "a:x.db0:1:2:3:4".into(), row: "{ name = \"wood\\\\a\", tab = \"\\t\" }".into() },
        );
        s.files
            .entry("sound".into())
            .or_default()
            .insert("sounds/a.ogg".into(), Entry { stamp: "l:1:2".into(), row: String::new() });
        let dir = std::env::temp_dir().join(format!("gz_gamelib_state_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.txt");
        std::fs::write(&path, s.to_text()).unwrap();
        assert_eq!(State::read(&path), s);
        assert_eq!(s.to_text(), State::read(&path).to_text());
        std::fs::write(&path, "something else\n").unwrap();
        assert_eq!(State::read(&path), State::default());
        assert_eq!(State::read(&dir.join("missing")), State::default());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_group_stamp_follows_its_parts_in_any_order() {
        let a = ("level".to_owned(), "1".to_owned());
        let b = ("level.geom".to_owned(), "2".to_owned());
        assert_eq!(join_stamps(&[a.clone(), b.clone()]), join_stamps(&[b.clone(), a.clone()]));
        assert_ne!(
            join_stamps(&[a.clone(), b]),
            join_stamps(&[a, ("level.geom".to_owned(), "3".to_owned())])
        );
    }
}
