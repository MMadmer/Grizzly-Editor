//! What the scene area keeps while a project is open: the store, the documents the scene was read
//! from, the library, the pick tree, the recovery session, the load status, the problems its
//! files hold and the property panels' view; loading, saving and following the scene's files.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use std::time::Duration;

use gz_base::{Origin, Problem, RelPath, Rev, SmolStr};
use gz_editor::{CmdError, DirtyDoc, DirtySource, Editor, JobError, ProjectSession};
use gz_project::Filter;
use gz_query::Derived;
use gz_store::{Store, StoreOptions};
use parking_lot::Mutex;

use crate::apply::{ReplaceKind, SceneCmd};
use crate::format::{self, SceneFiles, Texts};
use crate::library::Library;
use crate::model::{ObjData, SceneState};
use crate::objects::radio::{RadioCx, Radios};
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
    /// What the scene's terrains made in the library: their meshes and the ground they cut.
    pub terrains: Arc<crate::objects::terrain::Terrains>,
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
    /// What the linked game says a radio is, with the configs it was read from (their address).
    radios: Mutex<(usize, Arc<Radios>)>,
    /// The radio rows' context as last made: the radios and the manifest's generation it is of.
    radio_cx: Mutex<Option<(usize, Rev, Arc<RadioCx>)>>,
    /// The documents the open scene was read from or last saved to: what a save writes the
    /// difference into. `None` for a scene not in its own files yet (new, imported).
    files: Mutex<Option<Arc<SceneFiles>>>,
    /// One reader or writer of the scene's files at a time: a save and the taking in of what
    /// another program wrote never cross, so a reading made before a save is never put in after it.
    files_turn: Arc<tokio::sync::Mutex<()>>,
    /// The scene's files as they stood (size and write time) when they were last told against
    /// the documents: while they stand so, nothing is read to know they did not change.
    seen: Mutex<Option<Vec<(u64, u64)>>>,
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
        let library = Arc::new(Library::new(vec![
            root.join("rawdata").join("objects"),
            editor_root.join("rawdata").join("objects"),
        ]));
        // the models that are no files of the library: every crate that makes some adds them
        crate::library::ProviderMakers::of(editor).provide(editor, &root, &library);
        let svc = session.services.get_or_init(|| SceneService {
            store: Store::spawn("scene", SceneState::default(), &editor.exec, StoreOptions::default()),
            library,
            terrains: Arc::default(),
            recovery: Arc::new(Recovery::new()),
            pick: Derived::new(editor.exec.clone()),
            load: Mutex::new(LoadStatus::default()),
            view: Mutex::new(InspectorView::default()),
            root,
            editor_root,
            prop_cx: parking_lot::RwLock::new(Arc::new(gz_entities::PropContext::default())),
            radios: Mutex::new((0, Arc::new(Radios::unlinked()))),
            radio_cx: Mutex::new(None),
            files: Mutex::new(None),
            files_turn: Arc::default(),
            seen: Mutex::new(None),
            problems: Mutex::new(Vec::new()),
            opened: AtomicBool::new(false),
            watch_gen: AtomicU64::new(0),
        });
        start_recovery_timer(editor, &svc);
        learn_surface_uses(editor, &svc);
        follow_library(editor, &svc);
        follow_terrains(editor, &svc);
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

    /// What the radio rows, the audit and the bake read: the radios of the game as last read and
    /// the module's playlists as the manifest holds them now. Never waits: what a frame asks.
    #[must_use]
    pub fn radio_cx_now(&self, session: &ProjectSession) -> Arc<RadioCx> {
        let (rkey, radios) = {
            let r = self.radios.lock();
            (Arc::as_ptr(&r.1) as usize, r.1.clone())
        };
        let (mrev, module, playlists) = gz_mod::ManifestService::get(session)
            .map_or_else(|| (Rev::ZERO, String::new(), Arc::from(Vec::new())), |m| m.playlists());
        let mut c = self.radio_cx.lock();
        if let Some((k, r, cx)) = &*c
            && *k == rkey
            && *r == mrev
        {
            return cx.clone();
        }
        let cx = Arc::new(RadioCx { radios, module, playlists });
        *c = Some((rkey, mrev, cx.clone()));
        cx
    }

    /// The radio rows' context as last made (by [`crate::game::radio_cx`], or a frame).
    #[must_use]
    pub fn radio_cx_cached(&self) -> Arc<RadioCx> {
        self.radio_cx.lock().as_ref().map_or_else(|| Arc::new(RadioCx::unlinked()), |c| c.2.clone())
    }

    /// Reads the game's radios again when the configs are others than those last read.
    pub(crate) async fn refresh_radios(
        &self,
        editor: &Editor,
        settings: Option<Arc<crate::game::GameSettings>>,
    ) {
        let Some(g) = settings else { return };
        let key = std::ptr::from_ref(g.ltx()) as usize;
        if self.radios.lock().0 == key {
            return;
        }
        let read = editor.exec.io(move || Radios::read(g)).await;
        *self.radios.lock() = (key, Arc::new(read));
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

    /// The turn at the scene's files and at which scene is open: whoever reads the files into the
    /// scene, writes them or puts another scene in holds it for all of that, so no reading of one
    /// scene's files is put into another.
    pub(crate) async fn turn(&self) -> tokio::sync::OwnedMutexGuard<()> {
        self.files_turn.clone().lock_owned().await
    }

    /// The documents the open scene was read from.
    #[must_use]
    pub fn files(&self) -> Option<Arc<SceneFiles>> {
        self.files.lock().clone()
    }

    /// The scene is saved against `files` from now on: the documents on the disk as they were
    /// just read, with the problems that reading found.
    pub(crate) fn attach_files(&self, files: SceneFiles, problems: Vec<Problem>, stamp: Vec<(u64, u64)>) {
        *self.files.lock() = Some(Arc::new(files));
        *self.problems.lock() = problems;
        self.looked(stamp);
    }

    /// The scene's files were read as they stood at `stamp`: until they stand otherwise nothing
    /// is read to know they hold what the documents hold.
    fn looked(&self, stamp: Vec<(u64, u64)>) {
        *self.seen.lock() = Some(stamp);
    }

    /// A save wrote the painted layers: as they stand in `stamp` they are the scene's own. The
    /// text files keep what was seen of them - they are told against their documents.
    fn layers_looked(&self, stamp: &[(u64, u64)]) {
        let first = 1 + format::CLASS_FILES.len();
        if let Some(seen) = self.seen.lock().as_mut()
            && seen.len() == stamp.len()
            && let (Some(mine), Some(now)) = (seen.get_mut(first..), stamp.get(first..))
        {
            mine.copy_from_slice(now);
        }
    }

    /// The scene is no longer the one its files hold (a new scene).
    pub(crate) fn forget_files(&self) {
        *self.files.lock() = None;
        *self.seen.lock() = None;
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
        prefetch_into(editor, self.library.clone(), state, Some(self.terrains.clone())).await;
    }

    /// The library a long job reads: every model the scene names loaded, in a library of the
    /// job's own ([`Library::fork`]), so a file the watcher reports while the job runs takes no
    /// model from under it.
    pub async fn library_for(&self, editor: &Editor, state: &SceneState) -> Arc<Library> {
        self.prefetch_library(editor, state).await;
        let own = Arc::new(self.library.fork());
        // its terrains and the ground they cut are made from this state and from itself: the
        // open scene may be edited and its library emptied while the job runs
        own.add_scene_provider(crate::objects::terrain::Terrains::provider_over(
            Arc::new(state.clone()),
            Arc::downgrade(&own),
        ));
        // what was forgotten between the load and the fork is read into the job's own
        prefetch_into(editor, own.clone(), state, None).await;
        own
    }
}

