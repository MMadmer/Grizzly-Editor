//! `xrLC_GlobalData`: everything the level compiler holds while it builds, as a value the
//! caller owns (the C++ `lc_global_data()` and `inlc_global_data()`), and the calls `XrLC` makes
//! into `XrLCLight` as its methods.

use glam::Vec3;
use gz_fmt_level::{Aabb, BuildProject};

use crate::data::{BuildMaterial, BuildParams, BuildTexture, ShaderXrlc, Shaders, shader_of};
use crate::deflector::{Deflectors, LightEnv};
use crate::implicit::{ImplicitInput, ImplicitMap, has_implicit_lighting, implicit_lighting};
use crate::light::LightSet;
use crate::lightmap::{Lightmap, LightmapPage, merge_lightmaps};
use crate::mesh::{FaceId, Mesh, Smoothing};
use crate::mu::{MuEnv, MuModel, MuReference, cache_opacity, light_mu};
use crate::ray::{RapidCollector, RayFace, RayModel, RcFace, RcastForm, casts};
use crate::vertex::{apply_vertex_light, light_vertices};
use crate::{Ctl, LcError, Result};

/// `xrLC_GlobalData`.
#[derive(Clone, Debug)]
pub struct LcGlobalData {
    /// `g_params`.
    pub params: BuildParams,
    /// The compiler shader library.
    pub shaders: Shaders,
    pub materials: Vec<BuildMaterial>,
    pub textures: Vec<BuildTexture>,
    /// `L_static`.
    pub lights: LightSet,
    /// `RCAST_Model`.
    pub ray: Option<RayModel>,
    /// The pages made by the merge.
    pub lightmaps: Vec<Lightmap>,
    pub mu_models: Vec<MuModel>,
    pub mu_refs: Vec<MuReference>,
    /// `g_vertices`, `g_faces` and the error records.
    pub mesh: Mesh,
    /// `g_deflectors`.
    pub deflectors: Deflectors,
    /// `-nosun`.
    pub nosun: bool,
    /// `-att` (`gl_linear`).
    pub linear: bool,
}

/// What `CBuild::Light` makes besides what it changes in the data.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct LightOutput {
    /// The implicitly lighted textures.
    pub implicit: Vec<ImplicitMap>,
    /// The lightmap pages, in the order of `lightmaps`.
    pub pages: Vec<LightmapPage>,
}

impl LcGlobalData {
    #[must_use]
    pub fn new(
        params: BuildParams,
        shaders: Shaders,
        materials: Vec<BuildMaterial>,
        textures: Vec<BuildTexture>,
        lights: LightSet,
    ) -> Self {
        Self {
            params,
            shaders,
            materials,
            textures,
            lights,
            ray: None,
            lightmaps: Vec::new(),
            mu_models: Vec::new(),
            mu_refs: Vec::new(),
            mesh: Mesh::new(),
            deflectors: Deflectors::default(),
            nosun: false,
            linear: false,
        }
    }

    /// `CBuild::Load`'s MU part: the models and the references of `build.prj`.
    pub fn load_mu(&mut self, prj: &BuildProject) -> Result<()> {
        self.mu_models = prj.mu_models.iter().map(MuModel::load).collect::<Result<_>>()?;
        // the models share one vertex and one face pool, filled model after model
        let (mut vb, mut fb) = (0u64, 0u64);
        for m in &mut self.mu_models {
            m.mesh.rebase_pools(vb, fb);
            (vb, fb) = m.mesh.pool_ends();
        }
        self.mu_refs = prj.mu_refs.iter().map(MuReference::load).collect();
        if let Some(r) = self.mu_refs.iter().find(|r| r.model as usize >= self.mu_models.len()) {
            return Err(LcError::Invalid(format!(
                "A reference names the MU model {}, the build has {}.",
                r.model,
                self.mu_models.len()
            )));
        }
        Ok(())
    }

    /// `base_Face::Shader`.
    #[must_use]
    pub fn shader(&self, material: u16) -> &ShaderXrlc {
        shader_of(material, &self.shaders, &self.materials)
    }

    /// `DataFace::hasImplicitLighting`.
    #[must_use]
    pub fn has_implicit_lighting(&self, f: FaceId) -> bool {
        self.mesh
            .face(f)
            .is_some_and(|face| has_implicit_lighting(face, &self.shaders, &self.materials, &self.textures))
    }

