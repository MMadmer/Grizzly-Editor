//! The ray model the light is traced in: the collision soup of `CBuild::BuildRapid` (the
//! level's shadow-casting faces welded by `CDB::CollectorPacked`, the MU references'
//! `export_cform_rcast`) in a BVH (`gz_fmt_level::Bvh`, built on the caller's pool), with what a
//! ray needs of every triangle: the face it stands for, whether it casts a shadow, whether it is
//! opaque, its texture and texture coordinates. `trace` is `rayTrace` with `getLastRP_Scale`.
//!
//! **The order of hits.** The C++ multiplies the transparencies of the hits in the order its
//! tree reports them and stops at the first opaque one; here an opaque hit stops the walk (the
//! answer is 0 either way) and the transparent ones are multiplied nearest first, so the answer
//! is the same whatever tree holds the triangles.
//!
//! **The cached triangle.** The C++ keeps, per light, the opaque triangle that last stopped a
//! ray and tests it before the tree; that state lives in the shared light and makes the answer
//! depend on which texel came first. Here the cache is the worker's ([`LightCache`]) and is
//! tested with the tree's own box and triangle tests (`ray_triangle`: a hit there is a hit of the
//! walk, whose box test is monotonic in the box), never for the skipped face, so a cached answer
//! is the answer the tree would give: the light is the same on any number of threads, whatever
//! the rays traced before. A cold and a warm cache, in either order, are tested to agree.

use std::ops::ControlFlow;

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};
use gz_fmt_level::{Aabb, Bvh, CollectorPacked, Ray, RayHit, ray_triangle};

use crate::data::{BuildMaterial, BuildTexture, ShaderXrlc, Shaders, shader_of};
use crate::light::LightSet;
use crate::math::{color_get_a, i_floor};
use crate::mesh::FaceId;
use crate::{LcError, Result};

/// `CFORM_CURRENT_VERSION`.
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

/// What a ray needs of one triangle (the `CDB::TRI::pointer` of the C++, resolved).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RayFace {
    /// The level face it stands for: a ray cast from that face skips it. `None` for an MU
    /// model's face and a face of `build.cform`.
    pub face: Option<FaceId>,
    /// `bLIGHT_CastShadow` of its shader.
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
    bvh: Bvh,
    tris: Vec<[Vec3; 3]>,
    faces: Vec<RayFace>,
    opacity: Opacity,
    /// `CDB::OPT_CULL`: the detail compiler's rays see only the faces turned towards them.
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
        let bvh = Bvh::build(verts, tris)?;
        let corner = |i: u32| verts.get(i as usize).copied().unwrap_or_default();
        let tris = tris.iter().map(|t| [corner(t[0]), corner(t[1]), corner(t[2])]).collect();
        Ok(Self { bvh, tris, faces, opacity, cull })
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.tris.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }

    #[must_use]
    pub fn tri(&self, i: u32) -> Option<&[Vec3; 3]> {
        self.tris.get(i as usize)
    }

    #[must_use]
    pub fn face(&self, i: u32) -> Option<&RayFace> {
        self.faces.get(i as usize)
    }

    /// The triangles that touch a box (`box_query` with `OPT_FULL_TEST`), by index.
    pub fn query_box(&self, b: &Aabb, out: &mut Vec<u32>) {
        self.bvh.query_box(b, out);
    }

    fn ray(&self, p: Vec3, d: Vec3, range: f32) -> Ray {
        Ray { origin: p, dir: d, t_max: range, cull_back: self.cull }
    }

    /// How much of a transparent face's light gets through where the ray met it; `None` when
    /// its texture has no surface (the C++ takes the face as opaque then).
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

    /// `rayTrace`: how much light gets from `p` along `d` (of unit length) over `range`: 1 for a
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
        let ray = self.ray(p, d, range);
        let skipped = |f: &RayFace| skip.is_some() && f.face == skip;
        // 1. the triangle that last stopped this light
        if let (Some(tri), Some(face)) = (self.tri(*cached), self.face(*cached))
            && !skipped(face)
            && let Some((t, _, _)) = ray_triangle(&ray, tri)
            && t > 0.0
            && t < range
        {
            return 0.0;
        }
        // 2. the tree
        scratch.clear();
        let mut stopped = NO_TRI;
        self.bvh.ray_visit(&ray, |hit| {
            let Some(face) = self.face(hit.tri) else { return ControlFlow::Continue(()) };
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
        // 3. the transparent ones, nearest first
        scratch.sort_by(|a, b| a.t.total_cmp(&b.t).then(a.tri.cmp(&b.tri)));
        let mut scale = 1.0f32;
        for hit in scratch.iter() {
            let Some(face) = self.face(hit.tri) else { continue };
            match self.transparency(textures, face, hit) {
                Some(o) => scale *= o,
                None => {
                    *cached = hit.tri;
                    return 0.0;
                }
            }
        }
        scale
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

    /// Forgets the cached triangles (another light set, another model).
    pub fn reset(&mut self) {
        self.rgb.iter_mut().chain(self.hemi.iter_mut()).chain(self.sun.iter_mut()).for_each(|t| *t = NO_TRI);
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

/// The collision soup `BuildRapid` makes, as `build.cform` holds it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RcastForm {
    /// `scene_bb`.
    pub bounds: Aabb,
    pub verts: Vec<Vec3>,
    pub tris: Vec<[u32; 3]>,
    /// The word after a triangle's corners; the C++ writes the low half of a face pointer, the
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

    /// Reads `build.cform` (`global_claculation_data::xrLoad`, "Load CFORM").
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
        RayModel::new(&self.verts, &self.tris, faces, Opacity::Linear, true)
    }
}

/// `bLIGHT_CastShadow`.
#[must_use]
pub fn casts(s: &ShaderXrlc) -> bool {
    s.flags & ShaderXrlc::LIGHT_CAST_SHADOW != 0
}

/// What `BuildRapid` collects into a `CollectorPacked`: the soup with its faces.
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

#[cfg(test)]
mod tests {
    use super::*;

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
