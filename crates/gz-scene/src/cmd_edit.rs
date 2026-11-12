//! The commands of the scene's edits through files and of its parts: `scene_edit` (cut, copy,
//! paste, duplicate), `selection_file` (save, open), `rename_objects`, `level_parts`,
//! `scene_new`. The C++ reached these through `exec_command` only; these are their own wire.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply, Value, json};

use crate::apply::{SceneCmd, SceneReply};
use crate::class::ObjClass;
use crate::edit;
use crate::service::{self, SceneService};

const AREA: &str = "gz-scene";

pub(crate) fn register(reg: &mut Registry) {
    reg.add(AREA, "scene_edit", scene_edit);
    reg.add(AREA, "selection_file", selection_file);
    reg.add(AREA, "rename_objects", rename_objects);
    reg.add(AREA, "level_parts", level_parts);
    reg.add(AREA, "scene_new", scene_new);
    reg.add(AREA, "scene_summary", scene_summary);
    reg.add(AREA, "compiler_errors", compiler_errors);
    reg.add(AREA, "object_motion", object_motion);
    reg.add(AREA, "object_tool", object_tool);
    reg.add(AREA, "spawn_attach", spawn_attach);
    reg.add(AREA, "group_tool", group_tool);
}

/// The class a command works on: `class=` by its name, else the current tool's.
pub(crate) fn class_arg(a: &Args, svc: &SceneService) -> Result<ObjClass, CmdError> {
    match a.text("class") {
        Some(name) => ObjClass::by_name(name)
            .or_else(|| ObjClass::ALL.iter().copied().find(|c| c.desc().eq_ignore_ascii_case(name)))
            .ok_or_else(|| CmdError::invalid(format!("unknown class '{name}'"))),
        None => Ok(svc.store.snapshot().tools.target),
    }
}

fn path_json(p: &std::path::Path) -> Value {
    Value::String(p.display().to_string().replace('\\', "/"))
}

async fn scene_edit(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.need("action")?.to_ascii_lowercase();
    let class = class_arg(&a, &svc)?;
    match action.as_str() {
        "copy" => {
            let file = edit::copy(&ed, &svc, class).await?;
            let n = svc
                .store
                .snapshot()
                .of_class(class)
                .filter(|x| svc.store.snapshot().is_selected(x.id))
                .count();
            Ok(json!({ "action": "copy", "class": class.name(), "copied": n, "file": path_json(&file) }))
        }
        "cut" => {
            let (file, n) = edit::cut(&ed, &svc, class, o).await?;
            Ok(json!({ "action": "cut", "class": class.name(), "removed": n, "file": path_json(&file) }))
        }
        "paste" => {
            let added = edit::paste(&ed, &svc, o).await?;
            Ok(json!({ "action": "paste", "added": added.objects, "warnings": added.warnings }))
        }
        "duplicate" => {
            let added = edit::duplicate(&ed, &svc, class, o).await?;
            Ok(
                json!({ "action": "duplicate", "class": class.name(), "added": added.objects, "warnings": added.warnings }),
            )
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}' (cut, copy, paste, duplicate)"))),
    }
}

async fn selection_file(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.need("action")?.to_ascii_lowercase();
    let file = edit::maps_file(&svc, a.need("file")?, "level");
    match action.as_str() {
        "save" => {
            let class = class_arg(&a, &svc)?;
            edit::save_selection(&ed, &svc, Some(class), file.clone()).await?;
            Ok(json!({ "action": "save", "class": class.name(), "file": path_json(&file) }))
        }
        "open" => {
            let added = edit::load_selection(&ed, &svc, file.clone(), "Open Selection", o).await?;
            Ok(
                json!({ "action": "open", "file": path_json(&file), "added": added.objects, "warnings": added.warnings }),
            )
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}' (save, open)"))),
    }
}

async fn rename_objects(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let class = class_arg(&a, &svc)?;
    let snap = svc.store.snapshot();
    if !snap.of_class(class).any(|x| snap.is_selected(x.id)) {
        ed.log.info("Nothing is selected: no object is renamed.");
        return Ok(json!({ "class": class.name(), "renamed": 0 }));
    }
    let SceneReply::Count(n) = svc.store.send(SceneCmd::MultiRename(class), o).await? else {
        return Err(CmdError::internal("rename_objects"));
    };
    ed.log.info(format!("Objects renamed: {n}."));
    Ok(json!({ "class": class.name(), "renamed": n }))
}

/// The scene with unsaved changes is saved first (`save=true`), its changes lost
/// (`discard=true`), or the command answers `unsaved`.
pub(crate) async fn settle_scene(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    a: &Args,
) -> Result<(), CmdError> {
    if !svc.store.is_dirty() || a.flag("discard") {
        return Ok(());
    }
    if a.flag("save") {
        let file = svc.store.snapshot().meta.file.clone().ok_or_else(|| {
            CmdError::invalid("scene has no file yet - pass 'file' or Save As in the editor")
        })?;
        return service::save_scene(ed, svc, file).await.map_err(CmdError::io);
    }
    let snap = svc.store.snapshot();
    let name = snap.meta.name();
    Err(CmdError::unsaved(json!([{
        "kind": "scene",
        "name": if name.is_empty() { "Untitled scene".to_owned() } else { name },
        "path": snap.meta.file.as_ref().map(|f| f.display().to_string()),
    }])))
}

