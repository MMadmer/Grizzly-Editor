//! The detail objects - grass, small stones - as the game keeps them: a grid of 2 m slots over
//! the ground, each slot holding up to four detail meshes with a density per corner. They are
//! painted with a brush ([`paint`]: a mesh at a density, or erased) on the ground the slot grid
//! lies over ([`grid_over`]); a colour-index picture can be imported instead ([`initialize`]:
//! each colour of the picture names the meshes it scatters).
//!
//! Every slot is worked out on its own, so the slots run on every core.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Vec3, Vec4};
use gz_fmt_level::{DETAIL_SLOT_SIZE, DetailHeader, DetailSlot, SceneDetails};
use rayon::prelude::*;

use super::xr::{self, CrtRand, EPS_L, Frustum, Plane, i_floor};
use crate::library::{Library, LibraryObject};
use crate::math::{Aabb, Mat4, xform_box};
use crate::model::{ObjData, ObjId, SceneState};

const HALF: f32 = DETAIL_SLOT_SIZE * 0.5;
/// `EPS_L_VAR`.
const EPS_L_VAR: f32 = 0.001_234_5;

/// The base texture's pixels: `0xAARRGGBB`, rows top down.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BaseImage {
    pub w: u32,
    pub h: u32,
    pub data: Vec<u32>,
}

impl BaseImage {
    /// From RGBA8 rows.
    #[must_use]
    pub fn from_rgba(w: u32, h: u32, rgba: &[u8]) -> Self {
        let data = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| {
                (u32::from(p[3]) << 24) | (u32::from(p[0]) << 16) | (u32::from(p[1]) << 8) | u32::from(p[2])
            })
            .collect();
        Self { w, h, data }
    }

    /// `Valid`.
    #[must_use]
    pub fn valid(&self) -> bool {
        self.w > 0 && self.h > 0 && !self.data.is_empty()
    }

    /// The colour of a slot.
    #[must_use]
    pub fn color(&self, u: i32, v: i32) -> Option<u32> {
        if !self.valid() || u >= self.w as i32 || v >= self.h as i32 || u < 0 || v < 0 {
            return None;
        }
        self.data.get(v as usize * self.w as usize + u as usize).copied()
    }

    /// The pixel column of an X position.
    fn pixel_u(&self, x: f32, b: &Aabb) -> i32 {
        let u = (x - b.min.x) / (b.max.x - b.min.x);
        i_floor(u * (self.w - 1) as f32 + 0.5).max(0)
    }

    /// The pixel row of a Z position.
    fn pixel_v(&self, z: f32, b: &Aabb) -> i32 {
        let v = 1.0 - (z - b.min.z) / (b.max.z - b.min.z);
        i_floor(v * (self.h - 1) as f32 + 0.5).max(0)
    }
}

/// Sets a colour from a `u32`.
fn fcolor(c: u32) -> [f32; 4] {
    let f = 1.0f32 / 255.0;
    [
        f * ((c >> 16) & 0xff) as f32,
        f * ((c >> 8) & 0xff) as f32,
        f * (c & 0xff) as f32,
        f * ((c >> 24) & 0xff) as f32,
    ]
}

fn color_dist(c: &[f32; 4], src: &[f32; 4]) -> f32 {
    let inv_a = 1.0 - c[3];
    inv_a
        + ((c[0] - src[0]) * (c[0] - src[0])
            + (c[1] - src[1]) * (c[1] - src[1])
            + (c[2] - src[2]) * (c[2] - src[2]))
            .sqrt()
}

/// The snap objects' world boxes merged.
#[must_use]
pub fn objects_box(state: &SceneState, lib: &Library, ids: &[ObjId]) -> Option<Aabb> {
    let local = |r: &str| lib.cached(r).map(|o| o.bounds);
    let mut b = Aabb::INVALID;
    let mut any = false;
    for id in ids {
        let Some(o) = state.get(*id) else { continue };
        if let Some(ob) = crate::objects::world_box(state, o, &local) {
            b.merge(&ob);
            any = true;
        }
    }
    any.then_some(b)
}

