//! `sector_tool`, `portal_tool`, `way_tool`, `shape_tool`, `light_tool`, `fog_tool`: every
//! button, switch and gesture of the six panels as `action=`. The panels post these; the
//! viewport's gestures call the same functions.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{Args, ArgsExt, CmdError, Editor, Registry, Reply, Value, json};
use gz_fmt_xr::ShadersXrlc;

use super::sector::{self, DEFAULT_SECTOR};
use super::way::{self, LinkOp};
use super::{
    Batch, ObjToolState, Outcome, ToolEdit, commit, commit_job, portal, selected_of, set_state, shape,
};
use crate::apply::{SceneCmd, SelectMode};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{FV_EMITTER, WayData};
use crate::service::SceneService;

const AREA: &str = "gz-scene";

/// Registers the six tool commands.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "sector_tool", sector_tool);
    reg.add(AREA, "portal_tool", portal_tool);
    reg.add(AREA, "way_tool", way_tool);
    reg.add(AREA, "shape_tool", shape_tool);
    reg.add(AREA, "light_tool", light_tool);
    reg.add(AREA, "fog_tool", fog_tool);
}

fn scene(ed: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
    let svc = SceneService::of(ed)?;
    if !svc.store.snapshot().valid {
        return Err(CmdError::new("no_scene", "no scene"));
    }
    Ok(svc)
}

/// The scene, with the tool made the current one first when `show=true` (the Tools panel then
/// shows its form).
async fn scene_for(
    ed: &Arc<Editor>,
    a: &Args,
    o: Origin,
    class: ObjClass,
) -> Result<Arc<SceneService>, CmdError> {
    let svc = scene(ed)?;
    if a.flag("show") && svc.store.snapshot().tools.target != class {
        svc.store.send(SceneCmd::SetTarget(class), o).await?;
    }
    Ok(svc)
}

fn unknown(action: &str, known: &str) -> CmdError {
    CmdError::invalid(format!("unknown action '{action}' (one of {known})"))
}

/// An object of a class by name (names are kept lower case).
fn named(s: &SceneState, class: ObjClass, name: &str, what: &str) -> Result<ObjId, CmdError> {
    s.find(class, &name.to_lowercase())
        .map(|o| o.id)
        .ok_or_else(|| CmdError::not_found(format!("{what} '{name}' not found")))
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Value::Object(m), Value::Object(n)) = (&mut a, b) {
        m.extend(n);
    }
    a
}

/// The compiler shader library of the linked game (or the editor's own copy).
async fn shaders(ed: &Arc<Editor>) -> Option<Arc<ShadersXrlc>> {
    let (bytes, _) = gz_game::read_game_or_bundled(ed, "shaders_xrlc.xr").await.ok()?;
    ShadersXrlc::read(&bytes).ok().map(Arc::new)
}

fn log_lines(ed: &Editor, lines: &[String], error: bool) {
    for l in lines {
        if error || l.starts_with("*ERROR") || l.starts_with('!') {
            ed.log.error(l.clone());
        } else {
            ed.log.warn(l.clone());
        }
    }
}

// ---- sector_tool --------------------------------------------------------------------------------

fn sector_mode_json(m: &super::SectorMode) -> Value {
    json!({
        "create_single": m.create_single,
        "create_multiple": m.create_multiple,
        "mesh_mode": if m.mesh_add { "add" } else { "remove" },
        "box_pick": m.box_pick,
    })
}

fn sector_state(s: &SceneState, lib: &Library) -> Value {
    let sectors: Vec<Value> = s.of_class(ObjClass::Sector).map(|o| sector::sector_json(s, lib, o)).collect();
    json!({
        "mode": sector_mode_json(&s.tools.obj.sector),
        "has_default": s.find(ObjClass::Sector, DEFAULT_SECTOR).is_some(),
        "sectors": sectors,
    })
}

/// The sector an action works on: `sector=`, or the one selected.
fn target_sector(s: &SceneState, a: &Args) -> Result<ObjId, CmdError> {
    match a.str("sector") {
        Some(n) => named(s, ObjClass::Sector, n, "sector"),
        None => sector::selected_sector(s)?.ok_or_else(|| CmdError::invalid("Select a sector first")),
    }
}

