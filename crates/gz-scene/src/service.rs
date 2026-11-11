//! What the scene area keeps while a project is open: the store, the library, the pick tree,
//! the recovery session, the load status and the property panels' view.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Weak};
use std::time::Duration;

use gz_base::{Origin, RelPath, SmolStr};
use gz_editor::{CmdError, DirtyDoc, DirtySource, Editor, JobError, ProjectSession};
use gz_query::Derived;
use gz_store::{Store, StoreOptions};
use parking_lot::Mutex;

use crate::apply::{ReplaceKind, SceneCmd};
use crate::class::ObjClass;
use crate::io::{self, PartFiles, SyncSettings};
use crate::library::Library;
use crate::model::{ObjData, SceneState};
use crate::pick::ScenePick;
use crate::recovery::Recovery;

/// What `scene_load_status` answers about the last load.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LoadStatus {
    pub attempted: bool,
    pub loaded: bool,
    pub rollback_ok: bool,
    pub path: String,
    pub error: String,
}

/// The Details and Level Settings panels' own state (`property_inspector`'s filter, open).
#[derive(Clone, Debug, Default)]
pub struct InspectorView {
    pub filter: [String; 2],
    pub open: [bool; 2],
    /// `property_choose`: the row whose chooser is open, and the item picked in it.
    pub choosing: Option<String>,
    pub chosen: String,
}

/// The open project's scene.
pub struct SceneService {
    pub store: Store<SceneState>,
    pub library: Arc<Library>,
    pub recovery: Arc<Recovery>,
    pick: Derived<ScenePick>,
    pub load: Mutex<LoadStatus>,
    pub view: Mutex<InspectorView>,
    /// The project's folder and the editor's.
    pub root: PathBuf,
    pub editor_root: PathBuf,
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
            // the scene's own save, with its reason when it fails (C++ SettleUnsaved)
            let Some(file) = snap.meta.file.clone() else {
                return Err(format!(
                    "{}: the scene has no file yet: save_scene file=<path> first",
                    if name.is_empty() { "Untitled scene" } else { &name }
                ));
            };
            save_scene(editor, &svc, file).await.map_err(|e| format!("{name}: {e}"))
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
        let editor_root = editor.paths.root.clone();
        let mut initial = SceneState::default();
        initial.meta.guid = io::new_guid(root.as_os_str().len() as u64);
        let svc = session.services.get_or_init(|| SceneService {
            store: Store::spawn("scene", initial, &editor.exec, StoreOptions::default()),
            library: Arc::new(Library::new(vec![
                root.join("rawdata").join("objects"),
                editor_root.join("rawdata").join("objects"),
            ])),
            recovery: Arc::new(Recovery::new()),
            pick: Derived::new(editor.exec.clone()),
            load: Mutex::new(LoadStatus { rollback_ok: true, ..LoadStatus::default() }),
            view: Mutex::new(InspectorView::default()),
            root,
            editor_root,
        });
        start_recovery_timer(editor, &svc);
        svc
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

    /// The `$maps$` of the project: where a bare scene name is looked for.
    #[must_use]
    pub fn maps_dir(&self) -> PathBuf {
        self.root.join("rawdata").join("levels")
    }

    /// Loads every library object the scene names, on the IO pool.
    pub async fn prefetch_library(&self, editor: &Editor, state: &SceneState) {
        let refs: Vec<SmolStr> = state
            .objects
            .values()
            .filter_map(|o| match &o.data {
                ObjData::SceneObject(d) if !d.visual_mode() => Some(d.reference.clone()),
                _ => None,
            })
            .collect();
        if refs.is_empty() {
            return;
        }
        let lib = self.library.clone();
        let exec = editor.exec.clone();
        editor.exec.io(move || exec.io_pool().install(|| lib.prefetch(&refs))).await;
    }
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
            let snap = svc.store.snapshot();
            let source = snap.meta.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
            let rec = svc.recovery.clone();
            let result =
                ed.exec.cpu(move || io::write_undo_stream(&snap, true)).await.map_err(|e| e.to_string());
            let saved = match result {
                Ok(stream) => ed.exec.io(move || rec.save_now(&stream, &source)).await,
                Err(e) => Err(e),
            };
            match saved {
                Ok(()) => ed.log.info("* recovery: snapshot saved"),
                Err(e) => {
                    svc.recovery.retry_later();
                    ed.log.warn(format!("! recovery: autosave failed: {e}"));
                }
            }
        }
    });
}

/// Whether the project's game is linked (touches the disk: on the IO pool).
pub(crate) async fn game_linked(editor: &Arc<Editor>) -> bool {
    let Ok((_, project)) = gz_project::Project::of(editor) else { return false };
    editor.exec.io(move || project.game_linked()).await
}

/// Reads the numbered part files of the classes a load needs, in parallel.
fn read_parts(exec: &gz_task::Executors, file: &Path, classes: Vec<ObjClass>) -> PartFiles {
    let file = file.to_path_buf();
    let files = exec.par_io(classes, move |c| {
        let mut out = Vec::new();
        for i in 0.. {
            if i > 0 && !c.uses_ltx() {
                break;
            }
            let p = io::part_path(&file, c, i);
            match std::fs::read(&p) {
                Ok(b) => out.push((p, b)),
                Err(_) => break,
            }
        }
        (c, out)
    });
    PartFiles { files }
}

