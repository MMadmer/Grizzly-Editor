//! The Detail Objects tool's generation (`ESceneDOTools_Generate.cpp`, `_Pick.cpp`): the slot
//! grid over the snap objects, each slot's height from the faces under it, and the objects a
//! slot scatters from the colours of the base texture.
//!
//! Every slot is worked out on its own, so the slots run on every core.

use std::collections::BTreeMap;
use std::sync::Arc;

use glam::{Vec3, Vec4};
use gz_fmt_level::{DETAIL_SLOT_SIZE, DetailHeader, DetailSlot, SceneDetails};
use rayon::prelude::*;

use super::xr::{self, CrtRand, EPS_L, Frustum, Plane, i_floor};
use crate::class::ObjClass;
use crate::library::Library;
use crate::math::Aabb;
use crate::model::{ObjId, SceneState};

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

    /// `GetColor`.
    #[must_use]
    pub fn color(&self, u: i32, v: i32) -> Option<u32> {
        if !self.valid() || u >= self.w as i32 || v >= self.h as i32 || u < 0 || v < 0 {
            return None;
        }
        self.data.get(v as usize * self.w as usize + u as usize).copied()
    }

    /// `GetPixelUFromX`.
    fn pixel_u(&self, x: f32, b: &Aabb) -> i32 {
        let u = (x - b.min.x) / (b.max.x - b.min.x);
        i_floor(u * (self.w - 1) as f32 + 0.5).max(0)
    }

    /// `GetPixelVFromZ`.
    fn pixel_v(&self, z: f32, b: &Aabb) -> i32 {
        let v = 1.0 - (z - b.min.z) / (b.max.z - b.min.z);
        i_floor(v * (self.h - 1) as f32 + 0.5).max(0)
    }
}

/// `Fcolor::set(u32)`.
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

/// The snap objects of the tool, by name.
#[must_use]
pub fn snap_ids(state: &SceneState, d: &SceneDetails) -> Vec<ObjId> {
    d.snap_objects
        .iter()
        .filter_map(|n| state.find(ObjClass::SceneObject, &n.to_lowercase()).map(|o| o.id))
        .collect()
}

/// `Scene->GetBox(bb, list)`: the snap objects' world boxes merged.
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

/// `UpdateHeader`: the slot grid over the box.
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

/// `GetSlotRect`: x1, z1, x2, z2.
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

/// The faces of the snap objects meeting a box, object by object.
fn faces_in(state: &SceneState, lib: &Library, ids: &[ObjId], b: &Aabb) -> Vec<[Vec3; 3]> {
    super::wallmark::box_triangles(state, lib, ids, b)
}

/// `UpdateSlotBBox`: the height of the faces under a slot, a little over and under them; an
/// empty slot where there are none.
#[must_use]
pub fn slot_box(
    state: &SceneState,
    lib: &Library,
    ids: &[ObjId],
    bbox: &Aabb,
    h: &DetailHeader,
    sx: u32,
    sz: u32,
) -> DetailSlot {
    let r = slot_rect(h, sx, sz);
    let mut b = Aabb::new(Vec3::new(r[0], bbox.min.y, r[1]), Vec3::new(r[2], bbox.max.y, r[3]));
    let faces = faces_in(state, lib, ids, &b);
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
    // oracle: a slot whose faces all fall outside keeps the C++ allocation's garbage; empty here
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

/// `SIndexDist`.
#[derive(Clone, Copy, Debug, Default)]
struct IndexDist {
    index: u32,
    dist: f32,
    dens: [f32; 4],
    cnt: [f32; 4],
}

/// The colour index map the slots are filled from: colour -> object indices (`m_ColorIndices`,
/// a map, so in the order of the colours).
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

/// `FindClosestIndex`.
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

/// `CalcClosestCount`.
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

/// `UpdateSlotObjects`: the objects of a slot from the colours of its part of the base texture,
/// their densities per corner. `seed` is the `srand(time(NULL))` the C++ gives every slot.
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
    // `GetSlotTCRect`: v runs against z
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
    // oracle: part 3's last row is worked out from dx, not dy
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
        // the C++ asserts here; the slot stays empty
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

/// Why the details could not be made: the C++ messages.
pub mod msg {
    pub const SNAP_EMPTY: &str = "Snap list empty!";
    pub const BASE_EMPTY: &str = "Base texture empty!";
    pub const BASE_INVALID: &str = "Invalid base texture!";
    pub const OBJECTS_EMPTY: &str = "Object list empty!";
}

/// `Initialize` / `Reinitialize`: the header and every slot's height again, then (with objects)
/// every slot's objects.
pub fn initialize(
    state: &SceneState,
    lib: &Library,
    d: &SceneDetails,
    inp: &DetailInputs<'_>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> Result<SceneDetails, &'static str> {
    let ids = snap_ids(state, d);
    if d.snap_objects.is_empty() {
        return Err(msg::SNAP_EMPTY);
    }
    if !inp.base.valid() {
        return Err(msg::BASE_EMPTY);
    }
    let Some(bbox) = objects_box(state, lib, &ids) else { return Err(msg::SNAP_EMPTY) };
    let header = header_for(&bbox, &d.header);
    let total = header.size_x as usize * header.size_z as usize;
    let done = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<DetailSlot> = (0..total)
        .into_par_iter()
        .map(|i| {
            let (x, z) = ((i % header.size_x as usize) as u32, (i / header.size_x as usize) as u32);
            let s = slot_box(state, lib, &ids, &bbox, &header, x, z);
            let n = done.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n.is_multiple_of(1024) {
                progress(n, total * 2);
            }
            s
        })
        .collect();
    let mut out = SceneDetails { header, slots: slots.into(), bbox, ..d.clone() };
    if !out.indices.objects.is_empty() {
        out.slots = objects_for(&out, None, inp, &|n, t| progress(total + n, total + t)).into();
    }
    Ok(out)
}

/// `UpdateObjects(bUpdateTex, bUpdateSelectedOnly)`: the objects of every slot, or of the
/// selected ones, again.
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

/// `InvalidateSlots`: no slot names an object.
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

/// `ClearSlots`.
#[must_use]
pub fn clear_slots(d: &SceneDetails) -> SceneDetails {
    SceneDetails { header: DetailHeader::default(), slots: Arc::from([]), ..d.clone() }
}

/// A slot's box (`RaySelect` / `FrustumSelect` / the boxes drawn).
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

/// `RaySelect`'s pick: the nearest slot box the ray meets.
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

/// `FrustumSelect`'s test over the slot boxes.
#[must_use]
pub fn slots_in_frustum(d: &SceneDetails, planes: &[Vec4]) -> Vec<usize> {
    (0..d.slots.len())
        .into_par_iter()
        .filter(|&i| slot_aabb(d, i).is_some_and(|b| crate::objtools::geom::box_in_frustum(planes, &b)))
        .collect()
}

/// The slot selection resized to the slots (`m_Selected.resize`: what was there stays).
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
}
