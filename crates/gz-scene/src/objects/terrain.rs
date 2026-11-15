//! The terrain in the editor: its mesh and the cut ground as library objects made in memory.
//!
//! A terrain's mesh is the provided library object `@terrain\<name>` (in the world: it is drawn,
//! hit and compiled at the identity), made from the text and, with `base = "ground"` or
//! `surface = "ground"`, from the level's ground under it: every static scene object that is not
//! `authored` (another terrain or an `authored` object is no ground). A terrain with `cut` on
//! removes the ground it replaces wherever the editor looks: a static scene object that is not
//! `authored` and meets its rectangle is drawn, hit and measured as `@cut\<object's name>`, its
//! model without what the clip removes ([`gz_terrain::clip_mesh`], the game's rule: kind 1 over
//! the old ground sampled on the terrain's own grid, [`gz_terrain::TOLERANCE`] round it). The
//! scene object keeps its `ref`; only the mesh a reference resolves to for the picture and the
//! rays is swapped ([`mesh_reference`]).
//!
//! Both are made on the IO pool when a library load asks for them, with a stamp of everything
//! they were made from; [`Terrains::refresh`] forgets one whose stamp moved, and the next load
//! makes it again. [`cuts`] and [`ai_edits`] are what the level build writes into `overlay.cut`
//! and `overlay.aiedit`.

use std::sync::{Arc, Weak};

use glam::{Vec2, Vec3};
use gz_base::SmolStr;
use gz_fmt_object::{Face, FaceVert, Mesh, Object as EditObject, SurfFaces, VMap, VMapKind, VMapPt};
use gz_terrain::{AiEdit, Base, Corner, Grid, GroundTri, Look, Looks, Region, Spec, Surface, Terrain};
use parking_lot::Mutex;
use rustc_hash::FxHashMap;

use crate::class::ObjClass;
use crate::library::{Library, LibraryObject, Provider};
use crate::math::{Aabb, Mat4};
use crate::model::{ObjData, Object, SceneState};
use crate::objtools::geom::is_static_any;

/// A terrain's own data: what its file says beside the transform.
pub type TerrainData = Spec;

/// The references of the models made here.
pub const TERRAIN_PREFIX: &str = "@terrain\\";
pub const CUT_PREFIX: &str = "@cut\\";

/// The steepest ground the AI map's generation puts a node on: its cosine (60 degrees, the
/// generation's own limit in `bulk::aimap`).
pub const AI_STEEPEST_COS: f32 = 0.5;

/// The library reference of a terrain's mesh.
#[must_use]
pub fn reference(name: &str) -> String {
    format!("{TERRAIN_PREFIX}{}", name.to_lowercase())
}

/// The library reference of a cut object's model.
#[must_use]
pub fn cut_reference(name: &str) -> String {
    format!("{CUT_PREFIX}{}", name.to_lowercase())
}

/// A terrain object as the terrain crate takes it: the centre is its position, the heading its
/// rotation about the vertical.
#[must_use]
pub fn terrain(o: &Object) -> Option<Terrain> {
    let ObjData::Terrain(d) = &o.data else { return None };
    Some(Terrain { centre: o.xform.position, heading: o.xform.rotation.y, spec: (**d).clone() })
}

/// The rectangle's box in the world over XZ.
fn rect_xz(t: &Terrain) -> (Vec2, Vec2) {
    let c = t.corners(0.0);
    let mut lo = Vec2::splat(f32::MAX);
    let mut hi = Vec2::splat(f32::MIN);
    for p in c {
        lo = lo.min(Vec2::new(p.x, p.z));
        hi = hi.max(Vec2::new(p.x, p.z));
    }
    (lo, hi)
}

fn meets_xz(b: &Aabb, (lo, hi): (Vec2, Vec2)) -> bool {
    b.is_valid() && b.min.x <= hi.x && b.max.x >= lo.x && b.min.z <= hi.y && b.max.z >= lo.y
}

/// A terrain's world box: its mesh's once it is made, the rectangle a metre round its centre
/// before.
#[must_use]
pub fn world_box(o: &Object, d: &TerrainData, local_ref: &dyn Fn(&str) -> Option<Aabb>) -> Aabb {
    if let Some(b) = local_ref(&reference(&o.name)).filter(Aabb::is_valid) {
        return b;
    }
    let t = Terrain { centre: o.xform.position, heading: o.xform.rotation.y, spec: d.clone() };
    let (lo, hi) = rect_xz(&t);
    let y = o.xform.position.y;
    Aabb::new(Vec3::new(lo.x, y - 1.0, lo.y), Vec3::new(hi.x, y + 1.0, hi.y))
}

// ---------------------------------------------------------------- the ground

/// FNV-1a, the same on every machine.
#[derive(Clone, Copy)]
struct Fnv(u64);

impl Fnv {
    fn new() -> Self {
        Self(0xcbf2_9ce4_8422_2325)
    }

    fn bytes(mut self, b: &[u8]) -> Self {
        for x in b {
            self.0 = (self.0 ^ u64::from(*x)).wrapping_mul(0x0100_0000_01b3);
        }
        self
    }

    fn u64(self, v: u64) -> Self {
        self.bytes(&v.to_le_bytes())
    }

    fn f32s(self, v: &[f32]) -> Self {
        v.iter().fold(self, |h, x| h.bytes(&x.to_bits().to_le_bytes()))
    }

    fn matrix(self, m: &Mat4) -> Self {
        m.0.iter().fold(self, |h, row| h.f32s(row))
    }
}

/// Whether an object is ground a terrain lies on or cuts: a static scene object (not in visual
/// mode, its library object loaded and static) that is not `authored`.
fn is_ground(o: &Object, lib: &Library) -> Option<Arc<LibraryObject>> {
    if o.marks.authored {
        return None;
    }
    let ObjData::SceneObject(d) = &o.data else { return None };
    if d.visual_mode() || d.reference.is_empty() {
        return None;
    }
    lib.cached(&d.reference).filter(|lo| is_static_any(lo))
}

