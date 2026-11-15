//! Picking against a snapshot: a tree over the
//! objects' world boxes gives the candidates, each candidate is tested in its own space - a
//! scene object against its library object's `Bvh`. Nothing here locks the scene.

use std::sync::Arc;

use gz_fmt_level::Ray;

use crate::class::ObjClass;
use crate::library::{Library, LibraryObject, NO_SURFACE};
use crate::math::{Aabb, Vec3, ray_box, ray_point, tri_normal};
use crate::model::{ObjData, ObjId, Object, SceneState};
use crate::objects::{self, PSOBJECT_SIZE, VIS_RADIUS};

/// A hit: the object, the distance along the ray, the point, the surface normal when the hit
/// was a triangle, and then the triangle's surface (an index into the library object's
/// `object.surfaces`) when it has one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PickHit {
    pub id: ObjId,
    pub t: f32,
    pub point: Vec3,
    pub normal: Option<Vec3>,
    pub surface: Option<u32>,
}

/// Which kinds a scene ray takes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RayKinds {
    /// The objects with a mesh: static and dynamic scene objects, and terrains.
    #[default]
    Meshes,
    /// Every object with geometry: the meshes, and by their shapes the shapes, the sound
    /// environments, the fog volumes and the spawn points that own a shape.
    Geometry,
}

/// What a scene ray takes: the kinds, visible objects only, `only` a list, never `except`'s.
#[derive(Clone, Copy, Debug, Default)]
pub struct RayFilter<'a> {
    pub kinds: RayKinds,
    pub only: Option<&'a [ObjId]>,
    pub except: &'a [ObjId],
}

impl RayFilter<'_> {
    /// Whether the ray tests this object.
    #[must_use]
    pub fn accepts(&self, o: &Object) -> bool {
        let kind = match self.kinds {
            RayKinds::Meshes => matches!(o.class, ObjClass::SceneObject | ObjClass::Terrain),
            RayKinds::Geometry => match &o.data {
                ObjData::SceneObject(_)
                | ObjData::Terrain(_)
                | ObjData::Shape(_)
                | ObjData::SoundEnv(_)
                | ObjData::FogVolume(_) => true,
                ObjData::Spawn(sp) => sp.attached.is_some(),
                _ => false,
            },
        };
        kind && o.flags.visible && self.only.is_none_or(|l| l.contains(&o.id)) && !self.except.contains(&o.id)
    }
}

#[derive(Clone, Copy, Debug)]
struct Node {
    bounds: Aabb,
    /// A leaf's first item, or an inner node's right child.
    a: u32,
    /// Items of a leaf; 0 for an inner node.
    count: u32,
}

/// The top-level tree of one snapshot: every object with a world box.
pub struct ScenePick {
    items: Vec<(ObjId, Aabb)>,
    nodes: Vec<Node>,
}

const LEAF: usize = 4;

impl ScenePick {
    /// The boxes of every object (library objects as far as they are loaded).
    #[must_use]
    pub fn build(state: &SceneState, lib: &Library) -> Self {
        let local = |r: &str| lib.cached(r).map(|o| o.bounds);
        let mut items: Vec<(ObjId, Aabb)> = state
            .objects
            .values()
            .filter_map(|o| objects::world_box(state, o, &local).filter(Aabb::is_valid).map(|b| (o.id, b)))
            .collect();
        let mut nodes = Vec::with_capacity(items.len() * 2 / LEAF + 1);
        if !items.is_empty() {
            let n = items.len();
            build_node(&mut items, 0, n, &mut nodes);
        }
        Self { items, nodes }
    }

    /// The box around every object of the tree (invalid for an empty tree).
    #[must_use]
    pub fn bounds(&self) -> Aabb {
        self.nodes.first().map_or(Aabb::INVALID, |n| n.bounds)
    }

