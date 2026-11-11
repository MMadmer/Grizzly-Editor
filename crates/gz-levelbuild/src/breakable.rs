//! The breakable and climable objects `BuildGame` adds to `level.spawn`
//! (`ESceneObjectToolsExportBreakable.cpp` over `GeometryPartExtractor.cpp`).
//!
//! The faces of the static scene objects whose game material is breakable (climable) are
//! gathered in world space; the parts are their connected sets; each part is boxed (RAPID), laid
//! into its box's frame and - when every face of it has a neighbour - cut into fragments that
//! become the bones of a rigid skeleton (`meshes\brkbl#N.ogf` beside the level, a
//! `breakable_object` naming it) or a box shape (`climable_object`).
//!
//! What the C++ does, kept because the files follow it:
//! - `VCPacked::add_vert` of this editor no longer welds (its hash grid was taken out): every
//!   face corner is a vertex of its own, so no two faces share one, every face is a part of its
//!   own, has no neighbour, and `prepare` refuses it ("Error face found at pos ..."). A build
//!   therefore writes no breakable or climable object, only those errors and "Can't export
//!   invalid part #N" per face. The rest is ported all the same and runs where parts have
//!   neighbours.
//! - The fragments' sizes come from `Random.randI`, the engine's global generator seeded from
//!   the CPU clock at start and advanced by everything before; here a generator of its own
//!   starts at 1 (`CRandom`'s default).
//! - The surfaces of a mesh are walked in file order (the C++ walks a map keyed by surface
//!   pointers), which numbers the parts.
//! - A failing part leaves the C++'s `brkbl#N.ogf` files of the parts before it on the disk;
//!   here a failing build writes none.

use glam::{Vec2, Vec3};
use gz_entities::{Entity, PropContext, PropValue, Settings, Shape};
use gz_fmt_level::i_floor;
use gz_base::SmolStr;
use gz_fmt_object::xmath::{Aabb, Mat4};
use gz_fmt_ogf::{BreakableBone, BreakableFace, BreakablePart, Obb, export_breakable, rapid_box};
use gz_scene::{ObjClass, ObjData, SceneState};

use crate::libs::{Engine, mtl_flags};
use crate::prj::Lookup;
use crate::validate::lmapped;
use crate::{BuildError, Result};

/// What the two exports add to the level.
#[derive(Clone, Debug, Default)]
pub struct Exported {
    /// The spawn packets, in the order they open their chunks of `level.spawn`.
    pub spawns: Vec<Vec<u8>>,
    /// `meshes\brkbl#N.ogf` and their bytes, relative to the level folder.
    pub meshes: Vec<(String, Vec<u8>)>,
    pub log: Vec<String>,
}

/// The surface a face comes from.
#[derive(Clone, Debug)]
struct Surf {
    shader: SmolStr,
    texture: SmolStr,
    game_mtl: SmolStr,
    two_sided: bool,
}

/// `SBFace`.
#[derive(Clone, Debug)]
struct Face {
    vert_id: [usize; 3],
    marked: bool,
    adjs: Vec<usize>,
    o: [Vec3; 3],
    n: [Vec3; 3],
    uv: [Vec2; 3],
    bone_id: i32,
    surf: usize,
}

impl Face {
    /// `CalcArea`.
    fn area(&self) -> f32 {
        let v0 = sub(self.o[1], self.o[0]);
        let v1 = sub(self.o[2], self.o[0]);
        let sqm0 = v0.x * v0.x + v0.y * v0.y + v0.z * v0.z;
        let sqm1 = v1.x * v1.x + v1.y * v1.y + v1.z * v1.z;
        let d = v0.x * v1.x + v0.y * v1.y + v0.z * v1.z;
        0.5 * (sqm0 * sqm1 - d * d).sqrt()
    }
}

fn sub(a: Vec3, b: Vec3) -> Vec3 {
    Vec3::new(a.x - b.x, a.y - b.y, a.z - b.z)
}

/// `SBBone`.
#[derive(Clone, Debug)]
struct Bone {
    name: String,
    parent: String,
    mtl: SmolStr,
    offset: Vec3,
    f_cnt: u32,
    area: f32,
}

/// `SBPart`.
#[derive(Clone, Debug)]
struct Part {
    faces: Vec<usize>,
    bones: Vec<Bone>,
    bbox: Aabb,
    obb: Obb,
    ref_offset: Vec3,
    ref_rotate: Vec3,
    valid: bool,
}

/// `CRandom` with the C++ remainder semantics of `randI(min, max)`.
struct Random(i32);

impl Random {
    fn rand_i(&mut self) -> i32 {
        self.0 = self.0.wrapping_mul(214_013).wrapping_add(2_531_011);
        (self.0 >> 16) & 0x7fff
    }

    /// `randI(min, max)`: `min + randI() % (max - min)`, the remainder taking the dividend's
    /// sign; `None` where the C++ divides by zero.
    fn range(&mut self, min: i32, max: i32) -> Option<i32> {
        let span = max.wrapping_sub(min);
        if span == 0 {
            return None;
        }
        Some(min.wrapping_add(self.rand_i() % span))
    }
}