/// The ground objects whose box meets a terrain's rectangle, in scene order.
fn ground_objects<'a>(
    state: &'a SceneState,
    lib: &Library,
    t: &Terrain,
) -> Vec<(&'a Arc<Object>, Arc<LibraryObject>)> {
    let rect = rect_xz(t);
    let local = |r: &str| lib.cached(r).map(|x| x.bounds);
    state
        .of_class(ObjClass::SceneObject)
        .filter_map(|o| is_ground(o, lib).map(|lo| (o, lo)))
        .filter(|(o, _)| crate::objects::world_box(state, o, &local).is_some_and(|b| meets_xz(&b, rect)))
        .collect()
}

/// Whether a terrain takes the look of the ground under it and has no ground there at all: its
/// faces then wear no texture.
#[must_use]
pub fn wears_no_ground(state: &SceneState, lib: &Library, o: &Object) -> bool {
    terrain(o).is_some_and(|t| t.spec.surface == Surface::Ground && ground_objects(state, lib, &t).is_empty())
}

/// What a terrain's ground is made of, as one number.
fn ground_stamp(state: &SceneState, lib: &Library, t: &Terrain) -> u64 {
    ground_objects(state, lib, t)
        .iter()
        .fold(Fnv::new(), |h, (o, lo)| h.bytes(o.name.as_bytes()).u64(lo.stamp).matrix(&o.world()))
        .0
}

/// The ground's triangles under a terrain with their looks.
fn ground_tris(state: &SceneState, lib: &Library, t: &Terrain) -> (Vec<GroundTri>, Vec<Look>) {
    let mut tris = Vec::new();
    let mut looks: Vec<Look> = Vec::new();
    let mut look_of: FxHashMap<(SmolStr, SmolStr, SmolStr, SmolStr), u32> = FxHashMap::default();
    for (o, lo) in ground_objects(state, lib, t) {
        let world = o.world();
        let obj = &lo.object;
        for m in &obj.meshes {
            for sf in &m.surf_faces {
                let Some(s) = obj.surfaces.get(sf.surface as usize) else { continue };
                let k = (s.texture.clone(), s.shader.clone(), s.shader_xrlc.clone(), s.game_mtl.clone());
                let look = *look_of.entry(k).or_insert_with(|| {
                    looks.push(Look {
                        texture: s.texture.to_string(),
                        shader: s.shader.to_string(),
                        compile: s.shader_xrlc.to_string(),
                        material: s.game_mtl.to_string(),
                        tile: 1.0,
                    });
                    (looks.len() - 1) as u32
                });
                let two = s.two_sided();
                for &fi in sf.faces.iter() {
                    let Some(f) = m.faces.get(fi as usize) else { continue };
                    let p = f.v.map(|v| {
                        world.transform_tiny(m.points.get(v.point as usize).copied().unwrap_or_default())
                    });
                    let uv = f.v.map(|v| m.corner_uv(v).unwrap_or_default());
                    tris.push(GroundTri { p, uv, look });
                    if two {
                        tris.push(GroundTri { p: [p[0], p[2], p[1]], uv: [uv[0], uv[2], uv[1]], look });
                    }
                }
            }
        }
    }
    (tris, looks)
}

/// Loads the library objects every possible ground object names: what a terrain samples has to
/// be read first. Blocking: on the IO pool.
fn load_ground(state: &SceneState, lib: &Library) {
    let refs: Vec<SmolStr> = state
        .of_class(ObjClass::SceneObject)
        .filter(|o| !o.marks.authored)
        .filter_map(|o| match &o.data {
            ObjData::SceneObject(d) if !d.visual_mode() => Some(d.reference.clone()),
            _ => None,
        })
        .collect();
    lib.prefetch(&refs);
}

/// The terrains whose `cut` is on and whose values are sound. A terrain that does not cut is
/// passed by before anything of its text is copied.
fn cutters(state: &SceneState) -> impl Iterator<Item = (&Arc<Object>, Terrain)> {
    state
        .of_class(ObjClass::Terrain)
        .filter(|o| matches!(&o.data, ObjData::Terrain(d) if d.cut))
        .filter_map(|o| terrain(o).map(|t| (o, t)))
        .filter(|(_, t)| t.check().is_ok())
}

/// A terrain's rectangle over XZ from its transform and its size alone.
fn rect_of(o: &Object, d: &Spec) -> (Vec2, Vec2) {
    rect_xz(&Terrain {
        centre: o.xform.position,
        heading: o.xform.rotation.y,
        spec: Spec { size: d.size, ..Spec::default() },
    })
}

/// [`cutters`] whose rectangle meets a box. What every scene object asks in every pass: a
/// terrain's text - its edits, its rows - is copied and checked only for the few that meet it.
fn cutters_over<'a>(state: &'a SceneState, b: &Aabb) -> impl Iterator<Item = (&'a Arc<Object>, Terrain)> {
    let b = *b;
    state
        .of_class(ObjClass::Terrain)
        .filter(move |o| matches!(&o.data, ObjData::Terrain(d) if d.cut && meets_xz(&b, rect_of(o, d))))
        .filter_map(|o| terrain(o).map(|t| (o, t)))
        .filter(|(_, t)| t.check().is_ok())
}

// ---------------------------------------------------------------- what each is made of

