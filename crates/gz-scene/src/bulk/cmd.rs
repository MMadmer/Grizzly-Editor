//! `aimap_tool`, `details_tool`, `wallmark_tool`: every button, switch and gesture of the three
//! panels as `action=`. The panels post these; the viewport's gestures call the same functions
//! ([`add_nodes`], [`ai_pick`], [`ai_select_frustum`], [`ai_move`], [`ai_rotate`],
//! [`slot_pick`], [`slots_select_frustum`], [`mark_add`], [`mark_move`], [`mark_pick`],
//! [`marks_select_frustum`], [`remove_selected`], [`select_all`]).

use std::sync::Arc;
use std::time::Instant;

use glam::{Vec3, Vec4};
use gz_base::{Origin, SmolStr};
use gz_editor::{Args, ArgsExt, CmdError, Editor, JobError, Registry, Reply, Value, json};
use gz_fmt_level::{AiMapNode, SceneDetails, node_flags as nf, tool_flags};
use gz_fmt_xr::ShadersXrlc;

use super::aimap::{self, AiWork, Gen, LinkMode, SIDE_FLAGS, SIDES, flags as ai_flags};
use super::collide::{CollideModel, ModelCache};
use super::details::{self, BaseImage, DetailInputs};
use super::wallmark::{self, Lay, Wallmarks};
use super::{BulkEdit, Kind, Outcome, ViewAxes, commit, set_switches};
use crate::apply::{SceneCmd, SelectMode};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::SceneState;
use crate::service::SceneService;
use crate::tools::{AiMapPart, DetailsPart, WallmarkPart};

const AREA: &str = "gz-scene";

/// Registers the three tool commands.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "aimap_tool", aimap_tool);
    reg.add(AREA, "details_tool", details_tool);
    reg.add(AREA, "wallmark_tool", wallmark_tool);
}

fn scene(ed: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
    let svc = SceneService::of(ed)?;
    if !svc.store.snapshot().valid {
        return Err(CmdError::new("no_scene", "no scene"));
    }
    Ok(svc)
}

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

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn v3(a: &Args, key: &str) -> Option<Vec3> {
    a.vec3(key).map(Vec3::from_array).filter(|v| v.is_finite())
}

fn need_v3(a: &Args, key: &str) -> Result<Vec3, CmdError> {
    v3(a, key).ok_or_else(|| CmdError::invalid(format!("missing '{key}': x,y,z")))
}

fn mode_of(a: &Args, default: SelectMode) -> SelectMode {
    match a.str("mode") {
        Some("set") => SelectMode::Set,
        Some("add") => SelectMode::Add,
        Some("remove") => SelectMode::Remove,
        Some("toggle") => SelectMode::Toggle,
        _ => default,
    }
}

/// A list of points: `positions` as `[[x,y,z], ...]` or `"x,y,z;x,y,z"`, or one `position`.
fn points(a: &Args) -> Option<Vec<Vec3>> {
    if let Some(Value::Array(items)) = a.get("positions") {
        let out: Vec<Vec3> = items
            .iter()
            .filter_map(|v| match v {
                Value::Array(c) if c.len() == 3 => {
                    let f = |i: usize| c.get(i).and_then(Value::as_f64).map(|x| x as f32);
                    Some(Vec3::new(f(0)?, f(1)?, f(2)?))
                }
                Value::String(s) => parse_v3(s),
                _ => None,
            })
            .collect();
        return Some(out);
    }
    if let Some(s) = a.str("positions") {
        return Some(s.split(';').filter_map(parse_v3).collect());
    }
    v3(a, "position").map(|p| vec![p])
}

fn parse_v3(s: &str) -> Option<Vec3> {
    let c: Vec<f32> = s.split(',').filter_map(|x| x.trim().parse().ok()).collect();
    (c.len() == 3).then(|| Vec3::new(c[0], c[1], c[2])).filter(|v| v.is_finite())
}

/// The planes of an axis-aligned box (`n . p + w >= 0` inside).
fn box_planes(min: Vec3, max: Vec3) -> [Vec4; 6] {
    [
        Vec4::new(1.0, 0.0, 0.0, -min.x),
        Vec4::new(-1.0, 0.0, 0.0, max.x),
        Vec4::new(0.0, 1.0, 0.0, -min.y),
        Vec4::new(0.0, -1.0, 0.0, max.y),
        Vec4::new(0.0, 0.0, 1.0, -min.z),
        Vec4::new(0.0, 0.0, -1.0, max.z),
    ]
}

/// The compiler shader library of the linked game (or the editor's own copy).
async fn shaders(ed: &Arc<Editor>) -> Option<Arc<ShadersXrlc>> {
    let (bytes, _) = gz_game::read_game_or_bundled(ed, "shaders_xrlc.xr").await.ok()?;
    ShadersXrlc::read(&bytes).ok().map(Arc::new)
}

// ---- aimap_tool ---------------------------------------------------------------------------------

/// The AI tool's snap objects' box (`Scene->GetBox(bb, m_SnapObjects)`); `None` for an empty list.
#[must_use]
pub fn ai_snap_box(state: &SceneState, lib: &Library) -> Option<Option<Aabb>> {
    let names = state.bulk.ai_map.as_ref().map(|a| a.header.snap_objects.clone()).unwrap_or_default();
    if names.is_empty() {
        return None;
    }
    let ids: Vec<crate::ObjId> = names
        .iter()
        .filter_map(|n| state.find(ObjClass::SceneObject, &n.to_lowercase()).map(|o| o.id))
        .collect();
    Some(details::objects_box(state, lib, &ids))
}

fn ai_names(state: &SceneState) -> Vec<SmolStr> {
    state.bulk.ai_map.as_ref().map(|a| a.header.snap_objects.clone()).unwrap_or_default()
}

/// The collision model of the AI tool's snap objects, built when it changed (library objects
/// loaded first).
pub async fn ai_model(ed: &Arc<Editor>, svc: &Arc<SceneService>) -> Result<Arc<CollideModel>, CmdError> {
    let snap = svc.store.snapshot();
    let names = ai_names(&snap);
    if names.is_empty() {
        return Err(CmdError::invalid("Fill snap list before generating slots!"));
    }
    svc.prefetch_library(ed, &snap).await;
    let sh = shaders(ed).await;
    let cache = ed.project()?.services.get_or_init(ModelCache::default);
    let lib = svc.library.clone();
    Ok(ed.exec.cpu(move || cache.get(&snap, &lib, &names, sh.as_deref())).await)
}

fn ai_state(s: &SceneState) -> Value {
    let t = &s.tools.bulk;
    let Some(a) = s.bulk.ai_map.as_ref() else {
        return json!({
            "nodes": 0, "selected": 0, "links": [0, 0, 0, 0, 0], "box": Value::Null,
            "ignore_constraints": t.ignore_constraints, "auto_link": t.auto_link, "link_mode": t.link_mode.name(),
            "ignored_materials": t.ignored, "snap_objects": Vec::<String>::new(),
            "draw_nodes": true, "slow_calculate": false,
        });
    };
    let h = &a.header;
    let nodes = a.all_nodes();
    let selected = nodes.iter().filter(|n| n.flags & nf::SELECTED != 0).count();
    let b = h.bbox;
    json!({
        "nodes": nodes.len(),
        "selected": selected,
        "links": aimap::link_histogram(&nodes),
        "box": if b.is_valid() { json!({ "min": [b.min.x, b.min.y, b.min.z], "max": [b.max.x, b.max.y, b.max.z] }) } else { Value::Null },
        "params": { "patch_size": h.params.patch_size, "test_height": h.params.test_height, "can_up": h.params.can_up, "can_down": h.params.can_down },
        "brush_size": h.brush_size,
        "vis_radius": h.vis_radius,
        "smooth_height": h.smooth_height,
        "draw_nodes": h.flags & ai_flags::HIDE_NODES == 0,
        "slow_calculate": h.flags & ai_flags::SLOW_CALCULATE != 0,
        "ignore_constraints": t.ignore_constraints,
        "auto_link": t.auto_link,
        "link_mode": t.link_mode.name(),
        "ignored_materials": t.ignored,
        "snap_objects": h.snap_objects,
    })
}

