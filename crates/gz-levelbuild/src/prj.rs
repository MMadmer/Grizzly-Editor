//! The build project the level compiler reads (`build.prj`).

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_base::{Finding, Findings};
use gz_fmt_level::{
    Aabb, BuildFace, BuildGlow, BuildLight, BuildLightDynamic, BuildLod, BuildMaterial, BuildParams,
    BuildProject, BuildTexture, Light, LightControl, LodFace, MuModel, MuReference, Occluder, Portal,
    XRCL_CURRENT_VERSION, i_floor,
};
use gz_fmt_object::{Mesh, ObjectFlags, Surface, SurfaceFlags, VMapKind};
use gz_scene::library::LibraryObject;
use gz_scene::math::{Mat4, xform_box};
use gz_scene::objects::{LT_POINT, SO_USE_SURFACE, light_flags, terrain};
use gz_scene::{ObjClass, ObjData, Object, SceneState};

use crate::hemi::{HEMI_1, HEMI_2, HEMI_3};
use crate::libs::{Engine, mtl_flags, xrlc_flags};
use crate::merge::{self, SourceImage};
use crate::{BuildError, Result};

pub const LCONTROL_HEMI: &str = "$hemi";
pub const LCONTROL_SUN: &str = "$sun";
pub const LEVEL_LODS_TEX_NAME: &str = "level_lods";
const LOD_SHADER_NAME: &str = "details\\lod";
const LOD_SAMPLE_COUNT: u32 = 8;
const LOD_IMAGE_SIZE: u32 = 64;
const D3DLIGHT_DIRECTIONAL: u32 = 3;
const EPS_S: f32 = 0.000_001;
/// The draft quality.
const QUALITY_DRAFT: u16 = 0;
/// The glow's fixed size.
const GLOW_FIXED_SIZE: u16 = 1 << 0;

/// A library object by reference.
pub type Lookup<'a> = &'a (dyn Fn(&str) -> Option<Arc<LibraryObject>> + Sync);
/// A LOD picture by texture name (`lod\lod_props_box`): its RGBA pixels, width, height.
pub type LodSource<'a> = &'a (dyn Fn(&str) -> Option<(Vec<u8>, u32, u32)> + Sync);

/// Vertices per square metre, for the scene statistic.
#[derive(Clone, Debug, Default)]
pub struct SceneStat {
    pub min: Vec3,
    pub sx: u32,
    pub sz: u32,
    pub svert: Vec<u32>,
    pub muvert: Vec<u32>,
}

impl SceneStat {
    fn new(b: &Aabb) -> Self {
        let size = b.max - b.min;
        let sx = i_floor(size.x + 1.0).max(1) as u32;
        let sz = i_floor(size.z + 1.0).max(1) as u32;
        let n = (sx as usize).saturating_mul(sz as usize).min(1 << 28);
        Self { min: b.min, sx, sz, svert: vec![0; n], muvert: vec![0; n] }
    }

    fn cell(&self, p: Vec3) -> usize {
        let ix = (i_floor(p.x - self.min.x).max(0) as u32).min(self.sx - 1);
        let iz = (i_floor(p.z - self.min.z).max(0) as u32).min(self.sz - 1);
        (iz * self.sx + ix) as usize
    }

    fn add_svert(&mut self, p: Vec3) {
        let c = self.cell(p);
        if let Some(v) = self.svert.get_mut(c) {
            *v += 1;
        }
    }

    fn add_muvert(&mut self, p: Vec3) {
        let c = self.cell(p);
        if let Some(v) = self.muvert.get_mut(c) {
            *v += 1;
        }
    }
}

/// What the compile made.
#[derive(Clone, Debug)]
pub struct Compiled {
    pub project: BuildProject,
    pub stat: SceneStat,
    /// `level_lods.dds` and `level_lods_nm.dds` when the scene has LODs.
    pub lod_textures: Option<(Vec<u8>, Vec<u8>)>,
    pub log: Vec<Finding>,
}

/// The sectors of the scene, numbered, with their volumes.
struct Sectors {
    list: Vec<SectorVol>,
    default: u16,
}

struct SectorVol {
    num: u16,
    name: String,
    items: Vec<(String, String)>,
    bbox: Aabb,
    center: Vec3,
    radius: f32,
}

/// Whether a mesh is visible.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Visible {
    None,
    PartialOutside,
    PartialInside,
    Fully,
}

impl SectorVol {
    /// Whether a sphere intersects the sector.
    fn intersect(&self, c: Vec3, r: f32) -> Visible {
        let dist = self.center.distance(c);
        let half = (self.bbox.max - self.bbox.min) * 0.5;
        let in_sphere = dist + r < self.radius && r > half.x && r > half.y && r > half.z;
        let contains = (0..3).all(|a| c[a] >= self.bbox.min[a] && c[a] <= self.bbox.max[a]);
        if contains {
            return if in_sphere { Visible::Fully } else { Visible::PartialInside };
        }
        if dist < r + self.radius { Visible::PartialOutside } else { Visible::None }
    }
}

impl Sectors {
    fn new(scene: &SceneState, lib: Lookup<'_>) -> Self {
        let mut list = Vec::new();
        let mut default = None;
        // over a level of the game the cells are the game's: a sector the module's scene holds is
        // not built (`S222`), and a piece hangs on the level's sectors where it stands
        let own = scene.of_class(ObjClass::Sector).filter(|_| !scene.meta.base.is_set());
        for (i, o) in own.enumerate() {
            let ObjData::Sector(s) = &o.data else { continue };
            if s.default {
                default = Some(i as u16);
            }
            let mut bbox = Aabb::INVALID;
            for (obj, mesh) in &s.items {
                let Some(so) = scene.find(ObjClass::SceneObject, obj) else { continue };
                let Some(lo) = reference(so).and_then(lib) else { continue };
                let Some(m) = lo.object.meshes.iter().find(|m| m.name == mesh.as_str()) else { continue };
                let wb = xform_box(&Aabb::new(m.bbox.min, m.bbox.max), &so.world());
                if wb.is_valid() {
                    bbox.modify(wb.min);
                    bbox.modify(wb.max);
                }
            }
            let center = if bbox.is_valid() { (bbox.min + bbox.max) * 0.5 } else { Vec3::ZERO };
            let radius = if bbox.is_valid() { center.distance(bbox.max) } else { 0.0 };
            list.push(SectorVol {
                num: i as u16,
                name: o.name.to_string(),
                items: s.items.iter().map(|(a, b)| (a.to_string(), b.to_string())).collect(),
                bbox,
                center,
                radius,
            });
        }
        let default = default.unwrap_or(list.len() as u16);
        Self { list, default }
    }

    /// The sector a mesh of an object lies in.
    fn of_mesh(&self, object: &str, mesh: &str) -> u16 {
        self.list
            .iter()
            .find(|s| s.items.iter().any(|(o, m)| o == object && m == mesh))
            .map_or(self.default, |s| s.num)
    }

    /// The sector of a mesh.
    fn at(&self, p: Vec3, r: f32) -> u16 {
        for s in &self.list {
            let v = s.intersect(p, r);
            if (v == Visible::PartialInside || v == Visible::Fully) && s.num != self.default {
                return s.num;
            }
        }
        self.default
    }
}

fn reference(o: &Object) -> Option<&str> {
    match &o.data {
        ObjData::SceneObject(d) if !d.visual_mode() && !d.reference.is_empty() => Some(d.reference.as_str()),
        _ => None,
    }
}

/// What an object is compiled as.
struct Model {
    /// The faces: a terrain's mesh, a ground object's cut model, or the object's own.
    lo: Arc<LibraryObject>,
    /// The object's own model: its name and LOD.
    base: Arc<LibraryObject>,
    /// Where the faces are: a terrain's mesh is made in the world.
    world: Mat4,
    /// A cutting terrain meets the object, and its cut model is not made: it is its own whole.
    uncut: bool,
}

/// A scene object or a terrain as the build compiles it. The ground a cutting terrain meets is
/// its cut model ([`terrain::is_cut`]), the swap the editor's picture and rays make, so nothing
/// under a terrain is built or casts a shadow twice.
fn model_of(scene: &SceneState, o: &Object, lib: Lookup<'_>) -> Option<Model> {
    match o.class {
        ObjClass::Terrain => {
            let lo = lib(&terrain::reference(&o.name))?;
            Some(Model { base: lo.clone(), lo, world: Mat4::IDENTITY, uncut: false })
        }
        ObjClass::SceneObject => {
            let base = reference(o).and_then(lib)?;
            let (lo, uncut) = if terrain::is_cut(scene, lib, o) {
                lib(&terrain::cut_reference(&o.name)).map_or_else(|| (base.clone(), true), |c| (c, false))
            } else {
                (base.clone(), false)
            };
            Some(Model { lo, base, world: o.world(), uncut })
        }
        _ => None,
    }
}