fn parts_json(svc: &SceneService) -> Value {
    let snap = svc.store.snapshot();
    Value::Array(
        ObjClass::ALL
            .iter()
            .map(|c| {
                let t = snap.tools.part(*c);
                json!({
                    "class": c.name(),
                    "title": c.desc(),
                    "enabled": t.enabled,
                    "readonly": t.readonly,
                    "visible": t.visible,
                    "objects": if c.is_object_tool() { snap.count(*c) } else { 0 },
                    "can_disable": *c != ObjClass::SceneObject,
                })
            })
            .collect(),
    )
}

/// `level_parts`: every class's part of the scene; `class=` with `enabled=` loads or unloads it
/// (`CommandEnableTarget`; unloading unsaved work needs `save=` / `discard=`), `readonly=` locks
/// it (`flForceReadonly`: its objects cannot be changed and its part is not saved over).
async fn level_parts(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    if a.has("class") {
        let class = class_arg(&a, &svc)?;
        if let Some(r) = a.bool("readonly") {
            svc.store.send(SceneCmd::ToolFlags { class, visible: None, readonly: Some(r) }, o).await?;
        }
        if let Some(on) = a.bool("enabled") {
            let now = svc.store.snapshot().tools.part(class).enabled;
            if on != now {
                if class == ObjClass::SceneObject && !on {
                    return Err(CmdError::invalid("the scene objects' part is always loaded"));
                }
                if !on {
                    settle_scene(&ed, &svc, &a).await?;
                }
                edit::set_part_enabled(&ed, &svc, class, on, o).await?;
            }
        }
    }
    Ok(json!({ "parts": parts_json(&svc) }))
}

/// `scene_new`: `CommandClear` (unsaved work needs `save=` / `discard=`).
async fn scene_new(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    if let Err(e) = svc.recovery.can_replace_scene() {
        return Err(CmdError::invalid(e));
    }
    settle_scene(&ed, &svc, &a).await?;
    edit::new_scene(&ed, &svc, o).await?;
    Ok(json!({ "scene": "" }))
}

// ---- reports: the compiler's error list, the summary ----------------------------------------

/// The summary of the last collect (`s_summary`): the Scene > Summary Info sheet reads it.
#[derive(Default)]
pub struct SummaryStore {
    pub summary: parking_lot::Mutex<Option<Arc<crate::summary::Summary>>>,
}

impl SummaryStore {
    #[must_use]
    pub fn of(session: &gz_editor::ProjectSession) -> Arc<SummaryStore> {
        session.services.get_or_init(SummaryStore::default)
    }
}

/// Where a texture's description and its animation are: the project's and the editor's
/// `rawdata\textures\<name>.thm`, `gamedata\textures\<name>.seq`.
fn texture_info(roots: &[std::path::PathBuf], name: &str) -> Option<(gz_fmt_tex::TextureParams, u32)> {
    let rel = name.replace('/', "\\");
    let thm = roots
        .iter()
        .find_map(|r| std::fs::read(r.join("rawdata\\textures").join(format!("{rel}.thm"))).ok())?;
    let params = gz_fmt_tex::read_texture_thm(&thm).ok()?.params;
    // `MemoryUsage`: the `.seq` holds a line per frame after its first
    let frames = roots
        .iter()
        .find_map(|r| std::fs::read(r.join("gamedata\\textures").join(format!("{rel}.seq"))).ok())
        .map_or(1, |b| {
            let text = String::from_utf8_lossy(&b);
            let n = text.lines().count().saturating_sub(1) as u32;
            n.max(1)
        });
    Some((params, frames))
}

/// `scene_summary`: action collect (clear, collect and keep; the default) | show (what was
/// collected, as the sheet's rows) | export (file=, the C++ text) | clear.
async fn scene_summary(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let session = ed.project()?;
    let store = SummaryStore::of(&session);
    let action = a.str("action").unwrap_or("collect").to_ascii_lowercase();
    let rows_json = |s: &crate::summary::Summary| {
        Value::Array(s.rows().into_iter().map(|(k, v)| json!({ "key": k, "value": v })).collect())
    };
    match action.as_str() {
        "clear" => {
            *store.summary.lock() = None;
            Ok(json!({ "cleared": true }))
        }
        "collect" => {
            let snap = svc.store.snapshot();
            if !snap.valid {
                return Err(CmdError::new("no_scene", "no scene"));
            }
            svc.prefetch_library(&ed, &snap).await;
            let lib = svc.library.clone();
            let roots = vec![svc.root.clone(), svc.editor_root.clone()];
            let (summary, log) = ed
                .exec
                .io(move || {
                    let mut s = crate::summary::collect(&snap, &lib);
                    let log = s.prepare(&lib, &|n| texture_info(&roots, n));
                    (s, log)
                })
                .await;
            for l in log {
                ed.log.warn(l);
            }
            let s = Arc::new(summary);
            *store.summary.lock() = Some(s.clone());
            Ok(json!({ "rows": rows_json(&s) }))
        }
        "show" => {
            let s = store
                .summary
                .lock()
                .clone()
                .ok_or_else(|| CmdError::invalid("nothing collected yet: action=collect first"))?;
            Ok(json!({ "rows": rows_json(&s) }))
        }
        "export" => {
            let s = store
                .summary
                .lock()
                .clone()
                .ok_or_else(|| CmdError::invalid("nothing collected yet: action=collect first"))?;
            let file = edit::maps_file(&svc, a.need("file")?, "txt");
            let text = s.export();
            edit::writable(&ed, &file)?;
            let f2 = file.clone();
            ed.exec
                .io(move || {
                    if let Some(d) = f2.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    std::fs::write(&f2, text.as_bytes())
                })
                .await
                .map_err(|e| CmdError::io(format!("Export failed. ({e})")))?;
            Ok(json!({ "file": path_json(&file) }))
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}' (collect, show, export, clear)"))),
    }
}