/// An AI map edit computed on the working copy: `f` edits it and answers the label (`None`:
/// nothing changed), the kind of change and the reply.
async fn ai_edit<F>(ed: &Arc<Editor>, svc: &Arc<SceneService>, o: Origin, f: F) -> Result<Value, CmdError>
where
    F: Fn(&mut AiWork, &SceneState) -> Result<(Option<&'static str>, Kind, Value), CmdError>
        + Send
        + Sync
        + 'static,
{
    commit(ed, svc, o, move |s, _| {
        let base = s.bulk.ai_map.as_deref();
        let mut w = AiWork::new(base);
        let (label, kind, answer) = f(&mut w, s)?;
        let Some(label) = label else { return Ok((BulkEdit::default(), answer)) };
        let mut e = BulkEdit::new(label, s.serial).with_ai(w.into_part(base));
        e.kind = kind;
        Ok((e, answer))
    })
    .await
}

/// How nodes are added.
#[derive(Clone, Debug)]
pub struct AddNodes {
    pub positions: Vec<Vec3>,
    /// `SelectObjects(false)` first: the start of a stroke.
    pub deselect: bool,
    pub brush: Option<u32>,
    pub ignore_constraints: Option<bool>,
    pub auto_link: Option<bool>,
    /// The stroke's key: its steps are one undo step.
    pub merge: Option<SmolStr>,
}

/// `TUI_ControlAIMapNodeAdd`: `AddNode` at each position (the brush past a size of one).
pub async fn add_nodes(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    req: AddNodes,
) -> Result<Value, CmdError> {
    let model = ai_model(ed, svc).await?;
    let req = Arc::new(req);
    commit(ed, svc, o, move |s, lib| {
        let base = s.bulk.ai_map.as_deref();
        let mut w = AiWork::new(base);
        w.update_box(ai_snap_box(s, lib).flatten());
        let t = &s.tools.bulk;
        let bic = req.ignore_constraints.unwrap_or(t.ignore_constraints);
        let auto = req.auto_link.unwrap_or(t.auto_link);
        let brush = req.brush.unwrap_or(w.header.brush_size).max(1);
        let g = Gen { model: &model, params: w.header.params, ignored: model.ignored_mask(&t.ignored) };
        if req.deselect {
            w.select_all(false);
        }
        let mut added = 0;
        let mut failed: Vec<&str> = Vec::new();
        for p in &req.positions {
            match w.add_node(&g, *p, bic, auto, brush) {
                Ok(n) => added += n,
                Err(e) => failed.push(e),
            }
        }
        let nodes = w.nodes.len();
        let mut e = BulkEdit::new("Add Nodes", s.serial).with_ai(w.into_part(base));
        e.merge.clone_from(&req.merge);
        if added == 0 {
            e.kind = Kind::Selection;
        }
        Ok((e, json!({ "added": added, "nodes": nodes, "failed": failed })))
    })
    .await
}

/// `PickObjects`: where a ray meets the AI tool's snap objects.
#[must_use]
pub fn ai_ground(state: &SceneState, lib: &Library, start: Vec3, dir: Vec3, range: f32) -> Option<Vec3> {
    let ids: Vec<crate::ObjId> = ai_names(state)
        .iter()
        .filter_map(|n| state.find(ObjClass::SceneObject, &n.to_lowercase()).map(|o| o.id))
        .collect();
    wallmark::ray_triangle(state, lib, &ids, start, dir, range).map(|(t, _)| start + dir * t)
}

/// `RaySelect` of nodes: without `toggle` the selection is let go first; the node hit is selected
/// (oracle: the C++ toggles and then sets the flag again, so Ctrl+click selects too).
pub async fn ai_pick(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    start: Vec3,
    dir: Vec3,
    range: f32,
    keep: bool,
) -> Result<Value, CmdError> {
    ai_edit(ed, svc, o, move |w, s| {
        if s.bulk.ai_map.as_ref().is_some_and(|a| a.header.flags & ai_flags::HIDE_NODES != 0) {
            return Ok((None, Kind::Selection, json!({ "node": Value::Null })));
        }
        if !keep {
            w.select_all(false);
        }
        let hit = w.pick(start, dir, range);
        if let Some((i, _)) = hit {
            w.select(&[i], true);
        }
        let position = hit.and_then(|(i, _)| w.nodes.get(i as usize)).map(|n| [n.pos.x, n.pos.y, n.pos.z]);
        Ok((
            Some("Select"),
            Kind::Selection,
            json!({ "node": hit.map(|(i, _)| i), "position": position, "selected": w.selected_count() }),
        ))
    })
    .await
}

/// `FrustumSelect` of nodes.
pub async fn ai_select_frustum(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    planes: Vec<Vec4>,
    on: bool,
) -> Result<Value, CmdError> {
    ai_edit(ed, svc, o, move |w, s| {
        if s.bulk.ai_map.as_ref().is_some_and(|a| a.header.flags & ai_flags::HIDE_NODES != 0) {
            return Ok((None, Kind::Selection, json!({ "count": 0 })));
        }
        let ids = w.in_frustum(&planes);
        let n = w.select(&ids, on);
        Ok((Some("Select"), Kind::Selection, json!({ "count": n, "selected": w.selected_count() })))
    })
    .await
}

/// The selected nodes moved to positions, one merged undo step per gesture.
pub async fn ai_move(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    moves: Vec<(u32, Vec3)>,
    merge: Option<SmolStr>,
) -> Result<Value, CmdError> {
    let moves = Arc::new(moves);
    commit(ed, svc, o, move |s, _| {
        let base = s.bulk.ai_map.as_deref();
        let mut w = AiWork::new(base);
        let n = w.move_nodes(&moves);
        let mut e = BulkEdit::new("Move Nodes", s.serial).with_ai(w.into_part(base));
        e.merge.clone_from(&merge);
        Ok((e, json!({ "moved": n })))
    })
    .await
}

/// The selected nodes' planes turned (`RotateProcess`), one merged undo step per gesture.
pub async fn ai_rotate(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    about_x: bool,
    angle: f32,
    merge: Option<SmolStr>,
) -> Result<Value, CmdError> {
    commit(ed, svc, o, move |s, _| {
        let base = s.bulk.ai_map.as_deref();
        let mut w = AiWork::new(base);
        let n = w.rotate_selected(about_x, angle);
        let mut e = BulkEdit::new("Rotate Nodes", s.serial).with_ai(w.into_part(base));
        e.merge.clone_from(&merge);
        Ok((e, json!({ "rotated": n })))
    })
    .await
}

/// The selected nodes with their positions (what a move drag holds).
#[must_use]
pub fn selected_nodes(state: &SceneState) -> Vec<(u32, Vec3)> {
    let Some(a) = state.bulk.ai_map.as_ref() else { return Vec::new() };
    let mut out = Vec::new();
    let mut i = 0u32;
    for t in &a.nodes {
        for n in t.iter() {
            if n.flags & nf::SELECTED != 0 {
                out.push((i, n.pos));
            }
            i += 1;
        }
    }
    out
}

/// `GenerateMap(bFromSelectedOnly)` as a job of the Task Center: the queue is worked through
/// on the CPU pool and the result goes in as one undo step. Stop keeps what was made, as the C++
/// did; a scene that changed meanwhile drops it.
pub async fn ai_generate(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    selected_only: bool,
) -> Result<Value, CmdError> {
    let snap = svc.store.snapshot();
    if ai_names(&snap).is_empty() {
        return Err(CmdError::invalid("Fill snap list before generating slots!"));
    }
    if snap.bulk.ai_map.as_ref().is_none_or(|a| a.node_count() == 0) {
        return Err(CmdError::invalid("Append at least one node."));
    }
    let (e2, s2) = (ed.clone(), svc.clone());
    let title =
        if selected_only { "Generating the AI map from the selected nodes" } else { "Generating the AI map" };
    let started = ed.jobs.start("aimap_generate", Some("scene"), title, move |ctx| async move {
        let (ed, svc) = (e2, s2);
        ctx.stage("Building the collision model");
        let model = ai_model(&ed, &svc).await.map_err(|e| JobError::new(e.message))?;
        let snap = svc.store.snapshot();
        let lib = svc.library.clone();
        ctx.stage("Building nodes");
        let begun = Instant::now();
        let c = ctx.clone();
        let (part, flood, nodes) = ed
            .exec
            .cpu(move || {
                let base = snap.bulk.ai_map.as_deref();
                let mut w = AiWork::new(base);
                w.update_box(ai_snap_box(&snap, &lib).flatten());
                let size = w.header.bbox.max - w.header.bbox.min;
                let ps = w.header.params.patch_size;
                let estimated = ((size.x / ps) * (size.z / ps)).max(1.0);
                let g = Gen { model: &model, params: w.header.params, ignored: model.ignored_mask(&snap.tools.bulk.ignored) };
                let flood = w.generate(&g, selected_only, &|done, total| {
                    let p = 0.1 * done as f32 / total.max(1) as f32 + 0.9 * total as f32 / estimated;
                    c.fraction(p.clamp(0.0, 1.0));
                    c.stage(format!("Building nodes: {total}"));
                    !c.cancel.is_cancelled()
                });
                let nodes = w.nodes.len();
                (w.into_part(base), flood, (nodes, snap.serial))
            })
            .await;
        let secs = begun.elapsed().as_secs_f32();
        ed.log.info(format!("-building time: {secs:.3}"));
        let (count, serial) = nodes;
        let mut e = BulkEdit::new(if selected_only { "Generate Selected" } else { "Generate AI Map" }, serial).with_ai(part);
        e.kind = Kind::Step;
        match svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await {
            Ok(_) => {}
            Err(err) if err.code == "busy" => {
                return Err(JobError::new("The scene changed while the AI map was generated: nothing was changed. Generate it again."));
            }
            Err(err) => return Err(JobError::new(err.message)),
        }
        Ok(json!({ "added": flood.added, "nodes": count, "seconds": secs, "stopped": flood.stopped }))
    });
    let id = started.map_err(|busy| CmdError::busy(format!("{} is still running", busy.title)))?;
    wait_job(ed, id).await
}

async fn wait_job(ed: &Arc<Editor>, id: gz_editor::JobId) -> Result<Value, CmdError> {
    match ed.jobs.wait(id).await {
        Some(i) if i.state == gz_editor::JobState::Done => Ok(i.result.map_or(Value::Null, |v| (*v).clone())),
        Some(i) if i.state == gz_editor::JobState::Cancelled => Err(CmdError::new("cancelled", "Stopped.")),
        Some(i) => Err(CmdError::invalid(i.error.unwrap_or_else(|| "The command failed.".into()))),
        None => Err(CmdError::internal("the job is gone")),
    }
}

/// A side of the map from an argument: `left`, `forward`, `right`, `back`, `all`; with
/// `camera=true` the pad's `away`, `left`, `right`, `towards` turned by the camera's heading.
fn side_flags(ed: &Editor, a: &Args) -> Result<u8, CmdError> {
    let side = a.need("side")?.to_ascii_lowercase();
    if side == "all" {
        return Ok(SIDE_FLAGS.iter().fold(0, |m, f| m | f));
    }
    let k = if a.flag("camera") {
        let pad = match side.as_str() {
            "away" | "up" | "forward" => 1,
            "left" => 0,
            "right" => 2,
            "towards" | "down" | "back" => 3,
            _ => return Err(CmdError::invalid("side is away, left, right, towards or all")),
        };
        let heading = ViewAxes::of(ed).get().map_or(0.0, |v| v.heading);
        aimap::side_for_heading(pad, heading)
    } else {
        SIDES
            .iter()
            .position(|s| *s == side)
            .ok_or_else(|| CmdError::invalid("side is left, forward, right, back or all"))?
    };
    Ok(SIDE_FLAGS[k])
}

async fn aimap_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::AiMap).await?;
    let action = a.str_or("action", "state").to_owned();
    match action.as_str() {
        "state" => Ok(ai_state(&svc.store.snapshot())),
        "set" => {
            let s = svc.store.snapshot();
            let mode = match a.str("link_mode").or_else(|| a.str("mode")) {
                Some(m) => Some(
                    LinkMode::parse(m)
                        .ok_or_else(|| CmdError::invalid("link_mode is add, remove or invert"))?,
                ),
                None => None,
            };
            let (bic, auto) = (a.bool("ignore_constraints"), a.bool("auto_link"));
            if bic.is_some() || auto.is_some() || mode.is_some() {
                set_switches(&svc, o, |t| {
                    if let Some(v) = bic {
                        t.ignore_constraints = v;
                    }
                    if let Some(v) = auto {
                        t.auto_link = v;
                    }
                    if let Some(m) = mode {
                        t.link_mode = m;
                    }
                })
                .await?;
            }
            let header_args = [
                "brush_size",
                "can_up",
                "can_down",
                "vis_radius",
                "smooth_height",
                "draw_nodes",
                "slow_calculate",
            ];
            if header_args.iter().any(|k| a.has(k)) {
                let mut part = s
                    .bulk
                    .ai_map
                    .as_deref()
                    .cloned()
                    .unwrap_or_else(|| AiMapPart::from_nodes(Default::default(), &[], None));
                let h = &mut part.header;
                if let Some(v) = a.u32("brush_size") {
                    h.brush_size = v.clamp(1, 100);
                }
                if let Some(v) = a.f32("can_up") {
                    h.params.can_up = v.clamp(0.0, 10.0);
                }
                if let Some(v) = a.f32("can_down") {
                    h.params.can_down = v.clamp(0.0, 10.0);
                }
                if let Some(v) = a.f32("vis_radius") {
                    h.vis_radius = v.clamp(10.0, 250.0);
                }
                if let Some(v) = a.f32("smooth_height") {
                    h.smooth_height = v.clamp(0.1, 100.0);
                }
                if let Some(v) = a.bool("draw_nodes") {
                    h.flags =
                        if v { h.flags & !ai_flags::HIDE_NODES } else { h.flags | ai_flags::HIDE_NODES };
                }
                if let Some(v) = a.bool("slow_calculate") {
                    h.flags = if v {
                        h.flags | ai_flags::SLOW_CALCULATE
                    } else {
                        h.flags & !ai_flags::SLOW_CALCULATE
                    };
                }
                // the packing changes with the parameters
                let nodes = part.all_nodes();
                let part = AiMapPart::from_nodes(part.header.clone(), &nodes, s.bulk.ai_map.as_deref());
                let e = BulkEdit::new("AI Map Settings", svc.store.snapshot().serial).with_ai(part);
                svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await?;
            }
            Ok(ai_state(&svc.store.snapshot()))
        }
        "generate" | "generate_full" | "generate_selected" => {
            let selected = action == "generate_selected" || a.flag("selected");
            ai_generate(&ed, &svc, o, selected).await
        }
        "clear" => {
            let snap = svc.store.snapshot();
            let names = ai_names(&snap);
            let lib = svc.library.clone();
            let has_names = !names.is_empty();
            if has_names {
                svc.prefetch_library(&ed, &snap).await;
            }
            ai_edit(&ed, &svc, o, move |w, s| {
                let n = w.nodes.len();
                // `Clear()`: the nodes go, the box is the snap objects' again
                w.clear_nodes();
                w.header.bbox = Aabb::INVALID;
                w.update_box(ai_snap_box(s, &lib).flatten());
                Ok((Some("Clear AI Map"), Kind::Step, json!({ "removed": n })))
            })
            .await
        }
        "reset_selected" => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.reset_selected();
                Ok((if n > 0 { Some("Reset Selected") } else { None }, Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        "smooth_selected" => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.smooth_selected();
                Ok((if n > 0 { Some("Smooth Selected") } else { None }, Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        "select_links" => {
            let links = a.usize("links").ok_or_else(|| CmdError::invalid("missing 'links': 0, 1 or 2"))?;
            if links > 4 {
                return Err(CmdError::invalid("links is 0 to 4"));
            }
            ai_edit(&ed, &svc, o, move |w, _| {
                let n = w.select_by_links(links);
                Ok((Some("Select By Links"), Kind::Step, json!({ "selected": n })))
            })
            .await
        }
        "select_all" | "deselect_all" | "invert_selection" => {
            let act = action.clone();
            ai_edit(&ed, &svc, o, move |w, _| {
                match act.as_str() {
                    "select_all" => w.select_all(true),
                    "deselect_all" => w.select_all(false),
                    _ => w.invert_selection(),
                }
                Ok((Some("Select"), Kind::Selection, json!({ "selected": w.selected_count() })))
            })
            .await
        }
        "select_nodes" => {
            let mode = mode_of(&a, SelectMode::Set);
            let ids: Option<Vec<u32>> = a
                .get("nodes")
                .and_then(Value::as_array)
                .map(|v| v.iter().filter_map(Value::as_u64).map(|x| x as u32).collect());
            let pts = points(&a);
            if ids.is_none() && pts.is_none() {
                return Err(CmdError::invalid("missing 'nodes' (indices) or 'positions'"));
            }
            ai_edit(&ed, &svc, o, move |w, _| {
                let ps = w.ps();
                let mut which: Vec<u32> = ids.clone().unwrap_or_default();
                for p in pts.iter().flatten() {
                    if let Some(i) = w.find_node(*p, ps * 0.5) {
                        which.push(i);
                    }
                }
                match mode {
                    SelectMode::Set => {
                        w.select_all(false);
                        w.select(&which, true);
                    }
                    SelectMode::Add => {
                        w.select(&which, true);
                    }
                    SelectMode::Remove => {
                        w.select(&which, false);
                    }
                    SelectMode::Toggle => {
                        for i in &which {
                            if let Some(n) = w.nodes.get_mut(*i as usize) {
                                n.flags ^= nf::SELECTED;
                            }
                        }
                    }
                }
                Ok((
                    Some("Select"),
                    Kind::Selection,
                    json!({ "matched": which.len(), "selected": w.selected_count() }),
                ))
            })
            .await
        }
        "pick" => {
            let (start, dir) = (need_v3(&a, "start")?, need_v3(&a, "dir")?);
            ai_pick(
                &ed,
                &svc,
                o,
                start,
                dir.normalize_or_zero(),
                a.f32("range").unwrap_or(1000.0),
                a.flag("keep"),
            )
            .await
        }
        "select_box" => {
            let (min, max) = (need_v3(&a, "min")?, need_v3(&a, "max")?);
            ai_select_frustum(
                &ed,
                &svc,
                o,
                box_planes(min.min(max), min.max(max)).to_vec(),
                a.str("mode") != Some("remove"),
            )
            .await
        }
        "links" | "link" => {
            let flags = side_flags(&ed, &a)?;
            let snap = svc.store.snapshot();
            let mode = match a.str("mode") {
                Some(m) => {
                    LinkMode::parse(m).ok_or_else(|| CmdError::invalid("mode is add, remove or invert"))?
                }
                None => snap.tools.bulk.link_mode,
            };
            let bic = a.bool("ignore_constraints").unwrap_or(snap.tools.bulk.ignore_constraints);
            ai_edit(&ed, &svc, o, move |w, _| {
                let n = w.make_links(flags, mode, bic);
                Ok((Some("Edit Links"), Kind::Step, json!({ "changed": n, "mode": mode.name() })))
            })
            .await
        }
        "remove_links" => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.remove_links();
                Ok((Some("Remove Links"), Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        "invert_links" => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.invert_links();
                Ok((Some("Invert Links"), Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        "add_nodes" | "add_node" => {
            let positions =
                points(&a).ok_or_else(|| CmdError::invalid("missing 'position' or 'positions'"))?;
            let req = AddNodes {
                positions,
                deselect: a.bool("deselect").unwrap_or(true),
                brush: a.u32("brush_size").or_else(|| a.u32("brush")),
                ignore_constraints: a.bool("ignore_constraints"),
                auto_link: a.bool("auto_link"),
                merge: None,
            };
            add_nodes(&ed, &svc, o, req).await
        }
        "remove_selected" | "delete_selected" => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.remove_selected();
                Ok((
                    if n > 0 { Some("Remove Nodes") } else { None },
                    Kind::Step,
                    json!({ "removed": n, "nodes": w.nodes.len() }),
                ))
            })
            .await
        }
        "move_selected" => {
            let delta = need_v3(&a, "delta")?;
            let moves: Vec<(u32, Vec3)> =
                selected_nodes(&svc.store.snapshot()).into_iter().map(|(i, p)| (i, p + delta)).collect();
            ai_move(&ed, &svc, o, moves, None).await
        }
        "rotate_selected" => {
            let about_x = a.str_or("axis", "z").eq_ignore_ascii_case("x");
            let angle = a.f32("angle").ok_or_else(|| CmdError::invalid("missing 'angle' (degrees)"))?;
            ai_rotate(&ed, &svc, o, about_x, angle.to_radians(), None).await
        }
        "ignore_material" | "unignore_material" | "clear_ignored" => {
            let name = a.str("name").map(SmolStr::new);
            if action != "clear_ignored" && name.is_none() {
                return Err(CmdError::invalid("missing 'name' argument"));
            }
            let act = action.clone();
            let t = set_switches(&svc, o, move |t| match act.as_str() {
                "ignore_material" => {
                    if let Some(n) = &name
                        && !t.ignored.iter().any(|x| x.eq_ignore_ascii_case(n))
                    {
                        t.ignored.push(n.clone());
                    }
                }
                "unignore_material" => {
                    t.ignored.retain(|x| name.as_ref().is_none_or(|n| !x.eq_ignore_ascii_case(n)))
                }
                _ => t.ignored.clear(),
            })
            .await?;
            Ok(json!({ "ignored_materials": t.ignored }))
        }
        other => Err(unknown(
            other,
            "state, set, generate, generate_selected, clear, reset_selected, smooth_selected, select_links, select_all, deselect_all, invert_selection, select_nodes, pick, select_box, links, remove_links, invert_links, add_nodes, remove_selected, move_selected, rotate_selected, ignore_material, unignore_material, clear_ignored",
        )),
    }
}

// ---- details_tool -------------------------------------------------------------------------------

fn details_of(s: &SceneState) -> SceneDetails {
    s.bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default()
}

fn details_state(s: &SceneState) -> Value {
    let d = details_of(s);
    let sel = &s.tools.bulk.slots_selected;
    let selected = sel.iter().take(d.slots.len()).filter(|x| **x).count();
    let filled = d.slots.iter().filter(|x| !x.is_empty()).count();
    json!({
        "slots": d.slots.len(),
        "filled": filled,
        "size": [d.header.size_x, d.header.size_z],
        "offset": [d.header.offs_x, d.header.offs_z],
        "selected": selected,
        "objects": d.indices.objects.iter().map(|o| o.reference.as_str()).collect::<Vec<_>>(),
        "colors": d.indices.colors.len(),
        "base_texture": d.base_texture,
        "density": d.density.unwrap_or(0.3),
        "draw_objects": d.flags & tool_flags::OBJECTS_DRAW != 0,
        "draw_base_texture": d.flags & tool_flags::BASE_TEXTURE_DRAW != 0,
        "base_texture_blended": d.flags & tool_flags::BASE_TEXTURE_BLENDED != 0,
        "draw_slot_boxes": d.flags & tool_flags::SLOT_BOXES_DRAW != 0,
        "snap_objects": d.snap_objects,
    })
}

/// The base texture's pixels: the project's, the editor's, then the linked game's
/// `textures\<name>.dds` (`ImageLib.LoadTextureData` reads `$game_textures$`).
pub async fn load_base(ed: &Arc<Editor>, svc: &SceneService, name: &str) -> Option<BaseImage> {
    if name.is_empty() {
        return None;
    }
    let rel: Vec<String> = name.split(['\\', '/']).filter(|s| !s.is_empty()).map(str::to_owned).collect();
    let mut candidates = Vec::new();
    for root in [&svc.root, &svc.editor_root] {
        let mut p = root.join("gamedata").join("textures");
        for (i, part) in rel.iter().enumerate() {
            if i + 1 == rel.len() {
                p.push(format!("{part}.dds"));
            } else {
                p.push(part);
            }
        }
        candidates.push(p);
    }
    let bytes = ed.exec.io(move || candidates.iter().find_map(|p| std::fs::read(p).ok())).await;
    let bytes = match bytes {
        Some(b) => b,
        None => {
            let link = gz_game::GameLink::of(ed).ok()?;
            let files = link.files().await.ok()?;
            let key = format!("textures/{}.dds", rel.join("/").to_lowercase());
            ed.exec.io(move || files.read(&key).ok()).await?
        }
    };
    ed.exec
        .cpu(move || {
            let l = gz_fmt_tex::load_image(&bytes, Some("dds")).ok()?;
            Some(BaseImage::from_rgba(l.image.width, l.image.height, &l.image.rgba))
        })
        .await
}

fn time_seed() -> u32 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(1, |d| d.as_secs() as u32)
}

/// A regeneration of the details as a job: `Initialize` / `Reinitialize` (`objects_only`
/// false) or `UpdateObjects` of every slot or the selected ones.
pub async fn details_job(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    which: &'static str,
    seed: Option<u32>,
) -> Result<Value, CmdError> {
    let snap = svc.store.snapshot();
    let d = details_of(&snap);
    let all = matches!(which, "first_initialize" | "reinitialize");
    if all && d.snap_objects.is_empty() {
        return Err(CmdError::invalid(details::msg::SNAP_EMPTY));
    }
    let tex = d.base_texture.clone().unwrap_or_default();
    let base = load_base(ed, svc, &tex).await;
    let Some(base) = base else {
        return Err(CmdError::invalid(if all {
            details::msg::BASE_EMPTY
        } else {
            details::msg::BASE_INVALID
        }));
    };
    let seed = seed.unwrap_or_else(time_seed);
    let label = match which {
        "first_initialize" => "First Initialize",
        "reinitialize" => "Reinitialize All",
        "reinitialize_objects" => "Reinitialize Objects",
        _ => "Reinitialize Selected",
    };
    let (e2, s2) = (ed.clone(), svc.clone());
    let started = ed.jobs.start(
        "details_generate",
        Some("scene"),
        format!("Details: {label}"),
        move |ctx| async move {
            let (ed, svc) = (e2, s2);
            ctx.stage("Loading the library objects");
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            ctx.stage("Updating the slots");
            let lib = svc.library.clone();
            let c = ctx.clone();
            let base = Arc::new(base);
            let res = ed
                .exec
                .cpu(move || {
                    let d = details_of(&snap);
                    let inp = DetailInputs { base: &base, seed };
                    let progress = |n: usize, t: usize| c.progress(n, t);
                    let out = if all {
                        details::initialize(&snap, &lib, &d, &inp, &progress)
                    } else {
                        let sel = details::resized(&snap.tools.bulk.slots_selected, d.slots.len());
                        details::update_objects(
                            &d,
                            (which == "reinitialize_selected").then_some(&sel[..]),
                            &inp,
                            &progress,
                        )
                    };
                    out.map(|m| {
                        (
                            m,
                            snap.serial,
                            details::resized(&snap.tools.bulk.slots_selected, d.slots.len()),
                            snap.tools.bulk.clone(),
                        )
                    })
                })
                .await
                .map_err(JobError::new)?;
            let (model, serial, _, mut tools) = res;
            tools.slots_selected = details::resized(&tools.slots_selected, model.slots.len());
            let slots = model.slots.len();
            let filled = model.slots.iter().filter(|x| !x.is_empty()).count();
            let mut e =
                BulkEdit::new(label, serial).with_details(DetailsPart { raw: None, model: Arc::new(model) });
            e.tools = Some(tools);
            match svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await {
                Ok(_) => Ok(json!({ "slots": slots, "filled": filled, "seed": seed })),
                Err(err) if err.code == "busy" => Err(JobError::new(
                    "The scene changed while the details were made: nothing was changed. Do it again.",
                )),
                Err(err) => Err(JobError::new(err.message)),
            }
        },
    );
    let id = started.map_err(|busy| CmdError::busy(format!("{} is still running", busy.title)))?;
    wait_job(ed, id).await
}

/// A details edit computed from the snapshot.
async fn details_edit<F>(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    f: F,
) -> Result<Value, CmdError>
where
    F: Fn(&SceneState) -> Result<Outcome, CmdError> + Send + Sync + 'static,
{
    commit(ed, svc, o, move |s, _| f(s)).await
}

/// The slot selection changed by `f` (no undo step).
fn with_slots(s: &SceneState, f: impl FnOnce(&SceneDetails, &mut Vec<bool>) -> usize) -> Outcome {
    let d = details_of(s);
    let mut sel: Vec<bool> = details::resized(&s.tools.bulk.slots_selected, d.slots.len()).to_vec();
    let n = f(&d, &mut sel);
    let mut tools = s.tools.bulk.clone();
    tools.slots_selected = sel.iter().copied().collect();
    let count = sel.iter().filter(|x| **x).count();
    let mut e = BulkEdit::new("Select Slots", s.serial);
    e.tools = Some(tools);
    e.kind = Kind::Selection;
    (e, json!({ "changed": n, "selected": count }))
}

fn apply_mode(v: &mut bool, mode: SelectMode) {
    *v = match mode {
        SelectMode::Set | SelectMode::Add => true,
        SelectMode::Remove => false,
        SelectMode::Toggle => !*v,
    };
}

/// `EDetailManager::RaySelect`: the slot box hit; only while the slot boxes are drawn.
pub async fn slot_pick(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    start: Vec3,
    dir: Vec3,
    range: f32,
    toggle: bool,
) -> Result<Value, CmdError> {
    details_edit(ed, svc, o, move |s| {
        Ok(with_slots(s, |d, sel| {
            if d.flags & tool_flags::SLOT_BOXES_DRAW == 0 {
                return 0;
            }
            if !toggle {
                sel.iter_mut().for_each(|x| *x = false);
            }
            match details::pick_slot(d, start, dir, range) {
                Some(i) => {
                    if let Some(x) = sel.get_mut(i) {
                        apply_mode(x, if toggle { SelectMode::Toggle } else { SelectMode::Add });
                    }
                    1
                }
                None => 0,
            }
        }))
    })
    .await
}

/// `EDetailManager::FrustumSelect`.
pub async fn slots_select_frustum(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    planes: Vec<Vec4>,
    mode: SelectMode,
) -> Result<Value, CmdError> {
    details_edit(ed, svc, o, move |s| {
        Ok(with_slots(s, |d, sel| {
            if d.flags & tool_flags::SLOT_BOXES_DRAW == 0 {
                return 0;
            }
            let found = details::slots_in_frustum(d, &planes);
            for i in &found {
                if let Some(x) = sel.get_mut(*i) {
                    apply_mode(x, mode);
                }
            }
            found.len()
        }))
    })
    .await
}

fn details_set_flag(d: &mut SceneDetails, bit: u32, on: Option<bool>) {
    if let Some(v) = on {
        d.flags = if v { d.flags | bit } else { d.flags & !bit };
    }
}

async fn details_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::DetailObjects).await?;
    let action = a.str_or("action", "state").to_owned();
    match action.as_str() {
        "state" => Ok(details_state(&svc.store.snapshot())),
        "first_initialize" | "initialize" => {
            details_job(&ed, &svc, o, "first_initialize", a.u32("seed")).await
        }
        "reinitialize" | "reinitialize_all" => details_job(&ed, &svc, o, "reinitialize", a.u32("seed")).await,
        "reinitialize_objects" => details_job(&ed, &svc, o, "reinitialize_objects", a.u32("seed")).await,
        "reinitialize_selected" => details_job(&ed, &svc, o, "reinitialize_selected", a.u32("seed")).await,
        "update_renderer" | "show" => {
            // the viewport follows the details by itself; Update Renderer is the view flag
            details_edit(&ed, &svc, o, |s| {
                let mut d = details_of(s);
                if d.flags & tool_flags::OBJECTS_DRAW != 0 {
                    return Ok((BulkEdit::default(), json!({ "draw_objects": true })));
                }
                d.flags |= tool_flags::OBJECTS_DRAW;
                let e = BulkEdit::new("Show Details", s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, json!({ "draw_objects": true })))
            })
            .await
        }
        "set" => {
            let texture = a.str("base_texture").map(SmolStr::new);
            let density = a.f32("density");
            let flags = [
                (tool_flags::OBJECTS_DRAW, a.bool("draw_objects")),
                (tool_flags::BASE_TEXTURE_DRAW, a.bool("draw_base_texture")),
                (tool_flags::BASE_TEXTURE_BLENDED, a.bool("base_texture_blended")),
                (tool_flags::SLOT_BOXES_DRAW, a.bool("draw_slot_boxes")),
            ];
            let out = details_edit(&ed, &svc, o, move |s| {
                let mut d = details_of(s);
                let mut note = Value::Null;
                if let Some(t) = &texture
                    && d.base_texture.as_deref().unwrap_or("") != t.as_str()
                {
                    // `OnBaseTextureChange`: the slots name no object until they are made again
                    d = details::invalidate_slots(&d);
                    d.base_texture = (!t.is_empty()).then(|| t.clone());
                    note = json!("Texture changed. Reinitialize objects.");
                }
                if let Some(v) = density {
                    d.density = Some(v);
                }
                for (bit, on) in flags {
                    details_set_flag(&mut d, bit, on);
                }
                let e = BulkEdit::new("Detail Settings", s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, json!({ "note": note })))
            })
            .await?;
            let mut st = details_state(&svc.store.snapshot());
            if let (Value::Object(m), Some(n)) = (&mut st, out.get("note").filter(|n| !n.is_null())) {
                m.insert("note".into(), n.clone());
            }
            Ok(st)
        }
        "select_all" | "deselect_all" | "invert_selection" => {
            let act = action.clone();
            details_edit(&ed, &svc, o, move |s| {
                Ok(with_slots(s, |d, sel| {
                    match act.as_str() {
                        "select_all" => sel.iter_mut().for_each(|x| *x = true),
                        "deselect_all" => sel.iter_mut().for_each(|x| *x = false),
                        // `InvertSelection`: only while the boxes are drawn
                        _ if d.flags & tool_flags::SLOT_BOXES_DRAW != 0 => {
                            sel.iter_mut().for_each(|x| *x = !*x)
                        }
                        _ => {}
                    }
                    sel.len()
                }))
            })
            .await
        }
        "select_slots" => {
            let mode = mode_of(&a, SelectMode::Set);
            let list: Vec<(i64, i64)> = match a.get("slots") {
                Some(Value::Array(v)) => v
                    .iter()
                    .filter_map(|x| match x {
                        Value::Array(p) if p.len() == 2 => Some((p[0].as_i64()?, p[1].as_i64()?)),
                        Value::Number(n) => n.as_i64().map(|i| (i, -1)),
                        _ => None,
                    })
                    .collect(),
                _ => return Err(CmdError::invalid("missing 'slots': [index, ...] or [[x, z], ...]")),
            };
            details_edit(&ed, &svc, o, move |s| {
                Ok(with_slots(s, |d, sel| {
                    if mode == SelectMode::Set {
                        sel.iter_mut().for_each(|x| *x = false);
                    }
                    let mut n = 0;
                    for (a, b) in &list {
                        let i = if *b < 0 {
                            usize::try_from(*a).ok()
                        } else {
                            u32::try_from(*a)
                                .ok()
                                .zip(u32::try_from(*b).ok())
                                .and_then(|(x, z)| d.header.slot_index(x, z))
                        };
                        if let Some(x) = i.and_then(|i| sel.get_mut(i)) {
                            apply_mode(x, mode);
                            n += 1;
                        }
                    }
                    n
                }))
            })
            .await
        }
        "pick" => {
            let (start, dir) = (need_v3(&a, "start")?, need_v3(&a, "dir")?);
            slot_pick(
                &ed,
                &svc,
                o,
                start,
                dir.normalize_or_zero(),
                a.f32("range").unwrap_or(1000.0),
                a.flag("toggle"),
            )
            .await
        }
        "select_box" => {
            let (min, max) = (need_v3(&a, "min")?, need_v3(&a, "max")?);
            slots_select_frustum(
                &ed,
                &svc,
                o,
                box_planes(min.min(max), min.max(max)).to_vec(),
                mode_of(&a, SelectMode::Add),
            )
            .await
        }
        // the Detail Objects dialog's lists
        "add_objects" | "remove_objects" => {
            let refs =
                a.list("objects").ok_or_else(|| CmdError::invalid("missing 'objects': [reference, ...]"))?;
            let add = action == "add_objects";
            details_edit(&ed, &svc, o, move |s| {
                let mut d = details_of(s);
                let mut n = 0;
                for r in &refs {
                    let r = r.replace('/', "\\");
                    let at = d.indices.objects.iter().position(|x| x.reference.eq_ignore_ascii_case(&r));
                    match (add, at) {
                        (true, None)
                            if d.indices.objects.len() < usize::from(gz_fmt_level::DetailSlot::ID_EMPTY) =>
                        {
                            d.indices.objects.push(gz_fmt_level::DetailObjectRef {
                                reference: SmolStr::new(&r),
                                ..Default::default()
                            });
                            n += 1;
                        }
                        (false, Some(i)) => {
                            let gone = d.indices.objects.remove(i).reference;
                            for c in &mut d.indices.colors {
                                c.objects.retain(|x| !x.eq_ignore_ascii_case(&gone));
                            }
                            n += 1;
                        }
                        _ => {}
                    }
                }
                // `RemoveDO`: the slots' object ids mean other objects now
                if !add && n > 0 {
                    d = details::invalidate_slots(&d);
                }
                let label = if add { "Add Detail Objects" } else { "Remove Detail Objects" };
                let e = BulkEdit::new(label, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, json!({ "changed": n })))
            })
            .await
        }
        "set_color" | "remove_color" => {
            let text = a.need("color")?.trim().trim_start_matches('#').trim_start_matches("0x").to_owned();
            let mut color = u32::from_str_radix(&text, 16)
                .map_err(|_| CmdError::invalid("color is AARRGGBB or RRGGBB in hex"))?;
            if text.len() <= 6 {
                color |= 0xff00_0000;
            }
            let refs = a.list("objects").unwrap_or_default();
            let set = action == "set_color";
            details_edit(&ed, &svc, o, move |s| {
                let mut d = details_of(s);
                d.indices.colors.retain(|c| c.color != color);
                if set {
                    let objects: Vec<SmolStr> = refs
                        .iter()
                        .map(|r| r.replace('/', "\\"))
                        .filter(|r| d.indices.objects.iter().any(|x| x.reference.eq_ignore_ascii_case(r)))
                        .map(SmolStr::new)
                        .collect();
                    d.indices.colors.push(gz_fmt_level::ColorIndex { color, objects });
                }
                let e = BulkEdit::new(if set { "Set Colour Index" } else { "Remove Colour Index" }, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, json!({ "color": format!("{color:08X}") })))
            })
            .await
        }
        "clear_slots" | "clear" => {
            let all = action == "clear";
            details_edit(&ed, &svc, o, move |s| {
                let d = details_of(s);
                let (slots, objects) = (d.slots.len(), d.indices.objects.len());
                let model = if all { SceneDetails::default() } else { details::clear_slots(&d) };
                let mut tools = s.tools.bulk.clone();
                tools.slots_selected = Arc::from([]);
                let mut e = BulkEdit::new(if all { "Clear Details" } else { "Clear Slots" }, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(model) });
                e.tools = Some(tools);
                Ok((e, json!({ "slots": slots, "objects": if all { objects } else { 0 } })))
            })
            .await
        }
        other => Err(unknown(
            other,
            "state, first_initialize, reinitialize, reinitialize_objects, reinitialize_selected, update_renderer, set, select_all, deselect_all, invert_selection, select_slots, pick, select_box, add_objects, remove_objects, set_color, remove_color, clear_slots, clear",
        )),
    }
}

