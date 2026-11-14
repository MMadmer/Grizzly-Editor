//! Export from an editable object: the static exporter
//! and the skeleton exporter (skinned, with bone data and motions).
//!
//! What the exporters do, kept here because files depend on it:
//! - neither exporter welds vertices: every face corner is a vertex of its own;
//! - neither exporter calculates tangents: a static model carries none, a
//!   skinned one the constructor's (1, 0, 0) and (0, 0, 1);
//! - the static exporter's VIPM pass returns at once, so it never writes a progressive mesh;
//!   a skeleton flagged progressive goes through the VIPM pass (`vipm`) instead of the face
//!   optimiser - over a triangle soup it collapses nothing, and the split comes out with its
//!   vertices taken last first, its faces newest first through the cheap cache optimiser
//!   (mode 1), one sliding window;
//! - the bone boxes are Wild Magic's minimal boxes (`obb::min_box`), a breakable part's
//!   fragments too;
//! - the build time of the description is never set (`ctime` only formats it): it is 0.

use glam::{Vec2, Vec3};
use gz_chunk::Writer;
use gz_fmt_object::xmath::{Aabb, EPS, EPS_L, Mat4, cvtt, magnitude, similar};
use gz_fmt_object::{BoneState, Mesh, Object, ObjectFlags, SMotionFlags, SkinVertex, animate, rest_pose};
use smallvec::SmallVec;
use smol_str::SmolStr;

use crate::bones::{BoneIk, BoneName, Obb, write_names};
use crate::format::{Desc, Header, MT, chunk, crc32};
use crate::motion::{FL_R_KEY_ABSENT, FL_T_KEY_16, FL_T_KEY_PRESENT};
use crate::optimize::{optimize_faces, optimize_vertices};
use crate::vertex::VertexBuffer;
use crate::{OgfError, Result};

/// `MAX_BONE` of the editor's build.
const MAX_BONE: usize = 223;
/// `BI_NONE`.
const BI_NONE: u16 = u16::MAX;

/// What the export takes from the machine and the library.
#[derive(Clone, Debug, Default)]
pub struct ExportOptions {
    /// `m_LibName`: the object's library name, written into the description.
    pub source_file: SmolStr,
    /// `"\\\\" + computer + "\\" + user`.
    pub build_name: SmolStr,
    /// The game's materials (`gamemtl.xr`): a surface's material must be one of them, a bone's
    /// one that is dynamic. `None` checks nothing.
    pub materials: Option<std::sync::Arc<gz_fmt_xr::GameMaterials>>,
}

/// An exported model and what the export would have logged.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Export {
    pub bytes: Vec<u8>,
    pub warnings: Vec<String>,
}

fn fail(what: impl Into<String>) -> OgfError {
    OgfError::Export(what.into())
}

fn desc(obj: &Object, opts: &ExportOptions) -> Desc {
    Desc {
        source_file: opts.source_file.clone(),
        build_name: opts.build_name.clone(),
        build_time: 0,
        create_name: obj.created_by.clone(),
        create_time: i64::from(obj.created_at),
        modif_name: obj.modified_by.clone(),
        modif_time: i64::from(obj.modified_at),
        time_size: 8,
    }
}

fn check_tex_count(obj: &Object, surface: usize) -> Result<&gz_fmt_object::Surface> {
    let s = obj
        .surfaces
        .get(surface)
        .ok_or_else(|| fail("A face list names a surface the object does not have."))?;
    if s.tex_count() != 1 {
        return Err(fail(format!("Surface '{}' must have exactly one texture coordinate set.", s.name)));
    }
    Ok(s)
}

// ------------------------------------------------------------------ static

#[derive(Clone, Copy)]
struct OgfVert {
    p: Vec3,
    n: Vec3,
    uv: Vec2,
}

struct Part {
    verts: Vec<OgfVert>,
    faces: Vec<[u16; 3]>,
    bbox: Aabb,
}

impl Default for Part {
    fn default() -> Self {
        Self { verts: Vec::new(), faces: Vec::new(), bbox: Aabb::INVALID }
    }
}

impl Part {
    fn pack(&mut self, v: OgfVert) -> Option<u16> {
        let p = self.verts.len();
        if p >= 0xFFFF {
            return None;
        }
        self.verts.push(v);
        Some(p as u16)
    }

    /// False asks for a new part.
    fn add_face(&mut self, v: [OgfVert; 3], warnings: &mut Vec<String>) -> bool {
        if similar(v[0].p, v[1].p, EPS) || similar(v[0].p, v[2].p, EPS) || similar(v[1].p, v[2].p, EPS) {
            warnings.push("Degenerate face found. Removed.".to_owned());
            return true;
        }
        // a vertex packed before the part filled up stays in it, its face is lost
        let (Some(a), Some(b), Some(c)) = (self.pack(v[0]), self.pack(v[1]), self.pack(v[2])) else {
            return false;
        };
        self.faces.push([a, b, c]);
        true
    }
}

struct StaticSplit {
    surface: usize,
    parts: Vec<Part>,
    current: Option<usize>,
}

fn save_static_part(w: &mut Writer, obj: &Object, surface: usize, part: &Part) {
    let s = &obj.surfaces[surface];
    w.chunk(chunk::HEADER, |w| Header::around(MT::NORMAL, &part.bbox).write(w));
    w.chunk(chunk::TEXTURE, |w| {
        w.w_stringz(&s.texture);
        w.w_stringz(&s.shader);
    });
    let verts: Vec<(Vec3, Vec3, Vec2)> = part.verts.iter().map(|v| (v.p, v.n, v.uv)).collect();
    w.chunk(chunk::VERTICES, |w| VertexBuffer::write_static(&verts, w));
    w.chunk(chunk::INDICES, |w| {
        w.w_u32(part.faces.len() as u32 * 3);
        for f in &part.faces {
            f.iter().for_each(|i| w.w_u16(*i));
        }
    });
}