/// A terrain's stamp: its text and the ground it samples.
#[must_use]
pub fn stamp(state: &SceneState, lib: &Library, o: &Object) -> Option<u64> {
    let t = terrain(o)?;
    let h = Fnv::new()
        .bytes(format!("{:?}", t.spec).as_bytes())
        .f32s(&[t.centre.x, t.centre.y, t.centre.z, t.heading]);
    let needs_ground = t.spec.base == Base::Ground || t.spec.surface == Surface::Ground || t.spec.cut;
    Some(if needs_ground { h.u64(ground_stamp(state, lib, &t)).0 } else { h.0 })
}

/// The terrains that cut an object, with their stamps: empty for an object nothing cuts.
fn cutting(state: &SceneState, lib: &Library, o: &Object) -> Vec<(Arc<Object>, Terrain)> {
    let Some(lo) = is_ground(o, lib) else { return Vec::new() };
    let b = crate::math::xform_box(&lo.bounds, &o.world());
    cutters_over(state, &b).map(|(c, t)| (c.clone(), t)).collect()
}

/// The reference a scene object's mesh is drawn and hit as: `@cut\<name>` while a cutting
/// terrain meets it, `None` for its own `ref`. Cheap: the cutting terrains are few.
#[must_use]
pub fn mesh_reference(state: &SceneState, lib: &Library, o: &Object) -> Option<String> {
    if o.class != ObjClass::SceneObject || state.count(ObjClass::Terrain) == 0 {
        return None;
    }
    (!cutting(state, lib, o).is_empty()).then(|| cut_reference(&o.name))
}

/// [`mesh_reference`]'s question over a lookup of library objects instead of a library: whether a
/// cutting terrain meets a static scene object that is no `authored` one. What the level build
/// asks to compile the object as its cut model.
#[must_use]
pub fn is_cut(state: &SceneState, lookup: &dyn Fn(&str) -> Option<Arc<LibraryObject>>, o: &Object) -> bool {
    if o.class != ObjClass::SceneObject || o.marks.authored || state.count(ObjClass::Terrain) == 0 {
        return false;
    }
    let ObjData::SceneObject(d) = &o.data else { return false };
    if d.visual_mode() || d.reference.is_empty() {
        return false;
    }
    let Some(lo) = lookup(&d.reference).filter(|lo| is_static_any(lo)) else { return false };
    let b = crate::math::xform_box(&lo.bounds, &o.world());
    cutters_over(state, &b).next().is_some()
}

/// The library object a scene object is drawn and hit as: its cut model once made, else its own.
#[must_use]
pub fn mesh_of(state: &SceneState, lib: &Library, o: &Object) -> Option<Arc<LibraryObject>> {
    match &o.data {
        ObjData::SceneObject(d) => {
            mesh_reference(state, lib, o).and_then(|r| lib.cached(&r)).or_else(|| lib.cached(&d.reference))
        }
        ObjData::Terrain(_) => lib.cached(&reference(&o.name)),
        _ => None,
    }
}

fn cut_stamp(state: &SceneState, lib: &Library, o: &Object) -> Option<u64> {
    let ObjData::SceneObject(d) = &o.data else { return None };
    let lo = lib.cached(&d.reference)?;
    let mut h = Fnv::new().bytes(d.reference.as_bytes()).u64(lo.stamp).matrix(&o.world());
    for (c, _) in cutting(state, lib, o) {
        h = h.u64(stamp(state, lib, &c)?);
    }
    Some(h.0)
}

// ---------------------------------------------------------------- making them

/// What was made of one terrain: the old ground on its grid, its heights, and its ground's
/// triangles and looks.
pub struct Made {
    pub terrain: Terrain,
    pub old_ground: Grid,
    pub heights: Grid,
    tris: Vec<GroundTri>,
    looks: Vec<Look>,
    faces: Vec<Option<u32>>,
}

/// A terrain worked out over the library objects loaded now; `Err` with the sentence of a bad
/// value.
pub fn make(state: &SceneState, lib: &Library, o: &Object) -> Result<Made, String> {
    let t = terrain(o).ok_or_else(|| "not a terrain".to_owned())?;
    t.check()?;
    let (tris, looks) = ground_tris(state, lib, &t);
    let sample = gz_terrain::sample_ground(&t, &tris);
    let old_ground = sample.grid(&t);
    let heights = t.heights(Some(&old_ground))?;
    let faces = sample.face_tris(&t);
    Ok(Made { terrain: t, old_ground, heights, tris, looks, faces })
}

impl Made {
    /// The mesh, in the world.
    #[must_use]
    pub fn mesh(&self) -> EditObject {
        match &self.terrain.spec.surface {
            Surface::Look(l) => self.terrain.mesh(&self.heights, Looks::Table(l)),
            Surface::Ground => self.terrain.mesh(
                &self.heights,
                Looks::Ground { faces: &self.faces, tris: &self.tris, looks: &self.looks },
            ),
        }
    }

    /// The region of `overlay.cut` that removes the ground this terrain replaces.
    #[must_use]
    pub fn region(&self) -> Region {
        self.terrain.region(&self.old_ground)
    }
}

const BACKFACE: u32 = 1 << 3;

