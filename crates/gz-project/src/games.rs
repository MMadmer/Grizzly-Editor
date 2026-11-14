//! The game is found, not asked for: the editor keeps ONE default game in its home
//! (`machine.ltx`), and finds it by itself when it is unset or no game any more - off the UI
//! thread, in this order:
//!
//! 1. the game roots this home has linked before: the per-project state files and the old
//!    `project.ltx` values seen while converting;
//! 2. the installed Dead Air: Refined, by its installer's uninstall entry
//!    (`Software\Microsoft\Windows\CurrentVersion\Uninstall\{9732DFF1-E40D-4B23-B215-6D28B1DD0DE0}_is1`,
//!    `InstallLocation`, the current user's then the machine's);
//! 3. an `xrEngine.exe` next to or above the editor's own folder.
//!
//! The first one that is a game ([`gz_vfs::looks_like_game`]) wins and is saved. [`Machine`] is
//! the editor's service of it, and of the library folder.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_editor::{CmdError, Editor, Executors};
use gz_vfs::looks_like_game;
use serde::Serialize;

use crate::diskdoc::DiskDoc;
use crate::machine::{self, Found, MachineFile, ProjectState};
use crate::paths;

/// One place a game may be.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Candidate {
    /// The folder, backslashes, no trailing separator.
    pub root: String,
    pub found: Found,
    /// Why it is no game; `None`: it is one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

impl Candidate {
    #[must_use]
    pub fn valid(&self) -> bool {
        self.problem.is_none()
    }
}

/// Where the finder looks.
pub struct FindCx<'a> {
    pub home: &'a Path,
    pub exe_dir: &'a Path,
    /// The game folders the installer's entries name (read from the registry by the caller).
    pub installed: Vec<String>,
    /// The roots `machine.ltx` has seen.
    pub seen: Vec<String>,
    /// The engine's program name a game folder holds ([`ENGINE`]).
    pub engine: &'a str,
}

/// The program a game folder holds.
pub const ENGINE: &str = "xrEngine.exe";

/// The game roots the projects' state files of `home` name.
fn linked_before(home: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(home.join(machine::STATES)) else { return Vec::new() };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| {
            e.file_name().to_str().is_some_and(|n| {
                n.len() > 4
                    && n[n.len() - 4..].eq_ignore_ascii_case(".ltx")
                    && !n.to_ascii_lowercase().ends_with(".tabs.ltx")
            })
        })
        .map(|e| (e.metadata().and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH), e.path()))
        .collect();
    // the project worked on last first
    files.sort_by_key(|f| std::cmp::Reverse(f.0));
    files
        .into_iter()
        .filter_map(|(_, p)| std::fs::read(p).ok())
        .map(|b| ProjectState::parse(&gz_base::text::decode_guess(&b)).game_root())
        .filter(|r| !r.is_empty())
        .collect()
}

/// The folders next to and above the editor's that hold the engine.
fn beside(exe_dir: &Path, engine: &str) -> Vec<String> {
    exe_dir
        .ancestors()
        .filter(|d| d.join(engine).is_file())
        .filter_map(|d| d.to_str().map(str::to_owned))
        .collect()
}

/// Every place the finder looks at, in its order, each checked. Blocking: a few attribute reads
/// per place.
#[must_use]
pub fn candidates(cx: &FindCx<'_>) -> Vec<Candidate> {
    let mut places: Vec<(String, Found)> = Vec::new();
    for r in cx.seen.iter().cloned().chain(linked_before(cx.home)) {
        places.push((r, Found::Linked));
    }
    for r in &cx.installed {
        places.push((r.clone(), Found::Installed));
    }
    let beside_editor = if home_only() { Vec::new() } else { beside(cx.exe_dir, cx.engine) };
    for r in beside_editor {
        places.push((r, Found::BesideEditor));
    }
    let mut out: Vec<Candidate> = Vec::new();
    for (root, found) in places {
        let root = paths::trim_trailing(paths::to_backslash(root.trim())).trim_end_matches('\\').to_owned();
        if root.is_empty() || out.iter().any(|c| paths::same_path(&c.root, &root)) {
            continue;
        }
        let problem = looks_like_game(&root).err().map(str::to_owned);
        out.push(Candidate { root, found, problem });
    }
    out
}