/// Exports a whole object.
pub fn export_static(obj: &Object, opts: &ExportOptions) -> Result<Export> {
    if obj.meshes.is_empty() {
        return Err(fail("The object has no meshes."));
    }
    let mut warnings = Vec::new();
    let mut splits: Vec<StaticSplit> = Vec::new();
    for mesh in &obj.meshes {
        let normals = mesh.vertex_normals();
        for sf in &mesh.surf_faces {
            let surface = sf.surface as usize;
            let surf = check_tex_count(obj, surface)?;
            let si = match splits.iter().position(|s| s.surface == surface) {
                Some(i) => i,
                None => {
                    if let Some(m) = &opts.materials
                        && m.by_name(&surf.game_mtl).is_none()
                    {
                        return Err(fail(format!(
                            "Surface: '{}' contains undefined game material.",
                            surf.name
                        )));
                    }
                    splits.push(StaticSplit { surface, parts: Vec::new(), current: None });
                    splits.len() - 1
                }
            };
            let two_sided = surf.two_sided();
            let split = &mut splits[si];
            let mut elapsed = if two_sided { sf.faces.len() * 2 } else { sf.faces.len() } as i64;
            if split.current.is_none() {
                split.parts.push(Part::default());
                split.current = Some(split.parts.len() - 1);
            }
            // the counter drops once per face, so a two-sided list is walked twice
            loop {
                for &f in sf.faces.iter() {
                    let face = mesh
                        .faces
                        .get(f as usize)
                        .ok_or_else(|| fail("A face list names a face the mesh does not have."))?;
                    let mut v = [OgfVert { p: Vec3::ZERO, n: Vec3::Y, uv: Vec2::ZERO }; 3];
                    for (k, fv) in face.v.iter().enumerate() {
                        let uv = mesh.corner_uv(*fv).ok_or_else(|| fail("uv empty"))?;
                        let p = *mesh
                            .points
                            .get(fv.point as usize)
                            .ok_or_else(|| fail("A face names a point the mesh does not have."))?;
                        let n = *normals
                            .get(f as usize * 3 + k)
                            .ok_or_else(|| fail("Normal index out of range."))?;
                        v[k] = OgfVert { p, n, uv };
                    }
                    elapsed -= 1;
                    let cur = split.current.unwrap_or(0);
                    let mut new_part = !split.parts[cur].add_face(v, &mut warnings);
                    if two_sided {
                        for x in &mut v {
                            x.n = -x.n;
                        }
                        if !split.parts[cur].add_face([v[2], v[1], v[0]], &mut warnings) {
                            new_part = true;
                        }
                    }
                    if new_part && elapsed > 0 {
                        split.parts.push(Part::default());
                        split.current = Some(split.parts.len() - 1);
                    }
                }
                if elapsed <= 0 {
                    break;
                }
            }
        }
    }
    let mut total = Aabb::INVALID;
    for s in &mut splits {
        for p in &mut s.parts {
            p.bbox = Aabb::INVALID;
            for v in &p.verts {
                p.bbox.modify(v.p);
            }
            total.merge(&p.bbox);
        }
    }
    let mut w = Writer::with_capacity(1 << 16);
    if splits.len() == 1 && splits[0].parts.len() == 1 {
        save_static_part(&mut w, obj, splits[0].surface, &splits[0].parts[0]);
    } else {
        w.chunk(chunk::HEADER, |w| Header::around(MT::HIERRARHY, &total).write(w));
        w.chunk(chunk::S_DESC, |w| desc(obj, opts).write(w));
        w.chunk(chunk::CHILDREN, |w| {
            let mut id = 0;
            for s in &splits {
                for p in &s.parts {
                    w.chunk(id, |w| save_static_part(w, obj, s.surface, p));
                    id += 1;
                }
            }
        });
    }
    w.chunk(chunk::COLLISION_VERTICES, |w| {
        for s in &splits {
            for p in &s.parts {
                p.verts.iter().for_each(|v| w.w_vec3(v.p));
            }
        }
    });
    w.chunk(chunk::COLLISION_INDICES, |w| {
        let mut offset = 0u32;
        for s in &splits {
            for p in &s.parts {
                for f in &p.faces {
                    f.iter().for_each(|i| w.w_u32(offset + u32::from(*i)));
                }
                offset += p.verts.len() as u32;
            }
        }
    });
    Ok(Export { bytes: w.into_bytes(), warnings })
}

// ------------------------------------------------------------------ skeleton

#[derive(Clone, Copy, Debug, PartialEq)]
struct Bw {
    id: u16,
    w: f32,
}

#[derive(Clone, Debug)]
struct SkelVert {
    offs: Vec3,
    norm: Vec3,
    uv: Vec2,
    bones: SmallVec<[Bw; 4]>,
}

impl SkelVert {
    /// The bones sorted by id.
    fn new(sv: &SkinVertex, bones: &[Bw]) -> Self {
        let mut b: SmallVec<[Bw; 4]> = bones.iter().copied().collect();
        b.sort_by_key(|x| x.id);
        Self { offs: sv.offs, norm: sv.norm, uv: sv.uv, bones: b }
    }

    fn sort_by_weight(&mut self) {
        self.bones.sort_by(|a, b| b.w.partial_cmp(&a.w).unwrap_or(std::cmp::Ordering::Equal));
    }
}

const TANGENT: Vec3 = Vec3::new(1.0, 0.0, 0.0);
const BINORMAL: Vec3 = Vec3::new(0.0, 0.0, 1.0);

struct SkelSplit {
    shader: SmolStr,
    texture: SmolStr,
    part: u16,
    link_type: u32,
    verts: Vec<SkelVert>,
    faces: Vec<[u16; 3]>,
    used_bones: Vec<u16>,
    bbox: Aabb,
    /// `m_SWR`: the sliding windows of a progressive split.
    swr: Vec<crate::vipm::Swr>,
}

impl SkelSplit {
    /// False for a degenerate face, which is left out.
    fn add_face(&mut self, v: [&SkelVert; 3], warnings: &mut Vec<String>) -> Result<bool> {
        if similar(v[0].offs, v[1].offs, EPS)
            || similar(v[0].offs, v[2].offs, EPS)
            || similar(v[1].offs, v[2].offs, EPS)
        {
            warnings.push("Degenerate face found. Removed.".to_owned());
            return Ok(false);
        }
        if self.verts.len() + 3 > 0xFFFF {
            return Err(fail(format!(
                "Split '{}' / '{}' has more than 65535 vertices.",
                self.shader, self.texture
            )));
        }
        let base = self.verts.len() as u16;
        self.verts.extend(v.iter().map(|x| (*x).clone()));
        self.faces.push([base, base + 1, base + 2]);
        Ok(true)
    }

    /// The collapse order (vertices permuted, faces in window order) and
    /// the sliding windows. When the pass gives nothing the split stays as it is, unoptimised,
    /// as the X-Ray SDK's exporter leaves it ("The model could not be made progressive.").
    fn make_progressive(&mut self, warnings: &mut Vec<String>) {
        let verts: Vec<(Vec3, Vec2)> = self.verts.iter().map(|v| (v.offs, v.uv)).collect();
        let Some(r) = crate::vipm::convert(&verts, &self.faces) else {
            warnings.push("The model could not be made progressive.".to_owned());
            return;
        };
        let source = self.verts.clone();
        for (i, v) in source.into_iter().enumerate() {
            if let Some(dst) = r.permute_verts.get(i).and_then(|p| self.verts.get_mut(usize::from(*p))) {
                *dst = v;
            }
        }
        self.faces = r.indices.as_chunks::<3>().0.to_vec();
        self.swr = r.swr;
    }

    /// The face order of the cache model, then the vertices in first use.
    fn stripify(&mut self) {
        let flat: Vec<u16> = self.faces.iter().flatten().copied().collect();
        let remap = optimize_faces(&flat);
        let source = self.faces.clone();
        for (slot, r) in self.faces.iter_mut().zip(&remap) {
            if let Some(f) = source.get(*r as usize) {
                *slot = *f;
            }
        }
        let flat: Vec<u16> = self.faces.iter().flatten().copied().collect();
        let remap = optimize_vertices(&flat, self.verts.len());
        let source = self.verts.clone();
        for (old, new) in remap.iter().enumerate() {
            if let (Some(dst), Some(src)) = (self.verts.get_mut(*new as usize), source.get(old)) {
                *dst = src.clone();
            }
        }
        for f in &mut self.faces {
            for i in f.iter_mut() {
                *i = remap.get(usize::from(*i)).map_or(*i, |n| *n as u16);
            }
        }
    }