/// `compiler_errors`: action import (file=) | export (file=) | clear | state: the compiler's
/// error list drawn in the viewport (Build > Import / Export / Clear Error List).
async fn compiler_errors(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.str("action").unwrap_or("state").to_ascii_lowercase();
    let state = |d: &crate::debug::DebugDraw| json!({ "points": d.points.len(), "lines": d.lines.len(), "faces": d.wire_faces.len() });
    // `$logs$` of the project
    let logs = svc.root.join("logs");
    let file_of = |a: &Args| -> Result<std::path::PathBuf, CmdError> {
        let f = std::path::PathBuf::from(a.need("file")?.replace('/', "\\"));
        Ok(if f.is_absolute() { f } else { logs.join(f) })
    };
    match action.as_str() {
        "state" => Ok(state(&svc.store.snapshot().tools.debug)),
        "clear" => {
            let d = Arc::new(crate::debug::DebugDraw::default());
            svc.store.send(SceneCmd::DebugDraw(d.clone()), o).await?;
            Ok(state(&d))
        }
        "import" => {
            let file = file_of(&a)?;
            let shown = file.display().to_string();
            let bytes = ed.exec.io(move || std::fs::read(&file)).await.map_err(|e| {
                CmdError::not_found(format!("the error list could not be read: {shown} ({e})"))
            })?;
            let d =
                crate::debug::read_compiler_error(&bytes).map_err(|e| CmdError::invalid(e.to_string()))?;
            let d = Arc::new(d);
            svc.store.send(SceneCmd::DebugDraw(d.clone()), o).await?;
            Ok(state(&d))
        }
        "export" => {
            let file = file_of(&a)?;
            let bytes = crate::debug::write_compiler_error(&svc.store.snapshot().tools.debug);
            edit::writable(&ed, &file)?;
            let f2 = file.clone();
            ed.exec
                .io(move || {
                    if let Some(d) = f2.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    std::fs::write(&f2, &bytes)
                })
                .await
                .map_err(|e| CmdError::io(format!("the error list could not be written: {e}")))?;
            Ok(json!({ "file": path_json(&file) }))
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}' (import, export, clear, state)"))),
    }
}

// ---- the motion of a dynamic scene object --------------------------------------------------------

/// A motion playing (`SAnimParams::bPlay`): which object, from which time, since when. The
/// viewport poses the object from it on its own frames; Pause makes the pose a scene change.
#[derive(Clone, Copy, Debug)]
pub struct Play {
    pub id: crate::model::ObjId,
    pub from: f32,
    pub started: std::time::Instant,
}

/// The session's playback.
#[derive(Default)]
pub struct MotionPlayback {
    pub playing: parking_lot::Mutex<Option<Play>>,
}

impl MotionPlayback {
    #[must_use]
    pub fn of(session: &gz_editor::ProjectSession) -> Arc<MotionPlayback> {
        session.services.get_or_init(MotionPlayback::default)
    }

    /// The time of a playing motion now (`SAnimParams::Update` with loop: past the end it wraps
    /// to the start).
    #[must_use]
    pub fn time_of(play: &Play, min_t: f32, max_t: f32) -> f32 {
        let mut t = play.from + play.started.elapsed().as_secs_f32();
        if t > max_t {
            let len = max_t - min_t;
            if len > 0.0 {
                let k = ((t - min_t) / len).floor();
                t -= k * len;
            } else {
                t = max_t;
            }
        }
        t
    }
}

/// Sends a motion edit of one object as its scene command.
async fn send_motion(
    svc: &SceneService,
    o: &crate::model::Object,
    edit: &crate::objmotion::MotionEdit,
    origin: Origin,
) -> Result<(), CmdError> {
    let e = crate::objmotion::apply(o, edit).map_err(CmdError::invalid)?;
    svc.store
        .send(
            SceneCmd::Motion {
                id: o.id,
                motion: e.motion,
                flags: e.flags,
                pose: e.pose,
                label: e.label.to_owned(),
                merge: e.merge.then(|| gz_base::SmolStr::new_static("motion_frame")),
            },
            origin,
        )
        .await?;
    Ok(())
}