/// What the QA suites give the finder instead of this machine (`GRIZZLY_GAME_SEARCH`): `home` -
/// only what this home knew, never the installed game; `installed:<folder>` - that folder as the
/// installer's entry, and nothing beside the editor. `None`: the machine as it is.
fn qa_search() -> Option<Vec<String>> {
    let v = std::env::var("GRIZZLY_GAME_SEARCH").ok()?;
    let v = v.trim();
    if v.eq_ignore_ascii_case("home") {
        return Some(Vec::new());
    }
    v.strip_prefix("installed:").map(|folder| vec![folder.trim().to_owned()])
}

fn home_only() -> bool {
    qa_search().is_some()
}

/// The installer's entries on this machine.
fn installed_roots() -> Vec<String> {
    if let Some(given) = qa_search() {
        return given;
    }
    #[cfg(windows)]
    return gz_win::installed_game_roots();
    #[cfg(not(windows))]
    Vec::new()
}

/// The editor's places on this machine: the default game and the library, `<home>\machine.ltx`.
pub struct Machine {
    file: DiskDoc<MachineFile>,
    home: PathBuf,
    exe_dir: PathBuf,
    data_root: PathBuf,
    exec: Executors,
    /// One finder at a time.
    finding: tokio::sync::Mutex<()>,
}

impl Machine {
    /// The service, made on first use. Its file is read by [`Machine::load`], which the editor
    /// starts once at start (`install`); until then the snapshot is empty.
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Machine> {
        editor.services.get_or_init(|| Machine {
            file: DiskDoc::new(
                "machine",
                editor.paths.home.join(machine::MACHINE_FILE),
                MachineFile::default(),
                &editor.exec,
            ),
            home: editor.paths.home.clone(),
            exe_dir: editor.paths.exe_dir.clone(),
            data_root: editor.paths.root.clone(),
            exec: editor.exec.clone(),
            finding: tokio::sync::Mutex::new(()),
        })
    }

    /// The file as it was last read: for readers that must not wait.
    #[must_use]
    pub fn snapshot(&self) -> MachineFile {
        self.file.snapshot()
    }

    pub async fn load(&self) -> Result<MachineFile, CmdError> {
        self.file.load().await
    }

    pub async fn edit(&self, f: impl FnOnce(&MachineFile) -> MachineFile) -> Result<MachineFile, CmdError> {
        self.file.edit(f).await
    }

    /// The default game's folder as last known (`""`: none). Never touches the disk.
    #[must_use]
    pub fn default_game(&self) -> String {
        self.snapshot().game_root()
    }

    /// The editor's library folder: the preference, else the editor's data root.
    #[must_use]
    pub fn library_root(&self) -> PathBuf {
        let set = self.snapshot().library_root();
        if set.is_empty() { self.data_root.clone() } else { PathBuf::from(set) }
    }

    fn find_cx_parts(&self) -> (PathBuf, PathBuf, Vec<String>) {
        (self.home.clone(), self.exe_dir.clone(), self.snapshot().seen())
    }

    /// Every place a game was looked for, checked (Link Game's pick list, `game.info`).
    pub async fn candidates(&self) -> Vec<Candidate> {
        let (home, exe, seen) = self.find_cx_parts();
        self.exec
            .io(move || {
                candidates(&FindCx {
                    home: &home,
                    exe_dir: &exe,
                    installed: installed_roots(),
                    seen,
                    engine: ENGINE,
                })
            })
            .await
    }

