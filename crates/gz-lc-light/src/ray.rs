//! The ray model the light is traced in: the collision soup (the
//! level's shadow-casting faces welded,
//! the MU references' collision faces) in a four-wide BVH (`bvh4.rs`, built on the caller's pool), with
//! what a ray needs of every triangle: the face it stands for, whether it casts a shadow, whether it is
//! opaque, its texture and texture coordinates. `trace` also gives the last hit's scale. The scalar
//! tree of `gz_fmt_level` answers the box queries of the detail light (made on the first one).
//!
//! **The order of hits.** X-Ray's compiler multiplies the transparencies of the hits in the order its
//! tree reports them and stops at the first opaque one; here an opaque hit stops the walk (the
//! answer is 0 either way) and the transparent ones are multiplied nearest first, so the answer
//! is the same whatever tree holds the triangles.
//!
//! **The cached triangle.** X-Ray's compiler keeps, per light, the opaque triangle that last stopped a
//! ray and tests it before the tree; that state lives in the shared light and makes the answer
//! depend on which texel came first. Here the cache is the worker's ([`LightCache`]) and is
//! tested with the trees' own box and triangle tests (`ray_triangle`: a hit there is a hit of the
//! walk, whose box test is monotonic in the box), never for the skipped face, so a cached answer
//! is the answer the tree would give: the light is the same on any number of threads, whatever
//! the rays traced before. A cold and a warm cache, in either order, are tested to agree.

use std::sync::OnceLock;

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};
use gz_fmt_level::{Aabb, Bvh, CollectorPacked, Ray, RayHit};
use rayon::prelude::*;
use smallvec::SmallVec;

use crate::bvh4::{Bvh4, Probe, Scalar, Walk};
use crate::data::{BuildMaterial, BuildTexture, ShaderXrlc, Shaders, shader_of};
use crate::light::LightSet;
use crate::math::{Corners, color_get_a, i_floor, weld_packed};
use crate::mesh::{Face, FaceId, Mesh};
use crate::mu::{MuModel, MuReference};
use crate::{Ctl, LcError, Result};

/// The collision form's current version.
pub const CFORM_VERSION: u32 = 4;
/// A cache slot that holds no triangle.
pub const NO_TRI: u32 = u32::MAX;

/// How a transparent texel lets light through.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Opacity {
    /// The level compiler: `1 - (a / 255)^2`.
    Squared,
    /// The detail compiler: `1 - a / 255`.
    Linear,
}

/// What a ray needs of one triangle (the pointer of X-Ray's compiler, resolved).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayFace {
    /// The level face it stands for: a ray cast from that face skips it. `None` for an MU
    /// model's face and a face of `build.cform`.
    pub face: Option<FaceId>,
    /// Whether the face's shader casts a shadow.
    pub cast_shadow: bool,
    /// Lets no light through; a transparent face's texture alpha is read.
    pub opaque: bool,
    /// The texture (`b_material::surfidx`).
    pub texture: u32,
    pub uv: [Vec2; 3],
}

/// The ray model.
#[derive(Clone, Debug)]
pub struct RayModel {
    tree: Bvh4,
    /// The scalar tree of the box queries, made on the first one.
    pick: OnceLock<Option<Bvh>>,
    tris: Vec<[Vec3; 3]>,
    faces: Vec<RayFace>,
    opacity: Opacity,
    /// The detail compiler's rays see only the faces turned towards them.
    cull: bool,
}

impl RayModel {
    /// The model of an indexed soup; `faces[i]` describes triangle `i`. Builds its tree on the
    /// caller's pool.
    pub fn new(
        verts: &[Vec3],
        tris: &[[u32; 3]],
        faces: Vec<RayFace>,
        opacity: Opacity,
        cull: bool,
    ) -> Result<Self> {
        if faces.len() != tris.len() {
            return Err(LcError::Invalid(format!(
                "The ray model has {} triangles and {} descriptions of them.",
                tris.len(),
                faces.len()
            )));
        }
        if let Some(bad) = tris.iter().position(|t| t.iter().any(|&c| c as usize >= verts.len())) {
            return Err(LcError::Invalid(format!(
                "Triangle {bad} of the ray model names a vertex outside the {} that were given.",
                verts.len()
            )));
        }
        let corner = |i: u32| verts.get(i as usize).copied().unwrap_or_default();
        let tris: Vec<[Vec3; 3]> =
            tris.par_iter().with_min_len(4096).map(|t| [corner(t[0]), corner(t[1]), corner(t[2])]).collect();
        let tree = Bvh4::build(&tris, &faces);
        Ok(Self { tree, pick: OnceLock::new(), tris, faces, opacity, cull })
    }