/// Whether a terrain's look is the level's ground under it.
fn wears_the_ground(o: &Object) -> bool {
    matches!(&o.data, ObjData::Terrain(d) if d.surface == gz_terrain::Surface::Ground)
}

/// The terrains' meshes' box: they are made in the world.
fn terrains_box(scene: &SceneState, lib: Lookup<'_>, b: &mut Aabb) {
    for o in scene.of_class(ObjClass::Terrain) {
        if let Some(lo) = lib(&terrain::reference(&o.name))
            && lo.bounds.is_valid()
        {
            b.modify(lo.bounds.min);
            b.modify(lo.bounds.max);
        }
    }
}

/// Whether an object is static / multiple-usage static.
fn is_static(o: &gz_fmt_object::Object) -> bool {
    !o.flags.intersects(
        ObjectFlags::SOUND_OCCLUDER | ObjectFlags::DYNAMIC | ObjectFlags::HOM | ObjectFlags::MULTIPLE_USAGE,
    )
}

fn is_mu_static(o: &gz_fmt_object::Object) -> bool {
    !o.flags.intersects(ObjectFlags::SOUND_OCCLUDER | ObjectFlags::DYNAMIC | ObjectFlags::HOM)
        && o.flags.contains(ObjectFlags::MULTIPLE_USAGE)
}

/// The surfaces a scene object wears: the library's, with the scene's own shader, compiler
/// shader, material, texture and vertex map where it keeps them.
#[must_use]
pub fn object_surfaces(o: &Object, lo: &LibraryObject) -> Vec<Surface> {
    let mut out = lo.object.surfaces.clone();
    if let ObjData::SceneObject(d) = &o.data
        && d.flags & SO_USE_SURFACE != 0
    {
        for s in &mut out {
            if let Some(ov) = d.surfaces.iter().find(|v| v.name == s.name) {
                s.shader = ov.shader.clone();
                s.shader_xrlc = ov.shader_xrlc.clone();
                s.game_mtl = ov.game_mtl.clone();
                s.texture = ov.texture.clone();
                s.vmap = ov.vmap.clone();
            }
        }
    }
    out
}

/// The area: Heron's formula.
fn calc_area(a: Vec3, b: Vec3, c: Vec3) -> f32 {
    let (e1, e2, e3) = (a.distance(b), a.distance(c), b.distance(c));
    let p = (e1 + e2 + e3) / 2.0;
    (p * (p - e1) * (p - e2) * (p - e3)).sqrt()
}

/// The UV of a face corner: the first UV map entry of its vertex-map reference.
fn corner_uv(mesh: &Mesh, vmref: u32) -> Vec2 {
    let Some(refs) = mesh.vmrefs.get(vmref as usize) else { return Vec2::ZERO };
    for pt in refs {
        let Some(map) = mesh.vmaps.get(pt.vmap as usize) else { continue };
        if map.kind != VMapKind::Uv {
            continue;
        }
        let i = pt.index as usize * 2;
        return Vec2::new(
            map.values.get(i).copied().unwrap_or(0.0),
            map.values.get(i + 1).copied().unwrap_or(0.0),
        );
    }
    Vec2::ZERO
}

/// The builder's lists (`SceneBuilder::l_*`).
#[derive(Default)]
struct Builder {
    verts: Vec<Vec3>,
    faces: Vec<BuildFace>,
    smgroups: Vec<u32>,
    materials: Vec<BuildMaterial>,
    shaders: Vec<String>,
    shaders_xrlc: Vec<String>,
    textures: Vec<String>,
    glows: Vec<BuildGlow>,
    portals: Vec<Portal>,
    controls: Vec<String>,
    lights_static: Vec<BuildLight>,
    lights_dynamic: Vec<BuildLightDynamic>,
    lods: Vec<(BuildLod, String, Vec<u8>, Vec<u8>)>,
    mu_models: Vec<MuModel>,
    mu_refs: Vec<MuReference>,
    occluders: Vec<Occluder>,
    log: Vec<Finding>,
    /// The faces made now keep the level's own light ([`gz_lc_light::KEEPS_LEVEL_LIGHT`]).
    keep_light: bool,
    /// The shaders for models the scene's static surfaces named, each with the shader for level
    /// geometry it was compiled with.
    twins: Vec<(String, String)>,
}

struct Ctx<'a> {
    engine: &'a Engine,
    quality: u16,
    lods: LodSource<'a>,
}

impl Builder {
    fn light_control(&mut self, name: &str) -> u32 {
        if let Some(i) = self.controls.iter().position(|c| c == name) {
            return i as u32;
        }
        self.controls.push(name.to_owned());
        (self.controls.len() - 1) as u32
    }

    fn shader(&mut self, cx: &Ctx<'_>, name: &str) -> Result<u16> {
        if let Some(i) = self.shaders.iter().position(|s| s == name) {
            return Ok(i as u16);
        }
        if !cx.engine.has_shader(name) {
            return Err(BuildError::new(format!("Can't find engine shader: {name}")));
        }
        self.shaders.push(name.to_owned());
        Ok((self.shaders.len() - 1) as u16)
    }

    fn shader_xrlc(&mut self, cx: &Ctx<'_>, name: &str) -> Result<u16> {
        if let Some(i) = self.shaders_xrlc.iter().position(|s| s == name) {
            return Ok(i as u16);
        }
        if cx.engine.xrlc_flags(name).is_none() {
            return Err(BuildError::new(format!("Can't find compiler shader: {name}")));
        }
        self.shaders_xrlc.push(name.to_owned());
        Ok((self.shaders_xrlc.len() - 1) as u16)
    }

    fn texture(&mut self, name: &str) -> Result<u16> {
        if name.is_empty() {
            return Err(BuildError::new("Invalid texture name found."));
        }
        if let Some(i) = self.textures.iter().position(|t| t.eq_ignore_ascii_case(name)) {
            return Ok(i as u16);
        }
        self.textures.push(name.to_owned());
        Ok((self.textures.len() - 1) as u16)
    }

    fn find_material(&mut self, m: BuildMaterial) -> u32 {
        if let Some(i) = self.materials.iter().position(|x| {
            x.surface == m.surface
                && x.shader == m.shader
                && x.shader_xrlc == m.shader_xrlc
                && x.sector == m.sector
                && x.internal_max_area == m.internal_max_area
        }) {
            return i as u32;
        }
        self.materials.push(m);
        (self.materials.len() - 1) as u32
    }

    /// The material of a surface in a sector.
    fn material(&mut self, cx: &Ctx<'_>, s: &Surface, sector: u16, allow_draft: bool) -> Result<u32> {
        self.material_of(cx, &s.shader, &s.shader_xrlc, &s.texture, sector, allow_draft)
    }

    /// The material of an engine shader, a compiler shader and a texture in a sector.
    fn material_of(
        &mut self,
        cx: &Ctx<'_>,
        esh: &str,
        csh: &str,
        tex: &str,
        sector: u16,
        allow_draft: bool,
    ) -> Result<u32> {
        // a compiled model placed as a static object still names the shader it was made for as
        // a model: level geometry is drawn with the library's shader for a level that treats the
        // texture's alpha the same way - with the model's, the game draws nothing
        let level = cx.engine.level_shader(esh);
        if !level.eq_ignore_ascii_case(esh)
            && !self.twins.iter().any(|(was, _)| was.eq_ignore_ascii_case(esh))
        {
            self.twins.push((esh.to_owned(), level.to_owned()));
        }
        let esh = level;
        let draft = allow_draft
            && cx.quality == QUALITY_DRAFT
            && cx.engine.xrlc_flags(csh).is_some_and(|f| f & xrlc_flags::RENDERING != 0);
        let (shader, shader_xrlc) = if draft {
            (self.shader(cx, "def_shaders\\def_vertex")?, self.shader_xrlc(cx, "def_shaders\\def_vertex")?)
        } else {
            (self.shader(cx, esh)?, self.shader_xrlc(cx, csh)?)
        };
        let surface = self.texture(tex)?;
        Ok(self.find_material(BuildMaterial {
            surface,
            shader,
            shader_xrlc,
            sector,
            reserved: 0,
            internal_max_area: if self.keep_light { gz_lc_light::KEEPS_LEVEL_LIGHT } else { 0 },
        }))
    }

