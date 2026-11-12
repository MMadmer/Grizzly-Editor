//! The MU models - meshes placed many times by reference (`xrMU_Model*.cpp`,
//! `xrMU_Model_Reference*.cpp`, `mu_model_light*.cpp`, `fitter.cpp`): loading from `build.prj`,
//! the material subdivisions, the opacity of the faces, the collision of a reference, and the
//! light: a model's own (its hemisphere in its own shadow), each reference's in the level, and
//! the linear fit `c_scale` / `c_bias` that turns the first into the second.

use std::sync::atomic::{AtomicUsize, Ordering};

use glam::Vec3;
use gz_fmt_level::{Aabb, CollectorPacked};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::color::{BaseColor, ColorC};
use crate::data::{BuildMaterial, BuildParams, BuildTexture, QUALITY_DRAFT, ShaderXrlc, Shaders, shader_of};
use crate::light::LightSet;
use crate::math::{EPS, EPS_L, EPS_S, Matrix, XrRandom, deg2rad, exact_normalize, mad, point_words};
use crate::mesh::{Face, FaceId, Mesh, Smoothing, Tcf, VertId};
use crate::point::{Falloff, LP_DEFAULT, LP_DONT_HEMI, LP_DONT_RGB, LP_DONT_SUN, Tracer, light_point};
use crate::ray::{LightCache, Opacity, RapidCollector, RayFace, RayModel, RcFace, casts};
use crate::vertex::{TransGroups, mix_group};
use crate::{Ctl, LcError, Result};

/// `xrMU_Model::_subdiv`: a run of faces of one material; the rest is the geometry export's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Subdiv {
    pub material: u32,
    pub start: u32,
    pub count: u32,
    pub ogf: u32,
    pub vb_id: u32,
    pub vb_start: u32,
    pub ib_id: u32,
    pub ib_start: u32,
    pub sw_id: u32,
}

impl Subdiv {
    fn new(material: u32, start: u32) -> Self {
        Self {
            material,
            start,
            count: 1,
            ogf: u32::MAX,
            vb_id: u32::MAX,
            vb_start: 0,
            ib_id: u32::MAX,
            ib_start: 0,
            sw_id: u32::MAX,
        }
    }
}

/// `xrMU_Model`.
#[derive(Clone, Debug, Default)]
pub struct MuModel {
    pub name: SmolStr,
    pub lod_id: u16,
    pub mesh: Mesh,
    pub subdivs: Vec<Subdiv>,
    /// The model's own light, per vertex in list order.
    pub color: Vec<BaseColor>,
}

/// What a model's light reads of the build.
#[derive(Clone, Copy, Debug)]
pub struct MuEnv<'a> {
    pub shaders: &'a Shaders,
    pub materials: &'a [BuildMaterial],
    pub textures: &'a [BuildTexture],
    pub params: &'a BuildParams,
    /// `L_static`.
    pub lights: &'a LightSet,
    pub nosun: bool,
    pub linear: bool,
}

impl MuModel {
    /// `xrMU_Model::Load`: the vertices with no normal, the faces with their one set of
    /// coordinates, normal and smoothing group.
    pub fn load(src: &gz_fmt_level::MuModel) -> Result<Self> {
        let mut mesh = Mesh::new_model();
        let verts: Vec<VertId> = src.verts.iter().map(|&p| mesh.create_vertex(p)).collect();
        for (i, b) in src.faces.iter().enumerate() {
            if b.material_game >= 65536 {
                return Err(LcError::Invalid(format!(
                    "The model '{}' has a game material id past 65535.",
                    src.name
                )));
            }
            let corner = |k: usize| {
                verts.get(b.v[k] as usize).copied().ok_or_else(|| {
                    LcError::Invalid(format!(
                        "A face of the model '{}' names a vertex it does not have.",
                        src.name
                    ))
                })
            };
            let v = [corner(0)?, corner(1)?, corner(2)?];
            let f = mesh.create_face(v);
            if let Some(face) = mesh.face_mut(f) {
                face.material = b.material;
                face.material_game = b.material_game as u16;
                face.tc.push(Tcf { uv: b.t });
                face.sm_group = src.smooth_groups.get(i).copied().unwrap_or(0);
            }
            mesh.calc_normal(f);
        }
        Ok(Self { name: src.name.clone(), lod_id: src.lod_id, mesh, subdivs: Vec::new(), color: Vec::new() })
    }

