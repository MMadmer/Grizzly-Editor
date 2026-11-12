//! `CDeflector`: a group of faces mapped onto one lightmap rectangle, and its light
//! (`xrDeflector.cpp`, `xrDeflectorLight.cpp`, `xrDeflectoL_Direct.cpp`): the projection that
//! gives the faces their texture coordinates (`OA_*`), the texels lit through the faces
//! (`L_Direct`, the jitter of `m_lm_jitter_samples`, `L_Direct_Edge`), the borders, the shrink to
//! nothing or to a smaller size when the light is flat enough (`compress_Zero`, `compress_RMS`
//! with `m_lm_rms_zero` / `m_lm_rms`), and the final border.

use std::sync::atomic::{AtomicUsize, Ordering};

use glam::{Vec2, Vec3};
use gz_fmt_tex::{ImfFilter, imf_resample};
use rayon::prelude::*;

use crate::color::{BaseColor, ColorC};
use crate::data::{BuildMaterial, BuildParams, BuildTexture, Shaders, shader_of};
use crate::hash2d::Hash2D;
use crate::layer::{BORDER, LmLayer, apply_borders, image_of, lblit, pixels_of};
use crate::light::LightSet;
use crate::math::{
    Aabb, Box2, EPS, EPS_S, Matrix, color_get_a, color_get_b, color_get_g, color_get_r, cos_deg,
    exact_normalize, from_bary, i_ceil, i_floor, mad, magnitude, normalize, normalize_safe, sphere_of,
    u8_clr,
};
use crate::mesh::{DeflId, FaceId, Mesh, Tcf};
use crate::point::{Falloff, LP_DEFAULT, LP_DONT_SUN, LP_USE_FACE_DISABLE, Tracer, light_point};
use crate::ray::{LightCache, RayModel};
use crate::{Ctl, Result};

/// `c_LMAP_size`: the side of a lightmap page.
pub const LMAP_SIZE: u32 = 1024;
/// The largest side a deflector is lit at: a page's half less its border.
pub const MAX_LAYER: u32 = 512 - 2 * BORDER;
/// `HASH`: the grid a deflector's triangles are found in.
const HASH_SIDE: usize = 128;

/// `Jitter_Select`: the sample offsets of a texel for `m_lm_jitter_samples` (1, 9, anything else
/// is 4).
#[must_use]
pub fn jitter(samples: u32) -> &'static [Vec2] {
    const J1: [Vec2; 1] = [Vec2::ZERO];
    const J4: [Vec2; 4] =
        [Vec2::new(-1.0, -1.0), Vec2::new(1.0, -1.0), Vec2::new(1.0, 1.0), Vec2::new(-1.0, 1.0)];
    const J9: [Vec2; 9] = [
        Vec2::new(-1.0, -1.0),
        Vec2::new(0.0, -1.0),
        Vec2::new(1.0, -1.0),
        Vec2::new(-1.0, 0.0),
        Vec2::new(0.0, 0.0),
        Vec2::new(1.0, 0.0),
        Vec2::new(-1.0, 1.0),
        Vec2::new(0.0, 1.0),
        Vec2::new(1.0, 1.0),
    ];
    match samples {
        1 => &J1,
        9 => &J9,
        _ => &J4,
    }
}

/// `rms_zero`: `(4 + m_lm_rms_zero) / 2`.
#[must_use]
pub fn rms_zero(p: &BuildParams) -> u32 {
    (4 + p.lm_rms_zero) / 2
}

/// `rms_shrink`: `(8 + m_lm_rms) / 2`.
#[must_use]
pub fn rms_shrink(p: &BuildParams) -> u32 {
    (8 + p.lm_rms) / 2
}

/// `UVtri`: a face's corners in the deflector's texture space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTri {
    pub uv: [Vec2; 3],
    pub owner: FaceId,
}

/// What the light of the level is computed from.
#[derive(Clone, Copy, Debug)]
pub struct LightEnv<'a> {
    pub mesh: &'a Mesh,
    pub model: &'a RayModel,
    pub textures: &'a [BuildTexture],
    /// `L_static`.
    pub lights: &'a LightSet,
    pub params: &'a BuildParams,
    /// `-nosun`.
    pub nosun: bool,
    /// `-att`: point lights fall off linearly.
    pub linear: bool,
}