    fn save(&mut self, w: &mut Writer) {
        let kind = if self.swr.is_empty() { MT::SKELETON_GEOMDEF_ST } else { MT::SKELETON_GEOMDEF_PM };
        w.chunk(chunk::HEADER, |w| Header::around(kind, &self.bbox).write(w));
        w.chunk(chunk::TEXTURE, |w| {
            w.w_stringz(&self.texture);
            w.w_stringz(&self.shader);
        });
        let link = self.link_type;
        let verts = &mut self.verts;
        w.chunk(chunk::VERTICES, |w| {
            w.w_u32(link);
            w.w_u32(verts.len() as u32);
            for v in verts.iter_mut() {
                v.sort_by_weight();
                let b0 = v.bones.first().copied().unwrap_or(Bw { id: 0, w: 1.0 });
                let common = |w: &mut Writer, v: &SkelVert| {
                    w.w_vec3(v.offs);
                    w.w_vec3(v.norm);
                    w.w_vec3(TANGENT);
                    w.w_vec3(BINORMAL);
                };
                match link {
                    1 => {
                        common(w, v);
                        w.w_vec2(v.uv);
                        w.w_u32(u32::from(b0.id));
                    }
                    2 => {
                        w.w_u16(b0.id);
                        let weight = if v.bones.len() == 2 {
                            w.w_u16(v.bones[1].id);
                            v.bones[1].w / (v.bones[0].w + v.bones[1].w)
                        } else {
                            w.w_u16(b0.id);
                            1.0
                        };
                        common(w, v);
                        w.w_f32(weight);
                        w.w_vec2(v.uv);
                    }
                    n => {
                        let n = n as usize;
                        for i in 0..n {
                            w.w_u16(v.bones.get(i).map_or(b0.id, |b| b.id));
                        }
                        common(w, v);
                        for i in 0..n - 1 {
                            w.w_f32(v.bones.get(i).map_or(0.0, |b| b.w));
                        }
                        w.w_vec2(v.uv);
                    }
                }
            }
        });
        w.chunk(chunk::INDICES, |w| {
            w.w_u32(self.faces.len() as u32 * 3);
            for f in &self.faces {
                f.iter().for_each(|i| w.w_u16(*i));
            }
        });
        if !self.swr.is_empty() {
            w.chunk(chunk::SWIDATA, |w| {
                // 16 reserved bytes, the count, the records
                for _ in 0..4 {
                    w.w_u32(0);
                }
                w.w_u32(self.swr.len() as u32);
                for s in &self.swr {
                    w.w_u32(s.offset);
                    w.w_u16(s.num_tris);
                    w.w_u16(s.num_verts);
                }
            });
        }
    }
}

/// The break-apart group of every bone.
fn build_groups(obj: &Object, bone: usize, groups: &mut [u16], mut id: u16, last: &mut u16) {
    if obj.bones[bone].ik.breakable() {
        *last += 1;
        id = *last;
    }
    groups[bone] = id;
    for c in obj.bone_children(bone) {
        build_groups(obj, c, groups, id, last);
    }
}

fn prepare_geometry(
    obj: &Object,
    influence: u8,
    warnings: &mut Vec<String>,
) -> Result<(Vec<SkelSplit>, Aabb)> {
    if obj.meshes.is_empty() {
        return Err(fail("The object has no meshes."));
    }
    if obj.bones.is_empty() {
        return Err(fail("There are no bones in the object."));
    }
    if obj.bones.len() > MAX_BONE {
        return Err(fail("Object cannot handle more than 223 bones."));
    }
    if !(obj.is_dynamic() && obj.is_skeleton()) {
        return Err(fail("The object is not a dynamic skeleton."));
    }
    let breakable = obj.bones.iter().any(|b| b.ik.breakable());
    let mut groups = vec![0u16; obj.bones.len()];
    if breakable {
        let root = obj.root_bone().ok_or_else(|| fail("The object has no root bone."))?;
        let mut last = 0;
        build_groups(obj, root, &mut groups, 0, &mut last);
    }
    let mut splits: Vec<SkelSplit> = Vec::new();
    for mesh in &obj.meshes {
        let normals = mesh.vertex_normals();
        let svs = mesh
            .skin_vertices(&obj.bones, u32::from(influence), &normals)
            .map_err(|e| fail(e.to_string()))?;
        prepare_mesh(obj, mesh, &svs, &groups, breakable, &mut splits, warnings)?;
    }
    let mut kept = Vec::with_capacity(splits.len());
    for mut s in splits {
        if s.verts.is_empty() || s.faces.is_empty() {
            warnings
                .push(format!("Empty split found (Shader/Texture: {}/{}). Removed.", s.shader, s.texture));
            continue;
        }
        s.used_bones.sort_unstable();
        s.used_bones.dedup();
        kept.push(s);
    }
    let mut total = Aabb::INVALID;
    for s in &mut kept {
        s.bbox = Aabb::INVALID;
        for v in &s.verts {
            s.bbox.modify(v.offs);
        }
        total.merge(&s.bbox);
    }
    Ok((kept, total))
}

fn prepare_mesh(
    obj: &Object,
    mesh: &Mesh,
    svs: &[SkinVertex],
    groups: &[u16],
    breakable: bool,
    splits: &mut Vec<SkelSplit>,
    warnings: &mut Vec<String>,
) -> Result<()> {
    let mut bone_list: Vec<u16> = Vec::with_capacity(12);
    for sf in &mesh.surf_faces {
        let surf = check_tex_count(obj, sf.surface as usize)?;
        for &f in sf.faces.iter() {
            let f = f as usize;
            let corner = |k: usize| svs.get(f * 3 + k).ok_or_else(|| fail("A face has no skin vertices."));
            let sv = [corner(0)?, corner(1)?, corner(2)?];
            let link_type = sv.iter().map(|s| s.bones.len()).max().unwrap_or(0);
            if link_type == 0 {
                return Err(fail(format!("A vertex of mesh '{}' is bound to no bone.", mesh.name)));
            }
            bone_list.clear();
            let mut v: SmallVec<[SkelVert; 3]> = SmallVec::new();
            for s in sv {
                let first = s.bones.first().map_or(0, |b| b.id);
                if link_type == 1 {
                    v.push(SkelVert::new(s, &[Bw { id: first, w: 1.0 }, Bw { id: first, w: 0.0 }]));
                    bone_list.push(first);
                } else {
                    let b: SmallVec<[Bw; 4]> = s.bones.iter().map(|b| Bw { id: b.id, w: b.w }).collect();
                    v.push(SkelVert::new(s, &b));
                    bone_list.extend(s.bones.iter().map(|b| b.id));
                }
            }
            let mut part = 0u16;
            if breakable {
                bone_list.sort_unstable();
                bone_list.dedup();
                part = bone_list.first().and_then(|b| groups.get(usize::from(*b))).copied().unwrap_or(0);
                if bone_list.iter().any(|b| groups.get(usize::from(*b)).copied().unwrap_or(0) != part) {
                    return Err(fail("Can't export object as breakable. Object have N-Link face(s)."));
                }
            }
            let si = match splits
                .iter()
                .position(|s| s.shader == surf.shader && s.texture == surf.texture && s.part == part)
            {
                Some(i) => i,
                None => {
                    splits.push(SkelSplit {
                        shader: surf.shader.clone(),
                        texture: surf.texture.clone(),
                        part,
                        link_type: 0,
                        verts: Vec::new(),
                        faces: Vec::new(),
                        used_bones: Vec::new(),
                        bbox: Aabb::INVALID,
                        swr: Vec::new(),
                    });
                    splits.len() - 1
                }
            };
            let split = &mut splits[si];
            split.link_type = split.link_type.max(link_type as u32);
            split.used_bones.extend_from_slice(&bone_list);
            split.add_face([&v[0], &v[1], &v[2]], warnings)?;
            if surf.two_sided() {
                for x in &mut v {
                    x.norm = -x.norm;
                }
                split.add_face([&v[0], &v[2], &v[1]], warnings)?;
            }
        }
    }
    Ok(())
}

