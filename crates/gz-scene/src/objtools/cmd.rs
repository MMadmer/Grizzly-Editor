//! What the way paths, the shapes and the fog volumes do beyond their properties (`scene.set`)
//! and their placing (`scene.place`): `scene.way` edits a way's points and links, `scene.shape`
//! joins and splits shapes and makes the level bound, `scene.fog_group` puts fog volumes in one
//! group. The visibility cells are `scene.zones` ([`super::zones`]). The viewport's gestures
//! call the same functions.

use std::sync::Arc;

use gz_base::Origin;
use gz_editor::{CmdError, Editor, JsonSchema, Registry, Reply, Spec, Value, json};
use serde::Deserialize;

use super::way::{self, LinkOp};
use super::{Batch, Ctl, ObjToolState, Outcome, ToolEdit, commit, commit_job, selected_of, set_state, shape};
use crate::apply::{SceneCmd, SelectMode};
use crate::class::ObjClass;
use crate::math::Vec3;
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{FV_EMITTER, WayData};
use crate::service::{self, SceneService};

/// Registers `scene.way`, `scene.shape`, `scene.fog_group` and the zones.
pub fn register(reg: &mut Registry) {
    reg.cmd(
        Spec::write(
            "scene.way",
            "Edits a way path's points and links: add, select, move and remove points, link them one way or both ways.",
        )
        .example("way=patrol_01 action=add_point position=12,0,4"),
        |ed, p: WayParams, o| async move { scene_way(ed, p, o).await },
    );
    reg.cmd(
        Spec::write(
            "scene.shape",
            "Joins shapes into one, gives a shape's parts back as shapes of their own, or makes the level's bound.",
        )
        .example("action=join objects=shape_01,shape_02"),
        |ed, p: ShapeParams, o| async move { scene_shape(ed, p, o).await },
    );
    reg.cmd(
        Spec::write(
            "scene.fog_group",
            "Puts fog volumes into one group of their own, or takes them out of theirs.",
        )
        .example("objects=fog_01,fog_02"),
        |ed, p: FogParams, o| async move { scene_fog_group(ed, p, o).await },
    );
    super::zones::register(reg);
}

/// A seed from the clock.
#[must_use]
pub fn time_seed() -> i32 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(1, |d| (d.as_nanos() & 0x7fff_ffff) as i32)
}

fn merge(mut a: Value, b: Value) -> Value {
    if let (Value::Object(m), Value::Object(n)) = (&mut a, b) {
        m.extend(n);
    }
    a
}

/// An object of a kind by name.
fn named(s: &SceneState, class: ObjClass, name: &str) -> Result<ObjId, CmdError> {
    s.find_any(name).filter(|o| o.class == class).map(|o| o.id).ok_or_else(|| {
        CmdError::not_found(format!("there is no {} named {name}", class.label().to_lowercase()))
    })
}

// ---------------------------------------------------------------- way paths

fn way_json(s: &SceneState, o: &Object, w: &WayData) -> Value {
    let points: Vec<Value> = w
        .points
        .iter()
        .map(|p| {
            let links: Vec<Value> = p
                .links
                .iter()
                .map(|(t, pr)| json!({ "to": w.points.get(*t as usize).map_or("", |q| q.name.as_str()), "probability": pr }))
                .collect();
            json!({
                "name": p.name.as_str(),
                "selected": p.selected,
                "position": crate::cmd::v3(p.position),
                "links": links,
            })
        })
        .collect();
    json!({ "name": o.name.as_str(), "selected": s.is_selected(o.id), "points": points })
}

