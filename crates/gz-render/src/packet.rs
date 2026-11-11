//! A frame as the render thread draws it: built from a [`DrawList`] and a [`View`] on the CPU
//! pool, immutable, self-contained - handles and instance data.

use std::ops::Range;
use std::sync::Arc;

use bytemuck::{Pod, Zeroable};
use glam::{Mat3, Mat4, Vec3};
use rayon::prelude::*;
use rustc_hash::FxHashMap;
use smol_str::SmolStr;

use crate::assets::{Assets, MaterialHandle, MeshHandle, MeshInfo};
use crate::camera::project;
use crate::color::Color;
use crate::draw_list::{
    DrawList, Instance, InstanceFlags, LightKind, RoofMap, Sky, SpriteAlign, SpriteBatch, SpriteCull, SpriteSize,
    Visibility,
};
use crate::gpu::MeshPipe;
use crate::helpers::{HelperGeometry, grid_lines, tessellate};
use crate::material::{EditorMaterial, MaterialDesc, Pass, texture_key};
use crate::math::{Aabb, Frustum};
use crate::mesh::MeshVertex;
use crate::view::{View, ViewFlags, ViewMode};

/// The most lights the game-lighting mode draws, nearest first (`EGameLighting::MAX_LIGHTS`).
pub(crate) const MAX_LIGHTS: usize = 128;
/// The directional lights the C++ editor's fixed-function emulation takes.
const MAX_EDITOR_LIGHTS: usize = 4;
/// The most labels a frame tests for depth.
pub(crate) const MAX_LABEL_QUERIES: usize = 4096;
/// Lists at least this long are culled and sorted in parallel.
const PARALLEL: usize = 2048;
/// `bones_at` of an instance that is not skinned.
pub(crate) const NO_BONES: u32 = u32::MAX;

/// The frame block of `common.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct FrameUniform {
    pub view_proj: [[f32; 4]; 4],
    pub view: [[f32; 4]; 4],
    pub camera_pos: [f32; 4],
    pub camera_dir: [f32; 4],
    pub viewport: [f32; 4],
    pub mode: [u32; 4],
    pub ambient: [f32; 4],
    pub hemi: [f32; 4],
    pub sun_dir: [f32; 4],
    pub sun_color: [f32; 4],
    pub fog_color: [f32; 4],
    pub fog: [f32; 4],
    pub roof: [f32; 4],
    pub selection: [f32; 4],
    pub wire: [f32; 4],
}

/// An instance of `common.wgsl` (144 bytes).
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct InstanceGpu {
    pub model: [[f32; 4]; 4],
    pub normal: [[f32; 4]; 3],
    pub color: [f32; 4],
    pub id: u32,
    pub flags: u32,
    pub bones_at: u32,
    pub pad: u32,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Pod, Zeroable)]
pub(crate) struct LightGpu {
    pub pos_range: [f32; 4],
    pub color_cos: [f32; 4],
    pub dir: [f32; 4],
}

/// One instanced draw: a part of a mesh with a material, for a run of instances.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Draw {
    /// Into [`FramePacket::meshes`].
    pub mesh: u32,
    pub submesh: u32,
    /// Into [`FramePacket::materials`].
    pub material: u32,
    pub pipe: MeshPipe,
    pub instances: Range<u32>,
    /// Vertices of [`FramePacket::dynamic`] drawn as a triangle list instead of the mesh.
    pub dynamic: Option<Range<u32>>,
}

/// A label where the frame put it.
#[derive(Clone, Debug, PartialEq)]
pub struct ScreenLabel {
    /// Pixels from the top-left corner of the picture.
    pub position: [f32; 2],
    /// 1 at the near plane, 0 at the far one.
    pub depth: f32,
    pub text: SmolStr,
    pub color: Color,
    pub depth_tested: bool,
    /// In front of the camera and inside the picture; for a depth-tested label also not
    /// hidden, once the frame that drew it says so ([`crate::FrameInfo::labels`]).
    pub visible: bool,
    pub(crate) query: Option<u32>,
}

/// What a packet holds.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PacketStats {
    pub instances: usize,
    pub visible: usize,
    pub draws: usize,
    pub triangles: u64,
    /// Instances drawn as a grey box because their mesh is not there (yet).
    pub placeholders: usize,
    /// Lights whose range reaches the view, before the cut to the nearest 128.
    pub lights_in_view: usize,
    pub lights: usize,
    /// Sprites built (particles, glows).
    pub sprites: usize,
    /// Copies drawn from batches (detail objects).
    pub batched: usize,
}

/// A frame, ready to record.
#[derive(Clone, Debug)]
pub struct FramePacket {
    pub view: View,
    pub stats: PacketStats,
    pub(crate) frame: FrameUniform,
    pub(crate) clear: [f64; 4],
    pub(crate) instances: Vec<InstanceGpu>,
    /// Geometry built for this frame alone: the sprites' quads.
    pub(crate) dynamic: Vec<MeshVertex>,
    pub(crate) bones: Vec<[[f32; 4]; 4]>,
    pub(crate) lights: Vec<LightGpu>,
    pub(crate) meshes: Vec<MeshHandle>,
    pub(crate) materials: Vec<MaterialHandle>,
    /// Opaque, alpha-tested, sky, then blended (back to front).
    pub(crate) draws: Vec<Draw>,
    pub(crate) opaque: Range<usize>,
    pub(crate) alpha: Range<usize>,
    pub(crate) sky: Range<usize>,
    pub(crate) blended: Range<usize>,
    pub(crate) helpers: HelperGeometry,
    pub(crate) labels: Vec<ScreenLabel>,
    pub(crate) label_points: Vec<[f32; 4]>,
    pub(crate) roof: Option<Arc<RoofMap>>,
    pub(crate) id_pass: bool,
    pub(crate) assets_generation: u64,
    pub(crate) waiting: bool,
}

impl FramePacket {
    /// Whether an instance draws a placeholder for a mesh that is still loading.
    #[must_use]
    pub fn waiting_on_assets(&self) -> bool {
        self.waiting
    }

    /// Whether a packet built now would draw more: something it waited for has arrived.
    #[must_use]
    pub fn is_stale(&self, assets: &Assets) -> bool {
        self.waiting && assets.generation() != self.assets_generation
    }