    /// `calc_normals(xrMU_Model&)`.
    pub fn calc_normals(&mut self, sm_angle: f32, smoothing: Smoothing) {
        self.mesh.calc_normals(sm_angle, smoothing);
    }

    /// `calc_faceopacity`: `CacheOpacity` of every face.
    pub fn calc_faceopacity(&mut self, materials: &[BuildMaterial], textures: &[BuildTexture]) {
        let faces = self.mesh.faces().to_vec();
        for f in faces {
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.opaque = cache_opacity(face, materials, textures);
            }
        }
    }

    /// `calc_materials`: the faces sorted by material into runs, the runs of materials that are
    /// not rendered dropped. The C++ sort is not stable; here faces of one material keep their
    /// order.
    pub fn calc_materials(&mut self, shaders: &Shaders, materials: &[BuildMaterial]) {
        let mesh = &self.mesh;
        let mut faces = mesh.faces().to_vec();
        faces.sort_by_key(|&f| mesh.face(f).map_or(0, |x| x.material));
        let mat = |f: FaceId| mesh.face(f).map_or(0, |x| x.material);
        let mut subdivs = Vec::new();
        if let Some(&first) = faces.first() {
            let mut cur = Subdiv::new(u32::from(mat(first)), 0);
            for (i, &f) in faces.iter().enumerate().skip(1) {
                if cur.material != u32::from(mat(f)) {
                    subdivs.push(cur);
                    cur = Subdiv::new(u32::from(mat(f)), i as u32);
                } else {
                    cur.count += 1;
                }
            }
            subdivs.push(cur);
        }
        subdivs.retain(|s| {
            let first = faces.get(s.start as usize).copied().map_or(0, mat);
            shader_of(first, shaders, materials).flags & ShaderXrlc::RENDERING != 0
        });
        *self.mesh.faces_mut() = faces;
        self.subdivs = subdivs;
    }

    /// `export_cform_rcast`: every shadow-casting face once (a face equal to one already taken
    /// is left out), its corners through `xform`.
    pub fn export_cform_rcast(
        &mut self,
        cl: &mut RapidCollector,
        xform: &Matrix,
        shaders: &Shaders,
        materials: &[BuildMaterial],
    ) {
        let faces = self.mesh.faces().to_vec();
        for &f in &faces {
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.processed = false;
            }
        }
        let mut adjacent: Vec<FaceId> = Vec::with_capacity(36);
        for &f in &faces {
            let Some(face) = self.mesh.face(f) else { continue };
            if !casts(shader_of(face.material, shaders, materials)) {
                continue;
            }
            adjacent.clear();
            for v in face.v {
                if let Some(vx) = self.mesh.vertex(v) {
                    adjacent.extend_from_slice(&vx.adjacents);
                }
            }
            adjacent.sort_unstable();
            adjacent.dedup();
            let already = adjacent
                .iter()
                .any(|&t| t != f && self.mesh.face(t).is_some_and(|x| x.flags.processed && face.is_equal(x)));
            if already {
                continue;
            }
            let Some(p) = self.mesh.positions(f) else { continue };
            let p = p.map(|p| xform.transform_tiny(p));
            let texture =
                materials.get(usize::from(face.material)).map_or(u32::MAX, |m| u32::from(m.surface));
            let tc0 = face.tc0();
            let ray = RayFace { face: None, cast_shadow: true, opaque: face.flags.opaque, texture, uv: tc0 };
            let rc = RcFace {
                material: face.material,
                material_game: u32::from(face.material_game),
                t: tc0,
                reserved: 0,
            };
            cl.add(p, ray, rc, face.sm_group);
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.processed = true;
            }
        }
    }

    /// The model's own ray model: its shadow-casting faces as they stand (`calc_lighting()`'s
    /// collision form).
    pub fn own_ray_model(&mut self, shaders: &Shaders, materials: &[BuildMaterial]) -> Result<RayModel> {
        let mut bb = Aabb::INVALID;
        for &v in self.mesh.vertices() {
            if let Some(vx) = self.mesh.vertex(v) {
                bb.modify(vx.p);
            }
        }
        let mut cl = RapidCollector {
            collector: CollectorPacked::new(&bb, self.mesh.vertices().len(), self.mesh.faces().len()),
            faces: Vec::new(),
            rc_faces: Vec::new(),
        };
        self.export_cform_rcast(&mut cl, &Matrix::IDENTITY, shaders, materials);
        let verts = cl.collector.verts().to_vec();
        let tris: Vec<[u32; 3]> = cl.collector.tris().iter().map(|t| t.verts).collect();
        RayModel::new(&verts, &tris, cl.faces, Opacity::Squared, false)
    }

    /// `xrMU_Model::calc_lighting(dest, xform, model, lights, flags)`: every vertex lit at six
    /// points near it along normals jittered within 30 degrees (one at the draft quality),
    /// then the vertices of one point mixed by their translucency. Answers the light per vertex
    /// in list order. The C++ draws the jitter from one generator all threads share; here each
    /// vertex seeds its own from where it stands.
    #[must_use]
    pub fn calc_lighting_with(
        &self,
        env: &MuEnv<'_>,
        xform: &Matrix,
        model: &RayModel,
        flags: u32,
    ) -> Vec<BaseColor> {
        let rot = xform.without_translation().transposed().inverted_43();
        let samples = if env.params.quality == QUALITY_DRAFT { 1u32 } else { 6 };
        let tracer = Tracer { model, textures: env.textures, falloff: Falloff::Level { linear: env.linear } };
        let mesh = &self.mesh;
        let verts = mesh.vertices();

        let lit: Vec<BaseColor> = verts
            .par_iter()
            .with_min_len(16)
            .map_init(LightCache::default, |cache, &v| {
                let Some(vx) = mesh.vertex(v) else { return BaseColor::default() };
                let (mut amb, mut trans) = (0.0f32, 0.0f32);
                for f in &vx.adjacents {
                    if let Some(face) = mesh.face(*f) {
                        let s = shader_of(face.material, env.shaders, env.materials);
                        amb += s.vert_ambient;
                        trans += s.vert_translucency;
                    }
                }
                let n_adj = vx.adjacents.len() as f32;
                amb /= n_adj;
                trans /= n_adj;
                let vp = xform.transform_tiny(vx.p);
                let (vn, _) = exact_normalize(rot.transform_dir(vx.n));
                let mut rnd = XrRandom::from_words(&point_words(vp));
                let mut c = ColorC::default();
                for sample in 0..samples {
                    let a = 0.2 * sample as f32 / samples as f32;
                    let n = rnd.dir_cone(vn, deg2rad(30.0));
                    let p = mad(vp, n, a);
                    light_point(&tracer, cache, &mut c, p, n, env.lights, flags, None);
                }
                c.scale(samples);
                c.tmp = trans;
                if flags & LP_DONT_HEMI == 0 {
                    c.hemi += amb;
                }
                BaseColor::from_c(&c)
            })
            .collect();

        // the groups: a vertex joins every group its point is in, in list order
        let mut groups = TransGroups::default();
        for (i, &v) in verts.iter().enumerate() {
            if let Some(vx) = mesh.vertex(v) {
                groups.register(vx.p, i as u32, false);
            }
        }
        let mut colors = lit;
        for list in groups.in_order() {
            let mut list = list.to_vec();
            list.sort_unstable();
            list.dedup();
            let cs: Vec<ColorC> =
                list.iter().filter_map(|&i| colors.get(i as usize)).map(BaseColor::get).collect();
            for (&i, c) in list.iter().zip(mix_group(&cs, 0.5)) {
                if let Some(slot) = colors.get_mut(i as usize) {
                    *slot = BaseColor::from_c(&c);
                }
            }
        }
        colors
    }

    /// `xrMU_Model::calc_lighting()`: the model's own hemisphere in its own shadow, into
    /// `color`.
    pub fn calc_lighting(&mut self, env: &MuEnv<'_>) -> Result<()> {
        let model = self.own_ray_model(env.shaders, env.materials)?;
        self.color = self.calc_lighting_with(env, &Matrix::IDENTITY, &model, LP_DONT_RGB + LP_DONT_SUN);
        Ok(())
    }
}

