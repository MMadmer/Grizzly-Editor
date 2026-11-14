//! A mesh of a source model: reading, writing and modifying it.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};
use rayon::prelude::*;
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::bone::Bone;
use crate::io::{
    Text, TextEncoding, checked_count, chunk, chunk_u32, need_chunk, r_f32s, r_i32s, r_vec3s, w_str,
};
use crate::object::Surface;
use crate::xmath::{Aabb, EPS_S, magnitude, mknormal};
use crate::{ObjectError, Result};

const MESH_VERSION: u32 = 0x0011;
const CHUNK_VERSION: u32 = 0x1000;
const CHUNK_MESHNAME: u32 = 0x1001;
const CHUNK_FLAGS: u32 = 0x1002;
const CHUNK_BBOX: u32 = 0x1004;
const CHUNK_VERTS: u32 = 0x1005;
const CHUNK_FACES: u32 = 0x1006;
const CHUNK_VMAPS_0: u32 = 0x1007;
const CHUNK_VMREFS: u32 = 0x1008;
const CHUNK_SFACE: u32 = 0x1009;
const CHUNK_BOP: u32 = 0x1010;
const CHUNK_VMAPS_1: u32 = 0x1011;
const CHUNK_VMAPS_2: u32 = 0x1012;
const CHUNK_SG: u32 = 0x1013;

/// "No vertex map": what an unused reference slot holds.
pub const NO_INDEX: u32 = u32::MAX;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct MeshFlags: u8 {
        const VISIBLE = 1 << 0;
        const LOCKED = 1 << 1;
        /// The smoothing data came from 3ds Max smoothing groups.
        const SG_MASK = 1 << 2;
    }
}

/// One corner of a face.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FaceVert {
    /// Index into [`Mesh::points`].
    pub point: u32,
    /// Index into [`Mesh::vmrefs`].
    pub vmref: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Face {
    pub v: [FaceVert; 3],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum VMapKind {
    Uv = 0,
    Weight = 1,
}

/// `st_VMap`: a UV map (two floats per entry) or a weight map (one).
#[derive(Clone, Debug, PartialEq)]
pub struct VMap {
    pub name: SmolStr,
    pub kind: VMapKind,
    /// A per-face-corner map: `pindices` names the face of every entry.
    pub polymap: bool,
    pub values: Vec<f32>,
    /// The point every entry belongs to.
    pub vindices: Vec<u32>,
    pub pindices: Vec<u32>,
}

impl VMap {
    #[must_use]
    pub fn dim(&self) -> usize {
        match self.kind {
            VMapKind::Uv => 2,
            VMapKind::Weight => 1,
        }
    }

    /// Entries in the map.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len() / self.dim()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    #[must_use]
    pub fn uv(&self, index: u32) -> Option<Vec2> {
        let i = (index as usize).checked_mul(2)?;
        match (self.values.get(i), self.values.get(i + 1)) {
            (Some(u), Some(v)) => Some(Vec2::new(*u, *v)),
            _ => None,
        }
    }

    #[must_use]
    pub fn weight(&self, index: u32) -> Option<f32> {
        self.values.get(index as usize).copied()
    }
}

/// `st_VMapPt`: an entry of a vertex map.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct VMapPt {
    /// Index into [`Mesh::vmaps`].
    pub vmap: u32,
    /// Entry of that map.
    pub index: u32,
}

/// The faces of a mesh that wear one surface.
#[derive(Clone, Debug, PartialEq)]
pub struct SurfFaces {
    /// Index into the object's surfaces.
    pub surface: u32,
    /// Face indices, ascending.
    pub faces: Arc<[u32]>,
}

/// The skin vertex's bone.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BoneWeight {
    pub id: u16,
    pub w: f32,
}

/// `st_SVert`: a face corner ready for skinning.
#[derive(Clone, Debug, PartialEq)]
pub struct SkinVertex {
    pub offs: Vec3,
    pub norm: Vec3,
    pub uv: Vec2,
    pub bones: SmallVec<[BoneWeight; 4]>,
}

pub type VMapRefs = SmallVec<[VMapPt; 2]>;

/// A mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct Mesh {
    pub name: SmolStr,
    pub flags: MeshFlags,
    /// Two reserved numbers the file carries.
    pub options: [i32; 2],
    /// The box as stored; [`Mesh::recompute_bbox`] refreshes it.
    pub bbox: Aabb,
    pub points: Arc<[Vec3]>,
    pub faces: Arc<[Face]>,
    /// Per face: bits 0-2 mark a hard edge, bit 3 a back face (`face_smoth_flags.h`).
    pub smooth_groups: Arc<[u32]>,
    pub vmaps: Vec<VMap>,
    /// Per reference: the vertex map entries of a face corner (a UV, then weights).
    pub vmrefs: Vec<VMapRefs>,
    /// In the order of the file; exporters walk it in this order.
    pub surf_faces: Vec<SurfFaces>,
}

