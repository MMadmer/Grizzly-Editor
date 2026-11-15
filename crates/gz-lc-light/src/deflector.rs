//! A group of faces mapped onto one lightmap rectangle, and its light:
//! the projection that
//! gives the faces their texture coordinates, the texels lit through the faces
//! (with the jitter of the lightmap samples, and the edge texels), the borders, the shrink to
//! nothing or to a smaller size when the light is flat enough (the RMS thresholds),
//! and the final border.

use glam::{Vec2, Vec3};
use gz_fmt_tex::{ImfFilter, imf_resample};
use rayon::prelude::*;

use crate::bake::{LightJob, LitScene, Sample};
use crate::batch::{self, BATCH, Gathered, Run, light_samples};
use crate::color::{BaseColor, ColorC};
use crate::data::{BuildMaterial, BuildParams, BuildTexture, Shaders, shader_of};
use crate::hash2d::Hash2D;
use crate::layer::{BORDER, LmLayer, apply_borders, image_of, lblit, pixels_of};
use crate::light::LightSet;
use crate::math::{
    Aabb, Box2, EPS, EPS_S, Matrix, color_get_a, color_get_b, color_get_g, color_get_r, color_rgba, cos_deg,
    exact_normalize, from_bary, i_ceil, i_floor, mad, magnitude, normalize, normalize_safe, sphere_of,
    u8_clr,
};
use crate::mesh::{DeflId, FaceId, Mesh, Tcf};
use crate::point::{Falloff, LP_DEFAULT, LP_DONT_SUN, LP_USE_FACE_DISABLE};
use crate::ray::RayModel;
use crate::{Ctl, Result};

/// `c_LMAP_size`: the side of a lightmap page.
pub const LMAP_SIZE: u32 = 1024;
/// The largest side a deflector is lit at: a page's half less its border.
pub const MAX_LAYER: u32 = 512 - 2 * BORDER;
/// The grid a deflector's triangles are found in.
const HASH_SIDE: usize = 128;

/// The sample offsets of a texel for the build's jitter sample count (1, 9, anything else
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

/// A face's corners in the deflector's texture space.
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
    pub lights: &'a LightSet,
    pub params: &'a BuildParams,
    /// `-nosun`.
    pub nosun: bool,
    /// `-att`: point lights fall off linearly.
    pub linear: bool,
    /// A lightmap whose light allows it shrinks or collapses; off, every chart keeps the size its
    /// geometry gives it (a measure of the structure without the light).
    pub shrink: bool,
}

impl<'a> LightEnv<'a> {
    /// What the level's samples are lit with: the whole light set (a point light out of a
    /// sample's reach gives it nothing), the level's falloff.
    #[must_use]
    pub fn job(&self) -> LightJob<'a> {
        LightJob { lights: self.lights, falloff: Falloff::Level { linear: self.linear } }
    }

    #[must_use]
    pub fn sun_flag(&self) -> u32 {
        if self.nosun { LP_DONT_SUN } else { 0 }
    }
}