    /// The labels as this packet placed them; a depth-tested one's `visible` is settled by the
    /// frame that draws it.
    #[must_use]
    pub fn labels(&self) -> &[ScreenLabel] {
        &self.labels
    }
}

/// A visible instance.
struct Seen {
    index: u32,
    mesh: u32,
    depth: f32,
    local: Aabb,
}

/// A draw before grouping.
#[derive(Clone)]
struct Item {
    order: u8,
    pipe: MeshPipe,
    material: u32,
    mesh: u32,
    sub: u32,
    depth: f32,
    instance: InstanceGpu,
    /// A sprite batch's vertices in [`FramePacket::dynamic`].
    dynamic: Option<(u32, u32)>,
}

/// A part of a [`crate::Batch`]: drawn with every copy of the batch.
struct BatchPart {
    order: u8,
    pipe: MeshPipe,
    material: u32,
    mesh: u32,
    sub: u32,
    batch: usize,
}

/// [`build_packet_with`] with the packet's own culling.
#[must_use]
pub fn build_packet(list: &DrawList, view: &View, assets: &Assets) -> FramePacket {
    build_packet_with(list, view, assets, None)
}

/// Culls `list` for `view` (or takes the caller's `visibility`), groups what is visible by mesh
/// and material into instanced draws, sorts the blended ones back to front, places the lights,
/// helpers and labels. Asks `assets` for whatever is not loaded yet; runs on the caller's rayon
/// pool when the list is large.
#[must_use]
pub fn build_packet_with(
    list: &DrawList,
    view: &View,
    assets: &Assets,
    visibility: Option<&Visibility>,
) -> FramePacket {
    let size = [view.size[0].max(1), view.size[1].max(1)];
    let tick = assets.next_tick();
    let generation = assets.generation();
    let preview = view.flags.contains(ViewFlags::PREVIEW);
    let wireframe = view.mode == ViewMode::Wireframe;
    let view_m = view.camera.view_matrix();
    let view_proj = view.camera.view_proj(size);
    let frustum = Frustum::from_view_proj(&view_proj);
    let eye = view.camera.position;
    let forward = view.camera.forward;

    // the weather is drawn under Game Lighting, and round any view with Environment on
    let env = if view.mode == ViewMode::GameLighting || (view.style.environment && !wireframe) {
        list.environment.as_ref()
    } else {
        None
    };
    let sky_def = env.and_then(|e| e.sky.as_ref());

    // the meshes, each once; the placeholder last
    let mut mesh_index: FxHashMap<MeshHandle, u32> = FxHashMap::default();
    let mut meshes: Vec<MeshHandle> = Vec::new();
    let mut index_mesh = |h: MeshHandle| -> u32 {
        *mesh_index.entry(h).or_insert_with(|| {
            meshes.push(h);
            meshes.len() as u32 - 1
        })
    };
    for inst in &list.instances {
        index_mesh(inst.mesh);
    }
    let batch_meshes: Vec<u32> = list.batches.iter().map(|b| index_mesh(b.mesh)).collect();
    let skybox = sky_def.map(|_| index_mesh(assets.skybox()));
    let placeholder = index_mesh(assets.placeholder());
    let resolved = assets.resolve_meshes(&meshes, tick);
    let info_of = |m: u32| -> Option<&Arc<MeshInfo>> { resolved.get(m as usize).and_then(|r| r.0.as_ref()) };

    let cull = |i: u32| -> Option<Seen> {
        let inst = list.instances.get(i as usize)?;
        if preview && inst.flags.contains(InstanceFlags::HIDDEN_IN_PREVIEW) {
            return None;
        }
        let mesh = *mesh_index.get(&inst.mesh)?;
        let local = inst.bounds.or_else(|| info_of(mesh).map(|m| m.bounds)).unwrap_or(Aabb::UNIT);
        let world = local.transformed(&inst.transform);
        if visibility.is_none()
            && !inst.flags.contains(InstanceFlags::NO_CULL)
            && !frustum.intersects_aabb(&world)
        {
            return None;
        }
        let depth = if world.is_empty() { 0.0 } else { (world.center() - eye).dot(forward) };
        Some(Seen { index: i, mesh, depth, local })
    };
    let candidates: Vec<u32> = match visibility {
        Some(v) => v.visible.iter().copied().filter(|i| (*i as usize) < list.instances.len()).collect(),
        None => (0..list.instances.len() as u32).collect(),
    };
    let seen: Vec<Seen> = if candidates.len() >= PARALLEL {
        candidates.par_iter().filter_map(|i| cull(*i)).collect()
    } else {
        candidates.iter().filter_map(|i| cull(*i)).collect()
    };

    // the materials, each once
    let mut material_index: FxHashMap<MaterialHandle, u32> = FxHashMap::default();
    let mut materials: Vec<MaterialHandle> = Vec::new();
    let mut intern = |h: MaterialHandle| -> u32 {
        *material_index.entry(h).or_insert_with(|| {
            materials.push(h);
            materials.len() as u32 - 1
        })
    };
    let default_material = assets.default_material();
    let mut waiting = false;
    let mut placeholders = 0usize;
    // per seen instance a run of `part_materials`: the material index of each of its parts, or
    // one for a placeholder
    let mut part_materials: Vec<u32> = Vec::with_capacity(seen.len());
    let mut parts: Vec<Range<usize>> = Vec::with_capacity(seen.len());
    for s in &seen {
        let start = part_materials.len();
        if let Some(inst) = list.instances.get(s.index as usize) {
            match info_of(s.mesh) {
                Some(info) => part_materials
                    .extend(info.submeshes.iter().map(|sub| intern(inst.material.unwrap_or(sub.material)))),
                None => {
                    waiting |= resolved.get(s.mesh as usize).is_some_and(|r| r.1);
                    placeholders += 1;
                    part_materials.push(intern(inst.material.unwrap_or(default_material)));
                }
            }
        }
        parts.push(start..part_materials.len());
    }
    // a batch whose mesh is not there yet draws nothing until it is
    let batch_materials: Vec<Vec<u32>> = list
        .batches
        .iter()
        .zip(&batch_meshes)
        .map(|(b, &m)| match info_of(m) {
            Some(info) => info.submeshes.iter().map(|sub| intern(b.material.unwrap_or(sub.material))).collect(),
            None => {
                waiting |= resolved.get(m as usize).is_some_and(|r| r.1);
                Vec::new()
            }
        })
        .collect();
    let sprite_materials: Vec<u32> = list.sprites.iter().map(|b| intern(b.material)).collect();
    let sky_material = sky_def.map(|s| intern(assets.material(sky_material(s))));
    let mat_infos = assets.resolve_materials(&materials, tick);
    let material_info = |m: u32| {
        mat_infos
            .get(m as usize)
            .copied()
            .unwrap_or(crate::assets::MaterialInfo { kind: EditorMaterial::Opaque, two_sided: false })
    };

    // instance data, poses, and the draws before grouping
    let mut bones: Vec<[[f32; 4]; 4]> = Vec::new();
    let mut items: Vec<Item> = Vec::with_capacity(seen.len());
    let mut blended: Vec<Item> = Vec::new();
    let mut any_selected = false;
    for (s, run) in seen.iter().zip(&parts) {
        let Some(inst) = list.instances.get(s.index as usize) else { continue };
        let mats = part_materials.get(run.clone()).unwrap_or(&[]);
        any_selected |= inst.flags.contains(InstanceFlags::SELECTED);
        let info = info_of(s.mesh);
        let bones_at = match (&inst.pose, info) {
            (Some(pose), Some(i)) if i.skinned && !pose.is_empty() => {
                let at = bones.len() as u32;
                bones.extend(pose.iter().map(Mat4::to_cols_array_2d));
                at
            }
            _ => NO_BONES,
        };
        let (mesh, model) = match info {
            Some(_) => (s.mesh, inst.transform),
            None => (placeholder, inst.transform * s.local.unit_transform()),
        };
        let gpu = instance_gpu(&model, inst, bones_at);
        for (sub, &material) in mats.iter().enumerate() {
            let m = material_info(material);
            if m.kind == EditorMaterial::Distort {
                continue;
            }
            let two_sided = m.two_sided || inst.flags.contains(InstanceFlags::TWO_SIDED);
            let pipe = MeshPipe::of(m.kind, two_sided, wireframe);
            let pass = if wireframe && m.kind != EditorMaterial::Sky { Pass::Opaque } else { m.kind.pass() };
            let item = Item {
                order: pass_order(pass),
                pipe,
                material,
                mesh,
                sub: sub as u32,
                depth: s.depth,
                instance: gpu,
                dynamic: None,
            };
            // Edged Faces: the surface's edges once more, over it
            if view.style.edged && !wireframe && pass != Pass::Blended && m.kind != EditorMaterial::Sky {
                items.push(Item { pipe: MeshPipe::of(m.kind, two_sided, true), ..item.clone() });
            }
            if pass == Pass::Blended { blended.push(item) } else { items.push(item) }
        }
    }

    // the weather's sky box, round the camera
    if let (Some(s), Some(material), Some(mesh)) = (sky_def, sky_material, skybox)
        && info_of(mesh).is_some()
    {
        let q = |v: f32| ((v * 255.0).floor() as i32).clamp(0, 255) as u8;
        let tint = Color::rgba(q(s.color.x), q(s.color.y), q(s.color.z), q(s.weight));
        let flags = InstanceFlags::SKY_TC | InstanceFlags::NO_ID | InstanceFlags::NO_CULL;
        let model = Mat4::from_rotation_y(s.rotation);
        items.push(Item {
            order: pass_order(Pass::Sky),
            pipe: MeshPipe::Sky,
            material,
            mesh,
            sub: 0,
            depth: 0.0,
            instance: gpu_of(&model, tint, 0, flags, NO_BONES),
            dynamic: None,
        });
    }

    // sprites: quads built facing this camera, one draw per batch
    let mut dynamic: Vec<MeshVertex> = Vec::new();
    let mut sprite_count = 0usize;
    for (b, &material) in list.sprites.iter().zip(&sprite_materials) {
        if (preview && b.flags.contains(InstanceFlags::HIDDEN_IN_PREVIEW))
            || b.bounds.is_some_and(|bb| !frustum.intersects_aabb(&bb))
        {
            continue;
        }
        let m = material_info(material);
        if m.kind == EditorMaterial::Distort {
            continue;
        }
        let start = dynamic.len() as u32;
        expand_sprites(b, view, size, &mut dynamic);
        let end = dynamic.len() as u32;
        if end == start {
            continue;
        }
        sprite_count += b.sprites.len();
        let depth = match (b.bounds, b.sprites.first()) {
            (Some(bb), _) => (bb.center() - eye).dot(forward),
            (None, Some(s)) => (s.position - eye).dot(forward),
            (None, None) => 0.0,
        };
        let pass = m.kind.pass();
        let item = Item {
            order: pass_order(pass),
            pipe: MeshPipe::of(m.kind, true, false),
            material,
            mesh: placeholder,
            sub: 0,
            depth,
            instance: gpu_of(&Mat4::IDENTITY, Color::WHITE, b.id, b.flags, NO_BONES),
            dynamic: Some((start, end)),
        };
        if pass == Pass::Blended { blended.push(item) } else { items.push(item) }
    }

    // batches: every copy as it comes, a draw per part
    let mut batch_parts: Vec<BatchPart> = Vec::new();
    let mut batch_gpu: Vec<Vec<InstanceGpu>> = Vec::with_capacity(list.batches.len());
    let mut batched = 0usize;
    for (bi, ((b, mats), &mesh)) in list.batches.iter().zip(&batch_materials).zip(&batch_meshes).enumerate() {
        let shown = !(preview && b.flags.contains(InstanceFlags::HIDDEN_IN_PREVIEW)) && !b.instances.is_empty();
        if !shown || mats.is_empty() {
            batch_gpu.push(Vec::new());
            continue;
        }
        let make = |i: &crate::draw_list::BatchInstance| gpu_of(&i.transform, i.tint, b.id, b.flags, NO_BONES);
        batch_gpu.push(if b.instances.len() >= PARALLEL {
            b.instances.par_iter().map(make).collect()
        } else {
            b.instances.iter().map(make).collect()
        });
        batched += b.instances.len();
        for (sub, &material) in mats.iter().enumerate() {
            let m = material_info(material);
            if m.kind == EditorMaterial::Distort {
                continue;
            }
            let two_sided = m.two_sided || b.flags.contains(InstanceFlags::TWO_SIDED);
            let pass = if wireframe && m.kind != EditorMaterial::Sky { Pass::Opaque } else { m.kind.pass() };
            batch_parts.push(BatchPart {
                order: pass_order(pass),
                pipe: MeshPipe::of(m.kind, two_sided, wireframe),
                material,
                mesh,
                sub: sub as u32,
                batch: bi,
            });
            if view.style.edged && !wireframe && pass != Pass::Blended && m.kind != EditorMaterial::Sky {
                batch_parts.push(BatchPart {
                    order: pass_order(pass),
                    pipe: MeshPipe::of(m.kind, two_sided, true),
                    material,
                    mesh,
                    sub: sub as u32,
                    batch: bi,
                });
            }
        }
    }

    // opaque by state; blended back to front
    let key = |i: &Item| (i.order, i.pipe, i.material, i.mesh, i.sub, i.dynamic);
    if items.len() >= PARALLEL {
        items.par_sort_unstable_by_key(key);
    } else {
        items.sort_unstable_by_key(key);
    }
    if blended.len() >= PARALLEL {
        blended.par_sort_by(|a, b| b.depth.total_cmp(&a.depth));
    } else {
        blended.sort_by(|a, b| b.depth.total_cmp(&a.depth));
    }
    let mut instances: Vec<InstanceGpu> =
        Vec::with_capacity(items.len() + blended.len() + batch_gpu.iter().map(Vec::len).sum::<usize>());
    let mut draws: Vec<Draw> = Vec::new();
    let mut batch_ranges: Vec<Option<Range<u32>>> = vec![None; batch_gpu.len()];
    // the end of each pass's draws: opaque, alpha-tested, sky, blended
    let mut ends = [0usize; 4];
    for (order, set) in [(0u8, &items), (1, &items), (2, &items), (3, &blended)] {
        let pass_start = draws.len();
        for it in set.iter().filter(|i| i.order == order) {
            let at = instances.len() as u32;
            instances.push(it.instance);
            // only a run of the same draw within the pass merges; a sprite batch never does
            let in_pass = draws.len() > pass_start;
            match draws.last_mut() {
                Some(d)
                    if in_pass
                        && it.dynamic.is_none()
                        && d.dynamic.is_none()
                        && d.instances.end == at
                        && (d.pipe, d.material, d.mesh, d.submesh)
                            == (it.pipe, it.material, it.mesh, it.sub) =>
                {
                    d.instances.end = at + 1;
                }
                _ => draws.push(Draw {
                    mesh: it.mesh,
                    submesh: it.sub,
                    material: it.material,
                    pipe: it.pipe,
                    instances: at..at + 1,
                    dynamic: it.dynamic.map(|(a, b)| a..b),
                }),
            }
        }
        for p in batch_parts.iter().filter(|p| p.order == order) {
            let Some(slot) = batch_ranges.get_mut(p.batch) else { continue };
            let range = match slot {
                Some(r) => r.clone(),
                None => {
                    let at = instances.len() as u32;
                    instances.extend(batch_gpu.get(p.batch).map_or(&[][..], Vec::as_slice));
                    let r = at..instances.len() as u32;
                    *slot = Some(r.clone());
                    r
                }
            };
            draws.push(Draw {
                mesh: p.mesh,
                submesh: p.sub,
                material: p.material,
                pipe: p.pipe,
                instances: range,
                dynamic: None,
            });
        }
        if let Some(e) = ends.get_mut(order as usize) {
            *e = draws.len();
        }
    }
    let [opaque_end, alpha_end, sky_end, blended_end] = ends;
    let opaque = 0..opaque_end;
    let alpha = opaque_end..alpha_end;
    let sky = alpha_end..sky_end;
    let blended_range = sky_end..blended_end;

    let triangles = draws
        .iter()
        .map(|d| {
            if let Some(r) = &d.dynamic {
                return u64::from(r.end - r.start) / 3;
            }
            let tris = info_of(d.mesh)
                .and_then(|i| i.submeshes.get(d.submesh as usize))
                .map_or(12, |s| u64::from(s.indices.end - s.indices.start) / 3);
            tris * u64::from(d.instances.end - d.instances.start)
        })
        .sum();

    // lights: those that reach the view, nearest first
    let mut lights: Vec<LightGpu> = Vec::new();
    let mut lights_in_view = 0;
    let editor_lights = view.mode == ViewMode::Lit && view.flags.contains(ViewFlags::SCENE_LIGHTS);
    if editor_lights {
        // the emulation takes the first four directional lights, in the list's order
        for l in &list.lights {
            let LightKind::Directional { direction, ranged } = l.kind else { continue };
            lights_in_view += 1;
            if lights.len() < MAX_EDITOR_LIGHTS {
                lights.push(LightGpu {
                    pos_range: l.position.extend(if ranged { l.range } else { f32::MAX }).to_array(),
                    color_cos: l.color.extend(0.0).to_array(),
                    dir: direction.try_normalize().unwrap_or(-Vec3::Y).extend(if ranged { 1.0 } else { 2.0 }).to_array(),
                });
            }
        }
    } else if env.is_some() {
        let mut near: Vec<(f32, &crate::draw_list::Light)> = list
            .lights
            .iter()
            .filter(|l| !matches!(l.kind, LightKind::Directional { .. }))
            .filter(|l| l.range > 0.0 && frustum.intersects_sphere(l.position, l.range))
            .map(|l| (l.position.distance_squared(eye), l))
            .collect();
        lights_in_view = near.len();
        near.sort_by(|a, b| a.0.total_cmp(&b.0));
        lights.extend(near.iter().take(MAX_LIGHTS).map(|(_, l)| {
            let (dir, cos) = match l.kind {
                LightKind::Point => (Vec3::ZERO, 0.0),
                LightKind::Spot { direction, cos_half } => {
                    (direction.try_normalize().unwrap_or(Vec3::Z), cos_half.clamp(0.0, 0.9999))
                }
                LightKind::Directional { .. } => (Vec3::ZERO, 0.0),
            };
            LightGpu {
                pos_range: l.position.extend(l.range).to_array(),
                color_cos: l.color.extend(cos).to_array(),
                dir: dir.extend(0.0).to_array(),
            }
        }));
    }

    // helpers, the grid and the labels
    let mut helpers = HelperGeometry::default();
    if view.shows(ViewFlags::GRID) {
        grid_lines(&view.grid, &mut helpers.lines);
    }
    let mut labels = Vec::new();
    let mut label_points = Vec::new();
    if view.shows(ViewFlags::HELPERS) {
        tessellate(&list.helpers, &mut helpers);
        for l in &list.helpers.labels {
            let Some(p) = project(&view_proj, size, l.position) else { continue };
            if p.x < 0.0
                || p.y < 0.0
                || p.x > size[0] as f32
                || p.y > size[1] as f32
                || !(0.0..=1.0).contains(&p.z)
            {
                continue;
            }
            let query = (l.depth_tested && label_points.len() < MAX_LABEL_QUERIES).then(|| {
                label_points.push(l.position.extend(1.0).to_array());
                label_points.len() as u32 - 1
            });
            labels.push(ScreenLabel {
                position: [p.x, p.y],
                depth: p.z,
                text: l.text.clone(),
                color: l.color,
                depth_tested: l.depth_tested,
                visible: true,
                query,
            });
        }
    }

    let roof = env
        .and_then(|e| e.roof.clone())
        .filter(|r| r.cells > 0 && r.heights.len() >= (r.cells as usize).pow(2));
    let id_pass = view.shows(ViewFlags::SELECTION) && any_selected;
    let mode = match (view.mode, env) {
        (ViewMode::GameLighting, None) => ViewMode::Lit,
        (m, _) => m,
    };
    let st = &view.style;
    let mut flags = 0u32;
    if roof.is_some() {
        flags |= 1;
    }
    if view.camera.projection.is_ortho() {
        flags |= 4;
    }
    if editor_lights {
        flags |= 8;
    }
    if st.flat {
        flags |= 16;
    }
    if !st.textures {
        flags |= 32;
    }
    if st.fog {
        flags |= 64;
    }
    if !st.linear {
        flags |= 128;
    }
    if st.edged && !wireframe {
        flags |= 256;
    }
    let mut frame = FrameUniform {
        view_proj: view_proj.to_cols_array_2d(),
        view: view_m.to_cols_array_2d(),
        camera_pos: eye.extend(1.0).to_array(),
        camera_dir: forward.extend(0.0).to_array(),
        viewport: [size[0] as f32, size[1] as f32, 1.0 / size[0] as f32, 1.0 / size[1] as f32],
        mode: [mode.code(), lights.len() as u32, flags, u32::from(view.shows(ViewFlags::SELECTION))],
        selection: view.selection_color.to_f32(),
        wire: view.wire_color.to_f32(),
        ..FrameUniform::default()
    };
    let bg = view.background.to_f32();
    let mut clear = [f64::from(bg[0]), f64::from(bg[1]), f64::from(bg[2]), 1.0];
    if let Some(e) = env {
        frame.ambient = e.ambient.extend(e.exposure).to_array();
        frame.hemi = e.hemi.extend(e.indoor_hemi.clamp(0.0, 1.0)).to_array();
        frame.sun_dir = e.sun_dir.try_normalize().unwrap_or(-Vec3::Y).extend(0.0).to_array();
        frame.sun_color = e.sun_color.extend(0.0).to_array();
        frame.fog_color = e.fog_color.extend(1.0).to_array();
        frame.fog = [e.fog_near, e.fog_far, 0.0, 0.0];
        // Fog without Environment is the editor's own; Game Lighting always fogs
        if let Some(r) = &roof {
            frame.roof = [r.origin[0], r.origin[1], 1.0 / r.cell.max(1e-3), r.cells as f32];
        }
        // the background is the weather's fog colour, exposed
        let fog = (e.fog_color * e.exposure).clamp(Vec3::ZERO, Vec3::ONE);
        clear = [f64::from(fog.x), f64::from(fog.y), f64::from(fog.z), 1.0];
    }
    // `CToolCustom::GetCurrentFog` without the weather: from 0.1 x 0.85 of the far plane to 0.91
    // of it, in the tools' fog colour (white)
    if st.fog && (env.is_none() || (mode != ViewMode::GameLighting && !st.environment)) {
        let far = view.camera.projection.far();
        frame.fog = [(1.0 - 0.9) * 0.85 * far, 0.91 * far, 0.0, 0.0];
        frame.fog_color = [1.0, 1.0, 1.0, 1.0];
    }

    let stats = PacketStats {
        instances: list.instances.len(),
        visible: seen.len(),
        draws: draws.len(),
        triangles,
        placeholders,
        lights_in_view,
        lights: lights.len(),
        sprites: sprite_count,
        batched,
    };
    FramePacket {
        view: *view,
        stats,
        frame,
        clear,
        instances,
        dynamic,
        bones,
        lights,
        meshes,
        materials,
        draws,
        opaque,
        alpha,
        sky,
        blended: blended_range,
        helpers,
        labels,
        label_points,
        roof,
        id_pass,
        assets_generation: generation,
        waiting,
    }
}

