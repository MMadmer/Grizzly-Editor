//! The scene's commands, `scene.*`: open, new, save, convert, info, the objects and their
//! properties, the selection, the mode, recovery. What makes or removes objects (place, delete,
//! copy, paste, group, align, attach, motion) is in `cmd_edit`; the way and shape editing in
//! `objtools::cmd`; the painted layers in `bulk::cmd`.
//!
//! Every command that needs the scene loads the project's scene first when none is up and waits
//! for it (`service::need_scene`). Angles are degrees in every parameter and answer.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gz_base::{Origin, SmolStr};
use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec};
use gz_entities::PropContext;
use gz_project::{AssetLists, ProjectHooks};
use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::apply::{ReplaceKind, SceneCmd, SelectMode, TransformEdit};
use crate::class::ObjClass;
use crate::format::{self, fields, spawn, values};
use crate::library;
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, Object, SceneMode, SceneState};
use crate::props::{self, Effect, Kind, MergedRow, PropCx, Row, Val};
use crate::service::{self, SceneService};

/// Registers the scene's commands.
pub fn register(reg: &mut Registry) {
    reg.area("scene", "The open level: its objects, the selection, transforms, properties, tools, recovery");
    reg.cmd(
        Spec::write(
            "scene.open",
            "Opens a scene of the project: a .scene, or an X-Ray SDK .level to import.",
        )
        .destroys_with(&["discard"])
        .example("path=levels/escape.scene"),
        scene_open,
    );
    reg.cmd(
        Spec::write("scene.new", "Starts an empty scene with no file; the kinds' eyes and locks stay.")
            .destroys_with(&["discard"]),
        scene_new,
    );
    reg.cmd(
        Spec::write("scene.save", "Saves the scene into its files; path saves it as another scene.")
            .destroys_with(&["overwrite"])
            .example("path=levels/escape_night.scene"),
        scene_save,
    );
    reg.cmd(
        Spec::write(
            "scene.convert",
            "Writes an X-Ray SDK .level of the project as a .scene beside it; the .level stays as it is.",
        )
        .destroys_with(&["overwrite"])
        .example("path=maps/escape.level"),
        scene_convert,
    );
    reg.cmd(
        Spec::read(
            "scene.info",
            "What the open scene is: its file, what it holds by kind, the selection, the camera, its problems.",
        )
        .example("summary=true"),
        scene_info,
    );
    reg.cmd(
        Spec::read(
            "scene.objects",
            "Lists the scene's objects, by kind, found by name, in a box or near a point, with their boxes.",
        )
        .example("near=10,0,5 radius=8 sort=distance bounds=true"),
        scene_objects,
    );
    reg.cmd(
        Spec::read(
            "scene.get",
            "The properties of objects (the selection without objects) or of the level, with labels, units and choices.",
        )
        .example("objects=lamp_01 keys=range,color"),
        scene_get,
    );
    reg.cmd(
        Spec::write(
            "scene.set",
            "Sets properties of objects (the selection without objects), of the level, or a kind's eye and lock.",
        )
        .example("objects=lamp_01 key=range value=12"),
        scene_set,
    );
    reg.cmd(
        Spec::read("scene.choices", "What a property picks from: its words, or the items of its list.")
            .example("object=stalker_01 key=props.visual query=bandit"),
        scene_choices,
    );
    reg.cmd(
        Spec::view("scene.select", "Selects objects by name or kind, all, none, or the rest.")
            .example("classes=light add=true"),
        scene_select,
    );
    reg.cmd(
        Spec::view(
            "scene.mode",
            "Says or sets what the viewport's clicks do: select, place, paint a layer, zones.",
        )
        .example("mode=select"),
        scene_mode,
    );
    reg.cmd(
        Spec::write(
            "scene.recover",
            "The scene's recovery snapshots: what there is, take one now, restore the newest, discard them.",
        )
        .destroys_with(&["action=discard"])
        .example("action=restore"),
        scene_recover,
    );
    crate::cmd_edit::register(reg);
    crate::objtools::cmd::register(reg);
    crate::bulk::cmd::register(reg);
}

/// Hooks the scene into the project's life, the library lists (`file.list source=library`) and
/// the asset kinds.
pub fn install(editor: &Editor) {
    let kinds = gz_asset::Kinds::of(editor);
    kinds.add(crate::kind::scene());
    kinds.add(crate::kind::group());
    let lists = AssetLists::of(editor);
    lists.provide("Objects", |ed| {
        if let Some(session) = ed.project_opt()
            && let Some(svc) = session.services.get::<SceneService>()
        {
            return svc.library.list().as_ref().clone();
        }
        // no project: the editor's own library, listed once
        let lib = ed.services.get_or_init(|| {
            EditorLibrary(library::Library::new(vec![
                gz_project::library_root(ed).join("rawdata").join("objects"),
            ]))
        });
        lib.0.list().as_ref().clone()
    });
    lists.provide("Groups", |ed| {
        library::list_groups(&gz_project::library_root(ed).join("rawdata").join("groups"))
    });
    lists.provide("Particles", particles);
    lists.provide("Light Anims", light_anims);
    let hooks = ProjectHooks::of(editor);
    hooks.provide_live_scene(|ed| {
        ed.project_opt().and_then(|s| s.services.get::<SceneService>()).is_some_and(|svc| {
            let snap = svc.store.snapshot();
            snap.valid && (!snap.objects.is_empty() || svc.store.is_dirty())
        })
    });
    hooks.on_open(|ed, session| {
        let svc = SceneService::of_session(ed, session);
        let ed = ed.clone();
        let exec = ed.exec.clone();
        exec.spawn(async move {
            let rec = svc.recovery.clone();
            let root = svc.root.clone();
            ed.exec.io(move || rec.bind(&root)).await;
            let Ok((_, project)) = gz_project::Project::of(&ed) else { return };
            if let Ok(Some(rel)) = project.take_pending_scene().await {
                let file = rel.to_fs(&svc.root);
                if let Err(e) = service::load_scene(&ed, &svc, file).await {
                    ed.log.warn(format!("The last scene did not open: {e}"));
                }
            }
        });
    });
    hooks.on_close(|ed, session| {
        if let Some(svc) = session.services.get::<SceneService>() {
            let dirty = svc.store.is_dirty();
            let rec = svc.recovery.clone();
            ed.exec.spawn_io(move || rec.end(dirty));
        }
    });
}

/// The editor's own library while no project is open.
struct EditorLibrary(library::Library);

/// The library's particle effects by name, then its effect groups by name (blocking).
fn particles(ed: &Editor) -> Vec<String> {
    let Ok(bytes) = std::fs::read(gz_project::library_root(ed).join("gamedata").join("particles.xr")) else {
        return Vec::new();
    };
    let Ok(lib) = gz_fmt_xr::Particles::read(&bytes) else { return Vec::new() };
    let mut effects: Vec<String> = lib.effects.iter().map(|e| e.name.to_string()).collect();
    let mut groups: Vec<String> = lib.groups.iter().map(|g| g.name.to_string()).collect();
    effects.sort_unstable();
    groups.sort_unstable();
    effects.extend(groups);
    effects
}

/// The library's light animations in the file's order (blocking).
fn light_anims(ed: &Editor) -> Vec<String> {
    let Ok(bytes) = std::fs::read(gz_project::library_root(ed).join("gamedata").join("lanims.xr")) else {
        return Vec::new();
    };
    gz_fmt_xr::LightAnims::read(&bytes)
        .map(|lib| lib.items.iter().map(|a| a.name.to_string()).collect())
        .unwrap_or_default()
}

// ---------------------------------------------------------------- shared

/// A length or a factor as answers write it.
pub(crate) fn r4(v: f32) -> f64 {
    (f64::from(v) * 10_000.0).round() / 10_000.0
}

pub(crate) fn v3(v: Vec3) -> Value {
    json!([r4(v.x), r4(v.y), r4(v.z)])
}