/// Loads into `lib` every model `state` names - the scene objects', the detail objects', the
/// terrains' and the cut ground's - on the IO pool. With `settle` (the open scene's library) a
/// terrain or a cut model made for an earlier state of the scene is made again first: whoever
/// asks reads the scene as it is now, not as it was until the terrains' own follow-up caught up.
async fn prefetch_into(
    editor: &Editor,
    lib: Arc<Library>,
    state: &SceneState,
    settle: Option<Arc<crate::objects::terrain::Terrains>>,
) {
    let mut refs: Vec<SmolStr> = state
        .objects
        .values()
        .filter_map(|o| match &o.data {
            ObjData::SceneObject(d) if !d.visual_mode() => Some(d.reference.clone()),
            // what a spawn point's entity wears stands in the scene like any model
            ObjData::Spawn(_) => crate::objects::visual_ref(o).map(SmolStr::new),
            _ => None,
        })
        .collect();
    refs.sort_unstable();
    refs.dedup();
    if let Some(d) = state.bulk.details.as_ref() {
        refs.extend(d.model.indices.objects.iter().map(|o| o.reference.clone()));
    }
    let terrains = state.count(crate::class::ObjClass::Terrain) > 0;
    if refs.is_empty() && !terrains {
        return;
    }
    let exec = editor.exec.clone();
    let state = terrains.then(|| state.clone());
    editor
        .exec
        .io(move || {
            exec.io_pool().install(|| {
                lib.prefetch(&refs);
                // the terrains' meshes and the cut ground, once their ground is loaded
                if let Some(s) = state {
                    if let Some(made) = &settle {
                        made.refresh(&s, &lib);
                    }
                    lib.prefetch(&crate::objects::terrain::references(&s, &lib));
                }
            })
        })
        .await;
}

