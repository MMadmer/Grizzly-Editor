//! `scene.ai_map`, `scene.details`, `scene.wallmarks`: the painted layers of the level - the AI
//! map, the detail objects, the wallmarks - on the ground ([`super::ground`]). The viewport's
//! gestures in the painting modes call the same functions ([`add_nodes`], [`ai_pick`],
//! [`ai_select_frustum`], [`ai_move`], [`ai_rotate`], [`slot_pick`], [`slots_select_frustum`],
//! [`mark_add`], [`mark_move`], [`mark_pick`], [`marks_select_frustum`], [`remove_selected`],
//! [`select_all`]).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use glam::{Vec3, Vec4};
use gz_base::{Origin, SmolStr};
use gz_editor::{CmdError, Editor, JobError, JsonSchema, Registry, Reply, Spec, Value, json};
use gz_fmt_level::{AiMapNode, SceneAiMap, SceneDetails, node_flags as nf, tool_flags};
use gz_fmt_xr::ShadersXrlc;
use serde::Deserialize;

use super::aimap::{self, AiWork, Gen, LinkMode, SIDE_FLAGS, SIDES, flags as ai_flags};
use super::collide::{CollideModel, ModelCache};
use super::details::{self, BaseImage, DetailInputs, Stroke};
use super::ground;
use super::wallmark::{self, Lay, Wallmarks};
use super::{BulkEdit, Kind, Outcome, ViewAxes, commit, set_switches};
use crate::apply::{SceneCmd, SelectMode};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::{SceneMode, SceneState};
use crate::service::SceneService;
use crate::tools::{AiMapPart, DetailsPart, WallmarkPart};

/// Registers the three layer commands.
pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::write(
            "scene.ai_map",
            "The AI map: generate it over the ground, paint and link nodes, its settings, what it holds.",
        )
        .example("action=generate can_up=1.5 can_down=4"),
        |ed, p: AiMapParams, o| async move { ai_map(ed, p, o).await },
    );
    reg.cmd(
        Spec::write(
            "scene.details",
            "The detail objects (grass): their meshes, painting and erasing them on the ground, a colour-index import.",
        )
        .example("action=paint object=trees\\grass\\grass_kolosok position=10,0,5 radius=4 density=0.8"),
        |ed, p: DetailsParams, o| async move { details_cmd(ed, p, o).await },
    );
    reg.cmd(
        Spec::write(
            "scene.wallmarks",
            "The wallmarks: lay one on the ground, move, select or remove them, their settings.",
        )
        .example("action=add position=3,0.5,2 normal=0,1,0"),
        |ed, p: WallmarkParams, o| async move { wallmarks_cmd(ed, p, o).await },
    );
}

fn scene(ed: &Arc<Editor>) -> Result<Arc<SceneService>, CmdError> {
    let svc = SceneService::of(ed)?;
    if !svc.store.snapshot().valid {
        return Err(CmdError::not_found("No scene is open."));
    }
    Ok(svc)
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n == 1 { "" } else { "s" })
}

fn v3(v: Option<[f32; 3]>) -> Option<Vec3> {
    v.map(Vec3::from_array).filter(|v| v.is_finite())
}

fn need(v: Option<[f32; 3]>, key: &str) -> Result<Vec3, CmdError> {
    v3(v).ok_or_else(|| CmdError::invalid(format!("missing '{key}': x,y,z")))
}

fn select_mode(m: Option<&str>, default: SelectMode) -> Result<SelectMode, CmdError> {
    Ok(match m {
        None => default,
        Some("set") => SelectMode::Set,
        Some("add") => SelectMode::Add,
        Some("remove") => SelectMode::Remove,
        Some("toggle") => SelectMode::Toggle,
        Some(other) => {
            return Err(CmdError::invalid(format!("mode is set, add, remove or toggle, not '{other}'")));
        }
    })
}