/// Radians as degrees.
pub(crate) fn deg3(v: Vec3) -> Value {
    json!([values::degrees(v.x), values::degrees(v.y), values::degrees(v.z)])
}

/// A path as answers write it: project-relative with `/` inside the project, else absolute.
pub(crate) fn wire(root: &Path, file: &Path) -> String {
    gz_base::RelPath::from_fs(root, file)
        .map_or_else(|| file.display().to_string().replace('\\', "/"), |r| r.as_str().to_owned())
}

/// The kinds a `classes` list names (keys, labels or the X-Ray names).
pub(crate) fn kinds_of(list: Option<&[String]>) -> Result<Option<Vec<ObjClass>>, CmdError> {
    let Some(list) = list.filter(|l| !l.is_empty()) else { return Ok(None) };
    let mut out = Vec::with_capacity(list.len());
    for k in list {
        let c = ObjClass::by_key(k.trim()).ok_or_else(|| {
            let known: Vec<&str> =
                ObjClass::ALL.iter().filter(|c| c.is_object_tool()).map(|c| c.key()).collect();
            CmdError::invalid(format!("'{k}' is no kind of object: one of {}", known.join(", ")))
        })?;
        out.push(c);
    }
    Ok(Some(out))
}

/// An object as lists write it.
pub(crate) fn brief(s: &SceneState, o: &Object) -> Value {
    let caps = o.class.caps();
    let mut m = Map::new();
    m.insert("name".into(), json!(o.name.as_str()));
    m.insert("kind".into(), json!(o.class.key()));
    m.insert("position".into(), v3(o.position()));
    if caps.rotates {
        m.insert("rotation".into(), deg3(o.rotation()));
    }
    if caps.scales {
        m.insert("scale".into(), v3(o.scale()));
    }
    if let Some(r) = o.ref_name().filter(|r| !r.is_empty()) {
        m.insert("ref".into(), json!(r));
    }
    if s.is_selected(o.id) {
        m.insert("selected".into(), json!(true));
    }
    if !o.flags.visible {
        m.insert("hidden".into(), json!(true));
    }
    if o.flags.locked {
        m.insert("locked".into(), json!(true));
    }
    if let Some(g) = o.owner.and_then(|id| s.get(id)) {
        m.insert("group".into(), json!(g.name.as_str()));
    }
    Value::Object(m)
}

/// The scene is saved first (`save`), its changes dropped (`discard`), or the call answers
/// `unsaved`.
pub(crate) async fn settle_scene(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    save: bool,
    discard: bool,
) -> Result<(), CmdError> {
    if save && discard {
        return Err(CmdError::invalid("save and discard exclude each other"));
    }
    if !svc.store.is_dirty() || discard {
        return Ok(());
    }
    let snap = svc.store.snapshot();
    if save {
        let file =
            snap.meta.file.clone().ok_or_else(|| {
                CmdError::invalid("the scene has no file yet: scene.save path=<file> first")
            })?;
        return service::save_scene(ed, svc, file).await.map(|_| ()).map_err(CmdError::io);
    }
    let name = snap.meta.name();
    Err(CmdError::unsaved(json!([{
        "kind": "scene",
        "name": if name.is_empty() { "Untitled scene".to_owned() } else { name },
        "path": snap.meta.file.as_ref().map(|f| wire(&svc.root, f)),
    }])))
}

/// The Details sheet's lists the linked game gives (story ids, the spawn sections' names), read
/// now and kept for the panel (`SceneService::prop_cx`).
pub async fn prop_context(ed: &Arc<Editor>) -> Arc<PropContext> {
    let mut ctx = crate::game::settings(ed).await.map(|g| g.prop_context()).unwrap_or_default();
    ctx.profiles = crate::game::spawn_sections(ed)
        .await
        .iter()
        .map(|s| (SmolStr::new(&s.section), SmolStr::new(&s.group)))
        .collect();
    match SceneService::of(ed) {
        Ok(svc) => svc.set_prop_cx(ctx),
        Err(_) => Arc::new(ctx),
    }
}

// ---------------------------------------------------------------- open, new, save, convert

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct OpenParams {
    /// The scene: project-relative or absolute, `.scene` or an X-Ray SDK `.level` (imported:
    /// Save then writes the `.scene` beside it). Without an extension the `.scene`.
    path: String,
    /// Save the open scene's unsaved changes first.
    #[serde(default)]
    save: bool,
    /// Drop the open scene's unsaved changes.
    #[serde(default)]
    discard: bool,
}

/// A scene job under way (a load, a tool) is waited for.
async fn wait_scene_jobs(ed: &Arc<Editor>) {
    for _ in 0..8 {
        let running: Vec<_> = ed
            .jobs
            .snapshot()
            .into_iter()
            .filter(|j| j.running() && j.group.as_deref() == Some("scene"))
            .map(|j| j.id)
            .collect();
        if running.is_empty() {
            return;
        }
        for id in running {
            ed.jobs.wait(id).await;
        }
    }
}