    /// The default game, found when it is unset or no game any more. `Err` says that none was
    /// found (and the default stays unset).
    pub async fn ensure_default_game(&self) -> Result<String, String> {
        let _one = self.finding.lock().await;
        let current = self.load().await.map_err(|e| e.message)?;
        let root = current.game_root();
        if !root.is_empty() {
            let probe = root.clone();
            if self.exec.io(move || looks_like_game(&probe).is_ok()).await {
                return Ok(root);
            }
        }
        self.find_locked().await
    }

    /// Looks for the default game again (Preferences > Game > Find Again), whatever it is now.
    pub async fn find_again(&self) -> Result<String, String> {
        let _one = self.finding.lock().await;
        self.find_locked().await
    }

    async fn find_locked(&self) -> Result<String, String> {
        let found = self.candidates().await;
        let Some(first) = found.into_iter().find(Candidate::valid) else {
            return Err("no Dead Air Refined install was found on this computer".to_owned());
        };
        let (root, how) = (first.root.clone(), first.found);
        self.edit(|m| m.with_game(&root, how)).await.map_err(|e| e.message)?;
        Ok(first.root)
    }

    /// Makes `folder` the default game: checked first.
    pub async fn set_default_game(&self, folder: &str, how: Found) -> Result<String, String> {
        let clean = clean_root(folder);
        let probe = clean.clone();
        self.exec.io(move || looks_like_game(&probe)).await.map_err(str::to_owned)?;
        self.edit(|m| m.with_game(&clean, how)).await.map_err(|e| e.message)?;
        Ok(clean)
    }

    /// Remembers a game root this home has been linked to (an old `project.ltx` converted).
    pub async fn note_seen(&self, root: &str) {
        let root = clean_root(root);
        if !root.is_empty() {
            let _ = self.edit(|m| m.with_seen(&root)).await;
        }
    }
}

/// A folder as the editor keeps a game root: backslashes, no trailing separator.
#[must_use]
pub fn clean_root(folder: &str) -> String {
    paths::trim_trailing(paths::to_backslash(folder.trim())).trim_end_matches('\\').to_owned()
}

/// The editor's library folder (Preferences > Library): every reader of the library joins its
/// `rawdata` and `gamedata` to this.
#[must_use]
pub fn library_root(editor: &Editor) -> PathBuf {
    Machine::of(editor).library_root()
}

/// What every writer of the editor's library passes before it writes: the library is the
/// editor's own, and nothing of a game is ever written - not the default game, not the open
/// project's, not one this home has known. A library folder set to a game (an old setting, a
/// link into one) is read and never written.
#[derive(Clone, Debug, Default)]
pub struct LibraryGuard {
    games: Vec<gz_editor::fsx::WriteGuard>,
}