/// A scene object and one of its meshes, by name or index.
fn object_mesh(
    s: &SceneState,
    lib: &Library,
    object: &str,
    mesh: Option<&str>,
) -> Result<(ObjId, Option<usize>), CmdError> {
    let id = named(s, ObjClass::SceneObject, object, "object")?;
    let Some(mesh) = mesh else { return Ok((id, None)) };
    let o = s.get(id).ok_or_else(|| CmdError::not_found("object not found"))?;
    let ObjData::SceneObject(d) = &o.data else {
        return Err(CmdError::invalid("the object is not a scene object"));
    };
    let lo = lib
        .cached(&d.reference)
        .ok_or_else(|| CmdError::invalid("the object's library object could not be loaded"))?;
    let i = super::geom::mesh_index(&lo, mesh)
        .or_else(|| mesh.parse::<usize>().ok().filter(|i| *i < lo.object.meshes.len()))
        .ok_or_else(|| CmdError::not_found(format!("the object has no mesh '{mesh}'")))?;
    Ok((id, Some(i)))
}

async fn sector_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Sector).await?;
    let action = a.str_or("action", "state").to_owned();
    match action.as_str() {
        "state" => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            Ok(sector_state(&snap, &svc.library))
        }
        "set" => {
            let s = set_state(&svc, o, |st| {
                let m = &mut st.sector;
                if let Some(v) = a.bool("create_single") {
                    m.create_single = v;
                    if v {
                        m.create_multiple = false;
                    }
                }
                if let Some(v) = a.bool("create_multiple") {
                    m.create_multiple = v;
                    if v {
                        m.create_single = false;
                    }
                }
                if let Some(v) = a.str("mesh_mode") {
                    m.mesh_add = !v.eq_ignore_ascii_case("remove");
                }
                if let Some(v) = a.bool("box_pick") {
                    m.box_pick = v;
                }
                // "Not while a new sector is being made"
                if m.create_single || m.create_multiple {
                    m.box_pick = false;
                }
            })
            .await?;
            Ok(json!({ "mode": sector_mode_json(&s.sector) }))
        }
        "validate" => {
            let sh = shaders(&ed).await;
            let note = sh.is_none().then_some(
                "The compiler shaders (shaders_xrlc.xr) could not be read: the shader checks were left out.",
            );
            let out = commit_job(&ed, &svc, o, "Validating the sectors", move |s, lib, _| {
                let (valid, problems) = sector::validate(s, lib, sh.as_deref());
                Ok((Batch::new("Validate", s.serial), json!({ "valid": valid, "problems": problems })))
            })
            .await?;
            if let Some(p) = out.get("problems").and_then(Value::as_array) {
                let lines: Vec<String> = p.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
                log_lines(&ed, &lines, false);
            }
            let mut out = out;
            if let (Some(n), Value::Object(m)) = (note, &mut out) {
                m.insert("note".into(), json!(n));
            }
            Ok(out)
        }
        "create_default" => {
            commit_job(&ed, &svc, o, "Creating the default sector", |s, lib, _| {
                sector::create_default(s, lib)
            })
            .await
        }
        "remove_default" => {
            commit(&ed, &svc, o, super::Ctl::default(), |s, _, _| sector::remove_default(s)).await
        }
        "capture_volume" => {
            let id = target_sector(&svc.store.snapshot(), &a)?;
            commit_job(&ed, &svc, o, "Capturing the sector's volume", move |s, lib, _| {
                sector::capture_volume(s, lib, id)
            })
            .await
        }
        "distribute" | "distribute_objects" => {
            let s = svc.store.snapshot();
            let id = target_sector(&s, &a)?;
            let name = s.get(id).map(|x| x.name.to_string()).unwrap_or_default();
            // oracle: `CSector::DistributeInsideObjects` is commented out in the C++: it changes nothing
            Ok(
                json!({ "sector": name, "changed": 0, "note": "Distribute Objects changes nothing in this version." }),
            )
        }
        "add_mesh" | "remove_mesh" | "add_meshes" | "remove_meshes" => {
            let add = action.starts_with("add");
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let id = target_sector(&snap, &a)?;
            let mut picks = Vec::new();
            if let Some(items) = a.list("items") {
                for it in items {
                    let (obj, mesh) = it
                        .split_once(':')
                        .ok_or_else(|| CmdError::invalid(format!("an item is not object:mesh: {it}")))?;
                    let (oid, m) = object_mesh(&snap, &svc.library, obj, Some(mesh))?;
                    picks.push((oid, m.unwrap_or(0)));
                }
            } else {
                let obj = a.need("object")?;
                let (oid, m) = object_mesh(&snap, &svc.library, obj, Some(a.need("mesh")?))?;
                picks.push((oid, m.unwrap_or(0)));
            }
            let boxed = a.flag("box");
            let label = if add { "Add Mesh" } else { "Remove Mesh" };
            let out = commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| {
                sector::edit_meshes(s, lib, id, &picks, add, boxed && !add, label)
            })
            .await?;
            if let Some(n) = out.get("note").and_then(Value::as_str) {
                ed.log.info(n.to_owned());
            }
            Ok(out)
        }
        "new_from_mesh" => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let (oid, m) = object_mesh(&snap, &svc.library, a.need("object")?, Some(a.need("mesh")?))?;
            let mesh = m.unwrap_or(0);
            commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| {
                sector::new_from_mesh(s, lib, oid, mesh)
            })
            .await
        }
        "new_from_object" => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let (oid, _) = object_mesh(&snap, &svc.library, a.need("object")?, None)?;
            let seed = a.i64("seed").map_or_else(time_seed, |v| v as i32);
            commit(&ed, &svc, o, super::Ctl::default(), move |s, lib, _| {
                sector::new_from_object(s, lib, oid, seed)
            })
            .await
        }
        other => Err(unknown(
            other,
            "state, set, validate, create_default, remove_default, capture_volume, distribute, add_mesh, remove_mesh, add_meshes, remove_meshes, new_from_mesh, new_from_object",
        )),
    }
}

