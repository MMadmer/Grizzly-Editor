//! The static wallmarks as `ESceneWallmarkTool::OnRender` draws them: each mark's triangles
//! with its slot's shader and texture, white, a little nearer than the wall, those the view sees
//! and that are big enough on the picture; a selected mark's box.
//!
//! The marks are read from the tool's chunk as `LoadStream` reads it, once per chunk; a mark is a
//! mesh of its own.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_chunk::Reader;
use gz_render::{Aabb, Assets, Color, Frustum, Instance, InstanceFlags, MaterialDesc, MeshData, MeshVertex, SubMesh};
use gz_scene::tools::WallmarkPart;
use gz_scene::{ObjClass, ObjId, SceneState};
use smol_str::SmolStr;

use crate::draw::Built;

/// `flDrawWallmark`.
const DRAW_WALLMARK: u32 = 1 << 0;
/// `wallmark::flSelected`.
const SELECTED: u8 = 1 << 0;
/// `MAX_WALLMARK_VERTEX_COUNT`: a mark with more is dropped when it is read.
const MAX_VERTICES: u32 = 8192;
/// `FVF::LIT`: position, colour, texture coordinates.
const LIT: usize = 24;

/// One mark as the file holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Mark {
    pub flags: u8,
    pub bbox: Aabb,
    pub sphere: (Vec3, f32),
    /// Triangles of `(position, uv)`.
    pub verts: Vec<(Vec3, Vec2)>,
}

/// A slot: the marks of one shader and texture.
#[derive(Clone, Debug, PartialEq)]
pub struct Slot {
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub marks: Vec<Mark>,
}

/// What the tool's chunk says.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Wallmarks {
    pub flags: u32,
    pub slots: Vec<Slot>,
}

fn vec3(r: &mut Reader<'_>) -> Option<Vec3> {
    Some(Vec3::new(r.r_f32().ok()?, r.r_f32().ok()?, r.r_f32().ok()?))
}

/// `LoadStream`: version 3 (no mark sizes) or 4, a mark of too many vertices dropped.
#[must_use]
pub fn read(bytes: &[u8]) -> Option<Wallmarks> {
    let r = Reader::new(bytes);
    let version = r.open_chunk(1).ok()??.reader().r_u16().ok()?;
    if version != 3 && version != 4 {
        return None;
    }
    let flags = r.open_chunk(2).ok()??.reader().r_u32().ok()?;
    let (items, sized) = match r.open_chunk(4).ok()? {
        Some(c) => (c, false),
        None => (r.open_chunk(5).ok()??, true),
    };
    let mut slots = Vec::new();
    let items = items.reader();
    for id in 0u32.. {
        let Ok(Some(c)) = items.open_chunk(id) else { break };
        let mut s = c.reader();
        let Ok(count) = s.r_u32() else { continue };
        if count == 0 {
            continue;
        }
        let (Ok(shader), Ok(texture)) = (s.r_stringz(), s.r_stringz()) else { continue };
        let mut marks = Vec::new();
        for _ in 0..count {
            let Ok(flags) = s.r_u8() else { break };
            let (Some(min), Some(max)) = (vec3(&mut s), vec3(&mut s)) else { break };
            let (Some(p), Ok(radius)) = (vec3(&mut s), s.r_f32()) else { break };
            if sized {
                // the mark's width, height and turn: what the editing tools made it with
                for _ in 0..3 {
                    if s.r_f32().is_err() {
                        break;
                    }
                }
            }
            let Ok(n) = s.r_u32() else { break };
            if (n as usize).saturating_mul(LIT) > s.elapsed() {
                break;
            }
            let Ok(raw) = s.take(n as usize * LIT) else { break };
            if n > MAX_VERTICES {
                continue;
            }
            let verts = raw
                .as_chunks::<LIT>()
                .0
                .iter()
                .map(|v| {
                    let f = |at: usize| {
                        let mut b = [0u8; 4];
                        b.copy_from_slice(v.get(at..at + 4).unwrap_or(&[0; 4]));
                        f32::from_le_bytes(b)
                    };
                    (Vec3::new(f(0), f(4), f(8)), Vec2::new(f(16), f(20)))
                })
                .collect();
            marks.push(Mark { flags, bbox: Aabb::new(min, max), sphere: (p, radius), verts });
        }
        slots.push(Slot { shader: shader.into(), texture: texture.into(), marks });
    }
    Some(Wallmarks { flags, slots })
}

/// A mark's mesh: its triangles in white (`OnRender` fills every vertex with 255, 255, 255, 255).
fn mark_mesh(m: &Mark, material: &MaterialDesc) -> Option<MeshData> {
    let tris = m.verts.len() / 3 * 3;
    if tris == 0 {
        return None;
    }
    let verts: Vec<MeshVertex> = m
        .verts
        .iter()
        .take(tris)
        .map(|(p, uv)| {
            let mut v = MeshVertex::new(*p, Vec3::Y, *uv);
            v.color = Color::WHITE.0;
            v
        })
        .collect();
    let n = verts.len() as u32;
    MeshData::new(verts, (0..n).collect(), vec![SubMesh { indices: 0..n, material: material.clone() }]).ok()
}

/// What the wallmark pass keeps: the marks read and their meshes, per chunk.
#[derive(Default)]
pub struct WallmarkCache {
    part: Option<usize>,
    marks: Wallmarks,
    meshes: Vec<(String, gz_render::MeshHandle, usize, usize)>,
}