/// `Fmatrix::set(i, j, k, c)`.
fn obb_matrix(rotate: &[Vec3; 3], c: Vec3) -> Mat4 {
    let [i, j, k] = rotate;
    Mat4([[i.x, i.y, i.z, 0.0], [j.x, j.y, j.z, 0.0], [k.x, k.y, k.z, 0.0], [c.x, c.y, c.z, 1.0]])
}

/// `Fmatrix::getXYZ`: `getHPB(y, x, z)`.
fn get_xyz(m: &Mat4) -> Vec3 {
    let (i, j, k) = (m.row(0), m.row(1), m.row(2));
    let cy = (j.y * j.y + i.y * i.y).sqrt();
    let (h, p, b) = if cy > 16.0 * f32::EPSILON {
        (-k.x.atan2(k.z), -(-k.y).atan2(cy), -i.y.atan2(j.y))
    } else {
        (-(-i.z).atan2(i.x), -(-k.y).atan2(cy), 0.0)
    };
    Vec3::new(p, h, b)
}

/// `Fvector::normalize`: no guard.
fn normalize(v: Vec3) -> Vec3 {
    let mag = (1.0 / (v.x * v.x + v.y * v.y + v.z * v.z)).sqrt();
    Vec3::new(v.x * mag, v.y * mag, v.z * mag)
}

/// `Fvector::normalize_safe`.
fn normalize_safe(v: Vec3) -> Vec3 {
    let m = v.x * v.x + v.y * v.y + v.z * v.z;
    if m > f32::MIN_POSITIVE {
        let m = (1.0 / m).sqrt();
        Vec3::new(v.x * m, v.y * m, v.z * m)
    } else {
        v
    }
}

fn f2(v: f32) -> String {
    format!("{:.2}", f64::from(v))
}

/// `CGeomPartExtractor`.
struct Extractor {
    surfaces: Vec<Surf>,
    faces: Vec<Face>,
    verts: usize,
    adjs: Vec<Vec<usize>>,
    parts: Vec<Part>,
    face_min: u32,
}

impl Extractor {
    fn new(face_min: u32) -> Self {
        Self { surfaces: Vec::new(), faces: Vec::new(), verts: 0, adjs: Vec::new(), parts: Vec::new(), face_min }
    }

    /// `AppendFace`; `VCPacked::add_vert` hands out a new vertex for every corner.
    fn append_face(&mut self, surf: usize, v: [Vec3; 3], n: [Vec3; 3], uv: [Vec2; 3]) {
        let mut vert_id = [0usize; 3];
        for id in &mut vert_id {
            *id = self.verts;
            self.verts += 1;
        }
        self.faces.push(Face { vert_id, marked: false, adjs: Vec::new(), o: v, n, uv, bone_id: -1, surf });
    }

    /// `Process`: the faces around every vertex, the connected parts, each prepared.
    fn process(&mut self, rng: &mut Random, log: &mut Vec<String>) -> Result<()> {
        self.adjs = vec![Vec::new(); self.verts];
        for (fi, f) in self.faces.iter().enumerate() {
            for &v in &f.vert_id {
                if let Some(a) = self.adjs.get_mut(v) {
                    a.push(fi);
                }
            }
        }
        for start in 0..self.faces.len() {
            if self.faces[start].marked {
                continue;
            }
            // `recurse_tri`, depth first in the C++ order
            let mut part = Vec::new();
            let mut stack: Vec<(usize, usize, usize)> = Vec::new();
            self.faces[start].marked = true;
            part.push(start);
            stack.push((start, 0, 0));
            while let Some((f, k, i)) = stack.last_mut() {
                if *k >= 3 {
                    stack.pop();
                    continue;
                }
                let v = self.faces[*f].vert_id[*k];
                let list = self.adjs.get(v).map_or(&[][..], Vec::as_slice);
                if *i >= list.len() {
                    *k += 1;
                    *i = 0;
                    continue;
                }
                let next = list[*i];
                *i += 1;
                if self.faces[next].marked {
                    continue;
                }
                self.faces[next].marked = true;
                part.push(next);
                stack.push((next, 0, 0));
            }
            self.parts.push(Part {
                faces: part,
                bones: Vec::new(),
                bbox: Aabb::INVALID,
                obb: Obb::INVALID,
                ref_offset: Vec3::ZERO,
                ref_rotate: Vec3::ZERO,
                valid: true,
            });
        }
        for p in 0..self.parts.len() {
            self.prepare(p, rng, log)?;
        }
        Ok(())
    }

    /// `SBPart::use_face`.
    fn use_face(&mut self, f: usize, cnt: &mut u32, bone: u32, area: &mut f32) {
        let face = &mut self.faces[f];
        face.marked = true;
        face.bone_id = bone as i32;
        *area += face.area();
        *cnt += 1;
    }