impl<'a> LightEnv<'a> {
    #[must_use]
    pub fn tracer(&self) -> Tracer<'a> {
        Tracer { model: self.model, textures: self.textures, falloff: Falloff::Level { linear: self.linear } }
    }

    #[must_use]
    pub fn sun_flag(&self) -> u32 {
        if self.nosun { LP_DONT_SUN } else { 0 }
    }
}

/// `CDeflector`.
#[derive(Clone, Debug, PartialEq)]
pub struct Deflector {
    pub uv_polys: Vec<UvTri>,
    pub normal: Vec3,
    pub layer: LmLayer,
    pub sphere_p: Vec3,
    pub sphere_r: f32,
    /// Placed into a page.
    pub merged: bool,
}

impl Default for Deflector {
    /// `CDeflector()`.
    fn default() -> Self {
        Self {
            uv_polys: Vec::with_capacity(32),
            normal: Vec3::Y,
            layer: LmLayer::default(),
            sphere_p: Vec3::splat(f32::MAX),
            sphere_r: 0.0,
            merged: false,
        }
    }
}

/// The corners, the corner normals and the normal of a deflector's face.
#[derive(Clone, Copy)]
struct TriWorld {
    p: [Vec3; 3],
    n: [Vec3; 3],
    face_n: Vec3,
    face: FaceId,
}

impl Deflector {
    /// `OA_SetNormal`.
    pub fn oa_set_normal(&mut self, n: Vec3) {
        self.normal = normalize(n);
    }

    /// `OA_Place(Face*)`: takes the face when its normal is within the smoothing angle (plus a
    /// degree) of the deflector's.
    pub fn oa_place(&mut self, id: DeflId, mesh: &mut Mesh, f: FaceId, sm_angle: f32) -> bool {
        let Some(face) = mesh.face_mut(f) else { return false };
        let cosa = crate::math::dot(self.normal, face.n);
        if cosa < cos_deg(sm_angle + 1.0) {
            return false;
        }
        face.deflector = Some(id);
        self.uv_polys.push(UvTri { uv: [Vec2::ZERO; 3], owner: f });
        true
    }

    /// `OA_Place(vecFace&)`: takes these faces, whatever their normals.
    pub fn oa_place_all(&mut self, id: DeflId, mesh: &mut Mesh, faces: &[FaceId]) {
        self.uv_polys.clear();
        self.uv_polys.reserve(faces.len());
        for &f in faces {
            if let Some(face) = mesh.face_mut(f) {
                face.deflector = Some(id);
            }
            self.uv_polys.push(UvTri { uv: [Vec2::ZERO; 3], owner: f });
        }
    }

    /// `Face::OA_Unwarp` from `start`: the face and, through the faces of its corners, every
    /// face reachable that has no deflector and fits this one, in the order of the C++
    /// recursion. Answers how many faces were tried (`affected`, from 1).
    pub fn oa_unwarp(&mut self, id: DeflId, mesh: &mut Mesh, start: FaceId, sm_angle: f32) -> u32 {
        let mut affected = 1u32;
        // (face, corner, adjacent) - the recursion without the call stack
        let mut stack: Vec<(FaceId, usize, usize)> = Vec::new();
        let visit =
            |f: FaceId, mesh: &mut Mesh, stack: &mut Vec<(FaceId, usize, usize)>, d: &mut Deflector| {
                if mesh.face(f).is_none_or(|x| x.deflector.is_some()) {
                    return;
                }
                if d.oa_place(id, mesh, f, sm_angle) {
                    stack.push((f, 0, 0));
                }
            };
        visit(start, mesh, &mut stack, self);
        while let Some(top) = stack.last_mut() {
            let (f, corner, k) = *top;
            if corner >= 3 {
                stack.pop();
                continue;
            }
            let next = mesh
                .face(f)
                .and_then(|x| x.v.get(corner).copied())
                .and_then(|v| mesh.vertex(v))
                .and_then(|v| v.adjacents.get(k).copied());
            match next {
                Some(n) => {
                    top.2 += 1;
                    affected += 1;
                    visit(n, mesh, &mut stack, self);
                }
                None => {
                    top.1 += 1;
                    top.2 = 0;
                }
            }
        }
        affected
    }

