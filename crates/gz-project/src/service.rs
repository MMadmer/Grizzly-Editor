//! What the project area keeps while the editor runs.
//!
//! - [`Projects`] (editor scope): `_projects.ini` in a store, saved as it is edited. The file is
//!   read again before every use, as the C++ did: QA restores it behind the editor's back, and
//!   a list that only lived in memory would write the old one over it.
//! - [`Project`] (project scope, in `session.services`): `project.ltx` in a store, saved as it
//!   is edited, the file tree, the scene to reopen and the linked game's index (`gz-vfs`), with
//!   the revision that moves whenever another index takes its place.
//!
//! Neither holds unsaved work, so no `DirtySource` is registered.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, RelPath, Rev, RevCounter};
use gz_editor::{CmdError, Editor, Executors, Log, ProjectSession, Store};
use gz_store::{Apply, State, StoreOptions};
use gz_vfs::{GameFiles, MountOptions, looks_like_game};
use parking_lot::Mutex;
use serde_json::{Value, json};

use crate::list::{self, ProjectList, Sort};
use crate::paths;
use crate::project::ProjectFile;
use crate::tree::TreeWatch;

/// Reads a file the C++ edited in place with `WritePrivateProfileString`; `""` when absent.
async fn read_disk(exec: &Executors, path: &Path) -> Result<String, CmdError> {
    let file = path.to_path_buf();
    match exec.io(move || gz_editor::fsx::read_opt(&file)).await {
        Ok(Some(bytes)) => Ok(gz_base::text::decode_guess(&bytes).into_owned()),
        Ok(None) => Ok(String::new()),
        Err(e) => Err(CmdError::io(format!("{} could not be read: {e}", path.display()))),
    }
}

/// Writes `text` when it differs from what was read: an untouched file keeps every byte.
async fn write_disk(exec: &Executors, path: &Path, before: &str, text: String) -> Result<(), CmdError> {
    if text == before {
        return Ok(());
    }
    let file = path.to_path_buf();
    exec.io(move || gz_editor::fsx::write_atomic(&file, text.as_bytes()))
        .await
        .map_err(|e| CmdError::io(format!("{} could not be saved: {e}", path.display())))
}

#[derive(Clone)]
pub(crate) struct ListState {
    list: ProjectList,
}

pub(crate) enum ListCmd {
    Load(ProjectList),
    Opened { project: String, create_parent: String },
    Forget(String),
    Sort(Sort),
}

impl State for ListState {
    type Cmd = ListCmd;
    type Reply = ();

    fn apply(&mut self, cmd: ListCmd, cx: &mut Apply) -> Result<(), CmdError> {
        let next = match cmd {
            ListCmd::Load(list) => list,
            ListCmd::Opened { project, create_parent } => self.list.opened(&project, &create_parent),
            ListCmd::Forget(path) => self.list.without(&path),
            ListCmd::Sort(sort) => self.list.with_sort(sort),
        };
        if next.to_text() == self.list.to_text() {
            cx.unchanged();
            return Ok(());
        }
        self.list = next;
        cx.content_changed();
        Ok(())
    }
}

/// The project list of the editor.
///
/// The file is read again before every use and every edit is applied to what the disk holds -
/// as `WritePrivateProfileString` did - then written back only when its text changed. That is
/// why it is no `AutoSave` document: a saver that remembers what it wrote would skip a write the
/// disk no longer holds after QA restored the file.
pub struct Projects {
    store: Store<ListState>,
    file: PathBuf,
    exec: Executors,
    /// One read-edit-write at a time.
    lock: tokio::sync::Mutex<()>,
}

impl Projects {
    /// The list service, made on first use.
    #[must_use]
    pub fn of(editor: &Editor) -> Arc<Projects> {
        editor.services.get_or_init(|| Projects {
            store: Store::spawn(
                "projects",
                ListState { list: ProjectList::default() },
                &editor.exec,
                StoreOptions::default(),
            ),
            file: editor.paths.home.join(list::FILE),
            exec: editor.exec.clone(),
            lock: tokio::sync::Mutex::new(()),
        })
    }