/// `object_motion name= action=`: state (the default) | motionable (on=) | import / export
/// (file=, an `.anm`; relative to the project's `gamedata\anims`) | add_key | delete_key |
/// scale_keys (from, to, scale) | normalize_keys (from, to, speed) | clamp | first | prev_key |
/// prev_frame | play | pause | next_frame | next_key | last | auto_key (on=) | camera_view (on=)
/// | range (min, max) | current (t) | key_time (t).
async fn object_motion(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    use crate::objmotion::MotionEdit as E;
    let svc = SceneService::of(&ed)?;
    let session = ed.project()?;
    let name = a.need("name")?;
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).ok_or_else(|| CmdError::not_found("object not found"))?.clone();
    let playback = MotionPlayback::of(&session);
    let action = a.str("action").unwrap_or("state").to_ascii_lowercase();
    let f = |k: &str| a.f32(k).ok_or_else(|| CmdError::invalid(format!("missing '{k}' argument")));
    let anims = svc.root.join("gamedata").join("anims");
    let file_of = |a: &Args| -> Result<std::path::PathBuf, CmdError> {
        let p = std::path::PathBuf::from(a.need("file")?.replace('/', "\\"));
        let p = if p.is_absolute() { p } else { anims.join(p) };
        Ok(if p.extension().is_none() { p.with_extension("anm") } else { p })
    };
    let edit = match action.as_str() {
        "state" => None,
        "motionable" => Some(E::Motionable(a.bool("on").unwrap_or(true))),
        "import" => {
            let file = file_of(&a)?;
            let shown = file.display().to_string();
            let bytes =
                ed.exec.io(move || std::fs::read(&file)).await.map_err(|e| {
                    CmdError::not_found(format!("the motion could not be read: {shown} ({e})"))
                })?;
            let m = gz_fmt_object::read_anm(&bytes).map_err(|e| CmdError::invalid(e.to_string()))?;
            Some(E::Import(Box::new(m)))
        }
        "export" => {
            let file = file_of(&a)?;
            let m = obj
                .motion
                .as_ref()
                .and_then(|r| r.model())
                .ok_or_else(|| CmdError::invalid(format!("'{}' has no motion to export", obj.name)))?;
            let bytes = gz_fmt_object::write_anm(&m);
            edit::writable(&ed, &file)?;
            let f2 = file.clone();
            ed.exec
                .io(move || {
                    if let Some(d) = f2.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    std::fs::write(&f2, &bytes)
                })
                .await
                .map_err(|e| CmdError::io(format!("!Can't save object motion: {} ({e})", file.display())))?;
            None
        }
        "add_key" => Some(E::AddKey),
        "delete_key" => Some(E::DeleteKey),
        "scale_keys" => Some(E::ScaleKeys { from: f("from")?, to: f("to")?, scale: f("scale")? }),
        "normalize_keys" => Some(E::NormalizeKeys { from: f("from")?, to: f("to")?, speed: f("speed")? }),
        "clamp" => Some(E::Clamp),
        "first" => Some(E::FirstFrame),
        "prev_key" => Some(E::PreviousKey),
        "prev_frame" => Some(E::PreviousFrame),
        "next_frame" => Some(E::NextFrame),
        "next_key" => Some(E::NextKey),
        "last" => Some(E::LastFrame),
        "auto_key" => Some(E::AutoKey(a.bool("on").unwrap_or(true))),
        "camera_view" => Some(E::CameraView(a.bool("on").unwrap_or(true))),
        "range" => Some(E::Range { min_t: a.f32("min"), max_t: a.f32("max") }),
        "current" => Some(E::Current(f("t")?)),
        "key_time" => Some(E::KeyTime(f("t")?)),
        "play" => {
            let raw = obj
                .motion
                .as_ref()
                .ok_or_else(|| CmdError::invalid(format!("'{}' has no motion", obj.name)))?;
            let m = raw
                .model()
                .ok_or_else(|| CmdError::invalid("the motion has a version this editor does not read"))?;
            let p = crate::objmotion::params(&m, raw);
            *playback.playing.lock() =
                Some(Play { id: obj.id, from: p.t_current, started: std::time::Instant::now() });
            ed.exec.wake().ping();
            None
        }
        "pause" => {
            let play = playback.playing.lock().take();
            match play {
                Some(play) if play.id == obj.id => {
                    let raw = obj.motion.as_ref().ok_or_else(|| CmdError::invalid("no motion"))?;
                    let m = raw.model().ok_or_else(|| {
                        CmdError::invalid("the motion has a version this editor does not read")
                    })?;
                    let p = crate::objmotion::params(&m, raw);
                    Some(E::At(MotionPlayback::time_of(&play, p.min_t, p.max_t)))
                }
                _ => None,
            }
        }
        other => return Err(CmdError::invalid(format!("unknown action '{other}'"))),
    };
    if let Some(edit) = edit {
        // a control stops the playback, as `bPlay = FALSE`
        if !matches!(edit, E::AutoKey(_) | E::CameraView(_)) {
            let mut p = playback.playing.lock();
            if p.is_some_and(|p| p.id == obj.id) {
                *p = None;
            }
        }
        send_motion(&svc, &obj, &edit, o).await?;
    }
    let snap = svc.store.snapshot();
    let o2 = snap.get(obj.id).ok_or_else(|| CmdError::not_found("object not found"))?;
    let playing = playback.playing.lock().is_some_and(|p| p.id == o2.id);
    let state = match o2.motion.as_ref().and_then(|r| r.model().map(|m| (m, r.clone()))) {
        Some((m, raw)) if o2.co_flags() & crate::model::co::MOTION != 0 => {
            let p = crate::objmotion::params(&m, &raw);
            json!({
                "motionable": true,
                "keys": m.envelopes[0].keys.len(),
                "length": crate::objmotion::length(&m).0,
                "start": p.min_t,
                "end": p.max_t,
                "current": p.t_current,
                "auto_key": o2.co_flags() & crate::objmotion::AUTO_KEY != 0,
                "camera_view": o2.co_flags() & crate::objmotion::CAMERA_VIEW != 0,
                "playing": playing,
            })
        }
        _ => json!({ "motionable": false }),
    };
    Ok(json!({ "name": o2.name.as_str(), "motion": state }))
}

// ---- the Object, Spawn and Group tools' commands -----------------------------------------------