    /// `OA_Export`: the deflector's normal from its faces' (weighted by area), the faces
    /// projected along it into texture space, the size of its layer from the texture density.
    pub fn oa_export(
        &mut self,
        mesh: &mut Mesh,
        shaders: &Shaders,
        materials: &[BuildMaterial],
        params: &BuildParams,
    ) {
        if self.uv_polys.is_empty() {
            return;
        }
        let mut tn = Vec3::ZERO;
        let mut density = 0.0f32;
        let mut fcount = 0.0f32;
        for t in &self.uv_polys {
            let Some(face) = mesh.face(t.owner) else { continue };
            let k = 1.0 + EPS * mesh.calc_area(t.owner);
            tn = Vec3::new(tn.x + face.n.x * k, tn.y + face.n.y * k, tn.z + face.n.z * k);
            density += shader_of(face.material, shaders, materials).lm_density;
            fcount += 1.0;
        }
        if magnitude(tn) > EPS_S && tn.is_finite() {
            self.normal = normalize(tn);
        } else {
            for t in &self.uv_polys {
                if let Some(p) = mesh.positions(t.owner) {
                    mesh.errors.tjunction.extend_from_slice(&[p[0], p[1], p[1], p[2], p[2], p[0]]);
                }
            }
        }
        density /= fcount;

        // an arbitrarily oriented orthographic projection
        let at = Vec3::ZERO;
        let from = at + self.normal;
        let y = if self.normal.y.abs() > 0.99 { Vec3::X } else { Vec3::Y };
        let right = normalize_safe(crate::math::cross(y, self.normal));
        let up = normalize_safe(crate::math::cross(self.normal, right));
        let view = Matrix::camera(from, at, up);
        let mut bb = Aabb::INVALID;
        for t in &mut self.uv_polys {
            let Some(p) = mesh.positions(t.owner) else { continue };
            for (uv, p) in t.uv.iter_mut().zip(p) {
                let q = view.transform_tiny(p);
                *uv = Vec2::new(q.x, q.y);
                bb.modify(p);
            }
        }
        (self.sphere_p, self.sphere_r) = sphere_of(&bb);

        let (min, max) = self.rect();
        let size = max - min;
        let side =
            |s: f32| (i_ceil(s * params.lm_pixels_per_meter * density + 0.5) as u32).clamp(1, MAX_LAYER);
        let (w, h) = (side(size.x), side(size.y));
        self.layer.create(w, h);
    }

    /// `GetRect`.
    #[must_use]
    pub fn rect(&self) -> (Vec2, Vec2) {
        let Some(first) = self.uv_polys.first() else { return (Vec2::ZERO, Vec2::ZERO) };
        let (mut min, mut max) = (first.uv[0], first.uv[0]);
        for t in &self.uv_polys {
            for p in t.uv {
                min = Vec2::new(crate::math::min_f(min.x, p.x), crate::math::min_f(min.y, p.y));
                max = Vec2::new(crate::math::max_f(max.x, p.x), crate::math::max_f(max.y, p.y));
            }
        }
        (min, max)
    }

    /// `Bounds_Summary`.
    #[must_use]
    pub fn bounds_summary(&self) -> Box2 {
        let mut b = Box2::INVALID;
        for t in &self.uv_polys {
            b.merge(&Box2::of_tri(&t.uv));
        }
        b
    }

    /// `GetBaseMaterial`: the material of the first face.
    #[must_use]
    pub fn base_material(&self, mesh: &Mesh) -> u16 {
        self.uv_polys.first().and_then(|t| mesh.face(t.owner)).map_or(0, |f| f.material)
    }

    /// `GetFaceCount`.
    #[must_use]
    pub fn face_count(&self) -> usize {
        self.uv_polys.len()
    }