impl Default for Mesh {
    fn default() -> Self {
        Self {
            name: SmolStr::default(),
            flags: MeshFlags::VISIBLE,
            options: [0, 0],
            bbox: Aabb::ZERO,
            points: Arc::from([]),
            faces: Arc::from([]),
            smooth_groups: Arc::from([]),
            vmaps: Vec::new(),
            vmrefs: Vec::new(),
            surf_faces: Vec::new(),
        }
    }
}

/// A vertex map as a file holds it, before it is rebuilt.
struct RawVMap {
    name: SmolStr,
    dim: usize,
    kind: u8,
    values: Vec<f32>,
}

fn bad(mesh: &str, what: impl std::fmt::Display) -> ObjectError {
    ObjectError::Invalid(format!("Mesh '{mesh}': {what}."))
}

impl Mesh {
    /// Loads a mesh. `surfaces` are the object's: a face list names its surface.
    pub(crate) fn read(r: &Reader<'_>, surfaces: &[Surface], text: &mut Text) -> Result<Mesh> {
        let version = chunk_u32(r, CHUNK_VERSION).ok_or(gz_chunk::ChunkError::Missing(CHUNK_VERSION))?;
        if version != MESH_VERSION {
            return Err(ObjectError::Version { what: "The mesh", version });
        }
        let name = text.r_str(&mut need_chunk(r, CHUNK_MESHNAME)?)?;

        let mut c = need_chunk(r, CHUNK_BBOX)?;
        let bbox = Aabb { min: c.r_vec3()?, max: c.r_vec3()? };
        let flags = MeshFlags::from_bits_retain(need_chunk(r, CHUNK_FLAGS)?.r_u8()?);
        let mut options = [0i32; 2];
        if let Some(mut c) = chunk(r, CHUNK_BOP) {
            options = [c.r_i32()?, c.r_i32()?];
        }

        let mut c = need_chunk(r, CHUNK_VERTS)?;
        let vert_count = c.r_u32()? as usize;
        if vert_count < 3 {
            return Err(bad(&name, "it has fewer than three vertices"));
        }
        checked_count(&c, vert_count, 12)?;
        let points = r_vec3s(&mut c, vert_count)?;

        let mut c = need_chunk(r, CHUNK_FACES)?;
        let face_count = c.r_u32()? as usize;
        if face_count == 0 {
            return Err(bad(&name, "it has no faces"));
        }
        checked_count(&c, face_count, 24)?;
        let raw = r_i32s(&mut c, face_count * 6)?;

        let default_sg = if flags.contains(MeshFlags::SG_MASK) { 0 } else { u32::MAX };
        let mut smooth_groups = vec![default_sg; face_count];
        if let Some(mut c) = chunk(r, CHUNK_SG) {
            checked_count(&c, face_count, 4)?;
            for (dst, src) in smooth_groups.iter_mut().zip(r_i32s(&mut c, face_count)?) {
                *dst = src as u32;
            }
        }

        let mut c = need_chunk(r, CHUNK_VMREFS)?;
        let ref_count = checked_count(&c, c.clone().r_u32()? as usize, 1)?;
        c.advance(4)?;
        let mut vmrefs: Vec<VMapRefs> = Vec::with_capacity(ref_count);
        for _ in 0..ref_count {
            let n = usize::from(c.r_u8()?);
            checked_count(&c, n, 8)?;
            let mut pts = VMapRefs::with_capacity(n);
            for _ in 0..n {
                pts.push(VMapPt { vmap: c.r_u32()?, index: c.r_u32()? });
            }
            vmrefs.push(pts);
        }

        let mut faces = Vec::with_capacity(face_count);
        for f in raw.as_chunks::<6>().0 {
            let mut face = Face::default();
            for k in 0..3 {
                let (p, vr) = (f[k * 2], f[k * 2 + 1]);
                if p < 0 || p as usize >= vert_count || vr < 0 || vr as usize >= ref_count {
                    return Err(bad(
                        &name,
                        "a face points outside the vertices or the vertex map references",
                    ));
                }
                face.v[k] = FaceVert { point: p as u32, vmref: vr as u32 };
            }
            faces.push(face);
        }

        let mut c = need_chunk(r, CHUNK_SFACE)?;
        let surf_count = c.r_u16()?;
        let mut surf_faces: Vec<SurfFaces> = Vec::with_capacity(usize::from(surf_count).min(surfaces.len()));
        for _ in 0..surf_count {
            let surf_name = text.r_str(&mut c)?;
            let Some(surface) = surfaces.iter().position(|s| s.name.eq_ignore_ascii_case(&surf_name)) else {
                return Err(bad(
                    &name,
                    format_args!("its faces wear surface '{surf_name}', which the object lacks"),
                ));
            };
            let n = c.r_u32()? as usize;
            if n == 0 {
                return Err(bad(&name, format_args!("surface '{surf_name}' has no faces")));
            }
            checked_count(&c, n, 4)?;
            let mut list = Vec::with_capacity(n);
            for id in r_i32s(&mut c, n)? {
                if id < 0 || id as usize >= face_count {
                    return Err(bad(
                        &name,
                        format_args!("surface '{surf_name}' lists a face the mesh lacks"),
                    ));
                }
                list.push(id as u32);
            }
            list.sort_unstable();
            let entry = SurfFaces { surface: surface as u32, faces: Arc::from(list) };
            // a surface named twice keeps its last list, as the X-Ray SDK's map does
            match surf_faces.iter_mut().find(|s| s.surface == entry.surface) {
                Some(old) => *old = entry,
                None => surf_faces.push(entry),
            }
        }
        // the X-Ray SDK keeps these lists in a map keyed by the surface's address, so its
        // order is whatever the allocator gave; the file's order is kept here, which makes a
        // read and a write of a file the X-Ray SDK saved identical

        let (raw_vmaps, old_form) = Self::read_vmaps(r, text, &name)?;
        let mut mesh = Mesh {
            name,
            flags,
            options,
            bbox,
            points: Arc::from(points),
            faces: Arc::from(faces),
            smooth_groups: Arc::from(smooth_groups),
            vmaps: Vec::new(),
            vmrefs,
            surf_faces,
        };
        mesh.rebuild_from_raw(&raw_vmaps)?;
        if old_form {
            // the X-Ray SDK rebuilds twice on this path
            mesh.rebuild_vmaps()?;
        }
        Ok(mesh)
    }