    /// `SBPart::recurse_fragment`, its recursion as a stack.
    fn recurse_fragment(&mut self, start: usize, cnt: &mut u32, bone: u32, max: u32, area: &mut f32) {
        let mut stack: Vec<(Vec<usize>, usize)> = vec![(self.fill_nearest(start, cnt, bone, max, area), 0)];
        while let Some((list, i)) = stack.last_mut() {
            if *i >= list.len() || *cnt >= max {
                stack.pop();
                continue;
            }
            let a = list[*i];
            *i += 1;
            if self.faces[a].bone_id != bone as i32 {
                continue;
            }
            let next = self.fill_nearest(a, cnt, bone, max, area);
            stack.push((next, 0));
        }
    }

    /// The first half of `recurse_fragment`: the face itself, then its free neighbours.
    fn fill_nearest(&mut self, f: usize, cnt: &mut u32, bone: u32, max: u32, area: &mut f32) -> Vec<usize> {
        if !self.faces[f].marked {
            self.use_face(f, cnt, bone, area);
        }
        let mut out = Vec::new();
        let adjs = self.faces[f].adjs.clone();
        for n in adjs {
            if *cnt >= max {
                break;
            }
            if self.faces[n].marked {
                continue;
            }
            self.use_face(n, cnt, bone, area);
            out.push(n);
        }
        out
    }

    /// `SBPart::prepare`.
    fn prepare(&mut self, p: usize, rng: &mut Random, log: &mut Vec<String>) -> Result<()> {
        let faces = self.parts[p].faces.clone();
        let mut pts = Vec::with_capacity(faces.len() * 3);
        for &f in &faces {
            self.faces[f].marked = false;
            pts.extend_from_slice(&self.faces[f].o);
        }
        let obb = rapid_box(&pts);
        // the neighbours: faces sharing two vertices
        for &a in &faces {
            for k in 0..3 {
                let v = self.faces[a].vert_id[k];
                let list = self.adjs.get(v).cloned().unwrap_or_default();
                for b in list {
                    if a == b {
                        continue;
                    }
                    let (va, vb) = (self.faces[a].vert_id, self.faces[b].vert_id);
                    let shared = va.iter().map(|x| vb.iter().filter(|y| *y == x).count()).sum::<usize>();
                    if shared >= 2 {
                        if !self.faces[a].adjs.contains(&b) {
                            self.faces[a].adjs.push(b);
                        }
                        if !self.faces[b].adjs.contains(&a) {
                            self.faces[b].adjs.push(a);
                        }
                    }
                }
            }
        }
        let m = obb_matrix(&obb.rotate, obb.translate);
        let ref_offset = obb.translate;
        let ref_rotate = get_xyz(&m);
        let inv = Mat4::invert_43(&m);
        let mut bbox = Aabb::INVALID;
        let mut valid = true;
        for &f in &faces {
            let face = &mut self.faces[f];
            if face.adjs.is_empty() {
                let o = face.o[0];
                log.push(format!("Error face found at pos: [{},{},{}]", f2(o.x), f2(o.y), f2(o.z)));
                valid = false;
            }
            for k in 0..3 {
                face.n[k] = inv.transform_dir(face.n[k]);
                face.o[k] = inv.transform_tiny(face.o[k]);
                bbox.modify(face.o[k]);
            }
        }
        let part = &mut self.parts[p];
        part.obb = obb;
        part.ref_offset = ref_offset;
        part.ref_rotate = ref_rotate;
        part.bbox = bbox;
        part.valid = valid;
        if valid {
            let bones = self.fragments(&faces, rng)?;
            self.parts[p].bones = bones;
        }
        Ok(())
    }