    /// The scalar tree over the same triangles: what the box queries walk.
    fn pick(&self) -> Option<&Bvh> {
        self.pick
            .get_or_init(|| {
                let verts: Vec<Vec3> = self.tris.iter().flatten().copied().collect();
                let tris: Vec<[u32; 3]> =
                    (0..self.tris.len() as u32).map(|i| [3 * i, 3 * i + 1, 3 * i + 2]).collect();
                Bvh::build(&verts, &tris).ok()
            })
            .as_ref()
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tris.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    /// Every triangle's corners, by index: what another lighter builds its own tree from.
    #[must_use]
    pub fn tris(&self) -> &[[Vec3; 3]] {
        &self.tris
    }

    /// What a ray needs of every triangle, by index.
    #[must_use]
    pub fn faces(&self) -> &[RayFace] {
        &self.faces
    }

    /// How a transparent texel lets light through.
    #[must_use]
    pub fn opacity(&self) -> Opacity {
        self.opacity
    }

    /// Whether rays see only the faces turned towards them.
    #[must_use]
    pub fn cull(&self) -> bool {
        self.cull
    }

    #[must_use]
    pub fn tri(&self, i: u32) -> Option<&[Vec3; 3]> {
        self.tris.get(i as usize)
    }

    #[must_use]
    pub fn face(&self, i: u32) -> Option<&RayFace> {
        self.faces.get(i as usize)
    }

    /// The triangles that touch a box (a full test), by index.
    pub fn query_box(&self, b: &Aabb, out: &mut Vec<u32>) {
        if let Some(t) = self.pick() {
            t.query_box(b, out);
        }
    }

    fn ray(&self, p: Vec3, d: Vec3, range: f32) -> Ray {
        Ray { origin: p, dir: d, t_max: range, cull_back: self.cull }
    }

    /// How much of a transparent face's light gets through where the ray met it; `None` when
    /// its texture has no surface (X-Ray's compiler takes the face as opaque then).
    fn transparency(&self, textures: &[BuildTexture], face: &RayFace, hit: &RayHit) -> Option<f32> {
        let t = textures.get(face.texture as usize)?;
        let surface = t.surface.as_ref()?;
        let (w, h) = (t.width, t.height);
        if w == 0 || h == 0 {
            return None;
        }
        // W, U, V order
        let b = Vec3::new(1.0 - hit.u - hit.v, hit.u, hit.v);
        let c = &face.uv;
        let uv =
            Vec2::new(c[0].x * b.x + c[1].x * b.y + c[2].x * b.z, c[0].y * b.x + c[1].y * b.y + c[2].y * b.z);
        // `U %= dwWidth` takes the int as unsigned: a negative U wraps through 2^32
        let u = (i_floor(uv.x * w as f32 + 0.5) as u32) % w;
        let v = (i_floor(uv.y * h as f32 + 0.5) as u32) % h;
        let pixel = *surface.get(v as usize * w as usize + u as usize)?;
        let a = color_get_a(pixel) as f32 / 255.0;
        Some(match self.opacity {
            Opacity::Squared => 1.0 - a * a,
            Opacity::Linear => 1.0 - a,
        })
    }

    /// How much light gets from `p` along `d` (of unit length) over `range`: 1 for a
    /// free way, 0 behind an opaque face, the product of the transparencies otherwise. A face
    /// that casts no shadow, and `skip`, are passed through.
    #[allow(clippy::too_many_arguments)]
    pub fn trace(
        &self,
        textures: &[BuildTexture],
        scratch: &mut Vec<RayHit>,
        cached: &mut u32,
        p: Vec3,
        d: Vec3,
        range: f32,
        skip: Option<FaceId>,
    ) -> f32 {
        let one = Scalar::new(&self.ray(p, d, range));
        let skipped = |f: &RayFace| skip.is_some() && f.face == skip;
        // 1. the triangle that last stopped this light, tested as `ray_triangle` tests it
        if let (Some(tri), Some(face)) = (self.tri(*cached), self.face(*cached))
            && !skipped(face)
            && let Some(t) = one.own(tri)
            && t > 0.0
            && t < range
        {
            return 0.0;
        }
        // 2. the opaque triangles that stopped this worker's last rays, whatever light they came from:
        // one that stops this ray is one the walk would stop at
        let victims = Victims::of(scratch);
        if let Some(&v) = victims.iter().find(|&&v| {
            v != *cached
                && self.face(v).is_some_and(|f| f.opaque && f.cast_shadow && !skipped(f))
                && self.tri(v).and_then(|tri| one.own(tri)).is_some_and(|t| t > 0.0 && t < range)
        }) {
            *cached = v;
            Victims::remember(scratch, v);
            return 0.0;
        }
        // 3. the tree (it tests only the faces that cast a shadow)
        scratch.truncate(Victims::LEN);
        if let Walk::Stopped(tri) = self.tree.shadow(&Probe::new(one, skip.map(|f| f.0)), scratch) {
            *cached = tri;
            Victims::remember(scratch, tri);
            return 0.0;
        }
        // 4. the transparent ones, nearest first
        let hits = scratch.get_mut(Victims::LEN..).unwrap_or_default();
        hits.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.tri.cmp(&b.tri)));
        let mut scale = 1.0f32;
        for hit in hits.iter() {
            let Some(face) = self.face(hit.tri) else { continue };
            match self.transparency(textures, face, hit) {
                // a product that reached 0 stays 0 whatever follows (no factor is negative or NaN)
                Some(o) => {
                    scale *= o;
                    if scale == 0.0 {
                        return scale;
                    }
                }
                None => {
                    *cached = hit.tri;
                    return 0.0;
                }
            }
        }
        scale
    }
}

/// The head of a trace's buffer of hits: the opaque triangles that stopped the last rays traced
/// with it, whatever light they came from. The first entry marks the buffer as holding them and
/// the latest first; a buffer without the mark starts empty.
struct Victims;

impl Victims {
    const COUNT: usize = 4;
    const LEN: usize = Self::COUNT + 1;
    /// The mark: no distance of a hit is negative.
    const MARK: f32 = -1.0;

    /// The triangles a buffer holds, the latest first (`NO_TRI` for an empty place), made ready when
    /// it holds none.
    fn of(scratch: &mut Vec<RayHit>) -> [u32; Self::COUNT] {
        if scratch.len() < Self::LEN || scratch.first().is_none_or(|h| h.t != Self::MARK) {
            scratch.clear();
            scratch.push(RayHit { tri: 0, t: Self::MARK, u: 0.0, v: 0.0 });
            scratch.resize(Self::LEN, RayHit { tri: NO_TRI, t: 0.0, u: 0.0, v: 0.0 });
        }
        let mut out = [NO_TRI; Self::COUNT];
        for (o, h) in out.iter_mut().zip(scratch.iter().skip(1)) {
            *o = h.tri;
        }
        out
    }

    /// `tri` stopped a ray: it goes first, the oldest one goes when it was not there.
    fn remember(scratch: &mut [RayHit], tri: u32) {
        let Some(list) = scratch.get_mut(1..Self::LEN) else { return };
        let at = list.iter().position(|h| h.tri == tri).unwrap_or(Self::COUNT - 1);
        if let Some(part) = list.get_mut(..=at) {
            part.rotate_right(1);
        }
        if let Some(first) = list.first_mut() {
            first.tri = tri;
        }
    }
}

/// What one worker keeps between rays: the triangle that last stopped each light of the set it
/// lights with, and a buffer of hits.
#[derive(Clone, Debug, Default)]
pub struct LightCache {
    pub rgb: Vec<u32>,
    pub hemi: Vec<u32>,
    pub sun: Vec<u32>,
    pub hits: Vec<RayHit>,
}

impl LightCache {
    /// Room for every light of a set.
    pub fn fit(&mut self, set: &LightSet) {
        let grow = |v: &mut Vec<u32>, n: usize| {
            if v.len() < n {
                v.resize(n, NO_TRI);
            }
        };
        grow(&mut self.rgb, set.rgb.len());
        grow(&mut self.hemi, set.hemi.len());
        grow(&mut self.sun, set.sun.len());
    }

    /// Forgets the cached triangles (another light set, another model), those the buffer of hits
    /// keeps too.
    pub fn reset(&mut self) {
        self.rgb.iter_mut().chain(self.hemi.iter_mut()).chain(self.sun.iter_mut()).for_each(|t| *t = NO_TRI);
        self.hits.clear();
    }
}

/// `b_rc_face`: what the detail and AI compilers know of a triangle of `build.cform`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RcFace {
    pub material: u16,
    pub material_game: u32,
    pub t: [Vec2; 3],
    pub reserved: u16,
}

/// The collision soup, as `build.cform` holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RcastForm {
    /// `scene_bb`.
    pub bounds: Aabb,
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    /// The word after a triangle's corners; X-Ray's compiler writes the low half of a face pointer, the
    /// compiler the triangle's index.
    pub dummy: Vec<u32>,
    pub faces: Vec<RcFace>,
}