/// `base_Face::CacheOpacity`: opaque unless its texture has alpha and its surface was loaded.
#[must_use]
pub fn cache_opacity(face: &Face, materials: &[BuildMaterial], textures: &[BuildTexture]) -> bool {
    let Some(m) = materials.get(usize::from(face.material)) else { return true };
    let Some(t) = textures.get(usize::from(m.surface)) else { return true };
    !(t.has_alpha && t.has_surface)
}

/// `xrMU_Reference`.
#[derive(Clone, Debug, PartialEq)]
pub struct MuReference {
    pub model: u32,
    pub xform: Matrix,
    pub flags: u32,
    pub sector: u16,
    /// The reference's light per vertex of its model.
    pub color: Vec<BaseColor>,
    /// The fit `model light * c_scale + c_bias = reference light`, per channel.
    pub c_scale: ColorC,
    pub c_bias: ColorC,
}

impl MuReference {
    /// `xrMU_Reference::Load`.
    #[must_use]
    pub fn load(src: &gz_fmt_level::MuReference) -> Self {
        Self {
            model: src.model,
            xform: Matrix::from_rows(src.transform),
            flags: src.flags,
            sector: src.sector,
            color: Vec::new(),
            c_scale: ColorC { rgb: Vec3::ONE, hemi: 1.0, sun: 1.0, tmp: 0.0 },
            c_bias: ColorC::default(),
        }
    }