/// The geometry, the bones and, for an animated object, the motions.
pub fn export_skeleton(obj: &Object, opts: &ExportOptions, influence: u8) -> Result<Export> {
    let mut warnings = Vec::new();
    let (mut splits, total) = prepare_geometry(obj, influence, &mut warnings)?;
    let progressive = obj.flags.contains(ObjectFlags::PROGRESSIVE);
    let rest = rest_pose(&obj.bones);
    let mut bone_points: Vec<Vec<Vec3>> = vec![Vec::new(); obj.bones.len()];
    for s in &mut splits {
        if progressive {
            s.make_progressive(&mut warnings);
        } else {
            s.stripify();
        }
        for v in &s.verts {
            let b = usize::from(v.bones.first().map_or(0, |b| b.id));
            if let (Some(list), Some(inv)) = (bone_points.get_mut(b), rest.inverse.get(b)) {
                list.push(inv.transform_tiny(v.offs));
            }
        }
    }

    let mut w = Writer::with_capacity(1 << 16);
    let kind = if obj.is_animated() { MT::SKELETON_ANIM } else { MT::SKELETON_RIGID };
    w.chunk(chunk::HEADER, |w| Header::around(kind, &total).write(w));
    w.chunk(chunk::S_DESC, |w| desc(obj, opts).write(w));
    w.chunk(chunk::CHILDREN, |w| {
        for (i, s) in splits.iter_mut().enumerate() {
            w.chunk(i as u32, |w| s.save(w));
        }
    });
    let names: Vec<BoneName> = obj
        .bones
        .iter()
        .enumerate()
        .map(|(i, b)| BoneName {
            name: b.name.clone(),
            parent: b.parent.clone(),
            obb: crate::obb::min_box(&bone_points[i]),
        })
        .collect();
    w.chunk(chunk::S_BONE_NAMES, |w| write_names(&names, w));
    for b in &obj.bones {
        if !b.shape.valid() {
            return Err(fail(format!("Bone '{}' has invalid shape.", b.name)));
        }
        if let Some(m) = &opts.materials {
            match m.by_name(&b.game_mtl) {
                None => return Err(fail(format!("Bone '{}' has invalid game material.", b.name))),
                Some(mtl) if !mtl.is_dynamic() => {
                    return Err(fail(format!("Bone '{}' has non-dynamic game material.", b.name)));
                }
                Some(_) => {}
            }
        }
    }
    w.chunk(chunk::S_IKDATA, |w| obj.bones.iter().for_each(|b| BoneIk::from_bone(b).write(w)));
    if !obj.user_data.is_empty() {
        w.chunk(chunk::S_USERDATA, |w| w.w_stringz(&obj.user_data));
    }
    if !obj.lod_ref.is_empty() {
        w.chunk(chunk::S_LODS, |w| w.w_line(&obj.lod_ref));
    }
    if obj.is_animated() {
        write_motions(obj, &mut w, &mut warnings)?;
    }
    Ok(Export { bytes: w.into_bytes(), warnings })
}

// ------------------------------------------------------------------ breakable parts

/// A face of a breakable part, in the part's own frame.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakableFace {
    pub o: [Vec3; 3],
    pub n: [Vec3; 3],
    pub uv: [Vec2; 3],
    /// The fragment it belongs to.
    pub bone: u16,
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub two_sided: bool,
}

/// A fragment of a breakable part.
#[derive(Clone, Debug, PartialEq)]
pub struct BreakableBone {
    pub name: SmolStr,
    pub parent: SmolStr,
    pub material: SmolStr,
    pub offset: Vec3,
    /// The fragment's area: the engine takes it as the mass.
    pub area: f32,
}

/// What a breakable part's export writes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BreakablePart {
    pub faces: Vec<BreakableFace>,
    pub bones: Vec<BreakableBone>,
}

/// The box shape, the remove-after-break flag, the no-joint type and the breakable flag of the joint IK data.
const SHAPE_BOX: u16 = 1;
const SHAPE_REMOVE_AFTER_BREAK: u16 = 1 << 1;
const JOINT_NONE: u32 = 4;
const IK_BREAKABLE: u32 = 1;