/// A seed from the clock, as `Random` is seeded.
#[must_use]
pub fn time_seed() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| (d.as_nanos() & 0x7fff_ffff) as i32)
}

// ---- portal_tool --------------------------------------------------------------------------------

fn portal_state(s: &SceneState) -> Value {
    let portals: Vec<Value> = s
        .of_class(ObjClass::Portal)
        .filter_map(|o| match &o.data {
            ObjData::Portal(p) => Some(json!({
                "name": o.name.as_str(),
                "front": p.front.as_str(),
                "back": p.back.as_str(),
                "vertices": p.vertices.len(),
                "selected": s.is_selected(o.id),
            })),
            _ => None,
        })
        .collect();
    json!({
        "portals": portals,
        "selected_sectors": selected_of(s, ObjClass::Sector).len(),
        "sectors": s.count(ObjClass::Sector),
    })
}

/// The new portals' names: `GenObjectName(OBJCLASS_PORTAL)` once the old ones are gone.
fn name_gen<'a>(s: &'a SceneState, removed: &[ObjId]) -> impl FnMut() -> String + use<'a> {
    let gone: Vec<String> = removed.iter().filter_map(|id| s.get(*id)).map(|o| o.name.to_string()).collect();
    let mut taken: Vec<String> = Vec::new();
    move || {
        for i in 0u32.. {
            let n = format!("{i:02}");
            if (!s.name_used(&n) || gone.contains(&n)) && !taken.contains(&n) {
                taken.push(n.clone());
                return n;
            }
        }
        String::new()
    }
}

/// `CalculateAllPortals` / `CalculateSelectedPortals`.
fn compute_portals(
    s: &SceneState,
    lib: &Library,
    ctl: &super::Ctl,
    selected: bool,
    shaders: Option<&ShadersXrlc>,
) -> Result<Outcome, CmdError> {
    ctl.stage("Validating the sectors");
    let (valid, _) = sector::validate(s, lib, shaders);
    if !valid {
        return Err(CmdError::invalid(if selected {
            "*ERROR: Scene has non associated face (face without sector)!"
        } else {
            "*ERROR: Sector validation failed."
        }));
    }
    let (sectors, removed): (Vec<ObjId>, Vec<ObjId>) = if selected {
        let sel: Vec<ObjId> =
            s.of_class(ObjClass::Sector).filter(|o| s.is_selected(o.id)).map(|o| o.id).collect();
        if sel.len() < 2 {
            return Err(CmdError::invalid("Select at least 2 sectors."));
        }
        let names: Vec<String> = sel.iter().filter_map(|id| s.get(*id)).map(|o| o.name.to_string()).collect();
        let has = |n: &str| names.iter().any(|x| x.eq_ignore_ascii_case(n));
        let old = s
            .of_class(ObjClass::Portal)
            .filter(|p| matches!(&p.data, ObjData::Portal(d) if has(&d.front) && has(&d.back)))
            .map(|p| p.id)
            .collect();
        (sel, old)
    } else {
        (
            s.of_class(ObjClass::Sector).map(|o| o.id).collect(),
            s.of_class(ObjClass::Portal).map(|p| p.id).collect(),
        )
    };
    let mut names = name_gen(s, &removed);
    let stop = || ctl.stopped();
    let stage = |t: &str| ctl.stage(t);
    let c = portal::compute(s, lib, &sectors, &mut names, &stop, &stage).map_err(|_| super::stopped())?;
    let mut b =
        Batch::new(if selected { "Compute Selected Portals" } else { "Compute All Portals" }, s.serial);
    b.remove = removed.clone();
    let created = c.portals.len();
    let mut names = name_gen(s, &removed);
    for p in c.portals {
        let name = names();
        b.add.push((Object::new(ObjClass::Portal, &name, ObjData::Portal(p)), super::NameRule::Fixed(name)));
    }
    Ok((
        b,
        json!({
            "computed": c.loops,
            "created": created,
            "removed": removed.len(),
            "messages": c.log,
        }),
    ))
}

