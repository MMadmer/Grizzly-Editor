//! What the scene area keeps while a project is open: the store, the documents the scene was read
//! from, the library, the pick tree, the recovery session, the load status, the problems its
//! files hold and the property panels' view; loading, saving and following the scene's files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use gz_base::{Origin, Problem, RelPath, SmolStr};
use gz_editor::{CmdError, DirtyDoc, DirtySource, Editor, JobError, ProjectSession};
use gz_project::Filter;
use gz_query::Derived;
use gz_store::{Store, StoreOptions};
use parking_lot::Mutex;

use crate::apply::{ReplaceKind, SceneCmd};
use crate::format::{self, SceneFiles, Texts};
use crate::library::Library;
use crate::model::{ObjData, SceneState};
use crate::pick::ScenePick;
use crate::recovery::{Recovery, Snapshot};
use crate::sdk;

/// What `scene.info` answers about the last load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadStatus {
    pub attempted: bool,
    pub loaded: bool,
    pub path: String,
    pub error: String,
}

/// The Details and Level Settings panels' own state (the search, open), and a chooser put up to
/// be looked at.
#[derive(Clone, Debug, Default)]
pub struct InspectorView {
    pub filter: [String; 2],
    pub open: [bool; 2],
    /// The row whose chooser is open, and the item picked in it.
    pub choosing: Option<String>,
    pub chosen: String,
    /// A tool's chooser put up over MCP to be looked at (`ui.dialog open id=group_pivot`).
    pub tool_chooser: Option<String>,
}

/// The open project's scene.
pub struct SceneService {
    pub store: Store<SceneState>,
    pub library: Arc<Library>,
    pub recovery: Arc<Recovery>,
    pick: Derived<ScenePick>,
    pub load: Mutex<LoadStatus>,
    pub view: Mutex<InspectorView>,
    /// The project's folder and the editor's library folder.
    pub root: PathBuf,
    pub editor_root: PathBuf,
    /// The Details sheet's lists from the linked game (story ids, the spawn sections' names), the
    /// newest read.
    prop_cx: parking_lot::RwLock<Arc<gz_entities::PropContext>>,
    /// The documents the open scene was read from or last saved to: what a save writes the
    /// difference into. `None` for a scene not in its own files yet (new, imported).
    files: Mutex<Option<Arc<SceneFiles>>>,
    /// What the open scene's files hold that is wrong, positioned (reading, following them).
    problems: Mutex<Vec<Problem>>,
    /// A scene was loaded or made in this session.
    opened: AtomicBool,
    /// Moves with every load: the task following the files of an older one stops.
    watch_gen: AtomicU64,
}

/// Makes the dirty source register once per editor.
struct DirtyRegistered;

struct SceneDirty;

impl DirtySource for SceneDirty {
    fn dirty(&self, editor: &Editor) -> Vec<DirtyDoc> {
        let Some(session) = editor.project_opt() else { return Vec::new() };
        let Some(svc) = session.services.get::<SceneService>() else { return Vec::new() };
        if !svc.store.is_dirty() {
            return Vec::new();
        }
        let snap = svc.store.snapshot();
        let name = snap.meta.name();
        vec![DirtyDoc {
            kind: SmolStr::new_static("scene"),
            name: if name.is_empty() { "Untitled scene".to_owned() } else { name },
            path: snap.meta.file.as_ref().map(|f| f.display().to_string()),
        }]
    }

    fn save<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else {
                return Ok(());
            };
            if !svc.store.is_dirty() {
                return Ok(());
            }
            let snap = svc.store.snapshot();
            let name = snap.meta.name();
            let Some(file) = snap.meta.file.clone() else {
                return Err(format!(
                    "{}: the scene has no file yet: scene.save path=<file> first",
                    if name.is_empty() { "Untitled scene" } else { &name }
                ));
            };
            save_scene(editor, &svc, file).await.map(|_| ()).map_err(|e| format!("{name}: {e}"))
        })
    }

    fn discard<'a>(&'a self, editor: &'a Arc<Editor>) -> gz_editor::SettleFuture<'a> {
        Box::pin(async move {
            let Some(svc) = editor.project_opt().and_then(|s| s.services.get::<SceneService>()) else {
                return Ok(());
            };
            if !svc.store.is_dirty() {
                return Ok(());
            }
            svc.store.send(SceneCmd::Forget, Origin::Mcp).await.map(|_| ()).map_err(|e| e.message)
        })
    }
}