/// A model cut by regions: `None` when no triangle of it changes. The clip works in the world;
/// what stays is carried back into the model's own space.
#[must_use]
pub fn cut_model(object: &EditObject, world: &Mat4, regions: &[Region]) -> Option<EditObject> {
    let inverse = Mat4::invert_43(world);
    let mut out = object.clone();
    let mut changed = false;
    for m in &mut out.meshes {
        let mut surface_of = vec![u32::MAX; m.faces.len()];
        for sf in &m.surf_faces {
            for f in sf.faces.iter() {
                if let Some(s) = surface_of.get_mut(*f as usize) {
                    *s = sf.surface;
                }
            }
        }
        let tris: Vec<[Corner; 3]> = m
            .faces
            .iter()
            .map(|f| {
                f.v.map(|v| Corner {
                    pos: world.transform_tiny(m.points.get(v.point as usize).copied().unwrap_or_default()),
                    uv: m.corner_uv(v).unwrap_or_default(),
                })
            })
            .collect();
        let pieces = gz_terrain::clip_mesh(regions, &tris);
        if pieces.len() == tris.len() && pieces.iter().all(|p| !p.cut) {
            continue;
        }
        changed = true;
        let mut points: Vec<Vec3> = m.points.to_vec();
        let mut point_of: FxHashMap<[u32; 3], u32> = FxHashMap::default();
        let mut refs: Vec<(u32, Vec2)> = Vec::new();
        let mut ref_of: FxHashMap<(u32, [u32; 2]), u32> = FxHashMap::default();
        let mut faces = Vec::with_capacity(pieces.len());
        let mut flags = Vec::with_capacity(pieces.len());
        let mut by_surface: std::collections::BTreeMap<u32, Vec<u32>> = std::collections::BTreeMap::new();
        for p in &pieces {
            let src = m.faces[p.source];
            let mut face = Face::default();
            for k in 0..3 {
                let point = if p.cut {
                    let local = inverse.transform_tiny(p.corners[k].pos);
                    let key = [local.x.to_bits(), local.y.to_bits(), local.z.to_bits()];
                    *point_of.entry(key).or_insert_with(|| {
                        points.push(local);
                        (points.len() - 1) as u32
                    })
                } else {
                    src.v[k].point
                };
                let uv = p.corners[k].uv;
                let rk = (point, [uv.x.to_bits(), uv.y.to_bits()]);
                let vmref = *ref_of.entry(rk).or_insert_with(|| {
                    refs.push((point, uv));
                    (refs.len() - 1) as u32
                });
                face.v[k] = FaceVert { point, vmref };
            }
            let f = m.smooth_groups.get(p.source).copied().unwrap_or(0);
            flags.push(if p.cut { f & BACKFACE } else { f });
            by_surface.entry(surface_of[p.source]).or_default().push(faces.len() as u32);
            faces.push(face);
        }
        let mut uv = VMap {
            name: m
                .vmaps
                .iter()
                .find(|v| v.kind == VMapKind::Uv)
                .map_or_else(|| SmolStr::new_static("Texture"), |v| v.name.clone()),
            kind: VMapKind::Uv,
            polymap: false,
            values: Vec::with_capacity(refs.len() * 2),
            vindices: Vec::with_capacity(refs.len()),
            pindices: Vec::new(),
        };
        let mut vmrefs = Vec::with_capacity(refs.len());
        for (i, (point, t)) in refs.iter().enumerate() {
            uv.values.extend([t.x, t.y]);
            uv.vindices.push(*point);
            vmrefs.push(std::iter::once(VMapPt { vmap: 0, index: i as u32 }).collect());
        }
        *m = Mesh {
            points: Arc::from(points),
            faces: Arc::from(faces),
            smooth_groups: Arc::from(flags),
            vmaps: vec![uv],
            vmrefs,
            surf_faces: by_surface
                .into_iter()
                .filter(|(s, _)| *s != u32::MAX)
                .map(|(surface, faces)| SurfFaces { surface, faces: Arc::from(faces) })
                .collect(),
            ..m.clone()
        };
        let _ = m.rebuild_vmaps();
        m.recompute_bbox();
    }
    changed.then_some(out)
}

/// The library object of a terrain or of a cut object, made over the scene there is now.
/// Blocking: on the IO pool.
fn provide(state: &SceneState, lib: &Library, terrains: &Terrains, key: &str) -> Option<Arc<LibraryObject>> {
    if let Some(name) = key.strip_prefix(TERRAIN_PREFIX) {
        let o = state.find(ObjClass::Terrain, name)?;
        load_ground(state, lib);
        let stamp = stamp(state, lib, o)?;
        terrains.seen.lock().insert(key.to_owned(), stamp);
        let made = make(state, lib, o).ok()?;
        let mut lo = LibraryObject::provided(key, made.mesh());
        lo.stamp = stamp;
        return Some(Arc::new(lo));
    }
    let name = key.strip_prefix(CUT_PREFIX)?;
    let o = state.find(ObjClass::SceneObject, name)?;
    let ObjData::SceneObject(d) = &o.data else { return None };
    load_ground(state, lib);
    let stamp = cut_stamp(state, lib, o)?;
    terrains.seen.lock().insert(key.to_owned(), stamp);
    let original = lib.cached(&d.reference)?;
    let regions: Vec<Region> =
        cutting(state, lib, o).iter().filter_map(|(c, _)| terrains.old_region(state, lib, c)).collect();
    let object = cut_model(&original.object, &o.world(), &regions).unwrap_or_else(|| original.object.clone());
    let mut lo = LibraryObject::provided(key, object);
    lo.stamp = stamp;
    Some(Arc::new(lo))
}

/// What the scene's terrains made, kept between loads: the stamp each model was made with and the
/// old ground of every cutting terrain.
#[derive(Default)]
pub struct Terrains {
    seen: Mutex<FxHashMap<String, u64>>,
    regions: Mutex<FxHashMap<String, (u64, Arc<Region>)>>,
}

impl Terrains {
    /// How many models were made here and are not forgotten.
    #[must_use]
    pub fn made(&self) -> usize {
        self.seen.lock().len()
    }

    /// The region of a cutting terrain, worked out once per stamp.
    fn old_region(&self, state: &SceneState, lib: &Library, o: &Object) -> Option<Region> {
        let s = stamp(state, lib, o)?;
        let key = o.name.to_lowercase();
        if let Some((k, r)) = self.regions.lock().get(&key)
            && *k == s
        {
            return Some((**r).clone());
        }
        let r = make(state, lib, o).ok()?.region();
        self.regions.lock().insert(key, (s, Arc::new(r.clone())));
        Some(r)
    }