// ---- wallmark_tool ------------------------------------------------------------------------------

fn marks_of(s: &SceneState) -> Result<Wallmarks, CmdError> {
    match s.bulk.wallmarks.as_ref() {
        None => Ok(Wallmarks::default()),
        Some(p) => p.model.as_ref().map(|m| (**m).clone()).ok_or_else(|| {
            CmdError::invalid("The wallmarks of this scene could not be read; they are kept as they are.")
        }),
    }
}

fn wallmark_state(s: &SceneState) -> Value {
    let Ok(w) = marks_of(s) else {
        return json!({ "readable": false, "marks": s.bulk.wallmarks.as_ref().map_or(0, |p| p.count) });
    };
    let slots: Vec<Value> = w
        .slots
        .iter()
        .map(|sl| json!({ "shader": sl.shader, "texture": sl.texture, "marks": sl.items.len(), "selected": sl.items.iter().filter(|m| m.selected()).count() }))
        .collect();
    json!({
        "readable": true,
        "marks": w.count(),
        "selected": w.selected_count(),
        "slots": slots,
        "draw": w.draws(),
        "alignment": if w.flags & wallmark::flags::AXIS_ALIGN != 0 { "world" } else { "camera" },
        "width": w.width,
        "height": w.height,
        "rotate": w.rotate.to_degrees(),
        "shader": w.shader,
        "texture": w.texture,
        "snap_list_used": s.tools.snap_use,
    })
}

