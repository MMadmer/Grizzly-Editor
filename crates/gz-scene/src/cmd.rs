//! The scene's commands over MCP; the wire is `LevelMcpScene.cpp`, `LevelMcpProps.cpp` and
//! `GrizzlyMCP.cpp`'s `open_scene`.

use std::path::PathBuf;
use std::sync::Arc;

use gz_base::{Origin, SmolStr};
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply, Value, json};
use gz_entities::{Entity, PropContext};
use gz_project::{AssetLists, ProjectHooks};

use crate::apply::{ReplaceKind, SceneCmd, SceneReply, SelectMode, TransformEdit};
use crate::class::ObjClass;
use crate::library::{self, normalize};
use crate::math::{Vec3, Transform};
use crate::model::{ObjData, Object, SceneState};
use crate::objects::{
    ENVMOD_CHOOSE_NAME, PLAYERSTART_CHOOSE_NAME, PointKind, RPOINT_CHOOSE_NAME, SceneObjectData, ShapeData, ShapeDef, SpawnData, SpawnEntity,
    WayData, XmsOrder,
};
use crate::pick::ray_scene_objects;
use crate::props::{self, Effect, PropCx};
use crate::service::{self, SceneService};
use crate::snap::{DROP_REACH, SnapAction, drop_to_ground};

const AREA: &str = "gz-scene";

/// Registers the scene's commands.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "open_scene", open_scene);
    reg.add(AREA, "save_scene", save_scene);
    reg.add(AREA, "scene_info", |ed, _a, _o| async move { with_scene(&ed, |s, svc| scene_info(s, svc, false)) });
    reg.add(AREA, "scene_stats", |ed, _a, _o| async move { with_scene(&ed, |s, svc| scene_info(s, svc, true)) });
    reg.add(AREA, "scene_tree", |ed, a, _o| async move { with_scene(&ed, |s, _| Ok(scene_tree(s, &a))) });
    reg.add(AREA, "scene_load_status", |ed, _a, _o| async move { scene_load_status(&ed) });
    reg.add(AREA, "list_objects", |ed, a, _o| async move { with_scene(&ed, |s, _| Ok(list_objects(s, &a))) });
    reg.add(AREA, "object_info", |ed, a, _o| async move {
        with_scene(&ed, |s, _| {
            let name = a.str("name").ok_or_else(|| CmdError::invalid("missing 'name' argument"))?;
            let o = s.find_relaxed(name).ok_or_else(|| CmdError::not_found("object not found"))?;
            Ok(json!({ "object": object_json(s, o) }))
        })
    });
    reg.add(AREA, "selection", |ed, _a, _o| async move {
        with_scene(&ed, |s, _| {
            let list: Vec<Value> = s.selected_in_order().iter().filter_map(|id| s.get(*id)).map(|o| object_json(s, o)).collect();
            Ok(json!({ "selection": list }))
        })
    });
    reg.add(AREA, "select_objects", select_objects);
    reg.add(AREA, "deselect_all", |ed, _a, o| async move {
        let svc = SceneService::of(&ed)?;
        svc.store.send(SceneCmd::SelectClass { class: None, select: false }, o).await?;
        Ok(json!({}))
    });
    reg.add(AREA, "delete_selected", |ed, _a, o| async move {
        let svc = SceneService::of(&ed)?;
        let SceneReply::Count(n) = svc.store.send(SceneCmd::DeleteSelected, o).await? else { return Ok(json!({ "removed": 0 })) };
        Ok(json!({ "removed": n }))
    });
    reg.add(AREA, "rename_object", rename_object);
    reg.add(AREA, "object_set_position", |ed, a, o| set_transform(ed, a, o, 0));
    reg.add(AREA, "object_set_rotation", |ed, a, o| set_transform(ed, a, o, 1));
    reg.add(AREA, "object_set_scale", |ed, a, o| set_transform(ed, a, o, 2));
    reg.add(AREA, "place_object", place_object);
    reg.add(AREA, "place_spawn", place_spawn);
    reg.add(AREA, "place_way", place_way);
    reg.add(AREA, "spawn_order", spawn_order);
    reg.add(AREA, "spawn_set_visual", spawn_set_visual);
    reg.add(AREA, "spawn_set_fixed_bones", spawn_set_fixed_bones);
    reg.add(AREA, "mark_authored", |ed, a, o| async move {
        let svc = SceneService::of(&ed)?;
        let on = a.bool("authored").unwrap_or(true);
        let SceneReply::Count(n) = svc.store.send(SceneCmd::MarkAuthored(on), o).await? else { return Err(CmdError::internal("mark_authored")) };
        Ok(json!({ "changed": n, "authored": on }))
    });
    reg.add(AREA, "mark_removed", |ed, a, o| async move {
        let svc = SceneService::of(&ed)?;
        let on = a.bool("removed").unwrap_or(true);
        let SceneReply::Counts(n, own) = svc.store.send(SceneCmd::MarkRemoved(on), o).await? else { return Err(CmdError::internal("mark_removed")) };
        Ok(json!({ "changed": n, "skipped_authored": own, "removed": on }))
    });
    reg.add(AREA, "drop_objects", drop_objects);
    reg.add(AREA, "snap_objects", snap_objects);
    reg.add(AREA, "scan_walls", scan_walls);
    reg.add(AREA, "probe_vertical", probe_vertical);
    reg.add(AREA, "undo", |ed, _a, _o| undo_redo(ed, true));
    reg.add(AREA, "redo", |ed, _a, _o| undo_redo(ed, false));
    reg.add(AREA, "recovery", recovery);
    reg.add(AREA, "property_inspector", property_inspector);
    reg.add(AREA, "property_set", property_set);
    reg.add(AREA, "property_choose", property_choose);
}