/// A deflector.
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
    /// A new deflector.
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

    /// Projects the face from `start`: the face and, through the faces of its corners, every
    /// face reachable that has no deflector and fits this one, in the order of X-Ray's
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

    /// The deflector's normal from its faces' (weighted by area), the faces
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

    #[must_use]
    pub fn bounds_summary(&self) -> Box2 {
        let mut b = Box2::INVALID;
        for t in &self.uv_polys {
            b.merge(&Box2::of_tri(&t.uv));
        }
        b
    }

    /// The material of the first face.
    #[must_use]
    pub fn base_material(&self, mesh: &Mesh) -> u16 {
        self.uv_polys.first().and_then(|t| mesh.face(t.owner)).map_or(0, |f| f.material)
    }

    #[must_use]
    pub fn face_count(&self) -> usize {
        self.uv_polys.len()
    }

    /// The triangles mapped into the rectangle `base .. base + size` of a
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

    /// The deflector's own triangles remapped.
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

    /// The bounds of the faces as they are now.
    pub(crate) fn take_sphere(&mut self, mesh: &Mesh) {
        let mut bb = Aabb::INVALID;
        for t in &self.uv_polys {
            if let Some(p) = mesh.positions(t.owner) {
                p.iter().for_each(|p| bb.modify(*p));
            }
        }
        (self.sphere_p, self.sphere_r) = sphere_of(&bb);
    }

    /// The triangles mapped onto the layer's texels at its size and every texel's samples
    /// gathered: its jittered points through the first triangle that holds each, then - in X-Ray's
    /// order, since one edge may take a texel another would - the texels the triangles' edges
    /// cross but no point found, one sample each at the edge. The markers are set here: where a
    /// texel's light comes from is geometry's alone. A run's item is the texel's index.
    pub(crate) fn gather(&mut self, mesh: &Mesh, params: &BuildParams, sun_flag: u32) -> Gathered {
        let (w, h) = (self.layer.width, self.layer.height);
        self.remap_uv(0, 0, w, h, w, h, false);
        let boxes: Vec<Box2> = self.uv_polys.iter().map(|t| Box2::of_tri(&t.uv)).collect();
        let hash = Hash2D::build(HASH_SIDE, HASH_SIDE, self.bounds_summary(), &boxes);
        self.layer.create(w.min(LMAP_SIZE - 2 * BORDER), h.min(LMAP_SIZE - 2 * BORDER));
        let mut g = Gathered::default();
        let (w, h) = (self.layer.width as usize, self.layer.height as usize);
        if w == 0 || h == 0 {
            return g;
        }
        let tris = self.world(mesh);
        let flags = sun_flag | LP_USE_FACE_DISABLE;
        let dim = Vec2::new(w as f32, h as f32);
        let half = Vec2::new(0.5 / dim.x, 0.5 / dim.y);
        let js = Vec2::new(0.4999 / dim.x, 0.4999 / dim.y);
        let jit = jitter(params.lm_jitter_samples);
        g.samples.reserve(w * h * jit.len());
        for v in 0..h {
            for u in 0..w {
                let first = g.begin();
                for j in jit {
                    let p = Vec2::new(
                        u as f32 / dim.x + half.x + j.x * js.x,
                        v as f32 / dim.y + half.y + j.y * js.y,
                    );
                    for &i in hash.query(p.x, p.y) {
                        let Some(t) = self.uv_polys.get(i as usize) else { continue };
                        let Some(b) = (Tcf { uv: t.uv }).inside(p) else { continue };
                        let Some(Some(world)) = tris.get(i as usize) else { break };
                        let wp = from_bary(world.p[0], world.p[1], world.p[2], b);
                        let (wn, _) = exact_normalize(from_bary(world.n[0], world.n[1], world.n[2], b));
                        let (wn, _) = exact_normalize(wn + world.face_n);
                        g.samples.push(Sample::new(wp, wn, flags, Some(world.face)));
                        break;
                    }
                }
                let id = v * w + u;
                let lit = g.samples.len() as u32 > first;
                if lit {
                    g.end(id as u32, first);
                }
                if let Some(m) = self.layer.marker.get_mut(id) {
                    *m = if lit { 255 } else { 0 };
                }
            }
        }

        // the edges
        let texel = (1.0 / w.max(h) as f32) / 8.0;
        let edge_flags = sun_flag | LP_DEFAULT;
        for (t, world) in self.uv_polys.iter().zip(&tris) {
            let Some(world) = world else { continue };
            for e in 0..3 {
                let (a, b) = (e, (e + 1) % 3);
                edge_samples(
                    &mut self.layer,
                    &mut g,
                    (t.uv[a], t.uv[b]),
                    (world.p[a], world.p[b]),
                    world,
                    texel,
                    edge_flags,
                );
            }
        }
        g
    }

    /// The lights of [`Self::gather`]'s samples into the texels: a texel's samples averaged and
    /// halved.
    pub(crate) fn scatter(&mut self, lit: &[ColorC], runs: &[Run]) {
        for r in runs {
            let mut c = batch::sum(lit, r);
            c.scale(r.count);
            c.mul(0.5);
            if let Some(s) = self.layer.surface.get_mut(r.item as usize) {
                *s = BaseColor::from_c(&c);
            }
        }
    }

    /// The lit layer's borders grown, the layer shrunk when the light allows - to nothing, or to a
    /// smaller size resampled from what was lit (never lit again) - and grown by the final border.
    fn finish(&mut self, mesh: &Mesh, params: &BuildParams, sun_flag: u32, shrink: bool) {
        for r in (1..=254).rev() {
            if !apply_borders(&mut self.layer, r) {
                break;
            }
        }
        if shrink {
            if compress_zero(&mut self.layer, rms_zero(params)) {
                return;
            }
            if let Some((w, h)) = compress_rms(&self.layer, rms_shrink(params))
                && let Some(small) = resample(&self.layer, w, h)
            {
                // the markers of the smaller layer are the geometry's, its texels the resampled ones
                self.layer.create(w, h);
                let _ = self.gather(mesh, params, sun_flag);
                if small.len() == self.layer.surface.len() {
                    self.layer.surface = small;
                }
            }
        }
        expand(&mut self.layer);
    }
}