/// The slot grid over the box.
#[must_use]
pub fn header_for(bbox: &Aabb, old: &DetailHeader) -> DetailHeader {
    let mn_x = i_floor(bbox.min.x / DETAIL_SLOT_SIZE);
    let mn_z = i_floor(bbox.min.z / DETAIL_SLOT_SIZE);
    let mx_x = i_floor(bbox.max.x / DETAIL_SLOT_SIZE) + 1;
    let mx_z = i_floor(bbox.max.z / DETAIL_SLOT_SIZE) + 1;
    DetailHeader {
        offs_x: -mn_x,
        offs_z: -mn_z,
        size_x: (mx_x - mn_x).max(0) as u32,
        size_z: (mx_z - mn_z).max(0) as u32,
        ..*old
    }
}

/// X1, z1, x2, z2.
fn slot_rect(h: &DetailHeader, sx: u32, sz: u32) -> [f32; 4] {
    let x = h.from_slot_x(sx as i32);
    let z = h.from_slot_z(sz as i32);
    [x - HALF + EPS_L, z - HALF + EPS_L, x + HALF - EPS_L, z + HALF - EPS_L]
}

fn empty_slot() -> DetailSlot {
    let mut s = DetailSlot::default();
    for p in 0..4 {
        s.set_id(p, DetailSlot::ID_EMPTY);
    }
    s
}

/// The side of a cell of [`Ground`]'s buckets, in metres.
const GROUND_CELL: f32 = 32.0;

/// The ground as the slots ask it: each object's library object and transforms taken once, and
/// the objects bucketed over the ground plane, so that a slot meets only the objects over it. A
/// slot asking every object of a big level, and the library's lock for each, took minutes.
struct Ground {
    items: Vec<GroundItem>,
    origin: (f32, f32),
    nx: usize,
    nz: usize,
    cells: Vec<Vec<u32>>,
}

struct GroundItem {
    lo: Arc<LibraryObject>,
    world: Mat4,
    inv: Mat4,
    bounds: Aabb,
}

impl Ground {
    fn new(state: &SceneState, lib: &Library, ids: &[ObjId], bbox: &Aabb) -> Self {
        let mut items = Vec::with_capacity(ids.len());
        for id in ids {
            let Some(o) = state.get(*id) else { continue };
            let ObjData::SceneObject(d) = &o.data else { continue };
            let Some(lo) = lib.cached(&d.reference) else { continue };
            if lo.bvh.is_none() {
                continue;
            }
            let world = o.world();
            let inv = Mat4::invert_43(&world);
            let mut bounds = xform_box(&lo.bounds, &world);
            bounds.grow(EPS_L);
            items.push(GroundItem { lo, world, inv, bounds });
        }
        let span = |a: f32, b: f32| (((b - a) / GROUND_CELL).floor().max(0.0) as usize + 1).min(4096);
        let (nx, nz) = (span(bbox.min.x, bbox.max.x), span(bbox.min.z, bbox.max.z));
        let mut g =
            Self { items, origin: (bbox.min.x, bbox.min.z), nx, nz, cells: vec![Vec::new(); nx * nz] };
        for i in 0..g.items.len() {
            let b = g.items[i].bounds;
            let ((x0, z0), (x1, z1)) = (g.cell(b.min.x, b.min.z), g.cell(b.max.x, b.max.z));
            for z in z0..=z1 {
                for x in x0..=x1 {
                    g.cells[z * nx + x].push(i as u32);
                }
            }
        }
        g
    }

    fn cell(&self, x: f32, z: f32) -> (usize, usize) {
        let at = |v: f32, o: f32, n: usize| (((v - o) / GROUND_CELL).floor().max(0.0) as usize).min(n - 1);
        (at(x, self.origin.0, self.nx), at(z, self.origin.1, self.nz))
    }

    /// The faces meeting a box, object by object in the ground's order (as
    /// `wallmark::box_triangles` finds them).
    fn faces(&self, b: &Aabb) -> Vec<[Vec3; 3]> {
        let ((x0, z0), (x1, z1)) = (self.cell(b.min.x, b.min.z), self.cell(b.max.x, b.max.z));
        let mut near: Vec<u32> = Vec::new();
        for z in z0..=z1 {
            for x in x0..=x1 {
                near.extend_from_slice(&self.cells[z * self.nx + x]);
            }
        }
        near.sort_unstable();
        near.dedup();
        let (c, e) = (b.center(), (b.max - b.min) * 0.5);
        let mut out = Vec::new();
        let mut found = Vec::new();
        for i in near {
            let it = &self.items[i as usize];
            if it.bounds.min.cmpgt(b.max).any() || it.bounds.max.cmplt(b.min).any() {
                continue;
            }
            let Some(bvh) = &it.lo.bvh else { continue };
            found.clear();
            bvh.query_box(&xform_box(b, &it.inv), &mut found);
            found.sort_unstable();
            for &t in &found {
                let Some(tri) = it.lo.tri(t) else { continue };
                let w = [
                    it.world.transform_tiny(tri[0]),
                    it.world.transform_tiny(tri[1]),
                    it.world.transform_tiny(tri[2]),
                ];
                if xr::tri_box(c, e, &w, true) {
                    out.push(w);
                }
            }
        }
        out
    }
}