/// Hooks the scene into the project's life and `list_assets`.
pub fn install(editor: &Editor) {
    let lists = AssetLists::of(editor);
    lists.provide("Objects", |ed| {
        if let Some(session) = ed.project_opt()
            && let Some(svc) = session.services.get::<SceneService>()
        {
            return svc.library.list().as_ref().clone();
        }
        // no project: the editor's own library, listed once
        let lib = ed.services.get_or_init(|| EditorLibrary(library::Library::new(vec![ed.paths.root.join("rawdata").join("objects")])));
        lib.0.list().as_ref().clone()
    });
    lists.provide("Groups", |ed| library::list_groups(&ed.paths.root.join("rawdata").join("groups")));
    let hooks = ProjectHooks::of(editor);
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
                    ed.log.warn(format!("the last scene did not open: {e}"));
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

fn with_scene(ed: &Arc<Editor>, f: impl FnOnce(&SceneState, &SceneService) -> Reply) -> Reply {
    let svc = SceneService::of(ed)?;
    let snap = svc.store.snapshot();
    if !snap.valid {
        return Err(CmdError::new("no_scene", "no scene"));
    }
    f(&snap, &svc)
}

/// `%.3f` and friends: a number cut to as many decimals as the C++ prints.
fn r(v: f32, decimals: i32) -> f64 {
    let p = 10f64.powi(decimals);
    (f64::from(v) * p).round() / p
}

fn v3(v: Vec3, d: i32) -> Value {
    json!([r(v.x, d), r(v.y, d), r(v.z, d)])
}

/// `AppendObjectJson`.
fn object_json(s: &SceneState, o: &Object) -> Value {
    json!({
        "name": o.name.as_str(),
        "class": o.class.name(),
        "selected": s.is_selected(o.id),
        "visible": o.flags.visible,
        "position": v3(o.position(), 3),
        "rotation": v3(o.rotation(), 4),
        "scale": v3(o.scale(), 3),
    })
}

fn respond_object(s: &SceneState, o: &Object) -> Value {
    json!({ "object": object_json(s, o) })
}

/// `m_LastFileName`: the C++ lower-cases the name it loads and saves.
fn scene_path(s: &SceneState) -> String {
    s.meta.file.as_ref().map(|f| f.display().to_string().to_lowercase()).unwrap_or_default()
}

/// `Fvector::setHP`: the direction a heading and a pitch look in.
fn direction(hpb: Vec3) -> Vec3 {
    let (sh, ch) = hpb.x.sin_cos();
    let (sp, cp) = hpb.y.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

fn camera_json(s: &SceneState) -> Value {
    json!({
        "position": v3(s.meta.camera_pos, 3),
        "hpb": v3(s.meta.camera_hpb, 4),
        "direction": v3(direction(s.meta.camera_hpb), 3),
    })
}

fn tools_json(s: &SceneState) -> Value {
    Value::Array(ObjClass::ALL.iter().map(|c| json!({ "class": c.name(), "objects": if c.is_object_tool() { s.count(*c) } else { 0 } })).collect())
}

fn scene_info(s: &SceneState, svc: &SceneService, stats: bool) -> Reply {
    let modified = svc.store.is_dirty();
    if !stats {
        return Ok(json!({ "scene": scene_path(s), "modified": modified, "tools": tools_json(s) }));
    }
    let selected = s.selection.len();
    Ok(json!({ "scene": scene_path(s), "modified": modified, "selected": selected, "camera": camera_json(s), "tools": tools_json(s) }))
}

fn scene_tree(s: &SceneState, a: &Args) -> Value {
    let class = a.str("class").unwrap_or("");
    let filter = a.str("filter").unwrap_or("").to_lowercase();
    let limit = a.i64("limit").unwrap_or(200).max(0) as usize;
    let mut classes: Vec<ObjClass> =
        ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool() && (class.is_empty() || c.name().eq_ignore_ascii_case(class)) && s.count(*c) > 0).collect();
    classes.sort_by_key(|c| c.name());
    let mut tree = Vec::new();
    for c in classes {
        let total = s.count(c);
        let matched: Vec<&Arc<Object>> = s.of_class(c).filter(|o| filter.is_empty() || o.name.to_lowercase().contains(&filter)).collect();
        if matched.is_empty() {
            continue;
        }
        let objects: Vec<Value> =
            matched.iter().take(limit).map(|o| json!({ "name": o.name.as_str(), "selected": s.is_selected(o.id), "visible": o.flags.visible })).collect();
        tree.push(json!({ "class": c.name(), "count": matched.len(), "total": total, "objects": objects }));
    }
    json!({ "tree": tree })
}

fn list_objects(s: &SceneState, a: &Args) -> Value {
    let class = a.str("class").unwrap_or("");
    let contains = a.str("name_contains").unwrap_or("");
    let limit = a.i64("limit").unwrap_or(100).max(0) as usize;
    let (mut shown, mut total) = (Vec::new(), 0usize);
    for c in ObjClass::ALL.iter().copied().filter(|c| c.is_object_tool() && (class.is_empty() || c.name().eq_ignore_ascii_case(class))) {
        for o in s.of_class(c) {
            if !contains.is_empty() && !o.name.contains(contains) {
                continue;
            }
            total += 1;
            if shown.len() < limit {
                shown.push(json!({ "name": o.name.as_str(), "class": c.name() }));
            }
        }
    }
    json!({ "objects": shown, "shown": shown.len(), "total": total })
}

fn scene_load_status(ed: &Arc<Editor>) -> Reply {
    let svc = SceneService::of(ed)?;
    let st = svc.load.lock().clone();
    let fields = json!({ "attempted": st.attempted, "loaded": st.loaded, "rollback_ok": st.rollback_ok, "path": st.path, "error": st.error });
    if st.attempted && !st.loaded {
        return Err(CmdError::new("load_failed", st.error.clone()).with_data(fields));
    }
    Ok(fields)
}

async fn open_scene(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let scene = a.str("scene").ok_or_else(|| CmdError::invalid("missing 'scene' argument"))?;
    let discard = match a.get("discard") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(CmdError::invalid("'discard' must be a boolean")),
    };
    let full = service::resolve_scene(&svc, scene)?;
    let check = full.clone();
    let exists = ed.exec.io(move || check.is_file()).await;
    if !exists {
        return Err(CmdError::not_found(format!("scene not found: {}", full.display())));
    }
    if svc.store.is_dirty() && !discard {
        return Err(CmdError::new("unsaved", "scene has unsaved changes; pass discard=true to replace it"));
    }
    match service::load_scene(&ed, &svc, full).await {
        Ok(()) => Ok(json!({})),
        Err(_) => scene_load_status(&ed),
    }
}

async fn save_scene(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let file = match a.text("file") {
        Some(f) => PathBuf::from(f.replace('/', "\\")),
        None => svc.store.snapshot().meta.file.clone().ok_or_else(|| CmdError::invalid("scene has no file yet - pass 'file' or Save As in the editor"))?,
    };
    let file = if file.is_absolute() { file } else { svc.maps_dir().join(file) };
    service::save_scene(&ed, &svc, file.clone()).await.map_err(CmdError::io)?;
    Ok(json!({ "file": file.display().to_string().replace('\\', "/") }))
}