/// The rigid skeleton of a breakable part - one split per shader and
/// texture, one bone per fragment, each bone a removable box of its points.
///
/// The faces are not optimised (they are written in the order they came); the IK record's
/// friction, which the X-Ray SDK leaves uninitialised, is 0 here, as is the
/// count of degenerate faces a split starts with (also uninitialised there).
pub fn export_breakable(part: &BreakablePart) -> Result<Export> {
    if part.bones.len() > 63 {
        return Err(fail("Breakable object cannot handle more than 63 parts."));
    }
    let mut warnings = Vec::new();
    let mut splits: Vec<(SkelSplit, u32)> = Vec::new();
    for f in &part.faces {
        let si = match splits.iter().position(|(s, _)| s.shader == f.shader && s.texture == f.texture) {
            Some(i) => i,
            None => {
                splits.push((
                    SkelSplit {
                        shader: f.shader.clone(),
                        texture: f.texture.clone(),
                        part: 0,
                        link_type: 1,
                        verts: Vec::new(),
                        faces: Vec::new(),
                        used_bones: Vec::new(),
                        bbox: Aabb::INVALID,
                        swr: Vec::new(),
                    },
                    0,
                ));
                splits.len() - 1
            }
        };
        let (split, invalid) = &mut splits[si];
        let mut v: [SkelVert; 3] = std::array::from_fn(|k| SkelVert {
            offs: f.o[k],
            norm: f.n[k],
            uv: f.uv[k],
            bones: smallvec::smallvec![Bw { id: f.bone, w: 1.0 }],
        });
        if !split.add_face([&v[0], &v[1], &v[2]], &mut warnings)? {
            *invalid += 1;
        }
        if f.two_sided {
            for x in &mut v {
                x.norm = -x.norm;
            }
            // `add_face` counts the face and the caller counts it again
            if !split.add_face([&v[0], &v[2], &v[1]], &mut warnings)? {
                *invalid += 2;
            }
        }
    }
    let mut bone_points: Vec<Vec<Vec3>> = vec![Vec::new(); part.bones.len()];
    for (s, invalid) in &splits {
        if s.verts.is_empty() || s.faces.is_empty() {
            return Err(fail(format!("Degenerate part found (Texture '{}').", s.texture)));
        }
        if *invalid != 0 {
            warnings.push(format!(
                "Part [texture '{}'] have {invalid} duplicate(degenerate) face(s).",
                s.texture
            ));
        }
        for v in &s.verts {
            let b = usize::from(v.bones.first().map_or(0, |b| b.id));
            let bone =
                part.bones.get(b).ok_or_else(|| fail("A face names a fragment the part does not have."))?;
            if let Some(list) = bone_points.get_mut(b) {
                list.push(Vec3::new(
                    v.offs.x - bone.offset.x,
                    v.offs.y - bone.offset.y,
                    v.offs.z - bone.offset.z,
                ));
            }
        }
    }
    let mut total = Aabb::INVALID;
    for (s, _) in &mut splits {
        s.bbox = Aabb::INVALID;
        for v in &s.verts {
            s.bbox.modify(v.offs);
        }
        total.merge(&s.bbox);
    }

    let mut w = Writer::with_capacity(1 << 14);
    w.chunk(chunk::HEADER, |w| Header::around(MT::SKELETON_RIGID, &total).write(w));
    w.chunk(chunk::S_DESC, |w| Desc { time_size: 8, ..Desc::default() }.write(w));
    w.chunk(chunk::CHILDREN, |w| {
        for (i, (s, _)) in splits.iter_mut().enumerate() {
            w.chunk(i as u32, |w| s.save(w));
        }
    });
    let boxes: Vec<Obb> = bone_points.iter().map(|p| crate::obb::min_box(p)).collect();
    let names: Vec<BoneName> = part
        .bones
        .iter()
        .zip(&boxes)
        .map(|(b, obb)| BoneName { name: b.name.clone(), parent: b.parent.clone(), obb: *obb })
        .collect();
    w.chunk(chunk::S_BONE_NAMES, |w| write_names(&names, w));
    w.chunk(chunk::S_IKDATA, |w| {
        for (b, obb) in part.bones.iter().zip(&boxes) {
            let shape = gz_fmt_object::BoneShape {
                kind: SHAPE_BOX,
                flags: SHAPE_REMOVE_AFTER_BREAK,
                box_rotate: obb.rotate.map(|r| r.to_array()),
                box_translate: obb.translate,
                box_halfsize: obb.halfsize,
                ..gz_fmt_object::BoneShape::default()
            };
            // a joint limit's reset is a zero limit: its negation is written, -0
            let limit = [-0.0f32, -0.0, 1.0, 1.0];
            BoneIk {
                version: crate::bones::IKDATA_VERSION,
                game_mtl: b.material.clone(),
                shape,
                joint_type: JOINT_NONE,
                limits: [limit; 3],
                spring_factor: 1.0,
                damping_factor: 1.0,
                flags: IK_BREAKABLE,
                break_force: 0.0,
                break_torque: 0.0,
                friction: 0.0,
                rest_rotate: Vec3::ZERO,
                rest_offset: b.offset,
                mass: b.area,
                center_of_mass: obb.translate,
            }
            .write(w);
        }
    });
    Ok(Export { bytes: w.into_bytes(), warnings })
}

/// The motion keys and definitions alone.
pub fn export_omf(obj: &Object) -> Result<Export> {
    let mut warnings = Vec::new();
    let mut w = Writer::new();
    write_motions(obj, &mut w, &mut warnings)?;
    Ok(Export { bytes: w.into_bytes(), warnings })
}

fn write_motions(obj: &Object, w: &mut Writer, warnings: &mut Vec<String>) -> Result<()> {
    if obj.motion_refs.is_empty() {
        if obj.motions.is_empty() {
            return Err(fail("Object doesn't have own motion"));
        }
        let keys = motion_keys(obj, warnings)?;
        w.w(&keys);
    }
    motion_defs(obj, w)
}

fn align_y(m: &mut Mat4) {
    let mut i = m.row(0);
    let mut k = m.row(2);
    i.y = 0.0;
    k.y = 0.0;
    let j = Vec3::new(0.0, 1.0, 0.0);
    let smi = i.x * i.x + i.z * i.z;
    let smk = k.x * k.x + k.z * k.z;
    let bi = smi > gz_fmt_object::xmath::EPS_S;
    let bk = smk > gz_fmt_object::xmath::EPS_S;
    if smk > smi && bk {
        k *= 1.0 / smk.sqrt();
        i = gz_fmt_object::xmath::cross(j, k);
    } else if bi {
        i *= 1.0 / smi.sqrt();
        k = gz_fmt_object::xmath::cross(i, j);
    } else {
        *m = Mat4::IDENTITY;
        return;
    }
    m.set_row(0, i);
    m.set_row(1, j);
    m.set_row(2, k);
}