    /// `RemapUV(dest, ...)`: the triangles mapped into the rectangle `base .. base + size` of a
    /// `lm_u` x `lm_v` texture, texel centres at its edges; rotated swaps the axes.
    #[allow(clippy::too_many_arguments)]
    #[must_use]
    pub fn remap_uv_into(
        &self,
        base_u: u32,
        base_v: u32,
        size_u: u32,
        size_v: u32,
        lm_u: u32,
        lm_v: u32,
        rotate: bool,
    ) -> Vec<UvTri> {
        let (a_min, a_max) = self.rect();
        let a_size = a_max - a_min;
        let mut d_min = Vec2::new((base_u as f32 + 0.5) / lm_u as f32, (base_v as f32 + 0.5) / lm_v as f32);
        let mut d_max = Vec2::new(
            ((base_u + size_u) as f32 - 0.5) / lm_u as f32,
            ((base_v + size_v) as f32 - 0.5) / lm_v as f32,
        );
        if d_min.x >= d_max.x {
            let c = (d_min.x + d_max.x) / 2.0;
            d_min.x = c - EPS_S;
            d_max.x = c + EPS_S;
        }
        if d_min.y >= d_max.y {
            let c = (d_min.y + d_max.y) / 2.0;
            d_min.y = c - EPS_S;
            d_max.y = c + EPS_S;
        }
        let d_size = d_max - d_min;
        self.uv_polys
            .iter()
            .map(|t| {
                let uv = t.uv.map(|p| {
                    if rotate {
                        Vec2::new(
                            ((p.y - a_min.y) / a_size.y) * d_size.x + d_min.x,
                            ((p.x - a_min.x) / a_size.x) * d_size.y + d_min.y,
                        )
                    } else {
                        Vec2::new(
                            ((p.x - a_min.x) / a_size.x) * d_size.x + d_min.x,
                            ((p.y - a_min.y) / a_size.y) * d_size.y + d_min.y,
                        )
                    }
                });
                UvTri { uv, owner: t.owner }
            })
            .collect()
    }

    /// `RemapUV`: the deflector's own triangles remapped.
    #[allow(clippy::too_many_arguments)]
    pub fn remap_uv(
        &mut self,
        base_u: u32,
        base_v: u32,
        size_u: u32,
        size_v: u32,
        lm_u: u32,
        lm_v: u32,
        rotate: bool,
    ) {
        self.uv_polys = self.remap_uv_into(base_u, base_v, size_u, size_v, lm_u, lm_v, rotate);
    }

    fn world(&self, mesh: &Mesh) -> Vec<Option<TriWorld>> {
        self.uv_polys
            .iter()
            .map(|t| {
                let face = mesh.face(t.owner)?;
                Some(TriWorld {
                    p: mesh.positions(t.owner)?,
                    n: mesh.normals(t.owner)?,
                    face_n: face.n,
                    face: t.owner,
                })
            })
            .collect()
    }

    /// `L_Calculate`: the triangles mapped onto the layer's texels, then lit.
    fn l_calculate(&mut self, env: &LightEnv<'_>, lights: &LightSet) {
        let (w, h) = (self.layer.width, self.layer.height);
        self.remap_uv(0, 0, w, h, w, h, false);
        let boxes: Vec<Box2> = self.uv_polys.iter().map(|t| Box2::of_tri(&t.uv)).collect();
        let hash = Hash2D::build(HASH_SIDE, HASH_SIDE, self.bounds_summary(), &boxes);
        self.layer.create(w.min(LMAP_SIZE - 2 * BORDER), h.min(LMAP_SIZE - 2 * BORDER));
        self.l_direct(env, lights, &hash);
    }