/// A comma separated list as `strtok` cut it: blanks trimmed, empty items dropped.
fn names(text: &str) -> Vec<String> {
    text.split(',').map(|t| t.trim_matches(' ').to_owned()).filter(|t| !t.is_empty()).collect()
}

async fn select_objects(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let list = a.str("names").ok_or_else(|| CmdError::invalid("missing 'names' argument (comma-separated list)"))?;
    let snap = svc.store.snapshot();
    let mut ids = Vec::new();
    let mut missing = Vec::new();
    let mut first = None;
    for n in names(list) {
        match snap.find_relaxed(&n) {
            Some(obj) => {
                first.get_or_insert(obj.class);
                ids.push(obj.id);
            }
            None => missing.push(Value::String(n)),
        }
    }
    let found = ids.len();
    svc.store.send(SceneCmd::Select { ids, mode: SelectMode::Set }, o).await?;
    if let Some(c) = first {
        svc.store.send(SceneCmd::SetTarget(c), o).await?;
    }
    Ok(json!({ "selected": found, "missing": missing }))
}

async fn rename_object(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let (Some(name), Some(new_name)) = (a.str("name"), a.text("new_name")) else {
        return Err(CmdError::invalid("need 'name' and non-empty 'new_name'"));
    };
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).ok_or_else(|| CmdError::not_found("object not found"))?;
    let new_name = new_name.to_lowercase();
    if snap.find_any(&new_name).is_some() {
        return Err(CmdError::invalid("new name already used"));
    }
    let id = obj.id;
    svc.store.send(SceneCmd::Rename { id, name: SmolStr::new(&new_name) }, o).await?;
    let s = svc.store.snapshot();
    let obj = s.get(id).ok_or_else(|| CmdError::not_found("object not found"))?;
    Ok(respond_object(&s, obj))
}

async fn set_transform(ed: Arc<Editor>, a: Args, o: Origin, which: u8) -> Reply {
    let svc = SceneService::of(&ed)?;
    let name = a.str("name").ok_or_else(|| CmdError::invalid("missing 'name' argument"))?;
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).ok_or_else(|| CmdError::not_found("object not found"))?;
    let mut v = match which {
        1 => obj.rotation(),
        2 => obj.scale(),
        _ => obj.position(),
    };
    let mut any = false;
    for (i, k) in ["x", "y", "z"].iter().enumerate() {
        if let Some(x) = a.f32(k) {
            v[i] = x;
            any = true;
        }
    }
    if !any {
        return Err(CmdError::invalid("pass at least one of x,y,z"));
    }
    if !v.is_finite() {
        return Err(CmdError::invalid("non-finite component"));
    }
    let id = obj.id;
    // `CSceneObject::SetScale`: a dynamic object keeps its scale
    let dynamic = which == 2
        && match &obj.data {
            ObjData::SceneObject(d) => svc.library.cached(&d.reference).is_some_and(|l| l.is_dynamic()),
            _ => false,
        };
    if dynamic {
        ed.log.error(format!("Dynamic object {} - can't scale.", obj.name));
    } else {
        let (edit, label) = match which {
            1 => (TransformEdit::Rotation(v), "Rotate"),
            2 => (TransformEdit::Scale(v), "Scale"),
            _ => (TransformEdit::Position(v), "Move"),
        };
        svc.store.send(SceneCmd::Transform { items: vec![(id, edit)], members: false, label: label.into(), merge: None }, o).await?;
    }
    let s = svc.store.snapshot();
    let obj = s.get(id).ok_or_else(|| CmdError::not_found("object not found"))?;
    Ok(respond_object(&s, obj))
}

/// Where a placed object lands without a position: ten metres in front of the camera.
fn in_front(s: &SceneState) -> Vec3 {
    s.meta.camera_pos + direction(s.meta.camera_hpb) * 10.0
}

async fn drop_and_respond(ed: &Arc<Editor>, svc: &Arc<SceneService>, id: crate::model::ObjId, snap: bool, align: bool, o: Origin) -> Reply {
    let mut dropped = false;
    if snap {
        dropped = drop_ids(ed, svc, &[id], align, false, o).await? > 0;
    }
    let s = svc.store.snapshot();
    let obj = s.get(id).ok_or_else(|| CmdError::not_found("object not found"))?;
    let mut out = respond_object(&s, obj);
    if let Value::Object(m) = &mut out {
        m.insert("snapped_to_ground".into(), Value::Bool(dropped));
    }
    Ok(out)
}

async fn drop_ids(ed: &Arc<Editor>, svc: &Arc<SceneService>, ids: &[crate::model::ObjId], align: bool, line: bool, o: Origin) -> Result<usize, CmdError> {
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    let ids = ids.to_vec();
    let s = snap.clone();
    let landings = ed.exec.cpu(move || drop_to_ground(&s, &lib, &pick, &ids, align, line)).await;
    if landings.is_empty() {
        return Ok(0);
    }
    let n = landings.len();
    let items = landings.into_iter().map(|l| (l.id, TransformEdit::Land { position: l.position, rotation: l.rotation })).collect();
    svc.store.send(SceneCmd::Transform { items, members: false, label: "Drop to Ground".into(), merge: None }, o).await?;
    Ok(n)
}

