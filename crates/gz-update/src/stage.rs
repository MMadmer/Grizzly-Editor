//! The updater's work in `<installation>\.update\<version>\` (docs/UPDATES.md, sections 4 and 5):
//!
//! ```text
//! state.ltx             the release (version, asset id, archive size, the manifest's SHA-256),
//!                       how it is read, and every file that passed
//! update-manifest.txt   the new version's manifest, installed last
//! notes.md              the release notes, for "What's New" after the update
//! files\<path>          the staged files (<path>.part and <path>.raw while one is in flight)
//! archive.zip.part      the whole archive, when it is read whole
//! apply.journal         the transaction, a line per move
//! old\<path>            what the transaction moved out of the way
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::manifest::{MANIFEST, WORK, path_key};

/// How the delta is read out of the archive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// The records of the files that differ, by range.
    Ranges,
    /// The whole archive, then the same files out of it.
    Whole,
}

impl Mode {
    #[must_use]
    pub fn word(self) -> &'static str {
        match self {
            Self::Ranges => "ranges",
            Self::Whole => "whole",
        }
    }
}

/// What `state.ltx` keeps.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StageState {
    pub version: String,
    pub asset: u64,
    pub size: u64,
    pub manifest_sha: String,
    pub mode: Mode,
    /// path key -> (sha256, size, path)
    pub passed: BTreeMap<String, (String, u64, String)>,
}

/// The folders and files of one version's stage.
#[derive(Clone, Debug)]
pub struct Stage {
    pub dir: PathBuf,
}

impl Stage {
    #[must_use]
    pub fn new(root: &Path, version: &str) -> Self {
        Self { dir: root.join(WORK).join(version) }
    }

    #[must_use]
    pub fn files(&self) -> PathBuf {
        self.dir.join("files")
    }

    #[must_use]
    pub fn old(&self) -> PathBuf {
        self.dir.join("old")
    }

    #[must_use]
    pub fn state_file(&self) -> PathBuf {
        self.dir.join("state.ltx")
    }

    #[must_use]
    pub fn manifest(&self) -> PathBuf {
        self.dir.join(MANIFEST)
    }

    #[must_use]
    pub fn notes(&self) -> PathBuf {
        self.dir.join("notes.md")
    }

    #[must_use]
    pub fn journal(&self) -> PathBuf {
        self.dir.join("apply.journal")
    }

    #[must_use]
    pub fn archive_part(&self) -> PathBuf {
        self.dir.join("archive.zip.part")
    }

    /// Why the last apply at exit did not happen, for the next start to say.
    #[must_use]
    pub fn failed_note(&self) -> PathBuf {
        self.dir.join("apply.failed")
    }

    /// The staged file of a manifest path.
    #[must_use]
    pub fn file(&self, rel: &str) -> PathBuf {
        self.files().join(rel.replace('/', "\\"))
    }

    pub fn read_state(&self) -> Option<StageState> {
        let text = fs::read_to_string(self.state_file()).ok()?;
        let mut s = StageState {
            version: String::new(),
            asset: 0,
            size: 0,
            manifest_sha: String::new(),
            mode: Mode::Ranges,
            passed: BTreeMap::new(),
        };
        for line in text.lines() {
            let Some((k, v)) = line.split_once('=') else { continue };
            let (k, v) = (k.trim(), v.trim());
            match k {
                "version" => s.version = v.to_owned(),
                "asset" => s.asset = v.parse().ok()?,
                "size" => s.size = v.parse().ok()?,
                "manifest" => s.manifest_sha = v.to_owned(),
                "mode" => s.mode = if v == "whole" { Mode::Whole } else { Mode::Ranges },
                "file" => {
                    let mut parts = v.splitn(3, '|');
                    let (Some(sha), Some(size), Some(path)) = (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    let Ok(size) = size.parse() else { continue };
                    s.passed.insert(path_key(path), (sha.to_owned(), size, path.to_owned()));
                }
                _ => {}
            }
        }
        (!s.version.is_empty()).then_some(s)
    }

    /// Writes `state.ltx` whole: a new file renamed over the old one.
    pub fn write_state(&self, s: &StageState) -> io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let mut text = format!(
            "; Grizzly Editor's update in progress: what is staged here and what it is for\n[release]\nversion = {}\nasset = {}\nsize = {}\nmanifest = {}\nmode = {}\n\n[passed]\n",
            s.version,
            s.asset,
            s.size,
            s.manifest_sha,
            s.mode.word()
        );
        for (sha, size, path) in s.passed.values() {
            text.push_str(&format!("file = {sha}|{size}|{path}\n"));
        }
        let tmp = self.dir.join("state.ltx.tmp");
        fs::write(&tmp, text)?;
        fs::rename(&tmp, self.state_file())
    }
}

/// Every stage under the installation's work folder, by version folder name.
#[must_use]
pub fn stages(root: &Path) -> Vec<Stage> {
    let Ok(entries) = fs::read_dir(root.join(WORK)) else { return Vec::new() };
    let mut out: Vec<Stage> =
        entries.flatten().filter(|e| e.path().is_dir()).map(|e| Stage { dir: e.path() }).collect();
    out.sort_by(|a, b| a.dir.cmp(&b.dir));
    out
}

/// Removes a folder and everything in it; a read-only file goes too.
pub fn remove_tree(dir: &Path) -> io::Result<()> {
    match fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(_) => {
            clear_read_only(dir);
            fs::remove_dir_all(dir)
        }
    }
}

fn clear_read_only(dir: &Path) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if let Ok(meta) = fs::symlink_metadata(&p) {
            if meta.is_dir() {
                clear_read_only(&p);
            } else if meta.permissions().readonly() {
                let mut perm = meta.permissions();
                #[allow(clippy::permissions_set_readonly_false)]
                perm.set_readonly(false);
                let _ = fs::set_permissions(&p, perm);
            }
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn the_state_reads_back() {
        let dir = tempfile::tempdir().unwrap();
        let stage = Stage::new(dir.path(), "0.7.1");
        let mut s = StageState {
            version: "0.7.1".into(),
            asset: 42,
            size: 1234,
            manifest_sha: "ab".repeat(32),
            mode: Mode::Whole,
            passed: BTreeMap::new(),
        };
        s.passed.insert(path_key("Data/A b.txt"), ("cd".repeat(32), 7, "Data/A b.txt".into()));
        stage.write_state(&s).unwrap();
        assert_eq!(stage.read_state().unwrap(), s);
        assert_eq!(stages(dir.path()).len(), 1);
        assert!(stage.file("a/b.txt").ends_with("files\\a\\b.txt"));
    }
}