/// The height of the faces under a slot, a little over and under them; an
/// empty slot where there are none.
fn slot_box(ground: &Ground, bbox: &Aabb, h: &DetailHeader, sx: u32, sz: u32) -> DetailSlot {
    let r = slot_rect(h, sx, sz);
    let mut b = Aabb::new(Vec3::new(r[0], bbox.min.y, r[1]), Vec3::new(r[2], bbox.max.y, r[3]));
    let faces = ground.faces(&b);
    if faces.is_empty() {
        return empty_slot();
    }
    b.grow(EPS_L_VAR);
    let planes = [
        Plane::build(b.min, Vec3::new(-1.0, 0.0, 0.0)),
        Plane::build(b.min, Vec3::new(0.0, 0.0, -1.0)),
        Plane::build(b.max, Vec3::new(1.0, 0.0, 0.0)),
        Plane::build(b.max, Vec3::new(0.0, 0.0, 1.0)),
    ];
    let frustum = Frustum::from_planes(&planes);
    let mut y_min = f32::MAX;
    let mut y_max = -f32::MAX;
    // a slot whose faces all fall outside stays empty
    let mut slot = empty_slot();
    for f in &faces {
        let Some(poly) = frustum.clip_poly(f) else { continue };
        for p in &poly {
            let hh = p.y;
            if hh > y_max {
                y_max = hh + 0.03;
            }
            if hh < y_min {
                y_min = hh - 0.03;
            }
        }
        slot.set_heights(y_min, y_max - y_min);
        for part in 0..4 {
            slot.set_id(part, DetailSlot::ID_EMPTY);
        }
    }
    slot
}

#[derive(Clone, Copy, Debug, Default)]
struct IndexDist {
    index: u32,
    dist: f32,
    dens: [f32; 4],
    cnt: [f32; 4],
}

/// The colour index map the slots are filled from: colour -> object indices (a map, so in the
/// order of the colours).
#[must_use]
pub fn color_map(d: &SceneDetails) -> BTreeMap<u32, Vec<u8>> {
    let mut out: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
    for c in &d.indices.colors {
        let e = out.entry(c.color).or_default();
        for name in &c.objects {
            if let Some(i) = d.indices.objects.iter().position(|o| o.reference.eq_ignore_ascii_case(name)) {
                e.push(i as u8);
            }
        }
    }
    out
}

/// The closest index.
fn find_closest(c: &[f32; 4], map: &BTreeMap<u32, Vec<u8>>, best: &mut Vec<IndexDist>) {
    let mut dist = f32::MAX;
    let mut index = None;
    for &k in map.keys() {
        let d = color_dist(c, &fcolor(k));
        if d < dist {
            dist = d;
            index = Some(k);
        }
    }
    let Some(index) = index else { return };
    if best.len() < 4 {
        if let Some(b) = best.iter_mut().find(|b| b.index == index) {
            if dist < b.dist {
                b.dist = dist;
            }
        } else {
            best.push(IndexDist { index, dist, ..IndexDist::default() });
        }
    } else {
        let mut i: Option<usize> = None;
        let mut dd = f32::MAX;
        let mut found = false;
        for (k, b) in best.iter_mut().enumerate().take(4) {
            let d = dist - b.dist;
            if d < 0.0 && d < dd {
                i = Some(k);
                dd = d;
            }
            if b.index == index {
                if dist < b.dist {
                    b.dist = dist;
                }
                found = true;
                break;
            }
        }
        if !found && let Some(i) = i {
            best[i].dist = dist;
            best[i].index = index;
        }
    }
}

