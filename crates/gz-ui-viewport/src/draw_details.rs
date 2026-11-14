//! Detail objects as the detail tool draws them: the models of the slots round the
//! camera placed the way the game places them (the same
//! dither, jitter, yaw, scale and the same random seeds per slot, each dropped onto the
//! ground's surface), faded with distance, drawn instanced; the slot grid's
//! boxes within 75 m; the base texture over the ground.
//!
//! Slots decompress on the CPU pool as part of a scene pass, those of the 49 x 49 cache window
//! round the camera (`dm_cache_line`), and are kept until the window leaves them or what they
//! were made of changes (the details, the scene, the library).

use std::sync::Arc;

use glam::{Mat4, Vec3};
use gz_fmt_level::{DETAIL_SLOT_SIZE, DetailSlot, SceneDetails, tool_flags};
use gz_particles::Rng;
use gz_render::{
    Aabb, Assets, Batch, BatchInstance, Color, Frustum, Instance, InstanceFlags, MaterialDesc, MeshData,
    MeshVertex, ShapeKind, SubMesh,
};
use gz_scene::library::Library;
use gz_scene::{ObjClass, ObjId, SceneState};
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::draw::Built;

/// `dm_size`: slots each way from the camera's that the cache holds.
pub const DM_SIZE: i32 = 24;
/// `dm_fade`: where a detail has shrunk to nothing.
const DM_FADE: f32 = (2 * DM_SIZE) as f32 - 0.5;
/// `ps_r__Detail_density` when the scene does not say.
pub const DEFAULT_DENSITY: f32 = 0.3;
/// The slot boxes are drawn this far.
const BOX_REACH: f32 = 75.0;
/// xrCore's `EPS_L` and `EPS`.
const EPS_L: f32 = 0.001;
const EPS: f32 = 0.000_01;
const PI_MUL_2: f32 = 6.283_185_5;

/// `bwdithermap(2, dither)`: the 4 x 4 pattern spread over 16 x 16.
#[must_use]
pub fn dither_map() -> [[i32; 16]; 16] {
    const MAGIC: [[i32; 4]; 4] = [[0, 14, 3, 13], [11, 5, 8, 6], [12, 2, 15, 1], [7, 9, 4, 10]];
    let n = 255.0f32 / (2 - 1) as f32;
    let fact = f64::from((n - 1.0) / 16.0);
    let mut out = [[0i32; 16]; 16];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                for l in 0..4 {
                    let v = 0.5 + f64::from(MAGIC[i][j]) * fact + (f64::from(MAGIC[k][l]) / 16.0) * fact;
                    out[4 * k + i][4 * l + j] = v as i32;
                }
            }
        }
    }
    out
}

/// The four corner values across the slot.
fn interpolate(base: &[f32; 4], x: u32, y: u32, size: u32) -> f32 {
    let f = size as f32;
    let fx = x as f32 / f;
    let ifx = 1.0 - fx;
    let fy = y as f32 / f;
    let ify = 1.0 - fy;
    let c01 = base[0] * ifx + base[1] * fx;
    let c23 = base[2] * ifx + base[3] * fx;
    let c02 = base[0] * ify + base[2] * fy;
    let c13 = base[1] * ify + base[3] * fy;
    let cx = ify * c01 + fy * c23;
    let cy = ifx * c02 + fx * c13;
    (cx + cy) / 2.0
}

/// The interpolation and dither.
fn interpolate_and_dither(
    alpha255: &[f32; 4],
    x: u32,
    y: u32,
    sx: u32,
    sy: u32,
    size: u32,
    dither: &[[i32; 16]; 16],
) -> bool {
    let x = x.min(size.saturating_sub(1));
    let y = y.min(size.saturating_sub(1));
    let c = ((interpolate(alpha255, x, y, size) + 0.5).floor() as i32).clamp(0, 255);
    let row = ((y + sy) % 16) as usize;
    let col = ((x + sx) % 16) as usize;
    c > dither[col][row]
}