async fn place_object(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let reference = a.text("ref").ok_or_else(|| CmdError::invalid("missing 'ref' argument (library object, e.g. props\\\\box_1)"))?.replace('/', "\\");
    let snap = svc.store.snapshot();
    let mut pos = in_front(&snap);
    for (i, k) in ["x", "y", "z"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            pos[i] = v;
        }
    }
    let mut rot = Vec3::ZERO;
    for (i, k) in ["rx", "ry", "rz"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            rot[i] = v;
        }
    }
    let mut scale = Vec3::ONE;
    let mut explicit_scale = false;
    for (i, k) in ["sx", "sy", "sz"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            scale[i] = v;
            explicit_scale = true;
        }
    }
    if let Some(u) = a.f32("scale") {
        scale = Vec3::splat(u);
        explicit_scale = true;
    }
    if !(pos.is_finite() && rot.is_finite() && scale.is_finite()) {
        return Err(CmdError::invalid("non-finite transform component"));
    }
    if explicit_scale && (scale.x.abs() < 1e-7 || scale.y.abs() < 1e-7 || scale.z.abs() < 1e-7) {
        return Err(CmdError::invalid("zero scale component"));
    }
    let name = match a.text("name") {
        Some(n) => {
            let n = n.to_lowercase();
            if snap.find_any(&n).is_some() {
                return Err(CmdError::invalid("object name already used"));
            }
            n
        }
        None => snap.gen_name(Some(&reference)),
    };
    let lib = svc.library.clone();
    let key = normalize(&reference);
    let loaded = ed.exec.io(move || lib.load(&key)).await;
    if loaded.is_err() {
        return Err(CmdError::invalid("can't load reference object (check the library path)"));
    }
    let mut obj = Object::new(ObjClass::SceneObject, &name, ObjData::SceneObject(SceneObjectData::library(&reference)));
    obj.xform = Transform { position: pos, rotation: rot, scale };
    let SceneReply::Id(id) = svc.store.send(SceneCmd::Place(Box::new(obj)), o).await? else { return Err(CmdError::internal("place")) };
    drop_and_respond(&ed, &svc, id, a.flag("snap_to_ground"), a.flag("align_normal"), o).await
}

async fn place_spawn(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let section = a.str("section").ok_or_else(|| CmdError::invalid("missing 'section' argument (see list_spawn_sections)"))?;
    let point = [PLAYERSTART_CHOOSE_NAME, ENVMOD_CHOOSE_NAME, RPOINT_CHOOSE_NAME]
        .into_iter()
        .find(|p| p.eq_ignore_ascii_case(section) || p[1..].eq_ignore_ascii_case(section));
    let snap = svc.store.snapshot();
    let mut pos = in_front(&snap);
    for (i, k) in ["x", "y", "z"].iter().enumerate() {
        if let Some(v) = a.f32(k) {
            pos[i] = v;
        }
    }
    if !pos.is_finite() {
        return Err(CmdError::invalid("non-finite position"));
    }
    let mut data = match point {
        Some(PLAYERSTART_CHOOSE_NAME) => SpawnData::of_kind(PointKind::PlayerStart),
        Some(ENVMOD_CHOOSE_NAME) => SpawnData::of_kind(PointKind::EnvMod),
        Some(_) => SpawnData::of_kind(PointKind::RPoint),
        None => {
            let refused = || {
                CmdError::invalid(
                    "no such spawn section in pSettings (see list_spawn_sections; player_start, env_mod and rpoint place the Player Start, an Environment Modifier and a Respawn Point)",
                )
            };
            let settings = crate::game::settings(&ed).await.ok_or_else(refused)?;
            let sec = section.to_owned();
            let made = ed
                .exec
                .io(move || -> Result<(Entity, Option<f32>, bool), CmdError> {
                    use gz_entities::Settings;
                    if !settings.section_exist(&sec) || !settings.line_exist(&sec, "$spawn") {
                        return Err(refused());
                    }
                    let e = Entity::create(&sec, settings.as_ref()).map_err(|_| CmdError::invalid("the spawn factory refused this section"))?;
                    let def = settings.r_float(&sec, "$def_sphere");
                    let needs = e.class_info().is_some_and(|c| c.class.has_shape());
                    Ok((e, def, needs))
                })
                .await?;
            let (entity, def_sphere, needs_shape) = made;
            let mut d = SpawnData::of_kind(PointKind::Spawn);
            d.entity = Some(SpawnEntity::Decoded(Arc::new(entity)));
            if def_sphere.is_some() || needs_shape {
                let mut shape = Object::new(ObjClass::Shape, "", ObjData::Shape(ShapeData { shapes: vec![ShapeDef::sphere(Vec3::ZERO, def_sphere.unwrap_or(5.0))], usage: 0 }));
                shape.xform.position = pos;
                d.attached = Some(Arc::new(shape));
            }
            d
        }
    };
    if let Some(a) = &mut data.attached {
        Arc::make_mut(a).xform.position = pos;
    }
    let pref = point.map_or(section, |p| &p[1..]);
    let name = snap.gen_name(Some(pref));
    let mut obj = Object::new(ObjClass::SpawnPoint, &name, ObjData::Spawn(Box::new(data)));
    obj.xform.position = pos;
    let SceneReply::Id(id) = svc.store.send(SceneCmd::Place(Box::new(obj)), o).await? else { return Err(CmdError::internal("place")) };
    drop_and_respond(&ed, &svc, id, a.flag("snap_to_ground"), false, o).await
}

/// `"%f , %f , %f"` items of a `;` list.
fn parse_points(text: &str) -> Result<Vec<Vec3>, String> {
    let mut out = Vec::new();
    for item in text.split(';').map(str::trim).filter(|i| !i.is_empty()) {
        let parts: Vec<Option<f32>> = item.split(',').map(|p| p.trim().parse::<f32>().ok()).collect();
        match parts.as_slice() {
            [Some(x), Some(y), Some(z)] if x.is_finite() && y.is_finite() && z.is_finite() => out.push(Vec3::new(*x, *y, *z)),
            _ => return Err(item.to_owned()),
        }
    }
    Ok(out)
}

async fn place_way(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let name = a.text("name").ok_or_else(|| CmdError::invalid("missing 'name' - the path's name, what orders and quests call it"))?.to_owned();
    let text = a.str("points").ok_or_else(|| CmdError::invalid("missing 'points': \"x,y,z; x,y,z; ...\""))?;
    let snap = svc.store.snapshot();
    if snap.find_any(&name).is_some() {
        return Err(CmdError::invalid("an object of that name exists already"));
    }
    let mut points = parse_points(text).map_err(|item| CmdError::invalid(format!("a point is not x,y,z: {item}")))?;
    if points.is_empty() {
        return Err(CmdError::invalid("a path needs at least one point"));
    }
    if a.bool("snap_to_ground").unwrap_or(true) {
        svc.prefetch_library(&ed, &snap).await;
        let (snap, pick) = svc.pick().await;
        let lib = svc.library.clone();
        points = ed
            .exec
            .cpu(move || {
                points
                    .into_iter()
                    .map(|p| match ray_scene_objects(&pick, &snap, &lib, p + Vec3::Y * 2.0, Vec3::NEG_Y, DROP_REACH, None) {
                        Some(h) if h.point.is_finite() => h.point,
                        _ => p,
                    })
                    .collect()
            })
            .await;
    }
    let n = points.len();
    let way = WayData::build(&points, a.bool("loop").unwrap_or(true));
    let obj = Object::new(ObjClass::Way, &name, ObjData::Way(way));
    let SceneReply::Id(id) = svc.store.send(SceneCmd::Place(Box::new(obj)), o).await? else { return Err(CmdError::internal("place")) };
    let s = svc.store.snapshot();
    let obj = s.get(id).ok_or_else(|| CmdError::not_found("object not found"))?;
    let mut out = respond_object(&s, obj);
    if let Value::Object(m) = &mut out {
        m.insert("points".into(), json!(n));
    }
    Ok(out)
}