    /// The mesh's points through `parent`, its faces per surface; `real` places
    /// the faces in the world for the area check. Appends to `verts` / `faces` / `smgroups`.
    #[allow(clippy::too_many_arguments)]
    fn mesh(
        &mut self,
        cx: &Ctx<'_>,
        parent: &Mat4,
        real: &Mat4,
        object: &Object,
        lo: &LibraryObject,
        surfaces: &[Surface],
        mesh: &Mesh,
        sector: u16,
        out: &mut (Vec<Vec3>, Vec<BuildFace>, Vec<u32>),
    ) -> Result<()> {
        let offs = out.0.len() as u32;
        out.0.extend(mesh.points.iter().map(|p| parent.transform_tiny(*p)));
        let allow_draft = !is_mu_static(&lo.object);
        for sf in &mesh.surf_faces {
            let Some(surf) = surfaces.get(sf.surface as usize) else { continue };
            if surf.game_mtl == "materials\\occ" {
                continue;
            }
            let m_id = self.material(cx, surf, sector, allow_draft)?;
            let Some(gm_id) = cx.engine.material_id(&surf.game_mtl) else {
                return Err(BuildError::new(format!("Surface: '{}' contains bad game material.", surf.name)));
            };
            let Some(flags) = cx.engine.material_flags(gm_id) else {
                return Err(BuildError::new(format!(
                    "Surface: '{}' contains undefined game material.",
                    surf.name
                )));
            };
            if flags & mtl_flags::BREAKABLE != 0 {
                self.log.info(format!(
                    "Surface '{}' has a breakable game material and is not part of the static geometry.",
                    surf.name
                ));
                continue;
            }
            if flags & mtl_flags::CLIMABLE != 0 {
                self.log.info(format!(
                    "Surface '{}' has a climable game material and is not part of the static geometry.",
                    surf.name
                ));
                continue;
            }
            if flags & mtl_flags::DYNAMIC != 0 {
                return Err(BuildError::new(format!(
                    "Surface: '{}' contains non-static game material.",
                    surf.name
                )));
            }
            if (surf.fvf & 0xf00) >> 8 != 1 {
                return Err(BuildError::new(format!(
                    "Surface: '{}' contains more than 1 texture refs.",
                    surf.name
                )));
            }
            let mut invalid = 0u32;
            for &fi in sf.faces.iter() {
                let Some(face) = mesh.faces.get(fi as usize) else { continue };
                let p = face.v.map(|v| {
                    real.transform_tiny(mesh.points.get(v.point as usize).copied().unwrap_or(Vec3::ZERO))
                });
                let a = calc_area(p[0], p[1], p[2]);
                if !a.is_finite() || a < EPS_S {
                    invalid += 1;
                    continue;
                }
                let sg = mesh.smooth_groups.get(fi as usize).copied().unwrap_or(0);
                let mut f = BuildFace { material: m_id as u16, material_game: gm_id, ..BuildFace::default() };
                for k in 0..3 {
                    f.v[k] = face.v[k].point + offs;
                    f.t[k] = corner_uv(mesh, face.v[k].vmref);
                }
                out.1.push(f);
                out.2.push(sg & !(1 << 3));
                if surf.flags.contains(SurfaceFlags::TWO_SIDED) {
                    let mut b =
                        BuildFace { material: m_id as u16, material_game: gm_id, ..BuildFace::default() };
                    for k in 0..3 {
                        b.v[k] = face.v[2 - k].point + offs;
                        b.t[k] = corner_uv(mesh, face.v[2 - k].vmref);
                    }
                    out.1.push(b);
                    out.2.push(sg | (1 << 3));
                }
            }
            if invalid > 0 {
                self.log.warn(format!(
                    "Object '{}', mesh '{}': {invalid} invalid face(s) are removed.",
                    object.name, mesh.name
                ));
            }
        }
        Ok(())
    }

    /// The faces of a mesh placed by `world` whose compiler shader casts a shadow, as occluders:
    /// what a piece of a level is lit beside. A face of no texture or too small to be one is left
    /// out; a two-sided surface casts from both sides.
    fn occluders(&mut self, cx: &Ctx<'_>, world: &Mat4, surfaces: &[Surface], mesh: &Mesh) -> Result<()> {
        for sf in &mesh.surf_faces {
            let Some(surf) = surfaces.get(sf.surface as usize) else { continue };
            let casts = cx
                .engine
                .xrlc_flags(&surf.shader_xrlc)
                .is_some_and(|f| f & gz_fmt_xr::ShaderXrlc::LIGHT_CAST_SHADOW != 0);
            if surf.game_mtl == "materials\\occ" || !casts || surf.texture.is_empty() {
                continue;
            }
            let texture = self.texture(&surf.texture)?;
            for &fi in sf.faces.iter() {
                let Some(face) = mesh.faces.get(fi as usize) else { continue };
                let p = face.v.map(|v| {
                    world.transform_tiny(mesh.points.get(v.point as usize).copied().unwrap_or(Vec3::ZERO))
                });
                let a = calc_area(p[0], p[1], p[2]);
                if !a.is_finite() || a < EPS_S {
                    continue;
                }
                let t = face.v.map(|v| corner_uv(mesh, v.vmref));
                self.occluders.push(Occluder { p, t, texture });
                if surf.flags.contains(SurfaceFlags::TWO_SIDED) {
                    self.occluders.push(Occluder { p: [p[2], p[1], p[0]], t: [t[2], t[1], t[0]], texture });
                }
            }
        }
        Ok(())
    }

    /// A static object's meshes placed by `world` into the level's own lists, each mesh in its
    /// sector, counted in the scene statistic.
    fn static_object(
        &mut self,
        cx: &Ctx<'_>,
        o: &Object,
        lo: &LibraryObject,
        world: &Mat4,
        sectors: &Sectors,
        stat: &mut SceneStat,
    ) -> Result<()> {
        let surfaces = object_surfaces(o, lo);
        let mut out = (
            std::mem::take(&mut self.verts),
            std::mem::take(&mut self.faces),
            std::mem::take(&mut self.smgroups),
        );
        for m in &lo.object.meshes {
            let sector = sectors.of_mesh(&o.name, &m.name);
            let r = self.mesh(cx, world, world, o, lo, &surfaces, m, sector, &mut out);
            if let Err(e) = r {
                (self.verts, self.faces, self.smgroups) = out;
                self.log.error(e.0.clone());
                return Err(BuildError::new(format!("{}\nThe object '{}' could not be built.", e.0, o.name)));
            }
            for p in m.points.iter() {
                stat.add_svert(world.transform_tiny(*p));
            }
        }
        (self.verts, self.faces, self.smgroups) = out;
        Ok(())
    }

    /// A ground object a cutting terrain meets whose cut model is not made is built whole: said.
    fn uncut(&mut self, o: &Object, m: &Model) {
        if m.uncut {
            self.log.warn(format!(
                "The ground object '{}' that a terrain cuts is built whole: its cut model could not be made.",
                o.name
            ));
        }
    }

    /// `None` for an object without a LOD.
    fn lod(&mut self, cx: &Ctx<'_>, lo: &LibraryObject, sector: u16) -> Result<Option<u16>> {
        if !lo.object.flags.contains(ObjectFlags::USING_LOD) {
            return Ok(None);
        }
        let surface = self.texture(LEVEL_LODS_TEX_NAME)?;
        let shader = self.shader(cx, LOD_SHADER_NAME)?;
        let material = self.find_material(BuildMaterial {
            surface,
            shader,
            shader_xrlc: u16::MAX,
            sector,
            reserved: 0,
            internal_max_area: 0,
        });
        let name = lod_texture_name(&lo.name);
        let bb = lo.object.bounding_box();
        let mut lod = BuildLod { material, ..BuildLod::default() };
        for frame in 0..LOD_SAMPLE_COUNT {
            let (p, t) = lod_frame(frame, bb.min, bb.max);
            lod.faces[frame as usize] = LodFace { v: p, t };
        }
        let Some((data, w, h)) = (cx.lods)(&name) else {
            return Err(BuildError::new(format!("The LOD texture '{name}' is missing.")));
        };
        let nm = format!("{name}_nm");
        let Some((ndata, nw, nh)) = (cx.lods)(&nm) else {
            return Err(BuildError::new(format!("The LOD texture '{nm}' is missing.")));
        };
        let want = (LOD_IMAGE_SIZE * LOD_SAMPLE_COUNT, LOD_IMAGE_SIZE);
        if (w, h) != want || (nw, nh) != want {
            return Err(BuildError::new(format!("The LOD texture '{name}' is not {}x{}.", want.0, want.1)));
        }
        self.lods.push((lod, name, data, ndata));
        Ok(Some((self.lods.len() - 1) as u16))
    }
}