/// Three numbers: `[x, y, z]` or `"x, y, z"`; one number stands for all three.
fn vec3_arg(a: &Args, key: &str) -> Result<Option<crate::math::Vec3>, CmdError> {
    let bad = || CmdError::invalid(format!("'{key}' must be three numbers"));
    let Some(v) = a.get(key) else { return Ok(None) };
    let nums: Vec<f32> = match v {
        Value::Array(xs) => {
            xs.iter().map(|x| x.as_f64().map(|f| f as f32)).collect::<Option<_>>().ok_or_else(bad)?
        }
        Value::String(s) => {
            s.split(',').map(|p| p.trim().parse::<f32>().ok()).collect::<Option<_>>().ok_or_else(bad)?
        }
        Value::Number(n) => vec![n.as_f64().ok_or_else(bad)? as f32],
        _ => return Err(bad()),
    };
    match nums.as_slice() {
        [x] if x.is_finite() => Ok(Some(crate::math::Vec3::splat(*x))),
        [x, y, z] if x.is_finite() && y.is_finite() && z.is_finite() => {
            Ok(Some(crate::math::Vec3::new(*x, *y, *z)))
        }
        _ => Err(bad()),
    }
}

/// The Object tool's Randomize Added Objects and its Random Settings, as `object_tool
/// action=random` answers them (rotations in degrees).
fn random_json(part: &crate::tools::ToolPart) -> Value {
    use crate::tools::{
        FL_APPEND_RANDOM, FL_APPEND_RANDOM_PROPORTIONAL, FL_APPEND_RANDOM_ROTATION, FL_APPEND_RANDOM_SCALE,
    };
    let r = |v: f32| (f64::from(v) * 1000.0).round() / 1000.0;
    let v = |x: crate::math::Vec3| json!([r(x.x), r(x.y), r(x.z)]);
    let deg = |x: crate::math::Vec3| json!([r(x.x.to_degrees()), r(x.y.to_degrees()), r(x.z.to_degrees())]);
    let crate::tools::ToolOpts::Objects(o) = &part.opts else { return json!({ "on": false }) };
    json!({
        "on": part.flags & FL_APPEND_RANDOM != 0,
        "objects": o.objects.iter().map(|x| x.as_str()).collect::<Vec<_>>(),
        "scale": part.flags & FL_APPEND_RANDOM_SCALE != 0,
        "proportional": part.flags & FL_APPEND_RANDOM_PROPORTIONAL != 0,
        "min_scale": v(o.min_scale),
        "max_scale": v(o.max_scale),
        "rotate": part.flags & FL_APPEND_RANDOM_ROTATION != 0,
        "min_rotation": deg(o.min_rotation),
        "max_rotation": deg(o.max_rotation),
    })
}

/// `object_tool action=random [on=] [objects=] [scale=] [proportional=] [min_scale=] [max_scale=]
/// [rotate=] [min_rotation=] [max_rotation=]`: Randomize Added Objects and the Random Append
/// sheet (`FillAppendRandomProperties`). What is left out keeps its value; rotations are degrees;
/// `objects` (a list, at most 32) is what a click of Place picks from. One undo step.
async fn object_random(ed: &Arc<Editor>, svc: &SceneService, a: &Args, o: Origin) -> Reply {
    use crate::tools::{
        FL_APPEND_RANDOM, FL_APPEND_RANDOM_PROPORTIONAL, FL_APPEND_RANDOM_ROTATION, FL_APPEND_RANDOM_SCALE,
    };
    let snap = svc.store.snapshot();
    let mut t = snap.tools.part(ObjClass::SceneObject).clone();
    let before = t.clone();
    for (key, bit) in [
        ("on", FL_APPEND_RANDOM),
        ("scale", FL_APPEND_RANDOM_SCALE),
        ("proportional", FL_APPEND_RANDOM_PROPORTIONAL),
        ("rotate", FL_APPEND_RANDOM_ROTATION),
    ] {
        if let Some(on) = a.bool(key) {
            t.flags = if on { t.flags | bit } else { t.flags & !bit };
        }
    }
    let objects: Option<Vec<gz_base::SmolStr>> = match a.get("objects") {
        None => None,
        Some(Value::Array(xs)) => {
            Some(xs.iter().filter_map(Value::as_str).map(gz_base::SmolStr::new).collect())
        }
        Some(Value::String(s)) => Some(
            s.split([',', ';']).map(str::trim).filter(|x| !x.is_empty()).map(gz_base::SmolStr::new).collect(),
        ),
        Some(_) => return Err(CmdError::invalid("'objects' must be a list of library objects")),
    };
    let (min_s, max_s) = (vec3_arg(a, "min_scale")?, vec3_arg(a, "max_scale")?);
    let (min_r, max_r) = (vec3_arg(a, "min_rotation")?, vec3_arg(a, "max_rotation")?);
    if let crate::tools::ToolOpts::Objects(opts) = &mut t.opts {
        if let Some(list) = objects {
            // every one has to load: a click would log "Can't load reference object" otherwise
            for name in &list {
                let (lib, key) = (svc.library.clone(), crate::library::normalize(name));
                if ed.exec.io(move || lib.load(&key)).await.is_err() {
                    return Err(CmdError::not_found(format!("no library object '{name}'")));
                }
            }
            opts.objects = list.into_iter().take(32).collect();
        }
        let rad = |v: crate::math::Vec3| {
            crate::math::Vec3::new(v.x.to_radians(), v.y.to_radians(), v.z.to_radians())
        };
        if let Some(v) = min_s {
            opts.min_scale = v;
        }
        if let Some(v) = max_s {
            opts.max_scale = v;
        }
        if let Some(v) = min_r {
            opts.min_rotation = rad(v);
        }
        if let Some(v) = max_r {
            opts.max_rotation = rad(v);
        }
    }
    if t != before {
        svc.store
            .send(
                SceneCmd::SetWorld {
                    level: Box::new(snap.meta.level.clone()),
                    tools: vec![t],
                    serial: snap.serial,
                    label: "Random Settings".into(),
                },
                o,
            )
            .await?;
    }
    Ok(json!({ "random": random_json(svc.store.snapshot().state.tools.part(ObjClass::SceneObject)) }))
}