    fn read_vmaps(r: &Reader<'_>, text: &mut Text, mesh: &str) -> Result<(Vec<RawVMap>, bool)> {
        let mut out = Vec::new();
        let (mut c, form) = if let Some(c) = chunk(r, CHUNK_VMAPS_2) {
            (c, 2)
        } else if let Some(c) = chunk(r, CHUNK_VMAPS_1) {
            (c, 1)
        } else {
            (need_chunk(r, CHUNK_VMAPS_0)?, 0)
        };
        let count = checked_count(&c, c.clone().r_u32()? as usize, 5)?;
        c.advance(4)?;
        for _ in 0..count {
            let name = text.r_str(&mut c)?;
            let (dim, polymap, kind) = match form {
                // the X-Ray SDK keeps two bits of the dimension and of the type, one of the flag
                2 => (c.r_u8()? & 3, c.r_u8()? & 1 != 0, c.r_u8()? & 3),
                1 => (c.r_u8()? & 3, false, c.r_u8()? & 3),
                _ => (2, false, 0),
            };
            let dim = usize::from(dim);
            let size = c.r_u32()? as usize;
            if kind > 1 || dim == 0 || (kind == 0 && dim != 2) {
                return Err(bad(mesh, format_args!("vertex map '{name}' has an unknown layout")));
            }
            checked_count(&c, size, dim * 4)?;
            let values = r_f32s(&mut c, size * dim)?;
            if form == 2 {
                // point and face indices: rebuilt below, so only stepped over
                checked_count(&c, size, 4)?;
                c.advance(size * 4)?;
                if polymap {
                    checked_count(&c, size, 4)?;
                    c.advance(size * 4)?;
                }
            }
            out.push(RawVMap { name, dim, kind, values });
        }
        Ok((out, form != 2))
    }

    /// Rebuilds the vertex maps over maps as a file holds them.
    fn rebuild_from_raw(&mut self, raw: &[RawVMap]) -> Result<()> {
        let source = |pt: VMapPt| -> Option<(&RawVMap, [f32; 2])> {
            let map = raw.get(pt.vmap as usize)?;
            let at = (pt.index as usize).checked_mul(if map.kind == 0 { map.dim } else { 1 })?;
            let a = *map.values.get(at)?;
            let b = if map.kind == 0 { *map.values.get(at + 1)? } else { 0.0 };
            Some((map, [a, b]))
        };
        self.rebuild_with(|pt| source(pt).map(|(m, v)| (m.name.clone(), m.kind == 0, v)))
    }