    /// `export_cform_game`: the model's collision faces through the transform, with their game
    /// material and the reference's sector. The C++ takes them in the order of their addresses;
    /// here in the model's.
    pub fn export_cform_game(
        &self,
        model: &MuModel,
        cl: &mut CollectorPacked,
        shaders: &Shaders,
        materials: &[BuildMaterial],
    ) {
        for &f in model.mesh.faces() {
            let Some(face) = model.mesh.face(f) else { continue };
            if shader_of(face.material, shaders, materials).flags & ShaderXrlc::COLLISION == 0 {
                continue;
            }
            let Some(p) = model.mesh.positions(f) else { continue };
            let p = p.map(|p| self.xform.transform_tiny(p));
            cl.add_face(p[0], p[1], p[2], face.material_game, self.sector, face.sm_group);
        }
    }

    /// `xrMU_Reference::calc_lighting`: the reference lit in the level, then the fit.
    pub fn calc_lighting(&mut self, model: &MuModel, env: &MuEnv<'_>, level: &RayModel) {
        let flags = if env.nosun { LP_DONT_SUN } else { 0 } | LP_DEFAULT;
        self.color = model.calc_lighting_with(env, &self.xform, level, flags);
        let n = self.color.len().min(model.color.len());
        let a: Vec<f64> = model.color.iter().take(n).map(|c| f64::from(c.get().hemi)).collect();
        for i in 0..5 {
            let b: Vec<f64> = self.color.iter().take(n).map(|c| f64::from(c.get().channel(i))).collect();
            let (s, bias) = linear_regression(&a, &b);
            self.c_scale.set_channel(i, s);
            self.c_bias.set_channel(i, bias);
        }
        for i in 0..5 {
            // oracle: the C++ fits against the model's channel 4 (the sun, 0 in the model's own
            // light) here, after the regression above took the hemisphere
            let a: Vec<f64> = model.color.iter().take(n).map(|c| f64::from(c.get().channel(4))).collect();
            let b: Vec<f64> = self.color.iter().take(n).map(|c| f64::from(c.get().channel(i))).collect();
            let (mut s, mut bias) = (self.c_scale.channel(i), self.c_bias.channel(i));
            simple_optimize(&a, &b, &mut s, &mut bias);
            self.c_scale.set_channel(i, s);
            self.c_bias.set_channel(i, bias);
        }
    }
}