/// `EditSelectedWays`: a link edit of every selected way, on its selected points.
pub fn link_batch(s: &SceneState, op: LinkOp) -> Result<Outcome, CmdError> {
    let ways = selected_of(s, ObjClass::Way);
    let any_point = ways
        .iter()
        .filter_map(|id| s.get(*id))
        .any(|o| matches!(&o.data, ObjData::Way(w) if w.points.iter().any(|p| p.selected)));
    if op.needs_points() && !any_point {
        return Err(CmdError::invalid("select points of the way first"));
    }
    let mut b = Batch::new(op.label(), s.serial);
    let mut changed = Vec::new();
    for id in ways {
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

/// Adds a point to a way at `at`, linked to the selected points when auto link is on.
pub fn add_point_batch(s: &SceneState, way_id: ObjId, at: Vec3) -> Result<Outcome, CmdError> {
    let o = s.get(way_id).ok_or_else(|| CmdError::not_found("the way path is gone"))?;
    let ObjData::Way(w) = &o.data else { return Err(CmdError::invalid("the object is no way path")) };
    let mut w2 = w.clone();
    let i = way::add_point(&mut w2, at, s.tools.obj.auto_link);
    let name = w2.points.get(i).map(|p| p.name.to_string()).unwrap_or_default();
    let mut n = (**o).clone();
    n.data = ObjData::Way(w2);
    let mut b = Batch::new("Add Way Point", s.serial);
    b.replace.push(n);
    Ok((b, json!({ "way": o.name.as_str(), "point": name })))
}

/// Removes the selected points of ways; a way left without points goes.
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum WayAction {
    /// The way's points and links.
    #[default]
    Get,
    /// A point at `position`, linked from the selected points when auto link is on.
    AddPoint,
    /// Selects `points` (with `mode`).
    SelectPoints,
    /// Moves `points` to `position`.
    MovePoint,
    /// Removes the selected points (`points` selects them first).
    RemovePoints,
    /// Links the selected points one way, in the order they were selected.
    LinkOneWay,
    /// Links the selected points both ways.
    LinkBothWays,
    /// Makes the links of the selected ways one way.
    MakeOneWay,
    /// Makes the links of the selected ways both ways.
    MakeBothWays,
    /// Turns the selected points' links round.
    InvertLinks,
    /// Removes the selected points' links.
    Unlink,
    /// Whether a new point is linked from the selected ones (`on`).
    AutoLink,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum PointSelect {
    /// Only these points.
    #[default]
    Set,
    Add,
    Remove,
    Toggle,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct WayParams {
    #[serde(default)]
    action: WayAction,
    /// The way path. Left out: the selected one.
    #[serde(default)]
    way: Option<String>,
    /// Its points by name (`p0`, ...) or number.
    #[serde(default)]
    points: Option<Vec<String>>,
    /// Metres (add_point, move_point).
    #[serde(default)]
    position: Option<[f32; 3]>,
    /// select_points: how the points join the selection.
    #[serde(default)]
    mode: PointSelect,
    /// auto_link.
    #[serde(default)]
    on: Option<bool>,
}

/// The way an action works on: named, or the one selected.
fn target_way(s: &SceneState, way: Option<&str>) -> Result<ObjId, CmdError> {
    if let Some(n) = way {
        return named(s, ObjClass::Way, n);
    }
    match selected_of(s, ObjClass::Way).as_slice() {
        [one] => Ok(*one),
        _ => Err(CmdError::invalid("select one way path, or name it in way")),
    }
}

fn point_keys(w: &WayData, keys: Option<&[String]>) -> Result<Vec<usize>, CmdError> {
    let keys = keys.filter(|k| !k.is_empty()).ok_or_else(|| CmdError::invalid("this action takes points"))?;
    keys.iter()
        .map(|k| {
            way::point_index(w, k.trim())
                .ok_or_else(|| CmdError::not_found(format!("the way has no point {k}")))
        })
        .collect()
}

async fn select_points(
    svc: &SceneService,
    id: ObjId,
    keys: &[String],
    mode: SelectMode,
    o: Origin,
) -> Result<(), CmdError> {
    let snap = svc.store.snapshot();
    let Some(ObjData::Way(w)) = snap.get(id).map(|x| &x.data) else {
        return Err(CmdError::invalid("the object is no way path"));
    };
    let points = point_keys(w, Some(keys))?;
    if !snap.is_selected(id) {
        svc.store.send(SceneCmd::Select { ids: vec![id], mode: SelectMode::Add }, o).await?;
    }
    svc.store.send(SceneCmd::ObjTool(Box::new(ToolEdit::SelectPoints { way: id, points, mode })), o).await?;
    Ok(())
}

fn way_reply(svc: &SceneService, id: ObjId) -> Value {
    let s = svc.store.snapshot();
    let out = s.get(id).and_then(|x| match &x.data {
        ObjData::Way(w) => Some(way_json(&s, x, w)),
        _ => None,
    });
    json!({ "way": out, "auto_link": s.tools.obj.auto_link })
}

async fn scene_way(ed: Arc<Editor>, p: WayParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    let op = match p.action {
        WayAction::LinkOneWay => Some(LinkOp::Create1),
        WayAction::LinkBothWays => Some(LinkOp::Create2),
        WayAction::MakeOneWay => Some(LinkOp::Convert1),
        WayAction::MakeBothWays => Some(LinkOp::Convert2),
        WayAction::InvertLinks => Some(LinkOp::Invert),
        WayAction::Unlink => Some(LinkOp::Remove),
        _ => None,
    };
    if let Some(op) = op {
        if p.way.is_some() || p.points.is_some() {
            let id = target_way(&snap, p.way.as_deref())?;
            if let Some(keys) = &p.points {
                select_points(&svc, id, keys, SelectMode::Set, o).await?;
            } else {
                svc.store.send(SceneCmd::Select { ids: vec![id], mode: SelectMode::Set }, o).await?;
            }
        }
        return commit(&ed, &svc, o, Ctl::default(), move |s, _, _| link_batch(s, op)).await;
    }
    match p.action {
        WayAction::Get => {
            let id = target_way(&snap, p.way.as_deref())?;
            Ok(way_reply(&svc, id))
        }
        WayAction::AutoLink => {
            let on = p.on.ok_or_else(|| CmdError::invalid("auto_link takes on"))?;
            let st = set_state(&svc, o, |st| st.auto_link = on).await?;
            Ok(json!({ "auto_link": st.auto_link }))
        }
        WayAction::AddPoint => {
            let at =
                Vec3::from_array(p.position.ok_or_else(|| CmdError::invalid("add_point takes position"))?);
            if !at.is_finite() {
                return Err(CmdError::invalid("position takes finite numbers"));
            }
            let id = target_way(&snap, p.way.as_deref())?;
            let out = commit(&ed, &svc, o, Ctl::default(), move |s, _, _| add_point_batch(s, id, at)).await?;
            Ok(merge(out, way_reply(&svc, id)))
        }
        WayAction::SelectPoints => {
            let id = target_way(&snap, p.way.as_deref())?;
            let mode = match p.mode {
                PointSelect::Set => SelectMode::Set,
                PointSelect::Add => SelectMode::Add,
                PointSelect::Remove => SelectMode::Remove,
                PointSelect::Toggle => SelectMode::Toggle,
            };
            select_points(&svc, id, p.points.as_deref().unwrap_or_default(), mode, o).await?;
            Ok(way_reply(&svc, id))
        }
        WayAction::MovePoint => {
            let id = target_way(&snap, p.way.as_deref())?;
            let Some(ObjData::Way(w)) = snap.get(id).map(|x| &x.data) else {
                return Err(CmdError::invalid("the object is no way path"));
            };
            let points = point_keys(w, p.points.as_deref())?;
            let at =
                Vec3::from_array(p.position.ok_or_else(|| CmdError::invalid("move_point takes position"))?);
            if !at.is_finite() {
                return Err(CmdError::invalid("position takes finite numbers"));
            }
            let moves = points.into_iter().map(|i| (i, at)).collect();
            svc.store
                .send(
                    SceneCmd::ObjTool(Box::new(ToolEdit::MovePoints { way: id, points: moves, merge: None })),
                    o,
                )
                .await?;
            Ok(way_reply(&svc, id))
        }
        WayAction::RemovePoints => {
            let ids = match (&p.way, &p.points) {
                (_, Some(keys)) => {
                    let id = target_way(&snap, p.way.as_deref())?;
                    select_points(&svc, id, keys, SelectMode::Set, o).await?;
                    vec![id]
                }
                (Some(_), None) => vec![target_way(&snap, p.way.as_deref())?],
                (None, None) => selected_of(&snap, ObjClass::Way),
            };
            commit(&ed, &svc, o, Ctl::default(), move |s, _, _| Ok(remove_points_batch(s, &ids))).await
        }
        _ => Err(CmdError::internal("a link action was not taken")),
    }
}

// ---------------------------------------------------------------- shapes

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ShapeAction {
    /// The shapes `objects` names (or the selected ones) become parts of the first.
    Join,
    /// The selected shapes' parts become shapes of their own.
    Split,
    /// The level's bound: made around what the scene holds (`on=true`), made again
    /// (`recalculate=true`), or left alone (`on=false`).
    LevelBound,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct ShapeParams {
    action: ShapeAction,
    /// The shapes by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// level_bound: edit the bound (true) or stop editing it (false).
    #[serde(default)]
    on: Option<bool>,
    /// level_bound: make the bound again around what the scene holds now.
    #[serde(default)]
    recalculate: bool,
}

fn shape_state(s: &SceneState) -> Value {
    let bound = s
        .of_class(ObjClass::Shape)
        .find(|o| matches!(&o.data, ObjData::Shape(d) if d.usage == crate::objects::SHAPE_LEVEL_BOUND))
        .map(|o| o.name.to_string());
    json!({ "edit_level_bound": s.tools.obj.edit_level_bound, "level_bound": bound })
}

async fn scene_shape(ed: Arc<Editor>, p: ShapeParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    let snap = svc.store.snapshot();
    match p.action {
        ShapeAction::Join => {
            let ids: Vec<ObjId> = crate::edit::ids_of(&snap, p.objects.as_deref())?
                .into_iter()
                .filter(|id| snap.get(*id).is_some_and(|x| x.class == ObjClass::Shape))
                .collect();
            let [base, rest @ ..] = ids.as_slice() else {
                return Err(CmdError::invalid("join takes two shapes or more"));
            };
            if rest.is_empty() {
                return Err(CmdError::invalid("join takes two shapes or more"));
            }
            let base = *base;
            for from in rest.iter().copied() {
                commit(&ed, &svc, o, Ctl::default(), move |s, _, _| shape::attach(s, base, from)).await?;
            }
            let s = svc.store.snapshot();
            Ok(json!({ "shape": s.get(base).map(|x| x.name.to_string()), "joined": rest.len() }))
        }
        ShapeAction::Split => {
            if p.objects.is_some() {
                let ids = crate::edit::ids_of(&snap, p.objects.as_deref())?;
                svc.store.send(SceneCmd::Select { ids, mode: SelectMode::Set }, o).await?;
            }
            commit(&ed, &svc, o, Ctl::default(), |s, _, _| shape::detach_selected(s)).await
        }
        ShapeAction::LevelBound => {
            if p.recalculate {
                let out = commit_job(&ed, &svc, o, "Making the level bound", |s, lib, _| {
                    let mut st: ObjToolState = s.tools.obj;
                    st.edit_level_bound = true;
                    shape::edit_level_bound(s, lib, true, Some(st))
                })
                .await?;
                return Ok(merge(out, shape_state(&svc.store.snapshot())));
            }
            match p.on {
                Some(false) => {
                    set_state(&svc, o, |x| x.edit_level_bound = false).await?;
                    Ok(shape_state(&svc.store.snapshot()))
                }
                Some(true) => {
                    let mut st: ObjToolState = snap.tools.obj;
                    st.edit_level_bound = true;
                    let out = commit_job(&ed, &svc, o, "Editing the level bound", move |s, lib, _| {
                        shape::edit_level_bound(s, lib, false, Some(st))
                    })
                    .await?;
                    Ok(merge(out, shape_state(&svc.store.snapshot())))
                }
                None => Ok(shape_state(&snap)),
            }
        }
    }
}

// ---------------------------------------------------------------- fog volumes

/// The selected fog volumes into a group of their own (`group`), or out of theirs.
pub fn fog_group(s: &SceneState, group: bool) -> Outcome {
    let mut b = Batch::new(if group { "Group Fog Volumes" } else { "Ungroup Fog Volumes" }, s.serial);
    let mut st = s.tools.obj;
    // the counter is at least every group read or set; an ungrouped volume holds u32::MAX, so
    // the next group wraps round to 0
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

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct FogParams {
    /// The fog volumes by name. Left out: the selection.
    #[serde(default)]
    objects: Option<Vec<String>>,
    /// Take them out of their group instead.
    #[serde(default)]
    ungroup: bool,
}

async fn scene_fog_group(ed: Arc<Editor>, p: FogParams, o: Origin) -> Reply {
    let svc = service::need_scene(&ed).await?;
    if p.objects.is_some() {
        let ids = crate::edit::ids_of(&svc.store.snapshot(), p.objects.as_deref())?;
        svc.store.send(SceneCmd::Select { ids, mode: SelectMode::Set }, o).await?;
    }
    let group = !p.ungroup;
    let out = commit(&ed, &svc, o, Ctl::default(), move |s, _, _| Ok(fog_group(s, group))).await?;
    let s = svc.store.snapshot();
    let volumes: Vec<Value> = s
        .of_class(ObjClass::FogVolume)
        .filter(|x| s.is_selected(x.id))
        .filter_map(|x| match &x.data {
            ObjData::FogVolume(f) => Some(json!({
                "name": x.name.as_str(),
                "group": if f.group_id == u32::MAX { Value::Null } else { json!(f.group_id) },
                "kind": if f.kind == FV_EMITTER { "emitter" } else { "occlusion" },
            })),
            _ => None,
        })
        .collect();
    Ok(merge(out, json!({ "volumes": volumes })))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::FogData;

    #[test]
    fn grouping_fog_wraps_round_to_the_first_group() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(ObjClass::FogVolume, "a", ObjData::FogVolume(FogData::default())));
        s.selection.insert(a);
        let (b, out) = fog_group(&s, true);
        assert_eq!(out["group"], json!(0));
        assert_eq!(b.replace.len(), 1);
    }

    #[test]
    fn link_commands_that_need_points_say_so() {
        let s = SceneState::default();
        assert_eq!(
            link_batch(&s, LinkOp::Create1).err().map(|e| e.message),
            Some("select points of the way first".into())
        );
        assert!(link_batch(&s, LinkOp::Convert1).is_ok());
    }
}