impl LibraryGuard {
    /// The guard against these game folders (empty ones left out).
    #[must_use]
    pub fn against<'a>(games: impl IntoIterator<Item = &'a str>) -> Self {
        let mut seen: Vec<String> = Vec::new();
        let mut out = Vec::new();
        for game in games {
            let key = clean_root(game).to_lowercase();
            if key.is_empty() || seen.contains(&key) {
                continue;
            }
            seen.push(key);
            out.push(gz_editor::fsx::WriteGuard::new(game, None));
        }
        Self { games: out }
    }

    /// `Ok` when the library may write `path` (a file, or a folder it writes below). Blocking: it
    /// resolves where the paths really are, so a link cannot carry a write into a game.
    pub fn check(&self, path: &Path) -> Result<(), String> {
        for game in &self.games {
            if game.check(path).is_err() {
                return Err(format!(
                    "{} lies in a game's folder: the library is the editor's own and a game is never written (Preferences > Library names the library's folder)",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

/// The guard of the library's writers ([`LibraryGuard`]), from the games this editor knows now.
/// Never touches the disk; [`LibraryGuard::check`] does.
#[must_use]
pub fn library_guard(editor: &Editor) -> LibraryGuard {
    let machine = Machine::of(editor);
    let file = machine.snapshot();
    let mut games = vec![file.game_root()];
    games.extend(file.seen());
    if let Ok((_, project)) = crate::service::Project::of(editor) {
        games.push(project.game_root());
    }
    LibraryGuard::against(games.iter().map(String::as_str))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn game(dir: &Path) {
        std::fs::create_dir_all(dir.join("gamedata")).unwrap();
        std::fs::write(dir.join("fsgame.ltx"), "x").unwrap();
    }

    /// The library's writers never write under a game this editor knows: its files, its folder,
    /// and a folder that only leads into it. Another place is written; with no game known,
    /// nothing is refused.
    #[test]
    fn the_library_is_never_written_inside_a_game() {
        let t = tempfile::tempdir().unwrap();
        let (the_game, library) = (t.path().join("game"), t.path().join("library"));
        game(&the_game);
        std::fs::create_dir_all(library.join("gamedata")).unwrap();
        let guard = LibraryGuard::against([the_game.to_str().unwrap(), "", the_game.to_str().unwrap()]);
        assert_eq!(guard.games.len(), 1);
        let refused = guard.check(&the_game.join("gamedata").join("lanims.xr")).unwrap_err();
        assert!(refused.contains("a game is never written"), "{refused}");
        assert!(guard.check(&the_game).is_err());
        assert!(guard.check(&the_game.join("gamedata").join("textures").join("new").join("a.dds")).is_err());
        assert!(guard.check(&library.join("gamedata").join("lanims.xr")).is_ok());
        assert!(LibraryGuard::against([""]).check(&the_game.join("gamedata").join("lanims.xr")).is_ok());
    }

    #[test]
    fn the_finder_takes_what_this_home_knew_then_the_install_then_the_editors_folder() {
        let t = tempfile::tempdir().unwrap();
        let home = t.path().join("home");
        let (linked, installed, beside_dir) =
            (t.path().join("linked"), t.path().join("inst"), t.path().join("da"));
        game(&linked);
        game(&installed);
        game(&beside_dir);
        std::fs::write(beside_dir.join("engine.txt"), "x").unwrap();
        let exe_dir = beside_dir.join("tools").join("editor");
        std::fs::create_dir_all(&exe_dir).unwrap();
        std::fs::create_dir_all(home.join("projects")).unwrap();
        std::fs::write(
            home.join("projects").join("a.ltx"),
            ProjectState::default().with_game_root(linked.to_str().unwrap()).to_text(),
        )
        .unwrap();
        // a tabs file names no game
        std::fs::write(home.join("projects").join("a.tabs.ltx"), "[game]\r\nroot=Z:\\nope\r\n").unwrap();
        let gone = t.path().join("gone");
        let cx = FindCx {
            home: &home,
            exe_dir: &exe_dir,
            installed: vec![format!("{}\\", installed.display())],
            seen: vec![gone.display().to_string()],
            engine: "engine.txt",
        };
        let found = candidates(&cx);
        let order: Vec<(&str, Found, bool)> =
            found.iter().map(|c| (c.root.as_str(), c.found, c.valid())).collect();
        assert_eq!(
            order,
            [
                (gone.to_str().unwrap(), Found::Linked, false),
                (linked.to_str().unwrap(), Found::Linked, true),
                (installed.to_str().unwrap(), Found::Installed, true),
                (beside_dir.to_str().unwrap(), Found::BesideEditor, true),
            ]
        );
        // nothing known: the install
        let cx = FindCx {
            home: &t.path().join("fresh"),
            exe_dir: &exe_dir,
            installed: vec![installed.display().to_string()],
            seen: vec![],
            engine: "engine.txt",
        };
        assert_eq!(
            candidates(&cx).into_iter().find(Candidate::valid).map(|c| c.found),
            Some(Found::Installed)
        );
        // not even that: beside the editor
        let cx = FindCx {
            home: &t.path().join("fresh"),
            exe_dir: &exe_dir,
            installed: vec![],
            seen: vec![],
            engine: "engine.txt",
        };
        assert_eq!(
            candidates(&cx).into_iter().find(Candidate::valid).map(|c| c.found),
            Some(Found::BesideEditor)
        );
    }
}