/// The project's library folder as the watcher names it.
const LIBRARY_FOLDER: &str = "rawdata/objects";

/// The library reference a changed project file is, when it is a library object
/// (`rawdata/objects/props/box.object` is `props\box`).
#[must_use]
pub fn library_reference(path: &RelPath) -> Option<String> {
    let p = path.as_str();
    let head = p.get(..LIBRARY_FOLDER.len() + 1)?;
    if !head.eq_ignore_ascii_case(&format!("{LIBRARY_FOLDER}/")) {
        return None;
    }
    let rest = &p[head.len()..];
    let ext = rest.rsplit_once('.').map(|(_, e)| e)?;
    ext.eq_ignore_ascii_case("object").then(|| crate::library::normalize(rest))
}

/// The library hears the project's watcher: a library object changed on the disk is read again
/// the next time it is used.
/// A surface's use as the compiler shaders tell it: `flags` by shader name, lower case.
/// A helper object or the occluder material is neither drawn nor solid whatever its shader
/// says; a shader the list lacks is taken for one that does both.
pub fn use_rule(flags: rustc_hash::FxHashMap<String, u32>) -> crate::library::UseRule {
    use crate::library::{SurfaceUse, default_use};
    use gz_fmt_xr::ShaderXrlc;
    Arc::new(move |object, surface| {
        let base = default_use(object, surface);
        if base == SurfaceUse::NEITHER {
            return base;
        }
        match flags.get(&surface.shader_xrlc.to_ascii_lowercase()) {
            Some(f) => {
                SurfaceUse { draws: f & ShaderXrlc::RENDERING != 0, collides: f & ShaderXrlc::COLLISION != 0 }
            }
            None => base,
        }
    })
}

/// Reads the compiler shaders (the linked game's, else the editor's own) and gives the library
/// the rule they make: until it arrives every surface is taken for drawn and solid.
fn learn_surface_uses(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let lib = Arc::downgrade(&svc.library);
    let ed = editor.clone();
    editor.exec.spawn(async move {
        let Ok((bytes, _)) = gz_game::read_game_or_bundled(&ed, "shaders_xrlc.xr").await else { return };
        let Ok(list) = gz_fmt_xr::ShadersXrlc::read(&bytes) else { return };
        let flags = list.shaders.iter().map(|s| (s.name().to_ascii_lowercase(), s.flags)).collect();
        if let Some(lib) = lib.upgrade() {
            lib.set_use_rule(use_rule(flags));
        }
    });
}

fn follow_library(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let Ok((_, project)) = gz_project::Project::of(editor) else { return };
    let Ok(under) = RelPath::new(LIBRARY_FOLDER) else { return };
    let mut rx = project.tree.changes().subscribe(Filter::Under(under));
    let lib = Arc::downgrade(&svc.library);
    let scene = Arc::downgrade(svc);
    let exec = editor.exec.clone();
    editor.exec.spawn(async move {
        while let Some(change) = rx.recv().await {
            let Some(lib) = lib.upgrade() else { break };
            if change.kind == gz_project::ChangeKind::Rescan {
                // nobody says what changed: only what is another file now is read again, so a
                // job reading the cache keeps every model that is still there
                let l = lib.clone();
                exec.io(move || l.revalidate()).await;
            } else if let Some(r) = library_reference(&change.path) {
                lib.forget(&r);
            }
            // the ground a terrain samples or cuts may be what changed
            if let Some(svc) = scene.upgrade() {
                refresh_terrains(&exec, &svc).await;
            }
        }
    });
}

/// The terrains' models follow the scene: a terrain, the ground under it or an object it cuts
/// that changed has its model made again (worked out on the CPU pool after every change of the
/// scene; a library object changed on the disk asks the same in [`follow_library`]).
fn follow_terrains(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let mut rx = svc.store.subscribe();
    let weak = Arc::downgrade(svc);
    svc.library.add_scene_provider(svc.terrains.provider(weak.clone()));
    let exec = editor.exec.clone();
    editor.exec.spawn(async move {
        while rx.changed().await.is_ok() {
            let Some(svc) = weak.upgrade() else { break };
            refresh_terrains(&exec, &svc).await;
        }
    });
}