fn pass_order(p: Pass) -> u8 {
    match p {
        Pass::Opaque | Pass::Overlay => 0,
        Pass::AlphaTest => 1,
        Pass::Sky => 2,
        Pass::Blended => 3,
    }
}

fn instance_gpu(model: &Mat4, inst: &Instance, bones_at: u32) -> InstanceGpu {
    gpu_of(model, inst.tint, inst.id, inst.flags, bones_at)
}

fn gpu_of(model: &Mat4, tint: Color, id: u32, flags: InstanceFlags, bones_at: u32) -> InstanceGpu {
    let m3 = Mat3::from_mat4(*model);
    let n = if m3.determinant().abs() > 1e-12 { m3.inverse().transpose() } else { m3 };
    InstanceGpu {
        model: model.to_cols_array_2d(),
        normal: [
            n.x_axis.extend(0.0).to_array(),
            n.y_axis.extend(0.0).to_array(),
            n.z_axis.extend(0.0).to_array(),
        ],
        color: tint.to_f32(),
        id,
        flags: flags.bits(),
        bones_at,
        pad: 0,
    }
}

/// The material a weather's sky is drawn with: the two cube maps.
fn sky_material(s: &Sky) -> MaterialDesc {
    let mut m = MaterialDesc::textured(EditorMaterial::Sky, &s.textures[0]);
    m.detail = (!s.textures[1].is_empty()).then(|| texture_key(&s.textures[1]));
    m
}