async fn spawn_order(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let name = a.str("name").ok_or_else(|| CmdError::invalid("missing 'name' (a stalker placed with the Spawn tool)"))?;
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).ok_or_else(|| CmdError::not_found("no spawn object of that name"))?;
    let ObjData::Spawn(sp) = &obj.data else { return Err(CmdError::not_found("no spawn object of that name")) };
    if !sp.can_take_order() {
        return Err(CmdError::invalid("only a stalker takes an order - the game's schemes drive stalkers alone"));
    }
    let mut ord: XmsOrder = sp.order.clone();
    let mut changed = false;
    if let Some(t) = a.str("order") {
        let kind = XmsOrder::kind_from_key(t);
        if kind == 0 && !t.eq_ignore_ascii_case("none") {
            return Err(CmdError::invalid("order must be guard, patrol, wander, follow or none"));
        }
        ord.kind = kind;
        changed = true;
    }
    for (k, slot) in [("route", &mut ord.route), ("look", &mut ord.look), ("target", &mut ord.target)] {
        if let Some(t) = a.str(k) {
            *slot = SmolStr::new(t);
            changed = true;
        }
    }
    if let Some(v) = a.f32("radius") {
        ord.radius = v;
        changed = true;
    }
    if let Some(v) = a.f32("distance") {
        ord.distance = v;
        changed = true;
    }
    for (k, bit) in [("run", crate::objects::ORDER_RUN), ("hostile", crate::objects::ORDER_HOSTILE)] {
        if let Some(b) = a.bool(k) {
            ord.flags = if b { ord.flags | bit } else { ord.flags & !bit };
            changed = true;
        }
    }
    if ord.kind == 2 && ord.route.is_empty() {
        return Err(CmdError::invalid("a patrol needs a route: a way path of this level ('route')"));
    }
    let id = obj.id;
    let section = sp.section().unwrap_or("").to_owned();
    if changed {
        svc.store.send(SceneCmd::SetOrder { id, order: ord.clone() }, o).await?;
    }
    Ok(json!({
        "name": obj.name.as_str(),
        "section": section,
        "order": {
            "kind": XmsOrder::kind_key(ord.kind).unwrap_or("none"),
            "route": ord.route.as_str(),
            "look": ord.look.as_str(),
            "target": if ord.target.is_empty() { "actor" } else { ord.target.as_str() },
            "radius": g(ord.radius),
            "distance": g(ord.distance),
            "run": ord.flags & crate::objects::ORDER_RUN != 0,
            "hostile": ord.flags & crate::objects::ORDER_HOSTILE != 0,
        },
        "changed": changed,
        "custom_data": ord.section(),
    }))
}

/// `%g` as a JSON number.
fn g(v: f32) -> Value {
    crate::ltx::fmt_g(v).parse::<f64>().map_or(Value::Null, |n| json!(n))
}

/// A spawn's decoded entity and the row of it that ends in `suffix`.
fn spawn_entity_row(obj: &Object, suffix: &str) -> Result<(Arc<Entity>, String), CmdError> {
    let ObjData::Spawn(sp) = &obj.data else { return Err(CmdError::not_found("spawn object not found")) };
    let Some(SpawnEntity::Decoded(e)) = &sp.entity else { return Err(CmdError::invalid("object carries no spawn data")) };
    let path = e.properties(&PropContext::default()).into_iter().find(|p| p.path.ends_with(suffix)).map(|p| p.path);
    path.map(|p| (e.clone(), p)).ok_or_else(|| CmdError::invalid("this section has no visual"))
}

async fn replace_spawn(svc: &SceneService, obj: &Object, entity: Entity, label: &str, o: Origin, serial: u64) -> Result<(), CmdError> {
    let mut n = obj.clone();
    if let ObjData::Spawn(sp) = &mut n.data {
        sp.entity = Some(SpawnEntity::Decoded(Arc::new(entity)));
    }
    svc.store.send(SceneCmd::ReplaceObjects { objects: vec![n], serial, label: label.into() }, o).await?;
    Ok(())
}

async fn spawn_set_visual(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let (Some(name), Some(visual)) = (a.text("name"), a.text("visual")) else {
        return Err(CmdError::invalid("need 'name' and non-empty 'visual'"));
    };
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).filter(|x| x.class == ObjClass::SpawnPoint).ok_or_else(|| CmdError::not_found("spawn object not found"))?;
    let (entity, path) = spawn_entity_row(obj, "\\Model\\Visual")?;
    // normalized to what configs carry: meshes-relative, backslashes, no .ogf
    let mut v = visual.replace('/', "\\");
    if v.len() >= 7 && v[..7].eq_ignore_ascii_case("meshes\\") {
        v = v[7..].to_owned();
    }
    if let Some(dot) = v.rfind('.')
        && !v[dot..].contains('\\')
    {
        v.truncate(dot);
    }
    let rel = format!("meshes\\{v}.ogf");
    let reachable = {
        let in_project = svc.root.join("gamedata").join(&rel);
        let mut found = ed.exec.io(move || in_project.is_file()).await;
        if !found
            && let Ok(link) = gz_game::GameLink::of(&ed)
            && let Ok(files) = link.files().await
        {
            found = files.exists(&rel);
        }
        found
    };
    if !reachable {
        return Err(CmdError::not_found("mesh not found in the game or the project (meshes\\\\<visual>.ogf)"));
    }
    let mut e = (*entity).clone();
    e.set(&path, &gz_entities::PropValue::Text(v.clone()), &PropContext::default()).map_err(|err| CmdError::invalid(err.to_string()))?;
    replace_spawn(&svc, obj, e, "Visual", o, snap.serial).await?;
    Ok(json!({ "visual": v, "draws_nothing": false }))
}