    /// The objects whose box a ray crosses within `range`.
    #[must_use]
    pub fn ray_candidates(&self, start: Vec3, dir: Vec3, range: f32) -> Vec<(ObjId, f32)> {
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let Some(n) = self.nodes.get(i) else { continue };
            match ray_box(start, dir, &n.bounds) {
                Some((t, _)) if t <= range => {}
                _ => continue,
            }
            if n.count > 0 {
                for k in n.a as usize..(n.a + n.count) as usize {
                    if let Some((id, b)) = self.items.get(k)
                        && let Some((t, _)) = ray_box(start, dir, b)
                        && t <= range
                    {
                        out.push((*id, t));
                    }
                }
            } else {
                stack.push(i + 1);
                stack.push(n.a as usize);
            }
        }
        out
    }

    /// The objects whose box meets `b`.
    #[must_use]
    pub fn box_candidates(&self, b: &Aabb) -> Vec<ObjId> {
        let overlap = |x: &Aabb| (0..3).all(|a| x.min[a] <= b.max[a] && x.max[a] >= b.min[a]);
        let mut out = Vec::new();
        if self.nodes.is_empty() {
            return out;
        }
        let mut stack = vec![0usize];
        while let Some(i) = stack.pop() {
            let Some(n) = self.nodes.get(i) else { continue };
            if !overlap(&n.bounds) {
                continue;
            }
            if n.count > 0 {
                for k in n.a as usize..(n.a + n.count) as usize {
                    if let Some((id, ib)) = self.items.get(k)
                        && overlap(ib)
                    {
                        out.push(*id);
                    }
                }
            } else {
                stack.push(i + 1);
                stack.push(n.a as usize);
            }
        }
        out
    }

    /// The nearest hit of the objects `accept` takes (visibility is the
    /// caller's rule). Candidates are tested in parallel.
    #[must_use]
    pub fn ray_pick(
        &self,
        state: &SceneState,
        lib: &Library,
        start: Vec3,
        dir: Vec3,
        range: f32,
        accept: &(dyn Fn(&Object) -> bool + Sync),
    ) -> Option<PickHit> {
        use rayon::prelude::*;
        let cands = self.ray_candidates(start, dir, range);
        let test = |(id, _): &(ObjId, f32)| -> Option<PickHit> {
            let o = state.get(*id)?;
            if !accept(o) {
                return None;
            }
            ray_object(state, lib, o, start, dir, range)
        };
        let hits: Vec<PickHit> = if cands.len() > 32 {
            cands.par_iter().filter_map(test).collect()
        } else {
            cands.iter().filter_map(test).collect()
        };
        hits.into_iter().min_by(|a, b| a.t.total_cmp(&b.t))
    }

    /// Every object's nearest hit along a ray, nearest first.
    #[must_use]
    pub fn ray_hits(
        &self,
        state: &SceneState,
        lib: &Library,
        start: Vec3,
        dir: Vec3,
        range: f32,
        accept: &(dyn Fn(&Object) -> bool + Sync),
    ) -> Vec<PickHit> {
        let mut hits: Vec<PickHit> = self
            .ray_candidates(start, dir, range)
            .iter()
            .filter_map(|(id, _)| {
                let o = state.get(*id)?;
                if !accept(o) {
                    return None;
                }
                ray_object(state, lib, o, start, dir, range)
            })
            .collect();
        hits.sort_by(|a, b| a.t.total_cmp(&b.t));
        hits
    }
}

fn build_node(items: &mut [(ObjId, Aabb)], first: usize, count: usize, nodes: &mut Vec<Node>) {
    let slice = &mut items[first..first + count];
    let mut bounds = Aabb::INVALID;
    for (_, b) in slice.iter() {
        bounds.merge(b);
    }
    let me = nodes.len();
    nodes.push(Node {
        bounds,
        a: u32::try_from(first).unwrap_or(0),
        count: u32::try_from(count).unwrap_or(0),
    });
    if count <= LEAF {
        return;
    }
    let ext = bounds.max - bounds.min;
    let axis = if ext.x >= ext.y && ext.x >= ext.z {
        0
    } else if ext.y >= ext.z {
        1
    } else {
        2
    };
    let mid = count / 2;
    slice.select_nth_unstable_by(mid, |a, b| {
        (a.1.min[axis] + a.1.max[axis]).total_cmp(&(b.1.min[axis] + b.1.max[axis]))
    });
    build_node(items, first, mid, nodes);
    let right = nodes.len();
    build_node(items, first + mid, count - mid, nodes);
    if let Some(n) = nodes.get_mut(me) {
        n.a = u32::try_from(right).unwrap_or(0);
        n.count = 0;
    }
}

/// A scene object's own test: the ray carried into object space, the library tree's nearest
/// front-facing triangle (`OPT_ONLYNEAREST | OPT_CULL`). The distance is the world's.
#[must_use]
pub fn ray_library(lo: &LibraryObject, obj: &Object, start: Vec3, dir: Vec3, range: f32) -> Option<PickHit> {
    ray_library_at(lo, obj, &obj.world(), start, dir, range)
}