/// `lod_` and the library name with `\` as `_`, in the folder its first
/// part names (`lod\lod_props_box`).
#[must_use]
pub fn lod_texture_name(lib_name: &str) -> String {
    let name = format!("lod_{}", lib_name.replace('\\', "_"));
    match name.split_once('_') {
        Some((folder, _)) => format!("{folder}\\{name}"),
        None => name,
    }
}

/// The quad of one of the eight views, in object space, and its texture
/// coordinates in the 8 x 1 strip.
fn lod_frame(frame: u32, min: Vec3, max: Vec3) -> ([Vec3; 4], [Vec2; 4]) {
    let tex_w = (LOD_SAMPLE_COUNT * LOD_IMAGE_SIZE) as f32;
    let tex_h = LOD_IMAGE_SIZE as f32;
    let (hpx, hpy) = (0.5 * (1.0 / tex_w), 0.5 * (1.0 / tex_h));
    let (ox, oy) = (1.0 / tex_w, 1.0 / tex_h);
    let pos = [
        Vec3::new(-1.0 + ox, 1.0 - oy, 0.0),
        Vec3::new(1.0 - ox, 1.0 - oy, 0.0),
        Vec3::new(1.0 - ox, -1.0 + oy, 0.0),
        Vec3::new(-1.0 + ox, -1.0 + oy, 0.0),
    ];
    let c = (min + max) * 0.5;
    let s = (max - min) * 0.5;
    let r = s.x.max(s.z);
    let mut t = Mat4::IDENTITY;
    t.0[0][0] = r;
    t.0[1][1] = s.y;
    t.0[2][2] = r;
    t.set_c(c);
    let angle = frame as f32 * (std::f32::consts::TAU / LOD_SAMPLE_COUNT as f32);
    // rotateY(-angle)
    let (sn, cs) = (-angle).sin_cos();
    let mut rot = Mat4::IDENTITY;
    rot.0[0][0] = cs;
    rot.0[0][2] = -sn;
    rot.0[2][0] = sn;
    rot.0[2][2] = cs;
    let m = Mat4::mul(&t, &rot);
    let ts = (1.0 / LOD_SAMPLE_COUNT as f32, 1.0);
    let lt = Vec2::new(frame as f32 * ts.0 + hpx, hpy);
    let rb = Vec2::new((frame + 1) as f32 * ts.0 - hpx, ts.1 - hpy);
    (pos.map(|p| m.transform_tiny(p)), [lt, Vec2::new(rb.x, lt.y), rb, Vec2::new(lt.x, rb.y)])
}

/// The directions (away from the sky) and their share of `energy`.
#[must_use]
pub fn hemisphere(quality: u8, energy: f32) -> Vec<(Vec3, f32)> {
    let table: &[[f32; 3]] = match quality {
        1 => &HEMI_1,
        2 => &HEMI_2,
        3 => &HEMI_3,
        _ => &[],
    };
    let e = 1.0 / table.len().max(1) as f32;
    table
        .iter()
        .map(|h| {
            let (x, y, z) = (-h[0], -h[1], -h[2]);
            let mag = (x * x + y * y + z * z).sqrt();
            (Vec3::new(x / mag, y / mag, z / mag), e * energy)
        })
        .collect()
}

/// The heading and pitch of a direction.
fn set_hp(h: f32, p: f32) -> Vec3 {
    let (sh, ch) = h.sin_cos();
    let (sp, cp) = p.sin_cos();
    Vec3::new(-cp * sh, sp, cp * ch)
}

fn directional(diffuse: [f32; 4], direction: Vec3) -> Light {
    Light { kind: D3DLIGHT_DIRECTIONAL, diffuse, direction, ..Light::default() }
}

/// The colours scaled.
fn light_mul(mut l: Light, k: f32) -> Light {
    for c in [&mut l.diffuse, &mut l.specular, &mut l.ambient] {
        c[0] *= k;
        c[1] *= k;
        c[2] *= k;
    }
    l
}

impl Builder {
    /// Builds the hemisphere lights.
    fn hemi(&mut self, quality: u8, control: &str) {
        let id = self.light_control(control);
        if quality > 0 {
            for (dir, energy) in hemisphere(quality, 2.0) {
                let l = directional([1.0; 4], dir);
                self.lights_static.push(BuildLight { controller: id, light: light_mul(l, energy) });
            }
        } else {
            self.lights_static
                .push(BuildLight { controller: id, light: directional([1.0; 4], Vec3::new(0.0, -1.0, 0.0)) });
        }
    }

    /// Builds the sun.
    fn sun(&mut self, quality: u8, dir: [f32; 2]) -> Result<()> {
        let id = self.light_control(LCONTROL_SUN);
        let samples = match quality {
            0 => 1,
            1 => 3,
            2 => 4,
            3 => 7,
            _ => return Err(BuildError::new("Invalid case.")),
        };
        let e = 1.0 / (samples * samples) as f32;
        let disp = 3.0f32.to_radians();
        let da = disp / samples as f32;
        let (mn_x, mn_y) = (dir[0] - disp / 2.0, dir[1] - disp / 2.0);
        for x in 0..samples {
            let fx = mn_x + x as f32 * da;
            for y in 0..samples {
                let fy = mn_y + y as f32 * da;
                self.lights_static
                    .push(BuildLight { controller: id, light: directional([e, e, e, 1.0], set_hp(fy, fx)) });
            }
        }
        self.lights_dynamic.push(BuildLightDynamic {
            light: BuildLight { controller: id, light: directional([1.0; 4], set_hp(dir[1], dir[0])) },
            sectors: [0; 16],
            sector_count: 0,
        });
        Ok(())
    }

    /// `BuildLight`.
    fn light(&mut self, o: &Object, sectors: &Sectors, scene: &SceneState) -> Result<()> {
        let ObjData::Light(e) = &o.data else { return Ok(()) };
        if e.flags & (light_flags::AFFECT_STATIC | light_flags::AFFECT_DYNAMIC) == 0 {
            return Err(BuildError::new(format!("The object '{}' could not be built.", o.name)));
        }
        let lt = scene.tools.part(ObjClass::Light).light().cloned().unwrap_or_default();
        let Some(control) = lt.name_of(e.lcontrol).map(str::to_owned) else {
            self.log.error(format!("Light '{}' has an invalid light control name.", o.name));
            return Err(BuildError::new(format!("The object '{}' could not be built.", o.name)));
        };
        let rot = o.rotation();
        let light = Light {
            kind: e.kind,
            diffuse: [
                e.color[0] * e.brightness,
                e.color[1] * e.brightness,
                e.color[2] * e.brightness,
                e.color[3],
            ],
            position: o.position(),
            direction: set_hp(rot.y, rot.x),
            range: e.range,
            attenuation0: e.attenuation[0],
            attenuation1: e.attenuation[1],
            attenuation2: e.attenuation[2],
            phi: e.cone,
            ..Light::default()
        };
        let controller = self.light_control(&control);
        // the sectors a light of the level's own file shines in: only a point light is written
        // there (below), so no other kind is asked for one - or refused for standing in none
        let mut dyn_sectors: Vec<u16> = Vec::new();
        if e.flags & light_flags::AFFECT_DYNAMIC != 0 && e.kind == LT_POINT {
            if sectors.list.is_empty() {
                dyn_sectors.push(sectors.default);
            } else {
                for s in &sectors.list {
                    if dyn_sectors.len() >= 16 {
                        break;
                    }
                    let v = s.intersect(o.position(), e.range);
                    if v == Visible::PartialInside || v == Visible::Fully {
                        dyn_sectors.push(s.num);
                    }
                }
                for s in &sectors.list {
                    if dyn_sectors.len() >= 16 {
                        break;
                    }
                    if s.intersect(o.position(), e.range) == Visible::PartialOutside {
                        dyn_sectors.push(s.num);
                    }
                }
                if dyn_sectors.is_empty() {
                    return Err(BuildError::new(format!("The object '{}' could not be built.", o.name)));
                }
            }
        }
        // every kind is baked: a point light, a spot light inside its cone, a directional one
        // from its direction
        if e.flags & light_flags::AFFECT_STATIC != 0 {
            let soft = e
                .fuzzy
                .as_ref()
                .filter(|_| e.kind == LT_POINT && e.flags & light_flags::POINT_FUZZY != 0)
                .map(|f| f.points.as_slice())
                .filter(|p| !p.is_empty());
            match soft {
                Some(points) => {
                    let d = light.diffuse;
                    let mag = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    let k = if mag > 0.0 { mag / points.len() as f32 / mag } else { 0.0 };
                    let color = [d[0] * k, d[1] * k, d[2] * k, d[3]];
                    let xf = o.world();
                    for p in points {
                        self.lights_static.push(BuildLight {
                            controller,
                            light: Light { diffuse: color, position: xf.transform_tiny(*p), ..light },
                        });
                    }
                }
                None => self.lights_static.push(BuildLight { controller, light }),
            }
        }
        // what shines in the game from the level's own file is a point light: the game loads
        // every light of that file as one, and takes a directional one for the level's sun. A
        // spot light that shines in the game is the level things' (`gz-export-level`'s
        // `things`), a directional one is the scene's sun setting and nothing else.
        if e.flags & light_flags::AFFECT_DYNAMIC != 0 && e.kind == LT_POINT {
            let mut s = [0u16; 16];
            for (slot, v) in s.iter_mut().zip(&dyn_sectors) {
                *slot = *v;
            }
            self.lights_dynamic.push(BuildLightDynamic {
                light: BuildLight { controller, light },
                sectors: s,
                sector_count: dyn_sectors.len() as u32,
            });
        }
        Ok(())
    }