    /// `L_Direct`: every texel lit at its jittered samples through the first triangle that holds
    /// each, then the texels the triangles' edges cross but no sample found. The rows are lit
    /// in parallel; the edges in the C++ order, since one may take a texel another would.
    fn l_direct(&mut self, env: &LightEnv<'_>, lights: &LightSet, hash: &Hash2D) {
        let (w, h) = (self.layer.width as usize, self.layer.height as usize);
        if w == 0 || h == 0 {
            return;
        }
        let tris = self.world(env.mesh);
        let tracer = env.tracer();
        let flags = env.sun_flag() | LP_USE_FACE_DISABLE;
        let dim = Vec2::new(w as f32, h as f32);
        let half = Vec2::new(0.5 / dim.x, 0.5 / dim.y);
        let js = Vec2::new(0.4999 / dim.x, 0.4999 / dim.y);
        let jit = jitter(env.params.lm_jitter_samples);
        let uv_polys = &self.uv_polys;
        let layer = &mut self.layer;
        layer
            .surface
            .par_chunks_mut(w)
            .zip(layer.marker.par_chunks_mut(w))
            .take(h)
            .enumerate()
            .for_each_init(LightCache::default, |cache, (v, (srow, mrow))| {
                for (u, (s, m)) in srow.iter_mut().zip(mrow.iter_mut()).enumerate() {
                    let mut c = ColorC::default();
                    let mut count = 0u32;
                    for j in jit {
                        let p = Vec2::new(
                            u as f32 / dim.x + half.x + j.x * js.x,
                            v as f32 / dim.y + half.y + j.y * js.y,
                        );
                        for &i in hash.query(p.x, p.y) {
                            let Some(t) = uv_polys.get(i as usize) else { continue };
                            let Some(b) = (Tcf { uv: t.uv }).inside(p) else { continue };
                            let Some(Some(world)) = tris.get(i as usize) else { break };
                            let wp = from_bary(world.p[0], world.p[1], world.p[2], b);
                            let (wn, _) = exact_normalize(from_bary(world.n[0], world.n[1], world.n[2], b));
                            let (wn, _) = exact_normalize(wn + world.face_n);
                            light_point(&tracer, cache, &mut c, wp, wn, lights, flags, Some(world.face));
                            count += 1;
                            break;
                        }
                    }
                    if count > 0 {
                        c.scale(count);
                        c.mul(0.5);
                        *m = 255;
                    } else {
                        *m = 0;
                    }
                    *s = BaseColor::from_c(&c);
                }
            });

        // the edges
        let texel = (1.0 / w.max(h) as f32) / 8.0;
        let mut cache = LightCache::default();
        let edge = EdgeLight { tracer: &tracer, lights, flags: env.sun_flag() | LP_DEFAULT, texel };
        for (t, world) in self.uv_polys.iter().zip(&tris) {
            let Some(world) = world else { continue };
            for e in 0..3 {
                let (a, b) = (e, (e + 1) % 3);
                edge.run(&mut self.layer, &mut cache, (t.uv[a], t.uv[b]), (world.p[a], world.p[b]), world);
            }
        }
    }
}

/// What `L_Direct_Edge` lights with.
struct EdgeLight<'a> {
    tracer: &'a Tracer<'a>,
    lights: &'a LightSet,
    flags: u32,
    texel: f32,
}

impl EdgeLight<'_> {
    /// `L_Direct_Edge`: the texels an edge crosses that are still unlit, lit at the edge.
    fn run(
        &self,
        lm: &mut LmLayer,
        cache: &mut LightCache,
        (p1, p2): (Vec2, Vec2),
        (v1, v2): (Vec3, Vec3),
        world: &TriWorld,
    ) {
        let texel = self.texel;
        let vdir = v2 - v1;
        let size = Vec2::new(p2.x - p1.x, p2.y - p1.y);
        let du = i_ceil(size.x.abs() / texel);
        let dv = i_ceil(size.y.abs() / texel);
        let steps = du.max(dv);
        if steps <= 0 {
            return;
        }
        for i in 0..=steps {
            let time = i as f32 / steps as f32;
            let uv = Vec2::new(size.x * time + p1.x, size.y * time + p1.y);
            let x = i_floor(uv.x * lm.width as f32);
            let y = i_floor(uv.y * lm.height as f32);
            if x < 0 || x >= lm.width as i32 || y < 0 || y >= lm.height as i32 {
                continue;
            }
            let id = y as usize * lm.width as usize + x as usize;
            if lm.marker.get(id).is_none_or(|&m| m != 0) {
                continue;
            }
            let mut c = ColorC::default();
            let p = mad(v1, vdir, time);
            light_point(
                self.tracer,
                cache,
                &mut c,
                p,
                world.face_n,
                self.lights,
                self.flags,
                Some(world.face),
            );
            c.mul(0.5);
            if let (Some(s), Some(m)) = (lm.surface.get_mut(id), lm.marker.get_mut(id)) {
                *s = BaseColor::from_c(&c);
                *m = 255;
            }
        }
    }
}

impl Deflector {
    /// `CDeflector::Light`: the layer lit, its borders grown, shrunk when the light allows, and
    /// grown by the final border.
    pub fn light(&mut self, env: &LightEnv<'_>) {
        // the bounds of the faces as they are now
        let mut bb = Aabb::INVALID;
        for t in &self.uv_polys {
            if let Some(p) = env.mesh.positions(t.owner) {
                p.iter().for_each(|p| bb.modify(*p));
            }
        }
        (self.sphere_p, self.sphere_r) = sphere_of(&bb);
        let lights = env.lights.select(self.sphere_p, self.sphere_r);

        self.l_calculate(env, &lights);
        for r in (1..=254).rev() {
            if !apply_borders(&mut self.layer, r) {
                break;
            }
        }

        if compress_zero(&mut self.layer, rms_zero(env.params)) {
            return;
        }
        if let Some((w, h)) = compress_rms(&self.layer, rms_shrink(env.params)) {
            self.layer.create(w, h);
            self.l_calculate(env, &lights);
        }
        expand(&mut self.layer);
    }
}