/// `vfComputeLinearRegression`: `C` and `D` of `A * C + D = B` by least squares, in double,
/// `C` rounded to float before `D` is taken from it.
#[must_use]
pub fn linear_regression(a: &[f64], b: &[f64]) -> (f32, f32) {
    let n = a.len() as f64;
    let sx: f64 = a.iter().fold(0.0, |s, x| s + x);
    let sy: f64 = b.iter().fold(0.0, |s, x| s + x);
    let sxy: f64 = a.iter().zip(b).fold(0.0, |s, (x, y)| s + x * y);
    let sx2: f64 = a.iter().fold(0.0, |s, x| s + x * x);
    let den = n * sx2 - sx * sx;
    let c = if den.abs() > f64::from(EPS_S) { ((n * sxy - sx * sy) / den) as f32 } else { 0.0 };
    let d = if a.is_empty() { 0.0 } else { ((sy - f64::from(c) * sx) / n) as f32 };
    (c, d)
}

/// `simple_optimize`: a damped iteration of the fit from `scale` and `bias`, up to 128 rounds.
/// Answers the error.
pub fn simple_optimize(a: &[f64], b: &[f64], out_scale: &mut f32, out_bias: &mut f32) -> f64 {
    let mut scale = f64::from(*out_scale);
    let mut bias = f64::from(*out_bias);
    let mut error = f64::from(f32::MAX);
    let elements = a.len() as f64;
    let mut count = 0u32;
    loop {
        count += 1;
        if count > 128 {
            *out_scale = scale as f32;
            *out_bias = bias as f32;
            return error;
        }
        let (old_scale, old_bias) = (scale, bias);
        let mut accum = 0.0;
        let mut ok = 0u32;
        for (x, y) in a.iter().zip(b) {
            if x.abs() > f64::from(EPS_L) {
                accum += (y - bias) / x;
                ok += 1;
            }
        }
        let s = if ok > 0 { accum / f64::from(ok) } else { scale };
        let mut bb = bias;
        if scale.abs() > f64::from(EPS) {
            let accum: f64 = a.iter().zip(b).fold(0.0, |acc, (x, y)| acc + (y - x / scale));
            bb = accum / elements;
        }
        let conv = 7.0;
        scale = ((conv - 1.0) * scale + s) / conv;
        bias = ((conv - 1.0) * bias + bb) / conv;
        let accum: f64 = a.iter().zip(b).fold(0.0, |acc, (x, y)| acc + (y - (x * scale + bias)));
        let err = accum / elements;
        if err < error {
            error = err;
            if error < f64::from(EPS) {
                *out_scale = scale as f32;
                *out_bias = bias as f32;
                return error;
            }
        } else {
            *out_scale = old_scale as f32;
            *out_bias = old_bias as f32;
            return error;
        }
    }
}

/// `vfOptimizeParameters`: a gradient descent of `A * C + D = B` over several parameters
/// (`A[test][param]`), from `C = 1`, `D = 0`. Nothing of the compiler calls it; it is kept with
/// the fitter.
#[allow(clippy::too_many_arguments)]
pub fn optimize_parameters(
    a: &[Vec<f32>],
    b: &[Vec<f32>],
    epsilon: f32,
    alpha: f32,
    beta: f32,
    norma_factor: f32,
    max_iterations: u32,
) -> (Vec<f32>, Vec<f32>) {
    let tests = b.len();
    let Some(params) = b.first().map(Vec::len) else { return (Vec::new(), Vec::new()) };
    let mut c = vec![1.0f32; params];
    let mut d = vec![0.0f32; params];
    let mut delta = vec![0.0f32; params];
    let at = |m: &[Vec<f32>], i: usize, j: usize| m.get(i).and_then(|r| r.get(j)).copied().unwrap_or(0.0);
    let mut eval = vec![vec![0.0f32; params]; tests];
    let functional = |eval: &mut Vec<Vec<f32>>, c: &[f32], d: &[f32]| {
        let mut r = 0.0f32;
        for (i, row) in eval.iter_mut().enumerate() {
            for (j, e) in row.iter_mut().enumerate() {
                *e = at(a, i, j) * c.get(j).copied().unwrap_or(0.0) + d.get(j).copied().unwrap_or(0.0);
                let t = at(b, i, j) - *e;
                r += t * t;
            }
        }
        r
    };
    let mut f = functional(&mut eval, &c, &d);
    let mut iteration = 0u32;
    loop {
        let previous = f;
        let mut grad = vec![0.0f32; params];
        let mut norma = 0.0f32;
        for (j, g) in grad.iter_mut().enumerate() {
            for (i, row) in eval.iter().enumerate() {
                *g -= at(b, i, j) - row.get(j).copied().unwrap_or(0.0);
            }
            norma += *g * *g;
        }
        let norma = norma.sqrt();
        for g in &mut grad {
            *g /= (if norma > 1.0 { norma } else { 1.0 }) * norma_factor;
        }
        for ((dl, g), (cj, dj)) in delta.iter_mut().zip(&grad).zip(c.iter_mut().zip(d.iter_mut())) {
            *dl = *g * -alpha + *dl * beta;
            *cj += *dl;
            *dj += *dl;
        }
        f = functional(&mut eval, &c, &d);
        iteration += 1;
        if !((previous - f) / tests as f32 > epsilon && iteration <= max_iterations) {
            if previous < f {
                for ((dl, cj), dj) in delta.iter().zip(c.iter_mut()).zip(d.iter_mut()) {
                    *cj -= *dl;
                    *dj -= *dl;
                }
            }
            break;
        }
    }
    (c, d)
}