    /// The bone half of `prepare`: fragments grown from free faces, single faces attached.
    fn fragments(&mut self, faces: &[usize], rng: &mut Random) -> Result<Vec<Bone>> {
        let n = faces.len();
        let calc = i_floor(n as f32 / self.face_min as f32);
        let cnt_max = if calc < 62 {
            if calc <= 0 { 1 } else { calc }
        } else {
            62
        };
        let mut face_max = i_floor(n as f32 / cnt_max as f32 + 0.5);
        face_max = (face_max as f32 * 4.0) as i32;
        let mut bones: Vec<Bone> = Vec::new();
        let mut bone_idx: i32 = 0;
        let mut total: u32 = 0;
        let mut parent = String::new();
        loop {
            let mut found = None;
            for &f in faces {
                if !self.faces[f].marked {
                    found = Some(f);
                    let free = self.faces[f].adjs.iter().filter(|a| !self.faces[**a].marked).count();
                    if free == 0 || free >= 2 {
                        break;
                    }
                }
            }
            let Some(f) = found else { break };
            let mut area = 0.0f32;
            let mut accum = 0u32;
            let max_count = rng
                .range(self.face_min as i32, face_max.wrapping_add(1))
                .ok_or_else(|| BuildError::new("Breakable object: a fragment of no faces was asked for."))?
                as u32;
            self.recurse_fragment(f, &mut accum, bone_idx as u32, max_count, &mut area);
            if accum == 1 {
                self.faces[f].bone_id = -1;
            } else {
                let mtl = self.surfaces.get(self.faces[f].surf).map(|s| s.game_mtl.clone()).unwrap_or_default();
                bones.push(Bone {
                    name: bone_idx.to_string(),
                    parent: parent.clone(),
                    mtl,
                    offset: Vec3::ZERO,
                    f_cnt: accum,
                    area,
                });
                parent = "0".to_owned();
                bone_idx += 1;
                total += accum;
            }
            if bone_idx >= cnt_max {
                break;
            }
        }

        // the faces no fragment took go to a neighbour's fragment; the first pass sends every
        // one of them to fragment 0 (`face_accum_total_saved` starts equal)
        let mut saved = total;
        while (total as usize) < n {
            if saved != total {
                saved = total;
            } else {
                let before = total;
                for &f in faces {
                    if self.faces[f].bone_id == -1 {
                        self.faces[f].marked = true;
                        self.faces[f].bone_id = 0;
                        total += 1;
                    }
                }
                // the C++ would loop for ever here; the counts never let it
                if before == total {
                    break;
                }
            }
            for &f in faces {
                if self.faces[f].bone_id != -1 {
                    continue;
                }
                let mut parent_face = None;
                if self.faces[f].adjs.is_empty() {
                    self.faces[f].marked = true;
                    self.faces[f].bone_id = 0;
                    total += 1;
                } else {
                    parent_face = self.faces[f].adjs.iter().copied().find(|a| self.faces[*a].bone_id != -1);
                }
                if let Some(pf) = parent_face {
                    self.faces[f].marked = true;
                    self.faces[f].bone_id = self.faces[pf].bone_id;
                    total += 1;
                }
            }
        }

        // the fragments' centres; the first one's goes to zero, and the others keep theirs
        // (oracle: `offs` is a reference to the first bone's offset, zeroed by the first pass)
        for &f in faces {
            let face = &self.faces[f];
            let b = usize::try_from(face.bone_id)
                .ok()
                .and_then(|b| bones.get_mut(b))
                .ok_or_else(|| BuildError::new("Breakable object: a face belongs to no fragment."))?;
            for k in 0..3 {
                b.offset = Vec3::new(b.offset.x + face.o[k].x, b.offset.y + face.o[k].y, b.offset.z + face.o[k].z);
            }
        }
        for b in &mut bones {
            let s = (b.f_cnt * 3) as f32;
            b.offset = Vec3::new(b.offset.x / s, b.offset.y / s, b.offset.z / s);
        }
        for i in 0..bones.len() {
            let offs = bones[0].offset;
            bones[i].offset = sub(bones[i].offset, offs);
        }
        Ok(bones)
    }

    /// The part as `SBPart::Export` reads it.
    fn ogf_part(&self, p: &Part) -> BreakablePart {
        let faces = p
            .faces
            .iter()
            .map(|&f| {
                let face = &self.faces[f];
                let s = self.surfaces.get(face.surf);
                BreakableFace {
                    o: face.o,
                    n: face.n,
                    uv: face.uv,
                    bone: face.bone_id as u16,
                    shader: s.map(|s| s.shader.clone()).unwrap_or_default(),
                    texture: s.map(|s| s.texture.clone()).unwrap_or_default(),
                    two_sided: s.is_some_and(|s| s.two_sided),
                }
            })
            .collect();
        let bones = p
            .bones
            .iter()
            .map(|b| BreakableBone {
                name: b.name.as_str().into(),
                parent: b.parent.as_str().into(),
                material: b.mtl.clone(),
                offset: b.offset,
                area: b.area,
            })
            .collect();
        BreakablePart { faces, bones }
    }
}