async fn portal_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Portal).await?;
    let action = a.str_or("action", "state").to_owned();
    match action.as_str() {
        "state" => Ok(portal_state(&svc.store.snapshot())),
        "compute_all" | "compute_selected" => {
            let selected = action == "compute_selected";
            let sh = shaders(&ed).await;
            let out = commit_job(
                &ed,
                &svc,
                o,
                if selected { "Computing the selected portals" } else { "Computing the portals" },
                move |s, lib, ctl| compute_portals(s, lib, ctl, selected, sh.as_deref()),
            )
            .await?;
            if let Some(m) = out.get("messages").and_then(Value::as_array) {
                let lines: Vec<String> = m.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect();
                log_lines(&ed, &lines, true);
            }
            let n = out.get("computed").and_then(Value::as_u64).unwrap_or(0);
            Ok(merge(
                out,
                json!({ "message": format!("Computed {n} portal{}", if n == 1 { "" } else { "s" }) }),
            ))
        }
        "remove_duplicates" => {
            let out = commit(&ed, &svc, o, super::Ctl::default(), |s, _, _| {
                let gone = portal::similar_portals(s);
                let n = gone.len();
                let mut b = Batch::new("Remove Duplicate Portals", s.serial);
                b.remove = gone;
                Ok((b, json!({ "removed": n, "message": format!("Removed '{n}' portal(s).") })))
            })
            .await?;
            Ok(out)
        }
        "invert" | "invert_orientation" => {
            commit(&ed, &svc, o, super::Ctl::default(), |s, _, _| {
                let mut b = Batch::new("Invert Orientation", s.serial);
                for id in selected_of(s, ObjClass::Portal) {
                    let Some(p) = s.get(id) else { continue };
                    let mut p = (**p).clone();
                    if let ObjData::Portal(d) = &mut p.data {
                        d.vertices.reverse();
                    }
                    b.replace.push(p);
                }
                let n = b.replace.len();
                Ok((b, json!({ "inverted": n })))
            })
            .await
        }
        other => Err(unknown(other, "state, compute_all, compute_selected, remove_duplicates, invert")),
    }
}

// ---- way_tool -----------------------------------------------------------------------------------

fn way_json(s: &SceneState, o: &Object, w: &WayData) -> Value {
    let points: Vec<Value> = w
        .points
        .iter()
        .map(|p| {
            let links: Vec<Value> = p
                .links
                .iter()
                .map(|(t, pr)| {
                    json!({ "to": w.points.get(*t as usize).map_or("", |q| q.name.as_str()), "probability": pr })
                })
                .collect();
            json!({
                "name": p.name.as_str(),
                "selected": p.selected,
                "position": [p.position.x, p.position.y, p.position.z],
                "links": links,
            })
        })
        .collect();
    json!({ "name": o.name.as_str(), "selected": s.is_selected(o.id), "points": points })
}

fn way_state(s: &SceneState) -> Value {
    let ways: Vec<Value> = s
        .of_class(ObjClass::Way)
        .filter(|o| s.is_selected(o.id))
        .filter_map(|o| match &o.data {
            ObjData::Way(w) => Some(way_json(s, o, w)),
            _ => None,
        })
        .collect();
    json!({
        "mode": if s.tools.obj.way_points { "points" } else { "ways" },
        "auto_link": s.tools.obj.auto_link,
        "ways": s.count(ObjClass::Way),
        "selected": ways,
    })
}