/// `object_tool action=reset_surfaces [selected=true]`: `UIObjectTool::ClearSurface` - the
/// visible scene objects (the selected ones with `selected`) wear their reference's surfaces
/// again. One undo step. `action=random`: see [`object_random`].
async fn object_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.need("action")?.to_ascii_lowercase();
    if action == "random" {
        return object_random(&ed, &svc, &a, o).await;
    }
    if action != "reset_surfaces" {
        return Err(CmdError::invalid(format!("unknown action '{action}' (reset_surfaces, random)")));
    }
    let selected = a.bool("selected").unwrap_or(true);
    let snap = svc.store.snapshot();
    let mut objects = Vec::new();
    for obj in snap.of_class(ObjClass::SceneObject) {
        if !obj.flags.visible || (selected && !snap.is_selected(obj.id)) {
            continue;
        }
        let crate::model::ObjData::SceneObject(d) = &obj.data else { continue };
        if d.flags & crate::objects::SO_USE_SURFACE == 0 && d.surfaces.is_empty() {
            continue;
        }
        let mut n = (**obj).clone();
        if let crate::model::ObjData::SceneObject(d) = &mut n.data {
            d.flags &= !crate::objects::SO_USE_SURFACE;
            d.surfaces.clear();
        }
        objects.push(n);
    }
    let n = objects.len();
    if n > 0 {
        svc.store
            .send(
                SceneCmd::ReplaceObjects { objects, serial: snap.serial, label: "Reset Surfaces".into() },
                o,
            )
            .await?;
    }
    Ok(json!({ "reset": n }))
}

/// `spawn_attach action=attach spawn= object=` | `detach`: `CSpawnPoint::AttachObject` - a shape
/// becomes the selected spawn point's own (the point takes its place, rotation and scale); detach
/// gives the selected points' shapes back to the scene. One undo step.
async fn spawn_attach(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.need("action")?.to_ascii_lowercase();
    let snap = svc.store.snapshot();
    match action.as_str() {
        "attach" => {
            let spawn = match a.text("spawn") {
                Some(n) => snap.find(ObjClass::SpawnPoint, &n.to_lowercase()).cloned(),
                None => {
                    let sel: Vec<_> =
                        snap.of_class(ObjClass::SpawnPoint).filter(|x| snap.is_selected(x.id)).collect();
                    if sel.len() != 1 {
                        return Err(CmdError::invalid("Select one shape."));
                    }
                    sel.first().map(|x| (*x).clone())
                }
            }
            .ok_or_else(|| CmdError::not_found("spawn point not found"))?;
            let shape = snap
                .find_relaxed(a.need("object")?)
                .cloned()
                .ok_or_else(|| CmdError::not_found("object not found"))?;
            svc.store.send(SceneCmd::SpawnAttach { spawn: spawn.id, object: Some(shape.id) }, o).await?;
            Ok(json!({ "attached": shape.name.as_str(), "spawn": spawn.name.as_str() }))
        }
        "detach" => {
            let ids: Vec<_> = snap
                .of_class(ObjClass::SpawnPoint)
                .filter(|x| snap.is_selected(x.id))
                .filter(|x| matches!(&x.data, crate::model::ObjData::Spawn(s) if s.attached.is_some()))
                .map(|x| x.id)
                .collect();
            for id in &ids {
                svc.store.send(SceneCmd::SpawnAttach { spawn: *id, object: None }, o).await?;
            }
            Ok(json!({ "detached": ids.len() }))
        }
        // Attach on Click: the next click in the viewport attaches what it hits
        "on_click" => {
            let on = a.bool("on").unwrap_or(true);
            let s = crate::objtools::set_state(&svc, o, |st| st.spawn_attach = on).await?;
            Ok(json!({ "attach_on_click": s.spawn_attach }))
        }
        other => Err(CmdError::invalid(format!("unknown action '{other}' (attach, detach, on_click)"))),
    }
}