/// The texels an edge crosses that are still unlit, each given one sample at the edge and its
/// marker: an edge after it finds the texel taken.
#[allow(clippy::too_many_arguments)]
fn edge_samples(
    lm: &mut LmLayer,
    g: &mut Gathered,
    (p1, p2): (Vec2, Vec2),
    (v1, v2): (Vec3, Vec3),
    world: &TriWorld,
    texel: f32,
    flags: u32,
) {
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
        let Some(m) = lm.marker.get_mut(id) else { continue };
        if *m != 0 {
            continue;
        }
        *m = 255;
        let first = g.begin();
        g.samples.push(Sample::new(mad(v1, vdir, time), world.face_n, flags, Some(world.face)));
        g.end(id as u32, first);
    }
}

/// The layer resampled to `w` x `h` with the filter [`rms_test_size`] judged it by: the static
/// light and the sun, the sun and the hemisphere, as two pictures. `None` when a side is too small
/// for the filter.
fn resample(lm: &LmLayer, w: u32, h: u32) -> Option<Vec<BaseColor>> {
    let n = lm.width as usize * lm.height as usize;
    let mut a = Vec::with_capacity(n);
    let mut b = Vec::with_capacity(n);
    for s in lm.surface.iter().take(n) {
        let c = s.get();
        a.push(color_rgba(u8_clr(c.rgb.x), u8_clr(c.rgb.y), u8_clr(c.rgb.z), u8_clr(c.sun)));
        b.push(color_rgba(u8_clr(c.sun), u8_clr(c.hemi), 0, 255));
    }
    let down = |p: &[u32]| imf_resample(&image_of(lm.width, lm.height, p), w, h, ImfFilter::Lanczos3).ok();
    let (a, b) = (down(&a)?, down(&b)?);
    // a byte back into the fixed point with half a step more: the page packs it to the same byte
    let f = |v: u8| (f32::from(v) + 0.5) / 255.0;
    Some(
        a.rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(b.rgba.as_chunks::<4>().0)
            .map(|(a, b)| {
                BaseColor::from_c(&ColorC {
                    rgb: Vec3::new(f(a[0]), f(a[1]), f(a[2])),
                    hemi: f(b[1]),
                    sun: f(b[0]),
                    tmp: 0.0,
                })
            })
            .collect(),
    )
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

/// The deflectors of a build: an arena with stable ids and the list in X-Ray's
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

    /// Clears every deflector: the faces keep naming theirs, as X-Ray's faces keep
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

    /// Every deflector on the list lit through `scene`: the samples of as many deflectors as
    /// make a batch gathered at once (each deflector's on a worker), lit in one call, scattered
    /// back, then each layer finished. A deflector's light depends on nothing but itself, so the
    /// order is the list's, and while one batch is lit the next is gathered and the last
    /// scattered beside it: the batches and their samples stay the same, the CPU's work no
    /// longer waits for the lighter's.
    pub fn light_all(&mut self, env: &LightEnv<'_>, scene: &dyn LitScene, ctl: &Ctl<'_>) -> Result<()> {
        let ids: Vec<DeflId> = self.list.iter().copied().filter(|&d| self.get(d).is_some()).collect();
        let total = ids.len();
        let jit = jitter(env.params.lm_jitter_samples).len();
        let job = env.job();
        let sun = env.sun_flag();
        // the batches: whole deflectors, by their texels' samples (a batch changes only its own)
        let mut ranges = Vec::new();
        let mut at = 0usize;
        while at < ids.len() {
            let (mut end, mut n) = (at, 0usize);
            while let Some(&d) = ids.get(end) {
                let est = self.get(d).map_or(0, |x| x.layer.width as usize * x.layer.height as usize * jit);
                if end > at && n + est > BATCH {
                    break;
                }
                n += est;
                end += 1;
            }
            ranges.push(at..end);
            at = end;
        }
        let gather = |mut chunk: Vec<(DeflId, Deflector)>| -> ChartBatch {
            let gathered: Vec<Gathered> = chunk
                .par_iter_mut()
                .map(|(_, d)| {
                    if ctl.cancelled() {
                        return Gathered::default();
                    }
                    d.take_sphere(env.mesh);
                    d.gather(env.mesh, env.params, sun)
                })
                .collect();
            let mut bases = Vec::with_capacity(gathered.len());
            let mut samples = Vec::with_capacity(gathered.iter().map(|g| g.samples.len()).sum());
            for g in &gathered {
                bases.push(samples.len());
                samples.extend_from_slice(&g.samples);
            }
            ChartBatch { chunk, gathered, bases, samples }
        };
        let finish = |b: ChartBatch, lit: Vec<ColorC>| -> Vec<(DeflId, Deflector)> {
            let ChartBatch { mut chunk, gathered, bases, .. } = b;
            chunk.par_iter_mut().zip(gathered.par_iter().zip(&bases)).for_each(|((_, d), (g, &base))| {
                let mine = lit.get(base..base + g.samples.len()).unwrap_or_default();
                d.scatter(mine, &g.runs);
                d.finish(env.mesh, env.params, sun, env.shrink);
            });
            chunk
        };
        let mut next = ranges.first().map(|r| gather(self.take_range(&ids, r)));
        let mut done: Option<(ChartBatch, Vec<ColorC>)> = None;
        let mut outcome = Ok(());
        for k in 0..ranges.len() {
            let Some(cur) = next.take() else { break };
            if let Err(e) = ctl.check() {
                self.put_back(cur.chunk);
                outcome = Err(e);
                break;
            }
            let upcoming = ranges.get(k + 1).map(|r| self.take_range(&ids, r));
            let last = done.take();
            let (lit, (gathered, finished)) = rayon::join(
                || light_samples(scene, &job, &cur.samples, ctl),
                || (upcoming.map(gather), last.map(|(b, lit)| finish(b, lit))),
            );
            if let Some(chunk) = finished {
                self.put_back(chunk);
                ctl.progress(ranges[k].start, total);
            }
            next = gathered;
            match lit {
                Ok(lit) => done = Some((cur, lit)),
                Err(e) => {
                    self.put_back(cur.chunk);
                    outcome = Err(e);
                    break;
                }
            }
        }
        if let Some(b) = next.take() {
            self.put_back(b.chunk);
        }
        if let Some((b, lit)) = done.take() {
            let chunk = finish(b, lit);
            self.put_back(chunk);
        }
        outcome?;
        ctl.progress(total, total);
        ctl.check()
    }

    /// The deflectors of a batch, taken out of the arena while they are worked on.
    fn take_range(&mut self, ids: &[DeflId], r: &std::ops::Range<usize>) -> Vec<(DeflId, Deflector)> {
        ids.get(r.clone())
            .unwrap_or_default()
            .iter()
            .filter_map(|&d| self.arena.get_mut(d.0 as usize).and_then(Option::take).map(|x| (d, x)))
            .collect()
    }

    fn put_back(&mut self, chunk: Vec<(DeflId, Deflector)>) {
        for (id, d) in chunk {
            if let Some(slot) = self.arena.get_mut(id.0 as usize) {
                *slot = Some(d);
            }
        }
    }
}

/// A batch of deflectors with their gathered samples.
struct ChartBatch {
    chunk: Vec<(DeflId, Deflector)>,
    gathered: Vec<Gathered>,
    bases: Vec<usize>,
    samples: Vec<Sample>,
}

#[cfg(test)]
mod tests {
    use super::*;

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
