//! What the project area keeps while the editor runs.
//!
//! - [`Projects`] (editor scope): `_projects.ini`, the Start page's memory, read again before
//!   every use and saved as it is edited.
//! - [`Project`] (project scope, in `session.services`): the project file, this machine's state
//!   of the project (`<home>\projects\<id>.ltx`), the file tree, the scene to reopen and the
//!   game's index (`gz-vfs`) with the revision that moves whenever another index takes its place.
//!   The game is the project's own when it names one on this machine, else the editor's default
//!   ([`crate::Machine`]).
//!
//! Neither holds unsaved work, so no `DirtySource` is registered.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath, Rev, RevCounter};
use gz_editor::{CmdError, Editor, Executors, Log, ProjectSession, Store};
use gz_store::{Apply, State, StoreOptions};
use gz_vfs::{GameFiles, MountOptions, looks_like_game};
use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{Value, json};

use crate::diskdoc::{DiskDoc, read_disk, write_disk};
use crate::games::{Machine, clean_root};
use crate::list::{self, ProjectList, Sort};
use crate::machine::{self, Found, ProjectState};
use crate::paths;
use crate::project::ProjectFile;
use crate::tree::TreeWatch;

/// The project list of the editor.
pub struct Projects {
    file: DiskDoc<ProjectList>,
}

impl Projects {
    /// The list service, made on first use.
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Projects> {
        editor.services.get_or_init(|| Projects {
            file: DiskDoc::new(
                "projects",
                editor.paths.home.join(list::FILE),
                ProjectList::default(),
                &editor.exec,
            ),
        })
    }

    /// The list as the file has it now. A file that cannot be read leaves the last one read.
    pub async fn list(&self, editor: &Editor) -> ProjectList {
        match self.file.load().await {
            Ok(list) => list,
            Err(e) => {
                editor.log.warn(e.message);
                self.file.snapshot()
            }
        }
    }

    /// The list as it was last read, without reading the file.
    #[must_use]
    pub fn cached(&self) -> ProjectList {
        self.file.snapshot()
    }

    /// The New Project form's location: the list's, or `Documents\GrizzlyProjects`.
    #[must_use]
    pub fn create_parent(list: &ProjectList) -> String {
        match list.create_parent() {
            Some(p) => p.to_owned(),
            None => {
                let profile = std::env::var("USERPROFILE").unwrap_or_default();
                format!("{profile}\\Documents\\GrizzlyProjects")
            }
        }
    }

    /// Applies an edit to the list as the file has it now and writes it.
    pub(crate) async fn edit(&self, f: impl FnOnce(&ProjectList) -> ProjectList) -> Result<(), CmdError> {
        self.file.edit(f).await.map(|_| ())
    }

    /// Records the Start page's sort and saves it.
    pub async fn set_sort(&self, sort: Sort) -> Result<(), CmdError> {
        self.edit(|l| l.with_sort(sort)).await
    }
}

#[derive(Clone)]
struct FileState {
    file: ProjectFile,
}

impl State for FileState {
    type Cmd = ProjectFile;
    type Reply = ();

    fn apply(&mut self, next: ProjectFile, cx: &mut Apply) -> Result<(), CmdError> {
        self.file = next;
        cx.content_changed();
        Ok(())
    }
}

#[derive(Default)]
struct SceneSlots {
    /// The scene open as of the last frame.
    scene: Option<RelPath>,
    /// A scene load succeeded since the project opened.
    loaded: bool,
    /// The scene to open once the project is up (the project file's `scene`, when it exists).
    pending: Option<RelPath>,
    /// The autoload handed out by [`Project::take_pending_scene`].
    popped: Option<RelPath>,
}

/// Whose game the open project reads, and why.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GameSource {
    /// The project names a game of its own on this machine (Link Game).
    Project,
    /// The editor's default game, found the way it says.
    Default(Found),
}

impl GameSource {
    /// `project`, or how the default was found (`installed`, `linked`, ...).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            GameSource::Project => "project",
            GameSource::Default(Found::None) => "none",
            GameSource::Default(f) => f.key(),
        }
    }
}