/// The way an action works on: `way=`, or the one selected (`Select one WayObject.`).
fn target_way(s: &SceneState, a: &Args) -> Result<ObjId, CmdError> {
    if let Some(n) = a.str("way") {
        return named(s, ObjClass::Way, n, "way");
    }
    let sel: Vec<ObjId> = s.of_class(ObjClass::Way).filter(|o| s.is_selected(o.id)).map(|o| o.id).collect();
    match sel.as_slice() {
        [one] => Ok(*one),
        _ => Err(CmdError::invalid("Select one WayObject.")),
    }
}

/// `EditSelectedWays`: a link edit of every selected way.
pub fn link_batch(s: &SceneState, op: LinkOp) -> Result<Outcome, CmdError> {
    if op.needs_points() && !s.tools.obj.way_points {
        return Err(CmdError::invalid("Switch Select to Points first"));
    }
    let mut b = Batch::new(op.label(), s.serial);
    let mut changed = Vec::new();
    for id in selected_of(s, ObjClass::Way) {
        let Some(o) = s.get(id) else { continue };
        let ObjData::Way(w) = &o.data else { continue };
        let mut w2 = w.clone();
        if way::edit_links(&mut w2, op) && w2 != *w {
            let mut n = (**o).clone();
            n.data = ObjData::Way(w2);
            b.replace.push(n);
            changed.push(o.name.to_string());
        }
    }
    Ok((b, json!({ "action": op.label(), "changed": changed })))
}

/// Adds a point to a way at `at` (`TUI_ControlWayPointAdd::Start`).
pub fn add_point_batch(s: &SceneState, way_id: ObjId, at: Vec3) -> Result<Outcome, CmdError> {
    let o = s.get(way_id).ok_or_else(|| CmdError::not_found("way not found"))?;
    let ObjData::Way(w) = &o.data else { return Err(CmdError::invalid("the object is not a way")) };
    let mut w2 = w.clone();
    let i = way::add_point(&mut w2, at, s.tools.obj.auto_link);
    let name = w2.points.get(i).map(|p| p.name.to_string()).unwrap_or_default();
    let mut n = (**o).clone();
    n.data = ObjData::Way(w2);
    let mut b = Batch::new("Add Way Point", s.serial);
    b.replace.push(n);
    Ok((b, json!({ "way": o.name.as_str(), "point": name })))
}

/// Removes the selected points of ways; a way left without points goes (`OnSelectionRemove`).
pub fn remove_points_batch(s: &SceneState, ways: &[ObjId]) -> Outcome {
    let mut b = Batch::new("Remove Points", s.serial);
    let mut removed = 0;
    for id in ways {
        let Some(o) = s.get(*id) else { continue };
        let ObjData::Way(w) = &o.data else { continue };
        let mut w2 = w.clone();
        let n = way::remove_selected_points(&mut w2);
        if n == 0 {
            continue;
        }
        removed += n;
        if w2.points.is_empty() {
            b.remove.push(*id);
        } else {
            let mut x = (**o).clone();
            x.data = ObjData::Way(w2);
            b.replace.push(x);
        }
    }
    (b, json!({ "removed": removed }))
}

fn point_keys(w: &WayData, a: &Args) -> Result<Vec<usize>, CmdError> {
    let keys = a
        .list("points")
        .or_else(|| a.str("point").map(|p| vec![p.to_owned()]))
        .ok_or_else(|| CmdError::invalid("missing 'points' argument"))?;
    keys.iter()
        .map(|k| {
            way::point_index(w, k).ok_or_else(|| CmdError::not_found(format!("the way has no point '{k}'")))
        })
        .collect()
}