    /// Lays the vertex maps out again - one point map per name for the first
    /// corner that reaches a vertex, a polygon map for every further corner, one map per
    /// weight name - and points every reference at its new entry.
    pub fn rebuild_vmaps(&mut self) -> Result<()> {
        let old = std::mem::take(&mut self.vmaps);
        let res = self.rebuild_with(|pt| {
            let map = old.get(pt.vmap as usize)?;
            match map.kind {
                VMapKind::Uv => map.uv(pt.index).map(|uv| (map.name.clone(), true, [uv.x, uv.y])),
                VMapKind::Weight => map.weight(pt.index).map(|w| (map.name.clone(), false, [w, 0.0])),
            }
        });
        if res.is_err() {
            self.vmaps = old;
        }
        res
    }

    /// The walk of the vertex-map rebuild; `source` gives the name, the kind (UV or weight) and the
    /// value of an old entry.
    fn rebuild_with(&mut self, source: impl Fn(VMapPt) -> Option<(SmolStr, bool, [f32; 2])>) -> Result<()> {
        let mut vert_claimed = vec![false; self.points.len()];
        let mut maps: Vec<VMap> = Vec::new();
        let mut refs: Vec<VMapRefs> = self
            .vmrefs
            .iter()
            .map(|r| r.iter().map(|_| VMapPt { vmap: NO_INDEX, index: NO_INDEX }).collect())
            .collect();

        let find = |maps: &mut Vec<VMap>, name: &SmolStr, kind: VMapKind, polymap: bool| -> usize {
            match maps
                .iter()
                .position(|m| m.kind == kind && m.polymap == polymap && m.name.eq_ignore_ascii_case(name))
            {
                Some(i) => i,
                None => {
                    maps.push(VMap {
                        name: name.clone(),
                        kind,
                        polymap,
                        values: Vec::new(),
                        vindices: Vec::new(),
                        pindices: Vec::new(),
                    });
                    maps.len() - 1
                }
            }
        };

        for (f_id, face) in self.faces.iter().enumerate() {
            for fv in face.v {
                let (Some(old), Some(new)) =
                    (self.vmrefs.get(fv.vmref as usize), refs.get_mut(fv.vmref as usize))
                else {
                    return Err(bad(&self.name, "a face points outside the vertex map references"));
                };
                let Some(claimed) = vert_claimed.get_mut(fv.point as usize) else {
                    return Err(bad(&self.name, "a face points outside the vertices"));
                };
                for (pt_id, pt) in old.iter().enumerate() {
                    let Some((name, is_uv, value)) = source(*pt) else {
                        return Err(bad(&self.name, "a vertex map reference points outside its map"));
                    };
                    let idx = if is_uv {
                        let polymap = *claimed;
                        *claimed = true;
                        let idx = find(&mut maps, &name, VMapKind::Uv, polymap);
                        let m = &mut maps[idx];
                        m.values.push(value[0]);
                        m.values.push(value[1]);
                        m.vindices.push(fv.point);
                        if polymap {
                            m.pindices.push(f_id as u32);
                        }
                        idx
                    } else {
                        let idx = find(&mut maps, &name, VMapKind::Weight, false);
                        let m = &mut maps[idx];
                        m.values.push(value[0]);
                        m.vindices.push(fv.point);
                        idx
                    };
                    new[pt_id] = VMapPt { vmap: idx as u32, index: (maps[idx].len() - 1) as u32 };
                }
            }
        }
        self.vmaps = maps;
        self.vmrefs = refs;
        Ok(())
    }

    /// Saves a mesh. `surfaces` are the object's: a face list is stored under its surface's name.
    pub(crate) fn write(&self, w: &mut Writer, surfaces: &[Surface], enc: TextEncoding) {
        w.chunk(CHUNK_VERSION, |w| w.w_u16(MESH_VERSION as u16));
        w.chunk(CHUNK_MESHNAME, |w| w_str(w, &self.name, enc));
        w.chunk(CHUNK_BBOX, |w| {
            w.w_vec3(self.bbox.min);
            w.w_vec3(self.bbox.max);
        });
        w.chunk(CHUNK_FLAGS, |w| w.w_u8(self.flags.bits()));
        w.chunk(CHUNK_BOP, |w| {
            w.w_i32(self.options[0]);
            w.w_i32(self.options[1]);
        });
        w.chunk(CHUNK_VERTS, |w| {
            w.w_u32(self.points.len() as u32);
            for p in self.points.iter() {
                w.w_vec3(*p);
            }
        });
        w.chunk(CHUNK_FACES, |w| {
            w.w_u32(self.faces.len() as u32);
            for f in self.faces.iter() {
                for fv in f.v {
                    w.w_u32(fv.point);
                    w.w_u32(fv.vmref);
                }
            }
        });
        w.chunk(CHUNK_SG, |w| {
            for sg in self.smooth_groups.iter() {
                w.w_u32(*sg);
            }
        });
        w.chunk(CHUNK_VMREFS, |w| {
            w.w_u32(self.vmrefs.len() as u32);
            for pts in &self.vmrefs {
                w.w_u8(pts.len() as u8);
                for pt in pts {
                    w.w_u32(pt.vmap);
                    w.w_u32(pt.index);
                }
            }
        });
        w.chunk(CHUNK_SFACE, |w| {
            w.w_u16(self.surf_faces.len() as u16);
            for sf in &self.surf_faces {
                w_str(w, surfaces.get(sf.surface as usize).map_or("", |s| s.name.as_str()), enc);
                w.w_u32(sf.faces.len() as u32);
                for f in sf.faces.iter() {
                    w.w_u32(*f);
                }
            }
        });
        w.chunk(CHUNK_VMAPS_2, |w| {
            w.w_u32(self.vmaps.len() as u32);
            for m in &self.vmaps {
                w_str(w, &m.name, enc);
                w.w_u8(m.dim() as u8);
                w.w_u8(u8::from(m.polymap));
                w.w_u8(m.kind as u8);
                w.w_u32(m.len() as u32);
                for v in &m.values {
                    w.w_f32(*v);
                }
                for v in &m.vindices {
                    w.w_u32(*v);
                }
                if m.polymap {
                    for v in &m.pindices {
                        w.w_u32(*v);
                    }
                }
            }
        });
    }

