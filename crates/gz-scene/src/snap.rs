//! Snapping: the snap list (`SceneSnap.cpp`) and the drop to ground (`DropObjectsToGround`,
//! the End key) - computed from a snapshot, applied as one transform command.

use crate::class::ObjClass;
use crate::library::Library;
use crate::math::{Aabb, EPS_L, EPS_S, Mat4, Vec3, clip_triangle_xz, normal_align, rotation_matrix};
use crate::model::{ObjId, SceneState};
use crate::objects;
use crate::pick::{ScenePick, box_triangles, ray_scene_objects};

/// `kDropReach`.
pub const DROP_REACH: f32 = 1000.0;

/// What the Snap Objects section's buttons do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SnapAction {
    SetFromSelection,
    AddSelection,
    RemoveSelection,
    Select,
    Clear,
}

impl SnapAction {
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s.to_ascii_lowercase().as_str() {
            "set_from_selection" => Self::SetFromSelection,
            "add_selection" => Self::AddSelection,
            "remove_selection" => Self::RemoveSelection,
            "select" => Self::Select,
            "clear" => Self::Clear,
            _ => return None,
        })
    }
}

/// The selected, visible scene objects (`GetQueryObjects(lst, OBJCLASS_SCENEOBJECT, 1, 1, 0)`).
#[must_use]
pub fn selected_scene_objects(state: &SceneState) -> Vec<ObjId> {
    state
        .of_class(ObjClass::SceneObject)
        .filter(|o| o.flags.visible && state.is_selected(o.id))
        .map(|o| o.id)
        .collect()
}

/// The snap list of the current tool (`GetSnapList(true)`): the AI Map and Detail Objects tools
/// keep their own (by name, in their part of the scene), every other tool the scene's.
#[must_use]
pub fn tool_list(state: &SceneState) -> Vec<ObjId> {
    let names = match state.tools.target {
        ObjClass::AiMap => {
            state.bulk.ai_map.as_ref().map(|a| a.header.snap_objects.clone()).unwrap_or_default()
        }
        ObjClass::DetailObjects => {
            state.bulk.details.as_ref().map(|d| d.model.snap_objects.clone()).unwrap_or_default()
        }
        _ => return state.tools.snap.clone(),
    };
    names.iter().filter_map(|n| state.find(ObjClass::SceneObject, &n.to_lowercase()).map(|o| o.id)).collect()
}

/// Writes the current tool's snap list.
fn set_tool_list(state: &mut SceneState, list: Vec<ObjId>) {
    let names: Vec<gz_base::SmolStr> =
        list.iter().filter_map(|id| state.get(*id)).map(|o| o.name.clone()).collect();
    match state.tools.target {
        ObjClass::AiMap => {
            let mut part = state.bulk.ai_map.as_deref().cloned().unwrap_or_else(|| {
                crate::tools::AiMapPart::from_nodes(gz_fmt_level::SceneAiMap::default(), &[], None)
            });
            part.header.snap_objects = names;
            part.raw = None;
            state.bulk.ai_map = Some(std::sync::Arc::new(part));
        }
        ObjClass::DetailObjects => {
            let mut model = state.bulk.details.as_ref().map(|d| (*d.model).clone()).unwrap_or_default();
            model.snap_objects = names;
            state.bulk.details = Some(std::sync::Arc::new(crate::tools::DetailsPart {
                raw: None,
                model: std::sync::Arc::new(model),
            }));
        }
        _ => state.tools.snap = list,
    }
}

/// Applies a snap action to the current tool's list; `true` when the list or the selection
/// changed.
pub fn apply_snap(state: &mut SceneState, action: SnapAction) -> bool {
    let sel = selected_scene_objects(state);
    let mut list = tool_list(state);
    let changed = match action {
        SnapAction::SetFromSelection => {
            list = sel;
            true
        }
        SnapAction::AddSelection => {
            let mut changed = false;
            for id in sel {
                if !list.contains(&id) {
                    list.push(id);
                    changed = true;
                }
            }
            changed
        }
        SnapAction::RemoveSelection => {
            let before = list.len();
            list.retain(|id| !sel.contains(id));
            before != list.len()
        }
        SnapAction::Select => {
            let scene: Vec<ObjId> = state.by_class[ObjClass::SceneObject.index()].iter().copied().collect();
            for id in scene {
                state.selection.remove(&id);
            }
            for id in &list {
                if state.get(*id).is_some_and(|o| o.flags.visible) {
                    state.selection.insert(*id);
                }
            }
            return true;
        }
        SnapAction::Clear => {
            let had = !list.is_empty();
            list.clear();
            had
        }
    };
    if changed {
        set_tool_list(state, list);
    }
    changed
}

