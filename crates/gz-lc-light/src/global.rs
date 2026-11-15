//! Everything the level compiler holds while it builds, as a value the
//! caller owns, and the calls the level compiler makes
//! into the light as its methods.

use glam::Vec3;
use gz_fmt_level::{Aabb, BuildProject};

use crate::bake::{Lighter, LitScene};
use crate::data::{BuildMaterial, BuildParams, BuildTexture, ShaderXrlc, Shaders, shader_of};
use crate::deflector::{Deflectors, LightEnv};
use crate::implicit::{
    ImplicitInput, ImplicitMap, apply_implicit, has_implicit_lighting, implicit_lighting, implicit_shadows,
};
use crate::light::LightSet;
use crate::lightmap::{Lightmap, LightmapPage, PackReport, merge_lightmaps, pack_lightmaps};
use crate::mesh::{DeflId, FaceId, Mesh, Smoothing};
use crate::mu::{MuEnv, MuModel, MuReference, cache_opacity, light_mu};
use crate::ray::{Occluder, RayModel, RcastForm};
use crate::vertex::{apply_vertex_light, light_vertices};
use crate::{Ctl, LcError, Result};

/// The data a build holds.
#[derive(Clone, Debug)]
pub struct LcGlobalData {
    /// The build's parameters.
    pub params: BuildParams,
    /// The compiler shader library.
    pub shaders: Shaders,
    pub materials: Vec<BuildMaterial>,
    pub textures: Vec<BuildTexture>,
    pub lights: LightSet,
    pub ray: Option<RayModel>,
    /// The pages made by the merge.
    pub lightmaps: Vec<Lightmap>,
    pub mu_models: Vec<MuModel>,
    pub mu_refs: Vec<MuReference>,
    /// Faces that only cast shadows (a piece's build): in the ray model and nowhere else.
    pub occluders: Vec<Occluder>,
    /// The level's vertices and faces, and the error records.
    pub mesh: Mesh,
    /// The level's deflectors.
    pub deflectors: Deflectors,
    /// `-nosun`.
    pub nosun: bool,
    /// `-att` (`gl_linear`).
    pub linear: bool,
    /// A lightmap whose light allows it shrinks or collapses (off: `-no_lm_shrink`).
    pub shrink: bool,
}