/// A ray-triangle test with culling: the distance along `dir` to a front-facing triangle.
fn ray_tri(c: Vec3, d: Vec3, t: &[Vec3; 3]) -> Option<f32> {
    let e1 = t[1] - t[0];
    let e2 = t[2] - t[0];
    let p = d.cross(e2);
    let det = e1.dot(p);
    if det < EPS {
        return None;
    }
    let tv = c - t[0];
    let u = tv.dot(p);
    if u < 0.0 || u > det {
        return None;
    }
    let q = tv.cross(e1);
    let v = d.dot(q);
    if v < 0.0 || u + v > det {
        return None;
    }
    Some(e2.dot(q) * (1.0 / det))
}

/// One placed detail.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Item {
    /// Into the tool's object list.
    pub object: u8,
    pub pos: Vec3,
    pub yaw: f32,
    pub scale: f32,
}

/// A decompressed slot: its details and where they are.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SlotItems {
    pub items: Vec<Item>,
    /// The box of the items' positions (none for an empty slot).
    pub bounds: Option<Aabb>,
}

/// The box of the slot at `(sx, sz)` of the world grid (`cache_Task`).
#[must_use]
pub fn slot_box(slot: &DetailSlot, sx: i32, sz: i32) -> Aabb {
    let s = DETAIL_SLOT_SIZE;
    let min = Vec3::new(sx as f32 * s, slot.base_height(), sz as f32 * s);
    let max = Vec3::new(min.x + s, slot.base_height() + slot.height(), min.z + s);
    Aabb::new(min - Vec3::splat(EPS_L), max + Vec3::splat(EPS_L))
}

/// The slot of the world grid, or none outside the header's.
fn query(d: &SceneDetails, sx: i32, sz: i32) -> Option<&DetailSlot> {
    let x = sx.checked_add(d.header.offs_x)?;
    let z = sz.checked_add(d.header.offs_z)?;
    let (x, z) = (u32::try_from(x).ok()?, u32::try_from(z).ok()?);
    d.slots.get(d.header.slot_index(x, z)?)
}