/// The count of the closest indices.
fn closest_count(part: usize, c: &[f32; 4], best: &mut [IndexDist]) {
    let mut dist = f32::MAX;
    let mut idx = None;
    for (k, b) in best.iter().enumerate() {
        let d = color_dist(c, &fcolor(b.index));
        if d < dist {
            dist = d;
            idx = Some(k);
        }
    }
    if let Some(k) = idx
        && let Some(b) = best.get_mut(k)
    {
        b.cnt[part] += 1.0;
    }
}

/// The objects of a slot from the colours of its part of the base texture, their densities per
/// corner. `seed` is the random seed of the scatter.
#[allow(clippy::too_many_arguments)]
pub fn slot_objects(
    slot: &mut DetailSlot,
    h: &DetailHeader,
    bbox: &Aabb,
    sx: u32,
    sz: u32,
    base: &BaseImage,
    map: &BTreeMap<u32, Vec<u8>>,
    density: &[f32],
    seed: u32,
) {
    let rr = slot_rect(h, sx, sz);
    // v runs against z
    let (x1, x2) = (base.pixel_u(rr[0], bbox), base.pixel_u(rr[2], bbox));
    let (y2, y1) = (base.pixel_v(rr[1], bbox), base.pixel_v(rr[3], bbox));
    let mut best: Vec<IndexDist> = Vec::with_capacity(4);
    for v in y1..=y2 {
        for u in x1..=x2 {
            if let Some(clr) = base.color(u, v) {
                find_closest(&fcolor(clr), map, &mut best);
            }
        }
    }
    // MSVC's sort of four is an insertion sort: stable
    best.sort_by(|a, b| a.dist.partial_cmp(&b.dist).unwrap_or(std::cmp::Ordering::Equal));
    let dx = (x2 - x1) as f32 / 2.0;
    let dy = (y2 - y1) as f32 / 2.0;
    // 2 3
    // 0 1
    // part 3's last row is worked out from dx, not dy: the slots the game reads are laid out so
    let parts = [
        [x1, i_floor(y1 as f32 + dy + 0.501), i_floor(x1 as f32 + dx + 0.499), y2],
        [i_floor(x1 as f32 + dx + 0.501), i_floor(y1 as f32 + dy + 0.501), x2, y2],
        [x1, y1, i_floor(x1 as f32 + dx + 0.499), i_floor(y1 as f32 + dy + 0.499)],
        [i_floor(x1 as f32 + dx + 0.501), y1, x2, i_floor(y1 as f32 + dx + 0.499)],
    ];
    for (part, p) in parts.iter().enumerate() {
        let mut alpha = 0.0f32;
        let mut cnt = 0i32;
        for v in p[1]..=p[3] {
            for u in p[0]..=p[2] {
                if let Some(clr) = base.color(u, v) {
                    let c = fcolor(clr);
                    closest_count(part, &c, &mut best);
                    alpha += c[3];
                    cnt += 1;
                }
            }
        }
        alpha /= if cnt != 0 { cnt as f32 } else { 1.0 };
        alpha *= 0.5;
        for b in &mut best {
            b.dens[part] = if cnt != 0 { (b.cnt[part] * alpha) / cnt as f32 } else { 0.0 };
        }
    }
    if best.is_empty() {
        // nothing to scatter: the slot stays empty
        for k in 0..4 {
            slot.set_id(k, DetailSlot::ID_EMPTY);
        }
        return;
    }
    let size_of = |b: &IndexDist| map.get(&b.index).map_or(0, Vec::len);
    let o_cnt: usize = best.iter().map(size_of).sum();
    if o_cnt > best.len() {
        let mut id: i32 = -1;
        while best.len() < 4 {
            loop {
                id += 1;
                if id > 3 {
                    id = 0;
                }
                if best.get(id as usize).is_some_and(|b| size_of(b) > 1) {
                    break;
                }
            }
            let Some(copy) = best.get(id as usize).copied() else { break };
            best.push(copy);
            if best.len() == o_cnt {
                break;
            }
        }
    }
    let mut rng = CrtRand::seeded(seed);
    for (k, b) in best.iter().enumerate() {
        let Some(objs) = map.get(&b.index) else { continue };
        let mut elem: Vec<u8> = (0..objs.len() as u8).collect();
        rng.shuffle(&mut elem);
        for &e in &elem {
            let id = objs.get(usize::from(e)).copied().unwrap_or(DetailSlot::ID_EMPTY);
            slot.set_id(k, id);
            if (0..k).all(|j| slot.id(j) != slot.id(k)) {
                break;
            }
        }
        slot.set_editor_colors();
        let f = density.get(usize::from(slot.id(k))).copied().unwrap_or(1.0);
        for corner in 0..4 {
            let v = i_floor(b.dens[corner] * f * 15.0 + 0.5);
            slot.set_density(k, corner, (v & 0xf) as u8);
        }
    }
    for k in best.len()..4 {
        slot.set_id(k, DetailSlot::ID_EMPTY);
    }
}