    async fn load(&self) -> Result<String, CmdError> {
        let text = read_disk(&self.exec, &self.file).await?;
        self.store.send(ListCmd::Load(ProjectList::parse(&text)), Origin::Mcp).await?;
        Ok(text)
    }

    /// The list as the file has it now. A file that cannot be read leaves the last one read.
    pub async fn list(&self, editor: &Editor) -> ProjectList {
        let _one = self.lock.lock().await;
        if let Err(e) = self.load().await {
            editor.log.warn(e.message);
        }
        self.store.snapshot().list.clone()
    }

    /// The list as it was last read, without reading the file.
    #[must_use]
    pub fn cached(&self) -> ProjectList {
        self.store.snapshot().list.clone()
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
    pub(crate) async fn edit(&self, cmd: ListCmd) -> Result<(), CmdError> {
        let _one = self.lock.lock().await;
        let before = self.load().await?;
        self.store.send(cmd, Origin::Mcp).await?;
        let text = self.store.snapshot().list.to_text();
        write_disk(&self.exec, &self.file, &before, text).await
    }

    /// Records the Start page's sort and saves it.
    pub async fn set_sort(&self, sort: Sort) -> Result<(), CmdError> {
        self.edit(ListCmd::Sort(sort)).await
    }
}

#[derive(Clone)]
pub(crate) struct FileState {
    file: ProjectFile,
}

pub(crate) enum FileCmd {
    Load(ProjectFile),
    Manifest { name: String, sdk_root: String, game_root: String },
    GameRoot(String),
    LastScene(Option<String>),
    Set { section: String, key: String, value: String },
}

impl State for FileState {
    type Cmd = FileCmd;
    type Reply = ();