/// `cache_Decompress`: the details of a slot. `scales` are the objects' scale limits as the tool
/// holds them; `tris` the ground's triangles in the slot's box.
#[must_use]
#[allow(clippy::too_many_arguments)]
pub fn decompress(
    slot: &DetailSlot,
    sx: i32,
    sz: i32,
    scales: &[(f32, f32)],
    density: f32,
    dither: &[[i32; 16]; 16],
    tris: &[[Vec3; 3]],
) -> SlotItems {
    if slot.is_empty() || tris.is_empty() {
        return SlotItems::default();
    }
    let vis = slot_box(slot, sx, sz);
    let mut alpha255 = [[0.0f32; 4]; 4];
    for (part, a) in alpha255.iter_mut().enumerate() {
        for (corner, v) in a.iter_mut().enumerate() {
            *v = 255.0 * f32::from(slot.density(part, corner)) / 15.0;
        }
    }
    let density = if density > 0.0 { density } else { DEFAULT_DENSITY };
    let jitter = density / 1.7;
    let d_size = (DETAIL_SLOT_SIZE / density).ceil().max(1.0) as u32;
    let p_rnd = (sx.wrapping_mul(sz)) as u32;
    let seed = (0x1207_1980u32 ^ p_rnd) as i32;
    let (mut r_selection, mut r_jitter, mut r_yaw, mut r_scale) =
        (Rng::new(seed), Rng::new(seed), Rng::new(seed), Rng::new(seed));
    let mut out = SlotItems::default();
    let mut bounds = Aabb::EMPTY;
    let mut selected: Vec<usize> = Vec::with_capacity(4);
    let down = Vec3::new(0.0, -1.0, 0.0);
    for z in 0..=d_size {
        for x in 0..=d_size {
            let shift_x = r_jitter.rand_i_max(16) as u32;
            let shift_z = r_jitter.rand_i_max(16) as u32;
            selected.clear();
            for (part, a) in alpha255.iter().enumerate() {
                if slot.id(part) != DetailSlot::ID_EMPTY
                    && interpolate_and_dither(a, x, z, shift_x, shift_z, d_size, dither)
                {
                    selected.push(part);
                }
            }
            let index = match selected.as_slice() {
                [] => continue,
                [one] => *one,
                many => many.get(r_selection.rand_i_max(many.len() as i32) as usize).copied().unwrap_or(0),
            };
            let object = slot.id(index);
            let Some(&(min_scale, max_scale)) = scales.get(usize::from(object)) else { continue };
            let rx = (x as f32 / d_size as f32) * DETAIL_SLOT_SIZE + vis.min.x;
            let rz = (z as f32 / d_size as f32) * DETAIL_SLOT_SIZE + vis.min.z;
            // `Item_P.set(rx + jitter, top, rz + jitter)`: MSVC draws the arguments last first
            let jz = r_jitter.rand_fs(jitter);
            let jx = r_jitter.rand_fs(jitter);
            let mut p = Vec3::new(rx + jx, vis.max.y, rz + jz);
            let mut y = vis.min.y - 5.0;
            for t in tris {
                if let Some(range) = ray_tri(p, down, t)
                    && range >= 0.0
                {
                    y = y.max(p.y - range);
                }
            }
            if y < vis.min.y {
                continue;
            }
            p.y = y;
            let scale = r_scale.rand_f_range(min_scale * 0.5, max_scale * 0.9);
            let yaw = r_yaw.rand_f_range(0.0, PI_MUL_2);
            bounds.extend(p);
            out.items.push(Item { object, pos: p, yaw, scale });
        }
    }
    out.bounds = (!bounds.is_empty()).then_some(bounds);
    out
}

/// A rotation about Y by `yaw` at `p`, times a scale.
#[must_use]
pub fn item_transform(i: &Item, scale: f32) -> Mat4 {
    Mat4::from_translation(i.pos) * Mat4::from_rotation_y(i.yaw) * Mat4::from_scale(Vec3::splat(scale))
}

/// What made the cached slots: when it changes they are made again.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    details: usize,
    library: u64,
    /// The scene's content: the ground may have moved.
    serial: u64,
}

/// What the details pass keeps between frames: an `Editor` service. A clone shares the slots.
#[derive(Clone, Default)]
pub struct DetailCache {
    key: Option<Key>,
    slots: FxHashMap<(i32, i32), Arc<SlotItems>>,
    base: Option<String>,
    /// The objects the details lie on, with their world boxes: a slot asks only those over it.
    ground: Arc<Vec<(ObjId, gz_fmt_level::Aabb)>>,
}

const BASE_MESH: &str = "<details>/base";

/// What a details pass is given.
pub(crate) struct Pass<'a> {
    pub state: &'a SceneState,
    pub lib: &'a Library,
    pub assets: &'a Assets,
    pub frustum: &'a Frustum,
    pub eye: Vec3,
    /// `r_ssaDISCARD`: `ssaDISCARD^2 / (width * height)`.
    pub ssa_discard: f32,
}

impl DetailCache {
    fn drop_base(&mut self, assets: &Assets) {
        if self.base.take().is_some() {
            assets.remove_mesh(BASE_MESH);
        }
    }