/// The `OGF_S_MOTIONS` chunk.
fn motion_keys(obj: &Object, warnings: &mut Vec<String>) -> Result<Vec<u8>> {
    let bones = &obj.bones;
    let gt = Mat4::mul(
        &Mat4::translation(obj.actor_position),
        &Mat4::hpb(obj.actor_rotation.y, obj.actor_rotation.x, obj.actor_rotation.z),
    );
    let root_id = obj.root_bone().unwrap_or(0);
    let mut w = Writer::new();
    w.open_chunk(chunk::S_MOTIONS);
    w.chunk(0, |w| w.w_u32(obj.motions.len() as u32));
    for (mi, m) in obj.motions.iter().enumerate() {
        let root_mover = m.flags.contains(SMotionFlags::ROOT_MOVER);
        if root_mover && obj.bone_children(root_id).len() != 1 {
            return Err(fail(format!(
                "{} has moveXform flag - but skeleton root has more than one child or has mesh! add special root bone please!",
                m.name
            )));
        }
        let len = usize::try_from(m.length()).unwrap_or(0);
        if len == 0 {
            return Err(fail(format!("Motion '{}' has no frames.", m.name)));
        }
        let mut qr: Vec<Vec<[i16; 4]>> = vec![Vec::with_capacity(len); bones.len()];
        let mut tr: Vec<Vec<Vec3>> = vec![Vec::with_capacity(len); bones.len()];
        let mut anchor = Mat4::IDENTITY;
        for frame in m.frame_start..=m.frame_end {
            let t = frame as f32 / m.fps;
            let states: Vec<BoneState> = bones
                .iter()
                .enumerate()
                .map(|(i, b)| {
                    let (offset, rotate) = if m.bone_motions.len() > i {
                        m.evaluate(i, t).unwrap_or((b.rest_offset, b.rest_rotate))
                    } else {
                        (b.rest_offset, b.rest_rotate)
                    };
                    BoneState {
                        offset,
                        rotate,
                        world_orient: m.bone_flags(i) & gz_fmt_object::BoneMotion::WORLD_ORIENT != 0,
                    }
                })
                .collect();
            let mut pose = animate(bones, &states);
            if root_mover {
                let anchor_bone = obj.bone_children(root_id)[0];
                if frame == m.frame_start {
                    anchor = Mat4::invert_43(&pose.world[anchor_bone]);
                }
                // the root object's animation is calculated
                let bottom = (1..bones.len())
                    .filter(|i| !bones[*i].is_root())
                    .map(|i| pose.world[i].c().y)
                    .fold(f32::MAX, f32::min);
                let gl_anchor = Mat4::mul_43(&pose.world[anchor_bone], &anchor);
                let mut root = gl_anchor;
                align_y(&mut root);
                let mut c = gl_anchor.c();
                c.y = bottom;
                root.set_c(c);
                pose.world[root_id] = root;
                pose.local[root_id] = Mat4::mul_43(&Mat4::IDENTITY, &root);
                pose.local[anchor_bone] = Mat4::mul_43(&Mat4::invert_43(&root), &pose.world[anchor_bone]);
            }
            for (i, b) in bones.iter().enumerate() {
                let mut mat = pose.local[i];
                if b.is_root() {
                    mat = mat.mul_a43(&gt);
                }
                let q = mat.to_quat();
                let quant = |v: f32| cvtt(v * 32767.0).clamp(-32767, 32767) as i16;
                qr[i].push([quant(q.x), quant(q.y), quant(q.z), quant(q.w)]);
                tr[i].push(mat.c());
            }
        }
        w.open_chunk(mi as u32 + 1);
        w.w_stringz(&m.name);
        w.w_u32(len as u32);
        for i in 0..bones.len() {
            let keys_t = &tr[i];
            let keys_r = &qr[i];
            let mut mt = Vec3::ZERO;
            let (mut at, mut bt) = (keys_t[0], keys_t[0]);
            for t in keys_t {
                mt += *t;
                at = at.min(*t);
                bt = bt.max(*t);
            }
            mt /= len as f32;
            let ct = (bt + at) * 0.5;
            let mut st = (bt - at) * 0.5;
            let r0 = keys_r[0];
            let bit16 = magnitude(st) > 1.5;
            if bit16 {
                warnings.push(format!("animation [{}] is 16bit-transform ({:.6})m", m.name, magnitude(st)));
            }
            let t_present = keys_t.iter().any(|t| !similar(mt, *t, EPS_L));
            let r_present = keys_r.iter().any(|r| *r != r0);
            let q = |d: f32, s: f32, m: f32, lo: i32, hi: i32| cvtt(m * d / s).clamp(lo, hi);
            let mut k16 = Vec::with_capacity(len * 6);
            let mut k8 = Vec::with_capacity(len * 3);
            for t in keys_t {
                if bit16 {
                    for (d, s) in [(t.x - ct.x, st.x), (t.y - ct.y, st.y), (t.z - ct.z, st.z)] {
                        k16.extend_from_slice(&(q(d, s, 32767.0, -32767, 32767) as i16).to_le_bytes());
                    }
                } else {
                    for (d, s) in [(t.x - ct.x, st.x), (t.y - ct.y, st.y), (t.z - ct.z, st.z)] {
                        k8.push(q(d, s, 127.0, -128, 127) as i8 as u8);
                    }
                }
            }
            st /= if bit16 { 32767.0 } else { 127.0 };
            w.w_u8(
                (if t_present { FL_T_KEY_PRESENT } else { 0 })
                    | (if r_present { 0 } else { FL_R_KEY_ABSENT })
                    | (if bit16 { FL_T_KEY_16 } else { 0 }),
            );
            let rbytes = |keys: &[[i16; 4]]| {
                keys.iter().flat_map(|k| k.iter().flat_map(|v| v.to_le_bytes())).collect::<Vec<u8>>()
            };
            if r_present {
                let b = rbytes(keys_r);
                w.w_u32(crc32(&b));
                w.w(&b);
            } else {
                w.w(&rbytes(&keys_r[..1]));
            }
            if t_present {
                let b = if bit16 { &k16 } else { &k8 };
                w.w_u32(crc32(b));
                w.w(b);
                w.w_vec3(st);
                w.w_vec3(ct);
            } else {
                w.w_vec3(mt);
            }
        }
        w.close_chunk();
    }
    w.close_chunk();
    Ok(w.into_bytes())
}