/// `build_mesh` for every mesh of every static scene object: the faces of the surfaces whose
/// game material has `mask`, in world space. `Err` with the C++ messages when a material is
/// unknown or (for breakables) an engine shader is missing or bakes into a light map; the
/// objects after a failing one are still gathered, as the C++ breaks the mesh loop only.
fn gather(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    mask: u32,
    ignore_shader: bool,
    ex: &mut Extractor,
    log: &mut Vec<String>,
) -> bool {
    let mut ok = true;
    for o in scene.of_class(ObjClass::SceneObject) {
        let ObjData::SceneObject(d) = &o.data else { continue };
        let Some(lo) = lib(&d.reference) else { continue };
        let f = lo.object.flags;
        let statik = !f.intersects(
            gz_fmt_object::ObjectFlags::SOUND_OCCLUDER
                | gz_fmt_object::ObjectFlags::DYNAMIC
                | gz_fmt_object::ObjectFlags::HOM
                | gz_fmt_object::ObjectFlags::MULTIPLE_USAGE,
        );
        if !statik {
            continue;
        }
        let xf = o.world();
        'meshes: for mesh in &lo.object.meshes {
            let normals = mesh.vertex_normals();
            for sf in &mesh.surf_faces {
                let Some(s) = lo.object.surfaces.get(sf.surface as usize) else { continue };
                let Some(id) = engine.material_id(&s.game_mtl) else {
                    log.push(format!("Object '{}', surface '{}' contain invalid game material.", lo.name, s.name));
                    ok = false;
                    break 'meshes;
                };
                let Some(flags) = engine.material_flags(id) else {
                    log.push(format!("Object '{}', surface '{}' contain undefined game material.", lo.name, s.name));
                    ok = false;
                    break 'meshes;
                };
                if flags & mask == 0 {
                    continue;
                }
                if !ignore_shader {
                    if !engine.has_shader(&s.shader) {
                        log.push(format!(
                            "Can't find engine shader '{}'. Object '{}', surface '{}'. Export interrupted.",
                            s.shader, lo.name, s.name
                        ));
                        ok = false;
                        break 'meshes;
                    }
                    if lmapped(engine, &s.shader) == Some(true) {
                        log.push(format!(
                            "Object '{}', surface '{}' contain static engine shader - '{}'. Export interrupted.",
                            lo.name, s.name, s.shader
                        ));
                        ok = false;
                        break 'meshes;
                    }
                }
                let surf = ex.surfaces.len();
                ex.surfaces.push(Surf {
                    shader: s.shader.clone(),
                    texture: s.texture.clone(),
                    game_mtl: s.game_mtl.clone(),
                    two_sided: s.two_sided(),
                });
                for &fi in sf.faces.iter() {
                    let Some(face) = mesh.faces.get(fi as usize) else { continue };
                    let mut v = [Vec3::ZERO; 3];
                    let mut n = [Vec3::ZERO; 3];
                    let mut uv = [Vec2::ZERO; 3];
                    for k in 0..3 {
                        let p = mesh.points.get(face.v[k].point as usize).copied().unwrap_or(Vec3::ZERO);
                        v[k] = xf.transform_tiny(p);
                        let vn = normals.get(fi as usize * 3 + k).copied().unwrap_or(Vec3::Y);
                        n[k] = normalize(xf.transform_dir(vn));
                        uv[k] = mesh.corner_uv(face.v[k]).unwrap_or(Vec2::ZERO);
                    }
                    ex.append_face(surf, v, n, uv);
                }
            }
        }
    }
    ok
}

/// `ESceneObjectTool::GetBox`: whether any scene object has a box.
fn tool_has_box(scene: &SceneState, lib: Lookup<'_>) -> bool {
    let local = |r: &str| lib(r).map(|o| o.bounds);
    scene.of_class(ObjClass::SceneObject).any(|o| gz_scene::objects::world_box(scene, o, &local).is_some())
}

/// An entity of `section`, named and placed.
fn entity(section: &str, settings: Option<&(dyn Settings + Sync)>, name: &str, pos: Vec3, rot: Vec3) -> Result<Entity> {
    let settings =
        settings.ok_or_else(|| BuildError::new(format!("Can't create '{section}': the game's configs are not loaded.")))?;
    let mut e = Entity::create(section, settings)
        .map_err(|err| BuildError::new(format!("Can't create '{section}': {err}")))?;
    let _ = e.set_name(name);
    let _ = e.set_position(pos);
    let _ = e.set_angle(rot);
    Ok(e)
}

/// `ExportBreakableObjects`: false when the export failed (the build fails then).
fn breakables(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    settings: Option<&(dyn Settings + Sync)>,
    rng: &mut Random,
    out: &mut Exported,
) -> Result<bool> {
    if !tool_has_box(scene, lib) {
        return Ok(false);
    }
    let mut ex = Extractor::new(2);
    let mut ok = gather(scene, lib, engine, mtl_flags::BREAKABLE, false, &mut ex, &mut out.log);
    ex.process(rng, &mut out.log)?;
    if !ok {
        return Ok(false);
    }
    for (i, p) in ex.parts.iter().enumerate() {
        if !p.valid {
            out.log.push(format!("Can't export invalid part #{i}"));
            continue;
        }
        let sn = format!("meshes\\brkbl#{i}.ogf");
        match export_breakable(&ex.ogf_part(p)) {
            Ok(e) => {
                out.log.extend(e.warnings);
                out.meshes.push((sn.clone(), e.bytes));
            }
            Err(err) => {
                out.log.push(err.to_string());
                out.log.push("Invalid breakable object.".to_owned());
                ok = false;
                break;
            }
        }
        let mut e = entity("breakable_object", settings, &sn, p.ref_offset, p.ref_rotate)?;
        let ctx = PropContext::default();
        // `set_visual`: the name without its extension, lower case
        let visual = e
            .properties(&ctx)
            .into_iter()
            .find(|pr| pr.path.ends_with("\\Model\\Visual"))
            .ok_or_else(|| BuildError::new(format!("Can't set the visual of '{sn}': the entity has none.")))?;
        e.set(&visual.path, &PropValue::Text(sn.clone()), &ctx)
            .map_err(|err| BuildError::new(format!("Can't set the visual of '{sn}': {err}")))?;
        out.spawns.push(e.to_spawn_packet());
    }
    Ok(ok)
}