    /// `BuildGlow`.
    fn glow(&mut self, cx: &Ctx<'_>, o: &Object, sectors: &Sectors) -> Result<()> {
        let ObjData::Glow(g) = &o.data else { return Ok(()) };
        let fail = || BuildError::new(format!("The object '{}' could not be built.", o.name));
        let surface = self.texture(&g.texture).map_err(|_| fail())?;
        let shader = self.shader(cx, &g.shader).map_err(|_| fail())?;
        let sector = sectors.at(o.position(), g.radius);
        let material = self.find_material(BuildMaterial {
            surface,
            shader,
            shader_xrlc: u16::MAX,
            sector,
            reserved: 0,
            internal_max_area: 0,
        });
        self.glows.push(BuildGlow {
            pos: o.position(),
            size: g.radius,
            flags: u32::from(g.flags & GLOW_FIXED_SIZE != 0),
            material,
        });
        Ok(())
    }
}

/// The scene objects' and the terrains' box.
#[must_use]
pub fn level_box(scene: &SceneState, lib: Lookup<'_>) -> Option<Aabb> {
    let mut b = Aabb::INVALID;
    for o in scene.of_class(ObjClass::SceneObject) {
        if let Some(lo) = reference(o).and_then(lib) {
            let wb = xform_box(&lo.bounds, &o.world());
            if wb.is_valid() {
                b.modify(wb.min);
                b.modify(wb.max);
            }
        }
    }
    terrains_box(scene, lib, &mut b);
    b.is_valid().then_some(b)
}

/// The static geometry of the whole scene. `lods` gives a LOD picture by texture name.
pub fn compile(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    lods: LodSource<'_>,
) -> Result<Compiled> {
    compile_until(scene, lib, engine, lods, crate::NEVER)
}

/// [`compile`] that ends with [`BuildError::stopped`] once `stop` says so: asked per object.
pub fn compile_until(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    lods: LodSource<'_>,
    stop: crate::Stop<'_>,
) -> Result<Compiled> {
    build(scene, lib, engine, lods, stop, false)
}

/// The build project of a module's piece of a level of the game: the static objects the module
/// places (marked authored) are its geometry; every other static object of the scene that is not
/// marked removed is occluders, the faces the piece's light meets; the scene's lights light it.
/// No glow, portal, wallmark or LOD: a piece has none.
pub fn compile_piece_until(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    stop: crate::Stop<'_>,
) -> Result<Compiled> {
    build(scene, lib, engine, &|_| None, stop, true)
}

/// The authored scene objects' and the terrains' box: what a piece is made of. On a level of the
/// game a terrain is always the module's own.
fn authored_box(scene: &SceneState, lib: Lookup<'_>) -> Option<Aabb> {
    let mut b = Aabb::INVALID;
    for o in scene.of_class(ObjClass::SceneObject).filter(|o| o.marks.authored) {
        if let Some(lo) = reference(o).and_then(lib) {
            let wb = xform_box(&lo.bounds, &o.world());
            if wb.is_valid() {
                b.modify(wb.min);
                b.modify(wb.max);
            }
        }
    }
    terrains_box(scene, lib, &mut b);
    for p in own_mark_points(scene) {
        b.modify(p);
    }
    b.is_valid().then_some(b)
}

/// The corners of the wallmarks a scene over a level of the game paints: the module's own, part
/// of its piece.
fn own_mark_points(scene: &SceneState) -> Vec<Vec3> {
    let Some(part) = scene.bulk.wallmarks.as_ref().filter(|_| scene.meta.base.is_set()) else {
        return Vec::new();
    };
    crate::wallmarks::read(&part.raw)
        .iter()
        .flat_map(|slot| slot.marks.iter().flat_map(|m| m.verts.iter().map(|(p, _, _)| *p)))
        .collect()
}

/// Whether a scene over a level of the game paints wallmarks of its own.
#[must_use]
pub fn has_own_marks(scene: &SceneState) -> bool {
    // asked for every status: the first mark answers, no list of corners is made
    scene
        .bulk
        .wallmarks
        .as_ref()
        .filter(|_| scene.meta.base.is_set())
        .is_some_and(|part| crate::wallmarks::read(&part.raw).iter().any(|slot| !slot.marks.is_empty()))
}