    pub(crate) fn draw(&mut self, p: &Pass<'_>, built: &mut Built) {
        let tool = p.state.tools.part(ObjClass::DetailObjects);
        let Some(part) = p.state.bulk.details.as_ref().filter(|_| tool.visible) else {
            self.slots.clear();
            self.key = None;
            self.drop_base(p.assets);
            return;
        };
        let d = &part.model;
        if d.slots.is_empty() {
            self.drop_base(p.assets);
            return;
        }
        let key = Key { details: Arc::as_ptr(part) as usize, library: p.lib.rev().0, serial: p.state.serial };
        if self.key.as_ref() != Some(&key) {
            self.slots.clear();
            self.drop_base(p.assets);
            self.key = Some(key);
            // the details' ground: every static object, or the layer's own list
            let local = |r: &str| p.lib.cached(r).map(|o| o.bounds);
            self.ground = Arc::new(
                gz_scene::bulk::cmd::details_ground_ids(p.state, p.lib)
                    .into_iter()
                    .filter_map(|id| {
                        let o = p.state.get(id)?;
                        gz_scene::objects::world_box(p.state, o, &local).map(|b| (id, b))
                    })
                    .collect(),
            );
        }
        if d.flags & tool_flags::SLOT_BOXES_DRAW != 0 {
            slot_boxes(d, &p.state.tools.bulk.slots_selected, p, built);
        }
        let ground = self.ground.clone();
        if d.flags & tool_flags::BASE_TEXTURE_DRAW != 0 {
            let ids: Vec<ObjId> = ground.iter().map(|(id, _)| *id).collect();
            self.base_texture(d, p, &ids, built);
        }
        if d.flags & tool_flags::OBJECTS_DRAW != 0 {
            self.objects(d, p, &ground, built);
        }
    }

    /// The models of the window's slots, as one batch per detail object.
    fn objects(
        &mut self,
        d: &SceneDetails,
        p: &Pass<'_>,
        ground: &[(ObjId, gz_fmt_level::Aabb)],
        built: &mut Built,
    ) {
        let cx = (p.eye.x / DETAIL_SLOT_SIZE + 0.5).floor() as i32;
        let cz = (p.eye.z / DETAIL_SLOT_SIZE + 0.5).floor() as i32;
        self.slots.retain(|(x, z), _| (x - cx).abs() <= DM_SIZE && (z - cz).abs() <= DM_SIZE);
        let scales: Vec<(f32, f32)> = d.indices.objects.iter().map(|o| o.loaded_scales()).collect();
        let density = d.density.unwrap_or(DEFAULT_DENSITY);
        let dither = dither_map();
        let missing: Vec<(i32, i32)> = (cz - DM_SIZE..=cz + DM_SIZE)
            .flat_map(|z| (cx - DM_SIZE..=cx + DM_SIZE).map(move |x| (x, z)))
            .filter(|k| !self.slots.contains_key(k))
            .collect();
        // the ground under the window, which every slot of it then narrows
        let reach = (DM_SIZE as f32 + 1.0) * DETAIL_SLOT_SIZE;
        let ground: Vec<(ObjId, gz_fmt_level::Aabb)> = ground
            .iter()
            .filter(|(_, g)| {
                g.min.x <= p.eye.x + reach
                    && g.max.x >= p.eye.x - reach
                    && g.min.z <= p.eye.z + reach
                    && g.max.z >= p.eye.z - reach
            })
            .copied()
            .collect();
        let made: Vec<((i32, i32), Arc<SlotItems>)> = missing
            .par_iter()
            .map(|&(x, z)| {
                let items = match query(d, x, z) {
                    Some(slot) if !slot.is_empty() => {
                        let b = slot_box(slot, x, z);
                        let b = gz_fmt_level::Aabb::new(b.min, b.max);
                        let near: Vec<ObjId> = ground
                            .iter()
                            .filter(|(_, g)| g.min.cmple(b.max).all() && g.max.cmpge(b.min).all())
                            .map(|(id, _)| *id)
                            .collect();
                        let tris = gz_scene::pick::box_triangles(p.state, p.lib, &near, &b);
                        decompress(slot, x, z, &scales, density, &dither, &tris)
                    }
                    _ => SlotItems::default(),
                };
                ((x, z), Arc::new(items))
            })
            .collect();
        self.slots.extend(made);

        // per slot the fade, per item the size on the picture
        let fade_limit = DM_FADE * DM_FADE;
        let fade_start = 1.0f32;
        let fade_range = fade_limit - fade_start;
        let meshes: Vec<(gz_render::MeshHandle, Option<f32>)> = d
            .indices
            .objects
            .iter()
            .map(|o| {
                let m = p.assets.object(&o.reference);
                (m, p.assets.mesh_bounds(m).map(|b| b.sphere().1))
            })
            .collect();
        let mut per_object: Vec<Vec<BatchInstance>> = vec![Vec::new(); meshes.len()];
        let mut bounds: Vec<Aabb> = vec![Aabb::EMPTY; meshes.len()];
        for s in self.slots.values() {
            let Some(b) = s.bounds else { continue };
            // the items stand on the slot's ground: their models rise above it
            let test = Aabb::new(b.min - Vec3::new(0.5, 0.0, 0.5), b.max + Vec3::new(0.5, 2.0, 0.5));
            if !p.frustum.intersects_aabb(&test) {
                continue;
            }
            let dist_sq = p.eye.distance_squared(b.center()).max(1e-6);
            if dist_sq > fade_limit {
                continue;
            }
            let alpha = if dist_sq < fade_start { 0.0 } else { (dist_sq - fade_start) / fade_range };
            let alpha_i = 1.0 - alpha;
            for it in &s.items {
                let o = usize::from(it.object);
                let Some((_, radius)) = meshes.get(o) else { continue };
                let scale = it.scale * alpha_i;
                if let Some(r) = radius
                    && scale * scale * r * r / dist_sq < p.ssa_discard
                {
                    continue;
                }
                if let (Some(list), Some(bb)) = (per_object.get_mut(o), bounds.get_mut(o)) {
                    list.push(BatchInstance { transform: item_transform(it, scale), tint: Color::WHITE });
                    bb.extend(it.pos);
                }
            }
        }
        for ((mesh, _), (instances, b)) in meshes.into_iter().zip(per_object.into_iter().zip(bounds)) {
            if instances.is_empty() {
                continue;
            }
            built.list.batches.push(Batch {
                mesh,
                material: None,
                instances,
                bounds: b,
                id: 0,
                flags: InstanceFlags::NO_ID,
            });
        }
    }