/// `OrientToNorm`: the box's thinnest axis must be the one the normal leans on most; that axis
/// (and the next) turn to face it.
fn orient_to_norm(n: &mut Vec3, rotate: &mut [Vec3; 3], hs: Vec3) -> bool {
    let (mut max_proj, mut min_size) = (0usize, 0usize);
    for k in 1..3 {
        if n[k].abs() > n[max_proj].abs() {
            max_proj = k;
        }
        if hs[k] < hs[min_size] {
            min_size = k;
        }
    }
    if min_size != max_proj {
        return false;
    }
    if n[max_proj] < 0.0 {
        *n = -*n;
        rotate[max_proj] = -rotate[max_proj];
        rotate[(max_proj + 1) % 3] = -rotate[(max_proj + 1) % 3];
    }
    true
}

/// The climable's material (`set_additional_info`): `gz-entities` writes the class default,
/// the last string of the state; it is put in its place and the state's size made right.
fn with_material(packet: Vec<u8>, default: &str, material: &str) -> Option<Vec<u8>> {
    let (old, _) = gz_base::text::utf8_to_cp1251(default);
    let (new, _) = gz_base::text::utf8_to_cp1251(material);
    let tail = old.len() + 1;
    let cut = packet.len().checked_sub(tail)?;
    if packet.get(cut..packet.len() - 1)? != &old[..] || packet.last() != Some(&0) {
        return None;
    }
    let at = crate::spawn::state_size_offset(&packet)?;
    let size = u16::from_le_bytes([*packet.get(at)?, *packet.get(at + 1)?]);
    let size = usize::from(size).checked_sub(tail)? + new.len() + 1;
    let mut out = packet;
    out.truncate(cut);
    out.extend_from_slice(&new);
    out.push(0);
    out.get_mut(at..at + 2)?.copy_from_slice(&u16::try_from(size).ok()?.to_le_bytes());
    Some(out)
}

/// `ExportClimableObjects`.
fn climables(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    settings: Option<&(dyn Settings + Sync)>,
    rng: &mut Random,
    out: &mut Exported,
) -> Result<bool> {
    if !tool_has_box(scene, lib) {
        return Ok(false);
    }
    let mut ex = Extractor::new(i32::MAX as u32);
    let ok = gather(scene, lib, engine, mtl_flags::CLIMABLE, true, &mut ex, &mut out.log);
    ex.process(rng, &mut out.log)?;
    if !ok {
        return Ok(false);
    }
    for i in 0..ex.parts.len() {
        if !ex.parts[i].valid {
            out.log.push(format!("Can't export invalid part #{i}"));
            continue;
        }
        let sn = format!("clmbl#{i}");
        let mut normal = Vec3::ZERO;
        let mut material = SmolStr::default();
        for &f in &ex.parts[i].faces {
            let face = &ex.faces[f];
            for k in 0..3 {
                normal = Vec3::new(normal.x + face.n[k].x, normal.y + face.n[k].y, normal.z + face.n[k].z);
            }
            material = ex.surfaces.get(face.surf).map(|s| s.game_mtl.clone()).unwrap_or_default();
        }
        let mut normal = normalize_safe(normal);
        let p = &mut ex.parts[i];
        let b = p.bbox;
        let half = Vec3::new((b.max.x - b.min.x) * 0.5, (b.max.y - b.min.y) * 0.5, (b.max.z - b.min.z) * 0.5);
        let shape = Shape::Box {
            i: Vec3::new(half.x, 0.0, 0.0),
            j: Vec3::new(0.0, half.y, 0.0),
            k: Vec3::new(0.0, 0.0, half.z),
            c: Vec3::ZERO,
        };
        if !orient_to_norm(&mut normal, &mut p.obb.rotate, p.obb.halfsize) {
            let o = p.ref_offset;
            out.log.push(format!("Invalid climable object found. [{}, {}, {}]", f2(o.x), f2(o.y), f2(o.z)));
            continue;
        }
        let m = obb_matrix(&p.obb.rotate, p.obb.translate);
        p.ref_rotate = get_xyz(&m);
        let mut e = entity("climable_object", settings, &sn, p.ref_offset, p.ref_rotate)?;
        e.set_shapes(&[shape])
            .map_err(|err| BuildError::new(format!("Can't shape '{sn}': {err}")))?;
        let packet = e.to_spawn_packet();
        let default = gz_entities::model::Climable::default().material;
        let packet = if material == default {
            packet
        } else {
            with_material(packet, &default, &material)
                .ok_or_else(|| BuildError::new(format!("Can't set the material of '{sn}'.")))?
        };
        out.spawns.push(packet);
    }
    Ok(ok)
}