impl RcastForm {
    /// `build.cform`: chunk 0 the header, the vertices and the triangles, chunk 1 the faces.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::new();
        w.chunk(0, |w| {
            w.w_u32(CFORM_VERSION);
            w.w_u32(self.verts.len() as u32);
            w.w_u32(self.tris.len() as u32);
            w.w_vec3(self.bounds.min);
            w.w_vec3(self.bounds.max);
            for v in &self.verts {
                w.w_vec3(*v);
            }
            for (i, t) in self.tris.iter().enumerate() {
                for c in t {
                    w.w_u32(*c);
                }
                w.w_u32(self.dummy.get(i).copied().unwrap_or(i as u32));
            }
        });
        w.chunk(1, |w| {
            for f in &self.faces {
                w.w_u16(f.material);
                w.w_u16(0);
                w.w_u32(f.material_game);
                for t in f.t {
                    w.w_vec2(t);
                }
                w.w_u16(f.reserved);
                w.w_u16(0);
            }
        });
        w.into_bytes()
    }

    /// Reads `build.cform`.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let file = Reader::new(bytes);
        let main = file.need_chunk(0)?;
        let mut r = main.reader();
        let version = r.r_u32()?;
        if version != CFORM_VERSION {
            return Err(LcError::Invalid(format!("build.cform has version {version}, not {CFORM_VERSION}.")));
        }
        let (nv, nt) = (r.r_u32()? as usize, r.r_u32()? as usize);
        let bounds = Aabb { min: r.r_vec3()?, max: r.r_vec3()? };
        if nv.saturating_mul(12).saturating_add(nt.saturating_mul(16)) > r.elapsed() {
            return Err(LcError::Invalid("build.cform is cut short.".into()));
        }
        let mut out = RcastForm { bounds, ..RcastForm::default() };
        out.verts.reserve(nv);
        for _ in 0..nv {
            out.verts.push(r.r_vec3()?);
        }
        out.tris.reserve(nt);
        for _ in 0..nt {
            out.tris.push([r.r_u32()?, r.r_u32()?, r.r_u32()?]);
            out.dummy.push(r.r_u32()?);
        }
        let faces = file.need_chunk(1)?;
        let mut r = faces.reader();
        if nt.saturating_mul(36) > r.elapsed() {
            return Err(LcError::Invalid("build.cform has fewer faces than triangles.".into()));
        }
        for _ in 0..nt {
            let material = r.r_u16()?;
            r.advance(2)?;
            let material_game = r.r_u32()?;
            let t = [r.r_vec2()?, r.r_vec2()?, r.r_vec2()?];
            let reserved = r.r_u16()?;
            r.advance(2)?;
            out.faces.push(RcFace { material, material_game, t, reserved });
        }
        Ok(out)
    }

    /// The detail compiler's ray model of the form: a face is opaque unless its texture has
    /// alpha and a surface, it casts a shadow by its shader; rays see only front faces.
    pub fn ray_model(
        &self,
        shaders: &Shaders,
        materials: &[BuildMaterial],
        textures: &[BuildTexture],
    ) -> Result<RayModel> {
        let faces = self
            .faces
            .iter()
            .map(|f| {
                let texture =
                    materials.get(usize::from(f.material)).map_or(u32::MAX, |m| u32::from(m.surface));
                let opaque =
                    textures.get(texture as usize).is_none_or(|t| !(t.has_alpha && t.surface.is_some()));
                RayFace {
                    face: None,
                    cast_shadow: casts(shader_of(f.material, shaders, materials)),
                    opaque,
                    texture,
                    uv: f.t,
                }
            })
            .collect();
        let model = RayModel::new(&self.verts, &self.tris, faces, Opacity::Linear, true)?;
        // the detail light asks boxes from many threads at once: the tree is made before
        model.pick();
        Ok(model)
    }
}

/// Whether a shader casts a shadow.
#[must_use]
pub fn casts(s: &ShaderXrlc) -> bool {
    s.flags & ShaderXrlc::LIGHT_CAST_SHADOW != 0
}

/// What the collector gathers: the soup with its faces.
#[derive(Debug)]
pub struct RapidCollector {
    pub collector: CollectorPacked,
    pub faces: Vec<RayFace>,
    pub rc_faces: Vec<RcFace>,
}

impl RapidCollector {
    #[must_use]
    pub fn new(bounds: &Aabb, verts: usize, faces: usize) -> Self {
        Self {
            collector: CollectorPacked::new(bounds, verts, faces),
            faces: Vec::new(),
            rc_faces: Vec::new(),
        }
    }

    /// `add_face_D` with what a ray and `build.cform` keep of the face.
    pub fn add(&mut self, p: [Vec3; 3], face: RayFace, rc: RcFace, sm_group: u32) {
        let id = self.faces.len() as u32;
        self.collector.add_face_d(p[0], p[1], p[2], id, sm_group);
        self.faces.push(face);
        self.rc_faces.push(rc);
    }

    /// The level compiler's ray model and the form of `build.cform`.
    pub fn finish(self, bounds: Aabb) -> Result<(RayModel, RcastForm)> {
        let verts = self.collector.verts().to_vec();
        let tris: Vec<[u32; 3]> = self.collector.tris().iter().map(|t| t.verts).collect();
        let dummy: Vec<u32> = self.collector.tris().iter().map(|t| t.dummy).collect();
        let model = RayModel::new(&verts, &tris, self.faces, Opacity::Squared, false)?;
        Ok((model, RcastForm { bounds, verts, tris, dummy, faces: self.rc_faces }))
    }
}

/// A face off the list, or an equal face that stops another: see [`casting_faces`].
const BLOCKED: u32 = u32::MAX;