    /// The provider that makes `@terrain\<name>` and `@cut\<name>` over the service's scene.
    #[must_use]
    pub fn provider(self: &Arc<Self>, scene: Weak<crate::SceneService>) -> Provider {
        let me = Arc::downgrade(self);
        Arc::new(move |key: &str| {
            if !(key.starts_with(TERRAIN_PREFIX) || key.starts_with(CUT_PREFIX)) {
                return None;
            }
            let (svc, me) = (scene.upgrade()?, me.upgrade()?);
            let snap = svc.store.snapshot();
            provide(&snap, &svc.library, &me, key)
        })
    }

    /// The provider of `@terrain\<name>` and `@cut\<name>` over one state of the scene and one
    /// library: what a long job's own library makes them from, whatever the open scene and its
    /// library have become meanwhile.
    #[must_use]
    pub fn provider_over(state: Arc<SceneState>, lib: Weak<Library>) -> Provider {
        let made = Arc::new(Terrains::default());
        Arc::new(move |key: &str| {
            if !(key.starts_with(TERRAIN_PREFIX) || key.starts_with(CUT_PREFIX)) {
                return None;
            }
            let lib = lib.upgrade()?;
            provide(&state, &lib, &made, key)
        })
    }

    /// Forgets every model made here whose stamp moved: the terrain, its ground or the object it
    /// cuts changed, or it is gone. Answers whether something was forgotten.
    pub fn refresh(&self, state: &SceneState, lib: &Library) -> bool {
        let mut now: FxHashMap<String, u64> = FxHashMap::default();
        for o in state.of_class(ObjClass::Terrain) {
            if let Some(s) = stamp(state, lib, o) {
                now.insert(reference(&o.name), s);
            }
        }
        if state.count(ObjClass::Terrain) > 0 {
            for o in state.of_class(ObjClass::SceneObject) {
                if mesh_reference(state, lib, o).is_some()
                    && let Some(s) = cut_stamp(state, lib, o)
                {
                    now.insert(cut_reference(&o.name), s);
                }
            }
        }
        let stale: Vec<String> = {
            let seen = self.seen.lock();
            seen.iter().filter(|(k, s)| now.get(*k) != Some(*s)).map(|(k, _)| k.clone()).collect()
        };
        let mut any = false;
        for k in &stale {
            self.seen.lock().remove(k);
            any |= lib.forget(k);
        }
        self.regions.lock().retain(|k, (s, _)| now.get(&reference(k)) == Some(s));
        any
    }
}

/// The references the scene's terrains and cut objects are drawn and hit as: what a query loads
/// before it builds its pick tree.
#[must_use]
pub fn references(state: &SceneState, lib: &Library) -> Vec<SmolStr> {
    if state.count(ObjClass::Terrain) == 0 {
        return Vec::new();
    }
    let mut out: Vec<SmolStr> =
        state.of_class(ObjClass::Terrain).map(|o| SmolStr::new(reference(&o.name))).collect();
    out.extend(
        state.of_class(ObjClass::SceneObject).filter_map(|o| mesh_reference(state, lib, o)).map(SmolStr::new),
    );
    out
}

// ---------------------------------------------------------------- what the build writes

/// One cutting terrain as the build writes it into `overlay.cut`.
#[derive(Clone, Debug, PartialEq)]
pub struct Cut {
    /// The terrain's name.
    pub terrain: SmolStr,
    pub region: Region,
    /// The scene objects it cuts, by name, and the library references they name.
    pub objects: Vec<SmolStr>,
    pub references: Vec<SmolStr>,
}

/// The regions of the scene's cutting terrains, in scene order, with the old ground's heights on
/// each one's grid, and per region what it cuts. Over the library objects loaded now: the caller
/// loads the scene's references first.
#[must_use]
pub fn cuts(state: &SceneState, lib: &Library) -> Vec<Cut> {
    let mut out = Vec::new();
    for (o, t) in cutters(state) {
        let Ok(made) = make(state, lib, o) else { continue };
        let rect = rect_xz(&t);
        let mut objects = Vec::new();
        let mut references: Vec<SmolStr> = Vec::new();
        for (g, lo) in ground_objects(state, lib, &t) {
            let b = crate::math::xform_box(&lo.bounds, &g.world());
            if !meets_xz(&b, rect) {
                continue;
            }
            objects.push(g.name.clone());
            if let ObjData::SceneObject(d) = &g.data
                && !references.iter().any(|r| r.eq_ignore_ascii_case(&d.reference))
            {
                references.push(d.reference.clone());
            }
        }
        out.push(Cut { terrain: o.name.clone(), region: made.region(), objects, references });
    }
    out
}