/// Forgets the terrains' models that no longer match the scene.
async fn refresh_terrains(exec: &gz_task::Executors, svc: &SceneService) {
    let snap = svc.store.snapshot();
    if snap.count(crate::class::ObjClass::Terrain) + svc.terrains.made() == 0 {
        return;
    }
    let (lib, terrains) = (svc.library.clone(), svc.terrains.clone());
    exec.cpu(move || terrains.refresh(&snap, &lib)).await;
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
    // a file that does not read now: its objects as the scene holds them, written whole, so a
    // restore has them (the file on the disk is the author's and stays as it is)
    if !plan.kept.is_empty() {
        let whole = format::write(state, None, settings)?;
        for f in whole.files {
            if let (true, Some(bytes)) = (plan.kept.contains(&f.file.as_str()), f.bytes) {
                out.push((format!("{name}/{}", f.file), bytes));
            }
        }
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
                Ok(()) => ed.log.info("recovery: snapshot saved"),
                Err(e) => {
                    svc.recovery.retry_later();
                    ed.log.warn(format!("recovery: autosave failed: {e}"));
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
    /// Groups placed from their assets as the scene was read: the scene is unsaved.
    pub placed: usize,
    /// How the scene's files stood just before they were read (`files_stamp`).
    pub stamp: Vec<(u64, u64)>,
}

/// Why a scene did not open: the sentence, and the positioned problem when its file says where.
pub(crate) struct OpenFailed {
    pub message: String,
    pub problem: Option<Box<Problem>>,
}

impl From<String> for OpenFailed {
    fn from(message: String) -> Self {
        Self { message, problem: None }
    }
}

/// Reads a scene's files into the model (blocking: run on the IO / CPU pools).
pub(crate) fn open_files(
    root: &Path,
    file: &Path,
    settings: sdk::SyncSettings<'_>,
    levels: Option<&dyn crate::base::BaseSource>,
) -> Result<Opened, String> {
    open_scene(root, None, file, settings, levels, None).map_err(|e| e.message)
}

/// [`open_files`] with the editor's library (`editor_root`) for the groups placed at the read, the
/// levels of the game a scene may lie over (`levels`), and what the open scene keeps of a file
/// that does not parse (`keep`).
pub(crate) fn open_scene(
    root: &Path,
    editor_root: Option<&Path>,
    file: &Path,
    settings: sdk::SyncSettings<'_>,
    levels: Option<&dyn crate::base::BaseSource>,
    keep: Option<&SceneState>,
) -> Result<Opened, OpenFailed> {
    if is_sdk(file) {
        let main =
            std::fs::read(file).map_err(|e| format!("Cannot open scene file: {} ({e})", file.display()))?;
        let kind = sdk::scene_kind(&main)?;
        let parts = sdk::read_parts(file, &sdk::parts_needed(&main, kind));
        let loaded = sdk::parse_scene(&main, file, &parts, settings).map_err(|e| e.to_string())?;
        let rel = rel_of(root, file);
        let problems =
            loaded.warnings.into_iter().map(|w| Problem::warning("S040", w).in_file(rel.clone())).collect();
        return Ok(Opened { state: loaded.state, files: None, problems, placed: 0, stamp: Vec::new() });
    }
    // before the texts: a write that lands between the two is found by the next look
    let stamp = files_stamp(file);
    let texts = read_texts(file).map_err(|e| format!("Cannot open scene file: {} ({e})", file.display()))?;
    let main = rel_of(root, file);
    let dir = rel_of(root, &format::folder_of(file));
    let keep = keep.map(|state| format::Keep { state, files: &[] });
    let mut read = format::read_based(&texts, &main, &dir, settings, keep, levels)
        .map_err(|r| OpenFailed { message: r.to_string(), problem: Some(r.problem) })?;
    read.state.meta.file = Some(file.to_path_buf());
    read.files.base.meta.file = Some(file.to_path_buf());
    // the base stays the files as written: what was placed is unsaved and the next save writes it
    let (notes, placed) =
        crate::edit::place_unplaced(&mut read.state, &read.unplaced, root, editor_root, settings);
    read.problems.extend(notes);
    Ok(Opened { state: read.state, files: Some(read.files), problems: read.problems, placed, stamp })
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
        // from the reading to the documents nothing else reads or writes a scene's files
        let _turn = s.turn().await;
        let (root, lib, pf) = (s.root.clone(), s.editor_root.clone(), f.clone());
        let levels = crate::base::BaseLevels::of(&ed).get();
        let opened = ctx
            .exec
            .io(move || {
                let st: sdk::SyncSettings<'_> =
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                open_scene(&root, Some(&lib), &pf, st, levels.as_deref(), None)
            })
            .await
            .map_err(|e| JobError::new(e.message))?;
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
        if opened.placed > 0 {
            s.store.send(SceneCmd::Touch, Origin::Mcp).await.map_err(|e| JobError::new(e.message))?;
        }
        *s.files.lock() = opened.files.map(Arc::new);
        *s.problems.lock() = opened.problems;
        s.looked(opened.stamp);
        s.set_opened();
        if let Ok((_, project)) = gz_project::Project::of(&ed)
            && let Err(e) = project.note_recent_scene(&f).await
        {
            ed.log.warn(e.message);
        }
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

/// What another program wrote into the scene's files since they were last read comes in before
/// a command answers: whoever writes a file and asks about the scene at once is answered about
/// what it wrote, not about the scene of a moment ago. The watcher brings the same change a
/// little later and finds it in already.
async fn catch_up(editor: &Arc<Editor>, svc: &Arc<SceneService>) {
    let Some(file) = svc.store.snapshot().meta.file.clone() else { return };
    if svc.files().is_none() {
        return;
    }
    if let Err(e) = outside_change(editor, svc, &file).await {
        editor.log.warn(format!("the scene's files changed outside the editor: {e}"));
    }
}

/// The size and the write time of every file of a scene, in a fixed order - the `.scene`, the
/// files of the kinds, the painted layers; a file that is not there counts as nothing.
fn files_stamp(scene: &Path) -> Vec<(u64, u64)> {
    let dir = format::folder_of(scene);
    let of = |p: &Path| {
        std::fs::metadata(p).map_or((0, 0), |m| {
            let time = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map_or(0, |d| d.as_nanos() as u64);
            (m.len(), time)
        })
    };
    std::iter::once(of(scene))
        .chain(format::CLASS_FILES.iter().map(|cf| of(&dir.join(cf.file))))
        .chain(format::layers::LAYERS.iter().map(|l| of(&dir.join(l.file))))
        .collect()
}

/// The scene's files as they stand now when they hold another text than the documents the scene
/// is saved against; `None` when they hold the same. Blocking. While the files stand as they
/// stood at the last look nothing is read.
fn files_differ(svc: &SceneService, scene: &Path) -> Option<Vec<(u64, u64)>> {
    let held = svc.files()?;
    // taken before the texts: a write that lands between the two is looked at next time
    let stamp = files_stamp(scene);
    let seen = svc.seen.lock().clone();
    if seen.as_ref() == Some(&stamp) {
        return None;
    }
    // a `.scene` that is gone or cannot be read is a change too: whoever takes it in says why
    let Ok(now) = read_texts(scene) else { return Some(stamp) };
    let differ = now.scene != held.scene.text()
        || format::CLASS_FILES.iter().enumerate().any(|(i, cf)| {
            if held.broken.contains(&cf.file) {
                // no text of a file that did not read is held: it changed when the file itself
                // was written since the last look (the first entry of a stamp is the `.scene`)
                return seen.as_ref().and_then(|s| s.get(i + 1)) != stamp.get(i + 1);
            }
            let disk = now.files.iter().find(|(n, _)| n == cf.file).map(|(_, t)| t.as_str());
            let read = held.docs.iter().find(|d| d.file == cf.file).map(|d| d.doc.text());
            disk != read
        })
        // a painted layer is no text to tell against a document: it changed when its file was
        // written since the last look (a save of the editor's own says it looked, `layers_looked`)
        || seen.as_ref().is_some_and(|seen| {
            let first = 1 + format::CLASS_FILES.len();
            (first..first + format::layers::LAYERS.len()).any(|i| seen.get(i) != stamp.get(i))
        });
    if differ {
        return Some(stamp);
    }
    *svc.seen.lock() = Some(stamp);
    None
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
        catch_up(editor, &svc).await;
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
    // the files are this save's until it has written them and holds their documents
    let _turn = svc.turn().await;
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
    for f in &plan.kept {
        editor.log.warn(format!(
            "the scene's {f} does not read: the save leaves it as it is, and what the scene changed of its objects is not written"
        ));
    }
    let watch = gz_project::Project::of(editor).ok().map(|(_, p)| p.tree.changes().clone());
    let f = file.clone();
    let out = plan.files.clone();
    let (written, stamp) = editor
        .exec
        .io(move || commit(&f, &out, watch.as_ref()).map(|written| (written, files_stamp(&f))))
        .await?;
    svc.layers_looked(&stamp);
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
    // one reader or writer of the files at a time: what is read here is put in before anything
    // else writes or reads them
    let _turn = svc.turn().await;
    // the scene may have been switched while this waited for its turn
    if svc.store.snapshot().meta.file.as_deref() != Some(file) {
        return Ok(());
    }
    // a change that came in already (the watcher after a command took it in, a save of the
    // editor's own) is none
    let (s, f) = (svc.clone(), file.to_path_buf());
    let Some(stamp) = ed.exec.io(move || files_differ(&s, &f)).await else { return Ok(()) };
    let settings = crate::game::settings(ed).await;
    let (root, lib, f) = (svc.root.clone(), svc.editor_root.clone(), file.to_path_buf());
    let st = settings.clone();
    let snap = svc.store.snapshot();
    let last = snap.clone();
    let levels = crate::base::BaseLevels::of(ed).get();
    let opened = ed
        .exec
        .io(move || {
            open_scene(
                &root,
                Some(&lib),
                &f,
                st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                levels.as_deref(),
                Some(&last),
            )
        })
        .await;
    let opened = match opened {
        Ok(o) => o,
        Err(e) => {
            // the scene stays as it is; the problem replaces the one its file had before
            let p = e.problem.map_or_else(
                || Problem::error("syntax", e.message.clone()).in_file(rel_of(&svc.root, file)),
                |p| *p,
            );
            let mut problems = svc.problems.lock();
            problems.retain(|q| !(q.code == p.code && q.file == p.file));
            problems.push(p);
            // looked at as they stood: a scene that does not read is not read again on every
            // command until somebody writes it
            svc.looked(stamp);
            return Err(e.message);
        }
    };
    let Some(theirs) = opened.files else {
        svc.looked(opened.stamp);
        return Ok(());
    };
    // a file that did not read and reads again: what the scene changed of its objects meanwhile
    // is unsaved work like any other, whatever a save in between marked
    let held = match svc.files() {
        Some(mine) if mine.broken.iter().any(|f| !theirs.broken.contains(f)) => {
            let (s, st, again) = (snap.clone(), settings.clone(), theirs.broken.clone());
            ed.exec
                .cpu(move || {
                    let st = st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                    mine.broken
                        .iter()
                        .any(|f| !again.contains(f) && matches!(mine.pending(&s, f, st), Ok(Some(_))))
                })
                .await
        }
        _ => false,
    };
    if !svc.store.is_dirty() && !held {
        svc.prefetch_library(ed, &opened.state).await;
        svc.store
            .send(
                SceneCmd::Reload {
                    state: Box::new(opened.state),
                    label: "Reload from Disk".into(),
                    clean: opened.placed == 0,
                    of: file.to_path_buf(),
                },
                Origin::Mcp,
            )
            .await
            .map_err(|e| e.message)?;
        let mut theirs = theirs;
        theirs.hold_broken(svc.files().as_deref());
        *svc.files.lock() = Some(Arc::new(theirs));
        *svc.problems.lock() = opened.problems;
        svc.looked(opened.stamp);
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
                of: file.to_path_buf(),
            },
            Origin::Mcp,
        )
        .await
        .map_err(|e| e.message)?;
    *svc.files.lock() = Some(Arc::new(merged.1));
    *svc.problems.lock() = problems;
    svc.looked(opened.stamp);
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
        // a file that does not read keeps what this side has of it
        if theirs.broken.contains(&cf.file) {
            continue;
        }
        let theirs_doc = theirs.docs.iter().find(|d| d.file == cf.file);
        let mine_doc = mine.docs.iter().find(|d| d.file == cf.file);
        // it did not read on this side and reads again: what the scene changed of its objects
        // meanwhile goes over the new text, like any other unsaved change
        if let (Some(t), true) = (theirs_doc, mine.broken.contains(&cf.file)) {
            let text = match mine.pending(now, cf.file, settings)? {
                Some((base, ours)) => {
                    let (doc, conflicts) = gz_luadata::LuaDoc::rebase_with(
                        &base,
                        &ours,
                        t.doc.text(),
                        &format::class_rules(cf),
                        cf.file,
                        gz_luadata::Limits::scene(),
                    )
                    .map_err(|e| format!("{}: {}", cf.file, e.message))?;
                    for c in conflicts {
                        problems.push(conflict_problem(&c, cf.file));
                    }
                    doc.text().to_owned()
                }
                None => t.doc.text().to_owned(),
            };
            texts.files.push((cf.file.to_owned(), text));
            continue;
        }
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
    let keep = format::Keep { state: now, files: &theirs.broken };
    // the level under the scene is the one the files lie over now, as they were just read
    let level = &theirs.base.meta.base;
    let merged = format::read_based(&texts, &main, &dir, settings, Some(keep), Some(level))
        .map_err(|r| r.to_string())?;
    let mut state = merged.state;
    let base = state.meta.base.clone();
    state.meta = now.meta.clone();
    state.meta.base = base;
    let mut files = theirs.clone();
    files.base.meta = now.meta.clone();
    files.hold_broken(Some(mine));
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::class::ObjClass;
    use crate::math::Vec3;
    use crate::model::Object;
    use crate::objects::{LightData, SceneObjectData};

    fn disk() -> Texts {
        let mut s = SceneState { valid: true, ..SceneState::default() };
        for (name, x) in [("Box_01", 1.0), ("Box_02", 2.0)] {
            let mut o = Object::new(
                ObjClass::SceneObject,
                name,
                ObjData::SceneObject(SceneObjectData::library(r"props\box")),
            );
            o.xform.position = Vec3::new(x, 0.0, 0.0);
            s.insert(o);
        }
        s.insert(Object::new(ObjClass::Light, "Lamp_01", ObjData::Light(LightData::default())));
        let plan = format::write(&s, None, None).unwrap();
        let mut t = Texts::default();
        for o in plan.files {
            let Some(b) = o.bytes else { continue };
            let text = String::from_utf8(b).unwrap();
            if o.file.is_empty() {
                t.scene = text;
            } else if o.file.ends_with(".lua") {
                t.files.push((o.file, text));
            }
        }
        t
    }

    fn read(t: &Texts, keep: Option<format::Keep<'_>>) -> format::Read {
        let main = RelPath::new("test.scene").unwrap();
        let dir = RelPath::new("test").unwrap();
        format::read_over(t, &main, &dir, None, keep).unwrap()
    }

    fn boxes(s: &SceneState) -> usize {
        s.of_class(ObjClass::SceneObject).count()
    }

    #[test]
    fn unsaved_changes_over_a_part_file_that_does_not_read_lose_nothing() {
        let good = disk();
        let mine = read(&good, None);
        // an unsaved change of the open scene
        let mut now = mine.state.clone();
        let mut l = (**now.find(ObjClass::Light, "Lamp_01").unwrap()).clone();
        l.xform.position.y = 5.0;
        now.replace(l);
        // another program breaks objects.lua
        let mut broken = good.clone();
        for (f, t) in &mut broken.files {
            if f == "objects.lua" {
                t.push_str("\n}} oops\n");
            }
        }
        let theirs = read(&broken, Some(format::Keep { state: &now, files: &[] }));
        let ((merged, files), _) = rebase(&now, &mine.files, &theirs.files, None).unwrap();
        assert_eq!(boxes(&merged), 2, "the broken file's objects stay");
        assert_eq!(merged.find(ObjClass::Light, "Lamp_01").unwrap().xform.position.y, 5.0);
        assert_eq!(files.broken, ["objects.lua"]);
        let plan = format::write(&merged, Some(&files), None).unwrap();
        assert!(plan.files.iter().all(|f| f.file != "objects.lua"), "a save leaves the broken file");
        // fixed with one box moved: the file is read again
        let mut fixed = good.clone();
        for (f, t) in &mut fixed.files {
            if f == "objects.lua" {
                *t = t.replacen("pos = { 2, 0, 0 }", "pos = { 7, 0, 0 }", 1);
                assert!(t.contains("7, 0, 0"), "{t}");
            }
        }
        let theirs = read(&fixed, Some(format::Keep { state: &merged, files: &[] }));
        let ((back, files), _) = rebase(&merged, &files, &theirs.files, None).unwrap();
        assert!(files.broken.is_empty());
        assert_eq!(back.find(ObjClass::SceneObject, "Box_02").unwrap().xform.position.x, 7.0);
        assert_eq!(back.find(ObjClass::Light, "Lamp_01").unwrap().xform.position.y, 5.0);
    }

    /// `objects.lua` with a brace too many.
    fn broken_of(good: &Texts) -> Texts {
        let mut broken = good.clone();
        for (f, t) in &mut broken.files {
            if f == "objects.lua" {
                t.push_str("\n}} oops\n");
            }
        }
        broken
    }

    fn box_x(s: &SceneState, name: &str) -> f32 {
        s.find(ObjClass::SceneObject, name).map_or(f32::NAN, |o| o.xform.position.x)
    }

    fn move_box(s: &mut SceneState, name: &str, x: f32) {
        let mut o = (**s.find(ObjClass::SceneObject, name).unwrap()).clone();
        o.xform.position.x = x;
        s.replace(o);
    }

    /// What the scene changes of a file's objects while the file does not read is unsaved work:
    /// a save in between leaves the file and keeps the change, and when the file reads again the
    /// change goes over its new text beside what the file's author changed.
    #[test]
    fn a_change_made_while_a_file_does_not_read_goes_over_it_when_it_reads_again() {
        let good = disk();
        let mine = read(&good, None);
        let clean = mine.state.clone();
        // the file breaks under a scene with nothing unsaved: its last reading is held
        let theirs = read(&broken_of(&good), Some(format::Keep { state: &clean, files: &[] }));
        let mut files = theirs.files;
        files.hold_broken(Some(&mine.files));
        assert_eq!(files.held.len(), 1);
        assert!(files.pending_files(&clean, None).is_empty(), "nothing changed yet");
        // a box moved in the window, then a save: the file is left, the change is still held
        let mut now = clean.clone();
        move_box(&mut now, "Box_01", 40.0);
        assert_eq!(files.pending_files(&now, None), ["objects.lua"]);
        let plan = format::write(&now, Some(&files), None).unwrap();
        assert!(plan.files.iter().all(|f| f.file != "objects.lua"));
        let files = plan.after;
        assert_eq!(files.pending_files(&now, None), ["objects.lua"], "a save does not drop it");
        // the author fixes the file and moves the other box in it
        let mut fixed = good.clone();
        for (f, t) in &mut fixed.files {
            if f == "objects.lua" {
                *t = t.replacen("pos = { 2, 0, 0 }", "pos = { 7, 0, 0 }", 1);
            }
        }
        let theirs = read(&fixed, Some(format::Keep { state: &now, files: &[] }));
        let ((back, files), conflicts) = rebase(&now, &files, &theirs.files, None).unwrap();
        assert!(conflicts.is_empty(), "{conflicts:?}");
        assert!(files.broken.is_empty() && files.held.is_empty());
        assert_eq!((box_x(&back, "Box_01"), box_x(&back, "Box_02")), (40.0, 7.0));
        // and the save then writes this side's line into the author's file, nothing else
        let plan = format::write(&back, Some(&files), None).unwrap();
        let written: Vec<&str> = plan.files.iter().map(|f| f.file.as_str()).collect();
        assert_eq!(written, ["objects.lua"]);
        let text = String::from_utf8(plan.files[0].bytes.clone().unwrap()).unwrap();
        assert!(text.contains("40, 0, 0") && text.contains("7, 0, 0"), "{text}");
    }

    /// A file that never read in this session: an object made meanwhile is added to it when it
    /// reads.
    #[test]
    fn an_object_made_while_its_file_never_read_is_added_when_it_reads() {
        let good = disk();
        let empty = SceneState { valid: true, ..SceneState::default() };
        let opened = read(&broken_of(&good), Some(format::Keep { state: &empty, files: &[] }));
        assert_eq!(boxes(&opened.state), 0);
        let mut now = opened.state.clone();
        let mut o = Object::new(
            ObjClass::SceneObject,
            "Box_09",
            ObjData::SceneObject(SceneObjectData::library(r"props\box")),
        );
        o.xform.position = Vec3::new(9.0, 0.0, 0.0);
        now.insert(o);
        assert_eq!(opened.files.pending_files(&now, None), ["objects.lua"]);
        let theirs = read(&good, Some(format::Keep { state: &now, files: &[] }));
        let ((back, _), _) = rebase(&now, &opened.files, &theirs.files, None).unwrap();
        assert_eq!(boxes(&back), 3, "the file's two and the one made here");
        assert_eq!(box_x(&back, "Box_09"), 9.0);
    }

    /// A recovery snapshot of a scene with a file that does not read holds that file's objects,
    /// written whole: a restore has every object the scene had.
    #[test]
    fn a_snapshot_holds_the_objects_of_a_file_that_does_not_read() {
        let good = disk();
        let mine = read(&good, None);
        let mut now = mine.state.clone();
        move_box(&mut now, "Box_01", 40.0);
        let theirs = read(&broken_of(&good), Some(format::Keep { state: &now, files: &[] }));
        let mut files = theirs.files;
        files.hold_broken(Some(&mine.files));
        let snap = snapshot_of(&now, Some(&files), None).unwrap();
        let mut t = Texts::default();
        for (name, bytes) in &snap.files {
            let text = String::from_utf8(bytes.clone()).unwrap();
            match name.rsplit_once('/') {
                None => t.scene = text,
                Some((_, file)) if file.ends_with(".lua") => t.files.push((file.to_owned(), text)),
                Some(_) => {}
            }
        }
        let back = read(&t, None);
        assert!(back.files.broken.is_empty(), "the snapshot reads");
        assert_eq!((boxes(&back.state), box_x(&back.state, "Box_01")), (2, 40.0));
    }

    #[test]
    fn a_changed_library_file_names_its_reference() {
        let r = |p: &str| library_reference(&RelPath::new(p).unwrap());
        assert_eq!(r("rawdata/objects/props/Box_01.object").as_deref(), Some("props\\box_01"));
        assert_eq!(r("RawData/Objects/a.OBJECT").as_deref(), Some("a"));
        assert_eq!(r("rawdata/objects/props/box.thm"), None);
        assert_eq!(r("rawdata/objectsx/a.object"), None);
        assert_eq!(r("levels/a.object"), None);
    }

    #[test]
    fn a_forgotten_object_is_read_again() {
        let dir = std::env::temp_dir().join(format!("gz-library-forget-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let lib = Library::new(vec![dir.clone()]);
        assert!(lib.load("a").is_err(), "not there yet");
        std::fs::write(dir.join("a.object"), b"not an object").unwrap();
        assert!(lib.load("a").unwrap_err().contains("Can't find"), "the failure is remembered");
        let rev = lib.rev();
        lib.forget("A");
        assert_ne!(lib.rev(), rev);
        assert!(lib.load("a").unwrap_err().contains("Can't load"), "the file is read again");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
