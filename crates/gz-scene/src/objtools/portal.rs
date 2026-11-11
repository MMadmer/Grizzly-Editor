//! Portals: computed from the sectors' faces (`portalutils.cpp`'s `sCollector`), simplified and
//! turned to face the right way (`portal.cpp`'s `Update` / `Simplify`), inverted, and the
//! duplicates removed (`EScenePortalTools.cpp`'s `RemoveSimilar`).
//!
//! The faces are welded on a grid in the C++ order, the edges between two sectors found per face
//! (in parallel, collected in face order), sorted, chained into loops and each loop exported as
//! a portal. Each portal's checks run in parallel; its orientation is the C++ vote of 100 random
//! ray pairs, from a generator of its own.

use std::sync::Arc;

use glam::{Vec2, Vec4};
use gz_base::SmolStr;
use gz_fmt_level::Bvh;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use super::geom::{self, CRandom, mknormal, normalize, plane_build, plane_fit, similar};
use super::sector::{self, Item};
use crate::class::ObjClass;
use crate::library::Library;
use super::geom::{EPS, EPS_L, EPS_S};
use crate::math::{Aabb, Mat4, Vec3};
use crate::model::{ObjData, ObjId, SceneState};
use crate::objects::PortalData;

/// `clpMX`, `clpMY`, `clpMZ`.
const CLP: [usize; 3] = [64, 24, 64];
/// `XR_MAX_PORTAL_VERTS`.
pub const MAX_PORTAL_VERTS: usize = 6;

#[derive(Clone, Copy, Debug)]
struct Face {
    v: [u32; 3],
    sector: usize,
}