/// The changes of the scene's AI map under its cutting terrains, in the map's order: every node
/// inside a cutting terrain's rectangle with its old height, the terrain's height and normal
/// there, and `closed` where the new ground is steeper than the generation takes (60 degrees) or
/// an `authored` static object stands on the node.
#[must_use]
pub fn ai_edits(state: &SceneState, lib: &Library) -> Vec<AiEdit> {
    let Some(map) = state.bulk.ai_map.as_ref() else { return Vec::new() };
    let made: Vec<Made> = cutters(state).filter_map(|(o, _)| make(state, lib, o).ok()).collect();
    if made.is_empty() {
        return Vec::new();
    }
    let half_patch = map.header.params.patch_size * 0.5;
    let local = |r: &str| lib.cached(r).map(|x| x.bounds);
    let standing: Vec<Aabb> = state
        .of_class(ObjClass::SceneObject)
        .filter(|o| o.marks.authored)
        .filter(|o| match &o.data {
            ObjData::SceneObject(d) => lib.cached(&d.reference).is_some_and(|lo| is_static_any(&lo)),
            _ => false,
        })
        .filter_map(|o| crate::objects::world_box(state, o, &local))
        .collect();
    let mut out = Vec::new();
    for tile in &map.nodes {
        for n in tile.iter() {
            // the last terrain over a node is the ground there, as the last region cuts last
            let Some((y, normal)) = made.iter().rev().find_map(|m| m.terrain.surface_at(&m.heights, n.pos))
            else {
                continue;
            };
            let stands = standing.iter().any(|b| {
                b.min.x <= n.pos.x + half_patch
                    && b.max.x >= n.pos.x - half_patch
                    && b.min.z <= n.pos.z + half_patch
                    && b.max.z >= n.pos.z - half_patch
                    && b.min.y <= y + 1.0
                    && b.max.y >= y
            });
            out.push(AiEdit {
                x: n.pos.x,
                z: n.pos.z,
                old_y: n.pos.y,
                new_y: y,
                normal,
                closed: normal.y < AI_STEEPEST_COS || stands,
            });
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::objects::SceneObjectData;
    use crate::pick::{RayFilter, ScenePick, ray_scene};
    use crate::tools::AiMapPart;

    /// A flat ground of 100 x 100 m at y = 0, made in memory.
    fn ground_model() -> EditObject {
        let t = Terrain {
            centre: Vec3::ZERO,
            heading: 0.0,
            spec: Spec { size: Vec2::splat(100.0), cell: 4.0, base: Base::Height(0.0), ..Spec::default() },
        };
        let look = Look { texture: "ground\\grass".into(), ..Look::default() };
        t.mesh(&t.heights(None).expect("heights"), Looks::Table(&look))
    }

    /// The audit's check of a terrain's border, on the scene below and on one that stands 3 m
    /// above the ground it lies on.
    #[test]
    fn the_audit_finds_a_terrain_whose_border_stands_off_the_ground() {
        use crate::sense::audit::{AuditParams, audit};
        let run = |state: &SceneState| {
            let terrains = Arc::new(Terrains::default());
            let lib = library(state, &terrains);
            let pick = ScenePick::build(state, &lib);
            let sc = crate::sense::Scene { state, lib: &lib, pick: &pick };
            let p = AuditParams { checks: Some(vec!["terrain_border".to_owned()]), ..AuditParams::default() };
            audit(&sc, &p, None).map(|a| a.findings).unwrap_or_default()
        };
        // its border lies on the ground: nothing to say
        assert!(run(&scene(false)).is_empty());
        // flat at 3 m over ground at 0: the border is a step
        let mut high = scene(false);
        let id = high.find_any("Yard").map(|o| o.id).expect("the terrain");
        let mut o = (**high.get(id).expect("the terrain")).clone();
        if let ObjData::Terrain(d) = &mut o.data {
            d.base = Base::Height(3.0);
            d.edits.clear();
        }
        high.remove(id);
        high.insert(o);
        let found = run(&high);
        assert_eq!(found.len(), 1, "{found:?}");
        assert_eq!(found[0].code, "S211");
        assert!(found[0].text.starts_with("Yard's border stands 3.00 m off the ground"), "{}", found[0].text);
    }

    /// A thing dropped over a terrain lands on the terrain - with the ground under it cut away
    /// and without.
    #[test]
    fn a_thing_dropped_over_a_terrain_lands_on_it() {
        for cut in [false, true] {
            let mut state = scene(cut);
            // a square of one metre, five metres up over the middle of the yard
            let mut thing = Object::new(
                ObjClass::SceneObject,
                "thing",
                ObjData::SceneObject(SceneObjectData::library("ground\\flat")),
            );
            thing.xform.position = Vec3::new(0.0, 5.0, 0.0);
            thing.xform.scale = Vec3::splat(0.01);
            thing.marks.authored = true;
            let id = state.insert(thing);
            let terrains = Arc::new(Terrains::default());
            let lib = library(&state, &terrains);
            let pick = ScenePick::build(&state, &lib);
            for line in [false, true] {
                let landed = crate::landing::drop_to_ground(&state, &lib, &pick, &[id], false, line);
                assert_eq!(landed.len(), 1, "cut {cut}, line {line}");
                // the yard is flattened to 2 m at its middle
                assert!(
                    (landed[0].position.y - 2.0).abs() < 0.02,
                    "cut {cut}, line {line}: {:?}",
                    landed[0].position
                );
            }
        }
    }

    /// A terrain whose values make no model is named by the audit with the reason.
    #[test]
    fn the_audit_says_why_a_terrain_has_no_model() {
        use crate::sense::audit::{AuditParams, audit};
        let mut state = scene(false);
        let id = state.find_any("Yard").map(|o| o.id).expect("the terrain");
        let mut o = (**state.get(id).expect("the terrain")).clone();
        if let ObjData::Terrain(d) = &mut o.data {
            // one row where the grid has eleven
            d.rows = vec![vec![0.0; 3]];
        }
        state.remove(id);
        state.insert(o);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&state, &terrains);
        let pick = ScenePick::build(&state, &lib);
        let sc = crate::sense::Scene { state: &state, lib: &lib, pick: &pick };
        let found = audit(&sc, &AuditParams::default(), None).map(|a| a.findings).unwrap_or_default();
        let unmade: Vec<_> = found.iter().filter(|f| f.code == "S212").collect();
        assert_eq!(unmade.len(), 1, "{found:?}");
        assert!(
            unmade[0].text.starts_with("Yard has no model: rows holds 1 rows of 3 heights"),
            "{}",
            unmade[0].text
        );
    }

    /// A terrain is the module's own to the audit as it is to the build: read when nothing is
    /// named, never told it will not ship; and one that wears the ground's look where no ground
    /// is has no texture to wear.
    #[test]
    fn the_audit_reads_a_terrain_as_the_module_s_own_and_names_one_over_no_ground() {
        use crate::sense::audit::{AuditParams, audit};
        // on a level of the game (a base scene that has the floor): the terrain is not marked
        let state = scene(false);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&state, &terrains);
        let pick = ScenePick::build(&state, &lib);
        let sc = crate::sense::Scene { state: &state, lib: &lib, pick: &pick };
        let base: rustc_hash::FxHashSet<String> = ["floor".to_owned()].into_iter().collect();
        let found = audit(&sc, &AuditParams::default(), Some(&base)).map(|a| a.findings).unwrap_or_default();
        assert!(found.iter().all(|f| f.code != "S210" && f.code != "S213"), "{found:?}");
        // the same terrain with no ground under it
        let mut bare = scene(false);
        let floor = bare.find_any("Floor").map(|o| o.id).expect("the floor");
        bare.remove(floor);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&bare, &terrains);
        let pick = ScenePick::build(&bare, &lib);
        let sc = crate::sense::Scene { state: &bare, lib: &lib, pick: &pick };
        let found = audit(&sc, &AuditParams::default(), None).map(|a| a.findings).unwrap_or_default();
        let named: Vec<_> = found.iter().filter(|f| f.code == "S213").collect();
        assert_eq!(named.len(), 1, "{found:?}");
        assert!(named[0].text.starts_with("Yard takes the look of the ground"), "{}", named[0].text);
    }

    /// A thing that stands on a terrain is sound to the audit: it does not float, and it is not
    /// "in" the terrain it stands on.
    #[test]
    fn a_thing_on_a_terrain_neither_floats_nor_stands_in_it() {
        use crate::sense::audit::{AuditParams, audit};
        let mut state = scene(true);
        let mut thing = Object::new(
            ObjClass::SceneObject,
            "thing",
            ObjData::SceneObject(SceneObjectData::library("ground\\flat")),
        );
        // a square of one metre lying on the yard's flat middle, 2 m up
        thing.xform.position = Vec3::new(0.0, 2.0, 0.0);
        thing.xform.scale = Vec3::splat(0.01);
        thing.marks.authored = true;
        state.insert(thing);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&state, &terrains);
        let pick = ScenePick::build(&state, &lib);
        let sc = crate::sense::Scene { state: &state, lib: &lib, pick: &pick };
        let p = AuditParams { objects: Some(vec!["thing".to_owned()]), ..AuditParams::default() };
        let found = audit(&sc, &p, None).map(|a| a.findings).unwrap_or_default();
        assert!(found.is_empty(), "{found:?}");
    }

    fn scene(cut: bool) -> SceneState {
        let mut s = SceneState::default();
        s.insert(Object::new(
            ObjClass::SceneObject,
            "Floor",
            ObjData::SceneObject(SceneObjectData::library("ground\\flat")),
        ));
        let mut t = Object::new(
            ObjClass::Terrain,
            "Yard",
            ObjData::Terrain(Box::new(Spec {
                size: Vec2::splat(10.0),
                cell: 1.0,
                base: Base::Ground,
                edge: 0.0,
                edits: vec![gz_terrain::Edit::Flatten {
                    at: Vec2::ZERO,
                    radius: 2.0,
                    falloff: 1.0,
                    height: 2.0,
                }],
                cut,
                ..Spec::default()
            })),
        );
        t.xform.position = Vec3::new(0.0, 0.0, 0.0);
        s.insert(t);
        s
    }

    fn library(state: &SceneState, terrains: &Arc<Terrains>) -> Arc<Library> {
        let lib = Arc::new(Library::new(Vec::new()));
        lib.add_provider(Arc::new(|k: &str| {
            (k == "ground\\flat").then(|| Arc::new(LibraryObject::provided(k, ground_model())))
        }));
        let (weak, s, t) = (Arc::downgrade(&lib), state.clone(), terrains.clone());
        lib.add_provider(Arc::new(move |k: &str| {
            let lib = weak.upgrade()?;
            provide(&s, &lib, &t, k)
        }));
        assert!(lib.provided("ground\\flat").is_some());
        lib.prefetch(&references(state, &lib));
        lib
    }

    /// A job's library makes a terrain from the state of the scene the job works on: nothing
    /// made for the open scene is taken over, and an edit of the open scene changes nothing in
    /// it.
    #[test]
    fn a_fork_makes_its_terrains_from_its_own_state() {
        let state_of = |height: f32| {
            let mut s = scene(false);
            let mut yard = (**s.find_any("yard").expect("yard")).clone();
            if let ObjData::Terrain(d) = &mut yard.data {
                d.edits =
                    vec![gz_terrain::Edit::Flatten { at: Vec2::ZERO, radius: 2.0, falloff: 1.0, height }];
            }
            s.replace(yard);
            Arc::new(s)
        };
        let top = |lib: &Library| lib.load(&reference("Yard")).expect("the terrain").bounds.max.y;
        // the open scene's library, with the terrain as the scene had it before its last edit
        let open = Arc::new(Library::new(Vec::new()));
        open.add_provider(Arc::new(|k: &str| {
            (k == "ground\\flat").then(|| Arc::new(LibraryObject::provided(k, ground_model())))
        }));
        open.add_scene_provider(Terrains::provider_over(state_of(2.0), Arc::downgrade(&open)));
        assert!((top(&open) - 2.0).abs() < 1e-3);
        // the job works on the scene after the edit: its fork makes the terrain anew
        let own = Arc::new(open.fork());
        assert!(own.cached(&reference("Yard")).is_none(), "what the scene made is not taken over");
        assert!(own.cached("ground\\flat").is_some(), "what was read is");
        own.add_scene_provider(Terrains::provider_over(state_of(5.0), Arc::downgrade(&own)));
        assert!((top(&own) - 5.0).abs() < 1e-3, "{}", top(&own));
        assert!((top(&open) - 2.0).abs() < 1e-3, "the open scene's library keeps its own");
    }

    fn down(state: &SceneState, lib: &Library, x: f32, z: f32) -> Option<(f32, String)> {
        let pick = ScenePick::build(state, lib);
        ray_scene(&pick, state, lib, Vec3::new(x, 20.0, z), Vec3::NEG_Y, 50.0, &RayFilter::default())
            .map(|h| (h.point.y, state.get(h.id).map(|o| o.name.to_string()).unwrap_or_default()))
    }

    #[test]
    fn a_terrain_stands_on_the_ground_and_is_hit_as_a_mesh() {
        let s = scene(false);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&s, &terrains);
        let yard = lib.cached(&reference("Yard")).expect("the terrain's mesh");
        assert!(yard.stamp != 0 && yard.file.as_os_str().is_empty());
        // the flatten at the middle, the ground at the border
        let (y, name) = down(&s, &lib, 0.0, 0.0).expect("a hit");
        assert!((y - 2.0).abs() < 1e-3 && name == "Yard", "{y} {name}");
        let (y, _) = down(&s, &lib, 4.5, 0.0).expect("a hit");
        assert!(y.abs() < 1e-3, "{y}");
        // its surface is the ground's
        assert_eq!(yard.object.surfaces[0].texture, "ground\\grass");
        // without a cut the floor under it is whole
        assert!(mesh_reference(&s, &lib, s.find_any("floor").expect("floor")).is_none());
        assert!(cuts(&s, &lib).is_empty());
    }

    #[test]
    fn the_cut_takes_the_ground_away_under_the_terrain_only() {
        let s = scene(true);
        let terrains = Arc::new(Terrains::default());
        let lib = library(&s, &terrains);
        let floor = s.find_any("floor").expect("floor");
        assert_eq!(mesh_reference(&s, &lib, floor).as_deref(), Some("@cut\\floor"));
        let cut = lib.cached("@cut\\floor").expect("the cut model");
        // what stays of the floor's top is 100 x 100 less 10 x 10
        let area: f32 = cut
            .tris
            .iter()
            .map(|t| {
                let p = t.map(|i| cut.verts[i as usize]);
                // the top alone, not the model's skirt
                if p.iter().all(|q| q.y > -0.01) { (p[1] - p[0]).cross(p[2] - p[0]).y * 0.5 } else { 0.0 }
            })
            .sum();
        assert!((area - (10_000.0 - 100.0)).abs() < 1.0, "{area}");
        // a ray only at the floor finds no floor under the terrain, and finds it beside
        let only = [floor.id];
        let pick = ScenePick::build(&s, &lib);
        let f = RayFilter { only: Some(&only), ..RayFilter::default() };
        let start = |x: f32| Vec3::new(x, 20.0, 0.0);
        assert!(ray_scene(&pick, &s, &lib, start(1.0), Vec3::NEG_Y, 50.0, &f).is_none());
        assert!(ray_scene(&pick, &s, &lib, start(20.0), Vec3::NEG_Y, 50.0, &f).is_some());
        // the build's view of it: one region over the old ground, cutting the floor
        let c = cuts(&s, &lib);
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].objects, vec![SmolStr::new("Floor")]);
        assert_eq!(c[0].references, vec![SmolStr::new("ground\\flat")]);
        assert_eq!((c[0].region.columns, c[0].region.rows), (11, 11));
        assert!(c[0].region.heights.iter().all(|h| h.abs() < 1e-4));
        // moving the terrain makes both models stale; the next load makes them again
        let mut moved = s.clone();
        let mut t = (**moved.find_any("yard").expect("yard")).clone();
        t.xform.position.x = 30.0;
        moved.replace(t);
        assert!(terrains.refresh(&moved, &lib));
        assert!(lib.cached("@terrain\\yard").is_none() && lib.cached("@cut\\floor").is_none());
        // the same scene again forgets nothing
        assert!(!terrains.refresh(&moved, &lib));
    }

    #[test]
    fn the_ai_map_under_a_cutting_terrain_takes_its_heights() {
        let mut s = scene(true);
        let nodes = [
            gz_fmt_level::AiMapNode::new(Vec3::new(0.0, 0.0, 0.0), Vec3::Y),
            gz_fmt_level::AiMapNode::new(Vec3::new(3.0, 0.0, 0.0), Vec3::Y),
            gz_fmt_level::AiMapNode::new(Vec3::new(20.0, 0.0, 0.0), Vec3::Y),
        ];
        let header = gz_fmt_level::SceneAiMap {
            bbox: Aabb::new(Vec3::splat(-50.0), Vec3::splat(50.0)),
            ..gz_fmt_level::SceneAiMap::default()
        };
        s.bulk.ai_map = Some(Arc::new(AiMapPart::from_nodes(header, &nodes, None)));
        // a box the module adds stands on the node at x = 3
        s.insert({
            let mut o = Object::new(
                ObjClass::SceneObject,
                "crate",
                ObjData::SceneObject(SceneObjectData::library("ground\\flat")),
            );
            o.marks.authored = true;
            o.xform.position = Vec3::new(3.0, 0.0, 0.0);
            o.xform.scale = Vec3::splat(0.01);
            o
        });
        let terrains = Arc::new(Terrains::default());
        let lib = library(&s, &terrains);
        let e = ai_edits(&s, &lib);
        assert_eq!(e.len(), 2, "the node beside the terrain is no edit");
        assert!((e[0].new_y - 2.0).abs() < 1e-3 && e[0].old_y == 0.0 && !e[0].closed);
        assert!(e[0].normal.y > 0.99);
        assert!(e[1].closed, "something the module adds stands on it");
        // the files the build writes from them read back
        let bytes = gz_terrain::write_ai_edits(&e);
        assert_eq!(gz_terrain::read_ai_edits(&bytes).expect("read").len(), 2);
    }
}