async fn way_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Way).await?;
    let action = a.str_or("action", "state").to_owned();
    if let Some(op) = LinkOp::parse(&action) {
        return commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| link_batch(s, op)).await;
    }
    match action.as_str() {
        "state" => Ok(way_state(&svc.store.snapshot())),
        "mode" | "select_mode" => {
            let m = a.need("mode")?;
            let points = match m {
                "points" | "point" => true,
                "ways" | "way" => false,
                _ => return Err(CmdError::invalid("mode is ways or points")),
            };
            let s = set_state(&svc, o, |st| st.way_points = points).await?;
            Ok(json!({ "mode": if s.way_points { "points" } else { "ways" } }))
        }
        "auto_link" => {
            let on = a.bool("on").ok_or_else(|| CmdError::invalid("missing 'on' argument"))?;
            let s = set_state(&svc, o, |st| st.auto_link = on).await?;
            Ok(json!({ "auto_link": s.auto_link }))
        }
        "add_point" => {
            let p = a.vec3("position").ok_or_else(|| CmdError::invalid("missing 'position': x,y,z"))?;
            let at = Vec3::from_array(p);
            if !at.is_finite() {
                return Err(CmdError::invalid("non-finite position"));
            }
            let id = target_way(&svc.store.snapshot(), &a)?;
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| add_point_batch(s, id, at)).await
        }
        "select_points" => {
            let snap = svc.store.snapshot();
            let id = target_way(&snap, &a)?;
            let Some(ObjData::Way(w)) = snap.get(id).map(|x| &x.data) else {
                return Err(CmdError::invalid("the object is not a way"));
            };
            let points = point_keys(w, &a)?;
            let mode = match a.str_or("mode", "set") {
                "add" => SelectMode::Add,
                "remove" => SelectMode::Remove,
                "toggle" => SelectMode::Toggle,
                _ => SelectMode::Set,
            };
            svc.store
                .send(SceneCmd::ObjTool(Box::new(ToolEdit::SelectPoints { way: id, points, mode })), o)
                .await?;
            let s = svc.store.snapshot();
            let out = s.get(id).and_then(|x| match &x.data {
                ObjData::Way(w) => Some(way_json(&s, x, w)),
                _ => None,
            });
            Ok(json!({ "way": out }))
        }
        "move_point" => {
            let snap = svc.store.snapshot();
            let id = target_way(&snap, &a)?;
            let Some(ObjData::Way(w)) = snap.get(id).map(|x| &x.data) else {
                return Err(CmdError::invalid("the object is not a way"));
            };
            let points = point_keys(w, &a)?;
            let p = Vec3::from_array(
                a.vec3("position").ok_or_else(|| CmdError::invalid("missing 'position': x,y,z"))?,
            );
            let moves = points.into_iter().map(|i| (i, p)).collect();
            svc.store
                .send(
                    SceneCmd::ObjTool(Box::new(ToolEdit::MovePoints { way: id, points: moves, merge: None })),
                    o,
                )
                .await?;
            Ok(json!({ "moved": true }))
        }
        "remove_points" => {
            let ids = match a.str("way") {
                Some(_) => vec![target_way(&svc.store.snapshot(), &a)?],
                None => selected_of(&svc.store.snapshot(), ObjClass::Way),
            };
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| Ok(remove_points_batch(s, &ids))).await
        }
        other => Err(unknown(
            other,
            "state, mode, auto_link, create_1link, create_2link, convert_1link, convert_2link, invert_link, remove_link, add_point, select_points, move_point, remove_points",
        )),
    }
}

// ---- shape_tool ---------------------------------------------------------------------------------

fn shape_state(s: &SceneState) -> Value {
    let t = &s.tools.obj;
    let selected: Vec<Value> = selected_of(s, ObjClass::Shape)
        .iter()
        .filter_map(|id| s.get(*id))
        .map(|o| {
            let parts = match &o.data {
                ObjData::Shape(d) => d.shapes.len(),
                _ => 0,
            };
            json!({ "name": o.name.as_str(), "parts": parts })
        })
        .collect();
    let bound = s
        .of_class(ObjClass::Shape)
        .find(|o| matches!(&o.data, ObjData::Shape(d) if d.usage == crate::objects::SHAPE_LEVEL_BOUND))
        .map(|o| o.name.to_string());
    json!({
        "shape": if t.shape_sphere { "sphere" } else { "box" },
        "attach_on_click": t.attach_on_click,
        "edit_level_bound": t.edit_level_bound,
        "level_bound": bound,
        "selected": selected,
    })
}

