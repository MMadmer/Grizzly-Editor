//! The collision model the AI map is generated over (`GenerateMap`'s `CDB::Collector` of the snap
//! objects' faces): every face of a surface whose compiler shader collides, a two-sided one twice
//! (the second time turned round), in the snap list's order; and the queries the generation asks
//! of it - `BoxQuery(OPT_FULL_TEST)` and `RayQuery(OPT_ONLYNEAREST | OPT_CULL)`.
//!
//! A uniform grid over XZ finds the candidates; a query answers the triangles in the order they
//! were added, which is the collector's order.

use std::sync::Arc;

use glam::Vec3;
use gz_base::SmolStr;
use gz_fmt_xr::{ShaderXrlc, ShadersXrlc};
use rustc_hash::FxHashMap;

use super::xr::{self, tri_box};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::{ObjData, SceneState};

/// The side of a cell of the grid, metres.
const CELL: f32 = 2.0;
/// A grid never has more cells than this; a bigger level gets bigger cells.
const MAX_CELLS: f32 = 4_000_000.0;

/// The triangles of the snap objects and the grid over them.
#[derive(Debug, Default)]
pub struct CollideModel {
    pub tris: Vec<[Vec3; 3]>,
    /// Per triangle: an index into [`Self::materials`].
    pub material: Vec<u16>,
    /// The game materials of the surfaces, by name.
    pub materials: Vec<SmolStr>,
    pub bounds: Aabb,
    /// Surfaces whose compiler shader is not in the library (counted as colliding).
    pub unknown_shaders: usize,
    min: [f32; 2],
    cell: f32,
    nx: usize,
    nz: usize,
    /// `offsets[c]..offsets[c + 1]` into `items` are the triangles of cell `c`.
    offsets: Vec<u32>,
    items: Vec<u32>,
}

fn collides(shaders: Option<&ShadersXrlc>, name: &str, unknown: &mut usize) -> bool {
    let Some(lib) = shaders else { return true };
    match lib.shaders.iter().find(|s| s.name().eq_ignore_ascii_case(name)) {
        Some(s) => s.flags & ShaderXrlc::COLLISION != 0,
        None => {
            *unknown += 1;
            true
        }
    }
}

impl CollideModel {
    /// The faces of the scene objects named, as `GenerateMap` collects them.
    #[must_use]
    pub fn build(state: &SceneState, lib: &Library, snap: &[SmolStr], shaders: Option<&ShadersXrlc>) -> Self {
        let mut tris: Vec<[Vec3; 3]> = Vec::new();
        let mut material: Vec<u16> = Vec::new();
        let mut materials: Vec<SmolStr> = Vec::new();
        let mut mtl_index: FxHashMap<String, u16> = FxHashMap::default();
        let mut unknown = 0;
        for name in snap {
            let Some(o) = state.find(ObjClass::SceneObject, &name.to_lowercase()) else { continue };
            let ObjData::SceneObject(d) = &o.data else { continue };
            let Some(lo) = lib.cached(&d.reference) else { continue };
            let world = o.world();
            let obj = &lo.object;
            for m in &obj.meshes {
                for sf in &m.surf_faces {
                    let Some(surf) = obj.surfaces.get(sf.surface as usize) else { continue };
                    if !collides(shaders, &surf.shader_xrlc, &mut unknown) {
                        continue;
                    }
                    let key = surf.game_mtl.to_lowercase();
                    let next = materials.len() as u16;
                    let mi = *mtl_index.entry(key).or_insert_with(|| {
                        materials.push(surf.game_mtl.clone());
                        next
                    });
                    let two = surf.flags.contains(gz_fmt_object::SurfaceFlags::TWO_SIDED);
                    for &fi in sf.faces.iter() {
                        let Some(f) = m.faces.get(fi as usize) else { continue };
                        let at = |k: usize| {
                            m.points
                                .get(f.v[k].point as usize)
                                .map_or(Vec3::ZERO, |p| world.transform_tiny(*p))
                        };
                        let v = [at(0), at(1), at(2)];
                        tris.push(v);
                        material.push(mi);
                        if two {
                            tris.push([v[2], v[1], v[0]]);
                            material.push(mi);
                        }
                    }
                }
            }
        }
        let mut out = Self { tris, material, materials, unknown_shaders: unknown, ..Self::default() };
        out.index();
        out
    }

    /// A model of given triangles (tests, the hand-made scenes), all of one material.
    #[must_use]
    pub fn from_tris(tris: Vec<[Vec3; 3]>, material: Vec<u16>, materials: Vec<SmolStr>) -> Self {
        let mut out = Self { tris, material, materials, ..Self::default() };
        out.index();
        out
    }