    /// The ground's surfaces with the base texture laid over the box.
    fn base_texture(&mut self, d: &SceneDetails, p: &Pass<'_>, ground: &[ObjId], built: &mut Built) {
        let Some(texture) = d.base_texture.as_ref().filter(|t| !t.is_empty()) else { return };
        let blended = d.flags & tool_flags::BASE_TEXTURE_BLENDED != 0;
        let shader = if blended { "editor\\do_base" } else { "default" };
        let want = format!("{shader}|{texture}");
        let bbox = d.bbox;
        if self.base.as_deref() != Some(want.as_str()) {
            let tris = gz_scene::pick::box_triangles(p.state, p.lib, ground, &bbox);
            let size = bbox.max - bbox.min;
            let uv =
                |v: Vec3| glam::Vec2::new((v.x - bbox.min.x) / size.x, 1.0 - (v.z - bbox.min.z) / size.z);
            let mut verts = Vec::with_capacity(tris.len() * 3);
            for t in &tris {
                let n = (t[1] - t[0]).cross(t[2] - t[0]).try_normalize().unwrap_or(Vec3::Y);
                for v in t {
                    verts.push(MeshVertex::new(*v, n, uv(*v)));
                }
            }
            let n = verts.len() as u32;
            let table = p.assets.table();
            let material = MaterialDesc::textured(table.classify(shader), texture);
            match MeshData::new(verts, (0..n).collect(), vec![SubMesh { indices: 0..n, material }]) {
                Ok(mesh) => {
                    p.assets.add_mesh(BASE_MESH, mesh);
                    self.base = Some(want);
                }
                Err(_) => return,
            }
        }
        let mut inst =
            Instance::new(p.assets.mesh(gz_render::MeshKey::Named(BASE_MESH.into())), Mat4::IDENTITY)
                .with_flags(InstanceFlags::NO_ID | InstanceFlags::DECAL | InstanceFlags::TWO_SIDED);
        inst.bounds = crate::draw::ga(&bbox);
        built.list.instances.push(inst);
        built.owners.push(ObjId(0));
    }
}