async fn shape_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Shape).await?;
    let action = a.str_or("action", "state").to_owned();
    match action.as_str() {
        "state" => Ok(shape_state(&svc.store.snapshot())),
        "set" => {
            let shape = a.str("shape").map(|v| v.eq_ignore_ascii_case("sphere"));
            let attach = a.bool("attach_on_click");
            set_state(&svc, o, |st| {
                if let Some(v) = shape {
                    st.shape_sphere = v;
                }
                if let Some(v) = attach {
                    st.attach_on_click = v;
                }
            })
            .await?;
            Ok(shape_state(&svc.store.snapshot()))
        }
        "attach" => {
            let snap = svc.store.snapshot();
            let from = named(&snap, ObjClass::Shape, a.need("from")?, "shape")?;
            let to = match a.str("to") {
                Some(n) => Some(named(&snap, ObjClass::Shape, n, "shape")?),
                None => None,
            };
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| match to {
                Some(t) => shape::attach(s, t, from),
                None => shape::attach_to_selected(s, from),
            })
            .await
        }
        "add" => {
            // a Place click's command form (`DefaultAddObject` with the shape of the switch)
            let p = Vec3::from_array(
                a.vec3("position").ok_or_else(|| CmdError::invalid("missing 'position': x,y,z"))?,
            );
            if !p.is_finite() {
                return Err(CmdError::invalid("non-finite position"));
            }
            let sphere = a.str("shape").map(|v| v.eq_ignore_ascii_case("sphere"));
            commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| {
                let mut obj = Object::new(
                    ObjClass::Shape,
                    "",
                    ObjData::Shape(shape::new_shape_data(sphere.unwrap_or(s.tools.obj.shape_sphere))),
                );
                obj.xform.position = p;
                let mut b = Batch::new("Add Object", s.serial);
                b.add.push((obj, super::NameRule::Gen(Some(ObjClass::Shape.name().to_owned()))));
                b.select_added = true;
                Ok((b, json!({ "added": true })))
            })
            .await?;
            let s = svc.store.snapshot();
            let added =
                selected_of(&s, ObjClass::Shape).last().and_then(|id| s.get(*id)).map(|x| x.name.to_string());
            Ok(merge(json!({ "shape": added }), shape_state(&s)))
        }
        "detach" => commit(&ed, &svc, o, super::Ctl::default(), |s, _, _| shape::detach_selected(s)).await,
        "edit_level_bound" => {
            let on = a.bool("on").ok_or_else(|| CmdError::invalid("missing 'on' argument"))?;
            let mut st: ObjToolState = svc.store.snapshot().tools.obj;
            st.edit_level_bound = on;
            if !on {
                set_state(&svc, o, |x| x.edit_level_bound = false).await?;
                return Ok(shape_state(&svc.store.snapshot()));
            }
            let out = commit_job(&ed, &svc, o, "Editing the level bound", move |s, lib, _| {
                shape::edit_level_bound(s, lib, false, Some(st))
            })
            .await?;
            Ok(merge(out, shape_state(&svc.store.snapshot())))
        }
        "recalculate" | "recalculate_level_bound" => {
            if !svc.store.snapshot().tools.obj.edit_level_bound {
                return Err(CmdError::invalid("Turn Edit Level Bound on first"));
            }
            commit_job(&ed, &svc, o, "Recalculating the level bound", |s, lib, _| {
                shape::edit_level_bound(s, lib, true, None)
            })
            .await
        }
        other => Err(unknown(other, "state, set, add, attach, detach, edit_level_bound, recalculate")),
    }
}

// ---- light_tool ---------------------------------------------------------------------------------

fn light_state(s: &SceneState) -> Value {
    let lights: Vec<Value> = s
        .of_class(ObjClass::Light)
        .filter_map(|o| match &o.data {
            ObjData::Light(l) => Some(
                json!({ "name": o.name.as_str(), "selected": s.is_selected(o.id), "viewport": l.use_in_d3d }),
            ),
            _ => None,
        })
        .collect();
    json!({ "lights": lights })
}

/// `UILightTool::UseInD3D(all, flag)`: which lights light the viewport.
pub fn use_in_viewport(s: &SceneState, all: bool, on: bool) -> Outcome {
    let label = match (all, on) {
        (false, true) => "Enable Selected",
        (true, true) => "Enable All",
        (false, false) => "Disable Selected",
        (true, false) => "Disable All",
    };
    let mut b = Batch::new(label, s.serial);
    for o in s.of_class(ObjClass::Light) {
        if !all && !(s.is_selected(o.id) && o.flags.visible) {
            continue;
        }
        if let ObjData::Light(l) = &o.data
            && l.use_in_d3d != on
        {
            let mut n = (**o).clone();
            if let ObjData::Light(nl) = &mut n.data {
                nl.use_in_d3d = on;
            }
            b.replace.push(n);
        }
    }
    let n = b.replace.len();
    (b, json!({ "action": label, "changed": n }))
}