/// The final border: a layer one texel wide or high becomes a line of two texels and its
/// side is set to 0; any other is copied into a layer grown by its border, and the border
/// filled.
fn expand(layer: &mut LmLayer) {
    let (w, h) = (layer.width, layer.height);
    if w == 1 {
        let mut t = LmLayer::new(2 * BORDER, h + 2 * BORDER);
        for y in 0..t.height {
            let py = (y as i32 - BORDER as i32).clamp(0, h as i32 - 1) as usize;
            let c = layer.surface.get(py).copied().unwrap_or_default();
            for x in 0..2 {
                let at = y as usize * 2 + x;
                if let (Some(s), Some(m)) = (t.surface.get_mut(at), t.marker.get_mut(at)) {
                    *s = c;
                    *m = 255;
                }
            }
        }
        t.width = 0;
        t.height = h;
        *layer = t;
    } else if h == 1 {
        let mut t = LmLayer::new(w + 2 * BORDER, 2 * BORDER);
        for x in 0..t.width {
            let px = (x as i32 - BORDER as i32).clamp(0, w as i32 - 1) as usize;
            let c = layer.surface.get(px).copied().unwrap_or_default();
            for row in 0..2 {
                let at = (row * t.width + x) as usize;
                if let (Some(s), Some(m)) = (t.surface.get_mut(at), t.marker.get_mut(at)) {
                    *s = c;
                    *m = 255;
                }
            }
        }
        t.width = w;
        t.height = 0;
        *layer = t;
    } else {
        let old = std::mem::take(layer);
        let mut grown = LmLayer::new(w + 2 * BORDER, h + 2 * BORDER);
        lblit(&mut grown, &old, BORDER, BORDER, 255 - BORDER);
        *layer = grown;
        for r in [254, 253, 252, 251] {
            apply_borders(layer, r);
        }
        for r in (1..=250).rev() {
            if !apply_borders(layer, r) {
                break;
            }
        }
        layer.width = w;
        layer.height = h;
    }
}

fn rms_diff(a: u32, b: u32) -> u32 {
    a.abs_diff(b)
}

/// `rms_test(lm, w, h, rms)`: whether the layer survives a trip down to `w` x `h` (Lanczos) and
/// back up (the cubic filter, as the video board would) within `rms` per channel on its lit
/// texels, the hemisphere's channel within a third more.
#[must_use]
pub fn rms_test_size(lm: &LmLayer, w: u32, h: u32, rms: u32) -> bool {
    if w <= 1 || h <= 1 {
        return false;
    }
    let trip = |packed: &[u32]| -> Option<Vec<u32>> {
        let img = image_of(lm.width, lm.height, packed);
        let down = imf_resample(&img, w, h, ImfFilter::Lanczos3).ok()?;
        let up = imf_resample(&down, lm.width, lm.height, ImfFilter::Hermite).ok()?;
        Some(pixels_of(&up))
    };
    let base = lm.pack();
    let hemi = lm.pack_hemi();
    let (Some(base_r), Some(hemi_r)) = (trip(&base), trip(&hemi)) else { return false };
    let limit = 254 - BORDER;
    for (i, &m) in lm.marker.iter().enumerate().take(lm.width as usize * lm.height as usize) {
        if u32::from(m) < limit {
            continue;
        }
        let (Some(&p), Some(&pr), Some(&q), Some(&qr)) =
            (base.get(i), base_r.get(i), hemi.get(i), hemi_r.get(i))
        else {
            continue;
        };
        if rms_diff(color_get_r(pr), color_get_r(p)) > rms
            || rms_diff(color_get_g(pr), color_get_g(p)) > rms
            || rms_diff(color_get_b(pr), color_get_b(p)) > rms
            || rms_diff(color_get_a(pr), color_get_a(p)) > rms
            || rms_diff(color_get_r(qr), color_get_r(q)) > (rms * 4) / 3
        {
            return false;
        }
    }
    true
}