    /// Recomputes the bounding box.
    pub fn recompute_bbox(&mut self) {
        let Some(first) = self.points.first() else {
            self.bbox = Aabb::ZERO;
            return;
        };
        let mut b = Aabb { min: *first, max: *first };
        for p in self.points.iter().skip(1) {
            b.modify(*p);
        }
        self.bbox = b;
    }

    fn point(&self, i: u32) -> Vec3 {
        self.points.get(i as usize).copied().unwrap_or(Vec3::ZERO)
    }

    /// One normal per face.
    #[must_use]
    pub fn face_normals(&self) -> Vec<Vec3> {
        self.faces
            .iter()
            .map(|f| mknormal(self.point(f.v[0].point), self.point(f.v[1].point), self.point(f.v[2].point)))
            .collect()
    }

    /// Three normals per face, one per corner. A corner's normal is the
    /// sum of the normals of the faces around its vertex that it reaches over soft edges.
    #[must_use]
    pub fn vertex_normals(&self) -> Vec<Vec3> {
        let fnormals = self.face_normals();
        let face_count = self.faces.len();
        let default_sg = u32::MAX;

        // GenerateAdjacency: the faces around every vertex, in face order
        let mut offsets = vec![0u32; self.points.len() + 1];
        for f in self.faces.iter() {
            for fv in f.v {
                if let Some(o) = offsets.get_mut(fv.point as usize + 1) {
                    *o += 1;
                }
            }
        }
        for i in 1..offsets.len() {
            offsets[i] += offsets[i - 1];
        }
        let mut fill = offsets.clone();
        let mut adjacent = vec![0u32; face_count * 3];
        for (f_id, f) in self.faces.iter().enumerate() {
            for fv in f.v {
                if let Some(at) = fill.get_mut(fv.point as usize) {
                    if let Some(slot) = adjacent.get_mut(*at as usize) {
                        *slot = f_id as u32;
                    }
                    *at += 1;
                }
            }
        }

        let sg = |f: u32| self.smooth_groups.get(f as usize).copied().unwrap_or(default_sg);
        let mut out = vec![Vec3::ZERO; face_count * 3];
        out.par_chunks_mut(3).enumerate().for_each_init(
            || (Vec::<bool>::new(), Vec::<(u32, u32)>::new()),
            |(processed, stack), (f_id, corners)| {
                let Some(face) = self.faces.get(f_id) else { return };
                for (k, n_out) in corners.iter_mut().enumerate() {
                    let p = face.v[k].point as usize;
                    let (Some(a), Some(b)) = (offsets.get(p), offsets.get(p + 1)) else { continue };
                    let Some(list) = adjacent.get(*a as usize..*b as usize) else { continue };
                    let Some(first) = list.first() else { continue };
                    let fallback = fnormals.get(*first as usize).copied().unwrap_or(Vec3::ZERO);
                    let Some(start) = list.iter().position(|f| *f as usize == f_id) else {
                        *n_out = fallback;
                        continue;
                    };

                    processed.clear();
                    processed.resize(list.len(), false);
                    stack.clear();
                    let mut n = Vec3::ZERO;
                    // RecurseTri, without the recursion
                    stack.push((start as u32, 0u32));
                    while let Some(top) = stack.last_mut() {
                        let (from, test) = *top;
                        if test as usize >= list.len() {
                            stack.pop();
                            continue;
                        }
                        top.1 += 1;
                        if processed[test as usize] {
                            continue;
                        }
                        let (f_from, f_test) = (list[from as usize], list[test as usize]);
                        let (Some(face_a), Some(face_b)) =
                            (self.faces.get(f_from as usize), self.faces.get(f_test as usize))
                        else {
                            continue;
                        };
                        let Some((ea, eb)) = same_edge(face_a, face_b) else { continue };
                        if f_from == f_test || connects(sg(f_from), sg(f_test), ea, eb) {
                            processed[test as usize] = true;
                            let fnrm = fnormals.get(f_test as usize).copied().unwrap_or(Vec3::ZERO);
                            n.x += fnrm.x;
                            n.y += fnrm.y;
                            n.z += fnrm.z;
                            stack.push((test, 0));
                        }
                    }
                    let len = magnitude(n);
                    *n_out = if len > EPS_S { Vec3::new(n.x / len, n.y / len, n.z / len) } else { fallback };
                }
            },
        );
        out
    }