async fn light_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Light).await?;
    let action = a.str_or("action", "state").to_owned();
    let (all, on) = match action.as_str() {
        "state" => return Ok(light_state(&svc.store.snapshot())),
        "enable_selected" => (false, true),
        "enable_all" => (true, true),
        "disable_selected" => (false, false),
        "disable_all" => (true, false),
        other => {
            return Err(unknown(other, "state, enable_selected, enable_all, disable_selected, disable_all"));
        }
    };
    let out =
        commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| Ok(use_in_viewport(s, all, on))).await?;
    Ok(merge(out, light_state(&svc.store.snapshot())))
}

// ---- fog_tool -----------------------------------------------------------------------------------

fn fog_state(s: &SceneState) -> Value {
    let vols: Vec<Value> = s
        .of_class(ObjClass::FogVolume)
        .filter_map(|o| match &o.data {
            ObjData::FogVolume(f) => Some(json!({
                "name": o.name.as_str(),
                "selected": s.is_selected(o.id),
                "group": if f.group_id == u32::MAX { Value::Null } else { json!(f.group_id) },
                "type": if f.kind == FV_EMITTER { "emitter" } else { "occlusion" },
            })),
            _ => None,
        })
        .collect();
    json!({ "volumes": vols })
}

/// `GroupSelected` / `UnGroupCurrent`.
pub fn fog_group(s: &SceneState, group: bool) -> Outcome {
    let mut b = Batch::new(if group { "Group Selected" } else { "Ungroup Current" }, s.serial);
    let mut st = s.tools.obj;
    // `RegisterGroup`: the counter is at least every group read or set; oracle: an ungrouped
    // volume registers u32::MAX, so the next group wraps round to 0 as in the C++
    let mut counter = st.fog_group_counter;
    for o in s.of_class(ObjClass::FogVolume) {
        if let ObjData::FogVolume(f) = &o.data {
            counter = counter.max(f.group_id);
        }
    }
    let id = if group {
        counter = counter.wrapping_add(1);
        st.fog_group_counter = counter;
        b.state = Some(st);
        counter
    } else {
        u32::MAX
    };
    for o in s.of_class(ObjClass::FogVolume) {
        if !s.is_selected(o.id) {
            continue;
        }
        if let ObjData::FogVolume(f) = &o.data
            && f.group_id != id
        {
            let mut n = (**o).clone();
            if let ObjData::FogVolume(nf) = &mut n.data {
                nf.group_id = id;
            }
            b.replace.push(n);
        }
    }
    let n = b.replace.len();
    (b, json!({ "changed": n, "group": if group { json!(id) } else { Value::Null } }))
}

async fn fog_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::FogVolume).await?;
    let action = a.str_or("action", "state").to_owned();
    let group = match action.as_str() {
        "state" => return Ok(fog_state(&svc.store.snapshot())),
        "group" | "group_selected" => true,
        "ungroup" | "ungroup_current" => false,
        other => return Err(unknown(other, "state, group, ungroup")),
    };
    let out = commit(&ed, &svc, o, super::Ctl::default(), move |s, _, _| Ok(fog_group(s, group))).await?;
    Ok(merge(out, fog_state(&svc.store.snapshot())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::{FogData, LightData};

    #[test]
    fn grouping_fog_wraps_as_the_cpp_counter_does() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::FogVolume, "a", ObjData::FogVolume(FogData::default())));
        s.selection.insert(a);
        let (b, out) = fog_group(&s, true);
        assert_eq!(out["group"], json!(0));
        assert_eq!(b.replace.len(), 1);
    }

    #[test]
    fn only_selected_lights_change_with_selected() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::Light, "a", ObjData::Light(LightData::default())));
        s.insert(Object::new(ObjClass::Light, "b", ObjData::Light(LightData::default())));
        s.selection.insert(a);
        let (b, _) = use_in_viewport(&s, false, false);
        assert_eq!(b.replace.len(), 1);
        let (b, _) = use_in_viewport(&s, true, true);
        assert!(b.replace.is_empty());
    }

    #[test]
    fn link_commands_that_need_points_say_so() {
        let s = SceneState::default();
        assert_eq!(
            link_batch(&s, LinkOp::Create1).err().map(|e| e.message),
            Some("Switch Select to Points first".into())
        );
        assert!(link_batch(&s, LinkOp::Convert1).is_ok());
    }
}