/// `group_tool action=center_pivot` (`CenterToGroup`: every group's pivot to the middle of its
/// objects) | `align_pivot object=` (the selected groups' pivot to that object of theirs) |
/// `save_as file=` (the selected group as a library `.group`, `SaveSelectedObject`) |
/// `reload` (`ReloadRefsSelectedObject`: the selected groups made again from their library
/// `.group`).
async fn group_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.need("action")?.to_ascii_lowercase();
    let snap = svc.store.snapshot();
    match action.as_str() {
        // the panel's Group and Ungroup buttons
        "group" => {
            if snap.selected_in_order().is_empty() {
                return Err(CmdError::invalid("No object(s) selected."));
            }
            let name = match a.text("name") {
                Some(n) => n.to_owned(),
                None => snap.gen_name(Some("group")),
            };
            let reply = svc.store.send(SceneCmd::Group { name: name.as_str().into() }, o).await?;
            let id = match reply {
                SceneReply::Id(id) => id.0,
                _ => 0,
            };
            let snap = svc.store.snapshot();
            let members = match snap.get(crate::model::ObjId(id)).map(|g| &g.data) {
                Some(crate::model::ObjData::Group(g)) => g.members.len(),
                _ => 0,
            };
            Ok(json!({ "object": { "name": name, "id": id }, "members": members }))
        }
        "ungroup" => {
            let n = snap.of_class(ObjClass::Group).filter(|g| snap.is_selected(g.id)).count();
            if n == 0 {
                return Err(CmdError::invalid("No group selected."));
            }
            svc.store.send(SceneCmd::Ungroup, o).await?;
            Ok(json!({ "ungrouped": n }))
        }
        "center_pivot" | "align_pivot" => {
            let target = a.text("object").map(str::to_lowercase);
            if action == "align_pivot" && target.is_none() {
                return Err(CmdError::invalid("missing 'object' argument"));
            }
            let mut objects = Vec::new();
            for g in snap.of_class(ObjClass::Group) {
                // oracle: Center to Group takes every group of the tool, selected or not
                if action == "align_pivot" && !snap.is_selected(g.id) {
                    continue;
                }
                let crate::model::ObjData::Group(gd) = &g.data else { continue };
                let members: Vec<_> = gd.members.iter().filter_map(|m| snap.get(*m)).collect();
                if members.is_empty() {
                    continue;
                }
                let mut n = (**g).clone();
                if action == "center_pivot" {
                    let sum = members.iter().fold(crate::math::Vec3::ZERO, |acc, m| acc + m.position());
                    n.xform.position = sum / members.len() as f32;
                    n.xform.rotation = crate::math::Vec3::ZERO;
                } else if let Some(m) = members.iter().find(|m| Some(m.name.to_string()) == target) {
                    n.xform.position = m.position();
                    n.xform.rotation = m.rotation();
                } else {
                    continue;
                }
                objects.push(n);
            }
            if objects.is_empty() {
                ed.log.error("Nothing selected.");
                return Ok(json!({ "changed": 0 }));
            }
            let count = objects.len();
            svc.store
                .send(
                    SceneCmd::ReplaceObjects { objects, serial: snap.serial, label: "Group Pivot".into() },
                    o,
                )
                .await?;
            Ok(json!({ "changed": count }))
        }
        "save_as" => {
            let sel: Vec<_> = snap.of_class(ObjClass::Group).filter(|g| snap.is_selected(g.id)).collect();
            let Some(g) = sel.first() else {
                return Err(CmdError::invalid("No object(s) selected."));
            };
            let raw = std::path::PathBuf::from(a.need("file")?.replace('/', "\\"));
            let groups = svc.root.join("rawdata").join("groups");
            let file = if raw.is_absolute() { raw } else { groups.join(raw) };
            let file = if file.extension().is_none() { file.with_extension("group") } else { file };
            // `CGroupObject::SaveStream`: the object's body, without the scene's class wrapper
            let mut w = gz_chunk::Writer::new();
            crate::objects::write_stream(&mut w, g, &snap, 0)
                .map_err(|e| CmdError::invalid(e.to_string()))?;
            let bytes = w.into_bytes();
            let body = gz_chunk::Reader::new(&bytes)
                .find_chunk(crate::objects::CHUNK_OBJECT_BODY)
                .map(|(b, _)| b.to_vec())
                .ok_or_else(|| CmdError::internal("the group wrote no body"))?;
            edit::writable(&ed, &file)?;
            let f2 = file.clone();
            ed.exec
                .io(move || {
                    if let Some(d) = f2.parent() {
                        std::fs::create_dir_all(d)?;
                    }
                    std::fs::write(&f2, &body)
                })
                .await
                .map_err(|e| CmdError::io(format!("Cant write file [{}] ({e})", file.display())))?;
            Ok(json!({ "file": path_json(&file) }))
        }
        "reload" => {
            let sel: Vec<_> =
                snap.of_class(ObjClass::Group).filter(|g| snap.is_selected(g.id)).cloned().collect();
            if sel.is_empty() {
                ed.log.error("Nothing selected.");
                return Ok(json!({ "reloaded": 0 }));
            }
            let mut groups = Vec::new();
            for g in &sel {
                let crate::model::ObjData::Group(gd) = &g.data else { continue };
                if gd.reference.is_empty() {
                    return Err(CmdError::invalid(format!("ERROR: '{}' - has empty reference.", g.name)));
                }
                let loaded = library_group(&ed, &svc, &gd.reference).await?;
                if loaded.members.len() != gd.members.len() && !gd.members.is_empty() {
                    ed.log.error(format!("Not all objects synchronised correctly ({}).", g.name));
                }
                groups.push((g.id, Box::new(loaded)));
            }
            let n = groups.len();
            svc.store.send(SceneCmd::ReloadGroups { groups }, o).await?;
            Ok(json!({ "reloaded": n }))
        }
        "place" => {
            let reference = a.need("ref")?.replace('/', "\\");
            let mut at = crate::math::Transform::at(crate::cmd::in_front(&snap));
            for (i, k) in ["x", "y", "z"].iter().enumerate() {
                if let Some(v) = a.f32(k) {
                    at.position[i] = v;
                }
            }
            if !at.position.is_finite() {
                return Err(CmdError::invalid("non-finite position"));
            }
            let mut loaded = library_group(&ed, &svc, &reference).await?;
            // `GenObjectName` from the library name, the reference the one asked for
            loaded.obj.name = snap.gen_name(Some(&reference)).into();
            if let crate::model::ObjData::Group(g) = &mut loaded.obj.data {
                g.reference = reference.as_str().into();
            }
            let name = loaded.obj.name.to_string();
            let reply = svc.store.send(SceneCmd::PlaceGroup { loaded: Box::new(loaded), at }, o).await?;
            let id = match reply {
                crate::apply::SceneReply::Id(id) => id.0,
                _ => 0,
            };
            Ok(json!({ "object": { "name": name, "id": id } }))
        }
        other => Err(CmdError::invalid(format!(
            "unknown action '{other}' (group, ungroup, center_pivot, align_pivot, save_as, reload, place)"
        ))),
    }
}