/// The faces of a mesh the soup of shadow casters takes, in list order, as X-Ray's compiler's loop
/// takes them: a face whose shader casts a shadow, unless an equal face around it was taken before;
/// `processed` is left set on exactly the faces taken. The test of every face runs in parallel, the
/// choice between equal faces in order.
fn casting_faces(
    mesh: &mut Mesh,
    shaders: &Shaders,
    materials: &[BuildMaterial],
    ctl: &Ctl<'_>,
) -> Result<Vec<FaceId>> {
    let faces = mesh.faces().to_vec();
    for &f in &faces {
        if let Some(face) = mesh.face_mut(f) {
            face.flags.processed = false;
        }
    }
    // where each face stands in the list
    let mut at = vec![BLOCKED; faces.iter().map(|f| f.0 as usize + 1).max().unwrap_or(0)];
    for (i, &f) in faces.iter().enumerate() {
        match at.get_mut(f.0 as usize) {
            Some(slot) if *slot == BLOCKED => *slot = i as u32,
            // a face listed twice meets its own mark: the loop as it is written
            _ => return casting_faces_in_order(mesh, &faces, shaders, materials, ctl),
        }
    }
    let m = &*mesh;
    // per face: `None` when it is never taken, else the equal faces before it that would stop it
    let before: Vec<Option<SmallVec<[u32; 2]>>> = faces
        .par_iter()
        .enumerate()
        .with_min_len(1024)
        .map(|(i, &f)| {
            if i.is_multiple_of(4096) && ctl.cancelled() {
                return None;
            }
            let face = m.face(f)?;
            if !casts(shader_of(face.material, shaders, materials)) {
                return None;
            }
            m.positions(f)?;
            let mut adjacent: SmallVec<[FaceId; 36]> = SmallVec::new();
            for v in face.v {
                if let Some(vx) = m.vertex(v) {
                    adjacent.extend_from_slice(&vx.adjacents);
                }
            }
            adjacent.sort_unstable();
            adjacent.dedup();
            let mut out = SmallVec::new();
            for &t in &adjacent {
                let Some(x) = m.face(t) else { continue };
                if t == f || !face.is_equal(x) {
                    continue;
                }
                match at.get(t.0 as usize).copied() {
                    Some(j) if j != BLOCKED => {
                        if (j as usize) < i {
                            out.push(j);
                        }
                    }
                    // a face off the list keeps the mark it has
                    _ => {
                        if x.flags.processed {
                            out.push(BLOCKED);
                        }
                    }
                }
            }
            Some(out)
        })
        .collect();
    ctl.check()?;
    let mut taken = vec![false; faces.len()];
    let mut out = Vec::new();
    for (i, b) in before.iter().enumerate() {
        let Some(b) = b else { continue };
        if b.iter().any(|&j| j == BLOCKED || taken.get(j as usize).copied().unwrap_or(false)) {
            continue;
        }
        if let (Some(slot), Some(&f)) = (taken.get_mut(i), faces.get(i)) {
            *slot = true;
            out.push(f);
        }
    }
    for &f in &out {
        if let Some(face) = mesh.face_mut(f) {
            face.flags.processed = true;
        }
    }
    Ok(out)
}

/// [`casting_faces`] as X-Ray's compiler's loop is written, for a list that names a face twice.
fn casting_faces_in_order(
    mesh: &mut Mesh,
    faces: &[FaceId],
    shaders: &Shaders,
    materials: &[BuildMaterial],
    ctl: &Ctl<'_>,
) -> Result<Vec<FaceId>> {
    let mut out = Vec::new();
    let mut adjacent: Vec<FaceId> = Vec::with_capacity(36);
    for (n, &f) in faces.iter().enumerate() {
        if n.is_multiple_of(4096) {
            ctl.check()?;
        }
        let Some(face) = mesh.face(f) else { continue };
        if !casts(shader_of(face.material, shaders, materials)) {
            continue;
        }
        adjacent.clear();
        for v in face.v {
            if let Some(vx) = mesh.vertex(v) {
                adjacent.extend_from_slice(&vx.adjacents);
            }
        }
        adjacent.sort_unstable();
        adjacent.dedup();
        let already = adjacent
            .iter()
            .any(|&t| t != f && mesh.face(t).is_some_and(|x| x.flags.processed && face.is_equal(x)));
        if already || mesh.positions(f).is_none() {
            continue;
        }
        out.push(f);
        if let Some(face) = mesh.face_mut(f) {
            face.flags.processed = true;
        }
    }
    Ok(out)
}

/// What a ray and `build.cform` keep of a face.
fn described(face: &Face, materials: &[BuildMaterial], level: Option<FaceId>) -> (RayFace, RcFace) {
    let tc0 = face.tc0();
    let texture = materials.get(usize::from(face.material)).map_or(u32::MAX, |m| u32::from(m.surface));
    let ray = RayFace { face: level, cast_shadow: true, opaque: face.flags.opaque, texture, uv: tc0 };
    let rc =
        RcFace { material: face.material, material_game: u32::from(face.material_game), t: tc0, reserved: 0 };
    (ray, rc)
}