/// What the light step makes besides what it changes in the data.
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
            occluders: Vec::new(),
            mesh: Mesh::new(),
            deflectors: Deflectors::default(),
            nosun: false,
            linear: false,
            shrink: true,
        }
    }

    /// The MU part of the load: the models and the references of `build.prj`.
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

    /// A face's shader.
    #[must_use]
    pub fn shader(&self, material: u16) -> &ShaderXrlc {
        shader_of(material, &self.shaders, &self.materials)
    }

    /// Whether a face is implicitly lit.
    #[must_use]
    pub fn has_implicit_lighting(&self, f: FaceId) -> bool {
        self.mesh
            .face(f)
            .is_some_and(|face| has_implicit_lighting(face, &self.shaders, &self.materials, &self.textures))
    }

    /// The opacity cache of every face and every model's.
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

    /// Calculates the normals of the vertices and faces.
    pub fn calc_normals(&mut self, smoothing: Smoothing, ctl: &Ctl<'_>) -> Result<isize> {
        let stop = || ctl.cancelled();
        let added = self.mesh.calc_normals_until(self.params.sm_angle, smoothing, &stop);
        ctl.check()?;
        Ok(added.unwrap_or(0))
    }

    /// Calculates the normals of the MU models.
    pub fn mu_calc_normals(&mut self, smoothing: Smoothing, ctl: &Ctl<'_>) -> Result<()> {
        let angle = self.params.sm_angle;
        // one after another through the pools they share
        let mut vpool = self.mu_models.last().map(|m| m.mesh.vpool.clone()).unwrap_or_default();
        let mut fpool = self.mu_models.last().map(|m| m.mesh.fpool.clone()).unwrap_or_default();
        for m in &mut self.mu_models {
            ctl.check()?;
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
        Ok(())
    }

    /// The shadow-casting faces of the level (one of each set of equal
    /// faces) and of every reference into the ray model, which replaces the one before; answers
    /// the soup as `build.cform` holds it. `bounds` is the box of the build's vertices.
    pub fn build_rapid(&mut self, bounds: &Aabb) -> Result<RcastForm> {
        self.build_rapid_with(bounds, &Ctl::quiet())
    }

    /// [`Self::build_rapid`] stopped by `ctl` between faces.
    pub fn build_rapid_with(&mut self, bounds: &Aabb, ctl: &Ctl<'_>) -> Result<RcastForm> {
        self.ray = None;
        let (model, form) = crate::ray::build_ray_model(
            &mut self.mesh,
            &mut self.mu_models,
            &self.mu_refs,
            &self.occluders,
            &self.shaders,
            &self.materials,
            bounds,
            ctl,
        )?;
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
            shrink: self.shrink,
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

    /// The models' and the references' light, in a scene of `lighter` of their own.
    pub fn light_mu(&mut self, lighter: &dyn Lighter, ctl: &Ctl<'_>) -> Result<()> {
        self.bake_parts(lighter, ctl, &mut |_| {}, &[Part::Mu]).map(|_| ())
    }

    /// Every deflector lit.
    pub fn light_deflectors(&mut self, lighter: &dyn Lighter, ctl: &Ctl<'_>) -> Result<()> {
        self.bake_parts(lighter, ctl, &mut |_| {}, &[Part::Charts]).map(|_| ())
    }

    /// The light of the vertices.
    pub fn light_vertices(&mut self, lighter: &dyn Lighter, ctl: &Ctl<'_>) -> Result<()> {
        self.bake_parts(lighter, ctl, &mut |_| {}, &[Part::Vertices]).map(|_| ())
    }

    /// The implicit lighting: the maps, and every lit texture's surface with the hemisphere in
    /// its alpha.
    pub fn implicit_lighting(&mut self, lighter: &dyn Lighter, ctl: &Ctl<'_>) -> Result<Vec<ImplicitMap>> {
        self.bake_parts(lighter, ctl, &mut |_| {}, &[Part::Implicit])
    }

    /// Every deflector packed into pages as a visual of its own.
    pub fn merge_lightmaps(&mut self, ctl: &Ctl<'_>) -> Result<Vec<LightmapPage>> {
        merge_lightmaps(&mut self.deflectors, &mut self.mesh, &mut self.lightmaps, ctl)
    }

    /// The lit charts packed into pages by visual: `visuals` names each visual's deflectors.
    pub fn pack_lightmaps(
        &mut self,
        visuals: &[Vec<DeflId>],
        ctl: &Ctl<'_>,
    ) -> Result<(Vec<LightmapPage>, PackReport)> {
        pack_lightmaps(&mut self.deflectors, visuals, &mut self.mesh, &mut self.lightmaps, ctl)
    }

    /// [`Self::pack_lightmaps`] with every visual in a group (`groups[i]` the group of visual `i`):
    /// the visuals of one group share pages that hold no other group's, and every group's pages
    /// are numbered from 1 (`lmap#1`), as each lies in a folder of its own. Answers each page's
    /// group beside the pages. A group from `small_from` on is a piece: it gets the smallest page
    /// that holds it when one page does.
    pub fn pack_lightmaps_grouped(
        &mut self,
        visuals: &[Vec<DeflId>],
        groups: &[u32],
        small_from: u32,
        ctl: &Ctl<'_>,
    ) -> Result<(Vec<(u32, LightmapPage)>, PackReport)> {
        let (pages, report, of_page) = crate::lightmap::pack_grouped(
            &mut self.deflectors,
            visuals,
            Some(groups),
            small_from,
            &mut self.mesh,
            &mut self.lightmaps,
            ctl,
        )?;
        Ok((of_page.into_iter().zip(pages).collect(), report))
    }

    /// The light of a build, every part of it through `lighter`, in one scene of the level where
    /// it can: the static models and their references, the implicit textures, the lightmaps'
    /// charts, the vertices. `phase` hears each part as it begins (0 to 3). Answers the maps of
    /// the implicit light; the textures' surfaces take the hemisphere when the light is done (or
    /// before the charts when a transparent shadow caster wears one of them).
    pub fn bake(
        &mut self,
        lighter: &dyn Lighter,
        ctl: &Ctl<'_>,
        phase: &mut dyn FnMut(usize),
    ) -> Result<Vec<ImplicitMap>> {
        self.bake_parts(lighter, ctl, phase, &[Part::Mu, Part::Implicit, Part::Charts, Part::Vertices])
    }

    fn bake_parts(
        &mut self,
        lighter: &dyn Lighter,
        ctl: &Ctl<'_>,
        phase: &mut dyn FnMut(usize),
        parts: &[Part],
    ) -> Result<Vec<ImplicitMap>> {
        let LcGlobalData {
            params,
            shaders,
            materials,
            textures,
            lights,
            ray,
            mu_models,
            mu_refs,
            mesh,
            deflectors,
            nosun,
            linear,
            shrink,
            ..
        } = self;
        let model = ray.as_ref().ok_or_else(|| LcError::Invalid("The ray model is not built yet.".into()))?;
        let (nosun, linear, shrink) = (*nosun, *linear, *shrink);
        let mut maps = Vec::new();
        let mut applied = false;
        {
            let scene = lighter.scene(model, textures)?;
            if parts.contains(&Part::Mu) {
                phase(0);
                let env = MuEnv { shaders, materials, textures, params, lights, nosun, linear };
                light_mu(mu_models, mu_refs, &env, lighter, scene.as_ref(), ctl)?;
                ctl.check()?;
            }
            if parts.contains(&Part::Implicit) {
                phase(1);
                let input = ImplicitInput { model, shaders, materials, lights, params, nosun, linear };
                maps = implicit_lighting(mesh, textures, &input, scene.as_ref(), ctl)?;
                ctl.check()?;
            }
            let later = parts.contains(&Part::Charts) || parts.contains(&Part::Vertices);
            if !(later && implicit_shadows(model, &maps)) {
                let light = {
                    let env = LightEnv { mesh, model, textures, lights, params, nosun, linear, shrink };
                    Self::bake_rest(&env, deflectors, shaders, materials, scene.as_ref(), parts, phase, ctl)?
                };
                light.apply(mesh);
                applied = true;
            }
        }
        apply_implicit(textures, &maps);
        if !applied {
            // a transparent shadow caster wears a lit texture: the rest sees its new alpha
            let scene = lighter.scene(model, textures)?;
            let light = {
                let env = LightEnv { mesh, model, textures, lights, params, nosun, linear, shrink };
                Self::bake_rest(&env, deflectors, shaders, materials, scene.as_ref(), parts, phase, ctl)?
            };
            drop(scene);
            light.apply(mesh);
        }
        Ok(maps)
    }

    /// The charts and the vertices of `parts`; the vertices' light is the answer, written once
    /// the mesh is free.
    #[allow(clippy::too_many_arguments)]
    fn bake_rest(
        env: &LightEnv<'_>,
        deflectors: &mut Deflectors,
        shaders: &Shaders,
        materials: &[BuildMaterial],
        scene: &dyn LitScene,
        parts: &[Part],
        phase: &mut dyn FnMut(usize),
        ctl: &Ctl<'_>,
    ) -> Result<VertexLight> {
        if parts.contains(&Part::Charts) {
            phase(2);
            deflectors.light_all(env, scene, ctl)?;
        }
        if parts.contains(&Part::Vertices) {
            phase(3);
            return Ok(VertexLight(Some(light_vertices(env, shaders, materials, scene, ctl)?)));
        }
        Ok(VertexLight(None))
    }

    /// The implicit light, the deflectors, the vertices, then each deflector packed as a visual
    /// of its own. The MU light is [`Self::light_mu`].
    pub fn light(&mut self, lighter: &dyn Lighter, ctl: &Ctl<'_>) -> Result<LightOutput> {
        let implicit =
            self.bake_parts(lighter, ctl, &mut |_| {}, &[Part::Implicit, Part::Charts, Part::Vertices])?;
        let pages = self.merge_lightmaps(ctl)?;
        Ok(LightOutput { implicit, pages })
    }

    /// The box of the listed vertices.
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

    /// The vertices and faces of `build.prj`: the faces checked (the check is the caller's), their vertices joined.
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

/// The parts of a build's light.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Part {
    Mu,
    Implicit,
    Charts,
    Vertices,
}

/// The vertices' new light, when they were lit.
struct VertexLight(Option<Vec<Option<crate::color::BaseColor>>>);

impl VertexLight {
    fn apply(self, mesh: &mut Mesh) {
        if let Some(l) = self.0 {
            apply_vertex_light(mesh, &l);
        }
    }
}