impl SceneService {
    /// The scene of the open project, made on first use.
    pub fn of(editor: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
        let session = editor.project()?;
        Ok(Self::of_session(editor, &session))
    }

    pub(crate) fn of_session(editor: &Arc<Editor>, session: &Arc<ProjectSession>) -> Arc<SceneService> {
        if let Some(s) = session.services.get::<SceneService>() {
            return s;
        }
        editor.services.get_or_init(|| {
            editor.add_dirty_source(Arc::new(SceneDirty));
            DirtyRegistered
        });
        let root = session.root.clone();
        let editor_root = gz_project::library_root(editor);
        let svc = session.services.get_or_init(|| SceneService {
            store: Store::spawn("scene", SceneState::default(), &editor.exec, StoreOptions::default()),
            library: Arc::new(Library::new(vec![
                root.join("rawdata").join("objects"),
                editor_root.join("rawdata").join("objects"),
            ])),
            recovery: Arc::new(Recovery::new()),
            pick: Derived::new(editor.exec.clone()),
            load: Mutex::new(LoadStatus::default()),
            view: Mutex::new(InspectorView::default()),
            root,
            editor_root,
            prop_cx: parking_lot::RwLock::new(Arc::new(gz_entities::PropContext::default())),
            files: Mutex::new(None),
            problems: Mutex::new(Vec::new()),
            opened: AtomicBool::new(false),
            watch_gen: AtomicU64::new(0),
        });
        start_recovery_timer(editor, &svc);
        svc
    }

    /// The Details sheet's lists as last read.
    #[must_use]
    pub fn prop_cx(&self) -> Arc<gz_entities::PropContext> {
        self.prop_cx.read().clone()
    }

    pub(crate) fn set_prop_cx(&self, cx: gz_entities::PropContext) -> Arc<gz_entities::PropContext> {
        let cx = Arc::new(cx);
        *self.prop_cx.write() = cx.clone();
        cx
    }

    /// Whether a scene was loaded or made in this session.
    #[must_use]
    pub fn opened(&self) -> bool {
        self.opened.load(Ordering::Relaxed)
    }

    pub(crate) fn set_opened(&self) {
        self.opened.store(true, Ordering::Relaxed);
    }

    /// What the open scene's files hold that is wrong.
    #[must_use]
    pub fn problems(&self) -> Vec<Problem> {
        self.problems.lock().clone()
    }

    /// The documents the open scene was read from.
    #[must_use]
    pub fn files(&self) -> Option<Arc<SceneFiles>> {
        self.files.lock().clone()
    }

    /// The scene is no longer the one its files hold (a new scene).
    pub(crate) fn forget_files(&self) {
        *self.files.lock() = None;
        self.problems.lock().clear();
        self.watch_gen.fetch_add(1, Ordering::Relaxed);
        self.set_opened();
    }

    /// The pick tree of the snapshot there is now (built on the CPU pool once per revision of
    /// the scene and of the library).
    pub async fn pick(&self) -> (Arc<gz_store::Snap<SceneState>>, Arc<ScenePick>) {
        let snap = self.store.snapshot();
        let rev = snap.rev.mix(self.library.rev());
        let (s, lib) = (snap.clone(), self.library.clone());
        let pick = self.pick.fresh(rev, move || ScenePick::build(&s, &lib)).await;
        (snap, pick)
    }