/// `normalize_safe`.
fn normalize_safe(v: Vec3) -> Vec3 {
    let m = v.length_squared();
    if m > f32::MIN_POSITIVE { v * (1.0 / m).sqrt() } else { v }
}

/// `FillSprite` for every sprite of a batch: two triangles each, as the engine's quad index
/// buffer winds them, facing this view's camera.
fn expand_sprites(b: &SpriteBatch, view: &View, size: [u32; 2], out: &mut Vec<MeshVertex>) {
    let cam = &view.camera;
    let (up, right, fwd, eye) = (cam.up, cam.right(), cam.forward, cam.position);
    let ortho = cam.projection.is_ortho();
    // the picture's x scale: a glow's half size is width * radius / depth pixels
    let p11 = cam.proj_matrix(size).x_axis.x.abs().max(1e-6);
    out.reserve(b.sprites.len() * 6);
    let normal = (-fwd).to_array();
    for s in &b.sprites {
        let depth = if ortho { 1.0 } else { (s.position - eye).dot(fwd) };
        let half = match b.size {
            SpriteSize::World => s.half,
            SpriteSize::Screen => s.half * (2.0 / p11),
            SpriteSize::ScreenFixed => s.half * (depth / p11),
        };
        if b.size != SpriteSize::World && depth <= 0.0 {
            continue;
        }
        let (t, r) = match s.align {
            SpriteAlign::Camera => (up, right),
            SpriteAlign::Axes { top, right } => (top, right),
            SpriteAlign::Along(dir) => (dir, normalize_safe(dir.cross(fwd))),
        };
        let (sa, ca) = s.angle.sin_cos();
        let vr = (t * sa + r * ca) * half.x;
        let vt = (t * ca - r * sa) * half.y;
        let a = vt - vr;
        let bb = vt + vr;
        let (c, d) = (-a, -bb);
        let p = s.position;
        if b.cull != SpriteCull::None {
            // (d, a, c) clockwise on the picture: its normal points at the camera
            let to_eye = if ortho { -fwd } else { eye - (p + d) };
            let clockwise = (a - d).cross(c - d).dot(to_eye) > 0.0;
            if clockwise == (b.cull == SpriteCull::Clockwise) {
                continue;
            }
        }
        let [l, tp, rt, bt] = s.uv;
        let v = |o: Vec3, u: f32, w: f32| MeshVertex {
            position: (p + o).to_array(),
            normal,
            uv: [u, w],
            uv2: [0.0; 2],
            color: s.color.0,
            bones: [0; 4],
            weights: [0; 4],
        };
        let (vd, va, vc, vb) = (v(d, l, bt), v(a, l, tp), v(c, rt, bt), v(bb, rt, tp));
        // the quad index buffer: 0 1 2, 3 2 1
        out.extend_from_slice(&[vd, va, vc, vb, vc, va]);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::Camera;
    use crate::draw_list::{Helpers, Light};
    use crate::material::{MaterialDesc, MaterialTable};
    use crate::mesh::MeshData;
    use glam::Vec2;

    fn setup() -> (Arc<Assets>, MeshHandle, MeshHandle, MeshHandle) {
        let a = Assets::headless(MaterialTable::default());
        let boxes =
            a.add_mesh("box", MeshData::cube(MaterialDesc::textured(EditorMaterial::Opaque, "crate")));
        let glass = a.add_mesh(
            "glass",
            MeshData::quad(MaterialDesc::textured(EditorMaterial::Blended, "glass"), Vec2::ONE),
        );
        let fence = a.add_mesh(
            "fence",
            MeshData::quad(MaterialDesc::textured(EditorMaterial::AlphaTest, "fence"), Vec2::ONE),
        );
        (a, boxes, glass, fence)
    }

    fn at(x: f32, z: f32) -> Mat4 {
        Mat4::from_translation(Vec3::new(x, 0.0, z))
    }

    fn view() -> View {
        View::new(Camera::looking(Vec3::new(0.0, 0.0, -10.0), Vec3::Z), [800, 600])
    }

    /// A seeded list: boxes on a line, glass at increasing depth, a fence, some behind the camera.
    fn seeded(boxes: MeshHandle, glass: MeshHandle, fence: MeshHandle) -> DrawList {
        let mut list = DrawList::new();
        let mut seed = 0x2545_f491u32;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            seed
        };
        for i in 0..40 {
            let x = (next() % 9) as f32 - 4.0;
            list.push(Instance::new(boxes, at(x, i as f32)).with_id(i + 1));
        }
        for i in 0..10 {
            list.push(Instance::new(glass, at(0.0, 5.0 + (next() % 50) as f32)).with_id(100 + i));
        }
        list.push(Instance::new(fence, at(1.0, 3.0)).with_id(200));
        list.push(Instance::new(fence, at(-1.0, 3.0)).with_id(201));
        // behind the camera
        for i in 0..5 {
            list.push(Instance::new(boxes, at(0.0, -30.0 - i as f32)).with_id(300 + i));
        }
        list
    }

    #[test]
    fn instances_are_culled_grouped_and_sorted() {
        let (a, boxes, glass, fence) = setup();
        let list = seeded(boxes, glass, fence);
        let p = build_packet(&list, &view(), &a);
        assert_eq!(p.stats.instances, 57);
        assert_eq!(p.stats.visible, 52);
        assert_eq!(p.stats.placeholders, 0);
        assert!(!p.waiting_on_assets());
        // every box in one instanced draw, both fences in another
        assert_eq!(p.opaque.len(), 1);
        assert_eq!(p.draws[p.opaque.start].instances.len(), 40);
        assert_eq!(p.alpha.len(), 1);
        assert_eq!(p.draws[p.alpha.start].instances.len(), 2);
        assert!(p.sky.is_empty());
        // glass sorted back to front: consecutive equal draws merge, the depths never rise
        let glass_inst: Vec<f32> = p.draws[p.blended.clone()]
            .iter()
            .flat_map(|d| d.instances.clone())
            .map(|i| p.instances[i as usize].model[3][2])
            .collect();
        assert_eq!(glass_inst.len(), 10);
        assert!(glass_inst.windows(2).all(|w| w[0] >= w[1]), "{glass_inst:?}");
        assert_eq!(p.stats.draws, p.draws.len());
        assert_eq!(p.stats.triangles, 40 * 12 + 2 * 2 + 10 * 2);
        // the ranges tile the draws in pass order
        assert_eq!(p.opaque.start, 0);
        assert_eq!(p.alpha.start, p.opaque.end);
        assert_eq!(p.sky.start, p.alpha.end);
        assert_eq!(p.blended.start, p.sky.end);
        assert_eq!(p.blended.end, p.draws.len());
        // the instance data matches its draw
        let ids: Vec<u32> = p.draws[p.alpha.clone()]
            .iter()
            .flat_map(|d| d.instances.clone())
            .map(|i| p.instances[i as usize].id)
            .collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&200) && ids.contains(&201));
    }

    #[test]
    fn the_callers_visibility_replaces_culling() {
        let (a, boxes, glass, fence) = setup();
        let list = seeded(boxes, glass, fence);
        let vis = Visibility { visible: vec![0, 1, 2, 52, 9999] };
        let p = build_packet_with(&list, &view(), &a, Some(&vis));
        // index 52 lies behind the camera but the caller said it is visible; 9999 does not exist
        assert_eq!(p.stats.visible, 4);
        assert_eq!(p.instances.len(), 4);
    }

    #[test]
    fn parallel_culling_gives_the_same_packet() {
        let (a, boxes, glass, fence) = setup();
        let mut list = seeded(boxes, glass, fence);
        for i in 0..3000 {
            list.push(Instance::new(boxes, at((i % 7) as f32, (i % 50) as f32)));
        }
        let p = build_packet(&list, &view(), &a);
        let q = build_packet(&list, &view(), &a);
        assert_eq!(p.instances, q.instances);
        assert_eq!(p.draws, q.draws);
        assert_eq!(p.stats.visible, 52 + 3000);
    }

    #[test]
    fn placeholders_overrides_and_flags() {
        let (a, boxes, _, _) = setup();
        let ghost = a.material(MaterialDesc::color(EditorMaterial::Blended, Color::rgba(255, 0, 0, 128)));
        let mut list = DrawList::new();
        list.push(
            Instance::new(boxes, at(0.0, 0.0)).with_material(ghost).with_flags(InstanceFlags::SELECTED),
        );
        let mut missing = Instance::new(a.object("not\\loaded"), at(2.0, 0.0));
        missing.bounds = Some(Aabb::new(Vec3::ZERO, Vec3::new(2.0, 4.0, 2.0)));
        list.push(missing);
        list.push(Instance::new(boxes, at(-2.0, 0.0)).with_flags(InstanceFlags::HIDDEN_IN_PREVIEW));
        let v = view();
        let p = build_packet(&list, &v, &a);
        assert_eq!(p.stats.placeholders, 1);
        // the override made the box blended
        assert_eq!(p.blended.len(), 1);
        assert!(p.id_pass);
        // the placeholder is scaled to the instance's bounds
        let ph =
            p.draws[p.opaque.clone()].iter().find(|d| p.meshes[d.mesh as usize] == a.placeholder()).unwrap();
        let m = Mat4::from_cols_array_2d(&p.instances[ph.instances.start as usize].model);
        assert!((m.transform_point3(Vec3::splat(0.5)) - Vec3::new(4.0, 4.0, 2.0)).abs().max_element() < 1e-5);
        // a preview picture leaves out what is hidden in it and draws no selection
        let pv = build_packet(&list, &v.as_preview(), &a);
        assert_eq!(pv.stats.visible, 2);
        assert!(!pv.id_pass);
        assert!(pv.helpers.lines.is_empty());
        // wireframe draws every part as edges in the opaque pass
        let wire = build_packet(&list, &View { mode: ViewMode::Wireframe, ..v }, &a);
        assert!(wire.draws.iter().all(|d| d.pipe == MeshPipe::Wire));
        assert!(wire.blended.is_empty());
    }

    #[test]
    fn helpers_lights_and_labels() {
        let (a, ..) = setup();
        let mut list = DrawList::new();
        let mut h = Helpers::default();
        h.label(Vec3::new(0.0, 0.0, 5.0), "ahead", Color::WHITE, true);
        h.label(Vec3::new(0.0, 0.0, -20.0), "behind", Color::WHITE, false);
        h.label(Vec3::new(1.0, 1.0, 5.0), "flat", Color::WHITE, false);
        h.line(Vec3::ZERO, Vec3::X, Color::WHITE, false);
        list.helpers = h;
        for i in 0..200 {
            list.lights.push(Light {
                kind: LightKind::Point,
                position: Vec3::new(0.0, 0.0, i as f32),
                range: 2.0,
                color: Vec3::ONE,
            });
        }
        list.environment = Some(crate::draw_list::Environment::default());
        let v = View { mode: ViewMode::GameLighting, ..view() };
        let p = build_packet(&list, &v, &a);
        assert_eq!(p.labels.len(), 2);
        assert_eq!(p.labels[0].query, Some(0));
        assert_eq!(p.labels[1].query, None);
        assert!(
            (p.labels[0].position[0] - 400.0).abs() < 1e-3 && (p.labels[0].position[1] - 300.0).abs() < 1e-3
        );
        assert_eq!(p.label_points.len(), 1);
        assert_eq!(p.lights.len(), MAX_LIGHTS);
        assert!(p.stats.lights_in_view > MAX_LIGHTS);
        // nearest first
        assert!(p.lights.windows(2).all(|w| w[0].pos_range[2] <= w[1].pos_range[2]));
        assert_eq!(p.frame.mode[0], 3);
        // 402 grid lines and the helper line
        assert_eq!(p.helpers.lines.len(), (201 * 2 * 2) as usize);
        assert_eq!(p.helpers.lines_overlay.len(), 2);
        // without an environment the game-lighting view is lit the studio way
        list.environment = None;
        let p = build_packet(&list, &v, &a);
        assert_eq!(p.frame.mode[0], 0);
        assert!(p.lights.is_empty());
    }

    fn sprite(at: Vec3, align: SpriteAlign) -> crate::draw_list::Sprite {
        crate::draw_list::Sprite {
            position: at,
            half: Vec2::new(1.0, 0.5),
            angle: 0.0,
            color: Color::rgba(255, 128, 0, 200),
            uv: [0.0, 0.25, 0.5, 0.75],
            align,
        }
    }

    #[test]
    fn sprites_face_the_camera_and_cull_by_winding() {
        let (a, ..) = setup();
        let mat = a.material(MaterialDesc::textured(EditorMaterial::Particle, "pfx\\spark"));
        let batch = |cull, align| SpriteBatch {
            material: mat,
            sprites: vec![sprite(Vec3::new(0.0, 0.0, 5.0), align)],
            cull,
            size: SpriteSize::World,
            bounds: None,
            id: 7,
            flags: InstanceFlags::NO_ID,
        };
        let mut list = DrawList::new();
        list.sprites.push(batch(SpriteCull::None, SpriteAlign::Camera));
        let p = build_packet(&list, &view(), &a);
        assert_eq!(p.dynamic.len(), 6);
        assert_eq!(p.stats.sprites, 1);
        let d = &p.draws[p.blended.clone()][0];
        assert_eq!(d.dynamic, Some(0..6));
        // the camera looks along +Z with Y up: d is the bottom left, a the top left, c the bottom
        // right, b the top right; uv as the atlas frame says
        let pos = |i: usize| Vec3::from_array(p.dynamic[i].position);
        assert!((pos(0) - Vec3::new(-1.0, -0.5, 5.0)).abs().max_element() < 1e-5);
        assert!((pos(1) - Vec3::new(-1.0, 0.5, 5.0)).abs().max_element() < 1e-5);
        assert!((pos(2) - Vec3::new(1.0, -0.5, 5.0)).abs().max_element() < 1e-5);
        assert!((pos(3) - Vec3::new(1.0, 0.5, 5.0)).abs().max_element() < 1e-5);
        assert_eq!(p.dynamic[0].uv, [0.0, 0.75]);
        assert_eq!(p.dynamic[3].uv, [0.5, 0.25]);
        assert_eq!(p.dynamic[0].color, [255, 128, 0, 200]);
        // a sprite facing the camera is clockwise: CULL_CW drops it, CULL_CCW keeps it
        let mut list = DrawList::new();
        list.sprites.push(batch(SpriteCull::Clockwise, SpriteAlign::Camera));
        list.sprites.push(batch(SpriteCull::CounterClockwise, SpriteAlign::Camera));
        let p = build_packet(&list, &view(), &a);
        assert_eq!(p.dynamic.len(), 6);
        // along a direction: its top is the direction
        let mut list = DrawList::new();
        list.sprites.push(batch(SpriteCull::None, SpriteAlign::Along(Vec3::X)));
        let p = build_packet(&list, &view(), &a);
        let top = Vec3::from_array(p.dynamic[1].position) - Vec3::from_array(p.dynamic[0].position);
        assert!((top.normalize() - Vec3::X).abs().max_element() < 1e-5);
    }

    #[test]
    fn a_glow_is_as_wide_as_the_engine_draws_it() {
        let (a, ..) = setup();
        let mat = a.material(MaterialDesc::textured(EditorMaterial::Additive, "glow\\glow_01"));
        let glow = |z: f32| SpriteBatch {
            material: mat,
            sprites: vec![crate::draw_list::Sprite { half: Vec2::splat(0.5), ..sprite(Vec3::new(0.0, 0.0, z), SpriteAlign::Camera) }],
            cull: SpriteCull::None,
            size: SpriteSize::Screen,
            bounds: None,
            id: 0,
            flags: InstanceFlags::NO_ID,
        };
        let v = view();
        let width_px = |z: f32| {
            let mut list = DrawList::new();
            list.sprites.push(glow(z));
            let p = build_packet(&list, &v, &a);
            let vp = v.camera.view_proj(v.size);
            let s = |i: usize| project(&vp, v.size, Vec3::from_array(p.dynamic[i].position)).unwrap();
            s(2).x - s(0).x
        };
        // width * radius / depth pixels each way (CTLSprite): the camera stands at z = -10, so
        // 2 * 800 * 0.5 / 15 across at z = 5 and a quarter of that at z = 50
        assert!((width_px(5.0) - 800.0 / 15.0).abs() < 0.05);
        assert!((width_px(50.0) - 800.0 / 60.0).abs() < 0.05);
    }

    #[test]
    fn batches_draw_every_copy_and_the_sky_its_box() {
        let (a, boxes, ..) = setup();
        let mut list = DrawList::new();
        let instances: Vec<crate::draw_list::BatchInstance> = (0..3000)
            .map(|i| crate::draw_list::BatchInstance {
                transform: at((i % 7) as f32, (i % 50) as f32),
                tint: Color::WHITE,
            })
            .collect();
        list.batches.push(crate::draw_list::Batch {
            mesh: boxes,
            material: None,
            instances,
            bounds: Aabb::new(Vec3::ZERO, Vec3::splat(50.0)),
            id: 0,
            flags: InstanceFlags::NO_ID,
        });
        list.environment = Some(crate::draw_list::Environment {
            sky: Some(Sky {
                textures: ["sky\\sky_9_cube".into(), "sky\\sky_10_cube".into()],
                color: Vec3::new(0.5, 0.25, 1.5),
                weight: 0.5,
                rotation: 1.0,
            }),
            ..crate::draw_list::Environment::default()
        });
        let v = View { mode: ViewMode::GameLighting, ..view() };
        let p = build_packet(&list, &v, &a);
        assert_eq!(p.stats.batched, 3000);
        // one draw for all the copies, none culled
        let d = &p.draws[p.opaque.clone()];
        assert!(d.iter().any(|d| d.instances.len() == 3000));
        // the sky: one draw in its pass with its colour (clamped) and blend weight
        assert_eq!(p.sky.len(), 1);
        let sky = &p.draws[p.sky.start];
        let inst = &p.instances[sky.instances.start as usize];
        assert_eq!(inst.color, Color::rgba(127, 63, 255, 127).to_f32());
        assert_ne!(inst.flags & InstanceFlags::SKY_TC.bits(), 0);
        // no sky outside game lighting
        let p = build_packet(&list, &view(), &a);
        assert!(p.sky.is_empty());
    }
}