async fn scene_open(ed: Arc<Editor>, p: OpenParams, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let full = service::resolve_scene(&svc, p.path.trim())?;
    let check = full.clone();
    if !ed.exec.io(move || check.is_file()).await {
        return Err(CmdError::not_found(format!("There is no scene {}.", wire(&svc.root, &full))));
    }
    wait_scene_jobs(&ed).await;
    settle_scene(&ed, &svc, p.save, p.discard).await?;
    service::load_scene(&ed, &svc, full).await.map_err(|e| CmdError::new("load_failed", e))?;
    info_reply(&ed, &svc, false).await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct NewParams {
    /// Save the open scene's unsaved changes first.
    #[serde(default)]
    save: bool,
    /// Drop the open scene's unsaved changes.
    #[serde(default)]
    discard: bool,
}

async fn scene_new(ed: Arc<Editor>, p: NewParams, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    svc.recovery.can_replace_scene().map_err(CmdError::invalid)?;
    wait_scene_jobs(&ed).await;
    settle_scene(&ed, &svc, p.save, p.discard).await?;
    crate::edit::new_scene(&ed, &svc, o).await?;
    svc.set_opened();
    Ok(json!({ "file": Value::Null, "objects": 0 }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SaveParams {
    /// Save as this scene (project-relative or absolute; `.scene` is added): the scene then
    /// lives there. Left out: its own file.
    #[serde(default)]
    path: Option<String>,
    /// With path: replace another scene that is there already (its files are lost).
    #[serde(default)]
    overwrite: bool,
}

/// The `.scene` a save path means.
fn save_path(svc: &SceneService, path: &str) -> Result<PathBuf, CmdError> {
    let p = PathBuf::from(path.trim().replace('/', "\\"));
    let full = if p.is_absolute() { p } else { svc.root.join(p) };
    match full.extension().and_then(|e| e.to_str()) {
        None => Ok(full.with_extension(format::EXT)),
        Some(e) if e.eq_ignore_ascii_case(format::EXT) => Ok(full),
        Some(_) => Err(CmdError::invalid("a scene is saved as a .scene file")),
    }
}

async fn scene_save(ed: Arc<Editor>, p: SaveParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let file = match p.path.as_deref().filter(|s| !s.trim().is_empty()) {
        Some(path) => save_path(&svc, path)?,
        None => svc
            .store
            .snapshot()
            .meta
            .file
            .clone()
            .ok_or_else(|| CmdError::invalid("the scene has no file yet: scene.save path=<file>"))?,
    };
    // another scene's files are never replaced by the way: the caller names it
    let own = svc.store.snapshot().meta.file.clone();
    let same =
        own.as_ref().is_some_and(|o| o.to_string_lossy().eq_ignore_ascii_case(&file.to_string_lossy()));
    if !same && !p.overwrite {
        let there = file.clone();
        if ed.exec.io(move || there.exists()).await {
            return Err(CmdError::new(
                "exists",
                format!(
                    "{} is another scene: saving as it replaces its files (overwrite=true)",
                    wire(&svc.root, &file)
                ),
            ));
        }
    }
    let written = service::save_scene(&ed, &svc, file.clone()).await.map_err(CmdError::io)?;
    let mut out = json!({ "file": wire(&svc.root, &file), "written": written });
    // a part file that does not read was left as it is: the caller must know its objects' changes
    // are not on the disk
    let skipped: Vec<&'static str> = svc.files().map(|f| f.broken.clone()).unwrap_or_default();
    if let (false, Value::Object(m)) = (skipped.is_empty(), &mut out) {
        m.insert("skipped".into(), json!(skipped));
        m.insert(
            "warning".into(),
            json!(format!(
                "{} of the scene does not read and was left as it is: what the scene changed of its objects is not written. Fix the file (asset.check names the line) and save again.",
                skipped.join(", ")
            )),
        );
    }
    Ok(out)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ConvertParams {
    /// The X-Ray SDK scenes (`.level`, project-relative or absolute). Left out: every `.level`
    /// of the project with no `.scene` beside it.
    #[serde(default)]
    path: Option<Vec<String>>,
    /// Write the `.scene` even where one is there already (it is replaced).
    #[serde(default)]
    overwrite: bool,
}

/// Every `.level` under `root`, the editor's scratch and the tooling left out (blocking).
fn sdk_scenes(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(dir) = dirs.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if gz_project::rules::is_tooling_entry(&name) {
                continue;
            }
            let Ok(ft) = e.file_type() else { continue };
            let p = e.path();
            if ft.is_dir() {
                if dir == root && gz_project::rules::is_editor_only_entry(&name) {
                    continue;
                }
                dirs.push(p);
            } else if service::is_sdk(&p) {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

/// Imports an X-Ray SDK scene and writes it as the `.scene` beside it (blocking). Answers the
/// `.scene` and its import's warnings.
pub(crate) fn convert_file(
    root: &Path,
    level: &Path,
    settings: crate::sdk::SyncSettings<'_>,
    watch: Option<&gz_project::ProjectWatch>,
) -> Result<(PathBuf, Vec<String>), String> {
    let opened = service::open_files(root, level, settings)?;
    let scene = opened.state.meta.file.clone().unwrap_or_else(|| level.with_extension(format::EXT));
    let plan = format::write(&opened.state, None, settings)?;
    service::commit(&scene, &plan.files, watch)?;
    Ok((scene, opened.problems.iter().map(gz_base::Problem::line_text).collect()))
}

async fn scene_convert(ed: Arc<Editor>, p: ConvertParams, _o: Origin) -> Reply {
    let (_, project) = gz_project::Project::of(&ed).map_err(|_| CmdError::no_project())?;
    let root = ed.project()?.root.clone();
    let given = p.path.clone().filter(|l| !l.is_empty());
    let r = root.clone();
    let levels: Vec<PathBuf> = match &given {
        Some(list) => list
            .iter()
            .map(|s| {
                let p = PathBuf::from(s.trim().replace('/', "\\"));
                if p.is_absolute() { p } else { r.join(p) }
            })
            .collect(),
        None => ed.exec.io(move || sdk_scenes(&r)).await,
    };
    let guard = project.write_guard(None);
    let settings = crate::game::settings(&ed).await;
    let watch = project.tree.changes().clone();
    let mut converted = Vec::new();
    let mut skipped = Vec::new();
    let mut failed = Vec::new();
    for level in levels {
        if !service::is_sdk(&level) {
            failed.push(json!({ "path": wire(&root, &level), "error": "not an X-Ray SDK .level" }));
            continue;
        }
        let scene = level.with_extension(format::EXT);
        if let Err(e) = guard.check(&scene) {
            failed.push(json!({ "path": wire(&root, &level), "error": e.to_string() }));
            continue;
        }
        let (s, l) = (scene.clone(), level.clone());
        let (exists, present) = ed.exec.io(move || (s.is_file(), l.is_file())).await;
        if !present {
            failed.push(json!({ "path": wire(&root, &level), "error": "there is no such file" }));
            continue;
        }
        if exists && !p.overwrite {
            skipped.push(json!({ "path": wire(&root, &level), "scene": wire(&root, &scene) }));
            continue;
        }
        let (rt, lv, st, w) = (root.clone(), level.clone(), settings.clone(), watch.clone());
        let done = ed
            .exec
            .io(move || {
                convert_file(
                    &rt,
                    &lv,
                    st.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
                    Some(&w),
                )
            })
            .await;
        match done {
            Ok((scene, warnings)) => {
                for w in &warnings {
                    ed.log.warn(w);
                }
                converted.push(json!({
                    "path": wire(&root, &level),
                    "scene": wire(&root, &scene),
                    "warnings": warnings.len(),
                }));
            }
            Err(e) => failed.push(json!({ "path": wire(&root, &level), "error": e })),
        }
    }
    let out = json!({ "converted": converted, "skipped": skipped, "failed": failed });
    if failed.is_empty() { Ok(out) } else { Err(CmdError::report(out)) }
}

// ---------------------------------------------------------------- info, objects

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct InfoParams {
    /// Also count what the scene uses: objects, faces, textures and their memory (Scene >
    /// Summary Info). `scene.summary` writes it as a text file.
    #[serde(default)]
    summary: bool,
}

fn camera_json(s: &SceneState) -> Value {
    let hpb = s.meta.camera_hpb;
    json!({
        "position": v3(s.meta.camera_pos),
        "heading": values::degrees(hpb.x),
        "pitch": values::degrees(hpb.y),
        "bank": values::degrees(hpb.z),
    })
}

fn kinds_json(s: &SceneState) -> Value {
    Value::Array(
        ObjClass::ALL
            .iter()
            .filter(|c| c.is_object_tool())
            .map(|c| {
                let p = s.tools.part(*c);
                let mut m = Map::new();
                m.insert("kind".into(), json!(c.key()));
                m.insert("label".into(), json!(c.label_plural()));
                m.insert("objects".into(), json!(s.count(*c)));
                if !p.visible {
                    m.insert("hidden".into(), json!(true));
                }
                if p.locked {
                    m.insert("locked".into(), json!(true));
                }
                Value::Object(m)
            })
            .collect(),
    )
}

async fn info_reply(ed: &Arc<Editor>, svc: &Arc<SceneService>, summary: bool) -> Reply {
    let s = svc.store.snapshot();
    let load = svc.load.lock().clone();
    let problems: Vec<Value> =
        svc.problems().iter().map(|p| serde_json::to_value(p).unwrap_or(Value::Null)).collect();
    let mut out = json!({
        "file": s.meta.file.as_ref().map(|f| wire(&svc.root, f)),
        "imported": s.meta.imported.as_ref().map(|f| wire(&svc.root, f)),
        "modified": svc.store.is_dirty(),
        "objects": s.objects.len(),
        "selected": s.selection.len(),
        "mode": s.tools.mode.word(),
        "level": s.meta.level.level_path.as_str(),
        "kinds": kinds_json(&s),
        "camera": camera_json(&s),
        "load": {
            "attempted": load.attempted,
            "loaded": load.loaded,
            "path": if load.path.is_empty() { String::new() } else { wire(&svc.root, Path::new(&load.path)) },
            "error": load.error,
        },
        "problems": problems,
    });
    if summary {
        let rows = crate::cmd_edit::summary_rows(&*crate::cmd_edit::collect_summary(ed, svc).await?);
        if let Value::Object(m) = &mut out {
            m.insert("summary".into(), rows);
        }
    }
    Ok(out)
}

async fn scene_info(ed: Arc<Editor>, p: InfoParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    info_reply(&ed, &svc, p.summary).await
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ObjectsParams {
    /// Only these kinds (`object`, `light`, `glow`, `sound`, `sound_env`, `particles`,
    /// `shape`, `fog`, `spawn`, `way`, `sector`, `portal`, `group`).
    #[serde(default)]
    classes: Option<Vec<String>>,
    /// Only names that hold this text (any case).
    #[serde(default)]
    query: Option<String>,
    /// Only the selected objects.
    #[serde(default)]
    selected: bool,
    /// At most this many (default 100).
    #[serde(default)]
    limit: Option<usize>,
    /// Skip this many first.
    #[serde(default)]
    offset: Option<usize>,
    /// Answer the kinds with their counts and their objects under each (`kinds`).
    #[serde(default)]
    tree: bool,
    /// Each object carries its world box (`bounds {min, max}`) and `size`, where it has one.
    #[serde(default)]
    bounds: bool,
    /// Only the objects whose world box meets this box: `[minx, miny, minz, maxx, maxy, maxz]`.
    #[serde(default)]
    r#box: Option<[f32; 6]>,
    /// Only the objects within `radius` of this point (metres; their box, else their position).
    #[serde(default)]
    near: Option<[f32; 3]>,
    /// With near: how far, metres (10).
    #[serde(default)]
    radius: Option<f32>,
    /// The order: `name`, or `distance` from near (left out: by kind, as the scene holds them).
    #[serde(default)]
    sort: Option<ObjectsSort>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ObjectsSort {
    /// By name, any case.
    Name,
    /// Nearest to `near` first.
    Distance,
}

/// How far a point is from an object: from its box (0 inside it), else from its position.
fn distance_to(b: Option<&crate::math::Aabb>, o: &Object, at: Vec3) -> f32 {
    match b {
        Some(b) => (at.clamp(b.min, b.max) - at).length(),
        None => o.position().distance(at),
    }
}

/// A brief with the object's box when asked for.
fn brief_boxed(s: &SceneState, o: &Object, b: Option<&crate::math::Aabb>, with: bool) -> Value {
    let mut v = brief(s, o);
    if with && let (Value::Object(m), Some(b)) = (&mut v, b) {
        m.insert("bounds".into(), crate::sense::bounds_json(b));
        m.insert("size".into(), crate::sense::size_json(b));
    }
    v
}

async fn scene_objects(ed: Arc<Editor>, p: ObjectsParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let s = svc.store.snapshot();
    let classes = kinds_of(p.classes.as_deref())?;
    let near = p.near.map(Vec3::from_array).filter(|v| v.is_finite());
    if p.near.is_some() && near.is_none() {
        return Err(CmdError::invalid("near takes finite numbers"));
    }
    if p.radius.is_some() && near.is_none() {
        return Err(CmdError::invalid("radius goes with near: the point"));
    }
    if p.sort == Some(ObjectsSort::Distance) && near.is_none() {
        return Err(CmdError::invalid("sort=distance takes near: the point distances are from"));
    }
    let radius = p.radius.unwrap_or(10.0);
    if !radius.is_finite() || radius < 0.0 {
        return Err(CmdError::invalid("radius takes metres, 0 or more"));
    }
    let within = match p.r#box {
        Some([x0, y0, z0, x1, y1, z1]) => {
            let b = crate::math::Aabb::new(Vec3::new(x0, y0, z0), Vec3::new(x1, y1, z1));
            if !b.min.is_finite() || !b.max.is_finite() || b.max.cmplt(b.min).any() {
                return Err(CmdError::invalid(
                    "box takes [minx, miny, minz, maxx, maxy, maxz], max not below min",
                ));
            }
            Some(b)
        }
        None => None,
    };
    let spatial = p.bounds || within.is_some() || near.is_some();
    if spatial {
        svc.prefetch_library(&ed, &s).await;
    }
    let boxes: rustc_hash::FxHashMap<ObjId, crate::math::Aabb> = if spatial {
        s.objects
            .values()
            .filter_map(|o| crate::sense::world_bounds(&s, &svc.library, o).map(|b| (o.id, b)))
            .collect()
    } else {
        rustc_hash::FxHashMap::default()
    };
    let placed = |o: &Object| {
        let b = boxes.get(&o.id);
        let in_box = within
            .as_ref()
            .is_none_or(|w| b.is_some_and(|b| (0..3).all(|a| b.min[a] <= w.max[a] && b.max[a] >= w.min[a])));
        in_box && near.is_none_or(|at| distance_to(b, o, at) <= radius)
    };
    let query = p.query.as_deref().unwrap_or("").trim().to_lowercase();
    let (limit, offset) = (p.limit.unwrap_or(100), p.offset.unwrap_or(0));
    let mut kinds: Vec<ObjClass> = ObjClass::ALL
        .iter()
        .copied()
        .filter(|c| c.is_object_tool() && classes.as_ref().is_none_or(|l| l.contains(c)))
        .collect();
    kinds.sort_by_key(|c| c.label_plural());
    let matches = |o: &Object| {
        (query.is_empty() || o.name.to_lowercase().contains(&query))
            && (!p.selected || s.is_selected(o.id))
            && placed(o)
    };
    let shown_brief = |o: &Object| brief_boxed(&s, o, boxes.get(&o.id), p.bounds);
    if p.tree {
        let mut tree = Vec::new();
        for c in kinds {
            let found: Vec<&Arc<Object>> = s.of_class(c).filter(|o| matches(o)).collect();
            if found.is_empty() {
                continue;
            }
            let part = s.tools.part(c);
            tree.push(json!({
                "kind": c.key(),
                "label": c.label_plural(),
                "count": found.len(),
                "hidden": !part.visible,
                "locked": part.locked,
                "objects": found.iter().skip(offset).take(limit).map(|o| shown_brief(o)).collect::<Vec<_>>(),
            }));
        }
        return Ok(json!({ "kinds": tree }));
    }
    let mut found: Vec<&Arc<Object>> =
        kinds.iter().flat_map(|c| s.of_class(*c).filter(|o| matches(o))).collect();
    match (p.sort, near) {
        (Some(ObjectsSort::Name), _) => found.sort_by_cached_key(|o| o.name.to_lowercase()),
        (Some(ObjectsSort::Distance), Some(at)) => {
            found.sort_by(|a, b| {
                distance_to(boxes.get(&a.id), a, at).total_cmp(&distance_to(boxes.get(&b.id), b, at))
            });
        }
        _ => {}
    }
    let total = found.len();
    let shown: Vec<Value> = found
        .iter()
        .skip(offset)
        .take(limit)
        .map(|o| {
            let mut v = shown_brief(o);
            if let (Some(at), Value::Object(m)) = (near, &mut v) {
                m.insert("distance".into(), json!(r4(distance_to(boxes.get(&o.id), o, at))));
            }
            v
        })
        .collect();
    Ok(json!({ "objects": shown, "total": total }))
}

// ---------------------------------------------------------------- properties

/// What a row's kind is called in answers.
fn kind_word(k: &Kind) -> &'static str {
    match k {
        Kind::Caption => "info",
        Kind::Text => "text",
        Kind::Name => "name",
        Kind::Choose(_) => "pick",
        Kind::Bool | Kind::Flag => "bool",
        Kind::Float { .. } => "number",
        Kind::Angle { .. } => "angle",
        Kind::Int { .. } => "integer",
        Kind::Token(_) => "choice",
        Kind::Vector { .. } => "vector",
        Kind::Angle3 { .. } => "angles",
        Kind::Color | Kind::FColor => "color",
        Kind::Button(_) => "buttons",
        Kind::Time => "time",
        Kind::Canvas => "canvas",
    }
}

/// A row's value as `scene.set` takes it back.
fn value_json(r: &Row) -> Value {
    match (&r.kind, &r.value) {
        (Kind::Angle { .. }, Val::Num(v)) => json!(values::degrees(*v as f32)),
        (Kind::Int { .. } | Kind::Time, Val::Num(v)) => json!(*v as i64),
        (Kind::Token(_), Val::Num(_)) => json!(r.text()),
        (Kind::Float { .. }, Val::Num(v)) => json!(r4(*v as f32)),
        (Kind::Angle3 { .. }, Val::Vec3(v)) => deg3(*v),
        (_, Val::Vec3(v)) => v3(*v),
        (_, Val::Num(v)) => json!(*v),
        (_, Val::Bool(b)) => json!(*b),
        (_, Val::Text(t)) => json!(t),
        (_, Val::Color(_) | Val::FColor(_)) => json!(r.text()),
        (_, Val::None) => Value::Null,
    }
}

/// One row as `scene.get` answers it.
fn row_json(m: &MergedRow) -> Value {
    let r = &m.row;
    let mut o = Map::new();
    o.insert("key".into(), json!(r.key));
    o.insert("label".into(), json!(r.label));
    let section = r.path.rsplit_once('\\').map_or("", |(s, _)| s);
    if !section.is_empty() {
        o.insert("section".into(), json!(section));
    }
    o.insert("type".into(), json!(kind_word(&r.kind)));
    o.insert("value".into(), if m.mixed { json!("(mixed)") } else { value_json(r) });
    let unit = props::unit_of(r);
    if !unit.is_empty() {
        o.insert("unit".into(), json!(unit));
    }
    match &r.kind {
        Kind::Token(items) => {
            o.insert("choices".into(), json!(items.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>()));
        }
        Kind::Button(c) => {
            o.insert("choices".into(), json!(c.iter().map(SmolStr::as_str).collect::<Vec<_>>()));
        }
        Kind::Choose(c) => {
            o.insert("list".into(), json!(c.list.as_str()));
            if c.max > 1 {
                o.insert("several".into(), json!(true));
            }
        }
        _ => {}
    }
    if !r.enabled {
        o.insert("enabled".into(), json!(false));
    }
    let help = props::help_of(r);
    if !help.is_empty() {
        o.insert("help".into(), json!(help));
    }
    Value::Object(o)
}

/// The rows of one object, in the panel's order.
pub(crate) fn rows_of(s: &SceneState, svc: &SceneService, o: &Object, ctx: &PropContext) -> Vec<MergedRow> {
    let cx = PropCx { lib: Some(&svc.library), entity: ctx };
    props::tree_order(props::merge(vec![props::object_rows(s, o, &cx)]))
}

/// The rows of the selection (one list, "mixed" where the objects differ) or of the level.
pub fn selection_rows(
    s: &SceneState,
    lib: &library::Library,
    world: bool,
    ctx: &PropContext,
) -> Vec<MergedRow> {
    if world {
        return props::tree_order(props::merge(vec![props::world_rows(s)]));
    }
    let cx = PropCx { lib: Some(lib), entity: ctx };
    let lists: Vec<Vec<Row>> = s
        .selected_in_order()
        .iter()
        .filter_map(|id| s.get(*id))
        .map(|o| props::object_rows(s, o, &cx))
        .collect();
    props::tree_order(props::merge(lists))
}

/// Whether a row is one the caller asked for.
fn wanted(r: &Row, keys: Option<&[String]>, query: &str) -> bool {
    let by_key = keys.is_none_or(|ks| {
        ks.iter().any(|k| {
            let k = k.trim();
            r.key == k || r.key.starts_with(&format!("{k}.")) || r.label.eq_ignore_ascii_case(k)
        })
    });
    by_key
        && (query.is_empty()
            || r.key.to_lowercase().contains(query)
            || r.label.to_lowercase().contains(query))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct GetParams {
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// The level's own properties (Level Settings) instead of objects'.
    #[serde(default)]
    world: bool,
    /// Only these keys (a key also takes the keys under it: `order` gives `order.kind`, ...).
    #[serde(default)]
    keys: Option<Vec<String>>,
    /// Only keys or labels that hold this text.
    #[serde(default)]
    query: Option<String>,
}

async fn scene_get(ed: Arc<Editor>, p: GetParams, _o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let s = svc.store.snapshot();
    svc.prefetch_library(&ed, &s).await;
    let ctx = prop_context(&ed).await;
    let query = p.query.as_deref().unwrap_or("").trim().to_lowercase();
    let pick = |rows: Vec<MergedRow>| -> Vec<Value> {
        rows.iter().filter(|m| wanted(&m.row, p.keys.as_deref(), &query)).map(row_json).collect()
    };
    if p.world {
        return Ok(json!({ "properties": pick(selection_rows(&s, &svc.library, true, &ctx)) }));
    }
    let ids = crate::edit::ids_of(&s, p.objects.as_deref())?;
    let objects: Vec<Value> = ids
        .iter()
        .filter_map(|id| s.get(*id))
        .map(|o| {
            let mut b = brief(&s, o);
            if let Value::Object(m) = &mut b {
                if let Some(bx) = crate::sense::world_bounds(&s, &svc.library, o) {
                    m.insert("bounds".into(), crate::sense::bounds_json(&bx));
                    m.insert("size".into(), crate::sense::size_json(&bx));
                }
                m.insert("label".into(), json!(o.class.desc()));
                m.insert("properties".into(), Value::Array(pick(rows_of(&s, &svc, o, &ctx))));
            }
            b
        })
        .collect();
    Ok(json!({ "objects": objects }))
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SetParams {
    /// The objects by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Set the level's own properties (Level Settings) instead of objects'.
    #[serde(default)]
    world: bool,
    /// The kinds themselves (`light`, `spawn`, ...): their eye and lock, with hidden / locked.
    #[serde(default)]
    classes: Option<Vec<String>>,
    /// Properties by key (`scene.get` lists them) and their values, as `scene.get` writes
    /// them: numbers in their unit (degrees for angles), words for choices, `#RRGGBB` for
    /// colours, `[x,y,z]` for vectors, a table for a key that holds one (`order`).
    #[serde(default)]
    set: Option<Map<String, Value>>,
    /// One property by its key, with `value`: the same as `set` with one key.
    #[serde(default)]
    key: Option<String>,
    /// The value of `key`.
    #[serde(default)]
    value: Option<Value>,
    /// A new name (one object only). Names keep their case and are unique in the scene.
    #[serde(default)]
    name: Option<String>,
    /// Metres.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// Degrees.
    #[serde(default)]
    rotation: Option<[f32; 3]>,
    /// A factor per axis.
    #[serde(default)]
    scale: Option<[f32; 3]>,
    /// Hide (or show) in the viewport; kept with the scene.
    #[serde(default)]
    hidden: Option<bool>,
    /// Lock (or unlock): a locked object is not picked, moved or changed.
    #[serde(default)]
    locked: Option<bool>,
}

/// A JSON value as the text a row parses.
fn text_of(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        Value::Array(a) => a.iter().map(text_of).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        Value::Object(_) => v.to_string(),
    }
}

/// `props.visual` as the configs carry it: meshes-relative, backslashes, no `.ogf`.
fn visual_name(v: &str) -> String {
    let mut v = v.trim().replace('/', "\\");
    if v.len() >= 7 && v[..7].eq_ignore_ascii_case("meshes\\") {
        v = v[7..].to_owned();
    }
    if let Some(dot) = v.rfind('.')
        && !v[dot..].contains('\\')
        && v[dot..].eq_ignore_ascii_case(".ogf")
    {
        v.truncate(dot);
    }
    v
}

/// Whether the game or the project has a model.
async fn model_exists(ed: &Arc<Editor>, svc: &SceneService, visual: &str) -> bool {
    let rel = format!("meshes\\{visual}.ogf");
    let in_project = svc.root.join("gamedata").join(&rel);
    if ed.exec.io(move || in_project.is_file()).await {
        return true;
    }
    match gz_game::GameLink::of(ed) {
        Ok(link) => link.files().await.is_ok_and(|f| f.exists(&rel)),
        Err(_) => false,
    }
}

/// One key of one object: the edit it comes to.
fn set_key(
    s: &SceneState,
    o: &Object,
    key: &str,
    value: &Value,
    rows: &[MergedRow],
    cx: &PropCx<'_>,
) -> Result<Effect, String> {
    // a key of the files that holds a table takes one (`order = { kind = "guard" }`)
    if let Some(f) = fields::object_field(o.class, key)
        && matches!(f.ty, fields::Ty::Table(_) | fields::Ty::Range)
        && !matches!(key, "props" | "raw" | "values" | "section" | "members" | "control")
        && matches!(value, Value::Object(_) | Value::Array(_))
    {
        if !(f.applies)(o) {
            return Err("is no property of this object".into());
        }
        let mut n = o.clone();
        (f.set)(&mut n, &spawn::from_json(value))?;
        return Ok(Effect::Object(n));
    }
    let row = props::find(rows, key).ok_or("is no property of this object")?;
    let parsed = props::parse(&row.row, &text_of(value))?;
    props::set_object(s, o, &row.row.key, &parsed, cx)
}

async fn scene_set(ed: Arc<Editor>, mut p: SetParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    match (p.key.take(), p.value.take()) {
        (Some(k), Some(v)) => {
            p.set.get_or_insert_with(Map::new).insert(k, v);
        }
        (Some(k), None) => return Err(CmdError::invalid(format!("key={k} takes value"))),
        (None, Some(_)) => return Err(CmdError::invalid("value takes key: the property it is for")),
        (None, None) => {}
    }
    // the kinds' eyes and locks
    if let Some(classes) = kinds_of(p.classes.as_deref())? {
        if p.set.is_some()
            || p.name.is_some()
            || p.position.is_some()
            || p.rotation.is_some()
            || p.scale.is_some()
        {
            return Err(CmdError::invalid(
                "classes takes hidden and locked only: set objects' properties by objects",
            ));
        }
        if p.hidden.is_none() && p.locked.is_none() {
            return Err(CmdError::invalid("classes takes hidden or locked"));
        }
        for c in &classes {
            svc.store
                .send(SceneCmd::ClassView { class: *c, visible: p.hidden.map(|h| !h), locked: p.locked }, o)
                .await?;
        }
        let s = svc.store.snapshot();
        let kinds: Vec<Value> = classes
            .iter()
            .map(|c| {
                let t = s.tools.part(*c);
                json!({ "kind": c.key(), "hidden": !t.visible, "locked": t.locked })
            })
            .collect();
        return Ok(json!({ "kinds": kinds }));
    }
    if p.world {
        return set_world(&ed, &svc, p.set.unwrap_or_default(), o).await;
    }
    let snap = svc.store.snapshot();
    let ids = crate::edit::ids_of(&snap, p.objects.as_deref())?;
    if ids.is_empty() {
        return Err(CmdError::invalid("nothing is selected: name the objects"));
    }
    // the lock first: a locked object takes nothing else
    if let Some(l) = p.locked {
        svc.store.send(SceneCmd::Lock { ids: ids.clone(), locked: l }, o).await?;
    }
    if let Some(h) = p.hidden {
        svc.store.send(SceneCmd::Show { ids: ids.clone(), visible: !h }, o).await?;
    }
    if let Some(name) = p.name.as_deref() {
        let name = name.trim();
        let [id] = ids.as_slice() else {
            return Err(CmdError::invalid("name renames one object: name it in objects"));
        };
        if name.is_empty() {
            return Err(CmdError::invalid("a name cannot be empty"));
        }
        let s = svc.store.snapshot();
        if s.find_any(name).is_some_and(|x| x.id != *id) {
            return Err(CmdError::invalid(format!("'{name}' is the name of another object")));
        }
        svc.store.send(SceneCmd::Rename { id: *id, name: SmolStr::new(name) }, o).await?;
    }
    // transforms: one undo step, a group's objects move with it
    let mut items: Vec<(ObjId, TransformEdit)> = Vec::new();
    let check = |v: [f32; 3], what: &str| -> Result<Vec3, CmdError> {
        let v = Vec3::from_array(v);
        if v.is_finite() { Ok(v) } else { Err(CmdError::invalid(format!("{what} takes finite numbers"))) }
    };
    let s = svc.store.snapshot();
    for id in &ids {
        let Some(obj) = s.get(*id) else { continue };
        let caps = obj.class.caps();
        if let Some(v) = p.position {
            items.push((*id, TransformEdit::Position(check(v, "position")?)));
        }
        if let Some(v) = p.rotation {
            if !caps.rotates {
                return Err(CmdError::invalid(format!("{} does not turn: it takes no rotation", obj.name)));
            }
            let d = check(v, "rotation")?;
            let r = Vec3::new(
                values::radians(f64::from(d.x)),
                values::radians(f64::from(d.y)),
                values::radians(f64::from(d.z)),
            );
            items.push((*id, TransformEdit::Rotation(r)));
        }
        if let Some(v) = p.scale {
            if !caps.scales {
                return Err(CmdError::invalid(format!("{} takes no scale", obj.name)));
            }
            let v = check(v, "scale")?;
            if v.x.abs() < 1e-6 || v.y.abs() < 1e-6 || v.z.abs() < 1e-6 {
                return Err(CmdError::invalid("a scale of zero"));
            }
            let dynamic = match &obj.data {
                ObjData::SceneObject(d) => svc.library.cached(&d.reference).is_some_and(|l| l.is_dynamic()),
                _ => false,
            };
            if dynamic {
                return Err(CmdError::invalid(format!(
                    "{} is a dynamic object: it keeps its scale",
                    obj.name
                )));
            }
            items.push((*id, TransformEdit::Scale(v)));
        }
    }
    let ctx = prop_context(&ed).await;
    let mut objects: Vec<Object> = Vec::new();
    if let Some(set) = &p.set {
        let s = svc.store.snapshot();
        svc.prefetch_library(&ed, &s).await;
        let cx = PropCx { lib: Some(&svc.library), entity: &ctx };
        for id in &ids {
            let Some(obj) = s.get(*id) else { continue };
            // each key sees what the keys before it did
            let mut now: Object = (**obj).clone();
            for (key, value) in set {
                let mut value = value.clone();
                if key == "props.visual"
                    && let Value::String(v) = &value
                {
                    let v = visual_name(v);
                    if !v.is_empty() && !model_exists(&ed, &svc, &v).await {
                        return Err(CmdError::not_found(format!(
                            "{key}: there is no model meshes\\{v}.ogf in the game or the project"
                        )));
                    }
                    value = Value::String(v);
                }
                let rows = rows_of(&s, &svc, &now, &ctx);
                match set_key(&s, &now, key, &value, &rows, &cx)
                    .map_err(|e| CmdError::invalid(format!("{}: {key} {e}", now.name)))?
                {
                    Effect::Object(n) => now = n,
                    Effect::Objects(list) => {
                        for n in list {
                            if n.id == now.id {
                                now = n;
                            } else {
                                objects.retain(|x| x.id != n.id);
                                objects.push(n);
                            }
                        }
                    }
                    Effect::Transform(e) => items.push((*id, e)),
                    Effect::Nothing => {}
                }
            }
            if let ObjData::Spawn(d) = &now.data
                && d.order.kind == 2
                && d.order.route.is_empty()
            {
                return Err(CmdError::invalid(format!(
                    "{}: a patrol needs a route: a way path (order.route)",
                    now.name
                )));
            }
            if now != **obj {
                objects.retain(|x| x.id != now.id);
                objects.push(now);
            }
        }
    }
    if !items.is_empty() {
        svc.store
            .send(SceneCmd::Transform { items, members: true, label: "Transform".into(), merge: None }, o)
            .await?;
    }
    if !objects.is_empty() {
        let serial = svc.store.snapshot().serial;
        svc.store
            .send(SceneCmd::ReplaceObjects { objects, serial, label: "Set Properties".into() }, o)
            .await?;
    }
    let s = svc.store.snapshot();
    let keys: Option<Vec<String>> = p.set.as_ref().map(|m| m.keys().cloned().collect());
    let out: Vec<Value> = ids
        .iter()
        .filter_map(|id| s.get(*id))
        .map(|obj| {
            let mut b = brief(&s, obj);
            if let (Some(keys), Value::Object(m)) = (&keys, &mut b) {
                let rows = rows_of(&s, &svc, obj, &ctx);
                let vals: Map<String, Value> = keys
                    .iter()
                    .filter_map(|k| props::find(&rows, k).map(|r| (k.clone(), value_json(&r.row))))
                    .collect();
                m.insert("set".into(), Value::Object(vals));
            }
            b
        })
        .collect();
    Ok(json!({ "objects": out }))
}

async fn set_world(ed: &Arc<Editor>, svc: &Arc<SceneService>, set: Map<String, Value>, o: Origin) -> Reply {
    if set.is_empty() {
        return Err(CmdError::invalid("world takes set={key: value}: scene.get world=true lists the keys"));
    }
    for (key, value) in &set {
        let s = svc.store.snapshot();
        let field = format::SCENE_FIELDS.iter().find(|f| f.key == key.as_str());
        // a table of the scene file (random_placement, classes) is taken as a table
        let (level, tools) = match field {
            Some(f) if matches!(f.ty, fields::Ty::Table(_)) => {
                let mut n = (*s).clone();
                (f.set)(&mut n, &spawn::from_json(value))
                    .map_err(|e| CmdError::invalid(format!("{key} {e}")))?;
                let tools = ObjClass::ALL
                    .iter()
                    .filter(|c| n.tools.part(**c) != s.tools.part(**c))
                    .map(|c| n.tools.part(*c).clone())
                    .collect();
                (n.meta.level, tools)
            }
            _ => {
                let rows = props::merge(vec![props::world_rows(&s)]);
                let row = props::find(&rows, key)
                    .ok_or_else(|| CmdError::not_found(format!("{key} is no property of the level")))?;
                let parsed = props::parse(&row.row, &text_of(value))
                    .map_err(|e| CmdError::invalid(format!("{key} {e}")))?;
                props::set_world(&s, &row.row.key, &parsed)
                    .map_err(|e| CmdError::invalid(format!("{key} {e}")))?
            }
        };
        svc.store
            .send(
                SceneCmd::SetWorld {
                    level: Box::new(level),
                    tools,
                    serial: s.serial,
                    label: "Level Settings".into(),
                },
                o,
            )
            .await?;
    }
    let s = svc.store.snapshot();
    let ctx = prop_context(ed).await;
    let rows = selection_rows(&s, &svc.library, true, &ctx);
    let vals: Map<String, Value> = set
        .keys()
        .filter_map(|k| match props::find(&rows, k) {
            Some(r) => Some((k.clone(), value_json(&r.row))),
            None => format::SCENE_FIELDS
                .iter()
                .find(|f| f.key == k.as_str())
                .map(|f| (k.clone(), spawn::to_json(&(f.get)(&s)))),
        })
        .collect();
    Ok(json!({ "set": vals }))
}

// ---------------------------------------------------------------- choices

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ChoicesParams {
    /// The object (left out: the first selected one).
    #[serde(default)]
    object: Option<String>,
    /// The property's key.
    key: String,
    /// A property of the level instead of an object's.
    #[serde(default)]
    world: bool,
    /// Only items that hold this text (any case).
    #[serde(default)]
    query: Option<String>,
    /// At most this many (default 500).
    #[serde(default)]
    limit: Option<usize>,
}

/// What a pick list holds, as far as the scene's crate can read it without the window: the
/// library's and the game's lists the project hooks give. The Level tab's crate answers the
/// rest (textures, shaders, models, motions) by registering this command over this one.
async fn list_items(ed: &Arc<Editor>, s: &SceneState, list: &str) -> Option<Vec<String>> {
    if let Some(items) = props::scene_items(s, list) {
        return Some(items);
    }
    let e = ed.clone();
    let lister: fn(&Editor) -> Vec<String> = match list {
        "object" => {
            let svc = SceneService::of(ed).ok()?;
            return Some(svc.library.list().as_ref().clone());
        }
        "group" => |ed| library::list_groups(&gz_project::library_root(ed).join("rawdata").join("groups")),
        "particles" => particles,
        "lightanim" => light_anims,
        "spawn_section" | "spawnitem" => {
            return Some(
                crate::game::spawn_sections(ed).await.iter().map(|x| x.section.to_string()).collect(),
            );
        }
        _ => return None,
    };
    Some(ed.exec.io(move || lister(&e)).await)
}

/// `scene.choices` as the scene's crate answers it: `{key, label, type, list?, choices,
/// total}`. Public for the Level tab's crate, which adds the lists it reads.
pub async fn choices(
    ed: &Arc<Editor>,
    object: Option<&str>,
    key: &str,
    world: bool,
) -> Result<(Row, Option<Vec<String>>), CmdError> {
    let svc = service::need_scene(ed).await?;
    let s = svc.store.snapshot();
    svc.prefetch_library(ed, &s).await;
    let ctx = prop_context(ed).await;
    let rows = if world {
        selection_rows(&s, &svc.library, true, &ctx)
    } else {
        let o = match object {
            Some(n) => s.find_any(n).ok_or_else(|| CmdError::not_found(format!("no object is named {n}")))?,
            None => s
                .selected_in_order()
                .first()
                .and_then(|id| s.get(*id))
                .ok_or_else(|| CmdError::invalid("nothing is selected: name the object"))?,
        };
        rows_of(&s, &svc, o, &ctx)
    };
    let row = props::find(&rows, key)
        .ok_or_else(|| CmdError::not_found(format!("{key} is no property here (scene.get lists them)")))?
        .row
        .clone();
    let items = match &row.kind {
        Kind::Token(items) => Some(items.iter().map(|(n, _)| n.to_string()).collect()),
        Kind::Button(c) => Some(c.iter().map(ToString::to_string).collect()),
        Kind::Bool | Kind::Flag => Some(vec!["true".to_owned(), "false".to_owned()]),
        Kind::Choose(c) => list_items(ed, &s, &c.list).await,
        _ => {
            return Err(CmdError::invalid(format!(
                "{key} picks from no list: it takes a {}",
                kind_word(&row.kind)
            )));
        }
    };
    Ok((row, items))
}

/// The answer of `scene.choices` for a row and its items.
#[must_use]
pub fn choices_reply(row: &Row, items: &[String], query: Option<&str>, limit: Option<usize>) -> Value {
    let q = query.unwrap_or("").trim().to_lowercase();
    let found: Vec<&String> =
        items.iter().filter(|i| q.is_empty() || i.to_lowercase().contains(&q)).collect();
    let mut m = Map::new();
    m.insert("key".into(), json!(row.key));
    m.insert("label".into(), json!(row.label));
    m.insert("type".into(), json!(kind_word(&row.kind)));
    if let Kind::Choose(c) = &row.kind {
        m.insert("list".into(), json!(c.list.as_str()));
        if c.max > 1 {
            m.insert("several".into(), json!(true));
        }
    }
    m.insert("value".into(), value_json(row));
    m.insert("total".into(), json!(found.len()));
    m.insert("choices".into(), json!(found.into_iter().take(limit.unwrap_or(500)).collect::<Vec<_>>()));
    Value::Object(m)
}

async fn scene_choices(ed: Arc<Editor>, p: ChoicesParams, _o: Origin) -> Reply {
    let (row, items) = choices(&ed, p.object.as_deref(), &p.key, p.world).await?;
    let Some(items) = items else {
        let list = match &row.kind {
            Kind::Choose(c) => c.list.to_string(),
            _ => String::new(),
        };
        return Err(CmdError::new(
            "no_window",
            format!("the list '{list}' is read by the Level tab: run the editor with its window"),
        ));
    };
    Ok(choices_reply(&row, &items, p.query.as_deref(), p.limit))
}

// ---------------------------------------------------------------- selection, mode

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct SelectParams {
    /// These objects by name.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Every visible, unlocked object of these kinds.
    #[serde(default)]
    classes: Option<Vec<String>>,
    /// Every visible, unlocked object.
    #[serde(default)]
    all: bool,
    /// Nothing.
    #[serde(default)]
    none: bool,
    /// What is selected is not, and the rest is (of `classes` when given).
    #[serde(default)]
    invert: bool,
    /// Add to the selection there is instead of replacing it.
    #[serde(default)]
    add: bool,
    /// Take these out of the selection.
    #[serde(default)]
    remove: bool,
}

async fn scene_select(ed: Arc<Editor>, p: SelectParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let classes = kinds_of(p.classes.as_deref())?;
    let mut missing: Vec<String> = Vec::new();
    if p.none {
        svc.store.send(SceneCmd::SelectAll { classes: None, select: false, invert: false }, o).await?;
    } else if p.invert || p.all || (classes.is_some() && p.objects.is_none()) {
        if !p.add && !p.invert {
            svc.store.send(SceneCmd::SelectAll { classes: None, select: false, invert: false }, o).await?;
        }
        svc.store.send(SceneCmd::SelectAll { classes, select: !p.remove, invert: p.invert }, o).await?;
    } else if let Some(names) = &p.objects {
        let s = svc.store.snapshot();
        let mut ids = Vec::new();
        for n in names {
            match s.find_any(n.trim()) {
                Some(x) if classes.as_ref().is_none_or(|l| l.contains(&x.class)) => ids.push(x.id),
                Some(_) => {}
                None => missing.push(n.clone()),
            }
        }
        let mode = if p.remove {
            SelectMode::Remove
        } else if p.add {
            SelectMode::Add
        } else {
            SelectMode::Set
        };
        svc.store.send(SceneCmd::Select { ids, mode }, o).await?;
    } else {
        return Err(CmdError::invalid("scene.select takes objects, classes, all, none or invert"));
    }
    let s = svc.store.snapshot();
    let names: Vec<&str> =
        s.selected_in_order().iter().filter_map(|id| s.get(*id)).map(|x| x.name.as_str()).take(100).collect();
    let mut out = json!({ "selected": s.selection.len(), "objects": names });
    if !missing.is_empty()
        && let Value::Object(m) = &mut out
    {
        m.insert("missing".into(), json!(missing));
    }
    Ok(out)
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ModeParams {
    /// Left out: says the mode there is.
    #[serde(default)]
    mode: Option<SceneMode>,
    /// What a click of Place mode places (`object`, `light`, `spawn`, `way`, ...).
    #[serde(default)]
    kind: Option<String>,
}

async fn scene_mode(ed: Arc<Editor>, p: ModeParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let kind = kinds_of(p.kind.as_ref().map(std::slice::from_ref))?.and_then(|k| k.first().copied());
    match (p.mode, kind) {
        (Some(SceneMode::Place) | None, Some(k)) => {
            svc.store.send(SceneCmd::Arm(k), o).await?;
        }
        (Some(m), Some(_)) => {
            return Err(CmdError::invalid(format!("kind goes with mode=place, not mode={}", m.word())));
        }
        (Some(m), None) => {
            svc.store.send(SceneCmd::SetMode(m), o).await?;
        }
        (None, None) => {}
    }
    let s = svc.store.snapshot();
    Ok(json!({ "mode": s.tools.mode.word(), "kind": s.tools.placing.key() }))
}

// ---------------------------------------------------------------- recovery

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum RecoverAction {
    /// What there is.
    #[default]
    Get,
    /// A snapshot of the open scene now.
    Snapshot,
    /// The newest snapshot becomes the open scene (unsaved: Save writes it to its file).
    Restore,
    /// The snapshots are deleted.
    Discard,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct RecoverParams {
    #[serde(default)]
    action: RecoverAction,
}

async fn scene_recover(ed: Arc<Editor>, p: RecoverParams, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let rec = svc.recovery.clone();
    let root = svc.root.clone();
    ed.exec.io(move || rec.bind(&root)).await;
    let linked = service::game_linked(&ed).await;
    let result: Result<(), String> = match p.action {
        RecoverAction::Get => Ok(()),
        RecoverAction::Snapshot => {
            if !svc.store.snapshot().valid {
                return Err(CmdError::new("no_scene", "no scene is open"));
            }
            match svc.recovery.can_snapshot(linked) {
                Ok(()) => service::snapshot(&ed, &svc).await,
                Err(e) => Err(e),
            }
        }
        RecoverAction::Restore => restore(&ed, &svc, linked, o).await,
        RecoverAction::Discard => {
            let rec = svc.recovery.clone();
            ed.exec.io(move || rec.discard().map(|_| ())).await
        }
    };
    let mut status = svc.recovery.status(svc.store.is_dirty(), linked).to_json();
    // the project's paths as answers write them
    if let Value::Object(m) = &mut status {
        for k in ["source", "snapshot"] {
            if let Some(Value::String(p)) = m.get_mut(k)
                && !p.is_empty()
            {
                *p = wire(&svc.root, Path::new(p.as_str()));
            }
        }
    }
    match result {
        Ok(()) => Ok(status),
        Err(e) => Err(CmdError::new("failed", e).with_data(status)),
    }
}

/// The newest snapshot read as a scene and put up, unsaved, under the file it was taken of.
async fn restore(ed: &Arc<Editor>, svc: &Arc<SceneService>, linked: bool, o: Origin) -> Result<(), String> {
    let (_, dir, source) = svc.recovery.begin_restore(svc.store.is_dirty(), linked)?;
    let settings = crate::game::settings(ed).await;
    let root = svc.root.clone();
    let d = dir.clone();
    let opened = ed
        .exec
        .io(move || {
            let file = crate::recovery::Snapshot::scene_file(&d).ok_or("the snapshot holds no scene")?;
            service::open_files(
                &root,
                &file,
                settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync)),
            )
        })
        .await;
    let outcome = match opened {
        Ok(mut opened) => {
            // the source is written project-relative
            opened.state.meta.file = (!source.is_empty()).then(|| {
                let p = PathBuf::from(source.replace('/', "\\"));
                if p.is_absolute() { p } else { svc.root.join(p) }
            });
            opened.state.meta.imported = None;
            svc.prefetch_library(ed, &opened.state).await;
            match svc.store.send(SceneCmd::Replace(Box::new(opened.state), ReplaceKind::Recovered), o).await {
                Ok(_) => {
                    svc.forget_files();
                    svc.set_opened();
                    svc.store.send(SceneCmd::Touch, o).await.map(|_| ()).map_err(|e| e.message)
                }
                Err(e) => Err(e.message),
            }
        }
        Err(e) => Err(e),
    };
    svc.recovery.end_restore(outcome.clone());
    if outcome.is_ok() {
        ed.log.info("The scene was restored from its recovery snapshot. Save writes it to its file.");
    }
    outcome
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_visual_is_named_as_the_configs_name_it() {
        assert_eq!(visual_name("meshes/actors/stalker_bandit_1.ogf"), "actors\\stalker_bandit_1");
        assert_eq!(visual_name("actors\\neutral"), "actors\\neutral");
    }

    #[test]
    fn kinds_take_keys_and_labels() {
        let k = kinds_of(Some(&["light".to_owned(), "Glow".to_owned()])).unwrap().unwrap();
        assert_eq!(k, vec![ObjClass::Light, ObjClass::Glow]);
        assert!(kinds_of(Some(&["lamp".to_owned()])).is_err());
        assert_eq!(kinds_of(None).unwrap(), None);
    }

    #[test]
    fn json_values_read_as_the_rows_read_text() {
        assert_eq!(text_of(&json!([1, 2.5, 3])), "1, 2.5, 3");
        assert_eq!(text_of(&json!(true)), "true");
        assert_eq!(text_of(&json!("spot")), "spot");
    }

    #[test]
    fn the_camera_answers_in_degrees() {
        let mut s = SceneState::default();
        s.meta.camera_hpb = Vec3::new(std::f32::consts::FRAC_PI_2, 0.0, 0.0);
        assert_eq!(camera_json(&s)["heading"], json!(90.0));
    }
}