    /// The UVs of a face, from the first vertex map entry of every corner.
    #[must_use]
    pub fn face_uv(&self, face: usize) -> Option<[Vec2; 3]> {
        let f = self.faces.get(face)?;
        let mut out = [Vec2::ZERO; 3];
        for (k, fv) in f.v.iter().enumerate() {
            let pt = self.vmrefs.get(fv.vmref as usize)?.first()?;
            let map = self.vmaps.get(pt.vmap as usize)?;
            if map.kind != VMapKind::Uv {
                return None;
            }
            out[k] = map.uv(pt.index)?;
        }
        Some(out)
    }

    /// The first UV entry of a face corner: what both exporters take as the texture coordinate.
    #[must_use]
    pub fn corner_uv(&self, fv: FaceVert) -> Option<Vec2> {
        for pt in self.vmrefs.get(fv.vmref as usize)? {
            let map = self.vmaps.get(pt.vmap as usize)?;
            if map.kind == VMapKind::Uv {
                return map.uv(pt.index);
            }
        }
        None
    }

    /// Three skin vertices per face. `normals` are [`Mesh::vertex_normals`];
    /// `influence` is the most bones a vertex keeps (1 to 4).
    pub fn skin_vertices(&self, bones: &[Bone], influence: u32, normals: &[Vec3]) -> Result<Vec<SkinVertex>> {
        let influence = influence.clamp(1, 4) as usize;
        // the bone of every weight map, resolved once
        let map_bone: Vec<Option<u16>> = self
            .vmaps
            .iter()
            .map(|m| match m.kind {
                VMapKind::Weight => {
                    bones.iter().position(|b| b.wmap == m.name).and_then(|i| u16::try_from(i).ok())
                }
                VMapKind::Uv => None,
            })
            .collect();

        let mut out = Vec::with_capacity(self.faces.len() * 3);
        for (f_id, face) in self.faces.iter().enumerate() {
            for (k, fv) in face.v.iter().enumerate() {
                let mut sv = SkinVertex {
                    offs: self.point(fv.point),
                    norm: normals.get(f_id * 3 + k).copied().unwrap_or(Vec3::ZERO),
                    uv: Vec2::ZERO,
                    bones: SmallVec::new(),
                };
                let mut wb: SmallVec<[BoneWeight; 8]> = SmallVec::new();
                let pts = self.vmrefs.get(fv.vmref as usize).map(|r| r.as_slice()).unwrap_or(&[]);
                for pt in pts {
                    let Some(map) = self.vmaps.get(pt.vmap as usize) else { continue };
                    match map.kind {
                        VMapKind::Weight => {
                            let Some(id) = map_bone.get(pt.vmap as usize).copied().flatten() else {
                                return Err(bad(
                                    &self.name,
                                    format_args!("no bone is bound to weight map '{}'", map.name),
                                ));
                            };
                            wb.push(BoneWeight { id, w: map.weight(pt.index).unwrap_or(0.0) });
                        }
                        VMapKind::Uv => {
                            if let Some(uv) = map.uv(pt.index) {
                                sv.uv = uv;
                            }
                        }
                    }
                }
                // prepare_weights: the heaviest bones, weights brought to a sum of one
                wb.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap_or(std::cmp::Ordering::Equal));
                wb.truncate(influence);
                let mut sum = 0.0f32;
                for b in &wb {
                    sum += b.w;
                }
                for b in &mut wb {
                    b.w /= sum;
                }
                sv.bones.extend(wb);
                out.push(sv);
            }
        }
        Ok(out)
    }
}