/// The points of a request: `positions`, else `position`.
fn points(positions: &Option<Vec<[f32; 3]>>, position: Option<[f32; 3]>) -> Option<Vec<Vec3>> {
    match positions {
        Some(list) => Some(list.iter().copied().filter_map(|p| v3(Some(p))).collect()),
        None => v3(position).map(|p| vec![p]),
    }
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

async fn wait_job(ed: &Arc<Editor>, id: gz_editor::JobId, wait: Option<bool>) -> Result<Value, CmdError> {
    if wait == Some(false) {
        return Ok(json!({ "job": id.0 }));
    }
    match ed.jobs.wait(id).await {
        Some(i) if i.state == gz_editor::JobState::Done => {
            let mut v = i.result.map_or(json!({}), |v| (*v).clone());
            v["job"] = json!(id.0);
            Ok(v)
        }
        Some(i) if i.state == gz_editor::JobState::Cancelled => {
            Err(CmdError::new("failed", "Stopped.").with_data(json!({ "cancelled": true })))
        }
        Some(i) => Err(CmdError::new("failed", i.error.unwrap_or_else(|| "The command failed.".into()))),
        None => Err(CmdError::internal("the job is gone")),
    }
}

// ---- the AI map ---------------------------------------------------------------------------------

/// The ground the AI map lies on: the map's own list when it keeps one, inside its bounds volume.
#[must_use]
pub fn ai_ground_ids(state: &SceneState, lib: &Library) -> Vec<crate::ObjId> {
    let names = state.bulk.ai_map.as_ref().map(|a| a.header.snap_objects.clone()).unwrap_or_default();
    let explicit = ground::effective(state, lib, &names);
    ground::objects(state, lib, explicit.as_deref(), ground::ai_bounds(state).as_ref())
}

/// The box an AI map is generated in: the ground's, cut to the bounds volume.
#[must_use]
pub fn ai_box(state: &SceneState, lib: &Library) -> Option<Aabb> {
    let ids = ai_ground_ids(state, lib);
    ground::ground_box(state, lib, &ids, ground::ai_bounds(state).as_ref())
}

/// The collision model of the AI map's ground, built when it changed (library objects loaded
/// first).
pub async fn ai_model(ed: &Arc<Editor>, svc: &Arc<SceneService>) -> Result<Arc<CollideModel>, CmdError> {
    let snap = svc.store.snapshot();
    svc.prefetch_library(ed, &snap).await;
    let sh = shaders(ed).await;
    let cache = ed.project()?.services.get_or_init(ModelCache::default);
    let lib = svc.library.clone();
    let model = ed
        .exec
        .cpu(move || {
            let names = ground::names(&snap, &ai_ground_ids(&snap, &lib));
            (!names.is_empty()).then(|| cache.get(&snap, &lib, &names, sh.as_deref()))
        })
        .await;
    model.ok_or_else(|| CmdError::invalid("The scene has no static objects for the AI map to lie on."))
}

fn ai_info(s: &SceneState, lib: &Library) -> Value {
    let t = &s.tools.bulk;
    let names = s.bulk.ai_map.as_ref().map(|a| a.header.snap_objects.clone()).unwrap_or_default();
    let explicit = ground::effective(s, lib, &names);
    let bounds = ground::ai_bounds(s);
    let ground = json!({
        "objects": explicit.as_ref().map_or(Value::Null, |n| json!(n)),
        "all_static": explicit.is_none(),
        "bounds": bounds.map_or(Value::Null, |b| json!({ "min": [b.min.x, b.min.y, b.min.z], "max": [b.max.x, b.max.y, b.max.z] })),
    });
    let Some(a) = s.bulk.ai_map.as_ref() else {
        return json!({
            "nodes": 0, "selected": 0, "links": [0, 0, 0, 0, 0], "box": Value::Null, "ground": ground,
            "ignore_constraints": t.ignore_constraints, "auto_link": t.auto_link, "link_mode": t.link_mode.name(),
            "ignored_materials": t.ignored, "draw_nodes": true, "slow_calculate": false,
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
        "ground": ground,
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
    /// The selection let go first: the start of a stroke.
    pub deselect: bool,
    pub brush: Option<u32>,
    pub ignore_constraints: Option<bool>,
    pub auto_link: Option<bool>,
    /// The stroke's key: its steps are one undo step.
    pub merge: Option<SmolStr>,
}

/// The brush: a node at each position (patches around it past a size of one).
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
        w.update_box(ai_box(s, lib));
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

/// Where a ray meets the AI map's ground.
#[must_use]
pub fn ai_ground(state: &SceneState, lib: &Library, start: Vec3, dir: Vec3, range: f32) -> Option<Vec3> {
    let ids = ai_ground_ids(state, lib);
    wallmark::ray_triangle(state, lib, &ids, start, dir, range).map(|(t, _)| start + dir * t)
}

/// A node picked by a ray: without `keep` the selection is let go first; the node hit is
/// selected.
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

/// The nodes inside a frustum selected (or let go).
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

/// The selected nodes' planes turned, one merged undo step per gesture.
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

/// The spawn points a generation starts from: where the player and the NPCs stand.
#[must_use]
pub fn spawn_seeds(state: &SceneState) -> Vec<Vec3> {
    state.of_class(ObjClass::SpawnPoint).map(|o| o.position()).collect()
}

/// What a generation over the ground made.
#[derive(Clone, Debug)]
pub struct Generated {
    pub part: AiMapPart,
    pub nodes: usize,
    pub seeds: usize,
    pub stopped: bool,
}

/// A new AI map over the ground of `state`: the map's settings kept, the nodes made again from
/// the spawn points over the ground inside the bounds volume. Pure; `progress` answers `false` to
/// stop. `None` when there is no ground.
pub fn generate_map(
    state: &SceneState,
    lib: &Library,
    model: &CollideModel,
    progress: &(dyn Fn(f32, usize) -> bool + Sync),
) -> Option<Generated> {
    let within = ai_box(state, lib)?;
    // a little room above and below the ground for the nodes' planes
    let within = Aabb::new(within.min - Vec3::new(0.0, 2.0, 0.0), within.max + Vec3::new(0.0, 2.0, 0.0));
    let base = state.bulk.ai_map.as_deref();
    let header = base.map_or_else(SceneAiMap::default, |p| p.header.clone());
    let g = Gen { model, params: header.params, ignored: model.ignored_mask(&state.tools.bulk.ignored) };
    let seeds = aimap::seeds(model, &spawn_seeds(state), &within);
    let ps = header.params.patch_size;
    let size = within.max - within.min;
    let estimated = ((size.x / ps) * (size.z / ps)).max(1.0);
    let (w, flood, seeded) = aimap::generate_over(&g, header, within, &seeds, &|_, total| {
        progress((total as f32 / estimated).clamp(0.0, 0.99), total)
    });
    let nodes = w.nodes.len();
    Some(Generated { part: w.into_part(None), nodes, seeds: seeded, stopped: flood.stopped })
}

/// `scene.ai_map action=generate` as a job of the Task Center: the map made over the ground and
/// put into the scene as one undo step; Stop keeps what was made. A scene that changed meanwhile
/// drops it.
pub async fn ai_generate(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    selected_only: bool,
    wait: Option<bool>,
) -> Result<Value, CmdError> {
    let snap = svc.store.snapshot();
    if selected_only && snap.bulk.ai_map.as_ref().is_none_or(|a| a.node_count() == 0) {
        return Err(CmdError::invalid(
            "Select nodes of the AI map first: generating from the selection grows it from them.",
        ));
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
        let (part, added, stopped, seeds, serial) = ed
            .exec
            .cpu(move || {
                let keep_going = |f: f32, total: usize| {
                    c.fraction(f);
                    c.stage(format!("Building nodes: {total}"));
                    !c.cancel.is_cancelled()
                };
                if selected_only {
                    let base = snap.bulk.ai_map.as_deref();
                    let mut w = AiWork::new(base);
                    w.update_box(ai_box(&snap, &lib));
                    let g = Gen { model: &model, params: w.header.params, ignored: model.ignored_mask(&snap.tools.bulk.ignored) };
                    let flood = w.generate(&g, true, &|_, total| keep_going(0.5, total));
                    return Ok((w.into_part(base), flood.added, flood.stopped, 0, snap.serial));
                }
                match generate_map(&snap, &lib, &model, &keep_going) {
                    Some(g) if g.nodes > 0 => Ok((g.part, g.nodes, g.stopped, g.seeds, snap.serial)),
                    Some(_) => Err("No node could be made: the spawn points and the middle of the ground are on nothing an NPC can stand on. Place a graph point on the ground."),
                    None => Err("The scene has no static objects for the AI map to lie on."),
                }
            })
            .await
            .map_err(JobError::new)?;
        let secs = begun.elapsed().as_secs_f32();
        let mut e = BulkEdit::new(if selected_only { "Generate From Selected" } else { "Generate AI Map" }, serial).with_ai(part);
        e.kind = Kind::Step;
        match svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await {
            Ok(_) => {}
            Err(err) if err.code == "busy" => {
                return Err(JobError::new("The scene changed while the AI map was generated: nothing was changed. Generate it again."));
            }
            Err(err) => return Err(JobError::new(err.message)),
        }
        let nodes = svc.store.snapshot().bulk.ai_map.as_ref().map_or(0, |a| a.node_count());
        Ok(json!({ "added": added, "nodes": nodes, "seeds": seeds, "seconds": secs, "stopped": stopped }))
    });
    let id = started.map_err(|busy| {
        CmdError::busy(format!("{} is still running", busy.title)).with_data(json!({ "job": busy.id.0 }))
    })?;
    wait_job(ed, id, wait).await
}

/// A side of the map: `left`, `forward`, `right`, `back`, `all`; with `camera` the pad's `away`,
/// `left`, `right`, `towards` turned by the camera's heading.
fn side_flags(ed: &Editor, side: &str, camera: bool) -> Result<u8, CmdError> {
    let side = side.to_ascii_lowercase();
    if side == "all" {
        return Ok(SIDE_FLAGS.iter().fold(0, |m, f| m | f));
    }
    let k = if camera {
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

/// What `scene.ai_map` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum AiAction {
    /// What the map holds: nodes, links, the ground it lies on, its settings.
    #[default]
    Info,
    /// Changes the map's settings (the climb limits, the brush, the switches, ignored materials).
    Set,
    /// Makes the map again over the ground inside the AI Map Bounds volume (a job).
    Generate,
    /// Removes every node.
    Clear,
    /// The brush: nodes on the ground at `positions`.
    Paint,
    /// Links the selected nodes on a side: added, removed or inverted.
    Link,
    /// Selects nodes: `what` = all, none, invert, links, box, pick or nodes.
    Select,
    /// Removes the selected nodes.
    Remove,
    /// Smooths the selected nodes' heights.
    Smooth,
    /// Lays the selected nodes on the ground again.
    Reset,
    /// Moves the selected nodes by `delta`.
    Move,
    /// Turns the selected nodes' planes by `angle` degrees about `axis`.
    Rotate,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct AiMapParams {
    /// What to do; left out: info.
    #[serde(default)]
    action: AiAction,
    /// set, generate: how high a node may be above its neighbour, metres.
    #[serde(default)]
    can_up: Option<f32>,
    /// set, generate: how far a node may be below its neighbour, metres.
    #[serde(default)]
    can_down: Option<f32>,
    /// set: how far Smooth reaches, metres.
    #[serde(default)]
    smooth_height: Option<f32>,
    /// set: the radius round the camera the viewport draws nodes in, metres.
    #[serde(default)]
    vis_radius: Option<f32>,
    /// set, paint: the brush's size in patches.
    #[serde(default)]
    brush_size: Option<u32>,
    /// set: the viewport draws the nodes.
    #[serde(default)]
    draw_nodes: Option<bool>,
    /// set: the slower, more careful generation.
    #[serde(default)]
    slow_calculate: Option<bool>,
    /// set, generate: the scene objects the map lies on. Left out: what it lies on now (every
    /// static object, unless the map keeps a list); an empty list: every static object.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// generate: grow the map from the selected nodes only, keeping the rest.
    #[serde(default)]
    selected: bool,
    /// paint: where, as a list of points.
    #[serde(default)]
    positions: Option<Vec<[f32; 3]>>,
    /// paint: where, one point.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// set, paint, link: nodes made and linked whatever the height and slope limits say.
    #[serde(default)]
    ignore_constraints: Option<bool>,
    /// set, paint: a new node links to its neighbours.
    #[serde(default)]
    auto_link: Option<bool>,
    /// link: left, forward, right, back or all; with `camera`: away, left, right, towards.
    #[serde(default)]
    side: Option<String>,
    /// link: the side is the camera's.
    #[serde(default)]
    camera: bool,
    /// link: add, remove or invert (set: what the side buttons do); select: set, add, remove,
    /// toggle.
    #[serde(default)]
    mode: Option<String>,
    /// select: all, none, invert, links, box, pick or nodes.
    #[serde(default)]
    what: Option<String>,
    /// select links: the nodes with this many links (0 to 4).
    #[serde(default)]
    links: Option<usize>,
    /// select nodes: their indices.
    #[serde(default)]
    nodes: Option<Vec<u32>>,
    /// select box: one corner.
    #[serde(default)]
    min: Option<[f32; 3]>,
    /// select box: the other corner.
    #[serde(default)]
    max: Option<[f32; 3]>,
    /// select pick: where the ray starts.
    #[serde(default)]
    start: Option<[f32; 3]>,
    /// select pick: where the ray points.
    #[serde(default)]
    dir: Option<[f32; 3]>,
    /// select pick: how far the ray reaches, metres (1000).
    #[serde(default)]
    range: Option<f32>,
    /// move: by how much, metres.
    #[serde(default)]
    delta: Option<[f32; 3]>,
    /// rotate: degrees.
    #[serde(default)]
    angle: Option<f32>,
    /// rotate: x or z.
    #[serde(default)]
    axis: Option<String>,
    /// set: game materials the map is not generated on, added.
    #[serde(default)]
    ignore_materials: Option<Vec<String>>,
    /// set: game materials taken off the ignored list; an empty list clears it.
    #[serde(default)]
    unignore_materials: Option<Vec<String>>,
    /// generate: wait for the job to end (the default); false answers the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

async fn ai_map(ed: Arc<Editor>, p: AiMapParams, o: Origin) -> Reply {
    let svc = scene(&ed)?;
    match p.action {
        AiAction::Info => {
            let snap = svc.store.snapshot();
            svc.prefetch_library(&ed, &snap).await;
            let lib = svc.library.clone();
            Ok(ed.exec.cpu(move || ai_info(&snap, &lib)).await)
        }
        AiAction::Set => {
            ai_set(&svc, &p, o).await?;
            let snap = svc.store.snapshot();
            let lib = svc.library.clone();
            Ok(ed.exec.cpu(move || ai_info(&snap, &lib)).await)
        }
        AiAction::Generate => {
            if p.can_up.is_some() || p.can_down.is_some() || p.objects.is_some() {
                ai_set(
                    &svc,
                    &AiMapParams {
                        can_up: p.can_up,
                        can_down: p.can_down,
                        objects: p.objects.clone(),
                        ..AiMapParams::default()
                    },
                    o,
                )
                .await?;
            }
            ai_generate(&ed, &svc, o, p.selected, p.wait).await
        }
        AiAction::Clear => {
            ai_edit(&ed, &svc, o, move |w, _| {
                let n = w.nodes.len();
                w.clear_nodes();
                w.header.bbox = Aabb::INVALID;
                Ok((Some("Clear AI Map"), Kind::Step, json!({ "removed": n })))
            })
            .await
        }
        AiAction::Paint => {
            let positions = points(&p.positions, p.position)
                .ok_or_else(|| CmdError::invalid("paint needs 'position' or 'positions'"))?;
            let req = AddNodes {
                positions,
                deselect: true,
                brush: p.brush_size,
                ignore_constraints: p.ignore_constraints,
                auto_link: p.auto_link,
                merge: None,
            };
            add_nodes(&ed, &svc, o, req).await
        }
        AiAction::Link => {
            let flags = side_flags(&ed, p.side.as_deref().unwrap_or("all"), p.camera)?;
            let snap = svc.store.snapshot();
            let mode = match p.mode.as_deref() {
                Some(m) => {
                    LinkMode::parse(m).ok_or_else(|| CmdError::invalid("mode is add, remove or invert"))?
                }
                None => snap.tools.bulk.link_mode,
            };
            let bic = p.ignore_constraints.unwrap_or(snap.tools.bulk.ignore_constraints);
            ai_edit(&ed, &svc, o, move |w, _| {
                let n = w.make_links(flags, mode, bic);
                Ok((Some("Edit Links"), Kind::Step, json!({ "changed": n, "mode": mode.name() })))
            })
            .await
        }
        AiAction::Select => ai_select(&ed, &svc, &p, o).await,
        AiAction::Remove => {
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
        AiAction::Smooth => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.smooth_selected();
                Ok((if n > 0 { Some("Smooth Nodes") } else { None }, Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        AiAction::Reset => {
            ai_edit(&ed, &svc, o, |w, _| {
                let n = w.reset_selected();
                Ok((if n > 0 { Some("Reset Nodes") } else { None }, Kind::Step, json!({ "changed": n })))
            })
            .await
        }
        AiAction::Move => {
            let delta = need(p.delta, "delta")?;
            let moves: Vec<(u32, Vec3)> =
                selected_nodes(&svc.store.snapshot()).into_iter().map(|(i, q)| (i, q + delta)).collect();
            ai_move(&ed, &svc, o, moves, None).await
        }
        AiAction::Rotate => {
            let about_x = p.axis.as_deref().unwrap_or("z").eq_ignore_ascii_case("x");
            let angle = p.angle.ok_or_else(|| CmdError::invalid("rotate needs 'angle' (degrees)"))?;
            ai_rotate(&ed, &svc, o, about_x, angle.to_radians(), None).await
        }
    }
}

/// The settings of `scene.ai_map action=set` (and of a generate that names them).
async fn ai_set(svc: &Arc<SceneService>, p: &AiMapParams, o: Origin) -> Result<(), CmdError> {
    let mode = match p.mode.as_deref() {
        Some(m) => {
            Some(LinkMode::parse(m).ok_or_else(|| CmdError::invalid("mode is add, remove or invert"))?)
        }
        None => None,
    };
    let (bic, auto) = (p.ignore_constraints, p.auto_link);
    let (ign, unign) = (p.ignore_materials.clone(), p.unignore_materials.clone());
    if bic.is_some() || auto.is_some() || mode.is_some() || ign.is_some() || unign.is_some() {
        set_switches(svc, o, |t| {
            if let Some(v) = bic {
                t.ignore_constraints = v;
            }
            if let Some(v) = auto {
                t.auto_link = v;
            }
            if let Some(m) = mode {
                t.link_mode = m;
            }
            for n in ign.iter().flatten() {
                if !t.ignored.iter().any(|x| x.eq_ignore_ascii_case(n)) {
                    t.ignored.push(SmolStr::new(n));
                }
            }
            match &unign {
                Some(list) if list.is_empty() => t.ignored.clear(),
                Some(list) => t.ignored.retain(|x| !list.iter().any(|n| x.eq_ignore_ascii_case(n))),
                None => {}
            }
        })
        .await?;
    }
    let header_set = p.brush_size.is_some()
        || p.can_up.is_some()
        || p.can_down.is_some()
        || p.vis_radius.is_some()
        || p.smooth_height.is_some()
        || p.draw_nodes.is_some()
        || p.slow_calculate.is_some()
        || p.objects.is_some();
    if !header_set {
        return Ok(());
    }
    let s = svc.store.snapshot();
    let mut part = s
        .bulk
        .ai_map
        .as_deref()
        .cloned()
        .unwrap_or_else(|| AiMapPart::from_nodes(SceneAiMap::default(), &[], None));
    let h = &mut part.header;
    if let Some(v) = p.brush_size {
        h.brush_size = v.clamp(1, 100);
    }
    if let Some(v) = p.can_up {
        h.params.can_up = v.clamp(0.0, 10.0);
    }
    if let Some(v) = p.can_down {
        h.params.can_down = v.clamp(0.0, 10.0);
    }
    if let Some(v) = p.vis_radius {
        h.vis_radius = v.clamp(10.0, 250.0);
    }
    if let Some(v) = p.smooth_height {
        h.smooth_height = v.clamp(0.1, 100.0);
    }
    if let Some(v) = p.draw_nodes {
        h.flags = if v { h.flags & !ai_flags::HIDE_NODES } else { h.flags | ai_flags::HIDE_NODES };
    }
    if let Some(v) = p.slow_calculate {
        h.flags = if v { h.flags | ai_flags::SLOW_CALCULATE } else { h.flags & !ai_flags::SLOW_CALCULATE };
    }
    if let Some(list) = &p.objects {
        for n in list {
            if s.find(ObjClass::SceneObject, &n.to_lowercase()).is_none() {
                return Err(CmdError::not_found(format!("there is no scene object '{n}'")));
            }
        }
        h.snap_objects = list.iter().map(SmolStr::new).collect();
    }
    // the packing changes with the parameters
    let nodes = part.all_nodes();
    let part = AiMapPart::from_nodes(part.header.clone(), &nodes, s.bulk.ai_map.as_deref());
    let e = BulkEdit::new("AI Map Settings", s.serial).with_ai(part);
    svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await?;
    Ok(())
}

async fn ai_select(ed: &Arc<Editor>, svc: &Arc<SceneService>, p: &AiMapParams, o: Origin) -> Reply {
    let what = p.what.as_deref().unwrap_or("all");
    match what {
        "all" | "none" | "invert" => {
            let what = what.to_owned();
            ai_edit(ed, svc, o, move |w, _| {
                match what.as_str() {
                    "all" => w.select_all(true),
                    "none" => w.select_all(false),
                    _ => w.invert_selection(),
                }
                Ok((Some("Select"), Kind::Selection, json!({ "selected": w.selected_count() })))
            })
            .await
        }
        "links" => {
            let links = p.links.ok_or_else(|| CmdError::invalid("select links needs 'links': 0 to 4"))?;
            if links > 4 {
                return Err(CmdError::invalid("links is 0 to 4"));
            }
            ai_edit(ed, svc, o, move |w, _| {
                let n = w.select_by_links(links);
                Ok((Some("Select"), Kind::Selection, json!({ "selected": n })))
            })
            .await
        }
        "box" => {
            let (min, max) = (need(p.min, "min")?, need(p.max, "max")?);
            let on = p.mode.as_deref() != Some("remove");
            ai_select_frustum(ed, svc, o, box_planes(min.min(max), min.max(max)).to_vec(), on).await
        }
        "pick" => {
            let (start, dir) = (need(p.start, "start")?, need(p.dir, "dir")?);
            ai_pick(
                ed,
                svc,
                o,
                start,
                dir.normalize_or_zero(),
                p.range.unwrap_or(1000.0),
                p.mode.as_deref() == Some("add"),
            )
            .await
        }
        "nodes" => {
            let mode = select_mode(p.mode.as_deref(), SelectMode::Set)?;
            let ids = p.nodes.clone().unwrap_or_default();
            let pts = points(&p.positions, p.position).unwrap_or_default();
            if ids.is_empty() && pts.is_empty() {
                return Err(CmdError::invalid("select nodes needs 'nodes' (indices) or 'positions'"));
            }
            ai_edit(ed, svc, o, move |w, _| {
                let ps = w.ps();
                let mut which: Vec<u32> = ids.clone();
                for q in &pts {
                    if let Some(i) = w.find_node(*q, ps * 0.5) {
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
        other => Err(CmdError::invalid(format!(
            "what is all, none, invert, links, box, pick or nodes, not '{other}'"
        ))),
    }
}

// ---- the detail objects -------------------------------------------------------------------------

fn details_of(s: &SceneState) -> SceneDetails {
    s.bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default()
}

/// The ground the details lie on: the layer's own list when it keeps one.
#[must_use]
pub fn details_ground_ids(state: &SceneState, lib: &Library) -> Vec<crate::ObjId> {
    let d = details_of(state);
    let explicit = ground::effective(state, lib, &d.snap_objects);
    ground::objects(state, lib, explicit.as_deref(), None)
}

fn meshes_json(d: &SceneDetails) -> Vec<Value> {
    d.indices
        .objects
        .iter()
        .map(|o| {
            json!({
                "object": o.reference.as_str(),
                "density": o.density_factor,
                "min_scale": o.min_scale,
                "max_scale": o.max_scale,
                "waving": o.flags & gz_fmt_level::DO_NO_WAVING == 0,
            })
        })
        .collect()
}

fn details_info(s: &SceneState) -> Value {
    let d = details_of(s);
    let filled = d.slots.iter().filter(|x| !x.is_empty()).count();
    let per_mesh: Vec<usize> = (0..d.indices.objects.len())
        .map(|k| d.slots.iter().filter(|sl| (0..4).any(|p| usize::from(sl.id(p)) == k)).count())
        .collect();
    json!({
        "slots": d.slots.len(),
        "filled": filled,
        "size": [d.header.size_x, d.header.size_z],
        "offset": [d.header.offs_x, d.header.offs_z],
        "meshes": meshes_json(&d),
        "slots_per_mesh": per_mesh,
        "density": d.density.unwrap_or(0.3),
        "draw": d.flags & tool_flags::OBJECTS_DRAW != 0,
        "colors": d.indices.colors.len(),
        "ground": if d.snap_objects.is_empty() { Value::Null } else { json!(d.snap_objects) },
    })
}

/// A colour-index picture's pixels: the project's, the editor's, then the linked game's
/// `textures\<name>.dds`.
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

/// What a details job does to the layer.
#[derive(Clone)]
enum DetailWork {
    /// The colour-index import.
    Import { base: Arc<BaseImage>, seed: u32 },
    /// Strokes, the slot grid made over the ground first when there is none.
    Paint { strokes: Vec<Stroke>, fill: Option<(u8, f32)> },
}

/// A change of the details as a job: the slot grid over the ground is made first when the layer
/// has none (the slots' heights from the faces under them - every core, seconds on a big level).
async fn details_job(
    ed: &Arc<Editor>,
    svc: &Arc<SceneService>,
    o: Origin,
    label: &'static str,
    work: DetailWork,
    wait: Option<bool>,
) -> Result<Value, CmdError> {
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
            ctx.stage("Laying the slots on the ground");
            let lib = svc.library.clone();
            let c = ctx.clone();
            let res = ed
                .exec
                .cpu(move || {
                    let d = details_of(&snap);
                    let ids = details_ground_ids(&snap, &lib);
                    let progress = |n: usize, t: usize| c.progress(n, t);
                    // what was just laid down is shown: an imported scene may keep its details hidden
                    let shows = match &work {
                        DetailWork::Import { .. } => true,
                        DetailWork::Paint { strokes, fill } => {
                            fill.is_some() || strokes.iter().any(|s| !s.erase)
                        }
                    };
                    let mut out = match &work {
                        DetailWork::Import { base, seed } => details::initialize(
                            &snap,
                            &lib,
                            &ids,
                            &d,
                            &DetailInputs { base, seed: *seed },
                            &progress,
                        )?,
                        DetailWork::Paint { strokes, fill } => {
                            let d = if d.slots.is_empty() {
                                details::grid_over(&snap, &lib, &ids, &d, &progress)?
                            } else {
                                d
                            };
                            match fill {
                                Some((obj, density)) => fill_all(&d, *obj, *density),
                                None => details::paint(&d, strokes).0,
                            }
                        }
                    };
                    if shows {
                        out.flags |= tool_flags::OBJECTS_DRAW;
                    }
                    Ok::<_, &'static str>((out, snap.serial, snap.tools.bulk.clone()))
                })
                .await
                .map_err(JobError::new)?;
            let (model, serial, mut tools) = res;
            tools.slots_selected = details::resized(&tools.slots_selected, model.slots.len());
            let slots = model.slots.len();
            let filled = model.slots.iter().filter(|x| !x.is_empty()).count();
            let mut e =
                BulkEdit::new(label, serial).with_details(DetailsPart { raw: None, model: Arc::new(model) });
            e.tools = Some(tools);
            match svc.store.send(SceneCmd::Bulk(Box::new(e)), o).await {
                Ok(_) => Ok(json!({ "slots": slots, "filled": filled })),
                Err(err) if err.code == "busy" => Err(JobError::new(
                    "The scene changed while the details were made: nothing was changed. Do it again.",
                )),
                Err(err) => Err(JobError::new(err.message)),
            }
        },
    );
    let id = started.map_err(|busy| {
        CmdError::busy(format!("{} is still running", busy.title)).with_data(json!({ "job": busy.id.0 }))
    })?;
    wait_job(ed, id, wait).await
}

/// Every slot with ground under it holds `obj` at `density`.
fn fill_all(d: &SceneDetails, obj: u8, density: f32) -> SceneDetails {
    let (mn, mx) = (d.bbox.min, d.bbox.max);
    let center = (mn + mx) * 0.5;
    let radius = (mx - mn).length() + 4.0;
    details::paint(d, &[Stroke { object: Some(obj), center, radius, density, erase: false }]).0
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

/// A slot box picked by a ray; only while the slot boxes are drawn.
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

/// The slot boxes inside a frustum selected.
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

/// What `scene.details` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum DetailsAction {
    /// What the layer holds: slots, meshes, how many slots each covers.
    #[default]
    Info,
    /// The detail meshes with their density, scale range and waving.
    List,
    /// Adds detail meshes (`objects`: library references).
    Add,
    /// Removes detail meshes (`objects`) and what they covered.
    Remove,
    /// Changes a mesh's density, scale range or waving (`object`), or the layer's density and
    /// drawing.
    Set,
    /// Paints a mesh on the ground at `positions` within `radius`.
    Paint,
    /// Erases at `positions` within `radius`: one mesh (`object`) or all.
    Erase,
    /// Covers the whole ground with a mesh.
    Fill,
    /// Empties every slot (`all`: the mesh list too).
    Clear,
    /// Lays the details from a colour-index picture (`texture`, `colors`).
    ImportTexture,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct DetailsParams {
    /// What to do; left out: info.
    #[serde(default)]
    action: DetailsAction,
    /// add, remove: detail meshes by their library references.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// paint, erase, fill, set: one detail mesh by its library reference.
    #[serde(default)]
    object: Option<String>,
    /// paint, erase: where, as a list of points.
    #[serde(default)]
    positions: Option<Vec<[f32; 3]>>,
    /// paint, erase: where, one point.
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// paint, erase: the brush's radius, metres (4).
    #[serde(default)]
    radius: Option<f32>,
    /// paint, fill: how dense, 0..1 (1). add, set with `object`: the mesh's density factor. set
    /// without `object`: the layer's objects per square metre.
    #[serde(default)]
    density: Option<f32>,
    /// add, set: the smallest scale an instance gets.
    #[serde(default)]
    min_scale: Option<f32>,
    /// add, set: the largest scale an instance gets.
    #[serde(default)]
    max_scale: Option<f32>,
    /// add, set: the mesh sways in the wind.
    #[serde(default)]
    waving: Option<bool>,
    /// set: the viewport draws the details.
    #[serde(default)]
    draw: Option<bool>,
    /// clear: the mesh list goes too.
    #[serde(default)]
    all: bool,
    /// import_texture: the picture, by its name under textures (`terrain\escape_details`).
    #[serde(default)]
    texture: Option<String>,
    /// import_texture: what each colour of the picture scatters: `{"RRGGBB": [reference, ...]}`.
    #[serde(default)]
    colors: Option<BTreeMap<String, Vec<String>>>,
    /// import_texture: the random seed of the scatter.
    #[serde(default)]
    seed: Option<u32>,
    /// paint, erase, fill, import_texture: wait for the job to end (the default); false answers
    /// the job at once.
    #[serde(default)]
    wait: Option<bool>,
}

fn mesh_index(d: &SceneDetails, reference: &str) -> Result<u8, CmdError> {
    let r = reference.replace('/', "\\");
    d.indices.objects.iter().position(|x| x.reference.eq_ignore_ascii_case(&r)).map(|i| i as u8).ok_or_else(
        || CmdError::not_found(format!("'{reference}' is not a detail mesh of the scene: add it first")),
    )
}

fn color_of(text: &str) -> Result<u32, CmdError> {
    let t = text.trim().trim_start_matches('#').trim_start_matches("0x");
    let mut c =
        u32::from_str_radix(t, 16).map_err(|_| CmdError::invalid("a colour is RRGGBB or AARRGGBB in hex"))?;
    if t.len() <= 6 {
        c |= 0xff00_0000;
    }
    Ok(c)
}

async fn details_cmd(ed: Arc<Editor>, p: DetailsParams, o: Origin) -> Reply {
    let svc = scene(&ed)?;
    match p.action {
        DetailsAction::Info => Ok(details_info(&svc.store.snapshot())),
        DetailsAction::List => Ok(json!({ "meshes": meshes_json(&details_of(&svc.store.snapshot())) })),
        DetailsAction::Add | DetailsAction::Remove => {
            let refs = p
                .objects
                .clone()
                .ok_or_else(|| CmdError::invalid("missing 'objects': library references"))?;
            let add = p.action == DetailsAction::Add;
            let (density, min_scale, max_scale, waving) = (p.density, p.min_scale, p.max_scale, p.waving);
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
                            let mut m = gz_fmt_level::DetailObjectRef {
                                reference: SmolStr::new(&r),
                                ..Default::default()
                            };
                            if let Some(v) = density {
                                m.density_factor = v.clamp(0.0, 1.0);
                            }
                            if let Some(v) = min_scale {
                                m.min_scale = v.max(0.01);
                            }
                            if let Some(v) = max_scale {
                                m.max_scale = v.max(m.min_scale);
                            }
                            if waving == Some(false) {
                                m.flags |= gz_fmt_level::DO_NO_WAVING;
                            }
                            d.indices.objects.push(m);
                            n += 1;
                        }
                        (false, Some(i)) => {
                            d = details::remove_mesh(&d, i as u8);
                            n += 1;
                        }
                        _ => {}
                    }
                }
                let label = if add { "Add Detail Meshes" } else { "Remove Detail Meshes" };
                let e = BulkEdit::new(label, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, json!({ "changed": n })))
            })
            .await
        }
        DetailsAction::Set => {
            let object = p.object.clone();
            let (density, min_scale, max_scale, waving, draw) =
                (p.density, p.min_scale, p.max_scale, p.waving, p.draw);
            details_edit(&ed, &svc, o, move |s| {
                let mut d = details_of(s);
                match &object {
                    Some(r) => {
                        let i = usize::from(mesh_index(&d, r)?);
                        let m = &mut d.indices.objects[i];
                        if let Some(v) = density {
                            m.density_factor = v.clamp(0.0, 1.0);
                        }
                        if let Some(v) = min_scale {
                            m.min_scale = v.max(0.01);
                        }
                        if let Some(v) = max_scale {
                            m.max_scale = v.max(m.min_scale);
                        }
                        if let Some(v) = waving {
                            m.flags = if v {
                                m.flags & !gz_fmt_level::DO_NO_WAVING
                            } else {
                                m.flags | gz_fmt_level::DO_NO_WAVING
                            };
                        }
                    }
                    None => {
                        if let Some(v) = density {
                            d.density = Some(v.max(0.0));
                        }
                    }
                }
                if let Some(v) = draw {
                    d.flags = if v {
                        d.flags | tool_flags::OBJECTS_DRAW
                    } else {
                        d.flags & !tool_flags::OBJECTS_DRAW
                    };
                }
                let e = BulkEdit::new("Detail Settings", s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                Ok((e, Value::Null))
            })
            .await?;
            Ok(details_info(&svc.store.snapshot()))
        }
        DetailsAction::Paint | DetailsAction::Erase => {
            let erase = p.action == DetailsAction::Erase;
            let d = details_of(&svc.store.snapshot());
            let object = match &p.object {
                Some(r) => Some(mesh_index(&d, r)?),
                None if erase => None,
                None => return Err(CmdError::invalid("paint needs 'object': a detail mesh of the scene")),
            };
            let at = points(&p.positions, p.position)
                .ok_or_else(|| CmdError::invalid("missing 'position' or 'positions'"))?;
            let radius = p.radius.unwrap_or(4.0).clamp(0.5, 200.0);
            let density = p.density.unwrap_or(1.0);
            let strokes: Vec<Stroke> =
                at.into_iter().map(|center| Stroke { object, center, radius, density, erase }).collect();
            if d.slots.is_empty() && !erase {
                return details_job(&ed, &svc, o, "Paint", DetailWork::Paint { strokes, fill: None }, p.wait)
                    .await;
            }
            details_edit(&ed, &svc, o, move |s| {
                let (out, n) = details::paint(&details_of(s), &strokes);
                let e = BulkEdit::new(if erase { "Erase Details" } else { "Paint Details" }, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(out) });
                Ok((e, json!({ "slots_changed": n })))
            })
            .await
        }
        DetailsAction::Fill => {
            let d = details_of(&svc.store.snapshot());
            let object =
                mesh_index(&d, p.object.as_deref().ok_or_else(|| CmdError::invalid("fill needs 'object'"))?)?;
            let work =
                DetailWork::Paint { strokes: Vec::new(), fill: Some((object, p.density.unwrap_or(1.0))) };
            details_job(&ed, &svc, o, "Fill", work, p.wait).await
        }
        DetailsAction::Clear => {
            let all = p.all;
            details_edit(&ed, &svc, o, move |s| {
                let d = details_of(s);
                let (slots, meshes) = (d.slots.len(), d.indices.objects.len());
                let model = if all { SceneDetails::default() } else { details::clear_slots(&d) };
                let mut tools = s.tools.bulk.clone();
                tools.slots_selected = Arc::from([]);
                let mut e = BulkEdit::new(if all { "Clear Details" } else { "Clear Slots" }, s.serial)
                    .with_details(DetailsPart { raw: None, model: Arc::new(model) });
                e.tools = Some(tools);
                Ok((e, json!({ "slots": slots, "meshes": if all { meshes } else { 0 } })))
            })
            .await
        }
        DetailsAction::ImportTexture => {
            let texture =
                p.texture.clone().ok_or_else(|| CmdError::invalid("import_texture needs 'texture'"))?;
            if let Some(colors) = &p.colors {
                let mut parsed: Vec<gz_fmt_level::ColorIndex> = Vec::new();
                for (c, refs) in colors {
                    parsed.push(gz_fmt_level::ColorIndex {
                        color: color_of(c)?,
                        objects: refs.iter().map(|r| SmolStr::new(r.replace('/', "\\"))).collect(),
                    });
                }
                details_edit(&ed, &svc, o, move |s| {
                    let mut d = details_of(s);
                    d.indices.colors = parsed.clone();
                    let e = BulkEdit::new("Colour Indices", s.serial)
                        .with_details(DetailsPart { raw: None, model: Arc::new(d) });
                    Ok((e, Value::Null))
                })
                .await?;
            }
            let base = load_base(&ed, &svc, &texture).await.ok_or_else(|| {
                CmdError::not_found(format!("the picture textures\\{texture}.dds could not be read"))
            })?;
            let seed = p.seed.unwrap_or_else(time_seed);
            details_job(
                &ed,
                &svc,
                o,
                "Import Colour Indices",
                DetailWork::Import { base: Arc::new(base), seed },
                p.wait,
            )
            .await
        }
    }
}

// ---- the wallmarks ------------------------------------------------------------------------------

fn marks_of(s: &SceneState) -> Result<Wallmarks, CmdError> {
    match s.bulk.wallmarks.as_ref() {
        None => Ok(Wallmarks::default()),
        Some(p) => p.model.as_ref().map(|m| (**m).clone()).ok_or_else(|| {
            CmdError::invalid("The wallmarks of this scene could not be read; they are kept as they are.")
        }),
    }
}

fn wallmark_info(s: &SceneState) -> Value {
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

/// A new mark where the ray meets the ground, the others let go. Answers `added: null` when the
/// ray met nothing.
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
        return Err(CmdError::invalid("Choose the wallmark's texture first."));
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

/// The one selected mark laid again where the ray meets the ground, with its own shader,
/// texture, size and rotation.
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

/// A mark picked by a ray; only while the marks are drawn.
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

/// The marks inside a frustum selected.
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

/// What `scene.wallmarks` does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WallmarkAction {
    /// The marks and the settings a new one is laid with.
    #[default]
    Info,
    /// The settings a new mark is laid with: texture, shader, size, rotation, alignment, drawing.
    Set,
    /// Lays a mark where a ray meets the ground (`start` + `dir`, or `position` with `normal`).
    Add,
    /// Lays the one selected mark again where the ray meets the ground.
    Move,
    /// Selects marks: `what` = all, none, invert, box, pick or marks.
    Select,
    /// Removes the selected marks.
    Remove,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WallmarkParams {
    /// What to do; left out: info.
    #[serde(default)]
    action: WallmarkAction,
    /// set: the texture a new mark shows.
    #[serde(default)]
    texture: Option<String>,
    /// set: the engine shader of a new mark.
    #[serde(default)]
    shader: Option<String>,
    /// set: a new mark's width, metres.
    #[serde(default)]
    width: Option<f32>,
    /// set: a new mark's height, metres.
    #[serde(default)]
    height: Option<f32>,
    /// set: a new mark's rotation, degrees.
    #[serde(default)]
    rotate: Option<f32>,
    /// set: camera (the mark faces the camera's up) or world (the world's axes).
    #[serde(default)]
    alignment: Option<String>,
    /// set: the viewport draws the marks.
    #[serde(default)]
    draw: Option<bool>,
    /// add, move, select pick: where the ray starts.
    #[serde(default)]
    start: Option<[f32; 3]>,
    /// add, move, select pick: where the ray points.
    #[serde(default)]
    dir: Option<[f32; 3]>,
    /// add, move: a point on the ground (with `normal`, or seen from the camera).
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// add, move: the ground's normal at `position`.
    #[serde(default)]
    normal: Option<[f32; 3]>,
    /// add, move, select pick: how far the ray reaches, metres (1000).
    #[serde(default)]
    range: Option<f32>,
    /// select: all, none, invert, box, pick or marks.
    #[serde(default)]
    what: Option<String>,
    /// select marks: `[[slot, index], ...]`.
    #[serde(default)]
    marks: Option<Vec<[usize; 2]>>,
    /// select: set, add, remove or toggle.
    #[serde(default)]
    mode: Option<String>,
    /// select box: one corner.
    #[serde(default)]
    min: Option<[f32; 3]>,
    /// select box: the other corner.
    #[serde(default)]
    max: Option<[f32; 3]>,
}

async fn wallmarks_cmd(ed: Arc<Editor>, p: WallmarkParams, o: Origin) -> Reply {
    let svc = scene(&ed)?;
    // a ray: start + dir, or a point with the surface's normal, or a point seen from the camera
    let ray = |p: &WallmarkParams| -> Result<(Vec3, Vec3), CmdError> {
        if let (Some(s), Some(d)) = (v3(p.start), v3(p.dir)) {
            return Ok((s, d.normalize_or_zero()));
        }
        let at = need(p.position, "position")?;
        if let Some(n) = v3(p.normal) {
            let n = n.normalize_or_zero();
            return Ok((at + n, -n));
        }
        let cam = ViewAxes::of(&ed).get().map_or(at + Vec3::Y * 2.0, |c| c.position);
        let d = (at - cam).normalize_or_zero();
        Ok((at - d, d))
    };
    let range = p.range.unwrap_or(1000.0);
    match p.action {
        WallmarkAction::Info => Ok(wallmark_info(&svc.store.snapshot())),
        WallmarkAction::Set => {
            let align = match p.alignment.as_deref() {
                Some("world" | "axis") => Some(true),
                Some("camera") => Some(false),
                Some(_) => return Err(CmdError::invalid("alignment is camera or world")),
                None => None,
            };
            let (draw, width, height, rotate) = (p.draw, p.width, p.height, p.rotate);
            // a name of the game as the game spells it: back slashes, no blanks around it
            let spelled =
                |s: &Option<String>| s.as_deref().map(|v| SmolStr::new(v.trim().replace('/', "\\")));
            let (shader, texture) = (spelled(&p.shader), spelled(&p.texture));
            // a texture nobody has draws nothing: said before a mark wears it
            if let Some(t) = texture.as_deref().filter(|t| !t.is_empty())
                && let Some(names) = crate::names::NameSources::names(&ed).await
                && names.has("texture", t) == Some(false)
            {
                let near = names.nearest("texture", t);
                let hint = if near.is_empty() {
                    "`asset.find kind=texture` lists what there is".to_owned()
                } else {
                    format!("nearest: {}", near.join(", "))
                };
                return Err(CmdError::not_found(format!(
                    "neither the game nor the project has the texture '{t}': {hint}"
                )));
            }
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
            Ok(wallmark_info(&svc.store.snapshot()))
        }
        WallmarkAction::Add => {
            let (start, dir) = ray(&p)?;
            mark_add(&ed, &svc, o, start, dir, range).await
        }
        WallmarkAction::Move => {
            let (start, dir) = ray(&p)?;
            mark_move(&ed, &svc, o, start, dir, range).await
        }
        WallmarkAction::Select => match p.what.as_deref().unwrap_or("all") {
            "all" | "none" | "invert" => {
                let what = p.what.clone().unwrap_or_else(|| "all".into());
                marks_edit(&ed, &svc, o, move |w, _, _| {
                    if !w.draws() {
                        return Ok((None, Kind::Selection, json!({ "selected": 0 })));
                    }
                    match what.as_str() {
                        "all" => w.select_all(true),
                        "none" => w.select_all(false),
                        _ => w.invert_selection(),
                    }
                    Ok((Some("Select"), Kind::Selection, json!({ "selected": w.selected_count() })))
                })
                .await
            }
            "pick" => {
                let (start, dir) = (need(p.start, "start")?, need(p.dir, "dir")?);
                mark_pick(
                    &ed,
                    &svc,
                    o,
                    start,
                    dir.normalize_or_zero(),
                    range,
                    p.mode.as_deref() == Some("toggle"),
                )
                .await
            }
            "box" => {
                let (min, max) = (need(p.min, "min")?, need(p.max, "max")?);
                let mode = select_mode(p.mode.as_deref(), SelectMode::Add)?;
                marks_select_frustum(&ed, &svc, o, box_planes(min.min(max), min.max(max)).to_vec(), mode)
                    .await
            }
            "marks" => {
                let mode = select_mode(p.mode.as_deref(), SelectMode::Set)?;
                let list: Vec<(usize, usize)> = p
                    .marks
                    .clone()
                    .ok_or_else(|| CmdError::invalid("select marks needs 'marks': [[slot, index], ...]"))?
                    .into_iter()
                    .map(|[a, b]| (a, b))
                    .collect();
                marks_edit(&ed, &svc, o, move |w, _, _| {
                    let n = w.select(&list, mode);
                    Ok((
                        Some("Select"),
                        Kind::Selection,
                        json!({ "changed": n, "selected": w.selected_count() }),
                    ))
                })
                .await
            }
            other => Err(CmdError::invalid(format!(
                "what is all, none, invert, box, pick or marks, not '{other}'"
            ))),
        },
        WallmarkAction::Remove => {
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
    }
}

// ---- what the viewport's keys reach -------------------------------------------------------------

/// Delete in a painting mode: the layer's selected things go (`None` in another mode).
pub async fn remove_selected(ed: &Arc<Editor>, o: Origin) -> Option<Result<Value, CmdError>> {
    let svc = scene(ed).ok()?;
    match svc.store.snapshot().tools.mode {
        SceneMode::AiMap => Some(
            ai_edit(ed, &svc, o, |w, _| {
                let n = w.remove_selected();
                Ok((if n > 0 { Some("Remove Nodes") } else { None }, Kind::Step, json!({ "removed": n })))
            })
            .await,
        ),
        SceneMode::Wallmarks => Some(
            marks_edit(ed, &svc, o, |w, _, _| {
                if !w.draws() {
                    return Ok((None, Kind::Step, json!({ "removed": 0 })));
                }
                let n = w.remove_selected();
                Ok((if n > 0 { Some("Remove Wallmarks") } else { None }, Kind::Step, json!({ "removed": n })))
            })
            .await,
        ),
        // the details have no selection that goes
        SceneMode::Details => Some(Ok(json!({ "removed": 0 }))),
        _ => None,
    }
}

/// Select All, Deselect All, Invert Selection in a painting mode (`None` in another mode).
pub async fn select_all(ed: &Arc<Editor>, o: Origin, which: &'static str) -> Option<Result<Value, CmdError>> {
    let svc = scene(ed).ok()?;
    let what = match which {
        "select_all" => "all",
        "deselect_all" => "none",
        _ => "invert",
    };
    let mode = svc.store.snapshot().tools.mode;
    Some(match mode {
        SceneMode::AiMap => {
            ai_select(ed, &svc, &AiMapParams { what: Some(what.into()), ..AiMapParams::default() }, o).await
        }
        SceneMode::Wallmarks => {
            wallmarks_cmd(
                ed.clone(),
                WallmarkParams {
                    action: WallmarkAction::Select,
                    what: Some(what.into()),
                    ..WallmarkParams::default()
                },
                o,
            )
            .await
        }
        SceneMode::Details => {
            details_edit(ed, &svc, o, move |s| {
                Ok(with_slots(s, |d, sel| {
                    match what {
                        "all" => sel.iter_mut().for_each(|x| *x = true),
                        "none" => sel.iter_mut().for_each(|x| *x = false),
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
        _ => return None,
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