/// `rms_test(lm, r, g, b, s, h, rms)`: whether every lit texel is within `rms` of one colour.
#[must_use]
pub fn rms_test_flat(lm: &LmLayer, c: [u8; 5], rms: u32) -> bool {
    let [r, g, b, s, h] = c.map(u32::from);
    for (i, &m) in lm.marker.iter().enumerate() {
        if m < 254 {
            continue;
        }
        let [pr, pg, pb, ps, ph] = lm.pixel(i).map(u32::from);
        if rms_diff(r, pr) > rms
            || rms_diff(g, pg) > rms
            || rms_diff(b, pb) > rms
            || rms_diff(s, ps) > rms
            || rms_diff(h, ph) > (rms * 4) / 3
        {
            return false;
        }
    }
    true
}

/// `rms_average`: the sum of the lit texels and their count.
#[must_use]
pub fn rms_average(lm: &LmLayer) -> (ColorC, u32) {
    let mut c = ColorC::default();
    let mut count = 0;
    for (s, &m) in lm.surface.iter().zip(&lm.marker) {
        if m >= 254 {
            c.add(&s.get());
            count += 1;
        }
    }
    (c, count)
}

/// `compress_Zero`: a layer whose lit texels are all within `rms` of their mean becomes that
/// mean, 2 x 2 texels with the border, and its size 0.
pub fn compress_zero(lm: &mut LmLayer, rms: u32) -> bool {
    let (mut c, count) = rms_average(lm);
    if count == 0 {
        return false;
    }
    c.scale(count);
    let bytes = [u8_clr(c.rgb.x), u8_clr(c.rgb.y), u8_clr(c.rgb.z), u8_clr(c.sun), u8_clr(c.hemi)];
    if !rms_test_flat(lm, bytes, rms) {
        return false;
    }
    let n = (BORDER * 2 * BORDER * 2) as usize;
    lm.surface = vec![BaseColor::from_c(&c); n];
    lm.marker = vec![255; n];
    lm.width = 0;
    lm.height = 0;
    true
}

/// `compress_RMS`: the smallest of a half, three quarters or a quarter of each side that
/// survives [`rms_test_size`]; `None` when neither side can shrink.
#[must_use]
pub fn compress_rms(lm: &LmLayer, rms: u32) -> Option<(u32, u32)> {
    let (lw, lh) = (lm.width, lm.height);
    let mut w = 0;
    let mut h = 0;
    if lw >= 2 {
        w = lw / 2;
        if !rms_test_size(lm, w, lh, rms) {
            w = (lw * 3) / 4;
            if !rms_test_size(lm, w, lh, rms) {
                w = 0;
            }
        } else {
            let nw = lw / 4;
            if rms_test_size(lm, nw, lh, rms) {
                w = nw;
            }
        }
    }
    if lh >= 2 {
        h = lh / 2;
        if !rms_test_size(lm, lw, h, rms) {
            h = (lh * 3) / 4;
            if !rms_test_size(lm, lw, h, rms) {
                h = 0;
            }
        } else {
            let nh = lh / 4;
            if rms_test_size(lm, lw, nh, rms) {
                h = nh;
            }
        }
    }
    if w == 0 && h == 0 {
        return None;
    }
    Some((if w == 0 { lw } else { w }, if h == 0 { lh } else { h }))
}

/// The deflectors of a build (`g_deflectors`): an arena with stable ids and the list in the C++
/// order, as faces name their deflector.
#[derive(Clone, Debug, Default)]
pub struct Deflectors {
    arena: Vec<Option<Deflector>>,
    list: Vec<DeflId>,
}

impl Deflectors {
    /// A new deflector at the end of the list.
    pub fn create(&mut self) -> DeflId {
        let id = DeflId(self.arena.len() as u32);
        self.arena.push(Some(Deflector::default()));
        self.list.push(id);
        id
    }

    #[must_use]
    pub fn get(&self, id: DeflId) -> Option<&Deflector> {
        self.arena.get(id.0 as usize)?.as_ref()
    }

    pub fn get_mut(&mut self, id: DeflId) -> Option<&mut Deflector> {
        self.arena.get_mut(id.0 as usize)?.as_mut()
    }

    /// The deflectors in list order.
    #[must_use]
    pub fn list(&self) -> &[DeflId] {
        &self.list
    }