/// The scene file a name means: absolute as given, otherwise under the project's `$maps$`,
/// `.level` added when the name has no extension.
pub fn resolve_scene(svc: &SceneService, scene: &str) -> Result<PathBuf, CmdError> {
    let mut name = scene.replace('/', "\\");
    let leaf = name.rsplit('\\').next().unwrap_or("");
    match leaf.rfind('.') {
        None => name.push_str(".level"),
        Some(dot) if !leaf[dot..].eq_ignore_ascii_case(".level") => {
            return Err(CmdError::invalid("scene must use the .level extension"));
        }
        Some(_) => {}
    }
    let absolute = name.contains(':') || name.starts_with("\\\\");
    Ok(if absolute { PathBuf::from(name) } else { svc.maps_dir().join(name) })
}

/// The load as a job: the files on the IO pool, the parse on the CPU pool, the library objects
/// prefetched, then one command replaces the scene. Answers when it ended.
pub async fn load_scene(editor: &Arc<Editor>, svc: &Arc<SceneService>, file: PathBuf) -> Result<(), String> {
    {
        let mut st = svc.load.lock();
        *st = LoadStatus {
            attempted: true,
            loaded: false,
            rollback_ok: true,
            path: file.display().to_string().to_lowercase(),
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
        ctx.stage("Reading the scene");
        let main_path = f.clone();
        let main = ctx
            .exec
            .io(move || std::fs::read(&main_path))
            .await
            .map_err(|e| JobError::new(format!("Cannot open scene file: {} ({e})", f.display())))?;
        let kind = io::scene_kind(&main).map_err(JobError::new)?;
        let classes = io::parts_needed(&main, kind);
        ctx.check()?;
        let exec = ctx.exec.clone();
        let pf = f.clone();
        let parts = ctx.exec.io(move || read_parts(&exec, &pf, classes)).await;
        ctx.check()?;
        ctx.stage("Reading the game's configs");
        let settings = crate::game::settings(&ed).await;
        ctx.stage("Parsing");
        let pf = f.clone();
        let loaded = ctx
            .exec
            .cpu(move || {
                let st: SyncSettings<'_> =
                    settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
                io::parse_scene(&main, &pf, &parts, st)
            })
            .await
            .map_err(|e| JobError::new(e.to_string()))?;
        ctx.check()?;
        ctx.stage("Loading the library objects");
        s.prefetch_library(&ed, &loaded.state).await;
        for w in &loaded.warnings {
            ed.log.warn(w);
        }
        let count = loaded.state.objects.len();
        s.store
            .send(SceneCmd::Replace(Box::new(loaded.state), ReplaceKind::Loaded), Origin::Mcp)
            .await
            .map_err(|e| JobError::new(e.message))?;
        ed.log.info(format!("EScene: {count} objects loaded"));
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
        Some(i) => Err(i.error.unwrap_or_else(|| "Scene load failed".into())),
        None => Err("Scene load failed".into()),
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
    }
    result
}

/// Who is told of a scene loaded or saved (`EPrefs->AppendRecentFile`): File > Recent Scene.
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

/// `Project::note_scene`: the project's `last_scene` follows the open scene.
pub(crate) fn note_scene(editor: &Arc<Editor>, svc: &SceneService, file: &Path) {
    SceneFileHooks::of(editor).fire(editor, file);
    if let Ok((_, project)) = gz_project::Project::of(editor) {
        project.note_scene(RelPath::from_fs(&svc.root, file));
    }
}

/// Writes the files of a save the way `SaveTransaction` does: every target staged, the old
/// version of each kept as its `.~` backup, the old contents put back when one write fails.
pub fn commit_files(files: &[io::SaveFile]) -> Result<(), String> {
    let mut written: Vec<(PathBuf, Option<Vec<u8>>)> = Vec::new();
    let rollback = |written: &[(PathBuf, Option<Vec<u8>>)]| {
        for (p, old) in written.iter().rev() {
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
        let old = std::fs::read(&f.path).ok();
        match &f.bytes {
            Some(bytes) => {
                if let Some(o) = &old {
                    let _ = std::fs::write(io::backup_path(&f.path), o);
                }
                if let Err(e) = gz_editor::fsx::write_atomic(&f.path, bytes) {
                    rollback(&written);
                    return Err(format!("Cannot commit staged save file '{}' ({e})", f.path.display()));
                }
                written.push((f.path.clone(), old));
            }
            None => {
                // a numbered part past what the tool needs, and every one after it
                let mut p = f.path.clone();
                while p.exists() {
                    if let Err(e) = std::fs::remove_file(&p) {
                        rollback(&written);
                        return Err(format!("Cannot remove stale level part '{}' ({e})", p.display()));
                    }
                    p = next_numbered(&p);
                }
            }
        }
    }
    Ok(())
}

fn next_numbered(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    let digits = s.chars().rev().take_while(char::is_ascii_digit).count();
    let (base, num) = s.split_at(s.len() - digits);
    let n: u32 = num.parse().unwrap_or(0);
    PathBuf::from(format!("{base}{}", n + 1))
}

/// The save: the snapshot serialised off the store, the files committed on the IO pool, then
/// the store marked saved.
pub async fn save_scene(editor: &Arc<Editor>, svc: &Arc<SceneService>, file: PathBuf) -> Result<(), String> {
    let snap = svc.store.snapshot();
    let f = file.clone();
    let files = editor.exec.cpu(move || io::write_scene(&snap, &f)).await.map_err(|e| e.to_string())?;
    editor.exec.io(move || commit_files(&files)).await?;
    svc.store.send(SceneCmd::Saved { file: file.clone() }, Origin::Mcp).await.map_err(|e| e.message)?;
    let rec = svc.recovery.clone();
    editor.exec.io(move || rec.reset()).await;
    note_scene(editor, svc, &file);
    Ok(())
}