/// What a regeneration needs besides the scene.
pub struct DetailInputs<'a> {
    pub base: &'a BaseImage,
    pub seed: u32,
}

/// Why the details could not be made.
pub mod msg {
    pub const GROUND_EMPTY: &str = "The scene has no static objects for the details to lie on.";
    pub const BASE_EMPTY: &str = "Base texture empty!";
    pub const BASE_INVALID: &str = "Invalid base texture!";
    pub const OBJECTS_EMPTY: &str = "Object list empty!";
}

/// The slot grid over the ground objects `ids`: the header over their box and every slot's
/// height from the faces under it; the slots name no object. What painting starts from.
pub fn grid_over(
    state: &SceneState,
    lib: &Library,
    ids: &[ObjId],
    d: &SceneDetails,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<SceneDetails, &'static str> {
    let Some(bbox) = objects_box(state, lib, ids) else { return Err(msg::GROUND_EMPTY) };
    let header = header_for(&bbox, &d.header);
    let total = header.size_x as usize * header.size_z as usize;
    let ground = Ground::new(state, lib, ids, &bbox);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<DetailSlot> = (0..total)
        .into_par_iter()
        .map(|i| {
            let (x, z) = ((i % header.size_x as usize) as u32, (i / header.size_x as usize) as u32);
            let s = slot_box(&ground, &bbox, &header, x, z);
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n.is_multiple_of(1024) {
                progress(n, total);
            }
            s
        })
        .collect();
    Ok(SceneDetails { header, slots: slots.into(), bbox, ..d.clone() })
}

/// The import of a colour-index picture: the slot grid over the ground again, then every slot's
/// objects from the colours of its part of the picture (each colour names the detail meshes it
/// scatters).
pub fn initialize(
    state: &SceneState,
    lib: &Library,
    ids: &[ObjId],
    d: &SceneDetails,
    inp: &DetailInputs<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<SceneDetails, &'static str> {
    if !inp.base.valid() {
        return Err(msg::BASE_EMPTY);
    }
    let mut out = grid_over(state, lib, ids, d, &|n, t| progress(n, t * 2))?;
    let total = out.slots.len();
    if !out.indices.objects.is_empty() {
        out.slots = objects_for(&out, None, inp, &|n, t| progress(total + n, total + t)).into();
    }
    Ok(out)
}

/// One stroke of the detail brush: a detail mesh painted at a density, or taken away.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stroke {
    /// The detail mesh (an index into the layer's mesh list); `None` erases every mesh.
    pub object: Option<u8>,
    pub center: Vec3,
    /// Metres, in the ground plane.
    pub radius: f32,
    /// 0..1 of the densest a slot corner holds; ignored when erasing.
    pub density: f32,
    pub erase: bool,
}

/// The centres of a slot's four corners (`2 3` over `0 1`), in the ground plane.
fn corners(h: &DetailHeader, i: usize) -> [(f32, f32); 4] {
    let sx = h.size_x.max(1) as usize;
    let (x, z) = ((i % sx) as i32, (i / sx) as i32);
    let (cx, cz) = (h.from_slot_x(x), h.from_slot_z(z));
    let q = HALF * 0.5;
    [(cx - q, cz - q), (cx + q, cz - q), (cx - q, cz + q), (cx + q, cz + q)]
}