/// A group body's chunks with the reference chunk (`GROUPOBJ_CHUNK_REFERENCE`, 4) naming
/// `reference`; a body that does not parse as chunks is given back as it is.
fn with_reference(body: &[u8], reference: &str) -> Vec<u8> {
    const CHUNK_REFERENCE: u32 = 4;
    let chunk = |out: &mut Vec<u8>| {
        out.extend_from_slice(&CHUNK_REFERENCE.to_le_bytes());
        out.extend_from_slice(&u32::try_from(reference.len() + 1).unwrap_or(u32::MAX).to_le_bytes());
        out.extend_from_slice(reference.as_bytes());
        out.push(0);
    };
    let mut out = Vec::with_capacity(body.len() + reference.len() + 9);
    let mut at = 0;
    let mut written = false;
    while at + 8 <= body.len() {
        let id = u32::from_le_bytes([body[at], body[at + 1], body[at + 2], body[at + 3]]);
        let size = u32::from_le_bytes([body[at + 4], body[at + 5], body[at + 6], body[at + 7]]) as usize;
        let Some(end) = (at + 8).checked_add(size).filter(|e| *e <= body.len()) else {
            return body.to_vec();
        };
        if id != CHUNK_REFERENCE {
            out.extend_from_slice(&body[at..end]);
        } else if !written {
            chunk(&mut out);
            written = true;
        }
        at = end;
    }
    if at != body.len() {
        return body.to_vec();
    }
    if !written {
        chunk(&mut out);
    }
    out
}

/// A library group (`$groups$\<reference>.group`, the project's first) read as the scene reads a
/// group: `CGroupObject::LoadStream` of `UpdateReference`.
async fn library_group(
    ed: &Arc<Editor>,
    svc: &SceneService,
    reference: &str,
) -> Result<crate::objects::Loaded, CmdError> {
    let rel = format!("{}.group", reference.replace('/', "\\"));
    let files = [
        svc.root.join("rawdata").join("groups").join(&rel),
        ed.paths.root.join("rawdata").join("groups").join(&rel),
    ];
    let shown = files[0].display().to_string();
    let body = ed
        .exec
        .io(move || files.iter().find_map(|f| std::fs::read(f).ok()))
        .await
        .ok_or_else(|| CmdError::invalid(format!("ERROR: Can't open group file: '{shown}'.")))?;
    // the file is `CGroupObject::SaveStream`: the body without the class wrapper, its reference
    // chunk empty - `UpdateReference` sets the name it was opened by
    let mut w = gz_chunk::Writer::new();
    w.open_chunk(crate::objects::CHUNK_OBJECT_CLASS);
    w.w_u32(ObjClass::Group.id());
    w.close_chunk();
    w.open_chunk(crate::objects::CHUNK_OBJECT_BODY);
    w.w(&with_reference(&body, reference));
    w.close_chunk();
    let bytes = w.into_bytes();
    let settings = crate::game::settings(ed).await;
    let mut cx = crate::objects::ReadCx::new(settings.as_deref().map(|s| s as &dyn gz_entities::Settings));
    let loaded = crate::objects::read_stream(&gz_chunk::Reader::new(&bytes), &mut cx)
        .map_err(|e| CmdError::invalid(format!("Can't load group '{reference}': {e}")))?;
    for w in cx.warnings {
        ed.log.warn(w);
    }
    if !matches!(loaded.obj.data, crate::model::ObjData::Group(_)) {
        return Err(CmdError::invalid(format!("Can't load group '{reference}': it holds no objects.")));
    }
    Ok(loaded)
}

#[cfg(test)]
mod tests {
    use super::with_reference;

    fn chunk(id: u32, body: &[u8]) -> Vec<u8> {
        let mut v = id.to_le_bytes().to_vec();
        v.extend_from_slice(&u32::try_from(body.len()).unwrap().to_le_bytes());
        v.extend_from_slice(body);
        v
    }

    #[test]
    fn a_library_group_gets_the_reference_it_was_opened_by() {
        let body = [chunk(0, &[0x11, 0]), chunk(4, &[0]), chunk(1, &[1, 2, 3])].concat();
        let want = [chunk(0, &[0x11, 0]), chunk(4, b"doors\0"), chunk(1, &[1, 2, 3])].concat();
        assert_eq!(with_reference(&body, "doors"), want);
        // no reference chunk: one is added; a torn body is left as it is
        let bare = chunk(0, &[0x11, 0]);
        assert_eq!(with_reference(&bare, "a"), [bare.clone(), chunk(4, b"a\0")].concat());
        assert_eq!(with_reference(&bare[..5], "a"), bare[..5].to_vec());
    }
}