    fn index(&mut self) {
        let mut b = Aabb::INVALID;
        for t in &self.tris {
            for v in t {
                b.modify(*v);
            }
        }
        self.bounds = b;
        if !b.is_valid() {
            self.nx = 0;
            self.nz = 0;
            self.offsets = vec![0];
            return;
        }
        let (w, d) = ((b.max.x - b.min.x).max(CELL), (b.max.z - b.min.z).max(CELL));
        let mut cell = CELL;
        while (w / cell).ceil() * (d / cell).ceil() > MAX_CELLS {
            cell *= 2.0;
        }
        self.cell = cell;
        self.min = [b.min.x, b.min.z];
        self.nx = ((w / cell).ceil() as usize).max(1);
        self.nz = ((d / cell).ceil() as usize).max(1);
        let n = self.nx * self.nz;
        let mut counts = vec![0u32; n + 1];
        let ranges: Vec<[usize; 4]> = self.tris.iter().map(|t| self.cell_range_of_tri(t)).collect();
        for r in &ranges {
            for z in r[2]..=r[3] {
                for x in r[0]..=r[1] {
                    counts[z * self.nx + x + 1] += 1;
                }
            }
        }
        for i in 1..=n {
            counts[i] += counts[i - 1];
        }
        let mut items = vec![0u32; counts[n] as usize];
        let mut fill = counts.clone();
        for (ti, r) in ranges.iter().enumerate() {
            for z in r[2]..=r[3] {
                for x in r[0]..=r[1] {
                    let c = z * self.nx + x;
                    if let Some(slot) = items.get_mut(fill[c] as usize) {
                        *slot = ti as u32;
                    }
                    fill[c] += 1;
                }
            }
        }
        self.offsets = counts;
        self.items = items;
    }

    fn cell_x(&self, x: f32) -> usize {
        (((x - self.min[0]) / self.cell).floor().max(0.0) as usize).min(self.nx.saturating_sub(1))
    }

    fn cell_z(&self, z: f32) -> usize {
        (((z - self.min[1]) / self.cell).floor().max(0.0) as usize).min(self.nz.saturating_sub(1))
    }

    fn cell_range_of_tri(&self, t: &[Vec3; 3]) -> [usize; 4] {
        let (x0, x1) = (t[0].x.min(t[1].x).min(t[2].x), t[0].x.max(t[1].x).max(t[2].x));
        let (z0, z1) = (t[0].z.min(t[1].z).min(t[2].z), t[0].z.max(t[1].z).max(t[2].z));
        [self.cell_x(x0), self.cell_x(x1), self.cell_z(z0), self.cell_z(z1)]
    }

    /// The triangles whose cells a box's XZ footprint covers, ascending, each once.
    fn candidates(&self, min: Vec3, max: Vec3, out: &mut Vec<u32>) {
        out.clear();
        if self.nx == 0
            || !(min.x <= self.bounds.max.x
                && max.x >= self.bounds.min.x
                && min.z <= self.bounds.max.z
                && max.z >= self.bounds.min.z)
        {
            return;
        }
        let (x0, x1, z0, z1) =
            (self.cell_x(min.x), self.cell_x(max.x), self.cell_z(min.z), self.cell_z(max.z));
        for z in z0..=z1 {
            for x in x0..=x1 {
                let c = z * self.nx + x;
                let (a, b) = (self.offsets[c] as usize, self.offsets[c + 1] as usize);
                if let Some(s) = self.items.get(a..b) {
                    out.extend_from_slice(s);
                }
            }
        }
        if z1 > z0 || x1 > x0 {
            out.sort_unstable();
            out.dedup();
        }
    }

    /// `BoxQuery(bb, full ? OPT_FULL_TEST : 0)`: the triangles meeting the box, in model order.
    #[must_use]
    pub fn box_query(&self, bb: &Aabb, full: bool) -> Vec<u32> {
        let mut cand = Vec::new();
        self.candidates(bb.min, bb.max, &mut cand);
        // `getcenter` and `getradius`
        let c =
            Vec3::new((bb.min.x + bb.max.x) * 0.5, (bb.min.y + bb.max.y) * 0.5, (bb.min.z + bb.max.z) * 0.5);
        let e =
            Vec3::new((bb.max.x - bb.min.x) * 0.5, (bb.max.y - bb.min.y) * 0.5, (bb.max.z - bb.min.z) * 0.5);
        cand.retain(|&i| self.tris.get(i as usize).is_some_and(|t| tri_box(c, e, t, full)));
        cand
    }