/// `CMUThread` and `CMULight` (`run_mu_light`, `wait_mu_*`): every model's materials and own
/// light, then every reference's light, each set on as many workers as the pool has.
pub fn light_mu(
    models: &mut [MuModel],
    refs: &mut [MuReference],
    env: &MuEnv<'_>,
    level: &RayModel,
    ctl: &Ctl<'_>,
) -> Result<()> {
    ctl.stage("MU : Models...");
    let done = AtomicUsize::new(0);
    let total = models.len() + refs.len();
    let results: Vec<Result<()>> = models
        .par_iter_mut()
        .map(|m| {
            ctl.check()?;
            m.calc_materials(env.shaders, env.materials);
            m.calc_lighting(env)?;
            ctl.progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
            Ok(())
        })
        .collect();
    results.into_iter().collect::<Result<()>>()?;
    ctl.stage("MU : References...");
    let models = &*models;
    refs.par_iter_mut().for_each(|r| {
        if ctl.cancelled() {
            return;
        }
        if let Some(m) = models.get(r.model as usize) {
            r.calc_lighting(m, env, level);
        }
        ctl.progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
    });
    ctl.check()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_regression_finds_a_line_and_the_fitter_moves_the_bias() {
        let a = [0.0, 1.0, 2.0, 3.0];
        let b = [1.0, 3.0, 5.0, 7.0];
        assert_eq!(linear_regression(&a, &b), (2.0, 1.0));
        assert_eq!(linear_regression(&[1.0, 1.0], &[2.0, 4.0]), (0.0, 3.0));
        // against a channel that is 0 the scale cannot move and the bias goes towards the mean
        let (mut s, mut bias) = (2.0f32, 1.0f32);
        simple_optimize(&[0.0; 4], &b, &mut s, &mut bias);
        assert_eq!(s, 2.0);
        assert!(bias > 1.0 && bias <= 4.0, "{bias}");
    }

    #[test]
    fn the_gradient_fitter_lowers_the_error() {
        let a: Vec<Vec<f32>> = (0..8).map(|i| vec![i as f32]).collect();
        let b: Vec<Vec<f32>> = (0..8).map(|i| vec![i as f32 + 1.0]).collect();
        let err =
            |c: f32, d: f32| -> f32 { (0..8).map(|i| (c * i as f32 + d - (i as f32 + 1.0)).powi(2)).sum() };
        let (c, d) = optimize_parameters(&a, &b, 1e-6, 1.0, 0.01, 10.0, 10000);
        // the C++ moves both parameters by one step, so it lands on the best line of slope - bias = 1
        assert!(err(c[0], d[0]) < err(1.0, 0.0), "{c:?} {d:?}");
        assert!((c[0] - d[0] - 1.0).abs() < 1e-4);
    }
}