/// Every slot within 75 m that the view sees, its box shrunk by 5 cm, in the
/// inactive grey (a slot is selected by the details' editing tools).
fn slot_boxes(d: &SceneDetails, selected: &[bool], p: &Pass<'_>, built: &mut Built) {
    let h = &d.header;
    let inactive = Color::from_argb(0xff80_8080);
    let chosen = Color::from_argb(0xffff_ffff);
    let lim = BOX_REACH * BOX_REACH;
    let s = DETAIL_SLOT_SIZE;
    // the slot columns and rows whose centre may lie within reach
    let lo = |c: f32, offs: i32| (((c - BOX_REACH - s) / s).floor() as i64 + i64::from(offs)).max(0);
    let hi = |c: f32, offs: i32, n: u32| {
        (((c + BOX_REACH) / s).ceil() as i64 + i64::from(offs)).min(i64::from(n) - 1)
    };
    let (x0, x1) = (lo(p.eye.x, h.offs_x), hi(p.eye.x, h.offs_x, h.size_x));
    let (z0, z1) = (lo(p.eye.z, h.offs_z), hi(p.eye.z, h.offs_z, h.size_z));
    for z in z0..=z1 {
        for x in x0..=x1 {
            let (Ok(xu), Ok(zu)) = (u32::try_from(x), u32::try_from(z)) else { continue };
            let Some(slot) = h.slot_index(xu, zu).and_then(|i| d.slots.get(i)) else { continue };
            let c = Vec3::new(
                h.from_slot_x(xu as i32),
                slot.base_height() + slot.height() * 0.5,
                h.from_slot_z(zu as i32),
            );
            if p.eye.distance_squared(c) >= lim || !p.frustum.intersects_sphere(c, s * 0.5) {
                continue;
            }
            let half = s * 0.5 - 0.05;
            let b = Aabb::new(
                Vec3::new(c.x - half, slot.base_height() + 0.05, c.z - half),
                Vec3::new(c.x + half, slot.base_height() + slot.height() - 0.05, c.z + half),
            );
            let on = h.slot_index(xu, zu).and_then(|i| selected.get(i)).copied().unwrap_or(false);
            built.list.helpers.shape(
                ShapeKind::SelectionBox,
                b.unit_transform(),
                None,
                Some(if on { chosen } else { inactive }),
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dither_map_is_the_engines() {
        let m = dither_map();
        // magicfact = 254 / 16 = 15.875: the first cells
        assert_eq!(m[0][0], 0);
        // i = 0, j = 1 (14), k = l = 0: 0.5 + 14 * 15.875 = 222.75
        assert_eq!(m[0][1], 222);
        // k = 0, l = 1 adds 14 / 16 * 15.875: 0.5 + 13.890625
        assert_eq!(m[0][4], 14);
        assert!(m.iter().flatten().all(|v| (0..=255).contains(v)));
    }

    #[test]
    fn the_ray_hits_a_front_face_only() {
        // a floor at y = 0 wound so that it faces up (clockwise seen from above)
        let up = [Vec3::new(-1.0, 0.0, -1.0), Vec3::new(-1.0, 0.0, 1.0), Vec3::new(1.0, 0.0, -1.0)];
        assert!(
            (ray_tri(Vec3::new(-0.5, 2.0, -0.5), Vec3::new(0.0, -1.0, 0.0), &up).unwrap() - 2.0).abs() < 1e-5
        );
        let down = [up[0], up[2], up[1]];
        assert!(ray_tri(Vec3::new(-0.5, 2.0, -0.5), Vec3::new(0.0, -1.0, 0.0), &down).is_none());
    }

    /// A slot built by hand: object 0 everywhere at full density, object 1 nowhere.
    fn full_slot(base: f32) -> DetailSlot {
        let mut s = DetailSlot::default();
        s.set_heights(base, 2.0);
        s.set_id(0, 0);
        s.set_id(1, DetailSlot::ID_EMPTY);
        s.set_id(2, DetailSlot::ID_EMPTY);
        s.set_id(3, DetailSlot::ID_EMPTY);
        for c in 0..4 {
            s.set_density(0, c, 15);
        }
        s
    }

    fn floor(y: f32) -> Vec<[Vec3; 3]> {
        let (a, b, c, d) = (
            Vec3::new(-10.0, y, -10.0),
            Vec3::new(-10.0, y, 10.0),
            Vec3::new(10.0, y, -10.0),
            Vec3::new(10.0, y, 10.0),
        );
        vec![[a, b, c], [c, b, d]]
    }

    #[test]
    fn a_full_slot_decompresses_onto_the_ground() {
        let slot = full_slot(0.0);
        let dither = dither_map();
        let got = decompress(&slot, 1, 2, &[(1.0, 1.0)], 0.3, &dither, &floor(0.5));
        // d_size = ceil(2 / 0.3) = 7: 8 x 8 places, all filled at 255 > every dither value
        assert_eq!(got.items.len(), 64);
        for it in &got.items {
            assert!((it.pos.y - 0.5).abs() < 1e-4);
            assert!(it.pos.x >= 2.0 - 0.3 / 1.7 - 1e-3 && it.pos.x <= 4.0 + 0.3 / 1.7 + 1e-3);
            assert!((0.5..=0.9).contains(&it.scale));
            assert!((0.0..=PI_MUL_2).contains(&it.yaw));
            assert_eq!(it.object, 0);
        }
        // the same slot gives the same details: the seeds are the slot's
        assert_eq!(got, decompress(&slot, 1, 2, &[(1.0, 1.0)], 0.3, &dither, &floor(0.5)));
        // the first place is the jitter's: x from the second draw, z from the first
        let mut r = Rng::new((0x1207_1980u32 ^ 2) as i32);
        r.rand_i_max(16);
        r.rand_i_max(16);
        let jz = r.rand_fs(0.3 / 1.7);
        let jx = r.rand_fs(0.3 / 1.7);
        let first = got.items[0].pos;
        assert!((first.x - (2.0 - EPS_L + jx)).abs() < 1e-4 && (first.z - (4.0 - EPS_L + jz)).abs() < 1e-4);
        // no ground under the slot: nothing; ground below the slot's base: nothing either
        assert!(decompress(&slot, 1, 2, &[(1.0, 1.0)], 0.3, &dither, &[]).items.is_empty());
        assert!(decompress(&slot, 1, 2, &[(1.0, 1.0)], 0.3, &dither, &floor(-3.0)).items.is_empty());
        // an object the list does not have places nothing
        assert!(decompress(&slot, 1, 2, &[], 0.3, &dither, &floor(0.5)).items.is_empty());
    }

    #[test]
    fn an_empty_corner_thins_the_slot() {
        let mut slot = full_slot(0.0);
        slot.set_density(0, 3, 0);
        let got = decompress(&slot, 0, 0, &[(1.0, 1.0)], 0.3, &dither_map(), &floor(0.5));
        assert!(!got.items.is_empty() && got.items.len() < 64);
    }

    #[test]
    fn slot_boxes_follow_the_grid() {
        let slot = full_slot(-1.0);
        let b = slot_box(&slot, -3, 4);
        assert!((b.min.x - -6.0).abs() < 0.01 && (b.max.z - 10.0).abs() < 0.01);
        assert!((b.min.y - slot.base_height()).abs() < 0.01);
    }
}