    /// `RayQuery(OPT_ONLYNEAREST | OPT_CULL)`: the nearest front face within `range` (strictly
    /// ahead), and its range.
    #[must_use]
    pub fn ray_nearest(&self, start: Vec3, dir: Vec3, range: f32) -> Option<(u32, f32)> {
        let end = xr::mad(start, dir, range);
        let mut cand = Vec::new();
        self.candidates(start.min(end), start.max(end), &mut cand);
        let mut best: Option<(u32, f32)> = None;
        let mut reach = range;
        for i in cand {
            let Some(t) = self.tris.get(i as usize) else { continue };
            let Some(r) = xr::ray_tri(start, dir, t, true) else { continue };
            if r <= 0.0 || r > reach {
                continue;
            }
            if best.is_none_or(|(_, b)| r < b) {
                best = Some((i, r));
                reach = r;
            }
        }
        best
    }

    #[must_use]
    pub fn tri(&self, i: u32) -> Option<&[Vec3; 3]> {
        self.tris.get(i as usize)
    }

    /// Per material: whether its name is in `ignored` (case-insensitive).
    #[must_use]
    pub fn ignored_mask(&self, ignored: &[SmolStr]) -> Vec<bool> {
        self.materials.iter().map(|m| ignored.iter().any(|i| i.eq_ignore_ascii_case(m))).collect()
    }
}

/// What a model was made of: rebuilt only when this changes.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelKey {
    pub objects: Vec<(SmolStr, [u32; 16])>,
    pub library: u64,
    pub shaders: usize,
}

impl ModelKey {
    #[must_use]
    pub fn of(state: &SceneState, lib: &Library, snap: &[SmolStr], shaders: Option<&ShadersXrlc>) -> Self {
        let objects = snap
            .iter()
            .filter_map(|n| state.find(ObjClass::SceneObject, &n.to_lowercase()))
            .map(|o| {
                let m = o.world();
                let mut bits = [0u32; 16];
                for (r, row) in m.0.iter().enumerate() {
                    for (k, v) in row.iter().enumerate() {
                        bits[r * 4 + k] = v.to_bits();
                    }
                }
                (SmolStr::new(o.ref_name().unwrap_or_default()), bits)
            })
            .collect();
        Self { objects, library: lib.rev().0, shaders: shaders.map_or(0, |s| s.shaders.len() + 1) }
    }
}

/// The last model built, kept for the next brush stroke (a session service).
#[derive(Default)]
pub struct ModelCache {
    last: parking_lot::Mutex<Option<(ModelKey, Arc<CollideModel>)>>,
}

impl ModelCache {
    /// The model for this scene, built when what it is made of changed.
    pub fn get(
        &self,
        state: &SceneState,
        lib: &Library,
        snap: &[SmolStr],
        shaders: Option<&ShadersXrlc>,
    ) -> Arc<CollideModel> {
        let key = ModelKey::of(state, lib, snap, shaders);
        if let Some((k, m)) = self.last.lock().as_ref()
            && *k == key
        {
            return m.clone();
        }
        let m = Arc::new(CollideModel::build(state, lib, snap, shaders));
        *self.last.lock() = Some((key, m.clone()));
        m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn floor(size: f32) -> Vec<[Vec3; 3]> {
        let (a, b, c, d) = (
            Vec3::new(-size, 0.0, -size),
            Vec3::new(size, 0.0, -size),
            Vec3::new(size, 0.0, size),
            Vec3::new(-size, 0.0, size),
        );
        // clockwise from above: front faces look up
        vec![[a, d, c], [a, c, b]]
    }

    #[test]
    fn boxes_and_rays_find_the_floor() {
        let m = CollideModel::from_tris(floor(10.0), vec![0, 0], vec!["default".into()]);
        let hits = m.box_query(&Aabb::new(Vec3::new(-0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, 0.5)), true);
        assert!(!hits.is_empty());
        assert!(
            m.box_query(&Aabb::new(Vec3::new(-0.5, 1.0, -0.5), Vec3::new(0.5, 2.0, 0.5)), true).is_empty()
        );
        let (_, r) = m.ray_nearest(Vec3::new(0.3, 1.0, 0.3), Vec3::NEG_Y, 3.0).unwrap();
        assert!((r - 1.0).abs() < 1e-5);
        assert!(m.ray_nearest(Vec3::new(0.3, -1.0, 0.3), Vec3::Y, 3.0).is_none());
    }
}