/// The strokes laid on the slots: a painted mesh takes the part it has in a slot, an empty part,
/// or the part of the thinnest mesh there; each corner inside the brush gets the density (times
/// the mesh's own density factor). Erasing sets the corners to nothing, and a part left with
/// nothing names no mesh. A slot with no ground under it is left alone. Answers the layer and
/// how many slots changed.
#[must_use]
pub fn paint(d: &SceneDetails, strokes: &[Stroke]) -> (SceneDetails, usize) {
    let h = d.header;
    let mut slots: Vec<DetailSlot> = d.slots.to_vec();
    let mut changed = vec![false; slots.len()];
    for st in strokes {
        let reach = st.radius + HALF * std::f32::consts::SQRT_2;
        let r2 = st.radius * st.radius;
        let value = |obj: u8| -> u8 {
            let f = d.indices.objects.get(usize::from(obj)).map_or(1.0, |o| o.density_factor);
            (st.density.clamp(0.0, 1.0) * f * 15.0).round().clamp(0.0, 15.0) as u8
        };
        for (i, slot) in slots.iter_mut().enumerate() {
            let c = corners(&h, i);
            let (mx, mz) = ((c[0].0 + c[3].0) * 0.5, (c[0].1 + c[3].1) * 0.5);
            if (mx - st.center.x).powi(2) + (mz - st.center.z).powi(2) > reach * reach {
                continue;
            }
            let hit: Vec<usize> = (0..4)
                .filter(|k| (c[*k].0 - st.center.x).powi(2) + (c[*k].1 - st.center.z).powi(2) <= r2)
                .collect();
            if hit.is_empty() || slot.height() <= 0.0 {
                continue;
            }
            let before = *slot;
            if st.erase {
                for part in 0..4 {
                    let id = slot.id(part);
                    if id == DetailSlot::ID_EMPTY || st.object.is_some_and(|o| o != id) {
                        continue;
                    }
                    for k in &hit {
                        slot.set_density(part, *k, 0);
                    }
                    if (0..4).all(|k| slot.density(part, k) == 0) {
                        slot.set_id(part, DetailSlot::ID_EMPTY);
                    }
                }
            } else if let Some(obj) = st.object {
                let total =
                    |s: &DetailSlot, p: usize| (0..4).map(|k| u32::from(s.density(p, k))).sum::<u32>();
                let part = (0..4)
                    .find(|p| slot.id(*p) == obj)
                    .or_else(|| (0..4).find(|p| slot.id(*p) == DetailSlot::ID_EMPTY))
                    .unwrap_or_else(|| (0..4).min_by_key(|p| total(slot, *p)).unwrap_or(0));
                if slot.id(part) != obj {
                    slot.set_id(part, obj);
                    for k in 0..4 {
                        slot.set_density(part, k, 0);
                    }
                }
                for k in &hit {
                    slot.set_density(part, *k, value(obj));
                }
                slot.set_editor_colors();
            }
            if *slot != before {
                changed[i] = true;
            }
        }
    }
    let n = changed.iter().filter(|c| **c).count();
    (SceneDetails { slots: slots.into(), ..d.clone() }, n)
}

/// What a slot holds, part by part: the mesh and its density in each corner.
#[must_use]
pub fn coverage(slot: &DetailSlot) -> Vec<(u8, [u8; 4])> {
    (0..4)
        .filter(|p| slot.id(*p) != DetailSlot::ID_EMPTY)
        .map(|p| (slot.id(p), [0, 1, 2, 3].map(|k| slot.density(p, k))))
        .collect()
}

/// The layer without the mesh `object`: its parts go, and the parts of the meshes after it
/// name them by their new places.
#[must_use]
pub fn remove_mesh(d: &SceneDetails, object: u8) -> SceneDetails {
    let mut out = d.clone();
    if usize::from(object) < out.indices.objects.len() {
        let gone = out.indices.objects.remove(usize::from(object)).reference;
        for c in &mut out.indices.colors {
            c.objects.retain(|x| !x.eq_ignore_ascii_case(&gone));
        }
    }
    let slots: Vec<DetailSlot> = d
        .slots
        .iter()
        .map(|s| {
            let mut s = *s;
            for p in 0..4 {
                let id = s.id(p);
                if id == DetailSlot::ID_EMPTY {
                    continue;
                }
                if id == object {
                    s.set_id(p, DetailSlot::ID_EMPTY);
                    for k in 0..4 {
                        s.set_density(p, k, 0);
                    }
                } else if id > object {
                    s.set_id(p, id - 1);
                }
            }
            s
        })
        .collect();
    out.slots = slots.into();
    out
}