/// Where an object lands: its new position and, when aligned, its new rotation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Landing {
    pub id: ObjId,
    pub position: Vec3,
    pub rotation: Option<Vec3>,
}

/// `DropObjectsToGround`: every object of `batch` settled on the scene objects under it - the
/// whole box swept down (`line == false`, every triangle in the column clipped to the
/// footprint, the tallest point wins) or one ray from the pivot (`line == true`).
#[must_use]
pub fn drop_to_ground(
    state: &SceneState,
    lib: &Library,
    pick: &ScenePick,
    batch: &[ObjId],
    align: bool,
    line: bool,
) -> Vec<Landing> {
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let candidates: Vec<(ObjId, Aabb)> = state
        .of_class(ObjClass::SceneObject)
        .filter(|o| o.flags.visible && !batch.contains(&o.id))
        .filter_map(|o| objects::world_box(state, o, &local).map(|b| (o.id, b)))
        .collect();
    if candidates.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for id in batch {
        let Some(obj) = state.get(*id) else { continue };
        let Some(bx) = objects::world_box(state, obj, &local) else { continue };
        let floor: Vec<ObjId> = candidates
            .iter()
            .filter(|(_, fb)| {
                !(fb.max.x < bx.min.x - EPS_L || fb.min.x > bx.max.x + EPS_L)
                    && !(fb.max.z < bx.min.z - EPS_L || fb.min.z > bx.max.z + EPS_L)
                    && fb.min.y <= bx.max.y
            })
            .map(|(i, _)| *i)
            .collect();
        if floor.is_empty() {
            continue;
        }
        let mut drop = f32::MAX;
        let mut normal = Vec3::Y;
        if line {
            let start = obj.position() + Vec3::new(0.0, EPS_L, 0.0);
            if let Some(hit) =
                ray_scene_objects(pick, state, lib, start, Vec3::NEG_Y, DROP_REACH, Some(&floor))
            {
                drop = obj.position().y - hit.point.y;
                if let Some(n) = hit.normal.filter(|n| n.is_finite()) {
                    normal = n;
                }
            }
        } else {
            let sweep = Aabb::new(
                Vec3::new(bx.min.x, bx.min.y - DROP_REACH, bx.min.z),
                Vec3::new(bx.max.x, bx.min.y + EPS_L, bx.max.z),
            );
            let mut best = -f32::MAX;
            for tri in box_triangles(state, lib, &floor, &sweep) {
                for c in clip_triangle_xz(&tri, bx.min.x, bx.max.x, bx.min.z, bx.max.z) {
                    if c.y > bx.min.y + EPS_L || c.y <= best {
                        continue;
                    }
                    best = c.y;
                    let n = (tri[1] - tri[0]).cross(tri[2] - tri[0]);
                    if n.length() > EPS_S {
                        let n = n.normalize();
                        normal = if n.y < 0.0 { -n } else { n };
                    }
                }
            }
            if best > -f32::MAX {
                drop = bx.min.y - best;
            }
        }
        if drop == f32::MAX || drop <= EPS_L {
            continue;
        }
        let mut pos = obj.position();
        pos.y -= drop;
        let rotation = align.then(|| {
            let m: Mat4 = rotation_matrix(obj.rotation());
            normal_align(normal, m.row(2))
        });
        out.push(Landing { id: *id, position: pos, rotation });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ObjData, Object};
    use crate::objects::SceneObjectData;

    #[test]
    fn snap_actions_follow_the_selection() {
        let mut s = SceneState::default();
        let a = s.insert(Object::new(
            ObjClass::SceneObject,
            "a",
            ObjData::SceneObject(SceneObjectData::library("x")),
        ));
        let b = s.insert(Object::new(
            ObjClass::SceneObject,
            "b",
            ObjData::SceneObject(SceneObjectData::library("x")),
        ));
        s.selection.insert(a);
        assert!(apply_snap(&mut s, SnapAction::SetFromSelection));
        assert_eq!(s.tools.snap, vec![a]);
        s.selection.insert(b);
        assert!(apply_snap(&mut s, SnapAction::AddSelection));
        assert_eq!(s.tools.snap, vec![a, b]);
        s.selection.clear();
        apply_snap(&mut s, SnapAction::Select);
        assert!(s.is_selected(a) && s.is_selected(b));
        assert!(apply_snap(&mut s, SnapAction::Clear));
        assert!(s.tools.snap.is_empty());
    }
}
