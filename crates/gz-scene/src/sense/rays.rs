//! `ray` and `box`: one ray through the scene, and what stands in a box.

use gz_editor::CmdError;
use serde_json::{Value, json};

use super::{
    Lists, QueryParams, Scene, bounds_json, facing, finite, kind_of, name_of, radians, surface_json,
};
use crate::cmd::{r4, v3};
use crate::math::{Aabb, Mat4, Transform, Vec3, xform_box};
use crate::model::{ObjData, ObjId};
use crate::pick::{PickHit, RayKinds, ray_scene, ray_scene_all, solid_triangles};

fn hit_json(sc: &Scene<'_>, h: &PickHit, dir: Vec3) -> Value {
    json!({
        "point": v3(h.point),
        "normal": h.normal.map_or(Value::Null, |n| v3(facing(n, dir))),
        "distance": r4(h.t),
        "object": name_of(sc, h.id),
        "kind": kind_of(sc, h.id),
        "surface": surface_json(sc, h),
    })
}

pub(super) fn ray(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let from = finite(p.from.ok_or_else(|| CmdError::invalid("ray takes from: [x, y, z]"))?, "from")?;
    let (dir, range) = match (p.to, p.direction) {
        (Some(_), Some(_)) => return Err(CmdError::invalid("ray takes to or direction, not both")),
        (Some(to), None) => {
            let d = finite(to, "to")? - from;
            (d.normalize_or_zero(), d.length())
        }
        (None, Some(d)) => {
            let reach = p.reach.unwrap_or(1000.0);
            if !reach.is_finite() || reach <= 0.0 {
                return Err(CmdError::invalid("reach takes a number of metres above 0"));
            }
            (finite(d, "direction")?.normalize_or_zero(), reach)
        }
        (None, None) => return Err(CmdError::invalid("ray takes to: [x, y, z], or direction and reach")),
    };
    if dir == Vec3::ZERO {
        return Err(CmdError::invalid("the ray has no direction: to is from, or direction is 0, 0, 0"));
    }
    let lists = Lists::of(sc.state, p)?;
    // what stands in the level; a volume is in nobody's way and is met only when asked for
    let f = lists.filter(if p.volumes { RayKinds::Geometry } else { RayKinds::Meshes });
    if p.all {
        let hits = ray_scene_all(sc.pick, sc.state, sc.lib, from, dir, range, &f);
        let total = hits.len();
        let limit = p.limit.unwrap_or(100);
        let shown: Vec<Value> = hits.iter().take(limit).map(|h| hit_json(sc, h, dir)).collect();
        return Ok(json!({ "hits": shown, "total": total }));
    }
    let hit = ray_scene(sc.pick, sc.state, sc.lib, from, dir, range, &f);
    Ok(json!({ "hit": hit.map_or(Value::Null, |h| hit_json(sc, &h, dir)) }))
}

/// A box that may be turned: its own space from `-half` to `half`, carried into the world by `m`.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OBox {
    pub m: Mat4,
    pub inv: Mat4,
    pub center: Vec3,
    pub half: Vec3,
}

impl OBox {
    /// A box of its own space `local` carried by a matrix.
    pub fn new(local: &Aabb, m: Mat4) -> Self {
        Self { inv: Mat4::invert_43(&m), m, center: local.center(), half: local.half_extents() }
    }

    pub fn world(&self) -> Aabb {
        xform_box(&Aabb::new(self.center - self.half, self.center + self.half), &self.m)
    }

    /// Whether a world triangle meets the box, the box shrunk by `margin` of its own units.
    pub fn meets(&self, t: &[Vec3; 3], margin: Vec3) -> Option<[Vec3; 3]> {
        let l = [self.inv.transform_tiny(t[0]), self.inv.transform_tiny(t[1]), self.inv.transform_tiny(t[2])];
        let half = (self.half - margin).max(Vec3::ZERO);
        crate::bulk::xr::tri_box(self.center, half, &l, true).then_some(l)
    }
}

/// The world triangles of a scene object that meet a turned box.
pub(crate) fn object_meets(sc: &Scene<'_>, id: ObjId, ob: &OBox, margin: Vec3) -> Vec<[Vec3; 3]> {
    solid_triangles(sc.state, sc.lib, &[id], &ob.world())
        .into_iter()
        .filter_map(|t| ob.meets(&t, margin))
        .collect()
}

pub(super) fn boxed(sc: &Scene<'_>, p: &QueryParams) -> Result<Value, CmdError> {
    let ob = match (p.min, p.max, p.position, p.size) {
        (Some(lo), Some(hi), None, None) => {
            if p.rotation.is_some() {
                return Err(CmdError::invalid("rotation turns a box given by position and size"));
            }
            let (lo, hi) = (finite(lo, "min")?, finite(hi, "max")?);
            if hi.cmplt(lo).any() {
                return Err(CmdError::invalid("max must not be below min on any axis"));
            }
            OBox::new(&Aabb::new(lo, hi), Mat4::IDENTITY)
        }
        (None, None, Some(at), Some(size)) => {
            let (at, size) = (finite(at, "position")?, finite(size, "size")?);
            if size.cmplt(Vec3::ZERO).any() {
                return Err(CmdError::invalid("size takes no negative numbers"));
            }
            let rotation = p.rotation.map(|r| finite(r, "rotation").map(|_| radians(r))).transpose()?;
            let x = Transform { position: at, rotation: rotation.unwrap_or(Vec3::ZERO), scale: Vec3::ONE };
            OBox::new(&Aabb::new(-size * 0.5, size * 0.5), x.matrix())
        }
        _ => return Err(CmdError::invalid("box takes min and max, or position and size (and rotation)")),
    };
    let lists = Lists::of(sc.state, p)?;
    let world = ob.world();
    let mut found: Vec<(ObjId, Aabb)> = sc
        .pick
        .box_candidates(&world)
        .into_iter()
        .filter(|id| lists.only.as_ref().is_none_or(|l| l.contains(id)) && !lists.except.contains(id))
        .filter_map(|id| sc.state.get(id).and_then(|o| sc.world_box(o)).map(|b| (id, b)))
        .filter(|(id, b)| {
            let Some(o) = sc.state.get(*id) else { return false };
            if p.exact && matches!(o.data, ObjData::SceneObject(_)) {
                return !object_meets(sc, *id, &ob, Vec3::ZERO).is_empty();
            }
            // the object's box carried into the turned box's space (and boxed there again)
            let local = xform_box(b, &ob.inv);
            (0..3).all(|a| {
                local.min[a] <= ob.center[a] + ob.half[a] && local.max[a] >= ob.center[a] - ob.half[a]
            })
        })
        .collect();
    found.sort_by(|a, b| {
        let (na, nb) = (sc.state.get(a.0).map(|o| o.name.clone()), sc.state.get(b.0).map(|o| o.name.clone()));
        na.cmp(&nb)
    });
    let total = found.len();
    let limit = p.limit.unwrap_or(100);
    let objects: Vec<Value> = found
        .iter()
        .take(limit)
        .filter_map(|(id, b)| {
            let o = sc.state.get(*id)?;
            let mut m = serde_json::Map::new();
            m.insert("name".into(), json!(o.name.as_str()));
            m.insert("kind".into(), json!(o.class.key()));
            if let Some(r) = o.ref_name().filter(|r| !r.is_empty()) {
                m.insert("ref".into(), json!(r));
            }
            m.insert("bounds".into(), bounds_json(b));
            Some(Value::Object(m))
        })
        .collect();
    Ok(json!({ "objects": objects, "total": total }))
}