async fn spawn_set_fixed_bones(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let name = a.text("name").ok_or_else(|| CmdError::invalid("need 'name'"))?;
    let bones = a.str("bones").unwrap_or("").to_owned();
    let snap = svc.store.snapshot();
    let obj = snap.find_relaxed(name).filter(|x| x.class == ObjClass::SpawnPoint).ok_or_else(|| CmdError::not_found("spawn object not found"))?;
    let physic = match &obj.data {
        ObjData::Spawn(sp) => sp.entity.as_ref().and_then(SpawnEntity::entity).and_then(Entity::class_info).is_some_and(|c| c.class == gz_entities::Class::ObjectPhysic),
        _ => false,
    };
    let entity = match &obj.data {
        ObjData::Spawn(sp) => match &sp.entity {
            Some(SpawnEntity::Decoded(e)) => e.clone(),
            _ => return Err(CmdError::not_found("spawn object not found")),
        },
        _ => return Err(CmdError::not_found("spawn object not found")),
    };
    if !physic {
        return Err(CmdError::invalid("this section has no fixed bones (not a physic object)"));
    }
    let path = entity
        .properties(&PropContext::default())
        .into_iter()
        .find(|p| p.path.ends_with("\\Model\\Fixed bones"))
        .map(|p| p.path)
        .ok_or_else(|| CmdError::invalid("this section has no fixed bones (not a physic object)"))?;
    let mut e = (*entity).clone();
    e.set(&path, &gz_entities::PropValue::Text(bones.clone()), &PropContext::default()).map_err(|err| CmdError::invalid(err.to_string()))?;
    replace_spawn(&svc, obj, e, "Fixed Bones", o, snap.serial).await?;
    Ok(json!({ "fixed_bones": bones }))
}

async fn drop_objects(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let snap = svc.store.snapshot();
    let mut batch = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    match a.text("names") {
        Some(list) => {
            for n in names(list) {
                match snap.find_relaxed(&n) {
                    Some(obj) => batch.push(obj.id),
                    None => missing.push(n),
                }
            }
        }
        None => batch = snap.selected_in_order(),
    }
    if batch.is_empty() {
        return Err(CmdError::invalid("nothing to drop (pass 'names' or select objects)"));
    }
    let line = a.str("mode").is_some_and(|m| m.eq_ignore_ascii_case("line"));
    let requested = batch.len();
    let dropped = drop_ids(&ed, &svc, &batch, a.flag("align_normal"), line, o).await?;
    Ok(json!({ "mode": if line { "line" } else { "box" }, "requested": requested, "dropped": dropped, "missing": missing.join(",") }))
}

async fn snap_objects(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = match a.text("action") {
        Some(t) => Some(SnapAction::parse(t).ok_or_else(|| {
            CmdError::invalid("action must be set_from_selection, add_selection, remove_selection, select or clear")
        })?),
        None => None,
    };
    svc.store.send(SceneCmd::Snap { action, use_list: a.bool("use"), pick: a.bool("pick") }, o).await?;
    let s = svc.store.snapshot();
    let list: Vec<&str> = s.tools.snap.iter().filter_map(|id| s.get(*id)).map(|x| x.name.as_str()).collect();
    Ok(json!({ "use": s.tools.snap_use, "pick": s.tools.snap_pick, "objects": list }))
}