/// The objects of every slot, or of the selected ones, made again.
pub fn update_objects(
    d: &SceneDetails,
    selected: Option<&[bool]>,
    inp: &DetailInputs<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<SceneDetails, &'static str> {
    if !inp.base.valid() {
        return Err(msg::BASE_INVALID);
    }
    if d.indices.objects.is_empty() {
        return Err(msg::OBJECTS_EMPTY);
    }
    let slots = objects_for(d, selected, inp, progress);
    Ok(SceneDetails { slots: slots.into(), ..d.clone() })
}

fn objects_for(
    d: &SceneDetails,
    selected: Option<&[bool]>,
    inp: &DetailInputs<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Vec<DetailSlot> {
    let map = color_map(d);
    let density: Vec<f32> = d.indices.objects.iter().map(|o| o.density_factor).collect();
    let h = d.header;
    let total = d.slots.len();
    let done = std::sync::atomic::AtomicUsize::new(0);
    d.slots
        .par_iter()
        .enumerate()
        .map(|(i, s)| {
            let mut s = *s;
            if selected.is_none_or(|sel| sel.get(i).copied().unwrap_or(false)) && h.size_x > 0 {
                let (x, z) = ((i % h.size_x as usize) as u32, (i / h.size_x as usize) as u32);
                slot_objects(&mut s, &h, &d.bbox, x, z, inp.base, &map, &density, inp.seed);
            }
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n.is_multiple_of(1024) {
                progress(n, total);
            }
            s
        })
        .collect()
}

/// No slot names an object.
#[must_use]
pub fn invalidate_slots(d: &SceneDetails) -> SceneDetails {
    let slots: Vec<DetailSlot> = d
        .slots
        .iter()
        .map(|s| {
            let mut s = *s;
            for p in 0..4 {
                s.set_id(p, DetailSlot::ID_EMPTY);
            }
            s
        })
        .collect();
    SceneDetails { slots: slots.into(), ..d.clone() }
}

/// Clears the slots.
#[must_use]
pub fn clear_slots(d: &SceneDetails) -> SceneDetails {
    SceneDetails { header: DetailHeader::default(), slots: Arc::from([]), ..d.clone() }
}

/// A slot's box (for picks and the boxes drawn).
#[must_use]
pub fn slot_aabb(d: &SceneDetails, i: usize) -> Option<Aabb> {
    let h = &d.header;
    let s = d.slots.get(i)?;
    let (x, z) = ((i % h.size_x.max(1) as usize) as i32, (i / h.size_x.max(1) as usize) as i32);
    let (fx, fz) = (h.from_slot_x(x), h.from_slot_z(z));
    Some(Aabb::new(
        Vec3::new(fx - HALF, s.base_height(), fz - HALF),
        Vec3::new(fx + HALF, s.base_height() + s.height(), fz + HALF),
    ))
}

/// The nearest slot box the ray meets.
#[must_use]
pub fn pick_slot(d: &SceneDetails, start: Vec3, dir: Vec3, dist: f32) -> Option<usize> {
    let mut best = None;
    let mut reach = dist;
    for i in 0..d.slots.len() {
        let Some(b) = slot_aabb(d, i) else { continue };
        if let Some((p, _)) = xr::box_pick2(b.min, b.max, start, dir) {
            let r = (p - start).length();
            if r < reach {
                reach = r;
                best = Some(i);
            }
        }
    }
    best
}

/// The frustum test over the slot boxes.
#[must_use]
pub fn slots_in_frustum(d: &SceneDetails, planes: &[Vec4]) -> Vec<usize> {
    (0..d.slots.len())
        .into_par_iter()
        .filter(|&i| slot_aabb(d, i).is_some_and(|b| crate::objtools::geom::box_in_frustum(planes, &b)))
        .collect()
}

/// The slot selection resized to the slots (what was there stays).
#[must_use]
pub fn resized(sel: &[bool], n: usize) -> Arc<[bool]> {
    let mut v: Vec<bool> = sel.iter().copied().take(n).collect();
    v.resize(n, false);
    v.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_fmt_level::{ColorIndex, DetailObjectRef};

    #[test]
    fn a_slot_from_a_two_colour_base_texture() {
        // left half red, right half green; red scatters object 0, green object 1
        let (w, h) = (8u32, 8u32);
        let mut rgba = Vec::new();
        for _y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(if x < w / 2 { &[255, 0, 0, 255] } else { &[0, 255, 0, 255] });
            }
        }
        let base = BaseImage::from_rgba(w, h, &rgba);
        let mut d = SceneDetails::default();
        d.indices.objects = vec![
            DetailObjectRef { reference: "grass_a".into(), ..DetailObjectRef::default() },
            DetailObjectRef { reference: "grass_b".into(), ..DetailObjectRef::default() },
        ];
        d.indices.colors = vec![
            ColorIndex { color: 0xffff_0000, objects: vec!["grass_a".into()] },
            ColorIndex { color: 0xff00_ff00, objects: vec!["grass_b".into()] },
        ];
        // one slot covering the whole texture
        d.bbox = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(2.0, 1.0, 2.0));
        d.header = header_for(&d.bbox, &DetailHeader::default());
        let map = color_map(&d);
        let mut slot = empty_slot();
        slot_objects(&mut slot, &d.header, &d.bbox, 0, 0, &base, &map, &[1.0, 1.0], 1);
        let ids = [slot.id(0), slot.id(1)];
        assert!(ids.contains(&0) && ids.contains(&1), "{ids:?}");
        assert_eq!(slot.id(2), DetailSlot::ID_EMPTY);
        // red sits on the left corners (0 and 2), green on the right ones (1 and 3)
        let red = if slot.id(0) == 0 { 0 } else { 1 };
        assert!(slot.density(red, 0) > 0 && slot.density(red, 1) == 0);
        assert_eq!(slot.colors()[2..], [0, 0, 0]);
    }

    /// A 4 x 4 grid of 2 m slots over (0..8, 0..8), every slot on flat ground.
    fn grid() -> SceneDetails {
        let mut d = SceneDetails::default();
        d.indices.objects = vec![
            DetailObjectRef { reference: "grass_a".into(), ..DetailObjectRef::default() },
            DetailObjectRef {
                reference: "grass_b".into(),
                density_factor: 0.5,
                ..DetailObjectRef::default()
            },
        ];
        d.bbox = Aabb::new(Vec3::new(0.0, 0.0, 0.0), Vec3::new(7.9, 1.0, 7.9));
        d.header = header_for(&d.bbox, &DetailHeader::default());
        let mut slot = empty_slot();
        slot.set_heights(0.0, 0.5);
        d.slots = vec![slot; (d.header.size_x * d.header.size_z) as usize].into();
        d
    }

    #[test]
    fn painted_details_are_the_games_slots_and_read_back() {
        let d = grid();
        assert_eq!((d.header.size_x, d.header.size_z), (4, 4));
        // a mesh painted round the middle of the slot at (1, 1)
        let at = Vec3::new(d.header.from_slot_x(1), 0.0, d.header.from_slot_z(1));
        let stroke = Stroke { object: Some(0), center: at, radius: 1.0, density: 1.0, erase: false };
        let (painted, n) = paint(&d, &[stroke]);
        assert_eq!(n, 1, "only the slot under the brush");
        let i = d.header.slot_index(1, 1).unwrap();
        assert_eq!(coverage(&painted.slots[i]), vec![(0, [15, 15, 15, 15])]);
        // a second mesh shares the slot; its density factor halves what it gets
        let (both, _) = paint(&painted, &[Stroke { object: Some(1), ..stroke }]);
        assert_eq!(coverage(&both.slots[i]), vec![(0, [15, 15, 15, 15]), (1, [8, 8, 8, 8])]);
        // erasing one mesh leaves the other; erasing everything empties the slot
        let (one, _) = paint(&both, &[Stroke { object: Some(0), erase: true, ..stroke }]);
        assert_eq!(coverage(&one.slots[i]), vec![(1, [8, 8, 8, 8])]);
        let (none, _) = paint(&both, &[Stroke { object: None, erase: true, ..stroke }]);
        assert!(none.slots[i].is_empty());
        // a slot without ground under it is left alone
        let mut holes = grid();
        let mut slots = holes.slots.to_vec();
        slots[i] = empty_slot();
        holes.slots = slots.into();
        assert_eq!(paint(&holes, &[stroke]).1, 0);
        // removing a mesh moves the meshes after it down
        let gone = remove_mesh(&both, 0);
        assert_eq!(gone.indices.objects.len(), 1);
        assert_eq!(coverage(&gone.slots[i]), vec![(0, [8, 8, 8, 8])]);
        // what the build writes from the painted slots: the same meshes and densities
        let exported = both.slots[i].for_export(&[0, 1]);
        assert_eq!(coverage(&exported), coverage(&both.slots[i]));
    }
}