/// `OGF_S_MOTION_REFS2`, or `OGF_S_SMPARAMS`.
fn motion_defs(obj: &Object, w: &mut Writer) -> Result<()> {
    if !obj.motion_refs.is_empty() {
        w.chunk(chunk::S_MOTION_REFS2, |w| {
            w.w_u32(obj.motion_refs.len() as u32);
            obj.motion_refs.iter().for_each(|r| w.w_stringz(r));
        });
        return Ok(());
    }
    let mut checked = obj.clone();
    let parts = obj.bone_parts.clone();
    if !parts.is_empty() && !checked.verify_bone_parts() {
        return Err(fail("Invalid bone parts (missing or duplicate bones)."));
    }
    let parts = checked.bone_parts;
    for m in &obj.motions {
        if !m.flags.contains(SMotionFlags::FX)
            && !(m.bone_or_part == BI_NONE || usize::from(m.bone_or_part) < parts.len())
        {
            return Err(fail(format!("Invalid Bone Part of motion: '{}'.", m.name)));
        }
    }
    w.open_chunk(chunk::S_SMPARAMS);
    w.w_u16(crate::motion::SMPARAMS_VERSION);
    if parts.is_empty() {
        // the default part writes the bone indices without their names, which the
        // engine's reader expects; such a file does not load
        w.w_u16(1);
        w.w_stringz("default");
        w.w_u16(obj.bones.len() as u16);
        for i in 0..obj.bones.len() {
            w.w_u32(i as u32);
        }
    } else {
        w.w_u16(parts.len() as u16);
        for p in &parts {
            w.w_stringz(&p.alias.to_ascii_lowercase());
            w.w_u16(p.bones.len() as u16);
            for b in &p.bones {
                w.w_stringz(b);
                w.w_u32(obj.bone_index(b).unwrap_or(usize::MAX) as u32);
            }
        }
    }
    w.w_u16(obj.motions.len() as u16);
    for (i, m) in obj.motions.iter().enumerate() {
        w.w_stringz(&m.name);
        w.w_u32(u32::from(m.flags.bits()));
        w.w_u16(m.bone_or_part);
        w.w_u16(i as u16);
        w.w_f32(m.speed);
        w.w_f32(m.power);
        w.w_f32(m.accrue);
        w.w_f32(m.falloff);
        w.w_u32(m.marks.len() as u32);
        for mark in &m.marks {
            w.w_line(&mark.name);
            w.w_u32(mark.intervals.len() as u32);
            for (a, b) in &mark.intervals {
                w.w_f32(*a);
                w.w_f32(*b);
            }
        }
    }
    w.close_chunk();
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gz_fmt_object::{
        Bone, BoneShape, Face, FaceVert, SMotion, SurfFaces, Surface, SurfaceFlags, VMap, VMapKind, VMapPt,
    };

    use super::*;
    use gz_chunk::Reader;

    use crate::Visual;
    use crate::motion::read_omf;

    fn vmap(name: &str, kind: VMapKind, values: &[f32]) -> VMap {
        let n = values.len() / if kind == VMapKind::Uv { 2 } else { 1 };
        VMap {
            name: name.into(),
            kind,
            polymap: false,
            values: values.to_vec(),
            vindices: (0..n as u32).collect(),
            pindices: Vec::new(),
        }
    }

    fn quad_mesh(two_sided_surface: u32) -> Mesh {
        let points: Arc<[Vec3]> = Arc::from(vec![Vec3::ZERO, Vec3::X, Vec3::new(1.0, 0.0, 1.0), Vec3::Z]);
        let fv = |p: u32| FaceVert { point: p, vmref: p };
        let faces: Arc<[Face]> =
            Arc::from(vec![Face { v: [fv(0), fv(1), fv(2)] }, Face { v: [fv(0), fv(2), fv(3)] }]);
        let uv = vmap("uv", VMapKind::Uv, &[0.0, 0.0, 1.0, 0.0, 1.0, 1.0, 0.0, 1.0]);
        Mesh {
            name: "quad".into(),
            points,
            faces,
            smooth_groups: Arc::from(vec![0, 0]),
            vmaps: vec![uv],
            vmrefs: (0..4).map(|i| smallvec::smallvec![VMapPt { vmap: 0, index: i }]).collect(),
            surf_faces: vec![SurfFaces { surface: two_sided_surface, faces: Arc::from(vec![0, 1]) }],
            ..Mesh::default()
        }
    }

    fn object() -> Object {
        Object {
            surfaces: vec![
                Surface {
                    name: "s".into(),
                    texture: "tex\\a".into(),
                    shader: "models\\model".into(),
                    fvf: 0x112,
                    ..Surface::default()
                },
                Surface {
                    name: "d".into(),
                    texture: "tex\\b".into(),
                    shader: "models\\model".into(),
                    fvf: 0x112,
                    flags: SurfaceFlags::TWO_SIDED,
                    ..Surface::default()
                },
            ],
            meshes: vec![quad_mesh(0)],
            ..Object::default()
        }
    }

    #[test]
    fn a_static_object_exports_one_part_without_welding() {
        let e = export_static(&object(), &ExportOptions::default()).unwrap();
        let v = Visual::read(&e.bytes).unwrap();
        assert_eq!(v.kind(), MT::NORMAL);
        assert_eq!(v.texture.as_ref().map(|t| t.0.as_str()), Some("tex\\a"));
        let vb = v.vertices.unwrap();
        assert_eq!(vb.vertices.len(), 6);
        assert_eq!(v.indices.as_deref(), Some(&[0, 1, 2, 3, 4, 5][..]));
        assert_eq!(v.collision_vertices.map(|c| c.len()), Some(6));
        assert_eq!(v.header.bb_max, Vec3::new(1.0, 0.0, 1.0));
    }

    #[test]
    fn two_surfaces_make_a_hierarchy_and_two_sided_faces_are_walked_twice() {
        let mut obj = object();
        let mut second = quad_mesh(1);
        second.name = "back".into();
        obj.meshes.push(second);
        let e = export_static(
            &obj,
            &ExportOptions {
                source_file: "a.object".into(),
                build_name: "\\\\pc\\me".into(),
                materials: None,
            },
        )
        .unwrap();
        let v = Visual::read(&e.bytes).unwrap();
        assert_eq!(v.kind(), MT::HIERRARHY);
        assert_eq!(v.children.len(), 2);
        // two faces, front and back, two passes: eight faces
        assert_eq!(v.children[1].indices.as_ref().map(|i| i.len()), Some(8 * 3));
        assert_eq!(v.desc.map(|d| (d.source_file, d.build_time)), Some(("a.object".into(), 0)));
    }

    fn skeleton() -> Object {
        let mut obj = object();
        obj.flags = ObjectFlags::DYNAMIC;
        let shape = BoneShape { kind: 2, sphere_radius: 0.5, ..BoneShape::default() };
        obj.bones = vec![
            Bone { name: "root".into(), wmap: "root".into(), shape, ..Bone::default() },
            Bone {
                name: "arm".into(),
                parent: "root".into(),
                wmap: "arm".into(),
                parent_id: Some(0),
                rest_offset: Vec3::X,
                shape,
                ..Bone::default()
            },
        ];
        let w = vmap("root", VMapKind::Weight, &[1.0, 1.0, 1.0, 1.0]);
        let m = &mut obj.meshes[0];
        m.vmaps.push(w);
        m.vmrefs = (0..4)
            .map(|i| smallvec::smallvec![VMapPt { vmap: 0, index: i }, VMapPt { vmap: 1, index: i }])
            .collect();
        // no bone motions: every bone keeps its rest pose
        obj.motions =
            vec![SMotion { name: "idle".into(), frame_start: 0, frame_end: 1, ..SMotion::default() }];
        // a skeleton without bone parts gets the X-Ray SDK's default part, which the engine
        // cannot read (`a_skeleton_without_parts_writes_the_unreadable_default_part`)
        obj.bone_parts = vec![gz_fmt_object::BonePart {
            alias: "default".into(),
            bones: vec!["root".into(), "arm".into()],
        }];
        obj
    }

    #[test]
    fn a_skeleton_without_parts_writes_the_unreadable_default_part() {
        // the X-Ray SDK's skeleton exporter writes the bone indices without their names; the engine's
        // `motions_value::load` reads a name before each index, so neither it nor this reader
        // takes such a file
        let mut obj = skeleton();
        obj.bone_parts.clear();
        let e = export_skeleton(&obj, &ExportOptions::default(), 4).unwrap();
        assert!(Visual::read(&e.bytes).is_err());
        let r = Reader::new(&e.bytes);
        let mut p = r.sub(chunk::S_SMPARAMS).unwrap().unwrap();
        assert_eq!(p.r_u16().unwrap(), crate::motion::SMPARAMS_VERSION);
        assert_eq!((p.r_u16().unwrap(), p.r_stringz().unwrap().as_str()), (1, "default"));
        assert_eq!((p.r_u16().unwrap(), p.r_u32().unwrap(), p.r_u32().unwrap()), (2, 0, 1));
    }

    #[test]
    fn a_skeleton_exports_geometry_bones_and_motions() {
        let obj = skeleton();
        let e = export_skeleton(&obj, &ExportOptions::default(), 4).unwrap();
        let v = Visual::read(&e.bytes).unwrap();
        assert_eq!(v.kind(), MT::SKELETON_ANIM);
        assert_eq!(v.bones.iter().map(|b| b.name.as_str()).collect::<Vec<_>>(), ["root", "arm"]);
        assert_eq!(v.bones[1].parent, "root");
        assert_eq!(v.ik.len(), 2);
        assert_eq!(v.children.len(), 1);
        let child = &v.children[0];
        assert_eq!(child.kind(), MT::SKELETON_GEOMDEF_ST);
        let vb = child.vertices.as_ref().unwrap();
        assert_eq!(vb.vertices.len(), 6);
        assert!(vb.vertices.iter().all(|x| x.bones[0] == 0 && x.tangent == Vec3::X && x.binormal == Vec3::Z));
        let motions = v.motions.unwrap();
        assert_eq!(motions.motions.len(), 1);
        assert_eq!(motions.motions[0].length, 2);
        assert_eq!(motions.defs[0].name, "idle");
        // a motion that does not move: one rotation, no translation keys
        assert!(
            motions.motions[0]
                .bones
                .iter()
                .all(|b| b.flags & FL_R_KEY_ABSENT != 0 && b.flags & FL_T_KEY_PRESENT == 0)
        );
        // the arm's rest offset comes back from its constant key
        let pose = motions.pose(0, 0.0).unwrap();
        assert!(similar(pose[1].translation, Vec3::X, 1e-6));
    }

    #[test]
    fn an_omf_holds_the_keys_and_the_definitions() {
        let mut obj = skeleton();
        obj.bone_parts =
            vec![gz_fmt_object::BonePart { alias: "Body".into(), bones: vec!["root".into(), "arm".into()] }];
        let e = export_omf(&obj).unwrap();
        let set = read_omf(&e.bytes).unwrap();
        assert_eq!(set.parts[0].name, "body");
        assert_eq!(set.parts[0].bones, vec![("root".into(), 0), ("arm".into(), 1)]);
        assert_eq!(set.motions[0].bones.len(), 2);
    }

    #[test]
    fn an_object_without_bones_is_refused() {
        let mut obj = object();
        obj.flags = ObjectFlags::DYNAMIC;
        assert!(export_skeleton(&obj, &ExportOptions::default(), 4).is_err());
        assert!(export_static(&Object::default(), &ExportOptions::default()).is_err());
    }

    #[test]
    fn bone_boxes_are_the_minimal_ones() {
        let pts = [Vec3::ZERO, Vec3::new(2.0, 0.0, 0.0), Vec3::new(0.0, 1.0, 0.0), Vec3::new(2.0, 1.0, 0.5)];
        let b = crate::obb::min_box(&pts);
        assert!(b.halfsize.x > 0.0 && b.halfsize.y > 0.0);
        assert_eq!(crate::obb::min_box(&pts[..2]), Obb::INVALID);
    }

    #[test]
    fn key_quantisation_matches_the_original_exporter() {
        // truncation toward zero, and the x86 answer for a NaN
        assert_eq!(cvtt(0.9 * 127.0), 114);
        assert_eq!(cvtt(-0.9 * 127.0), -114);
        assert_eq!(cvtt(f32::NAN).clamp(-128, 127), -128);
    }

    #[test]
    fn a_progressive_skeleton_carries_its_windows() {
        // the split is a triangle soup (no welding), so the pass collapses nothing: one window,
        // the vertices taken last first, no warning about VIPM any more
        let mut obj = skeleton();
        obj.flags |= ObjectFlags::PROGRESSIVE;
        let e = export_skeleton(&obj, &ExportOptions::default(), 4).unwrap();
        assert!(e.warnings.iter().all(|w| !w.contains("VIPM") && !w.contains("progressive")));
        let v = Visual::read(&e.bytes).unwrap();
        let child = &v.children[0];
        assert_eq!(child.kind(), MT::SKELETON_GEOMDEF_PM);
        let swi = child.swi.as_ref().unwrap();
        assert_eq!(swi.reserved, [0; 4]);
        assert_eq!(swi.windows.len(), 1);
        assert_eq!((swi.windows[0].offset, swi.windows[0].num_tris, swi.windows[0].num_verts), (0, 2, 6));
        // the newest face first, its corners renumbered from the last vertex down
        assert_eq!(child.indices.as_deref(), Some(&[2, 1, 0, 5, 4, 3][..]));
    }

    #[test]
    fn a_breakable_part_is_a_rigid_skeleton_of_its_fragments() {
        let face = |o: [Vec3; 3], bone: u16| BreakableFace {
            o,
            n: [Vec3::Y; 3],
            uv: [Vec2::ZERO, Vec2::X, Vec2::Y],
            bone,
            shader: "models\\model".into(),
            texture: "glass\\a".into(),
            two_sided: false,
        };
        let part = BreakablePart {
            faces: vec![
                face([Vec3::ZERO, Vec3::Z, Vec3::X], 0),
                face([Vec3::X, Vec3::Z, Vec3::new(1.0, 0.0, 1.0)], 0),
                face([Vec3::new(2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 1.0), Vec3::new(3.0, 0.0, 0.0)], 1),
            ],
            bones: vec![
                BreakableBone {
                    name: "0".into(),
                    parent: "".into(),
                    material: "materials\\glass".into(),
                    offset: Vec3::ZERO,
                    area: 1.0,
                },
                BreakableBone {
                    name: "1".into(),
                    parent: "0".into(),
                    material: "materials\\glass".into(),
                    offset: Vec3::new(2.0, 0.0, 0.0),
                    area: 0.5,
                },
            ],
        };
        let e = export_breakable(&part).unwrap();
        let v = Visual::read(&e.bytes).unwrap();
        assert_eq!(v.kind(), MT::SKELETON_RIGID);
        assert_eq!(v.children.len(), 1);
        assert_eq!(v.children[0].kind(), MT::SKELETON_GEOMDEF_ST);
        assert_eq!(v.children[0].indices.as_deref(), Some(&[0, 1, 2, 3, 4, 5, 6, 7, 8][..]));
        assert_eq!(
            v.bones.iter().map(|b| (b.name.as_str(), b.parent.as_str())).collect::<Vec<_>>(),
            [("0", ""), ("1", "0")]
        );
        assert_eq!(v.ik.len(), 2);
        let ik = &v.ik[1];
        assert_eq!((ik.joint_type, ik.flags, ik.shape.kind, ik.shape.flags), (4, 1, 1, 2));
        assert_eq!(ik.limits[0][0].to_bits(), (-0.0f32).to_bits());
        assert_eq!((ik.rest_offset, ik.mass), (Vec3::new(2.0, 0.0, 0.0), 0.5));
        // a fragment's box is fitted to its points relative to its offset
        assert!(ik.shape.box_translate.abs_diff_eq(Vec3::new(1.0 / 3.0, 0.0, 1.0 / 3.0), 0.35));
        assert!(export_breakable(&BreakablePart { bones: vec![part.bones[0].clone(); 64], ..part }).is_err());
    }
}