/// The ray model of a build: the shadow-casting faces of the level (one of each set of equal
/// faces) and of every static model's reference, with the soup as `build.cform` holds it.
/// `bounds` is the box of the build's vertices; `ctl` stops it between faces.
///
/// The soup is what X-Ray's compiler's collector makes of these faces added in order
/// (`gz_fmt_level::CollectorPacked`, welded within `EPS_L`), welded here all at once
/// ([`weld_packed`]); a model's faces are chosen once for all its references.
pub fn build_ray_model(
    mesh: &mut Mesh,
    mu_models: &mut [MuModel],
    mu_refs: &[MuReference],
    shaders: &Shaders,
    materials: &[BuildMaterial],
    bounds: &Aabb,
    ctl: &Ctl<'_>,
) -> Result<(RayModel, RcastForm)> {
    let level = casting_faces(mesh, shaders, materials, ctl)?;
    let mut chosen: Vec<Option<Vec<FaceId>>> = vec![None; mu_models.len()];
    for r in mu_refs {
        let i = r.model as usize;
        if chosen.get(i).is_some_and(Option::is_none)
            && let Some(m) = mu_models.get_mut(i)
        {
            let list = casting_faces(&mut m.mesh, shaders, materials, ctl)?;
            if let Some(slot) = chosen.get_mut(i) {
                *slot = Some(list);
            }
        }
    }
    let mut points = Corners::with_capacity(level.len() * 3);
    let mut faces: Vec<RayFace> = Vec::with_capacity(level.len());
    let mut rc_faces: Vec<RcFace> = Vec::with_capacity(level.len());
    for &f in &level {
        let (Some(face), Some(p)) = (mesh.face(f), mesh.positions(f)) else { continue };
        let (ray, rc) = described(face, materials, Some(f));
        for (v, p) in face.v.iter().zip(p) {
            points.push(v.0, p);
        }
        faces.push(ray);
        rc_faces.push(rc);
    }
    for r in mu_refs {
        let i = r.model as usize;
        let (Some(m), Some(Some(list))) = (mu_models.get(i), chosen.get(i)) else { continue };
        points.begin();
        for &f in list {
            let (Some(face), Some(p)) = (m.mesh.face(f), m.mesh.positions(f)) else { continue };
            let (ray, rc) = described(face, materials, None);
            for (v, p) in face.v.iter().zip(p) {
                points.push(v.0, r.xform.transform_tiny(p));
            }
            faces.push(ray);
            rc_faces.push(rc);
        }
    }
    ctl.check()?;
    let stop = || ctl.cancelled();
    let Some((verts, index)) = weld_packed(bounds, &points.points, Some(&points.same), &stop) else {
        ctl.check()?;
        return Err(LcError::Cancelled);
    };
    drop(points);
    let tris: Vec<[u32; 3]> = index.as_chunks::<3>().0.to_vec();
    let dummy: Vec<u32> = (0..tris.len() as u32).collect();
    ctl.check()?;
    let model = RayModel::new(&verts, &tris, faces, Opacity::Squared, false)?;
    Ok((model, RcastForm { bounds: *bounds, verts, tris, dummy, faces: rc_faces }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::Matrix;
    use gz_fmt_level::ray_triangle;

    fn face(opaque: bool) -> RayFace {
        RayFace { face: None, cast_shadow: true, opaque, texture: 0, uv: [Vec2::ZERO, Vec2::X, Vec2::Y] }
    }

    #[test]
    fn a_ray_is_stopped_by_an_opaque_face_and_dimmed_by_alpha() {
        // two quads across the y axis at heights 1 and 2
        let verts = vec![
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(0.0, 1.0, 2.0),
            Vec3::new(-1.0, 2.0, -1.0),
            Vec3::new(1.0, 2.0, -1.0),
            Vec3::new(0.0, 2.0, 2.0),
        ];
        let tris = [[0, 1, 2], [3, 4, 5]];
        // a texture of alpha 128 everywhere
        let tex = BuildTexture {
            width: 2,
            height: 2,
            has_alpha: true,
            surface: Some(vec![0x8000_0000; 4]),
            ..BuildTexture::default()
        };
        let textures = [tex];
        let up = Vec3::Y;
        let (mut hits, mut cached) = (Vec::new(), NO_TRI);

        let clear =
            RayModel::new(&verts, &tris, vec![face(false), face(false)], Opacity::Squared, false).unwrap();
        // (128 / 255)^2 = 0.25197; two layers: (1 - 0.25197)^2 = 0.55955
        let a = 128.0f32 / 255.0;
        let one = 1.0 - a * a;
        let got = clear.trace(&textures, &mut hits, &mut cached, Vec3::ZERO, up, 10.0, None);
        assert!((got - one * one).abs() < 1e-6, "{got}");
        // out of range of the upper one
        let got = clear.trace(&textures, &mut hits, &mut cached, Vec3::ZERO, up, 1.5, None);
        assert!((got - one).abs() < 1e-6);
        // the detail compiler's rule is linear
        let linear =
            RayModel::new(&verts, &tris, vec![face(false), face(false)], Opacity::Linear, false).unwrap();
        let got = linear.trace(&textures, &mut hits, &mut cached, Vec3::ZERO, up, 1.5, None);
        assert!((got - (1.0 - a)).abs() < 1e-6);

        let solid =
            RayModel::new(&verts, &tris, vec![face(false), face(true)], Opacity::Squared, false).unwrap();
        assert_eq!(solid.trace(&textures, &mut hits, &mut cached, Vec3::ZERO, up, 10.0, None), 0.0);
        assert_eq!(cached, 1);
        // the cached triangle answers again, the same
        assert_eq!(solid.trace(&textures, &mut hits, &mut cached, Vec3::ZERO, up, 10.0, None), 0.0);
        // a culled model sees only fronts: a ray going up meets these triangles' fronts
        let culled =
            RayModel::new(&verts, &tris, vec![face(true), face(true)], Opacity::Linear, true).unwrap();
        let mut c2 = NO_TRI;
        let from_above =
            culled.trace(&textures, &mut hits, &mut c2, Vec3::new(0.0, 3.0, 0.0), -up, 10.0, None);
        let from_below = culled.trace(&textures, &mut hits, &mut c2, Vec3::ZERO, up, 10.0, None);
        assert!(from_above != from_below, "one side must be culled");
    }

    /// The cache saves work and never changes an answer: rays traced with a cold cache each, with
    /// one warm cache in order, and with one warm cache in the reverse order give the same light.
    /// The rays are the review's case - vertical and horizontal ones through corners and edges of
    /// a grid, origins on box faces - among oblique ones.
    #[test]
    fn a_warm_cache_answers_as_a_cold_one_in_any_order() {
        // a stack of ceilings and walls on a half-metre grid, some transparent
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut faces = Vec::new();
        let mut s = 0x9E37_79B9u32;
        let mut next = move || {
            s ^= s << 13;
            s ^= s >> 17;
            s ^= s << 5;
            s
        };
        for i in 0..300u32 {
            let mut g = || (next() % 13) as f32 * 0.5;
            let (x, y, z) = (g(), g(), g());
            let (a, b) = (g() * 0.5 + 0.5, g() * 0.5 + 0.5);
            let base = verts.len() as u32;
            match i % 3 {
                0 => verts.extend([Vec3::new(x, y, z), Vec3::new(x + a, y, z), Vec3::new(x, y, z + b)]),
                1 => verts.extend([Vec3::new(x, y, z), Vec3::new(x, y + a, z), Vec3::new(x + b, y, z)]),
                _ => verts.extend([Vec3::new(x, y, z), Vec3::new(x, y, z + a), Vec3::new(x, y + b, z)]),
            }
            tris.push([base, base + 1, base + 2]);
            faces.push(RayFace { face: Some(FaceId(i)), ..face(i % 4 != 0) });
        }
        let tex = BuildTexture {
            width: 2,
            height: 2,
            has_alpha: true,
            surface: Some(vec![0x8000_0000, 0x4000_0000, 0xC000_0000, 0x1000_0000]),
            ..BuildTexture::default()
        };
        let textures = [tex];
        let model = RayModel::new(&verts, &tris, faces, Opacity::Squared, false).unwrap();

        let mut rays = Vec::new();
        for (i, p) in verts.iter().enumerate() {
            let dirs = [Vec3::Y, Vec3::NEG_Y, Vec3::X, Vec3::NEG_Z, Vec3::new(0.3, 0.9, -0.2).normalize()];
            let d = dirs[i % dirs.len()];
            let skip = (i % 7 == 0).then_some(FaceId((i / 3) as u32));
            rays.push((*p - d * 0.5, d, 8.0, skip));
            rays.push((*p + Vec3::new(0.25, 0.0, 0.0) - d * 0.5, d, 8.0, None));
        }
        let cold: Vec<f32> = rays
            .iter()
            .map(|&(p, d, r, skip)| {
                let (mut hits, mut cached) = (Vec::new(), NO_TRI);
                model.trace(&textures, &mut hits, &mut cached, p, d, r, skip)
            })
            .collect();
        let warm = |order: &mut dyn Iterator<Item = usize>| {
            let (mut hits, mut cached) = (Vec::new(), NO_TRI);
            let mut out = vec![f32::NAN; rays.len()];
            for i in order {
                let (p, d, r, skip) = rays[i];
                out[i] = model.trace(&textures, &mut hits, &mut cached, p, d, r, skip);
            }
            out
        };
        let forward = warm(&mut (0..rays.len()));
        let backward = warm(&mut (0..rays.len()).rev());
        let bits = |v: &[f32]| v.iter().map(|x| x.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&forward), bits(&cold), "a warm cache in order");
        assert_eq!(bits(&backward), bits(&cold), "a warm cache in reverse");
        let shadowed = cold.iter().filter(|&&x| x == 0.0).count();
        let partial = cold.iter().filter(|&&x| x > 0.0 && x < 1.0).count();
        assert!(
            shadowed > 50 && partial > 20,
            "{shadowed} shadowed, {partial} partly lit: the test proves little"
        );
    }

    /// `trace` as it was before the four-wide tree: the walk of `gz_fmt_level`'s scalar tree over
    /// every triangle, the hits as it reports them. The reference the tree is held to.
    #[allow(clippy::too_many_arguments)]
    fn trace_reference(
        model: &RayModel,
        bvh: &Bvh,
        textures: &[BuildTexture],
        scratch: &mut Vec<RayHit>,
        cached: &mut u32,
        p: Vec3,
        d: Vec3,
        range: f32,
        skip: Option<FaceId>,
    ) -> f32 {
        use std::ops::ControlFlow;
        let ray = model.ray(p, d, range);
        let skipped = |f: &RayFace| skip.is_some() && f.face == skip;
        if let (Some(tri), Some(face)) = (model.tri(*cached), model.face(*cached))
            && !skipped(face)
            && let Some((t, _, _)) = ray_triangle(&ray, tri)
            && t > 0.0
            && t < range
        {
            return 0.0;
        }
        scratch.clear();
        let mut stopped = NO_TRI;
        bvh.ray_visit(&ray, |hit| {
            let Some(face) = model.face(hit.tri) else { return ControlFlow::Continue(()) };
            if skipped(face) || !face.cast_shadow {
                return ControlFlow::Continue(());
            }
            if face.opaque {
                stopped = hit.tri;
                return ControlFlow::Break(());
            }
            scratch.push(hit);
            ControlFlow::Continue(())
        });
        if stopped != NO_TRI {
            *cached = stopped;
            return 0.0;
        }
        scratch.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.tri.cmp(&b.tri)));
        let mut scale = 1.0f32;
        for hit in scratch.iter() {
            let Some(face) = model.face(hit.tri) else { continue };
            match model.transparency(textures, face, hit) {
                Some(o) => scale *= o,
                None => {
                    *cached = hit.tri;
                    return 0.0;
                }
            }
        }
        scale
    }

    /// xorshift: a fixed sequence, no dependency.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        /// In `-1..1`.
        fn signed(&mut self) -> f32 {
            (self.next() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    /// Walls, floors and ceilings on a half-metre grid (stacks of transparent ones among them, some
    /// repeated exactly, some casting no shadow), oblique triangles across them, degenerate ones and
    /// one with a NaN; four textures: alphas, none, no pixels, out of range.
    fn comparison_scene(seed: u64, opacity: Opacity, cull: bool) -> (RayModel, Vec<BuildTexture>, Vec<Vec3>) {
        let mut rng = Rng(seed);
        let mut verts = Vec::new();
        let mut tris = Vec::new();
        let mut faces = Vec::new();
        let mut push = |corners: [Vec3; 3], face: RayFace| {
            let base = verts.len() as u32;
            verts.extend(corners);
            tris.push([base, base + 1, base + 2]);
            faces.push(face);
        };
        let describe = |rng: &mut Rng, i: u32| RayFace {
            face: (!i.is_multiple_of(5)).then_some(FaceId(i / 2)),
            cast_shadow: !i.is_multiple_of(13),
            opaque: rng.below(5) == 0,
            texture: [0, 0, 0, 1, 2, 7][rng.below(6) as usize],
            uv: [Vec2::ZERO, Vec2::new(1.3, -0.2), Vec2::new(-0.7, 2.1)],
        };
        for i in 0..2400u32 {
            let g = |r: &mut Rng| (r.below(17) as f32) * 0.5;
            let (x, y, z) = (g(&mut rng), g(&mut rng), g(&mut rng));
            let (a, b) = (g(&mut rng) * 0.5 + 0.5, g(&mut rng) * 0.5 + 0.5);
            let corners = match i % 3 {
                0 => [Vec3::new(x, y, z), Vec3::new(x + a, y, z), Vec3::new(x, y, z + b)],
                1 => [Vec3::new(x, y, z), Vec3::new(x, y + a, z), Vec3::new(x + b, y, z)],
                _ => [Vec3::new(x, y, z), Vec3::new(x, y, z + a), Vec3::new(x, y + b, z)],
            };
            let f = describe(&mut rng, i);
            push(corners, f);
            if i % 17 == 0 {
                // the same triangle again, another face
                let again = describe(&mut rng, i + 1);
                push(corners, again);
            }
        }
        for i in 0..1200u32 {
            let at = Vec3::new(rng.signed() * 5.0 + 4.0, rng.signed() * 5.0 + 4.0, rng.signed() * 5.0 + 4.0);
            let e = |r: &mut Rng| Vec3::new(r.signed(), r.signed(), r.signed()) * 1.5;
            let corners = [at, at + e(&mut rng), at + e(&mut rng)];
            let f = describe(&mut rng, 5000 + i);
            push(corners, f);
        }
        let f = describe(&mut rng, 9001);
        push([Vec3::splat(2.0), Vec3::splat(2.0), Vec3::new(2.0, 3.0, 2.0)], f);
        let f = describe(&mut rng, 9002);
        push([Vec3::ZERO, Vec3::splat(1.0), Vec3::splat(2.0)], f);
        let f = describe(&mut rng, 9003);
        push([Vec3::new(f32::NAN, 1.0, 1.0), Vec3::ONE, Vec3::Y], f);
        let alphas = BuildTexture {
            width: 2,
            height: 2,
            has_alpha: true,
            surface: Some(vec![0x8000_0000, 0x4000_0000, 0xC000_0000, 0x1000_0000]),
            ..BuildTexture::default()
        };
        let solid = BuildTexture {
            width: 1,
            height: 1,
            has_alpha: true,
            surface: Some(vec![0xFF00_0000]),
            ..BuildTexture::default()
        };
        let bare =
            BuildTexture { width: 4, height: 4, has_alpha: true, surface: None, ..BuildTexture::default() };
        let model = RayModel::new(&verts, &tris, faces, opacity, cull).unwrap();
        (model, vec![alphas, solid, bare], verts)
    }

    type Case = (Vec3, Vec3, f32, Option<FaceId>);

    /// A hundred thousand rays: along the axes through every corner and every edge's middle, from
    /// on the corners' planes and from away; oblique ones from corners and from anywhere; short
    /// and long, some skipping the face they start on.
    fn comparison_rays(seed: u64, verts: &[Vec3], count: usize) -> Vec<Case> {
        let mut rng = Rng(seed);
        let axes = [Vec3::X, Vec3::NEG_X, Vec3::Y, Vec3::NEG_Y, Vec3::Z, Vec3::NEG_Z];
        let mut out = Vec::with_capacity(count);
        while out.len() < count {
            let i = rng.below(verts.len() as u64) as usize;
            let a = verts[i];
            let b = verts[(i / 3) * 3 + (i + 1) % 3];
            if !(a.is_finite() && b.is_finite()) {
                continue;
            }
            let skip = (rng.below(4) == 0).then_some(FaceId((i / 3) as u32 / 2));
            let range = [1000.0, 3.0, 0.5, 12.0][rng.below(4) as usize];
            let ray = match rng.below(6) {
                0 => {
                    let d = axes[rng.below(6) as usize];
                    (a - d * (rng.below(20) as f32 * 0.5), d, range, skip)
                }
                1 => {
                    let d = axes[rng.below(6) as usize];
                    ((a + b) * 0.5 - d * 6.0, d, range, skip)
                }
                2 => {
                    // from the corner itself: an origin on the faces of the boxes around it
                    let d = axes[rng.below(6) as usize];
                    (a, d, range, skip)
                }
                3 => {
                    let d = Vec3::new(rng.signed(), rng.signed(), rng.signed()).normalize_or(Vec3::Y);
                    (a + d * 0.01, d, range, skip)
                }
                4 => {
                    // a diagonal on the grid: two axes move, one stands still
                    let d = Vec3::new(rng.signed().signum(), 0.0, rng.signed().signum()).normalize();
                    (a - d * 3.0, d, range, skip)
                }
                _ => {
                    let p = Vec3::new(rng.signed(), rng.signed(), rng.signed()) * 6.0 + Vec3::splat(4.0);
                    let d = Vec3::new(rng.signed(), rng.signed(), rng.signed()).normalize_or(Vec3::X);
                    (p, d, range, None)
                }
            };
            out.push(ray);
        }
        out
    }

    fn bits(v: &[f32]) -> Vec<u32> {
        v.iter().map(|x| x.to_bits()).collect()
    }

    /// The four-wide tree against the scalar one, ray by ray and bit for bit: each with a cold cache
    /// per ray, each with one warm cache in order, and the new one in chunks on many threads and on
    /// one, for the level's rule and for the detail light's (culled, linear).
    #[test]
    fn the_tree_traces_what_the_scalar_tree_traced() {
        for (seed, opacity, cull) in
            [(0x2545_F491_4F6C_DD1D, Opacity::Squared, false), (0x9E37_79B9, Opacity::Linear, true)]
        {
            let (model, textures, verts) = comparison_scene(seed, opacity, cull);
            let scalar = model.pick().unwrap();
            assert_eq!(model.tree.len(), model.faces().iter().filter(|f| f.cast_shadow).count() - 1);
            let rays = comparison_rays(seed ^ 0xFFFF, &verts, 100_000);
            let cold_new: Vec<f32> = rays
                .par_iter()
                .map(|&(p, d, r, skip)| {
                    let (mut hits, mut cached) = (Vec::new(), NO_TRI);
                    model.trace(&textures, &mut hits, &mut cached, p, d, r, skip)
                })
                .collect();
            let cold_old: Vec<f32> = rays
                .par_iter()
                .map(|&(p, d, r, skip)| {
                    let (mut hits, mut cached) = (Vec::new(), NO_TRI);
                    trace_reference(&model, scalar, &textures, &mut hits, &mut cached, p, d, r, skip)
                })
                .collect();
            for (i, (a, b)) in cold_new.iter().zip(&cold_old).enumerate() {
                assert_eq!(a.to_bits(), b.to_bits(), "ray {i} {:?}: {a} against {b}", rays[i]);
            }
            let warm_old = {
                let (mut hits, mut cached) = (Vec::new(), NO_TRI);
                rays.iter()
                    .map(|&(p, d, r, skip)| {
                        trace_reference(&model, scalar, &textures, &mut hits, &mut cached, p, d, r, skip)
                    })
                    .collect::<Vec<_>>()
            };
            assert_eq!(bits(&warm_old), bits(&cold_old), "the reference with a warm cache");
            let chunked = || -> Vec<f32> {
                let mut out = vec![f32::NAN; rays.len()];
                rays.par_chunks(256).zip(out.par_chunks_mut(256)).for_each(|(rays, out)| {
                    let (mut hits, mut cached) = (Vec::new(), NO_TRI);
                    for (&(p, d, r, skip), o) in rays.iter().zip(out.iter_mut()) {
                        *o = model.trace(&textures, &mut hits, &mut cached, p, d, r, skip);
                    }
                });
                out
            };
            assert_eq!(bits(&chunked()), bits(&cold_old), "warm caches on many threads");
            let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
            assert_eq!(bits(&pool.install(chunked)), bits(&cold_old), "warm caches on one thread");
            let shadowed = cold_old.iter().filter(|&&x| x == 0.0).count();
            let partial = cold_old.iter().filter(|&&x| x > 0.0 && x < 1.0).count();
            let free = cold_old.iter().filter(|&&x| x == 1.0).count();
            assert!(
                shadowed > 10_000 && partial > 3_000 && free > 10_000,
                "{shadowed} shadowed, {partial} partly lit, {free} free: the test proves little"
            );
        }
    }

    /// The seeded comparison's one disagreement before the trees shared their leaves: a ray along a
    /// grid diagonal meets a triangle on its very corner; the triangle test takes it, but the box
    /// around it is left a few units in the last place short (the subtraction `8 - 7.8786798` keeps
    /// few digits). The scalar walk loses that hit, and so must this one, or the light would change.
    #[test]
    fn a_hit_the_scalar_walk_loses_on_a_box_border_is_lost_alike() {
        let verts = [Vec3::new(4.5, 7.0, 8.0), Vec3::new(4.5, 8.25, 8.0), Vec3::new(8.0, 7.0, 8.0)];
        let model = RayModel::new(&verts, &[[0, 1, 2]], vec![face(true)], Opacity::Squared, false).unwrap();
        let (p, d) = (Vec3::new(7.878_68, 7.0, 8.121_321), Vec3::new(0.707_106_77, 0.0, -0.707_106_77));
        let ray = model.ray(p, d, 0.5);
        assert_eq!(ray_triangle(&ray, &verts), None, "the triangle's own box refuses the ray");
        let mut all = Vec::new();
        model.pick().unwrap().ray_all(&ray, &mut all);
        assert!(all.is_empty(), "the scalar walk loses the hit");
        let (mut hits, mut cached) = (Vec::new(), NO_TRI);
        assert_eq!(model.trace(&[], &mut hits, &mut cached, p, d, 0.5, None), 1.0);
    }

    /// A mesh of quads on a grid, some faces repeated over the same vertices (equal faces), some of a
    /// material that casts no shadow, and a model placed twice.
    fn build_scene() -> (Mesh, Vec<MuModel>, Vec<MuReference>, Shaders, Vec<BuildMaterial>, Aabb) {
        let mut rng = Rng(0xD1B5_4A32_D192_ED03);
        let shaders = Shaders::new(vec![
            crate::data::default_shader(),
            ShaderXrlc { flags: ShaderXrlc::RENDERING, ..crate::data::default_shader() },
        ]);
        let materials = vec![
            BuildMaterial { surface: 0, reserved: 0, ..BuildMaterial::default() },
            BuildMaterial { surface: 1, reserved: 1, ..BuildMaterial::default() },
            BuildMaterial { surface: 2, reserved: 0, ..BuildMaterial::default() },
        ];
        let grid = |mesh: &mut Mesh, rng: &mut Rng, n: u32, jitter: f32| {
            let mut ids = Vec::new();
            for z in 0..=n {
                for x in 0..=n {
                    // welding distances apart, some
                    let j = if rng.below(4) == 0 {
                        (rng.signed() * jitter, rng.signed() * jitter)
                    } else {
                        (0.0, 0.0)
                    };
                    ids.push(mesh.create_vertex(Vec3::new(
                        x as f32 + j.0,
                        (x * z % 3) as f32 * 0.1,
                        z as f32 + j.1,
                    )));
                }
            }
            let at = |x: u32, z: u32| ids[(z * (n + 1) + x) as usize];
            for z in 0..n {
                for x in 0..n {
                    let quads = [
                        [at(x, z), at(x, z + 1), at(x + 1, z + 1)],
                        [at(x, z), at(x + 1, z + 1), at(x + 1, z)],
                    ];
                    for v in quads {
                        let copies = if rng.below(9) == 0 { 2 } else { 1 };
                        for c in 0..copies {
                            // a copy in another order of its corners is an equal face
                            let v = if c == 1 { [v[1], v[2], v[0]] } else { v };
                            let f = mesh.create_face(v);
                            if let Some(face) = mesh.face_mut(f) {
                                face.material = rng.below(3) as u16;
                                face.material_game = rng.below(5) as u16;
                                face.flags.opaque = rng.below(2) == 0;
                                face.add_channel(Vec2::ZERO, Vec2::X * (x as f32), Vec2::Y * (z as f32));
                            }
                        }
                    }
                }
            }
        };
        let mut mesh = Mesh::new();
        grid(&mut mesh, &mut rng, 40, 0.0015);
        let mut models = Vec::new();
        for _ in 0..3 {
            let mut m = MuModel { mesh: Mesh::new_model(), ..MuModel::default() };
            grid(&mut m.mesh, &mut rng, 5, 0.0015);
            models.push(m);
        }
        let place = |model: u32, dx: f32| {
            let mut xform = Matrix::IDENTITY;
            xform.m[3] = [dx, 0.0005, 3.0, 1.0];
            MuReference {
                model,
                xform,
                flags: 0,
                sector: 0,
                color: Vec::new(),
                c_scale: crate::ColorC::default(),
                c_bias: crate::ColorC::default(),
            }
        };
        let refs = vec![place(0, 2.0), place(1, 10.0), place(0, 2.0005), place(2, 30.0), place(0, 30.0)];
        let bounds = Aabb { min: Vec3::new(-1.0, -1.0, -1.0), max: Vec3::new(42.0, 2.0, 42.0) };
        (mesh, models, refs, shaders, materials, bounds)
    }

    /// The ray model as it was built before: every face through X-Ray's collector as it comes.
    fn build_ray_model_reference(
        mesh: &mut Mesh,
        mu_models: &mut [MuModel],
        mu_refs: &[MuReference],
        shaders: &Shaders,
        materials: &[BuildMaterial],
        bounds: &Aabb,
    ) -> (RayModel, RcastForm) {
        let faces = mesh.faces().to_vec();
        for &f in &faces {
            if let Some(face) = mesh.face_mut(f) {
                face.flags.processed = false;
            }
        }
        let mut cl = RapidCollector::new(bounds, mesh.vertices().len(), faces.len());
        let mut adjacent: Vec<FaceId> = Vec::new();
        for &f in &faces {
            let Some(face) = mesh.face(f) else { continue };
            if !casts(shader_of(face.material, shaders, materials)) {
                continue;
            }
            adjacent.clear();
            for v in face.v {
                if let Some(vx) = mesh.vertex(v) {
                    adjacent.extend_from_slice(&vx.adjacents);
                }
            }
            adjacent.sort_unstable();
            adjacent.dedup();
            let already = adjacent
                .iter()
                .any(|&t| t != f && mesh.face(t).is_some_and(|x| x.flags.processed && face.is_equal(x)));
            if already {
                continue;
            }
            let Some(p) = mesh.positions(f) else { continue };
            let (ray, rc) = described(face, materials, Some(f));
            let sm = face.sm_group;
            cl.add(p, ray, rc, sm);
            if let Some(face) = mesh.face_mut(f) {
                face.flags.processed = true;
            }
        }
        for r in mu_refs {
            if let Some(m) = mu_models.get_mut(r.model as usize) {
                m.export_cform_rcast(&mut cl, &r.xform, shaders, materials);
            }
        }
        cl.finish(*bounds).unwrap()
    }

    #[test]
    fn the_soup_is_what_the_collector_made_of_it() {
        let (mut mesh, mut models, refs, shaders, materials, bounds) = build_scene();
        let (mut mesh2, mut models2) = (mesh.clone(), models.clone());
        let (old_model, old_form) =
            build_ray_model_reference(&mut mesh, &mut models, &refs, &shaders, &materials, &bounds);
        let (model, form) =
            build_ray_model(&mut mesh2, &mut models2, &refs, &shaders, &materials, &bounds, &Ctl::quiet())
                .unwrap();
        assert!(
            old_form.verts.len() < old_form.tris.len() * 3 / 2,
            "nothing was welded: the test proves little"
        );
        assert!(old_form.tris.len() > 2000 && old_form.tris.len() < mesh.faces().len() + 400);
        assert!(form.write() == old_form.write(), "build.cform differs");
        assert_eq!(model.tris(), old_model.tris());
        assert_eq!(model.faces(), old_model.faces());
        let marks =
            |m: &Mesh| m.faces().iter().map(|&f| m.face(f).unwrap().flags.processed).collect::<Vec<_>>();
        assert_eq!(marks(&mesh2), marks(&mesh));
        for (a, b) in models2.iter().zip(&models) {
            assert_eq!(marks(&a.mesh), marks(&b.mesh));
        }
        // one thread: the same soup
        let pool = rayon::ThreadPoolBuilder::new().num_threads(1).build().unwrap();
        let (mut mesh3, mut models3) = (mesh2.clone(), models2.clone());
        let (_, one) = pool
            .install(|| {
                build_ray_model(&mut mesh3, &mut models3, &refs, &shaders, &materials, &bounds, &Ctl::quiet())
            })
            .unwrap();
        assert!(one.write() == old_form.write());
    }

    #[test]
    fn the_cform_file_round_trips() {
        let form = RcastForm {
            bounds: Aabb { min: Vec3::ZERO, max: Vec3::ONE },
            verts: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
            tris: vec![[0, 1, 2]],
            dummy: vec![0],
            faces: vec![RcFace {
                material: 3,
                material_game: 7,
                t: [Vec2::ZERO, Vec2::X, Vec2::ONE],
                reserved: 0,
            }],
        };
        let bytes = form.write();
        assert_eq!(bytes.len(), 8 + 36 + 36 + 16 + 8 + 36);
        assert_eq!(RcastForm::read(&bytes).unwrap(), form);
    }
}