/// Which game a link sets.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LinkScope {
    /// This project's own, on this machine.
    Project,
    /// The editor's default, for every project that names none.
    Default,
}

/// The open project. In `session.services`, made by the open.
pub struct Project {
    root: String,
    name: String,
    id: String,
    /// The project file, as it was last read or written.
    file: Store<FileState>,
    file_path: PathBuf,
    /// This machine's state of the project.
    state: Arc<DiskDoc<ProjectState>>,
    home: PathBuf,
    machine: Arc<Machine>,
    exec: Executors,
    file_lock: tokio::sync::Mutex<()>,
    /// The project's files, kept current.
    pub tree: TreeWatch,
    scene: Mutex<SceneSlots>,
    game: Mutex<Option<GameMount>>,
    /// Moves whenever another game index takes the place of the last one.
    game_rev: RevCounter,
    /// `appdata\game_index`: where the game indexes are saved between starts.
    index_dir: PathBuf,
    log: Log,
    /// [`Project::game_linked_now`]'s last answer: the game root it was for, when, the answer.
    linked: Arc<Mutex<Option<(String, std::time::Instant, bool)>>>,
    /// The project's own game was a game when it was last looked at (the open, a link): one that
    /// went away - a drive not plugged in - leaves the project on the default game.
    own_ok: std::sync::atomic::AtomicBool,
}

/// The game's index and the revision it was put in place at.
struct GameMount {
    root: String,
    files: Arc<GameFiles>,
    rev: Rev,
}

/// Why the open project has no game to read.
pub const NOT_LINKED: &str =
    "no Dead Air Refined install is known: link one (Link Game, or Preferences > Game)";