    /// The list, for a phase that reorders or erases from it.
    pub fn list_mut(&mut self) -> &mut Vec<DeflId> {
        &mut self.list
    }

    /// `xr_delete` of one: off the list and gone.
    pub fn destroy(&mut self, id: DeflId) {
        self.list.retain(|&d| d != id);
        if let Some(slot) = self.arena.get_mut(id.0 as usize) {
            *slot = None;
        }
    }

    /// `clear_and_free`: every deflector gone; the faces keep naming theirs, as the C++ faces keep
    /// their pointers.
    pub fn clear(&mut self) {
        self.arena.clear();
        self.list.clear();
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.list.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    /// `CBuild::LMapsLocal`: every deflector on the list lit, as many at once as the pool has
    /// workers (and each one's rows in parallel). The C++ shuffles the list first to spread the
    /// load over its six threads; a deflector's light depends on nothing but itself, so the
    /// order is the list's.
    pub fn light_all(&mut self, env: &LightEnv<'_>, ctl: &Ctl<'_>) -> Result<()> {
        let listed: std::collections::HashSet<DeflId> = self.list.iter().copied().collect();
        let total = self.list.len();
        let done = AtomicUsize::new(0);
        ctl.stage("Lighting...");
        self.arena.par_iter_mut().enumerate().for_each(|(i, d)| {
            let Some(d) = d else { return };
            if !listed.contains(&DeflId(i as u32)) || ctl.cancelled() {
                return;
            }
            d.light(env);
            let n = done.fetch_add(1, Ordering::Relaxed) + 1;
            ctl.progress(n, total);
        });
        ctl.check()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_jitter_tables_are_the_cpp_ones() {
        assert_eq!(jitter(1), &[Vec2::ZERO]);
        assert_eq!(jitter(4).len(), 4);
        assert_eq!(jitter(7).len(), 4);
        assert_eq!(jitter(9)[4], Vec2::ZERO);
    }

    #[test]
    fn a_flat_layer_compresses_to_nothing() {
        let mut lm = LmLayer::new(4, 4);
        let c = ColorC { rgb: Vec3::splat(0.5), hemi: 0.25, sun: 0.75, tmp: 0.0 };
        lm.surface.iter_mut().for_each(|s| *s = BaseColor::from_c(&c));
        lm.marker.iter_mut().for_each(|m| *m = 255);
        assert!(compress_zero(&mut lm, 4));
        assert_eq!((lm.width, lm.height, lm.surface.len()), (0, 0, 4));
        assert_eq!(lm.area(), 4);
    }

    #[test]
    fn a_smooth_gradient_shrinks_and_noise_does_not() {
        let mut lm = LmLayer::new(16, 16);
        for (i, s) in lm.surface.iter_mut().enumerate() {
            let x = (i % 16) as f32 / 16.0;
            *s = BaseColor::from_c(&ColorC { rgb: Vec3::splat(x * 0.2), hemi: 0.5, sun: 0.5, tmp: 0.0 });
        }
        lm.marker.iter_mut().for_each(|m| *m = 255);
        let shrunk = compress_rms(&lm, 6);
        assert!(shrunk.is_some_and(|(w, h)| w < 16 && h < 16), "{shrunk:?}");
        for (i, s) in lm.surface.iter_mut().enumerate() {
            let v = if (i * 7919) % 3 == 0 { 0.9 } else { 0.0 };
            *s = BaseColor::from_c(&ColorC { rgb: Vec3::splat(v), hemi: v, sun: v, tmp: 0.0 });
        }
        assert_eq!(compress_rms(&lm, 6), None);
    }

    #[test]
    fn remapping_puts_texel_centres_at_the_edges() {
        let mut d = Deflector::default();
        d.uv_polys
            .push(UvTri { uv: [Vec2::ZERO, Vec2::new(2.0, 0.0), Vec2::new(0.0, 4.0)], owner: FaceId(0) });
        let r = d.remap_uv_into(0, 0, 4, 8, 4, 8, false);
        assert_eq!(r[0].uv[0], Vec2::new(0.125, 0.0625));
        assert_eq!(r[0].uv[1], Vec2::new(0.875, 0.0625));
        let r = d.remap_uv_into(0, 0, 8, 4, 8, 4, true);
        // rotated: the source's y runs along the result's x
        assert_eq!(r[0].uv[2], Vec2::new(0.9375, 0.125));
    }
}