    /// `CBuild::Light_prepare`: `CacheOpacity` of every face and every model's.
    pub fn light_prepare(&mut self) {
        let faces = self.mesh.faces().to_vec();
        for f in faces {
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.opaque = cache_opacity(face, &self.materials, &self.textures);
            }
        }
        for m in &mut self.mu_models {
            m.calc_faceopacity(&self.materials, &self.textures);
        }
    }

    /// `CBuild::CalcNormals` (`calculate_normals<Vertex>`).
    pub fn calc_normals(&mut self, smoothing: Smoothing) -> isize {
        self.mesh.calc_normals(self.params.sm_angle, smoothing)
    }

    /// `CBuild::MU_ModelsCalculateNormals`.
    pub fn mu_calc_normals(&mut self, smoothing: Smoothing) {
        let angle = self.params.sm_angle;
        // one after another through the pools they share, as the C++ does
        let mut vpool = self.mu_models.last().map(|m| m.mesh.vpool.clone()).unwrap_or_default();
        let mut fpool = self.mu_models.last().map(|m| m.mesh.fpool.clone()).unwrap_or_default();
        for m in &mut self.mu_models {
            m.mesh.vpool = std::mem::take(&mut vpool);
            m.mesh.fpool = std::mem::take(&mut fpool);
            m.calc_normals(angle, smoothing);
            vpool = std::mem::take(&mut m.mesh.vpool);
            fpool = std::mem::take(&mut m.mesh.fpool);
        }
        if let Some(m) = self.mu_models.last_mut() {
            m.mesh.vpool = vpool;
            m.mesh.fpool = fpool;
        }
    }

    /// `CBuild::BuildRapid`: the shadow-casting faces of the level (one of each set of equal
    /// faces) and of every reference into the ray model, which replaces the one before; answers
    /// the soup as `build.cform` holds it. `bounds` is the box of the build's vertices.
    pub fn build_rapid(&mut self, bounds: &Aabb) -> Result<RcastForm> {
        self.ray = None;
        let faces = self.mesh.faces().to_vec();
        for &f in &faces {
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.processed = false;
            }
        }
        let mut cl = RapidCollector::new(bounds, self.mesh.vertices().len(), faces.len());
        let mut adjacent: Vec<FaceId> = Vec::with_capacity(36);
        for &f in &faces {
            let Some(face) = self.mesh.face(f) else { continue };
            if !casts(shader_of(face.material, &self.shaders, &self.materials)) {
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
            let tc0 = face.tc0();
            let texture =
                self.materials.get(usize::from(face.material)).map_or(u32::MAX, |m| u32::from(m.surface));
            let ray =
                RayFace { face: Some(f), cast_shadow: true, opaque: face.flags.opaque, texture, uv: tc0 };
            let rc = RcFace {
                material: face.material,
                material_game: u32::from(face.material_game),
                t: tc0,
                reserved: 0,
            };
            let sm = face.sm_group;
            cl.add(p, ray, rc, sm);
            if let Some(face) = self.mesh.face_mut(f) {
                face.flags.processed = true;
            }
        }
        for r in &self.mu_refs {
            if let Some(m) = self.mu_models.get_mut(r.model as usize) {
                m.export_cform_rcast(&mut cl, &r.xform, &self.shaders, &self.materials);
            }
        }
        let (model, form) = cl.finish(*bounds)?;
        self.ray = Some(model);
        Ok(form)
    }

    /// What the level's light reads; an error before [`Self::build_rapid`].
    pub fn light_env(&self) -> Result<LightEnv<'_>> {
        let model =
            self.ray.as_ref().ok_or_else(|| LcError::Invalid("The ray model is not built yet.".into()))?;
        Ok(LightEnv {
            mesh: &self.mesh,
            model,
            textures: &self.textures,
            lights: &self.lights,
            params: &self.params,
            nosun: self.nosun,
            linear: self.linear,
        })
    }

    #[must_use]
    pub fn mu_env(&self) -> MuEnv<'_> {
        MuEnv {
            shaders: &self.shaders,
            materials: &self.materials,
            textures: &self.textures,
            params: &self.params,
            lights: &self.lights,
            nosun: self.nosun,
            linear: self.linear,
        }
    }

    /// `run_mu_light` with its waits: the models' and the references' light.
    pub fn light_mu(&mut self, ctl: &Ctl<'_>) -> Result<()> {
        let model =
            self.ray.take().ok_or_else(|| LcError::Invalid("The ray model is not built yet.".into()))?;
        let mut models = std::mem::take(&mut self.mu_models);
        let mut refs = std::mem::take(&mut self.mu_refs);
        let out = light_mu(&mut models, &mut refs, &self.mu_env(), &model, ctl);
        self.mu_models = models;
        self.mu_refs = refs;
        self.ray = Some(model);
        out
    }

    /// `CBuild::LMaps`: every deflector lit.
    pub fn light_deflectors(&mut self, ctl: &Ctl<'_>) -> Result<()> {
        let mut defls = std::mem::take(&mut self.deflectors);
        let out = self.light_env().and_then(|env| defls.light_all(&env, ctl));
        self.deflectors = defls;
        out
    }

    /// `LightVertex`.
    pub fn light_vertices(&mut self, ctl: &Ctl<'_>) -> Result<()> {
        let light = {
            let env = self.light_env()?;
            light_vertices(&env, &self.shaders, &self.materials, ctl)?
        };
        apply_vertex_light(&mut self.mesh, &light);
        Ok(())
    }

    /// `ImplicitLighting`.
    pub fn implicit_lighting(&mut self, ctl: &Ctl<'_>) -> Result<Vec<ImplicitMap>> {
        let model =
            self.ray.as_ref().ok_or_else(|| LcError::Invalid("The ray model is not built yet.".into()))?;
        let input = ImplicitInput {
            model,
            shaders: &self.shaders,
            materials: &self.materials,
            lights: &self.lights,
            params: &self.params,
            nosun: self.nosun,
            linear: self.linear,
        };
        implicit_lighting(&mut self.mesh, &mut self.textures, &input, ctl)
    }

    /// `CBuild::xrPhase_MergeLM`.
    pub fn merge_lightmaps(&mut self, ctl: &Ctl<'_>) -> Result<Vec<LightmapPage>> {
        merge_lightmaps(&mut self.deflectors, &mut self.mesh, &self.materials, &mut self.lightmaps, ctl)
    }

    /// `CBuild::Light`: the implicit light, the deflectors, the vertices, then the merge into
    /// pages. The MU light is [`Self::light_mu`], which the C++ runs beside these from the
    /// moment the ray model exists.
    pub fn light(&mut self, ctl: &Ctl<'_>) -> Result<LightOutput> {
        let implicit = self.implicit_lighting(ctl)?;
        self.light_deflectors(ctl)?;
        self.light_vertices(ctl)?;
        let pages = self.merge_lightmaps(ctl)?;
        Ok(LightOutput { implicit, pages })
    }

    /// The box of the listed vertices (`scene_bb` of `CBuild::Load`).
    #[must_use]
    pub fn scene_bounds(&self) -> Aabb {
        let mut bb = Aabb::INVALID;
        for &v in self.mesh.vertices() {
            if let Some(vx) = self.mesh.vertex(v) {
                bb.modify(vx.p);
            }
        }
        bb
    }

    /// `CBuild::Load`'s vertices and faces of `build.prj` (`EB_Vertices`, `EB_Faces`,
    /// `EB_SmoothGroups`): the faces checked (`Verify` is the caller's), their vertices joined.
    pub fn load_mesh(&mut self, prj: &BuildProject) -> Result<()> {
        let verts: Vec<_> = prj.verts.iter().map(|&p| self.mesh.create_vertex(p)).collect();
        for (i, b) in prj.faces.iter().enumerate() {
            if b.material_game >= 65536 {
                return Err(LcError::Invalid(format!("The face {i} has a game material id past 65535.")));
            }
            let mut v = [crate::mesh::NO_VERT; 3];
            for (slot, &id) in v.iter_mut().zip(&b.v) {
                *slot = *verts.get(id as usize).ok_or_else(|| {
                    LcError::Invalid(format!("The face {i} names a vertex the build does not have."))
                })?;
            }
            let f = self.mesh.create_face(v);
            if let Some(face) = self.mesh.face_mut(f) {
                face.material = b.material;
                face.material_game = b.material_game as u16;
                face.add_channel(b.t[0], b.t[1], b.t[2]);
                if let Some(&sm) = prj.smooth_groups.get(i) {
                    face.sm_group = sm;
                }
            }
        }
        for &v in &verts {
            if let Some(vx) = self.mesh.vertex_mut(v) {
                vx.n = Vec3::ZERO;
            }
        }
        Ok(())
    }
}