impl WallmarkCache {
    fn clear(&mut self, assets: &Assets) {
        for (name, ..) in self.meshes.drain(..) {
            assets.remove_mesh(&name);
        }
        self.marks = Wallmarks::default();
        self.part = None;
    }

    /// Adds the marks the view sees: `ssa_clip` is `r_ssaDISCARD / 4`.
    pub(crate) fn draw(
        &mut self,
        state: &SceneState,
        assets: &Assets,
        frustum: &Frustum,
        eye: Vec3,
        ssa_clip: f32,
        built: &mut Built,
    ) {
        let part: Option<&Arc<WallmarkPart>> =
            state.bulk.wallmarks.as_ref().filter(|_| state.tools.part(ObjClass::Wallmark).visible);
        let Some(part) = part else {
            self.clear(assets);
            return;
        };
        let key = Arc::as_ptr(part) as usize;
        if self.part != Some(key) {
            self.clear(assets);
            self.part = Some(key);
            self.marks = read(&part.raw).unwrap_or_default();
            let table = assets.table();
            for (si, slot) in self.marks.slots.iter().enumerate() {
                let material = MaterialDesc::textured(table.classify(&slot.shader), &slot.texture);
                for (mi, m) in slot.marks.iter().enumerate() {
                    if let Some(mesh) = mark_mesh(m, &material) {
                        let name = format!("<wallmark>/{si}/{mi}");
                        let h = assets.add_mesh(&name, mesh);
                        self.meshes.push((name, h, si, mi));
                    }
                }
            }
        }
        if self.marks.flags & DRAW_WALLMARK == 0 {
            return;
        }
        for (_, mesh, si, mi) in &self.meshes {
            let Some(m) = self.marks.slots.get(*si).and_then(|s| s.marks.get(*mi)) else { continue };
            let (c, r) = m.sphere;
            if !frustum.intersects_sphere(c, r) {
                continue;
            }
            let ssa = r * r / eye.distance_squared(c).max(1e-6);
            if ssa < ssa_clip {
                continue;
            }
            let mut inst = Instance::new(*mesh, glam::Mat4::IDENTITY)
                .with_flags(InstanceFlags::NO_ID | InstanceFlags::DECAL | InstanceFlags::TWO_SIDED);
            inst.bounds = Some(m.bbox);
            built.list.instances.push(inst);
            built.owners.push(ObjId(0));
        }
        for slot in &self.marks.slots {
            for m in slot.marks.iter().filter(|m| m.flags & SELECTED != 0) {
                let (c, r) = m.sphere;
                if frustum.intersects_sphere(c, r) {
                    built.list.helpers.shape(
                        gz_render::ShapeKind::SelectionBox,
                        m.bbox.unit_transform(),
                        None,
                        Some(Color::WHITE),
                        true,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gz_chunk::Writer;

    fn chunk(version: u16, sized: bool) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(1, |w| w.w_u16(version));
        w.chunk(2, |w| w.w_u32(DRAW_WALLMARK));
        w.chunk(3, |w| {
            w.w_f32(1.0);
            w.w_f32(1.0);
            w.w_f32(0.0);
            w.w_stringz("effects\\wallmarkmult");
            w.w_stringz("");
        });
        w.chunk(if sized { 5 } else { 4 }, |w| {
            w.chunk(0, |w| {
                w.w_u32(1);
                w.w_stringz("effects\\wallmarkmult");
                w.w_stringz("wm\\wm_blood_1");
                w.w_u8(SELECTED);
                w.w_vec3(Vec3::ZERO);
                w.w_vec3(Vec3::ONE);
                w.w_vec3(Vec3::splat(0.5));
                w.w_f32(0.87);
                if sized {
                    w.w_f32(1.0);
                    w.w_f32(1.0);
                    w.w_f32(0.0);
                }
                w.w_u32(3);
                for (p, uv) in [(Vec3::ZERO, Vec2::ZERO), (Vec3::Y, Vec2::Y), (Vec3::X, Vec2::X)] {
                    w.w_vec3(p);
                    w.w_u32(0xff80_8080);
                    w.w_f32(uv.x);
                    w.w_f32(uv.y);
                }
            });
            w.chunk(1, |w| w.w_u32(0));
        });
        w.into_bytes()
    }

    #[test]
    fn both_versions_read() {
        for (version, sized) in [(4, true), (3, false)] {
            let wm = read(&chunk(version, sized)).unwrap();
            assert_eq!(wm.flags, DRAW_WALLMARK);
            assert_eq!(wm.slots.len(), 1);
            let m = &wm.slots[0].marks[0];
            assert_eq!(m.flags, SELECTED);
            assert_eq!(m.verts[1], (Vec3::Y, Vec2::Y));
            assert!((m.sphere.1 - 0.87).abs() < 1e-6);
        }
        assert!(read(&WallmarkPart::empty_stream()).is_some_and(|w| w.slots.is_empty()));
        assert!(read(&[1, 2, 3]).is_none());
    }

    #[test]
    fn a_mark_is_white_triangles() {
        let wm = read(&chunk(4, true)).unwrap();
        let mat = MaterialDesc::new(gz_render::EditorMaterial::Multiply2x);
        let mesh = mark_mesh(&wm.slots[0].marks[0], &mat).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        assert!(mesh.vertices.iter().all(|v| v.color == [255; 4]));
    }
}
