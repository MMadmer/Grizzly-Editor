//! The drop to ground (`DropObjectsToGround`, the End key): where objects land, computed from a
//! snapshot and applied as one transform command.

use crate::class::ObjClass;
use crate::library::Library;
use crate::math::{Aabb, EPS_L, EPS_S, Mat4, Vec3, clip_triangle_xz, normal_align, rotation_matrix};
use crate::model::{ObjId, SceneState};
use crate::objects;
use crate::pick::{ScenePick, box_triangles, ray_scene_objects};

/// `kDropReach`.
pub const DROP_REACH: f32 = 1000.0;

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