    /// Loads every library object the scene names, on the IO pool: its scene objects' and its
    /// detail objects'.
    pub async fn prefetch_library(&self, editor: &Editor, state: &SceneState) {
        let mut refs: Vec<SmolStr> = state
            .objects
            .values()
            .filter_map(|o| match &o.data {
                ObjData::SceneObject(d) if !d.visual_mode() => Some(d.reference.clone()),
                _ => None,
            })
            .collect();
        if let Some(d) = state.bulk.details.as_ref() {
            refs.extend(d.model.indices.objects.iter().map(|o| o.reference.clone()));
        }
        if refs.is_empty() {
            return;
        }
        let lib = self.library.clone();
        let exec = editor.exec.clone();
        editor.exec.io(move || exec.io_pool().install(|| lib.prefetch(&refs))).await;
    }
}

/// The scene's files for a recovery snapshot: what a save would write, every file whole.
fn snapshot_of(
    state: &SceneState,
    files: Option<&SceneFiles>,
    settings: sdk::SyncSettings<'_>,
) -> Result<Snapshot, String> {
    let name = state
        .meta
        .file
        .as_ref()
        .and_then(|f| f.file_stem())
        .map_or_else(|| "untitled".to_owned(), |s| s.to_string_lossy().into_owned());
    let plan = format::write(state, files, settings)?;
    let mut out: Vec<(String, Vec<u8>)> = Vec::new();
    // what the save would write, and every file it would leave as it is
    out.push((format!("{name}.{}", format::EXT), plan.after.scene.text().as_bytes().to_vec()));
    for d in &plan.after.docs {
        out.push((format!("{name}/{}", d.file), d.doc.text().as_bytes().to_vec()));
    }
    for l in format::layers::LAYERS {
        if format::layers::has(l, &state.bulk) {
            out.push((format!("{name}/{}", l.file), format::layers::write(l, &state.bulk)));
        }
    }
    Ok(Snapshot { files: out })
}

fn start_recovery_timer(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let weak: Weak<SceneService> = Arc::downgrade(svc);
    let ed = editor.clone();
    editor.exec.spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(5)).await;
            let Some(svc) = weak.upgrade() else { break };
            if ed.quit_token().is_cancelled() {
                break;
            }
            let rec = svc.recovery.clone();
            let root = svc.root.clone();
            ed.exec.io(move || rec.bind(&root)).await;
            if !svc.recovery.due(svc.store.is_dirty()) {
                continue;
            }
            let linked = game_linked(&ed).await;
            if svc.recovery.can_snapshot(linked).is_err() {
                continue;
            }
            match snapshot(&ed, &svc).await {
                Ok(()) => ed.log.info("* recovery: snapshot saved"),
                Err(e) => {
                    svc.recovery.retry_later();
                    ed.log.warn(format!("! recovery: autosave failed: {e}"));
                }
            }
        }
    });
}