fn build(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    lods: LodSource<'_>,
    stop: crate::Stop<'_>,
    piece: bool,
) -> Result<Compiled> {
    let level_box = if piece {
        authored_box(scene, lib).ok_or_else(|| {
            BuildError::new("The scene has no static object, terrain or wallmark of the module's own.")
        })?
    } else {
        level_box(scene, lib)
            .ok_or_else(|| BuildError::new("The level's bounding volume cannot be computed."))?
    };
    if scene.objects.is_empty() {
        return Err(BuildError::new("The static geometry could not be built."));
    }
    let opts = &scene.meta.level;
    let cx = Ctx { engine, quality: opts.build.quality, lods };
    let mut b = Builder::default();
    let mut stat = SceneStat::new(&level_box);
    let sectors = Sectors::new(scene, lib);
    let lt = scene.tools.part(ObjClass::Light).light().cloned().unwrap_or_default();

    // the hemisphere, then the sun
    // the X-Ray SDK takes the hemisphere's control from the light tool's setting,
    // which the scene file does not keep; it is `$hemi` here, as on every scene the tool made.
    b.hemi(opts.light_hemi_quality, LCONTROL_HEMI);
    b.sun(opts.light_sun_quality, lt.sun_shadow_dir)?;

    // the tools in class order: glows, scene objects, lights, portals
    for class in ObjClass::ALL {
        for o in scene.of_class(class) {
            if stop() {
                return Err(BuildError::stopped());
            }
            match class {
                ObjClass::Glow | ObjClass::Portal if piece => {}
                ObjClass::SceneObject if piece && !o.marks.authored => {
                    // the rest of the level: the piece's shadows, and nothing else
                    let Some(m) = model_of(scene, o, lib) else { continue };
                    let lo = &m.lo;
                    if o.marks.removed || !(is_static(&lo.object) || is_mu_static(&lo.object)) {
                        continue;
                    }
                    b.uncut(o, &m);
                    let surfaces = object_surfaces(o, lo);
                    for mesh in &lo.object.meshes {
                        b.occluders(&cx, &m.world, &surfaces, mesh)?;
                    }
                }
                ObjClass::Terrain => {
                    // level geometry like a static object; on a level of the game the module's own
                    let Some(m) = model_of(scene, o, lib) else {
                        let why = terrain::terrain(o).and_then(|t| t.check().err()).unwrap_or_else(|| {
                            "its mesh could not be made over the ground under it".to_owned()
                        });
                        return Err(BuildError::new(format!(
                            "The terrain '{}' cannot be built: {why}.",
                            o.name
                        )));
                    };
                    b.keep_light = piece && wears_the_ground(o);
                    let r = b.static_object(&cx, o, &m.lo, &m.world, &sectors, &mut stat);
                    b.keep_light = false;
                    r?;
                }
                ObjClass::Glow => b.glow(&cx, o, &sectors)?,
                ObjClass::Light => b.light(o, &sectors, scene)?,
                ObjClass::Portal => {
                    if let ObjData::Portal(p) = &o.data {
                        let num = |name: &str| {
                            sectors.list.iter().find(|v| v.name == name).map_or(sectors.default, |v| v.num)
                        };
                        // a portal writes its simplified vertices, which the load's update made
                        let simple =
                            gz_scene::objtools::portal::update(p.clone(), &o.name, None, 0, &mut Vec::new());
                        b.portals.push(Portal::new(num(&p.front), num(&p.back), &simple.simplified));
                    }
                }
                ObjClass::SceneObject => {
                    let Some(made) = model_of(scene, o, lib) else { continue };
                    b.uncut(o, &made);
                    let lo = made.lo.clone();
                    let surfaces = object_surfaces(o, &lo);
                    let world = made.world;
                    if is_static(&lo.object) {
                        b.static_object(&cx, o, &lo, &world, &sectors, &mut stat)?;
                    } else if is_mu_static(&lo.object) {
                        let first = lo.object.meshes.first().map_or(String::new(), |m| m.name.to_string());
                        let sector = sectors.of_mesh(&o.name, &first);
                        let model = match b.mu_models.iter().position(|m| m.name == lo.name) {
                            Some(i) => i,
                            None => {
                                // a piece's model draws itself: the level's LOD atlas is not the piece's;
                                // a cut model's LOD is its object's
                                let lod_id = if piece {
                                    u16::MAX
                                } else {
                                    b.lod(&cx, &made.base, sector)?.unwrap_or(u16::MAX)
                                };
                                let mut out = (Vec::new(), Vec::new(), Vec::new());
                                for m in &lo.object.meshes {
                                    b.mesh(
                                        &cx,
                                        &Mat4::IDENTITY,
                                        &world,
                                        o,
                                        &lo,
                                        &surfaces,
                                        m,
                                        sector,
                                        &mut out,
                                    )
                                    .map_err(|e| {
                                        BuildError::new(format!(
                                            "{}\nThe object '{}' could not be built.",
                                            e.0, o.name
                                        ))
                                    })?;
                                }
                                b.mu_models.push(MuModel {
                                    name: lo.name.clone(),
                                    verts: out.0,
                                    faces: out.1,
                                    lod_id,
                                    smooth_groups: out.2,
                                });
                                b.mu_models.len() - 1
                            }
                        };
                        b.mu_refs.push(MuReference {
                            model: model as u32,
                            transform: world.0,
                            flags: 0,
                            sector,
                            reserved: [0; 8],
                        });
                        if let Some(m) = b.mu_models.get(model) {
                            for v in &m.verts {
                                stat.add_muvert(world.transform_tiny(*v));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        // the Wallmark tool's marks: faces of their own material, after the objects
        // the X-Ray SDK leaves a mark face's game material and smoothing group as the
        // allocation held them; they are 0 here.
        // In a piece: the marks of a scene that lies over a level of the game are the module's
        // own - the level's are in the game's level - and ship as faces of the piece. A scene
        // that is the whole level keeps them the level build's.
        if class == ObjClass::Wallmark
            && (!piece || scene.meta.base.is_set())
            && let Some(part) = &scene.bulk.wallmarks
        {
            for slot in crate::wallmarks::read(&part.raw) {
                for mark in &slot.marks {
                    let sector = sectors.at(mark.center, mark.radius);
                    let m_id = b
                        .material_of(
                            &cx,
                            &slot.shader,
                            crate::wallmarks::COMPILER_SHADER,
                            &slot.texture,
                            sector,
                            false,
                        )
                        .map_or(u16::MAX, |m| m as u16);
                    for tri in mark.verts.as_chunks::<3>().0 {
                        let base = b.verts.len() as u32;
                        let mut f = BuildFace { material: m_id, ..BuildFace::default() };
                        for (k, (p, _, t)) in tri.iter().enumerate() {
                            b.verts.push(*p);
                            f.v[k] = base + k as u32;
                            f.t[k] = *t;
                        }
                        b.faces.push(f);
                        b.smgroups.push(0);
                    }
                }
            }
        }
    }

    if stop() {
        return Err(BuildError::stopped());
    }
    // the LOD pictures merged into one texture, their quads' coordinates moved into it
    let mut lod_textures = None;
    if !b.lods.is_empty() {
        let images: Vec<SourceImage> = b
            .lods
            .iter()
            .map(|(_, name, c, n)| SourceImage {
                name: name.clone(),
                w: LOD_IMAGE_SIZE * LOD_SAMPLE_COUNT,
                h: LOD_IMAGE_SIZE,
                layers: vec![c.clone(), n.clone()],
            })
            .collect();
        let Some(merged) = merge::merge_textures(2, images, 512, 4096, 64, 4096) else {
            return Err(BuildError::new("The merged LOD texture would be larger than 4096x4096."));
        };
        for (k, (lod, ..)) in b.lods.iter_mut().enumerate() {
            let id = merged.remap[k];
            for f in &mut lod.faces {
                for t in &mut f.t {
                    *t = merge::remap_uv(*t, merged.offsets[id], merged.scales[id], merged.rotated[id]);
                }
            }
        }
        let color = merge::game_texture(&merged.layers[0], merged.w, merged.h)?;
        let normal = merge::game_texture(&merged.layers[1], merged.w, merged.h)?;
        lod_textures = Some((color, normal));
    }

    if !b.twins.is_empty() {
        let said: Vec<String> = b.twins.iter().map(|(was, now)| format!("{was} -> {now}")).collect();
        b.log.info(format!(
            "Static surfaces that name a shader for models are compiled with the shader for level geometry of the same look: {}.",
            said.join(", ")
        ));
    }
    let p = &opts.build;
    let project = BuildProject {
        version: XRCL_CURRENT_VERSION,
        params: BuildParams {
            sm_angle: p.sm_angle,
            weld_distance: p.weld_distance,
            lm_pixels_per_meter: p.lm_pixels_per_meter,
            lm_jitter_samples: p.lm_jitter_samples,
            lm_rms_zero: p.lm_rms_zero,
            lm_rms: p.lm_rms,
            quality: p.quality,
            reserved: p.u_reserved,
            f_reserved: p.f_reserved,
        },
        verts: b.verts.into(),
        faces: b.faces.into(),
        smooth_groups: b.smgroups.into(),
        materials: b.materials,
        shaders: b.shaders.iter().map(|s| s.as_str().into()).collect(),
        shaders_xrlc: b.shaders_xrlc.iter().map(|s| s.as_str().into()).collect(),
        textures: b
            .textures
            .iter()
            .map(|t| BuildTexture { name: t.as_str().into(), width: 0, height: 0, has_alpha: 0, reserve: 0 })
            .collect(),
        glows: b.glows,
        portals: b.portals,
        light_controls: b
            .controls
            .iter()
            .map(|c| LightControl { name: c.as_str().into(), data: Vec::new() })
            .collect(),
        lights_static: b.lights_static,
        lights_dynamic: b.lights_dynamic,
        lods: b.lods.into_iter().map(|(l, ..)| l).collect(),
        mu_models: b.mu_models,
        mu_refs: b.mu_refs,
        occluders: b.occluders,
    };
    Ok(Compiled { project, stat, lod_textures, log: b.log })
}

/// `_splitpath(name, 0, dir, fname, 0)`: the folder (with its last backslash) and the name
/// without its extension.
fn split_tex(name: &str) -> (&str, &str) {
    let cut = name.rfind(['\\', '/']).map_or(0, |i| i + 1);
    let (dir, file) = name.split_at(cut);
    let stem = file.rfind('.').map_or(file, |i| &file[..i]);
    (dir, stem)
}

/// The text and a line end.
fn w_line(out: &mut String, s: &str) {
    out.push_str(s);
    out.push_str("\r\n");
}

/// `%f`.
fn f6(v: f32) -> String {
    format!("{:.6}", f64::from(v))
}

/// What Scene > Export ... as OBJ writes: the `.obj` text and its `.mtl`.
#[derive(Clone, Debug)]
pub struct ObjExport {
    pub obj: String,
    pub mtl: String,
    pub objects: usize,
}

/// Builds the scene into world space for an OBJ export - the static and
/// multiple-usage scene objects (the selected ones with `selected_only`) built into world space
/// with Z turned round, every multiple-usage object a model of its own - then
/// saved as an OBJ file. `obj_name` is the `.obj` file's stem (its `mtllib` line).
pub fn export_obj(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    selected_only: bool,
    obj_name: &str,
) -> Result<ObjExport> {
    let no_lods = |_: &str| None;
    let cx = Ctx { engine, quality: scene.meta.level.build.quality, lods: &no_lods };
    let mut b = Builder::default();
    let sectors = Sectors::new(scene, lib);
    let mut cv = Mat4::IDENTITY;
    cv.0[2][2] = -1.0;
    let mut objects = 0;
    for o in scene.of_class(ObjClass::SceneObject) {
        if selected_only && !scene.is_selected(o.id) {
            continue;
        }
        let Some(lo) = reference(o).and_then(lib) else { continue };
        let surfaces = object_surfaces(o, &lo);
        let world = o.world();
        let tm = Mat4::mul(&cv, &Mat4::mul(&Mat4::mul(&cv, &world), &cv));
        let fail =
            |e: BuildError| BuildError::new(format!("{}\nThe object '{}' could not be built.", e.0, o.name));
        if is_static(&lo.object) {
            let mut out =
                (std::mem::take(&mut b.verts), std::mem::take(&mut b.faces), std::mem::take(&mut b.smgroups));
            for m in &lo.object.meshes {
                let sector = sectors.of_mesh(&o.name, &m.name);
                b.mesh(&cx, &tm, &world, o, &lo, &surfaces, m, sector, &mut out).map_err(fail)?;
            }
            (b.verts, b.faces, b.smgroups) = out;
            objects += 1;
        } else if is_mu_static(&lo.object) {
            let first = lo.object.meshes.first().map_or(String::new(), |m| m.name.to_string());
            let sector = sectors.of_mesh(&o.name, &first);
            let mut out = (Vec::new(), Vec::new(), Vec::new());
            for m in &lo.object.meshes {
                b.mesh(&cx, &tm, &world, o, &lo, &surfaces, m, sector, &mut out).map_err(fail)?;
            }
            b.mu_models.push(MuModel {
                name: lo.name.clone(),
                verts: out.0,
                faces: out.1,
                lod_id: u16::MAX,
                smooth_groups: out.2,
            });
            objects += 1;
        }
    }
    // SaveBuildAsObject
    let texture_of = |material: u16| -> &str {
        b.materials
            .get(usize::from(material))
            .and_then(|m| b.textures.get(usize::from(m.surface)))
            .map_or("", String::as_str)
    };
    let mut mtl = String::new();
    for m in &b.materials {
        let name = b.textures.get(usize::from(m.surface)).map_or("", String::as_str);
        let (dir, stem) = split_tex(name);
        w_line(&mut mtl, &format!("newmtl {stem}"));
        w_line(&mut mtl, "Ka  0 0 0");
        w_line(&mut mtl, "Kd  1 1 1");
        w_line(&mut mtl, "Ks  0 0 0");
        // the X-Ray SDK's format ends in its own "\n" before the line end
        w_line(&mut mtl, &format!("map_Kd T:\\\\{dir}\\{stem}.tga\n"));
    }
    let mut obj = String::new();
    let mut faces = String::new();
    w_line(&mut obj, "# This file uses meters as units for non-parametric coordinates.");
    w_line(&mut obj, &format!("mtllib {obj_name}.mtl"));
    w_line(&mut obj, "g default");
    let v = |out: &mut String, p: &Vec3| {
        w_line(out, &format!("v {} {} {}", f6(p.x * 100.0), f6(p.y * 100.0), f6(p.z * 100.0)));
    };
    let vt = |out: &mut String, f: &BuildFace| {
        for t in &f.t {
            w_line(out, &format!("vt {} {}", f6(t.x), f6(1.0 - t.y)));
        }
    };
    for p in &b.verts {
        v(&mut obj, p);
    }
    for f in &b.faces {
        vt(&mut faces, f);
    }
    let mut total_tcs = b.faces.len() * 3;
    let mut last: Option<&str> = None;
    for (idx, f) in b.faces.iter().enumerate() {
        let t = texture_of(f.material);
        if last != Some(t) {
            w_line(&mut faces, &format!("usemtl {}", split_tex(t).1));
            last = Some(t);
        }
        w_line(
            &mut faces,
            &format!(
                "f {}/{} {}/{} {}/{}",
                f.v[0] + 1,
                idx * 3 + 1,
                f.v[1] + 1,
                idx * 3 + 2,
                f.v[2] + 1,
                idx * 3 + 3
            ),
        );
    }
    let mut total_vertices = b.verts.len();
    for m in &b.mu_models {
        for p in &m.verts {
            v(&mut obj, p);
        }
        for f in &m.faces {
            vt(&mut faces, f);
        }
        for (fi, f) in m.faces.iter().enumerate() {
            let t = texture_of(f.material);
            if last != Some(t) {
                w_line(&mut faces, &format!("usemtl {}", split_tex(t).1));
                last = Some(t);
            }
            w_line(
                &mut faces,
                &format!(
                    "f {}/{} {}/{} {}/{}",
                    f.v[0] as usize + 1 + total_vertices,
                    fi * 3 + 1 + total_tcs,
                    f.v[1] as usize + 1 + total_vertices,
                    fi * 3 + 2 + total_tcs,
                    f.v[2] as usize + 1 + total_vertices,
                    fi * 3 + 3 + total_tcs
                ),
            );
        }
        total_tcs += m.faces.len() * 3;
        total_vertices += m.verts.len();
    }
    obj.push_str(&faces);
    Ok(ObjExport { obj, mtl, objects })
}

#[cfg(test)]
mod obj_tests {
    use super::split_tex;

    #[test]
    fn a_texture_name_splits_like_splitpath() {
        assert_eq!(split_tex("ston\\ston_wall1"), ("ston\\", "ston_wall1"));
        assert_eq!(split_tex("plain"), ("", "plain"));
    }
}

/// A scene of a flat floor of the level (100 x 100 m at y = 0, `ground\flat`) and a terrain of
/// 10 x 10 m over its middle (`Yard`, flattened to 2 m in its middle), with a library that makes
/// the terrain's mesh and the cut floor as the editor's providers do.
#[cfg(test)]
#[allow(clippy::unwrap_used)]
pub(crate) mod terrain_fixture {
    use gz_scene::library::Library;
    use gz_scene::objects::SceneObjectData;
    use gz_terrain::{Base, Edit, Look, Looks, Spec, Terrain};

    use super::*;

    pub(crate) fn ground_model() -> gz_fmt_object::Object {
        let t = Terrain {
            centre: Vec3::ZERO,
            heading: 0.0,
            spec: Spec { size: Vec2::splat(100.0), cell: 4.0, base: Base::Height(0.0), ..Spec::default() },
        };
        let look =
            Look { texture: "ground\\grass".into(), material: "materials\\wood".into(), ..Look::default() };
        t.mesh(&t.heights(None).unwrap(), Looks::Table(&look))
    }

    pub(crate) fn scene(cut: bool) -> SceneState {
        let mut s = SceneState::default();
        s.insert(Object::new(
            ObjClass::SceneObject,
            "Floor",
            ObjData::SceneObject(SceneObjectData::library("ground\\flat")),
        ));
        s.insert(Object::new(
            ObjClass::Terrain,
            "Yard",
            ObjData::Terrain(Box::new(Spec {
                size: Vec2::splat(10.0),
                cell: 1.0,
                base: Base::Ground,
                edge: 0.0,
                edits: vec![Edit::Flatten { at: Vec2::ZERO, radius: 2.0, falloff: 1.0, height: 2.0 }],
                cut,
                ..Spec::default()
            })),
        ));
        s
    }

    pub(crate) fn library(state: &SceneState) -> Arc<Library> {
        let lib = Arc::new(Library::new(Vec::new()));
        let ground = Arc::new(LibraryObject::provided("ground\\flat", ground_model()));
        lib.add_provider(Arc::new(move |k: &str| (k == "ground\\flat").then(|| ground.clone())));
        lib.prefetch(&["ground\\flat".into()]);
        let (weak, s) = (Arc::downgrade(&lib), state.clone());
        lib.add_provider(Arc::new(move |k: &str| {
            let lib = weak.upgrade()?;
            if let Some(name) = k.strip_prefix(terrain::TERRAIN_PREFIX) {
                let o = s.find(ObjClass::Terrain, name)?;
                let made = terrain::make(&s, &lib, o).ok()?;
                return Some(Arc::new(LibraryObject::provided(k, made.mesh())));
            }
            let name = k.strip_prefix(terrain::CUT_PREFIX)?;
            let o = s.find(ObjClass::SceneObject, name)?;
            let regions: Vec<_> = terrain::cuts(&s, &lib).into_iter().map(|c| c.region).collect();
            let ground = lib.cached("ground\\flat")?;
            let cut = terrain::cut_model(&ground.object, &o.world(), &regions)?;
            Some(Arc::new(LibraryObject::provided(k, cut)))
        }));
        lib.prefetch(&terrain::references(state, &lib));
        lib
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod piece_tests {
    use std::path::PathBuf;

    use gz_fmt_object::{Face as MFace, FaceVert, Object as EditObject, SurfFaces};
    use gz_fmt_xr::{Blender, GameMaterial, GameMaterials, ShadersXrlc};
    use gz_scene::objects::SceneObjectData;

    use super::*;

    /// The engine's libraries with one shader, one compiler shader that casts shadows and one
    /// game material, and a library of one box made of them.
    fn fixture() -> (Engine, impl Fn(&str) -> Option<Arc<LibraryObject>> + Sync) {
        let corners: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 != 0 { 1.0 } else { -1.0 },
                    if i & 2 != 0 { 2.0 } else { 0.0 },
                    if i & 4 != 0 { 1.0 } else { -1.0 },
                )
            })
            .collect();
        let quads = [[0u32, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]];
        let fv = |p: u32| FaceVert { point: p, vmref: 0 };
        let faces: Vec<MFace> = quads
            .iter()
            .flat_map(|q| {
                [MFace { v: [fv(q[0]), fv(q[1]), fv(q[2])] }, MFace { v: [fv(q[0]), fv(q[2]), fv(q[3])] }]
            })
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
                shader: "default".into(),
                shader_xrlc: "default".into(),
                game_mtl: "materials\\wood".into(),
                fvf: 0x112,
                ..Surface::default()
            }],
            meshes: vec![mesh],
            ..EditObject::default()
        };
        let lo = Arc::new(LibraryObject::from_object("props\\box", PathBuf::new(), object));
        let wood = GameMaterial {
            id: 1,
            name: "materials\\wood".into(),
            desc: "".into(),
            flags: 0,
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
        let mut xrlc = gz_lc_light::default_shader();
        xrlc.name_raw = [0; 128];
        xrlc.name_raw[..7].copy_from_slice(b"default");
        let blender = Blender {
            class_id: *b"BLENDER ",
            name: "default".into(),
            computer: "".into(),
            time: 0,
            version: 0,
            props: Vec::new(),
        };
        let engine = Engine {
            shaders: Some(gz_fmt_xr::Shaders { blenders: vec![blender], ..gz_fmt_xr::Shaders::default() }),
            xrlc: Some(ShadersXrlc { shaders: vec![xrlc] }),
            materials: Some(GameMaterials { materials: vec![wood], ..GameMaterials::default() }),
        };
        (engine, move |_: &str| Some(lo.clone()))
    }

    fn scene() -> SceneState {
        let mut scene = SceneState::default();
        // two boxes of the module's own, one of the game's level, one the module took away
        for (name, x, authored, removed) in [
            ("mine_a", 0.0f32, true, false),
            ("mine_b", 4.0, true, false),
            ("base", 10.0, false, false),
            ("gone", 20.0, false, true),
        ] {
            let mut o = Object::new(
                ObjClass::SceneObject,
                name,
                ObjData::SceneObject(SceneObjectData::library("props\\box")),
            );
            o.xform.position = Vec3::new(x, 0.0, 0.0);
            o.marks.authored = authored;
            o.marks.removed = removed;
            scene.insert(o);
        }
        scene
    }

    #[test]
    fn a_terrain_is_level_geometry_and_the_ground_it_cuts_is_its_cut_model() {
        let (engine, _) = fixture();
        let s = terrain_fixture::scene(true);
        let lib = terrain_fixture::library(&s);
        let look = crate::files::lookup(lib.clone());
        let inside = |p: &BuildProject, f: &BuildFace| {
            let c = (p.verts[f.v[0] as usize] + p.verts[f.v[1] as usize] + p.verts[f.v[2] as usize]) / 3.0;
            c.x.abs() < 4.9 && c.z.abs() < 4.9
        };
        // the level of the module's own: the terrain's 10 x 10 cells and the floor without them
        let whole = compile_until(&s, &look, &engine, &|_| None, crate::NEVER).unwrap();
        let p = &whole.project;
        assert_eq!(p.faces.iter().filter(|f| inside(p, f)).count(), 200);
        let high = p.faces.iter().filter(|f| (0..3).all(|k| p.verts[f.v[k] as usize].y > 1.9)).count();
        assert!(high > 0, "the terrain's flattened middle is built");
        assert!(p.materials.iter().all(|m| m.internal_max_area == 0), "lit by the normal path");
        let lb = level_box(&s, &look).unwrap();
        assert!(lb.max.y > 1.9 && lb.min.x < -49.0);
        // without the cut, the floor under the terrain is built too
        let s2 = terrain_fixture::scene(false);
        let lib2 = terrain_fixture::library(&s2);
        let look2 = crate::files::lookup(lib2.clone());
        let p2 = compile_until(&s2, &look2, &engine, &|_| None, crate::NEVER).unwrap().project;
        assert!(p2.faces.iter().filter(|f| inside(&p2, f)).count() > 200);

        // a level of the game: the terrain is the piece whatever its marks, the cut floor its
        // shadows, and its faces of the ground's look keep the level's own light
        let piece = compile_piece_until(&s, &look, &engine, crate::NEVER).unwrap().project;
        assert_eq!(piece.faces.iter().filter(|f| inside(&piece, f)).count(), 200);
        assert!(!piece.occluders.is_empty());
        assert!(piece.occluders.iter().all(|o| {
            let c = (o.p[0] + o.p[1] + o.p[2]) / 3.0;
            !(c.x.abs() < 4.9 && c.z.abs() < 4.9)
        }));
        assert!(piece.faces.iter().all(|f| piece.materials[usize::from(f.material)].internal_max_area
            & gz_lc_light::KEEPS_LEVEL_LIGHT
            != 0));
    }

    #[test]
    fn a_piece_is_the_modules_objects_lit_beside_the_rest_as_occluders() {
        let (engine, look) = fixture();
        let s = scene();
        let piece = compile_piece_until(&s, &look, &engine, crate::NEVER).unwrap().project;
        // the authored boxes are the geometry, the game's box is occluders, the removed one nothing
        assert_eq!(piece.faces.len(), 24);
        assert_eq!(piece.occluders.len(), 12);
        assert!(piece.occluders.iter().all(|o| o.p.iter().all(|p| (9.0..=11.0).contains(&p.x))));
        assert_eq!(piece.textures[usize::from(piece.occluders[0].texture)].name, "wood\\a");
        assert!(piece.portals.is_empty() && piece.glows.is_empty() && piece.lods.is_empty());
        assert!(!piece.lights_static.is_empty(), "the sky and the sun light the piece");
        // the file reads back with its occluders
        assert_eq!(BuildProject::read(&piece.write()).unwrap().occluders, piece.occluders);
        // the level's own build: every box geometry, no occluder
        let whole = compile_until(&s, &look, &engine, &|_| None, crate::NEVER).unwrap().project;
        assert_eq!((whole.faces.len(), whole.occluders.len()), (48, 0));
        // nothing of the module's own: no piece
        let mut none = s.clone();
        let ids: Vec<_> = none.of_class(ObjClass::SceneObject).map(|o| o.id).collect();
        for id in ids {
            let mut o = (**none.get(id).unwrap()).clone();
            o.marks.authored = false;
            none.replace(o);
        }
        assert!(compile_piece_until(&none, &look, &engine, crate::NEVER).is_err());
    }

    /// Every kind of light is baked; of the lights that shine in the game the level's own file
    /// carries the point lights alone.
    #[test]
    fn every_kind_of_light_is_baked_and_only_point_lights_shine_from_the_levels_own_file() {
        use gz_scene::objects::{LT_DIRECT, LT_SPOT, LightData};
        let (engine, _) = fixture();
        let mut s = terrain_fixture::scene(false);
        let both = light_flags::AFFECT_STATIC | light_flags::AFFECT_DYNAMIC;
        for (name, kind, cone) in
            [("lamp", LT_POINT, 0.0f32), ("beam", LT_SPOT, 1.0), ("moon", LT_DIRECT, 0.0)]
        {
            let d = LightData { kind, flags: both, cone, range: 9.0, ..LightData::default() };
            let mut o = Object::new(ObjClass::Light, name, ObjData::Light(d));
            o.xform.position = Vec3::new(1.0, 3.0, 1.0);
            s.insert(o);
        }
        let lib = terrain_fixture::library(&s);
        let look = crate::files::lookup(lib.clone());
        let built = compile_until(&s, &look, &engine, &|_| None, crate::NEVER).unwrap().project;
        let at = |l: &gz_fmt_level::Light| l.position == Vec3::new(1.0, 3.0, 1.0);
        // baked: the three of them, each of its own kind, the spot with its cone
        let mut baked: Vec<(u32, f32)> = built
            .lights_static
            .iter()
            .filter(|l| at(&l.light))
            .map(|l| (l.light.kind, l.light.phi))
            .collect();
        baked.sort_by_key(|l| l.0);
        assert_eq!(baked, [(LT_POINT, 0.0), (LT_SPOT, 1.0), (LT_DIRECT, 0.0)]);
        // in the game from the level's own file: the point light alone
        let shine: Vec<u32> =
            built.lights_dynamic.iter().filter(|l| at(&l.light.light)).map(|l| l.light.light.kind).collect();
        assert_eq!(shine, [LT_POINT]);
    }
}