impl Face {
    fn has(&self, v: u32) -> bool {
        self.v.contains(&v)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Edge {
    s: [usize; 2],
    v: [u32; 2],
}

/// `sCollector`: welded vertices, faces, the edges between sectors and the loops of them.
struct Collector {
    verts: Vec<Vec3>,
    adj: Vec<Vec<u32>>,
    faces: Vec<Face>,
    min: Vec3,
    scale: Vec3,
    eps: Vec3,
    grid: Vec<Vec<u32>>,
}

impl Collector {
    fn new(bb: &Aabb) -> Self {
        let scale = bb.max - bb.min;
        let cell = |a: usize| (scale[a] / CLP[a] as f32 / 2.0).min(EPS_L);
        Self {
            verts: Vec::new(),
            adj: Vec::new(),
            faces: Vec::new(),
            min: bb.min,
            scale,
            eps: Vec3::new(cell(0), cell(1), cell(2)),
            grid: vec![Vec::new(); (CLP[0] + 1) * (CLP[1] + 1) * (CLP[2] + 1)],
        }
    }

    fn cell(&self, v: Vec3, a: usize) -> usize {
        let f = ((v[a] - self.min[a]) / self.scale[a] * CLP[a] as f32).floor();
        // oracle: the C++ asserts the cell is inside; a flat box (scale 0) gives NaN there
        if f.is_finite() { (f.max(0.0) as usize).min(CLP[a]) } else { 0 }
    }

    fn slot(&self, x: usize, y: usize, z: usize) -> usize {
        (x * (CLP[1] + 1) + y) * (CLP[2] + 1) + z
    }

    /// `VPack`: the first vertex within `EPS_L` in the vertex's cell, or a new one put into its
    /// cell and the neighbours its epsilon reaches.
    fn pack(&mut self, v: Vec3) -> u32 {
        let (ix, iy, iz) = (self.cell(v, 0), self.cell(v, 1), self.cell(v, 2));
        let here = self.slot(ix, iy, iz);
        if let Some(cell) = self.grid.get(here) {
            for &i in cell {
                if self.verts.get(i as usize).is_some_and(|p| similar(*p, v, EPS_L)) {
                    return i;
                }
            }
        }
        let p = u32::try_from(self.verts.len()).unwrap_or(u32::MAX);
        self.verts.push(v);
        self.adj.push(Vec::new());
        let e = v + self.eps;
        let (ex, ey, ez) = (self.cell(e, 0), self.cell(e, 1), self.cell(e, 2));
        let mut slots = vec![here];
        if ex != ix {
            slots.push(self.slot(ex, iy, iz));
        }
        if ey != iy {
            slots.push(self.slot(ix, ey, iz));
        }
        if ez != iz {
            slots.push(self.slot(ix, iy, ez));
        }
        if ex != ix && ey != iy {
            slots.push(self.slot(ex, ey, iz));
        }
        if ex != ix && ez != iz {
            slots.push(self.slot(ex, iy, ez));
        }
        if ey != iy && ez != iz {
            slots.push(self.slot(ix, ey, ez));
        }
        if ex != ix && ey != iy && ez != iz {
            slots.push(self.slot(ex, ey, ez));
        }
        for s in slots {
            if let Some(c) = self.grid.get_mut(s) {
                c.push(p);
            }
        }
        p
    }

    fn add_face(&mut self, tri: &[Vec3; 3], sector: usize) {
        let v = [self.pack(tri[0]), self.pack(tri[1]), self.pack(tri[2])];
        self.faces.push(Face { v, sector });
    }

    /// `update_adjacency`.
    fn update_adjacency(&mut self) {
        for (i, f) in self.faces.iter().enumerate() {
            let i = u32::try_from(i).unwrap_or(u32::MAX);
            for v in f.v {
                if let Some(a) = self.adj.get_mut(v as usize) {
                    a.push(i);
                }
            }
        }
    }

    /// `find_edges`: for each face and each of its three edges, every face of another sector
    /// that shares the edge gives one. Faces in parallel, the edges collected in face order.
    fn find_edges(&self) -> Vec<Edge> {
        let per_face = |(i, f): (usize, &Face)| -> Vec<Edge> {
            let mut out = Vec::new();
            for (a, b) in [(0, 1), (1, 2), (2, 0)] {
                let Some(adj) = self.adj.get(f.v[a] as usize) else { continue };
                for &t in adj {
                    if t as usize == i {
                        continue;
                    }
                    let Some(tf) = self.faces.get(t as usize) else { continue };
                    if tf.sector == f.sector || !tf.has(f.v[b]) {
                        continue;
                    }
                    out.push(Edge { s: [f.sector, tf.sector], v: [f.v[a], f.v[b]] });
                }
            }
            out
        };
        let lists: Vec<Vec<Edge>> = if self.faces.len() > 4096 {
            self.faces.par_iter().enumerate().map(per_face).collect()
        } else {
            self.faces.iter().enumerate().map(per_face).collect()
        };
        lists.into_iter().flatten().collect()
    }
}

/// `sort_edges`: each edge's vertices and sectors in order, the list sorted and made unique.
// oracle: the C++ sorts with `memcmp` over the first 16 bytes, which on x64 are the two sector
// pointers only, and `std::unique` then removes only neighbours; here the sort is by sectors
// (their order in the scene stands in for the pointers) and then vertices, so every duplicate
// goes
fn sort_edges(edges: &mut Vec<Edge>) {
    for e in edges.iter_mut() {
        if e.v[0] > e.v[1] {
            e.v.swap(0, 1);
        }
        if e.s[0] > e.s[1] {
            e.s.swap(0, 1);
        }
    }
    edges.sort_unstable_by(|a, b| a.s.cmp(&b.s).then(a.v.cmp(&b.v)));
    edges.dedup();
}

/// `make_portals`: chains of edges of one sector pair. The search looks only at the edges of
/// the chain's pair, which are together in the sorted list: the first match is the C++ one.
fn make_portals(edges: &mut [Edge]) -> Vec<(Vec<usize>, [usize; 2])> {
    let mut used = vec![false; edges.len()];
    let mut out = Vec::new();
    let mut start = 0;
    while start < edges.len() {
        let pair = edges[start].s;
        let mut end = start;
        while end < edges.len() && edges[end].s == pair {
            end += 1;
        }
        for e_it in start..end {
            if used[e_it] {
                continue;
            }
            let mut chain: std::collections::VecDeque<usize> = std::collections::VecDeque::new();
            chain.push_back(e_it);
            used[e_it] = true;
            while let (Some(&f), Some(&l)) = (chain.front(), chain.back()) {
                let v_first = edges[f].v[0];
                let v_last = edges[l].v[1];
                let mut found = false;
                for i in start..end {
                    if used[i] {
                        continue;
                    }
                    let e = edges[i];
                    if v_last == e.v[0] {
                        used[i] = true;
                        chain.push_back(i);
                        found = true;
                        break;
                    }
                    if v_last == e.v[1] {
                        used[i] = true;
                        edges[i].v.swap(0, 1);
                        chain.push_back(i);
                        found = true;
                        break;
                    }
                    if v_first == e.v[0] {
                        used[i] = true;
                        edges[i].v.swap(0, 1);
                        chain.push_front(i);
                        found = true;
                        break;
                    }
                    if v_first == e.v[1] {
                        used[i] = true;
                        chain.push_front(i);
                        found = true;
                        break;
                    }
                }
                if !found {
                    break;
                }
            }
            out.push((chain.into_iter().collect(), pair));
        }
        start = end;
    }
    out
}

/// What the faces of a sector answer a ray (`CSector::RayPick`): the items' visible meshes, each
/// with the inverse of its object's transform.
#[derive(Clone, Default)]
pub struct SectorRays {
    items: Vec<(Mat4, Arc<Bvh>)>,
}

impl SectorRays {
    /// Whether any item mesh is hit within `range`.
    #[must_use]
    pub fn hit(&self, start: Vec3, dir: Vec3, range: f32) -> bool {
        self.items.iter().any(|(inv, bvh)| geom::mesh_ray_hits(bvh, inv, start, dir, range))
    }
}

/// The ray trees of sectors' items, each library mesh built once (in parallel).
#[must_use]
pub fn sector_rays(sectors: &[Vec<Item>]) -> Vec<SectorRays> {
    let mut keys: Vec<(SmolStr, usize)> = Vec::new();
    let mut libs = FxHashMap::default();
    for it in sectors.iter().flatten() {
        let k = (it.lib.name.clone(), it.mesh);
        if !libs.contains_key(&k) {
            libs.insert(k.clone(), it.lib.clone());
            keys.push(k);
        }
    }
    let built: FxHashMap<(SmolStr, usize), Arc<Bvh>> = keys
        .par_iter()
        .filter_map(|k| {
            let lo = libs.get(k)?;
            geom::mesh_bvh(lo, k.1).map(|b| (k.clone(), Arc::new(b)))
        })
        .collect();
    sectors
        .iter()
        .map(|items| SectorRays {
            items: items
                .iter()
                .filter_map(|it| {
                    let b = built.get(&(it.lib.name.clone(), it.mesh))?;
                    Some((Mat4::invert_43(&it.object.world()), b.clone()))
                })
                .collect(),
        })
        .collect()
}

/// `tri_area`: Heron's formula in doubles over the float distances.
fn tri_area(p0: Vec2, p1: Vec2, p2: Vec2) -> f64 {
    let e1 = f64::from(p0.distance(p1));
    let e2 = f64::from(p1.distance(p2));
    let e3 = f64::from(p2.distance(p0));
    let p = (e1 + e2 + e3) / 2.0;
    (p * (p - e1) * (p - e2) * (p - e3)).sqrt()
}

/// `Isect2DLvs2DL`: where the line through `p1 p2` meets the line through `p3 p4`.
fn isect_lines(p1: Vec2, p2: Vec2, p3: Vec2, p4: Vec2) -> Option<Vec2> {
    let (a1, b1) = (f64::from(p2.y) - f64::from(p1.y), f64::from(p1.x) - f64::from(p2.x));
    let c1 = f64::from(p2.x) * f64::from(p1.y) - f64::from(p1.x) * f64::from(p2.y);
    let (a2, b2) = (f64::from(p4.y) - f64::from(p3.y), f64::from(p3.x) - f64::from(p4.x));
    let c2 = f64::from(p4.x) * f64::from(p3.y) - f64::from(p3.x) * f64::from(p4.y);
    let denom = a1 * b2 - a2 * b1;
    if denom.abs() < f64::from(EPS) {
        return None;
    }
    Some(Vec2::new(((b1 * c2 - b2 * c1) / denom) as f32, ((a2 * c1 - a1 * c2) / denom) as f32))
}

/// `SimplifyVertices`: the edge whose two neighbours' lines meet closest replaced by that
/// meeting point - one vertex fewer.
fn simplify_vertices(v: &mut Vec<Vec2>) {
    let n = v.len();
    if n < 3 {
        return;
    }
    let mut min = f64::MAX;
    let mut edge: Option<(usize, Vec2)> = None;
    for i in 0..n {
        let l1a = if i == 0 { n - 1 } else { i - 1 };
        let l1b = i;
        let l2a = (i + 1) % n;
        let l2b = (i + 2) % n;
        let Some(p) = isect_lines(v[l1a], v[l1b], v[l2b], v[l2a]) else { continue };
        let area = tri_area(v[l1b], p, v[l2a]);
        if area < min {
            min = area;
            edge = Some((i, p));
        }
    }
    let Some((i, p)) = edge else { return };
    let l2a = (i + 1) % n;
    v[i] = p;
    v.remove(l2a);
}

/// A portal being built: what `CPortal` holds besides its name.
#[derive(Clone, Debug)]
pub struct Built {
    pub data: PortalData,
    pub simplified: Vec<Vec3>,
    pub center: Vec3,
    pub normal: Vec3,
    pub valid: bool,
}

/// `CPortal::Simplify`: the plane, the 2D hull of the vertices on it, at most six corners.
fn simplify(b: &mut Built) {
    let (offset, normal) = plane_fit(&b.data.vertices);
    let from = offset;
    let view = normalize(-normal);
    let y = if normal.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
    let right = normalize(y.cross(normal));
    let up0 = normalize(normal.cross(right));
    // `build_camera(from, at, up)`
    let up = normalize(up0 - view * up0.dot(view));
    let right = up.cross(view);
    let to_2d = |p: Vec3| Vec2::new(p.dot(right) - from.dot(right), p.dot(up) - from.dot(up));
    let points: Vec<Vec2> = b.data.vertices.iter().map(|p| to_2d(*p)).collect();
    let hull = geom::convex_hull(&points);
    if hull.is_empty() {
        return;
    }
    let mut verts: Vec<Vec2> = hull.iter().filter_map(|i| points.get(*i).copied()).collect();
    while verts.len() > MAX_PORTAL_VERTS {
        let n = verts.len();
        simplify_vertices(&mut verts);
        if n == verts.len() {
            b.valid = false;
            return;
        }
    }
    if verts.len() <= 2 {
        b.valid = false;
        return;
    }
    b.valid = true;
    let back = |p: Vec2| from + right * p.x + up * p.y;
    b.simplified = verts.iter().map(|p| back(*p)).collect();
    let center = b.simplified.iter().copied().sum::<Vec3>() / b.simplified.len() as f32;
    let mut norm = Vec3::ZERO;
    for w in b.simplified.windows(2) {
        norm += mknormal(center, w[0], w[1]);
    }
    norm /= b.simplified.len() as f32;
    let m = norm.length();
    if m > 0.0 {
        norm /= m;
    }
    if norm.dot(b.normal) < 0.0 {
        b.simplified.reverse();
        b.normal = -norm;
    } else {
        b.normal = norm;
    }
}

/// `CPortal::InvertOrientation`.
pub fn invert(b: &mut Built) {
    b.data.vertices.reverse();
    b.simplified.reverse();
    b.normal = -b.normal;
}

/// `CPortal::Update(bLoadMode)`: the centre, the normal, the simplified model; with the sectors'
/// rays the orientation vote. Messages in the C++ words go to `log`.
pub fn update(data: PortalData, name: &str, rays: Option<(&SectorRays, &SectorRays)>, seed: i32, log: &mut Vec<String>) -> Built {
    let bounds = Aabb::from_points(data.vertices.iter());
    let (center, _) = geom::box_sphere(&bounds);
    let mut b = Built { data, simplified: Vec::new(), center, normal: Vec3::ZERO, valid: false };
    if b.data.vertices.len() < 3 {
        log.push(format!("Portal: '{name}' can't create.\nInvalid portal. (m_Vertices.size()<3)"));
        return b;
    }
    let mut n = Vec3::ZERO;
    for w in b.data.vertices.windows(2) {
        n += mknormal(center, w[0], w[1]);
    }
    let m = n.length();
    if m.abs() <= EPS_S {
        log.push("Portal: Degenerate portal found.".to_owned());
        return b;
    }
    b.normal = n / m;
    simplify(&mut b);
    if let (Some((front, back)), true) = (rays, b.valid) {
        let mut rng = CRandom::new(seed);
        let inv = -b.normal;
        let (mut a, mut bb) = (0, 0);
        for _ in 0..100 {
            let sf = rng.random_dir_cone(b.normal, std::f32::consts::FRAC_PI_4);
            let sb = rng.random_dir_cone(inv, std::f32::consts::FRAC_PI_4);
            a += i32::from(front.hit(b.center, sf, 1000.0));
            a += i32::from(back.hit(b.center, sb, 1000.0));
            bb += i32::from(front.hit(b.center, sb, 1000.0));
            bb += i32::from(back.hit(b.center, sf, 1000.0));
        }
        if a < bb {
            invert(&mut b);
        } else if a == bb {
            log.push(format!("Check portal orientation: '{name}'"));
        }
    }
    b
}

/// The portals the sectors' faces give: each loop's portal, the number of loops (what the C++
/// counts as "computed"), and the C++ messages.
pub struct Computed {
    pub portals: Vec<PortalData>,
    pub loops: usize,
    pub log: Vec<String>,
}

/// Why a computation stopped early.
pub struct Stopped;

/// `CalculateSelectedPortals(sectors)` over the snapshot: `sectors` in the order the C++ walks
/// them; `names` gives the new portals' names in order; `stop` is asked between the steps.
pub fn compute(
    state: &SceneState,
    lib: &Library,
    sectors: &[ObjId],
    names: &mut dyn FnMut() -> String,
    stop: &(dyn Fn() -> bool + Sync),
    stage: &dyn Fn(&str),
) -> Result<Computed, Stopped> {
    let bb = sector::scene_objects_box(state, lib).unwrap_or(Aabb::new(Vec3::ZERO, Vec3::ZERO));
    let mut cl = Collector::new(&bb);
    let mut items_of: Vec<Vec<Item>> = Vec::with_capacity(sectors.len());
    let mut sector_names: Vec<SmolStr> = Vec::with_capacity(sectors.len());
    for id in sectors {
        let Some(s) = state.get(*id) else { continue };
        let ObjData::Sector(d) = &s.data else { continue };
        items_of.push(sector::items(state, lib, d));
        sector_names.push(s.name.clone());
    }
    stage("Welding the vertices");
    // the transforms in parallel; the weld in the C++ order
    let faces: Vec<Vec<(usize, [Vec3; 3])>> = items_of
        .par_iter()
        .enumerate()
        .map(|(si, items)| {
            items
                .iter()
                .filter(|it| !geom::is_mu_static(&it.lib))
                .flat_map(|it| {
                    geom::mesh_world_faces(&it.lib, it.mesh, &it.object.world()).into_iter().map(move |f| (si, f))
                })
                .collect()
        })
        .collect();
    for (si, f) in faces.iter().flatten() {
        cl.add_face(f, *si);
    }
    if stop() {
        return Err(Stopped);
    }
    stage("Searching the edges");
    cl.update_adjacency();
    let mut edges = cl.find_edges();
    if stop() {
        return Err(Stopped);
    }
    sort_edges(&mut edges);
    stage("Calculating the portals");
    let loops = make_portals(&mut edges);
    if stop() {
        return Err(Stopped);
    }
    stage("Building the portals");
    let mut log = Vec::new();
    let mut todo: Vec<(String, PortalData, [usize; 2])> = Vec::new();
    for (chain, pair) in &loops {
        let (fa, fb) = (
            sector_names.get(pair[0]).cloned().unwrap_or_default(),
            sector_names.get(pair[1]).cloned().unwrap_or_default(),
        );
        if chain.len() > 1 {
            let mut vlist: Vec<u32> = Vec::with_capacity(chain.len() * 2);
            for e in chain {
                if let Some(e) = edges.get(*e) {
                    vlist.push(e.v[0]);
                    vlist.push(e.v[1]);
                }
            }
            vlist.dedup();
            let vertices = vlist.iter().filter_map(|i| cl.verts.get(*i as usize).copied()).collect();
            let name = names();
            todo.push((name, PortalData { front: fa, back: fb, vertices }, *pair));
        } else {
            log.push(format!("Can't create Portal from one edge :(\nPlease check geometry.\n'{fa}'<->'{fb}'"));
        }
    }
    let rays = sector_rays(&items_of);
    let built: Vec<(Built, Vec<String>)> = todo
        .into_par_iter()
        .enumerate()
        .map(|(i, (name, data, pair))| {
            let mut l = Vec::new();
            let r = match (rays.get(pair[0]), rays.get(pair[1])) {
                (Some(a), Some(b)) => Some((a, b)),
                _ => None,
            };
            let b = update(data, &name, r, 0x5EC7 + i as i32, &mut l);
            (b, l)
        })
        .collect();
    let mut portals = Vec::new();
    for (b, l) in built {
        log.extend(l);
        if b.valid {
            portals.push(b.data);
        } else {
            log.push(format!(
                "Can't simplify Portal :(\nPlease check geometry.\n'{}'<->'{}'",
                b.data.front, b.data.back
            ));
        }
    }
    Ok(Computed { portals, loops: loops.len(), log })
}

/// `plane_dot`.
fn plane_dot(a: Vec4, b: Vec4) -> f32 {
    a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w
}

/// `co_plane_pred`'s plane: the fit's plane divided by its four-component length.
fn co_plane(p: &PortalData) -> Vec4 {
    let (o, n) = plane_fit(&p.vertices);
    let pl = plane_build(o, n);
    let magn = plane_dot(pl, pl).sqrt();
    if magn > 0.0 { pl / magn } else { pl }
}

/// `contains(P, P0)`: `P` holds `P0`, or does once grown by a millimetre.
fn holds(p: &Aabb, p0: &Aabb) -> bool {
    let h = |b: &Aabb, q: &Aabb| geom::box_holds(b, q.min) && geom::box_holds(b, q.max);
    if h(p, p0) {
        return true;
    }
    let mut g = *p;
    g.grow(0.001);
    h(&g, p0)
}

fn portal_box(p: &PortalData) -> Aabb {
    if p.vertices.is_empty() { Aabb::new(Vec3::ZERO, Vec3::ZERO) } else { Aabb::from_points(p.vertices.iter()) }
}

/// `RemoveSimilar`: the portals that repeat another of the same sectors (one plane, the box
/// inside the other's), in the C++ order.
#[must_use]
pub fn similar_portals(state: &SceneState) -> Vec<ObjId> {
    let all: Vec<(ObjId, PortalData)> = state
        .of_class(ObjClass::Portal)
        .filter_map(|o| match &o.data {
            ObjData::Portal(p) => Some((o.id, p.clone())),
            _ => None,
        })
        .collect();
    let planes: Vec<Vec4> = all.iter().map(|(_, p)| co_plane(p)).collect();
    let boxes: Vec<Aabb> = all.iter().map(|(_, p)| portal_box(p)).collect();
    let mut live: Vec<usize> = (0..all.len()).collect();
    let mut deleted: Vec<usize> = Vec::new();
    let mut it = 0;
    while it < all.len() {
        if deleted.contains(&it) {
            it += 1;
            continue;
        }
        let me = &all[it].1;
        let found = live.iter().position(|&j| {
            if j == it {
                return false;
            }
            let other = &all[j].1;
            if !(other.front.eq_ignore_ascii_case(&me.front) && other.back.eq_ignore_ascii_case(&me.back)) {
                return false;
            }
            let dot = plane_dot(planes[it], planes[j]).abs();
            (dot - 1.0).abs() < EPS_L && holds(&boxes[it], &boxes[j])
        });
        match found {
            Some(k) => {
                deleted.push(live.remove(k));
                it = 0;
            }
            None => it += 1,
        }
    }
    deleted.iter().map(|i| all[*i].0).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Two unit rooms open towards each other at x = 1, each a sector: the four edges of the
    /// opening chain into one loop of five vertices (the first one again at the end, as
    /// `std::unique` leaves it) - one portal of four corners, facing along x.
    #[test]
    fn two_boxes_sharing_a_wall_give_one_portal_of_four_vertices() {
        fn quad(min: Vec3, axis: usize, at: f32) -> Vec<[Vec3; 3]> {
            let (u, v) = match axis {
                0 => (1, 2),
                1 => (0, 2),
                _ => (0, 1),
            };
            let corner = |a: f32, b: f32| {
                let mut p = min;
                p[axis] = at;
                p[u] = min[u] + a;
                p[v] = min[v] + b;
                p
            };
            vec![
                [corner(0.0, 0.0), corner(1.0, 0.0), corner(1.0, 1.0)],
                [corner(0.0, 0.0), corner(1.0, 1.0), corner(0.0, 1.0)],
            ]
        }
        // a unit room without the wall `open` (axis 0; 0 = its low side, 1 = its high side)
        fn room(min: Vec3, open: usize) -> Vec<[Vec3; 3]> {
            let mut f = Vec::new();
            for axis in 0..3 {
                for side in 0..2 {
                    if axis == 0 && side == open {
                        continue;
                    }
                    f.extend(quad(min, axis, min[axis] + side as f32));
                }
            }
            f
        }
        let bb = Aabb::new(Vec3::ZERO, Vec3::new(2.0, 1.0, 1.0));
        let mut cl = Collector::new(&bb);
        for f in room(Vec3::ZERO, 1) {
            cl.add_face(&f, 0);
        }
        for f in room(Vec3::new(1.0, 0.0, 0.0), 0) {
            cl.add_face(&f, 1);
        }
        cl.update_adjacency();
        let mut edges = cl.find_edges();
        sort_edges(&mut edges);
        assert_eq!(edges.len(), 4);
        let loops = make_portals(&mut edges);
        assert_eq!(loops.len(), 1);
        let (chain, pair) = &loops[0];
        assert_eq!(*pair, [0, 1]);
        assert_eq!(chain.len(), 4);
        let mut vlist: Vec<u32> = chain.iter().flat_map(|e| edges[*e].v).collect();
        vlist.dedup();
        assert_eq!(vlist.len(), 5);
        assert_eq!(vlist.first(), vlist.last());
        let verts: Vec<Vec3> = vlist.iter().map(|i| cl.verts[*i as usize]).collect();
        assert!(verts.iter().all(|v| (v.x - 1.0).abs() < 1e-6));
        let data = PortalData { front: "a".into(), back: "b".into(), vertices: verts };
        let mut log = Vec::new();
        let b = update(data, "00", None, 1, &mut log);
        assert!(b.valid, "{log:?}");
        assert_eq!(b.simplified.len(), 4);
        assert!((b.normal.x.abs() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn a_portal_repeating_another_is_found() {
        let mut s = SceneState::default();
        let sq = |k: f32| PortalData {
            front: "a".into(),
            back: "b".into(),
            vertices: vec![Vec3::ZERO, Vec3::new(k, 0.0, 0.0), Vec3::new(k, k, 0.0), Vec3::new(0.0, k, 0.0)],
        };
        s.insert(crate::model::Object::new(ObjClass::Portal, "00", ObjData::Portal(sq(2.0))));
        let small = s.insert(crate::model::Object::new(ObjClass::Portal, "01", ObjData::Portal(sq(1.0))));
        assert_eq!(similar_portals(&s), vec![small]);
    }

    #[test]
    fn simplifying_a_heptagon_leaves_six_corners() {
        let mut v: Vec<Vec2> = (0..7)
            .map(|i| {
                let a = i as f32 / 7.0 * std::f32::consts::TAU;
                Vec2::new(a.cos(), a.sin())
            })
            .collect();
        simplify_vertices(&mut v);
        assert_eq!(v.len(), 6);
    }
}