/// `has_same_edge`: the first pair of edges of two faces that join the same two points.
fn same_edge(a: &Face, b: &Face) -> Option<(u32, u32)> {
    let edge = |f: &Face, e: usize| {
        let (p, q) = (f.v[e].point, f.v[(e + 1) % 3].point);
        if p > q { (q, p) } else { (p, q) }
    };
    for e in 0..3 {
        let ea = edge(a, e);
        for r in 0..3 {
            if ea == edge(b, r) {
                return Some((e as u32, r as u32));
            }
        }
    }
    None
}

const BACKFACE: u32 = 1 << 3;

/// `is_soft_edge`.
fn soft_edge(flags: u32, edge: u32) -> bool {
    let edge = if flags & BACKFACE != 0 { (4 - edge) % 3 } else { edge };
    flags & (1 << edge) == 0
}

/// `do_connect_faces_by_faces_edge_flags`: two faces smooth over a shared edge when both face
/// the same way and both hold the edge soft.
fn connects(start: u32, test: u32, start_edge: u32, test_edge: u32) -> bool {
    if (start & BACKFACE != 0) != (test & BACKFACE != 0) {
        return false;
    }
    soft_edge(start, start_edge) && soft_edge(test, test_edge)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Two triangles sharing an edge, bent 90 degrees: (0,1,2) in the floor, (1,3,2) up the wall.
    pub(crate) fn bent_quad(sg: [u32; 2]) -> Mesh {
        let points = vec![
            Vec3::new(0.0, 0.0, 0.0),
            Vec3::new(0.0, 0.0, 1.0),
            Vec3::new(1.0, 0.0, 0.0),
            Vec3::new(1.0, 1.0, 1.0),
        ];
        let fv = |point, vmref| FaceVert { point, vmref };
        let faces =
            vec![Face { v: [fv(0, 0), fv(1, 1), fv(2, 2)] }, Face { v: [fv(1, 1), fv(3, 3), fv(2, 2)] }];
        let uv = VMap {
            name: "Texture".into(),
            kind: VMapKind::Uv,
            polymap: false,
            values: vec![0.0, 0.0, 0.0, 1.0, 1.0, 0.0, 1.0, 1.0],
            vindices: vec![0, 1, 2, 3],
            pindices: vec![],
        };
        let mut m = Mesh {
            name: "quad".into(),
            points: Arc::from(points),
            faces: Arc::from(faces),
            smooth_groups: Arc::from(sg.to_vec()),
            vmaps: vec![uv],
            vmrefs: (0..4).map(|i| VMapRefs::from_slice(&[VMapPt { vmap: 0, index: i }])).collect(),
            surf_faces: vec![SurfFaces { surface: 0, faces: Arc::from(vec![0u32, 1]) }],
            ..Mesh::default()
        };
        m.recompute_bbox();
        m
    }

    fn surfaces() -> Vec<Surface> {
        vec![Surface { name: "Mat".into(), ..Surface::default() }]
    }

    #[test]
    fn face_normals_follow_the_winding() {
        let m = bent_quad([0, 0]);
        let n = m.face_normals();
        // (p1-p0) x (p2-p1) = (0,0,1) x (1,0,-1) = (0,1,0)
        assert_eq!(n[0], Vec3::new(0.0, 1.0, 0.0));
        // (1,1,0) x (0,-1,-1) = (-1,1,-1) / sqrt(3)
        let s = 1.0 / 3.0f32.sqrt();
        assert!(crate::xmath::similar(n[1], Vec3::new(-s, s, -s), 1e-6));
        assert_eq!(m.bbox, Aabb { min: Vec3::ZERO, max: Vec3::ONE });
    }

    #[test]
    fn soft_edges_blend_and_hard_edges_do_not() {
        let s = 1.0 / 3.0f32.sqrt();
        // all edges soft: the shared vertices take the sum of both normals
        let soft = bent_quad([0, 0]).vertex_normals();
        assert_eq!(soft.len(), 6);
        assert_eq!(soft[0], Vec3::new(0.0, 1.0, 0.0));
        let sum = Vec3::new(-s, 1.0 + s, -s);
        let blended = sum / crate::xmath::magnitude(sum);
        assert!(crate::xmath::similar(soft[1], blended, 1e-6));
        assert!(crate::xmath::similar(soft[3], blended, 1e-6));
        assert!(crate::xmath::similar(soft[4], Vec3::new(-s, s, -s), 1e-6));

        // no smoothing data: every edge hard, every corner keeps its face normal
        let hard = bent_quad([u32::MAX, u32::MAX]).vertex_normals();
        assert_eq!(hard[1], Vec3::new(0.0, 1.0, 0.0));
        assert!(crate::xmath::similar(hard[3], Vec3::new(-s, s, -s), 1e-6));

        // the shared edge is edge 1 of the first face (points 1-2) and edge 2 of the second
        // (points 2-1): a hard flag on either side is enough
        let one_side = bent_quad([1 << 1, 0]).vertex_normals();
        assert_eq!(one_side[1], Vec3::new(0.0, 1.0, 0.0));
        let other_side = bent_quad([0, 1 << 2]).vertex_normals();
        assert_eq!(other_side[2], Vec3::new(0.0, 1.0, 0.0));
    }

    #[test]
    fn a_mesh_round_trips_and_its_maps_are_rebuilt() {
        let mut m = bent_quad([0, 5]);
        m.rebuild_vmaps().unwrap();
        // first corner of every vertex goes to the point map, the two repeats to a polygon map
        assert_eq!(m.vmaps.len(), 2);
        assert!(!m.vmaps[0].polymap);
        assert_eq!(m.vmaps[0].vindices, vec![0, 1, 2, 3]);
        assert!(m.vmaps[1].polymap);
        assert_eq!(m.vmaps[1].vindices, vec![1, 2]);
        assert_eq!(m.vmaps[1].pindices, vec![1, 1]);
        // a shared reference keeps the entry of the last corner that used it
        assert_eq!(m.vmrefs[1][0], VMapPt { vmap: 1, index: 0 });
        assert_eq!(m.face_uv(1), Some([Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0), Vec2::new(1.0, 0.0)]));

        let mut w = Writer::new();
        m.write(&mut w, &surfaces(), TextEncoding::Utf8);
        let bytes = w.into_bytes();
        let back = Mesh::read(&Reader::new(&bytes), &surfaces(), &mut Text::default()).unwrap();
        assert_eq!(back, m);
        let mut w = Writer::new();
        back.write(&mut w, &surfaces(), TextEncoding::Utf8);
        assert_eq!(w.into_bytes(), bytes);
    }

    #[test]
    fn hostile_meshes_are_errors() {
        let m = bent_quad([0, 0]);
        let mut w = Writer::new();
        m.write(&mut w, &surfaces(), TextEncoding::Utf8);
        let bytes = w.into_bytes();
        let read = |b: &[u8]| Mesh::read(&Reader::new(b), &surfaces(), &mut Text::default());
        assert!(read(&bytes).is_ok());
        // an unknown surface
        assert!(Mesh::read(&Reader::new(&bytes), &[], &mut Text::default()).is_err());
        // a vertex count far beyond the chunk
        let at = bytes.windows(4).position(|w| w == CHUNK_VERTS.to_le_bytes().as_slice()).unwrap() + 8;
        let mut huge = bytes.clone();
        huge[at..at + 4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(read(&huge).is_err());
        // a face that points outside the vertices
        let at = bytes.windows(4).position(|w| w == CHUNK_FACES.to_le_bytes().as_slice()).unwrap() + 12;
        let mut wild = bytes.clone();
        wild[at..at + 4].copy_from_slice(&77u32.to_le_bytes());
        assert!(read(&wild).is_err());
        // every prefix is an error or a mesh, never a panic
        for cut in 0..bytes.len() {
            let _ = read(&bytes[..cut]);
        }
    }

    #[test]
    fn skin_vertices_keep_the_heaviest_bones() {
        let mut m = bent_quad([0, 0]);
        for (name, w) in [("a", 0.2f32), ("b", 0.6), ("c", 0.2)] {
            m.vmaps.push(VMap {
                name: name.into(),
                kind: VMapKind::Weight,
                polymap: false,
                values: vec![w],
                vindices: vec![0],
                pindices: vec![],
            });
        }
        for (i, r) in m.vmrefs.iter_mut().enumerate() {
            if i == 0 {
                r.extend([1, 2, 3].map(|vmap| VMapPt { vmap, index: 0 }));
            } else {
                r.push(VMapPt { vmap: 2, index: 0 });
            }
        }
        let bones: Vec<Bone> = ["a", "b", "c"]
            .iter()
            .map(|n| Bone { name: (*n).into(), wmap: (*n).into(), ..Bone::default() })
            .collect();
        let normals = m.vertex_normals();
        let sv = m.skin_vertices(&bones, 2, &normals).unwrap();
        assert_eq!(sv.len(), 6);
        // b is the heaviest; of the two equal ones the first in the reference stays
        assert_eq!(sv[0].bones.len(), 2);
        assert_eq!((sv[0].bones[0].id, sv[0].bones[1].id), (1, 0));
        assert!((sv[0].bones[0].w - 0.75).abs() < 1e-6);
        assert!((sv[0].bones[1].w - 0.25).abs() < 1e-6);
        assert_eq!(sv[1].bones.as_slice(), &[BoneWeight { id: 1, w: 1.0 }]);
        assert_eq!(sv[1].uv, Vec2::new(0.0, 1.0));
        // a weight map no bone answers to
        assert!(m.skin_vertices(&bones[..1], 2, &normals).is_err());
    }
}