async fn scan_walls(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let (Some(x0), Some(x1), Some(z0), Some(z1), Some(y)) = (a.f32("x0"), a.f32("x1"), a.f32("z0"), a.f32("z1"), a.f32("y")) else {
        return Err(CmdError::invalid("x0, x1, z0, z1 and y are required"));
    };
    let step = a.f32("step").unwrap_or(0.5).max(0.1);
    let reach = a.f32("reach").unwrap_or(6.0).max(0.5);
    let res = a.f32("res").unwrap_or(0.1).max(0.02);
    let rays = a.i64("rays").unwrap_or(24).max(4) as usize;
    if x1 <= x0 || z1 <= z0 || (x1 - x0) > 80.0 || (z1 - z0) > 80.0 {
        return Err(CmdError::invalid("the area must be non-empty and at most 80 x 80 m"));
    }
    let w = ((x1 - x0) / res).ceil() as u32;
    let h = ((z1 - z0) / res).ceil() as u32;
    let mut origins = Vec::new();
    let mut ox = x0;
    while ox <= x1 {
        let mut oz = z0;
        while oz <= z1 {
            origins.push(Vec3::new(ox, y, oz));
            oz += step;
        }
        ox += step;
    }
    let snap = svc.store.snapshot();
    svc.prefetch_library(&ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    let cast = origins.len() * rays;
    let mut cells = ed
        .exec
        .cpu(move || {
            use rayon::prelude::*;
            origins
                .par_iter()
                .flat_map_iter(|o| {
                    let (snap, pick, lib) = (&snap, &pick, &lib);
                    (0..rays).filter_map(move |i| {
                        let ang = std::f32::consts::TAU * i as f32 / rays as f32;
                        let dir = Vec3::new(ang.sin(), 0.0, ang.cos());
                        let hit = ray_scene_objects(pick, snap, lib, *o, dir, reach, None)?;
                        let p = hit.point;
                        if !p.is_finite() || p.x < x0 || p.x >= x1 || p.z < z0 || p.z >= z1 {
                            return None;
                        }
                        let ix = (((p.x - x0) / res) as u32).min(w - 1);
                        let iz = (((p.z - z0) / res) as u32).min(h - 1);
                        Some(iz * w + ix)
                    })
                })
                .collect::<Vec<u32>>()
        })
        .await;
    cells.sort_unstable();
    cells.dedup();
    Ok(json!({ "x0": r(x0, 3), "z0": r(z0, 3), "res": r(res, 3), "w": w, "h": h, "rays": cast, "cells": cells }))
}

async fn probe_vertical(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let (Some(x), Some(y), Some(z)) = (a.f32("x"), a.f32("y"), a.f32("z")) else {
        return Err(CmdError::invalid("x, y and z are required"));
    };
    let reach = a.f32("reach").unwrap_or(30.0);
    let snap = svc.store.snapshot();
    svc.prefetch_library(&ed, &snap).await;
    let (snap, pick) = svc.pick().await;
    let lib = svc.library.clone();
    let (up, down) = ed
        .exec
        .cpu(move || {
            let start = Vec3::new(x, y, z);
            let cast = |d: Vec3| ray_scene_objects(&pick, &snap, &lib, start, d, reach, None).map(|h| h.point).filter(|p| p.is_finite()).map(|p| p.y);
            (cast(Vec3::Y), cast(Vec3::NEG_Y))
        })
        .await;
    Ok(json!({ "ceiling": up.map(|v| r(v, 3)), "floor": down.map(|v| r(v, 3)) }))
}

async fn undo_redo(ed: Arc<Editor>, undo: bool) -> Reply {
    let svc = SceneService::of(&ed)?;
    let done = if undo { svc.store.undo().await } else { svc.store.redo().await };
    if done.is_none() {
        return Err(CmdError::invalid(if undo { "undo buffer empty" } else { "redo buffer empty" }));
    }
    Ok(json!({}))
}

async fn recovery(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.text("action").unwrap_or("get").to_ascii_lowercase();
    let rec = svc.recovery.clone();
    let root = svc.root.clone();
    ed.exec.io(move || rec.bind(&root)).await;
    let linked = service::game_linked(&ed).await;
    let result: Result<(), String> = match action.as_str() {
        "get" => Ok(()),
        "snapshot" => {
            let snap = svc.store.snapshot();
            let source = snap.meta.file.as_ref().map(|f| f.display().to_string()).unwrap_or_default();
            match ed.exec.cpu(move || crate::io::write_undo_stream(&snap, true)).await {
                Ok(stream) => {
                    let rec = svc.recovery.clone();
                    ed.exec.io(move || rec.snapshot_now(&stream, &source, linked).map(|_| ())).await
                }
                Err(e) => Err(e.to_string()),
            }
        }
        "restore" => restore(&ed, &svc, linked, o).await,
        "discard" => {
            let rec = svc.recovery.clone();
            ed.exec.io(move || rec.discard().map(|_| ())).await
        }
        _ => return Err(CmdError::invalid("action must be get, snapshot, restore, or discard")),
    };
    let status = svc.recovery.status(svc.store.is_dirty(), linked).to_json();
    match result {
        Ok(()) => Ok(status),
        Err(e) => Err(CmdError::new("recovery", e).with_data(status)),
    }
}

/// `RestoreStartupEntry`: the recorded scene's file name, the snapshot's content.
async fn restore(ed: &Arc<Editor>, svc: &Arc<SceneService>, linked: bool, o: Origin) -> Result<(), String> {
    let (_, path, source) = svc.recovery.begin_restore(svc.store.is_dirty(), linked)?;
    let settings = crate::game::settings(ed).await;
    let p = path.clone();
    let result = ed
        .exec
        .io(move || -> Result<crate::io::LoadedScene, String> {
            let bytes = std::fs::read(&p).map_err(|e| e.to_string())?;
            let st: crate::io::SyncSettings<'_> = settings.as_deref().map(|g| g as &(dyn gz_entities::Settings + Sync));
            crate::io::parse_scene(&bytes, &p, &crate::io::PartFiles::default(), st).map_err(|_| "cannot deserialize the recovery snapshot".to_owned())
        })
        .await;
    let outcome = match result {
        Ok(mut loaded) => {
            loaded.state.meta.file = (!source.is_empty()).then(|| PathBuf::from(&source));
            svc.prefetch_library(ed, &loaded.state).await;
            match svc.store.send(SceneCmd::Replace(Box::new(loaded.state), ReplaceKind::Recovered), o).await {
                Ok(_) => svc.store.send(SceneCmd::Touch, o).await.map(|_| ()).map_err(|e| e.message),
                Err(e) => Err(e.message),
            }
        }
        Err(e) => Err(e),
    };
    svc.recovery.end_restore(outcome.clone());
    if outcome.is_ok() {
        ed.log.info(format!("Recovered scene from '{}'. Save explicitly to update the original file.", path.display()));
    }
    outcome
}

fn target_index(t: &str) -> Result<usize, CmdError> {
    match t {
        "selection" => Ok(0),
        "world" => Ok(1),
        _ => Err(CmdError::invalid("target must be selection or world")),
    }
}

/// The rows a target lists now.
fn rows_of(s: &SceneState, svc: &SceneService, target: usize) -> Vec<props::MergedRow> {
    if target == 1 {
        return props::merge(vec![props::world_rows(s)]);
    }
    let ctx = PropContext::default();
    let cx = PropCx { lib: Some(&svc.library), entity: &ctx };
    let class = s.tools.target;
    let lists: Vec<Vec<props::Row>> =
        s.of_class(class).filter(|o| s.is_selected(o.id)).map(|o| props::object_rows(s, o, &cx)).collect();
    props::merge(lists)
}

async fn property_inspector(ed: Arc<Editor>, a: Args, _o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let filter_arg = a.str("filter").unwrap_or("").to_owned();
    let action = a.text("action").map(str::to_ascii_lowercase).unwrap_or_else(|| if filter_arg.is_empty() { "get".into() } else { "filter".into() });
    let target_name = a.text("target").unwrap_or("selection").to_ascii_lowercase();
    let t = target_index(&target_name)?;
    {
        let mut v = svc.view.lock();
        match action.as_str() {
            "filter" => v.filter[t] = filter_arg,
            "expand" | "collapse" | "get" => {}
            "open" => v.open[t] = true,
            "close" => v.open[t] = false,
            _ => return Err(CmdError::invalid("action must be get, filter, expand, collapse, open, or close")),
        }
    }
    let (filter, open) = {
        let v = svc.view.lock();
        (v.filter[t].clone(), v.open[t])
    };
    let snap = svc.store.snapshot();
    let rows = rows_of(&snap, &svc, t);
    let f = filter.to_lowercase();
    let visible = rows.iter().filter(|r| f.is_empty() || r.row.key.to_lowercase().contains(&f)).count();
    let limit = a.i64("limit").unwrap_or(200).max(0) as usize;
    let out: Vec<Value> = rows
        .iter()
        .take(limit)
        .map(|m| {
            let mut o = serde_json::Map::new();
            o.insert("key".into(), json!(m.row.key));
            o.insert("label".into(), json!(props::label_of(&m.row.key)));
            o.insert("value".into(), json!(if m.mixed { "(mixed)".to_owned() } else { m.row.text() }));
            o.insert("enabled".into(), json!(m.row.enabled));
            o.insert("mixed".into(), json!(m.mixed));
            o.insert("help".into(), json!(props::help_of(&m.row)));
            let unit = props::unit_of(&m.row);
            if !unit.is_empty() {
                o.insert("unit".into(), json!(unit));
            }
            Value::Object(o)
        })
        .collect();
    Ok(json!({ "target": target_name, "filter": filter, "visible": visible, "total": rows.len(), "open": open, "rows": out }))
}

/// Applies a row's value as text; answers the row's key and its value after.
async fn set_row(ed: &Arc<Editor>, svc: &Arc<SceneService>, t: usize, k: &str, value: &str, o: Origin) -> Result<(String, String), CmdError> {
    let snap = svc.store.snapshot();
    let rows = rows_of(&snap, svc, t);
    let target_name = if t == 1 { "world" } else { "selection" };
    let row = props::find(&rows, k).ok_or_else(|| CmdError::not_found(format!("no row '{k}' (property_inspector target={target_name} lists them)")))?.clone();
    let key = row.row.key.clone();
    let parsed = props::parse(&row.row, value).map_err(|e| CmdError::invalid(format!("{key} {e}")))?;
    if t == 1 {
        let (level, tools) = props::set_world(&snap, &key, &parsed).map_err(|e| CmdError::invalid(format!("{key} {e}")))?;
        svc.store.send(SceneCmd::SetWorld { level: Box::new(level), tools, serial: snap.serial, label: "Level Settings".into() }, o).await?;
    } else {
        let ctx = PropContext::default();
        let cx = PropCx { lib: Some(&svc.library), entity: &ctx };
        let class = snap.tools.target;
        let selected: Vec<Arc<Object>> = snap.of_class(class).filter(|x| snap.is_selected(x.id)).cloned().collect();
        let mut objects = Vec::new();
        let mut transforms = Vec::new();
        for obj in &selected {
            match props::set_object(&snap, obj, &key, &parsed, &cx).map_err(|e| CmdError::invalid(format!("{key} {e}")))? {
                Effect::Object(n) => objects.push(n),
                Effect::Objects(list) => objects.extend(list),
                Effect::Transform(e) => transforms.push((obj.id, e)),
                Effect::Nothing => {}
            }
        }
        if !transforms.is_empty() {
            svc.store.send(SceneCmd::Transform { items: transforms, members: true, label: "Transform".into(), merge: None }, o).await?;
        }
        if !objects.is_empty() {
            svc.store.send(SceneCmd::ReplaceObjects { objects, serial: snap.serial, label: "Property".into() }, o).await?;
        }
    }
    let after = svc.store.snapshot();
    let rows = rows_of(&after, svc, t);
    let text = props::find(&rows, &key).map(|m| if m.mixed { "(mixed)".to_owned() } else { m.row.text() }).unwrap_or_default();
    let _ = ed;
    Ok((key, text))
}

async fn property_set(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let target_name = a.text("target").unwrap_or("selection").to_owned();
    let t = target_index(&target_name)?;
    let (Some(k), Some(value)) = (a.text("key"), a.get("value")) else {
        return Err(CmdError::invalid("property_set takes key= and value="));
    };
    let value = match value {
        Value::String(s) => s.clone(),
        Value::Null => return Err(CmdError::invalid("property_set takes key= and value=")),
        other => other.to_string(),
    };
    let (key, text) = set_row(&ed, &svc, t, k, &value, o).await?;
    Ok(json!({ "target": target_name, "key": key, "value": text }))
}

async fn property_choose(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = SceneService::of(&ed)?;
    let action = a.str("action").unwrap_or("").to_ascii_lowercase();
    let k = a.text("key").unwrap_or("Visual").to_owned();
    let item = a.str("item").unwrap_or("").to_owned();
    let snap = svc.store.snapshot();
    let rows = rows_of(&snap, &svc, 0);
    let row = props::find(&rows, &k).cloned();
    match action.as_str() {
        "open" => {
            let Some(r) = row else { return Err(CmdError::not_found("no such property row")) };
            let started = matches!(r.row.kind, props::Kind::Choose(_)) && svc.view.lock().choosing.is_none();
            if started {
                let mut v = svc.view.lock();
                v.choosing = Some(r.row.key.clone());
                v.chosen.clear();
            }
            let fields = json!({ "key": r.row.key, "value": r.row.text() });
            if started { Ok(fields) } else { Err(CmdError::invalid("the row has no chooser, or one is open").with_data(fields)) }
        }
        "cancel" => {
            let closed = svc.view.lock().choosing.take().is_some();
            if closed { Ok(json!({})) } else { Err(CmdError::invalid("no chooser is open")) }
        }
        "select" | "pick" => {
            let open = svc.view.lock().choosing.clone();
            let Some(key) = open else { return Err(CmdError::invalid("no chooser is open")) };
            let value = item.strip_suffix('*').unwrap_or(&item).to_owned();
            if value.is_empty() {
                return Err(CmdError::invalid("no item named"));
            }
            svc.view.lock().chosen = value.clone();
            if action == "pick" {
                svc.view.lock().choosing = None;
                set_row(&ed, &svc, 0, &key, &value, o).await?;
            }
            Ok(json!({}))
        }
        _ => {
            let v = svc.view.lock().clone();
            Ok(json!({
                "form_alive": v.choosing.is_some(),
                "pending": v.choosing.is_some(),
                "selected": v.chosen,
                "value": row.map(|r| r.row.text()).unwrap_or_default(),
            }))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn points_and_names_parse_like_the_cpp() {
        assert_eq!(parse_points("1,2,3; 4 , 5 , 6").unwrap(), vec![Vec3::new(1.0, 2.0, 3.0), Vec3::new(4.0, 5.0, 6.0)]);
        assert_eq!(parse_points("1,2").unwrap_err(), "1,2");
        assert_eq!(names(" a , b,, c "), vec!["a", "b", "c"]);
    }

    #[test]
    fn object_json_rounds_like_printf() {
        let mut s = SceneState::default();
        let mut o = Object::new(ObjClass::Light, "l", ObjData::Light(crate::objects::LightData::default()));
        o.xform.position = Vec3::new(1.23456, 0.0, -2.0);
        let id = s.insert(o);
        let j = object_json(&s, s.get(id).unwrap());
        assert_eq!(j["position"], json!([1.235, 0.0, -2.0]));
        assert_eq!(j["class"], "light");
    }

    #[test]
    fn the_camera_looks_where_its_heading_says() {
        let d = direction(Vec3::ZERO);
        assert!((d - Vec3::Z).length() < 1e-6);
    }
}