    fn apply(&mut self, cmd: FileCmd, cx: &mut Apply) -> Result<(), CmdError> {
        let next = match cmd {
            FileCmd::Load(file) => file,
            FileCmd::Manifest { name, sdk_root, game_root } => {
                self.file.with_manifest(&name, &sdk_root, &game_root)
            }
            FileCmd::GameRoot(root) => self.file.with_game_root(&root),
            FileCmd::LastScene(rel) => self.file.with_last_scene(rel.as_deref()),
            FileCmd::Set { section, key, value } => self.file.with_value(&section, &key, &value),
        };
        if next.to_text() == self.file.to_text() {
            cx.unchanged();
            return Ok(());
        }
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
    /// The scene to open once the project is up (`last_scene` that exists).
    pending: Option<RelPath>,
    /// The autoload handed out by [`Project::take_pending_scene`].
    popped: Option<RelPath>,
}

/// The open project. In `session.services`, made by `open_project`.
pub struct Project {
    root: String,
    name: String,
    /// `project.ltx`, kept the way [`Projects`] keeps the list.
    store: Store<FileState>,
    file_path: PathBuf,
    exec: Executors,
    lock: tokio::sync::Mutex<()>,
    /// The project's files, kept current.
    pub tree: TreeWatch,
    scene: Mutex<SceneSlots>,
    game: Mutex<Option<GameMount>>,
    /// Moves whenever another game index takes the place of the last one.
    game_rev: RevCounter,
    /// One mount at a time: two first readers must not read the game twice.
    mount_lock: Mutex<()>,
    /// `appdata\game_index`: where the game indexes are saved between starts.
    index_dir: PathBuf,
    log: Log,
    /// [`Project::game_linked_now`]'s last answer: the game root it was for, when, the answer.
    linked: Arc<Mutex<Option<(String, std::time::Instant, bool)>>>,
}

/// The linked game's index and the revision it was put in place at.
struct GameMount {
    root: String,
    files: Arc<GameFiles>,
    rev: Rev,
}

/// Why the open project has no game to read.
pub const NOT_LINKED: &str = "the project is not linked to a game install";

impl Project {
    pub(crate) fn new(editor: &Editor, root: &str, name: &str, file: ProjectFile) -> Project {
        let store = Store::spawn("project", FileState { file }, &editor.exec, StoreOptions::default());
        let (tree, problem) = TreeWatch::start(PathBuf::from(root), &editor.exec);
        if let Some(p) = problem {
            editor.log.warn(p);
        }
        Project {
            root: root.to_owned(),
            name: name.to_owned(),
            store,
            file_path: Path::new(root).join(crate::project::FILE),
            exec: editor.exec.clone(),
            lock: tokio::sync::Mutex::new(()),
            tree,
            scene: Mutex::new(SceneSlots::default()),
            game: Mutex::new(None),
            game_rev: RevCounter::new(),
            mount_lock: Mutex::new(()),
            index_dir: editor.paths.appdata.join("game_index"),
            log: editor.log.clone(),
            linked: Arc::new(Mutex::new(None)),
        }
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

    /// `project.ltx` as it is now.
    #[must_use]
    pub fn file(&self) -> ProjectFile {
        self.store.snapshot().file.clone()
    }

    #[must_use]
    pub fn game_root(&self) -> String {
        self.store.snapshot().file.game_root().to_owned()
    }

    /// Whether a game is linked and its folder still looks like one. Touches the disk.
    #[must_use]
    pub fn game_linked(&self) -> bool {
        let root = self.game_root();
        !root.is_empty() && looks_like_game(&root).is_ok()
    }

    /// [`Project::game_linked`] without touching the disk: the last answer, looked at again on
    /// the IO pool when it is a second old or the game root moved. For the window, which draws
    /// it every frame and must never wait on a file (a build keeping the disk busy made a frame
    /// of 70 ms of a `stat`).
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

    /// Applies an edit to `project.ltx` as the disk has it now and writes it.
    pub(crate) async fn edit(&self, cmd: FileCmd) -> Result<(), CmdError> {
        let _one = self.lock.lock().await;
        let before = read_disk(&self.exec, &self.file_path).await?;
        self.store.send(FileCmd::Load(ProjectFile::parse(&before)), Origin::Mcp).await?;
        self.store.send(cmd, Origin::Mcp).await?;
        let text = self.store.snapshot().file.to_text();
        write_disk(&self.exec, &self.file_path, &before, text).await
    }

    /// Sets a key of `project.ltx` for another area (`[xms] export_target`, ...) and answers
    /// once it is on disk.
    pub async fn set_value(&self, section: &str, key: &str, value: &str) -> Result<(), CmdError> {
        self.edit(FileCmd::Set { section: section.to_owned(), key: key.to_owned(), value: value.to_owned() })
            .await
    }

    /// The scene area says which scene is open (`None`: none), once a frame or on a load. A
    /// `Some` counts as a load that succeeded.
    pub fn note_scene(&self, scene: Option<RelPath>) {
        let mut s = self.scene.lock();
        if scene.is_some() {
            s.loaded = true;
        }
        s.scene = scene;
    }

    pub(crate) fn set_pending_scene(&self, scene: Option<RelPath>) {
        self.scene.lock().pending = scene;
    }

    /// The scene to open now that the project is up, once. `last_scene` is cleared on disk
    /// before it loads, so that a load that crashes cannot lock the project out; the close
    /// writes it again.
    pub async fn take_pending_scene(&self) -> Result<Option<RelPath>, CmdError> {
        let taken = {
            let mut s = self.scene.lock();
            let taken = s.pending.take();
            if taken.is_some() {
                s.popped.clone_from(&taken);
            }
            taken
        };
        if taken.is_some() {
            self.edit(FileCmd::LastScene(Some(String::new()))).await?;
        }
        Ok(taken)
    }

    /// What the close writes as `last_scene`: the scene open as the project closes; the
    /// autoload's when it never came to anything; the one the autoload never reached.
    pub(crate) fn scene_to_remember(&self) -> Option<String> {
        let s = self.scene.lock();
        let earlier = if !s.loaded && s.popped.is_some() { s.popped.as_ref() } else { s.pending.as_ref() };
        s.scene.as_ref().or(earlier).map(RelPath::to_backslash)
    }

    /// Mounts the game at `root` (over `prev`, the saved index then being that one) and logs what
    /// the C++ logged. Blocking.
    fn mount(&self, root: &str, prev: Option<&GameFiles>) -> Result<GameFiles, String> {
        let opts = MountOptions { cache_dir: Some(self.index_dir.clone()) };
        let files = match prev {
            Some(p) => p.refresh(&opts),
            None => GameFiles::mount(Path::new(root), &opts),
        }
        .map_err(|e| e.to_string())?;
        for (path, why) in files.skipped() {
            self.log.warn(format!("DARF: '{}' is not a readable archive, skipped ({why})", path.display()));
        }
        if let Some(problem) = &files.stats().cache_problem {
            self.log.warn(problem.clone());
        }
        self.log.info(format!(
            "DARF content mounted: {} archives, {} files ({root})",
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

    /// The linked game root when it still looks like a game, or [`NOT_LINKED`].
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

    /// The linked game's index and its revision, mounted on first need and again when the link
    /// changes. Blocking: call it on the IO pool (the mount runs its parallel work there).
    pub fn game_snapshot(&self) -> Result<(Arc<GameFiles>, Rev), String> {
        let root = self.linked_root()?;
        if let Some(hit) = self.current_game(&root) {
            return Ok(hit);
        }
        let _one = self.mount_lock.lock();
        if let Some(hit) = self.current_game(&root) {
            return Ok(hit);
        }
        let files = Arc::new(self.mount(&root, None)?);
        let rev = self.put_game(&root, files.clone());
        Ok((files, rev))
    }

    /// The linked game's index. Blocking, like [`Project::game_snapshot`].
    pub fn game_files(&self) -> Result<Arc<GameFiles>, String> {
        self.game_snapshot().map(|(files, _)| files)
    }

    /// The revision of the game index in place: what everything derived from the game is keyed by.
    #[must_use]
    pub fn game_rev(&self) -> Rev {
        self.game_rev.get()
    }

    /// Checks the linked game against the disk again (only what changed is read) and puts the
    /// new index in place. Blocking.
    pub fn refresh_game(&self) -> Result<(Arc<GameFiles>, Rev), String> {
        let root = self.linked_root()?;
        let _one = self.mount_lock.lock();
        let prev = self.current_game(&root).map(|(files, _)| files);
        let files = Arc::new(self.mount(&root, prev.as_deref())?);
        let rev = self.put_game(&root, files.clone());
        Ok((files, rev))
    }

    /// Links the project to the game in `folder`: checks it, mounts its content and writes
    /// `[project] game_root` (`EditorProject::LinkGame`). The reason on failure.
    pub async fn link_game(self: &Arc<Self>, folder: &str) -> Result<(), String> {
        let clean = paths::trim_trailing(paths::to_backslash(folder)).trim_end_matches('\\').to_owned();
        let current = self.game_root();
        if !current.is_empty() && paths::same_path(&current, &clean) {
            let probe = clean.clone();
            return self.exec.io(move || looks_like_game(&probe)).await.map_err(str::to_owned);
        }
        let (p, probe) = (self.clone(), clean.clone());
        let files = self
            .exec
            .io(move || {
                let _one = p.mount_lock.lock();
                p.mount(&probe, None)
            })
            .await?;
        self.edit(FileCmd::GameRoot(clean.clone())).await.map_err(|e| e.message)?;
        self.put_game(&clean, Arc::new(files));
        self.log.info(format!("Game linked: {clean}"));
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