/// A recovery snapshot of the scene now.
pub(crate) async fn snapshot(ed: &Arc<Editor>, svc: &Arc<SceneService>) -> Result<(), String> {
    let snap = svc.store.snapshot();
    let source = snap.meta.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
    let files = svc.files();
    let settings = crate::game::settings(ed).await;
    let s = snap.clone();
    let made = ed
        .exec
        .cpu(move || {
            snapshot_of(
                &s,
                files.as_deref(),
                settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await?;
    let rec = svc.recovery.clone();
    ed.exec.io(move || rec.save_now(&made, &source)).await
}

/// Whether the project's game is linked (touches the disk: on the IO pool).
pub(crate) async fn game_linked(editor: &Arc<Editor>) -> bool {
    let Ok((_, project)) = gz_project::Project::of(editor) else { return false };
    editor.exec.io(move || project.game_linked()).await
}

/// The scene file a path means: project-relative or absolute, `.scene` or the X-Ray SDK's
/// `.level`; without an extension the `.scene` (or, when there is none, the `.level`).
pub fn resolve_scene(svc: &SceneService, path: &str) -> Result<PathBuf, CmdError> {
    let p = PathBuf::from(path.replace('/', "\\"));
    let full = if p.is_absolute() { p } else { svc.root.join(p) };
    match full.extension().and_then(|e| e.to_str()) {
        Some(e) if e.eq_ignore_ascii_case(format::EXT) || e.eq_ignore_ascii_case(sdk::EXT) => Ok(full),
        None => {
            let scene = full.with_extension(format::EXT);
            let level = full.with_extension(sdk::EXT);
            Ok(if !scene.is_file() && level.is_file() { level } else { scene })
        }
        Some(_) => Err(CmdError::invalid("a scene is a .scene file (or an X-Ray SDK .level to import)")),
    }
}

/// Whether a file is the X-Ray SDK's scene.
#[must_use]
pub fn is_sdk(file: &Path) -> bool {
    file.extension().is_some_and(|e| e.eq_ignore_ascii_case(sdk::EXT))
}

/// The texts of a scene's files (blocking).
pub fn read_texts(scene: &Path) -> std::io::Result<Texts> {
    let mut t = Texts { scene: std::fs::read_to_string(scene)?, ..Texts::default() };
    let dir = format::folder_of(scene);
    for cf in &format::CLASS_FILES {
        if let Ok(text) = std::fs::read_to_string(dir.join(cf.file)) {
            t.files.push((cf.file.to_owned(), text));
        }
    }
    for l in format::layers::LAYERS {
        if let Ok(b) = std::fs::read(dir.join(l.file)) {
            t.layers.push((l.file.to_owned(), b));
        }
    }
    Ok(t)
}

/// A scene's path as problems name it: project-relative, else as it is.
fn rel_of(root: &Path, file: &Path) -> RelPath {
    RelPath::from_fs(root, file)
        .or_else(|| RelPath::new(&file.display().to_string().replace('\\', "/")).ok())
        .unwrap_or_else(RelPath::root)
}

/// A scene read from its files, or an X-Ray SDK scene imported (blocking parts on the pools).
pub(crate) struct Opened {
    pub state: SceneState,
    pub files: Option<SceneFiles>,
    pub problems: Vec<Problem>,
}

/// Reads a scene's files into the model (blocking: run on the IO / CPU pools).
pub(crate) fn open_files(
    root: &Path,
    file: &Path,
    settings: sdk::SyncSettings<'_>,
) -> Result<Opened, String> {
    if is_sdk(file) {
        let main =
            std::fs::read(file).map_err(|e| format!("Cannot open scene file: {} ({e})", file.display()))?;
        let kind = sdk::scene_kind(&main)?;
        let parts = sdk::read_parts(file, &sdk::parts_needed(&main, kind));
        let loaded = sdk::parse_scene(&main, file, &parts, settings).map_err(|e| e.to_string())?;
        let rel = rel_of(root, file);
        let problems =
            loaded.warnings.into_iter().map(|w| Problem::warning("S040", w).in_file(rel.clone())).collect();
        return Ok(Opened { state: loaded.state, files: None, problems });
    }
    let texts = read_texts(file).map_err(|e| format!("Cannot open scene file: {} ({e})", file.display()))?;
    let main = rel_of(root, file);
    let dir = rel_of(root, &format::folder_of(file));
    let mut read = format::read(&texts, &main, &dir, settings).map_err(|r| r.to_string())?;
    read.state.meta.file = Some(file.to_path_buf());
    read.files.base.meta.file = Some(file.to_path_buf());
    Ok(Opened { state: read.state, files: Some(read.files), problems: read.problems })
}

/// The load as a job: the files on the IO pool, the parse on the CPU pool, the library objects
/// prefetched, then one command replaces the scene. Answers when it ended.
pub async fn load_scene(editor: &Arc<Editor>, svc: &Arc<SceneService>, file: PathBuf) -> Result<(), String> {
    {
        let mut st = svc.load.lock();
        *st = LoadStatus {
            attempted: true,
            loaded: false,
            path: file.display().to_string(),
            error: String::new(),
        };
    }
    if let Err(e) = svc.recovery.can_replace_scene() {
        svc.load.lock().error = e.clone();
        return Err(e);
    }
    let ed = editor.clone();
    let s = svc.clone();
    let f = file.clone();
    let job = editor.jobs.start("scene_load", Some("scene"), "Loading the scene", move |ctx| async move {
        ctx.stage("Reading the game's configs");
        let settings = crate::game::settings(&ed).await;
        // the Details sheet's lists from the same configs
        crate::cmd::prop_context(&ed).await;
        ctx.check()?;
        ctx.stage("Reading the scene");
        let (root, pf) = (s.root.clone(), f.clone());
        let opened = ctx
            .exec
            .io(move || {
                let st: sdk::SyncSettings<'_> =
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                open_files(&root, &pf, st)
            })
            .await
            .map_err(JobError::new)?;
        ctx.check()?;
        ctx.stage("Loading the library objects");
        s.prefetch_library(&ed, &opened.state).await;
        for p in opened.problems.iter().filter(|p| p.severity != gz_base::Severity::Note) {
            ed.log.warn(p.line_text());
        }
        let count = opened.state.objects.len();
        s.store
            .send(SceneCmd::Replace(Box::new(opened.state), ReplaceKind::Loaded), Origin::Mcp)
            .await
            .map_err(|e| JobError::new(e.message))?;
        *s.files.lock() = opened.files.map(Arc::new);
        *s.problems.lock() = opened.problems;
        s.set_opened();
        ed.log.info(format!("the scene is open: {count} objects"));
        Ok(serde_json::json!({ "objects": count }))
    });
    let id = match job {
        Ok(id) => id,
        Err(busy) => {
            let e = format!("{} is still running", busy.title);
            svc.load.lock().error = e.clone();
            return Err(e);
        }
    };
    let info = editor.jobs.wait(id).await;
    let result = match info {
        Some(i) if i.error.is_none() && i.state == gz_editor::JobState::Done => Ok(()),
        Some(i) => Err(i.error.unwrap_or_else(|| "the scene did not open".into())),
        None => Err("the scene did not open".into()),
    };
    {
        let mut st = svc.load.lock();
        match &result {
            Ok(()) => {
                st.loaded = true;
                st.error.clear();
            }
            Err(e) => st.error = e.clone(),
        }
    }
    if result.is_ok() {
        let rec = svc.recovery.clone();
        editor.exec.io(move || rec.reset()).await;
        note_scene(editor, svc, &file);
        follow(editor, svc);
    }
    result
}

/// A scene command needs the scene: one is open, or the project's scene is opened now (and
/// waited for). `no_scene` when the project has none.
pub async fn need_scene(editor: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
    let svc = SceneService::of(editor)?;
    // a load under way (the project's own) is waited for; a tool's job is no reason to wait
    let running: Vec<_> = editor
        .jobs
        .snapshot()
        .into_iter()
        .filter(|j| j.running() && j.kind == "scene_load")
        .map(|j| j.id)
        .collect();
    for id in running {
        editor.jobs.wait(id).await;
    }
    if svc.opened() {
        return Ok(svc);
    }
    let Ok((_, project)) = gz_project::Project::of(editor) else {
        return Err(CmdError::new("no_scene", "no scene is open"));
    };
    let Some(rel) = project.file().scene().map(str::to_owned) else {
        return Err(CmdError::new(
            "no_scene",
            "the project has no scene: scene.new makes one, scene.open opens one",
        ));
    };
    let file = svc.root.join(rel.replace('/', "\\"));
    load_scene(editor, &svc, file).await.map_err(|e| CmdError::new("load_failed", e))?;
    Ok(svc)
}

/// Who is told of a scene loaded or saved: File > Recent Scene.
#[derive(Default)]
pub struct SceneFileHooks {
    hooks: parking_lot::RwLock<Vec<FileHook>>,
}

type FileHook = Box<dyn Fn(&Editor, &Path) + Send + Sync>;

impl SceneFileHooks {
    pub fn of(editor: &Editor) -> Arc<SceneFileHooks> {
        editor.services.get_or_init(SceneFileHooks::default)
    }

    /// `f` runs after every scene load and save that succeeded, with the scene's file.
    pub fn on_file(&self, f: impl Fn(&Editor, &Path) + Send + Sync + 'static) {
        self.hooks.write().push(Box::new(f));
    }

    fn fire(&self, editor: &Editor, file: &Path) {
        for h in self.hooks.read().iter() {
            h(editor, file);
        }
    }
}

/// The project's scene follows the open scene.
pub(crate) fn note_scene(editor: &Arc<Editor>, svc: &SceneService, file: &Path) {
    SceneFileHooks::of(editor).fire(editor, file);
    if let Ok((_, project)) = gz_project::Project::of(editor) {
        project.note_scene(RelPath::from_fs(&svc.root, file));
    }
}

/// Writes the files of a save: each changed file atomically, a file the scene no longer has
/// removed; a write that fails puts back what was written before it. Every write is told to
/// the project's watcher as the editor's own. Answers the files written or removed.
pub(crate) fn commit(
    scene: &Path,
    files: &[format::Out],
    watch: Option<&gz_project::ProjectWatch>,
) -> Result<usize, String> {
    let dir = format::folder_of(scene);
    let path_of = |f: &format::Out| if f.file.is_empty() { scene.to_path_buf() } else { dir.join(&f.file) };
    let mut done: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let rollback = |done: &[(PathBuf, Option<Vec<u8>>)]| {
        for (p, old) in done.iter().rev() {
            match old {
                Some(b) => {
                    let _ = gz_editor::fsx::write_atomic(p, b);
                }
                None => {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
    };
    for f in files {
        let p = path_of(f);
        let old = std::fs::read(&p).ok();
        match &f.bytes {
            Some(bytes) => {
                if let Some(parent) = p.parent()
                    && let Err(e) = std::fs::create_dir_all(parent)
                {
                    rollback(&done);
                    return Err(format!("the folder {} could not be made ({e})", parent.display()));
                }
                if let Err(e) = gz_editor::fsx::write_atomic(&p, bytes) {
                    rollback(&done);
                    return Err(format!("{} could not be written ({e})", p.display()));
                }
                if let Some(w) = watch {
                    w.note_written_fs(&p, bytes);
                }
                done.push((p, old));
            }
            None => {
                if old.is_some() {
                    if let Err(e) = std::fs::remove_file(&p) {
                        rollback(&done);
                        return Err(format!("{} could not be removed ({e})", p.display()));
                    }
                    done.push((p, old));
                }
            }
        }
    }
    Ok(done.len())
}

/// The save: the files worked out off the store (against the documents the scene was read from,
/// unless `file` is another one - a Save As writes everything), written on the IO pool, then
/// the store marked saved. Answers the files written or removed.
pub async fn save_scene(
    editor: &Arc<Editor>,
    svc: &Arc<SceneService>,
    file: PathBuf,
) -> Result<usize, String> {
    crate::edit::writable(editor, &file).map_err(|e| e.message)?;
    let snap = svc.store.snapshot();
    let same = snap.meta.file.as_deref() == Some(file.as_path()) && snap.meta.imported.is_none();
    let files = if same { svc.files() } else { None };
    let settings = crate::game::settings(editor).await;
    let s = snap.clone();
    let plan = editor
        .exec
        .cpu(move || {
            format::write(
                &s,
                files.as_deref(),
                settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await?;
    let watch = gz_project::Project::of(editor).ok().map(|(_, p)| p.tree.changes().clone());
    let f = file.clone();
    let out = plan.files.clone();
    let written = editor.exec.io(move || commit(&f, &out, watch.as_ref())).await?;
    let mut after = plan.after;
    after.base.meta.file = Some(file.clone());
    after.base.meta.imported = None;
    *svc.files.lock() = Some(Arc::new(after));
    svc.store.send(SceneCmd::Saved { file: file.clone() }, Origin::Mcp).await.map_err(|e| e.message)?;
    svc.set_opened();
    let rec = svc.recovery.clone();
    editor.exec.io(move || rec.reset()).await;
    note_scene(editor, svc, &file);
    follow(editor, svc);
    // what the scene still takes from the editor's library goes on the project file's list
    let snap = svc.store.snapshot();
    let refs: Vec<String> = snap
        .objects
        .values()
        .filter_map(|o| match &o.data {
            ObjData::SceneObject(d) if !d.visual_mode() => Some(d.reference.to_string()),
            _ => None,
        })
        .collect();
    if let Err(e) = gz_project::library::note_scene_refs(editor, refs).await {
        editor.log.warn(format!("the project file's library list was not updated: {}", e.message));
    }
    Ok(written)
}

// ---------------------------------------------------------------- following the files

/// Follows the open scene's files: a change another program made comes into the open scene.
fn follow(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let generation = svc.watch_gen.fetch_add(1, Ordering::Relaxed) + 1;
    let Some(file) = svc.store.snapshot().meta.file.clone() else { return };
    let Ok((_, project)) = gz_project::Project::of(editor) else { return };
    let (Some(main), Some(dir)) =
        (RelPath::from_fs(&svc.root, &file), RelPath::from_fs(&svc.root, &format::folder_of(&file)))
    else {
        return;
    };
    let under = main.parent().unwrap_or_else(RelPath::root);
    let mut rx = project.tree.changes().subscribe(Filter::Under(under));
    let weak = Arc::downgrade(svc);
    let ed = editor.clone();
    editor.exec.spawn(async move {
        while let Some(change) = rx.recv().await {
            let Some(svc) = weak.upgrade() else { break };
            if svc.watch_gen.load(Ordering::Relaxed) != generation {
                break;
            }
            let ours = change.path == main
                || change.path.starts_with(&dir) && change.path != dir
                || change.kind == gz_project::ChangeKind::Rescan;
            if !ours {
                continue;
            }
            // a burst of files (another program saving the whole scene) is one change
            tokio::time::sleep(Duration::from_millis(150)).await;
            while rx.try_recv().is_ok() {}
            if let Err(e) = outside_change(&ed, &svc, &file).await {
                ed.log.warn(format!("the scene's files changed outside the editor: {e}"));
            }
        }
    });
}

/// What another program wrote comes in: a clean scene takes its files again as one undo step
/// (Reload from Disk); a scene with unsaved changes keeps them over what the files say now
/// (each file's difference rebased), and a place both changed keeps the editor's value and is
/// named in the scene's problems. A file that does not read leaves the scene as it is and says
/// where.
pub(crate) async fn outside_change(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    file: &Path,
) -> Result<(), String> {
    let settings = crate::game::settings(ed).await;
    let (root, f) = (svc.root.clone(), file.to_path_buf());
    let st = settings.clone();
    let opened = ed
        .exec
        .io(move || open_files(&root, &f, st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync))))
        .await;
    let opened = match opened {
        Ok(o) => o,
        Err(e) => {
            let p = Problem::error("syntax", e.clone()).in_file(rel_of(&svc.root, file));
            svc.problems.lock().push(p);
            return Err(e);
        }
    };
    let Some(theirs) = opened.files else { return Ok(()) };
    let snap = svc.store.snapshot();
    if !svc.store.is_dirty() {
        svc.prefetch_library(ed, &opened.state).await;
        svc.store
            .send(
                SceneCmd::Reload {
                    state: Box::new(opened.state),
                    label: "Reload from Disk".into(),
                    clean: true,
                },
                Origin::Mcp,
            )
            .await
            .map_err(|e| e.message)?;
        *svc.files.lock() = Some(Arc::new(theirs));
        *svc.problems.lock() = opened.problems;
        ed.log
            .info("the scene was changed outside the editor: it is read again (Undo takes the change back)");
        return Ok(());
    }
    // unsaved changes: this side's difference applied over each file's new text
    let mine = svc.files();
    let Some(mine) = mine else { return Ok(()) };
    let (merged, conflicts) = ed
        .exec
        .cpu({
            let snap = snap.clone();
            move || {
                rebase(
                    &snap,
                    &mine,
                    &theirs,
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                )
            }
        })
        .await?;
    let mut problems = opened.problems;
    problems.extend(conflicts);
    svc.store
        .send(
            SceneCmd::Reload {
                state: Box::new(merged.0),
                label: "Merge Changes from Disk".into(),
                clean: false,
            },
            Origin::Mcp,
        )
        .await
        .map_err(|e| e.message)?;
    *svc.files.lock() = Some(Arc::new(merged.1));
    *svc.problems.lock() = problems;
    ed.log.info("the scene's files were changed outside the editor: the unsaved changes are kept over them");
    Ok(())
}

/// The open scene with its unsaved changes over the files as they are now: the merged state, the
/// documents to save against (the files as they are now) and a problem per place both changed.
fn rebase(
    now: &SceneState,
    mine: &SceneFiles,
    theirs: &SceneFiles,
    settings: sdk::SyncSettings<'_>,
) -> Result<((SceneState, SceneFiles), Vec<Problem>), String> {
    // what this side changed since it read the files, written as a save would write it, then
    // rebased over the new texts
    let plan = format::write(now, Some(mine), settings)?;
    let mut problems = Vec::new();
    let mut texts = Texts { scene: theirs.scene.text().to_owned(), ..Texts::default() };
    let changed: Vec<&str> = plan.files.iter().map(|f| f.file.as_str()).collect();
    // a file this side changed: its difference over the new text; a file it did not: the new text
    let rebase_one = |file: &str,
                      base: &gz_luadata::LuaDoc,
                      new_text: &str,
                      rules: &gz_luadata::DiffRules|
     -> Result<(String, Vec<gz_luadata::Conflict>), String> {
        let ours = plan.after.docs.iter().find(|d| d.file == file).map(|d| d.doc.value());
        let Some(ours) = ours else { return Ok((new_text.to_owned(), Vec::new())) };
        let (doc, conflicts) = gz_luadata::LuaDoc::rebase_with(
            &base.value(),
            &ours,
            new_text,
            rules,
            file,
            gz_luadata::Limits::scene(),
        )
        .map_err(|e| format!("{file}: {}", e.message))?;
        Ok((doc.text().to_owned(), conflicts))
    };
    if changed.contains(&"") {
        let (doc, conflicts) = gz_luadata::LuaDoc::rebase_with(
            &mine.scene.value(),
            &plan.after.scene.value(),
            theirs.scene.text(),
            &format::scene_rules(),
            format::HEADER,
            gz_luadata::Limits::scene(),
        )
        .map_err(|e| e.message)?;
        texts.scene = doc.text().to_owned();
        for c in conflicts {
            problems.push(conflict_problem(&c, "the .scene"));
        }
    }
    for cf in &format::CLASS_FILES {
        let theirs_doc = theirs.docs.iter().find(|d| d.file == cf.file);
        let mine_doc = mine.docs.iter().find(|d| d.file == cf.file);
        let text = match (theirs_doc, mine_doc, changed.contains(&cf.file)) {
            (Some(t), Some(m), true) => {
                let (text, conflicts) = rebase_one(cf.file, &m.doc, t.doc.text(), &format::class_rules(cf))?;
                for c in conflicts {
                    problems.push(conflict_problem(&c, cf.file));
                }
                Some(text)
            }
            (Some(t), _, _) => Some(t.doc.text().to_owned()),
            (None, _, true) => {
                plan.after.docs.iter().find(|d| d.file == cf.file).map(|d| d.doc.text().to_owned())
            }
            (None, _, false) => None,
        };
        if let Some(text) = text {
            texts.files.push((cf.file.to_owned(), text));
        }
    }
    for l in format::layers::LAYERS {
        texts.layers.push((l.file.to_owned(), format::layers::write(l, &now.bulk)));
    }
    let main = RelPath::new("scene.scene").unwrap_or_else(|_| RelPath::root());
    let dir = RelPath::root();
    let merged = format::read(&texts, &main, &dir, settings).map_err(|r| r.to_string())?;
    let mut state = merged.state;
    state.meta = now.meta.clone();
    let mut files = theirs.clone();
    files.base.meta = now.meta.clone();
    Ok(((state, files), problems))
}

fn conflict_problem(c: &gz_luadata::Conflict, file: &str) -> Problem {
    let what = gz_luadata::path::show(&c.path);
    let p = Problem::warning(
        "S050",
        format!(
            "{file}: '{what}' was changed both here and in the file: the editor's value is kept (save to write it)"
        ),
    );
    match c.span {
        Some(s) => p.at(s.line, s.column, s.end_line, s.end_column),
        None => p,
    }
}