/// [`ray_library`] with the model placed by `world` (a terrain's is in the world already).
fn ray_library_at(
    lo: &LibraryObject,
    obj: &Object,
    world: &gz_fmt_object::Mat4,
    start: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<PickHit> {
    let bvh = lo.bvh.as_ref()?;
    let world = *world;
    let inv = gz_fmt_object::Mat4::invert_43(&world);
    let s = inv.transform_tiny(start);
    let d = inv.transform_dir(dir);
    let mut ray = Ray::new(s, d, range);
    ray.cull_back = true;
    let hit = bvh.ray_nearest(&ray)?;
    let tri = lo.tri(hit.tri)?;
    let w = [world.transform_tiny(tri[0]), world.transform_tiny(tri[1]), world.transform_tiny(tri[2])];
    Some(PickHit {
        id: obj.id,
        t: hit.t,
        point: start + dir * hit.t,
        normal: Some(tri_normal(w[0], w[1], w[2])),
        surface: lo.tri_surface.get(hit.tri as usize).copied().filter(|s| *s != NO_SURFACE),
    })
}

/// The class's ray pick.
#[must_use]
pub fn ray_object(
    state: &SceneState,
    lib: &Library,
    o: &Object,
    start: Vec3,
    dir: Vec3,
    range: f32,
) -> Option<PickHit> {
    let at = |t: f32| PickHit { id: o.id, t, point: start + dir * t, normal: None, surface: None };
    let p = o.position();
    match &o.data {
        // a scene object a terrain cuts is hit as its cut model
        ObjData::SceneObject(_) => {
            let lo: Arc<LibraryObject> = objects::terrain::mesh_of(state, lib, o)?;
            ray_library(&lo, o, start, dir, range)
        }
        ObjData::Terrain(_) => {
            let lo = objects::terrain::mesh_of(state, lib, o)?;
            ray_library_at(&lo, o, &gz_fmt_object::Mat4::IDENTITY, start, dir, range)
        }
        ObjData::Light(_) | ObjData::SoundSrc(_) => {
            ray_point(start, dir, p, VIS_RADIUS).filter(|t| *t < range).map(at)
        }
        ObjData::Glow(g) => ray_point(start, dir, p, g.radius).filter(|t| *t < range).map(at),
        ObjData::Shape(s) => s.ray_pick(&o.xform, start, dir, range).map(at),
        ObjData::SoundEnv(s) => s.shape.ray_pick(&o.xform, start, dir, range).map(at),
        ObjData::FogVolume(s) => s.shape.ray_pick(&o.xform, start, dir, range).map(at),
        ObjData::Way(w) => w.ray_pick(start, dir, range).map(at),
        ObjData::Particles(_) => ray_box(start, dir, &crate::math::point_box(p, PSOBJECT_SIZE))
            .filter(|(t, inside)| !inside && *t < range)
            .map(|(t, _)| at(t)),
        ObjData::Spawn(sp) => {
            if let Some(a) = &sp.attached {
                return match &a.data {
                    ObjData::Shape(s) => s.ray_pick(&a.xform, start, dir, range).map(at),
                    _ => None,
                };
            }
            let b = objects::world_box(state, o, &|r: &str| lib.cached(r).map(|x| x.bounds))?;
            let (c, r) = (b.center(), (b.max - b.min).length() * 0.5);
            ray_point(start, dir, c, r)?;
            match ray_box(start, dir, &b) {
                Some((t, false)) if t < range => Some(at(t)),
                _ => None,
            }
        }
        ObjData::Group(g) => g
            .members
            .iter()
            .filter_map(|m| state.get(*m))
            .filter_map(|m| ray_object(state, lib, m, start, dir, range))
            .min_by(|a, b| a.t.total_cmp(&b.t))
            .map(|h| PickHit { id: o.id, ..h }),
        ObjData::Sector(_) | ObjData::Portal(_) | ObjData::Kept(_) => None,
    }
}

/// The nearest scene object a ray meets within its range: what `scene.align`, `scene.query`
/// and the drop read. Every scene object counts, seen by a camera or not; a hidden one does
/// not. `only` restricts it to a list.
#[must_use]
pub fn ray_scene_objects(
    pick: &ScenePick,
    state: &SceneState,
    lib: &Library,
    start: Vec3,
    dir: Vec3,
    range: f32,
    only: Option<&[ObjId]>,
) -> Option<PickHit> {
    ray_scene(pick, state, lib, start, dir, range, &RayFilter { only, ..RayFilter::default() })
}

/// The nearest object a ray meets within its range, of the kinds and lists `filter` takes.
#[must_use]
pub fn ray_scene(
    pick: &ScenePick,
    state: &SceneState,
    lib: &Library,
    start: Vec3,
    dir: Vec3,
    range: f32,
    filter: &RayFilter<'_>,
) -> Option<PickHit> {
    pick.ray_pick(state, lib, start, dir, range, &|o: &Object| filter.accepts(o))
}

/// Every object a ray meets within its range (each one's nearest hit), nearest first.
#[must_use]
pub fn ray_scene_all(
    pick: &ScenePick,
    state: &SceneState,
    lib: &Library,
    start: Vec3,
    dir: Vec3,
    range: f32,
    filter: &RayFilter<'_>,
) -> Vec<PickHit> {
    pick.ray_hits(state, lib, start, dir, range, &|o: &Object| filter.accepts(o))
}

/// The world triangles of these scene objects that may meet a box.
#[must_use]
pub fn box_triangles(state: &SceneState, lib: &Library, ids: &[ObjId], b: &Aabb) -> Vec<[Vec3; 3]> {
    let mut out = Vec::new();
    for id in ids {
        let Some(o) = state.get(*id) else { continue };
        let Some(lo) = objects::terrain::mesh_of(state, lib, o) else { continue };
        let Some(bvh) = &lo.bvh else { continue };
        let world = if o.class == ObjClass::Terrain { gz_fmt_object::Mat4::IDENTITY } else { o.world() };
        let inv = gz_fmt_object::Mat4::invert_43(&world);
        let local = crate::math::xform_box(b, &inv);
        let mut found = Vec::new();
        bvh.query_box(&local, &mut found);
        for t in found {
            if let Some(tri) = lo.tri(t) {
                out.push([
                    world.transform_tiny(tri[0]),
                    world.transform_tiny(tri[1]),
                    world.transform_tiny(tri[2]),
                ]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Object;
    use crate::objects::{GlowData, LightData};

    fn seeded(n: usize) -> SceneState {
        let mut s = SceneState::default();
        let mut seed = 12345u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 8) as f32 / (1u32 << 24) as f32
        };
        for i in 0..n {
            let mut o = if i % 2 == 0 {
                Object::new(ObjClass::Light, &format!("l{i}"), ObjData::Light(LightData::default()))
            } else {
                Object::new(
                    ObjClass::Glow,
                    &format!("g{i}"),
                    ObjData::Glow(GlowData { radius: 0.2 + next(), ..GlowData::default() }),
                )
            };
            o.xform.position = Vec3::new(next() * 100.0 - 50.0, next() * 10.0, next() * 100.0 - 50.0);
            s.insert(o);
        }
        s
    }

    #[test]
    fn the_tree_finds_what_brute_force_finds() {
        let s = seeded(300);
        let lib = Library::new(Vec::new());
        let pick = ScenePick::build(&s, &lib);
        for k in 0..40 {
            let a = k as f32 * 0.157;
            let start = Vec3::new(-60.0, 3.0, a * 3.0 - 30.0);
            let dir = Vec3::new(a.cos(), 0.02, a.sin()).normalize();
            let tree = pick.ray_pick(&s, &lib, start, dir, 500.0, &|_| true);
            let brute = s
                .objects
                .values()
                .filter_map(|o| ray_object(&s, &lib, o, start, dir, 500.0))
                .min_by(|a, b| a.t.total_cmp(&b.t));
            assert_eq!(tree.map(|h| h.id), brute.map(|h| h.id), "ray {k}");
        }
    }

    #[test]
    fn box_candidates_cover_overlaps() {
        let s = seeded(100);
        let lib = Library::new(Vec::new());
        let pick = ScenePick::build(&s, &lib);
        let b = Aabb::new(Vec3::new(-10.0, -1.0, -10.0), Vec3::new(10.0, 20.0, 10.0));
        let mut got = pick.box_candidates(&b);
        got.sort();
        let local = |_: &str| None;
        let mut want: Vec<ObjId> = s
            .objects
            .values()
            .filter(|o| {
                objects::world_box(&s, o, &local)
                    .is_some_and(|x| (0..3).all(|a| x.min[a] <= b.max[a] && x.max[a] >= b.min[a]))
            })
            .map(|o| o.id)
            .collect();
        want.sort();
        assert_eq!(got, want);
    }
}