/// `ESceneObjectTool::ExportGame` past its objects: the breakable objects, then the climable
/// ones. `Ok(false)` with the log when the C++ export fails.
pub fn export(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    settings: Option<&(dyn Settings + Sync)>,
) -> Result<(bool, Exported)> {
    let mut out = Exported::default();
    let mut rng = Random(1);
    if !breakables(scene, lib, engine, settings, &mut rng, &mut out)? {
        return Ok((false, out));
    }
    if !climables(scene, lib, engine, settings, &mut rng, &mut out)? {
        return Ok((false, out));
    }
    Ok((true, out))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn handed(r: &[Vec3; 3]) -> f32 {
        r[0].cross(r[1]).dot(r[2])
    }

    /// A box of 12 triangles around `c` of half size `h`, its corners shared as `weld` says.
    fn box_faces(ex: &mut Extractor, c: Vec3, h: f32, weld: bool) {
        let corner = |i: usize| {
            Vec3::new(
                c.x + if i & 1 != 0 { h } else { -h },
                c.y + if i & 2 != 0 { h } else { -h },
                c.z + if i & 4 != 0 { h } else { -h },
            )
        };
        let quads = [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let base = ex.verts;
        if weld {
            ex.verts += 8;
        }
        for q in quads {
            for t in [[q[0], q[1], q[2]], [q[0], q[2], q[3]]] {
                let v = t.map(corner);
                if weld {
                    ex.faces.push(Face {
                        vert_id: t.map(|i| base + i),
                        marked: false,
                        adjs: Vec::new(),
                        o: v,
                        n: [Vec3::Y; 3],
                        uv: [Vec2::ZERO; 3],
                        bone_id: -1,
                        surf: 0,
                    });
                } else {
                    ex.append_face(0, v, [Vec3::Y; 3], [Vec2::ZERO; 3]);
                }
            }
        }
    }

    fn surf() -> Surf {
        Surf {
            shader: "models\\model".into(),
            texture: "glass\\a".into(),
            game_mtl: "materials\\glass".into(),
            two_sided: false,
        }
    }

    #[test]
    fn two_boxes_without_welding_give_a_part_per_face_none_valid() {
        // what the C++ build does with any scene: no corner is shared
        let mut ex = Extractor::new(2);
        ex.surfaces.push(surf());
        box_faces(&mut ex, Vec3::ZERO, 1.0, false);
        box_faces(&mut ex, Vec3::new(2.0, 0.0, 0.0), 1.0, false);
        let mut log = Vec::new();
        ex.process(&mut Random(1), &mut log).unwrap();
        assert_eq!(ex.parts.len(), 24);
        assert!(ex.parts.iter().all(|p| !p.valid && p.faces.len() == 1));
        assert_eq!(log.len(), 24);
        assert_eq!(log[0], "Error face found at pos: [-1.00,-1.00,-1.00]");
    }

    #[test]
    fn two_connected_boxes_give_two_parts_with_fragments() {
        // the machinery past `add_vert`, on corners shared within each box: two parts
        let mut ex = Extractor::new(2);
        ex.surfaces.push(surf());
        box_faces(&mut ex, Vec3::ZERO, 1.0, true);
        box_faces(&mut ex, Vec3::new(2.0, 0.0, 0.0), 1.0, true);
        let mut log = Vec::new();
        ex.process(&mut Random(1), &mut log).unwrap();
        assert!(log.is_empty(), "{log:?}");
        assert_eq!(ex.parts.len(), 2);
        for p in &ex.parts {
            assert!(p.valid);
            assert_eq!(p.faces.len(), 12);
            // every face belongs to a fragment, the first fragment sits at zero
            assert!(p.faces.iter().all(|f| ex.faces[*f].bone_id >= 0));
            assert!(!p.bones.is_empty() && p.bones[0].offset == Vec3::ZERO);
            assert_eq!(p.bones[0].parent, "");
            assert!(p.bones.iter().skip(1).all(|b| b.parent == "0"));
            // the part's frame is the box's, a right-handed one, its box around the origin
            assert!((handed(&p.obb.rotate) - 1.0).abs() < 1e-4);
            assert!(p.bbox.min.abs_diff_eq(-p.bbox.max, 1e-4));
        }
        assert!(ex.parts[1].ref_offset.abs_diff_eq(Vec3::new(2.0, 0.0, 0.0), 1e-4));
        // the part exports as a rigid skeleton
        let e = export_breakable(&ex.ogf_part(&ex.parts[0])).unwrap();
        let v = gz_fmt_ogf::Visual::read(&e.bytes).unwrap();
        assert_eq!(v.kind(), gz_fmt_ogf::MT::SKELETON_RIGID);
        assert_eq!(v.bones.len(), ex.parts[0].bones.len());
    }

    #[test]
    fn a_scene_of_two_climable_boxes_exports_what_the_cpp_does() {
        use std::path::PathBuf;
        use std::sync::Arc;

        use gz_fmt_object::{Face as MFace, FaceVert, Mesh, Object as EditObject, SurfFaces, Surface};
        use gz_fmt_xr::{GameMaterial, GameMaterials};
        use gz_scene::Object;
        use gz_scene::library::LibraryObject;
        use gz_scene::objects::SceneObjectData;

        let corners: Vec<Vec3> = (0..8)
            .map(|i| Vec3::new(if i & 1 != 0 { 1.0 } else { -1.0 }, if i & 2 != 0 { 1.0 } else { -1.0 }, if i & 4 != 0 { 1.0 } else { -1.0 }))
            .collect();
        let quads = [[0u32, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let fv = |p: u32| FaceVert { point: p, vmref: 0 };
        let faces: Vec<MFace> = quads
            .iter()
            .flat_map(|q| [MFace { v: [fv(q[0]), fv(q[1]), fv(q[2])] }, MFace { v: [fv(q[0]), fv(q[2]), fv(q[3])] }])
            .collect();
        let mesh = Mesh {
            name: "box".into(),
            points: Arc::from(corners),
            faces: Arc::from(faces),
            smooth_groups: Arc::from(vec![0u32; 12]),
            surf_faces: vec![SurfFaces { surface: 0, faces: Arc::from((0..12).collect::<Vec<u32>>()) }],
            ..Mesh::default()
        };
        let object = EditObject {
            surfaces: vec![Surface {
                name: "s".into(),
                texture: "wood\\a".into(),
                shader: "models\\model".into(),
                game_mtl: "materials\\ladder".into(),
                fvf: 0x112,
                ..Surface::default()
            }],
            meshes: vec![mesh],
            ..EditObject::default()
        };
        let lo = Arc::new(LibraryObject::from_object("props\\ladder", PathBuf::new(), object));
        let look = move |_: &str| Some(lo.clone());
        let ladder = GameMaterial {
            id: 7,
            name: "materials\\ladder".into(),
            desc: "".into(),
            flags: mtl_flags::CLIMABLE,
            friction: 1.0,
            damping: 1.0,
            spring: 1.0,
            bounce_start_velocity: 0.0,
            bouncing: 0.0,
            shoot_factor: 0.0,
            bounce_damage_factor: 0.0,
            vis_transparency_factor: 0.0,
            snd_occlusion_factor: 0.0,
            shoot_factor_mp: 0.0,
            flotation_factor: None,
            injurious_speed: None,
            density_factor: None,
        };
        let engine = Engine {
            materials: Some(GameMaterials { materials: vec![ladder], ..GameMaterials::default() }),
            ..Engine::default()
        };
        let mut scene = SceneState::default();
        for (i, x) in [0.0f32, 2.0].into_iter().enumerate() {
            let mut o = Object::new(
                ObjClass::SceneObject,
                &format!("ladder_{i}"),
                ObjData::SceneObject(SceneObjectData::library("props\\ladder")),
            );
            o.xform.position = Vec3::new(x, 0.0, 0.0);
            scene.insert(o);
        }
        let (ok, out) = export(&scene, &look, &engine, None).unwrap();
        // nothing is breakable; every climable face is a part of its own and none is valid, so
        // nothing reaches the spawn and no entity is made (no configs are needed)
        assert!(ok);
        assert!(out.spawns.is_empty() && out.meshes.is_empty());
        assert_eq!(out.log.iter().filter(|l| l.starts_with("Error face found at pos: [")).count(), 24);
        assert_eq!(out.log.last().map(String::as_str), Some("Can't export invalid part #23"));
        // a surface whose material the library lacks fails the export
        let empty = Engine { materials: Some(GameMaterials::default()), ..Engine::default() };
        let (ok, out) = export(&scene, &look, &empty, None).unwrap();
        assert!(!ok);
        assert_eq!(out.log[0], "Object 'props\\ladder', surface 's' contain invalid game material.");
    }

    #[test]
    fn the_random_range_keeps_the_cpp_remainder() {
        let mut r = Random(1);
        // the first draw of a generator started at 1 is 41
        assert_eq!(r.range(0, 1000), Some(41));
        // a negative span: the remainder keeps the dividend's sign
        let mut r = Random(1);
        assert_eq!(r.range(i32::MAX, 8), Some(i32::MAX.wrapping_add(41)));
        assert_eq!(r.range(3, 3), None);
    }

    #[test]
    fn the_climable_axis_turns_to_its_normal() {
        let mut n = Vec3::new(0.0, -1.0, 0.0);
        let mut r = [Vec3::X, Vec3::Y, Vec3::Z];
        assert!(orient_to_norm(&mut n, &mut r, Vec3::new(1.0, 0.1, 1.0)));
        assert_eq!(n, Vec3::Y);
        assert_eq!(r, [Vec3::X, -Vec3::Y, -Vec3::Z]);
        let mut up = Vec3::Y;
        assert!(!orient_to_norm(&mut up, &mut r, Vec3::new(0.1, 1.0, 1.0)));
    }

    #[test]
    fn the_angles_are_the_engines() {
        // a quarter turn about y (heading) comes back as y
        let m = Mat4::hpb(0.5, 0.0, 0.0);
        let xyz = get_xyz(&m);
        assert!((xyz.y - 0.5).abs() < 1e-6 && xyz.x.abs() < 1e-6 && xyz.z.abs() < 1e-6, "{xyz:?}");
    }
}