impl Project {
    pub(crate) fn new(
        editor: &Editor,
        root: &str,
        file_path: PathBuf,
        file: ProjectFile,
        state: ProjectState,
    ) -> Project {
        let name = paths::leaf_of(root).to_owned();
        let id = file.id().to_owned();
        let store = Store::spawn("project", FileState { file }, &editor.exec, StoreOptions::default());
        let (tree, problem) = TreeWatch::start(PathBuf::from(root), &editor.exec);
        if let Some(p) = problem {
            editor.log.warn(p);
        }
        Project {
            root: root.to_owned(),
            name,
            state: Arc::new(DiskDoc::new(
                "project_state",
                machine::state_path(&editor.paths.home, &id),
                state,
                &editor.exec,
            )),
            id,
            file: store,
            file_path,
            home: editor.paths.home.clone(),
            machine: Machine::of(editor),
            exec: editor.exec.clone(),
            file_lock: tokio::sync::Mutex::new(()),
            tree,
            scene: Mutex::new(SceneSlots::default()),
            game: Mutex::new(None),
            game_rev: RevCounter::new(),
            index_dir: editor.paths.appdata.join("game_index"),
            log: editor.log.clone(),
            linked: Arc::new(Mutex::new(None)),
            own_ok: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Looks at the project's own game again (the open calls it): whether the project reads it
    /// or the default.
    pub(crate) async fn check_own_game(&self) {
        let own = self.state.snapshot().game_root();
        let ok = !own.is_empty() && self.exec.io(move || looks_like_game(&own).is_ok()).await;
        self.own_ok.store(ok, std::sync::atomic::Ordering::Relaxed);
    }

    /// The open project and its service, or `no active project`.
    pub fn of(editor: &Editor) -> Result<(Arc<ProjectSession>, Arc<Project>), CmdError> {
        let session = editor.project()?;
        let project = session.services.get::<Project>().ok_or_else(CmdError::no_project)?;
        Ok((session, project))
    }

    /// The folder, as it was opened (backslashes).
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// The folder's name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The project's id: what names it on this and every machine.
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// The project file on disk.
    #[must_use]
    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    /// The project file as it was last read or written.
    #[must_use]
    pub fn file(&self) -> ProjectFile {
        self.file.snapshot().file.clone()
    }

    /// This machine's state of the project, as last read or written.
    #[must_use]
    pub fn state(&self) -> ProjectState {
        self.state.snapshot()
    }

    /// Where the window keeps the tabs the project brings back.
    #[must_use]
    pub fn tabs_file(&self) -> PathBuf {
        machine::tabs_path(&self.home, &self.id)
    }

    /// The game this project reads: its own on this machine, else the editor's default.
    #[must_use]
    pub fn game_root(&self) -> String {
        self.game_source().0
    }

    /// The game this project reads and why: its own while that is a game, else the default.
    #[must_use]
    pub fn game_source(&self) -> (String, GameSource) {
        let own = self.state.snapshot().game_root();
        if !own.is_empty() && self.own_ok.load(std::sync::atomic::Ordering::Relaxed) {
            return (own, GameSource::Project);
        }
        let m = self.machine.snapshot();
        (m.game_root(), GameSource::Default(m.found()))
    }

    /// Where Build Into Game wrote last on this machine (`""`: never).
    #[must_use]
    pub fn export_target(&self) -> String {
        self.state.snapshot().export_target()
    }

    /// Remembers where Build Into Game wrote, on this machine.
    pub async fn set_export_target(&self, target: &str) -> Result<(), CmdError> {
        self.state.edit(|s| s.with_export_target(target)).await.map(|_| ())
    }

    pub(crate) async fn edit_state(
        &self,
        f: impl FnOnce(&ProjectState) -> ProjectState,
    ) -> Result<(), CmdError> {
        self.state.edit(f).await.map(|_| ())
    }

    /// Notes a scene that was just opened as the newest of this machine's recent scenes (File >
    /// Recent Scene). A scene outside the project is not noted.
    pub async fn note_recent_scene(&self, file: &std::path::Path) -> Result<(), CmdError> {
        let Some(rel) = crate::project::scene_in_project(&self.root, &file.to_string_lossy()) else {
            return Ok(());
        };
        let wire = rel.replace('\\', "/");
        self.edit_state(|s| s.with_recent_scene(&wire)).await
    }

    /// Applies `f` to the project file as the disk has it now and writes it when it changed. A
    /// file that no longer reads is not written over: the author's text wins.
    pub async fn edit_file(
        &self,
        f: impl FnOnce(ProjectFile) -> ProjectFile,
    ) -> Result<ProjectFile, CmdError> {
        let _one = self.file_lock.lock().await;
        let before = read_disk(&self.exec, &self.file_path).await?;
        let current = if before.is_empty() {
            self.file()
        } else {
            ProjectFile::parse(&before).map_err(|e| {
                CmdError::new(
                    "refused",
                    format!("{} does not read ({e}): fix it, nothing was written", self.file_path.display()),
                )
            })?
        };
        let next = f(current);
        if !next.changed() {
            self.file.send(next.clone(), Origin::Mcp).await?;
            return Ok(next);
        }
        let text = next.to_text().map_err(|e| CmdError::new("failed", e.to_string()))?;
        write_disk(&self.exec, &self.file_path, &before, text.clone()).await?;
        let written = next.written(&text);
        self.file.send(written.clone(), Origin::Mcp).await?;
        Ok(written)
    }

    /// What the project's scenes take from the editor's library (the project file's list).
    #[must_use]
    pub fn library(&self) -> Vec<String> {
        self.file().library()
    }

    /// The guard every writer of this project passes: nothing under the game's root but
    /// `module`'s install target and flat export (`None`: nothing at all).
    #[must_use]
    pub fn write_guard(&self, module: Option<&str>) -> gz_editor::fsx::WriteGuard {
        gz_editor::fsx::WriteGuard::new(&self.game_root(), module)
    }

    /// Whether a game is known and its folder still looks like one. Touches the disk.
    #[must_use]
    pub fn game_linked(&self) -> bool {
        let root = self.game_root();
        !root.is_empty() && looks_like_game(&root).is_ok()
    }

    /// [`Project::game_linked`] without touching the disk: the last answer, looked at again on
    /// the IO pool when it is a second old or the game root moved. For the window, which draws
    /// it every frame and must never wait on a file.
    #[must_use]
    pub fn game_linked_now(&self) -> bool {
        let root = self.game_root();
        let mut slot = self.linked.lock();
        let (answer, stale) = match slot.as_ref() {
            Some((r, at, a)) if *r == root => (*a, at.elapsed() >= std::time::Duration::from_secs(1)),
            _ => (false, true),
        };
        if stale {
            // the answer stands until the look ends; one look at a time
            *slot = Some((root.clone(), std::time::Instant::now(), answer));
            drop(slot);
            let cache = self.linked.clone();
            self.exec.spawn_io(move || {
                let a = !root.is_empty() && looks_like_game(&root).is_ok();
                let mut s = cache.lock();
                if s.as_ref().is_some_and(|(r, _, _)| *r == root) {
                    *s = Some((root, std::time::Instant::now(), a));
                }
            });
        }
        answer
    }

    /// The scene area says which scene is open (`None`: none), once a frame or on a load. A
    /// `Some` counts as a load that succeeded.
    pub fn note_scene(&self, scene: Option<RelPath>) {
        let mut s = self.scene.lock();
        if scene.is_some() && !s.loaded {
            s.loaded = true;
            // the load got through: the next start may open the scene again
            if s.popped.is_some() {
                let state = self.state.clone();
                self.exec.spawn(async move {
                    let _ = state.edit(|st| st.with_loading_scene("")).await;
                });
            }
        }
        s.scene = scene;
    }

    pub(crate) fn set_pending_scene(&self, scene: Option<RelPath>) {
        self.scene.lock().pending = scene;
    }

    /// The scene to open now that the project is up, once. It is noted in this machine's state
    /// before it loads: a load that brings the editor down is not tried again at the next start.
    pub async fn take_pending_scene(&self) -> Result<Option<RelPath>, CmdError> {
        let taken = {
            let mut s = self.scene.lock();
            let taken = s.pending.take();
            if taken.is_some() {
                s.popped.clone_from(&taken);
            }
            taken
        };
        if let Some(rel) = &taken {
            let rel = rel.to_string();
            self.state.edit(|s| s.with_loading_scene(&rel)).await?;
        }
        Ok(taken)
    }

    /// What the close remembers as the scene to open: the scene open as the project closes; the
    /// autoload's when it never came to anything; the one the autoload never reached.
    pub(crate) fn scene_to_remember(&self) -> Option<String> {
        let s = self.scene.lock();
        let earlier = if !s.loaded && s.popped.is_some() { s.popped.as_ref() } else { s.pending.as_ref() };
        s.scene.as_ref().or(earlier).map(RelPath::to_backslash)
    }

    /// Mounts the game at `root` (over `prev`, the saved index then being that one). Blocking.
    fn mount(&self, root: &str, prev: Option<&GameFiles>) -> Result<GameFiles, String> {
        let opts = MountOptions { cache_dir: Some(self.index_dir.clone()) };
        let files = match prev {
            Some(p) => p.refresh(&opts),
            None => GameFiles::mount(Path::new(root), &opts),
        }
        .map_err(|e| e.to_string())?;
        for (path, why) in files.skipped() {
            self.log
                .warn(format!("game files: '{}' is not a readable archive, skipped ({why})", path.display()));
        }
        if let Some(problem) = &files.stats().cache_problem {
            self.log.warn(problem.clone());
        }
        self.log.info(format!(
            "Game content mounted: {} archives, {} files ({root})",
            files.archives().len(),
            files.len()
        ));
        Ok(files)
    }

    fn put_game(&self, root: &str, files: Arc<GameFiles>) -> Rev {
        let rev = self.game_rev.bump();
        *self.game.lock() = Some(GameMount { root: root.to_owned(), files, rev });
        rev
    }

    /// Puts an index in place unless one of the same root is there already: that one is kept.
    fn put_game_first(&self, root: &str, files: Arc<GameFiles>) -> (Arc<GameFiles>, Rev) {
        let mut game = self.game.lock();
        if let Some(m) = game.as_ref().filter(|m| m.root.eq_ignore_ascii_case(root)) {
            return (m.files.clone(), m.rev);
        }
        let rev = self.game_rev.bump();
        *game = Some(GameMount { root: root.to_owned(), files: files.clone(), rev });
        (files, rev)
    }

    /// The game root when it still looks like a game, or [`NOT_LINKED`].
    fn linked_root(&self) -> Result<String, String> {
        let root = self.game_root();
        if root.is_empty() || looks_like_game(&root).is_err() {
            return Err(NOT_LINKED.into());
        }
        Ok(root)
    }

    fn current_game(&self, root: &str) -> Option<(Arc<GameFiles>, Rev)> {
        self.game
            .lock()
            .as_ref()
            .filter(|m| m.root.eq_ignore_ascii_case(root))
            .map(|m| (m.files.clone(), m.rev))
    }

    /// The game's index and its revision, mounted on first need and again when the game changes.
    /// Blocking: call it on the IO pool (the mount runs its parallel work there).
    pub fn game_snapshot(&self) -> Result<(Arc<GameFiles>, Rev), String> {
        let root = self.linked_root()?;
        if let Some(hit) = self.current_game(&root) {
            return Ok(hit);
        }
        // No lock is held across the mount: it runs parallel work on the IO pool, and a worker
        // waiting inside that work runs other queued jobs - another caller of this function, which
        // would wait on a lock the same thread holds further down its stack. Two first callers may
        // mount twice; the first index put in place is the one both get.
        let files = Arc::new(self.mount(&root, None)?);
        Ok(self.put_game_first(&root, files))
    }

    /// The game's index. Blocking, like [`Project::game_snapshot`].
    pub fn game_files(&self) -> Result<Arc<GameFiles>, String> {
        self.game_snapshot().map(|(files, _)| files)
    }

    /// The revision of the game index in place: what everything derived from the game is keyed by.
    #[must_use]
    pub fn game_rev(&self) -> Rev {
        self.game_rev.get()
    }

    /// Checks the game against the disk again (only what changed is read) and puts the new index
    /// in place. Blocking.
    pub fn refresh_game(&self) -> Result<(Arc<GameFiles>, Rev), String> {
        let root = self.linked_root()?;
        // no lock across the mount, for the reason game_snapshot gives: the last refresh wins
        let prev = self.current_game(&root).map(|(files, _)| files);
        let files = Arc::new(self.mount(&root, prev.as_deref())?);
        let rev = self.put_game(&root, files.clone());
        Ok((files, rev))
    }

    /// Links the game in `folder`: this project's own on this machine, or the editor's default.
    /// The folder is checked and its content mounted first. The reason on failure.
    pub async fn link_game(self: &Arc<Self>, folder: &str, scope: LinkScope) -> Result<(), String> {
        let clean = clean_root(folder);
        let probe = clean.clone();
        self.exec.io(move || looks_like_game(&probe)).await.map_err(str::to_owned)?;
        let mounted = self.current_game(&clean).is_some();
        if !mounted {
            let (p, probe) = (self.clone(), clean.clone());
            // no lock across the mount (game_snapshot says why)
            let files = self.exec.io(move || p.mount(&probe, None)).await?;
            self.put_game(&clean, Arc::new(files));
        }
        match scope {
            LinkScope::Project => {
                self.state.edit(|s| s.with_game_root(&clean)).await.map_err(|e| e.message)?;
                self.own_ok.store(true, std::sync::atomic::Ordering::Relaxed);
                self.machine.note_seen(&clean).await;
            }
            LinkScope::Default => {
                self.machine.set_default_game(&clean, Found::Preference).await?;
            }
        }
        self.log.info(format!("Game linked: {clean}"));
        Ok(())
    }

    /// Takes the project's own game away: it reads the editor's default from now on.
    pub async fn use_default_game(&self) -> Result<(), String> {
        self.state.edit(|s| s.with_game_root("")).await.map_err(|e| e.message)?;
        Ok(())
    }
}

/// What the `state` command answers about the project: `project_active`, `project` (the
/// folder, forward slashes) and `project_name`.
#[must_use]
pub fn project_state(editor: &Editor) -> Value {
    match Project::of(editor) {
        Ok((_, p)) => {
            json!({ "project_active": true, "project": paths::wire(p.root()), "project_name": p.name() })
        }
        Err(_) => json!({ "project_active": false, "project": "", "project_name": "" }),
    }
}