/// A wallmark edit computed from the snapshot: `f` edits the model.
async fn marks_edit<F>(ed: &Arc<Editor>, svc: &Arc<SceneService>, o: Origin, f: F) -> Result<Value, CmdError>
where
    F: Fn(&mut Wallmarks, &SceneState, &Library) -> Result<(Option<&'static str>, Kind, Value), CmdError>
        + Send
        + Sync
        + 'static,
{
    commit(ed, svc, o, move |s, lib| {
        let mut w = marks_of(s)?;
        let (label, kind, answer) = f(&mut w, s, lib)?;
        let Some(label) = label else { return Ok((BulkEdit::default(), answer)) };
        let mut e = BulkEdit::new(label, s.serial).with_wallmarks(WallmarkPart::of_model(w));
        e.kind = kind;
        Ok((e, answer))
    })
    .await
}

fn lay_of(w: &Wallmarks, ed: &Editor) -> Lay {
    let cam = ViewAxes::of(ed).get();
    Lay {
        width: w.width,
        height: w.height,
        rotate: w.rotate,
        axis_align: w.flags & wallmark::flags::AXIS_ALIGN != 0,
        camera_top: cam.map_or(Vec3::Y, |c| c.top),
        camera_right: cam.map_or(Vec3::X, |c| c.right),
    }
}

/// `TUI_ControlWallmarkAdd::Start`: the marks are let go, a new one laid where the ray meets the
/// snap list's objects. Answers `added: null` when the ray met nothing.
pub async fn mark_add(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    start: Vec3,
    dir: Vec3,
    range: f32,
) -> Result<Value, CmdError> {
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let w0 = marks_of(&snap)?;
    if w0.shader.is_empty() || w0.texture.is_empty() {
        return Err(CmdError::invalid("Select texture before add wallmark."));
    }
    let lay = lay_of(&w0, ed);
    marks_edit(ed, svc, o, move |w, s, lib| {
        let Some(mark) = wallmark::mark_at(s, lib, start, dir, range, &lay).map_err(CmdError::invalid)?
        else {
            return Ok((None, Kind::Step, json!({ "added": Value::Null })));
        };
        w.select_all(false);
        let (sh, tx) = (w.shader.clone(), w.texture.clone());
        let (si, mi, replaced) = w.insert(&sh, &tx, mark);
        Ok((
            Some("Add Wallmark"),
            Kind::Step,
            json!({ "added": { "slot": si, "index": mi, "replaced": replaced }, "marks": w.count() }),
        ))
    })
    .await
}

/// `MoveSelectedWallmarkTo`: the one selected mark laid again where the ray meets the geometry,
/// with its own shader, texture, size and rotation.
pub async fn mark_move(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    start: Vec3,
    dir: Vec3,
    range: f32,
) -> Result<Value, CmdError> {
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let w0 = marks_of(&snap)?;
    let base_lay = lay_of(&w0, ed);
    marks_edit(ed, svc, o, move |w, s, lib| {
        if !w.draws() {
            return Ok((None, Kind::Step, json!({ "moved": false })));
        }
        let Some((si, mi)) = w.single_selected() else {
            return Err(CmdError::invalid("Select one wallmark."));
        };
        let (sh, tx, old) = {
            let sl = &w.slots[si];
            (sl.shader.clone(), sl.texture.clone(), sl.items[mi].clone())
        };
        let lay = Lay { width: old.w, height: old.h, rotate: old.r, ..base_lay };
        let Some(mark) = wallmark::mark_at(s, lib, start, dir, range, &lay).map_err(CmdError::invalid)?
        else {
            return Ok((None, Kind::Step, json!({ "moved": false })));
        };
        if let Some(slot) = w.slots.get_mut(si) {
            slot.items.swap_remove(mi);
        }
        let (si, mi, _) = w.insert(&sh, &tx, mark);
        Ok((Some("Move Wallmark"), Kind::Step, json!({ "moved": true, "slot": si, "index": mi })))
    })
    .await
}

/// `ESceneWallmarkTool::RaySelect`: only while the marks are drawn.
pub async fn mark_pick(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    start: Vec3,
    dir: Vec3,
    range: f32,
    toggle: bool,
) -> Result<Value, CmdError> {
    marks_edit(ed, svc, o, move |w, _, _| {
        if !w.draws() {
            return Ok((None, Kind::Selection, json!({ "mark": Value::Null })));
        }
        if !toggle {
            w.select_all(false);
        }
        let hit = w.pick(start, dir, range);
        if let Some((si, mi, _)) = hit {
            // oracle: as in the node pick, the C++ sets the flag after toggling it
            w.select(&[(si, mi)], SelectMode::Add);
        }
        Ok((
            Some("Select"),
            Kind::Selection,
            json!({ "mark": hit.map(|(s, m, _)| [s, m]), "selected": w.selected_count() }),
        ))
    })
    .await
}

/// `ESceneWallmarkTool::FrustumSelect`.
pub async fn marks_select_frustum(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    planes: Vec<Vec4>,
    mode: SelectMode,
) -> Result<Value, CmdError> {
    marks_edit(ed, svc, o, move |w, _, _| {
        if !w.draws() {
            return Ok((None, Kind::Selection, json!({ "count": 0 })));
        }
        let found = w.in_frustum(&planes);
        let n = w.select(&found, mode);
        Ok((Some("Select"), Kind::Selection, json!({ "count": n, "selected": w.selected_count() })))
    })
    .await
}

async fn wallmark_tool(ed: Arc<Editor>, a: Args, o: Origin) -> Reply {
    let svc = scene_for(&ed, &a, o, ObjClass::Wallmark).await?;
    let action = a.str_or("action", "state").to_owned();
    // a ray: start + dir, or a point with the surface's normal, or a point seen from the camera
    let ray = |a: &Args| -> Result<(Vec3, Vec3), CmdError> {
        if let (Some(s), Some(d)) = (v3(a, "start"), v3(a, "dir")) {
            return Ok((s, d.normalize_or_zero()));
        }
        let p = need_v3(a, "position")?;
        if let Some(n) = v3(a, "normal") {
            let n = n.normalize_or_zero();
            return Ok((p + n, -n));
        }
        let cam = ViewAxes::of(&ed).get().map_or(p + Vec3::Y * 2.0, |c| c.position);
        let d = (p - cam).normalize_or_zero();
        Ok((p - d, d))
    };
    match action.as_str() {
        "state" => Ok(wallmark_state(&svc.store.snapshot())),
        "set" => {
            let draw = a.bool("draw");
            let align = match a.str("alignment") {
                Some("world" | "axis" | "world_axis") => Some(true),
                Some("camera") => Some(false),
                Some(_) => return Err(CmdError::invalid("alignment is camera or world")),
                None => None,
            };
            let (width, height, rotate) = (a.f32("width"), a.f32("height"), a.f32("rotate"));
            let (shader, texture) = (a.str("shader").map(SmolStr::new), a.str("texture").map(SmolStr::new));
            marks_edit(&ed, &svc, o, move |w, _, _| {
                if let Some(v) = draw {
                    w.flags =
                        if v { w.flags | wallmark::flags::DRAW } else { w.flags & !wallmark::flags::DRAW };
                }
                if let Some(v) = align {
                    w.flags = if v {
                        w.flags | wallmark::flags::AXIS_ALIGN
                    } else {
                        w.flags & !wallmark::flags::AXIS_ALIGN
                    };
                }
                if let Some(v) = width {
                    w.width = v.clamp(0.01, 10.0);
                }
                if let Some(v) = height {
                    w.height = v.clamp(0.01, 10.0);
                }
                if let Some(v) = rotate {
                    w.rotate = v.to_radians();
                }
                if let Some(v) = &shader {
                    w.shader = v.clone();
                }
                if let Some(v) = &texture {
                    w.texture = v.clone();
                }
                Ok((Some("Wallmark Settings"), Kind::Step, Value::Null))
            })
            .await?;
            Ok(wallmark_state(&svc.store.snapshot()))
        }
        "add" => {
            let (start, dir) = ray(&a)?;
            mark_add(&ed, &svc, o, start, dir, a.f32("range").unwrap_or(1000.0)).await
        }
        "move" | "move_selected" => {
            let (start, dir) = ray(&a)?;
            mark_move(&ed, &svc, o, start, dir, a.f32("range").unwrap_or(1000.0)).await
        }
        "pick" => {
            let (start, dir) = (need_v3(&a, "start")?, need_v3(&a, "dir")?);
            mark_pick(
                &ed,
                &svc,
                o,
                start,
                dir.normalize_or_zero(),
                a.f32("range").unwrap_or(1000.0),
                a.flag("toggle"),
            )
            .await
        }
        "select_box" => {
            let (min, max) = (need_v3(&a, "min")?, need_v3(&a, "max")?);
            marks_select_frustum(
                &ed,
                &svc,
                o,
                box_planes(min.min(max), min.max(max)).to_vec(),
                mode_of(&a, SelectMode::Add),
            )
            .await
        }
        "select_all" | "deselect_all" | "invert_selection" => {
            let act = action.clone();
            marks_edit(&ed, &svc, o, move |w, _, _| {
                if !w.draws() {
                    return Ok((None, Kind::Selection, json!({ "selected": 0 })));
                }
                match act.as_str() {
                    "select_all" => w.select_all(true),
                    "deselect_all" => w.select_all(false),
                    _ => w.invert_selection(),
                }
                Ok((Some("Select"), Kind::Selection, json!({ "selected": w.selected_count() })))
            })
            .await
        }
        "select_marks" => {
            let mode = mode_of(&a, SelectMode::Set);
            let list: Vec<(usize, usize)> = match a.get("marks") {
                Some(Value::Array(v)) => v
                    .iter()
                    .filter_map(|x| match x {
                        Value::Array(p) if p.len() == 2 => {
                            Some((p[0].as_u64()? as usize, p[1].as_u64()? as usize))
                        }
                        _ => None,
                    })
                    .collect(),
                _ => return Err(CmdError::invalid("missing 'marks': [[slot, index], ...]")),
            };
            marks_edit(&ed, &svc, o, move |w, _, _| {
                let n = w.select(&list, mode);
                Ok((Some("Select"), Kind::Selection, json!({ "changed": n, "selected": w.selected_count() })))
            })
            .await
        }
        "remove_selected" | "delete_selected" => {
            marks_edit(&ed, &svc, o, |w, _, _| {
                if !w.draws() {
                    return Ok((None, Kind::Step, json!({ "removed": 0 })));
                }
                let n = w.remove_selected();
                Ok((
                    if n > 0 { Some("Remove Wallmarks") } else { None },
                    Kind::Step,
                    json!({ "removed": n, "marks": w.count() }),
                ))
            })
            .await
        }
        other => Err(unknown(
            other,
            "state, set, add, move, pick, select_box, select_all, deselect_all, invert_selection, select_marks, remove_selected",
        )),
    }
}

// ---- what the viewport's keys reach -------------------------------------------------------------

/// Delete with a bulk tool current (`RemoveSelection` of the tool): `None` for another tool.
pub async fn remove_selected(ed: &Arc<Editor>, o: Origin) -> Option<Result<Value, CmdError>> {
    let svc = scene(ed).ok()?;
    match svc.store.snapshot().tools.target {
        ObjClass::AiMap => Some(
            ai_edit(ed, &svc, o, |w, _| {
                let n = w.remove_selected();
                Ok((if n > 0 { Some("Remove Nodes") } else { None }, Kind::Step, json!({ "removed": n })))
            })
            .await,
        ),
        ObjClass::Wallmark => Some(
            marks_edit(ed, &svc, o, |w, _, _| {
                if !w.draws() {
                    return Ok((None, Kind::Step, json!({ "removed": 0 })));
                }
                let n = w.remove_selected();
                Ok((if n > 0 { Some("Remove Wallmarks") } else { None }, Kind::Step, json!({ "removed": n })))
            })
            .await,
        ),
        // `EDetailManager::RemoveSelection` does nothing
        ObjClass::DetailObjects => Some(Ok(json!({ "removed": 0 }))),
        _ => None,
    }
}

/// Select All, Deselect All, Invert Selection with a bulk tool current: `None` for another tool.
pub async fn select_all(ed: &Arc<Editor>, o: Origin, which: &'static str) -> Option<Result<Value, CmdError>> {
    let svc = scene(ed).ok()?;
    let target = svc.store.snapshot().tools.target;
    let cmd = match target {
        ObjClass::AiMap => "aimap_tool",
        ObjClass::DetailObjects => "details_tool",
        ObjClass::Wallmark => "wallmark_tool",
        _ => return None,
    };
    let mut args = Args::new();
    args.insert("action".into(), Value::String(which.into()));
    let reply = ed.call(cmd, args, o).await;
    Some(if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        Err(CmdError::invalid(
            reply.get("error").and_then(Value::as_str).unwrap_or("The command failed.").to_owned(),
        ))
    })
}

/// The nodes of the AI map as the viewport draws and picks them.
#[must_use]
pub fn ai_nodes(state: &SceneState) -> Vec<AiMapNode> {
    state.bulk.ai_map.as_ref().map(|a| a.all_nodes()).unwrap_or_default()
}

/// Sentences for the panels' confirmations.
#[must_use]
pub fn count_line(n: usize, word: &str) -> String {
    plural(n, word)
}
