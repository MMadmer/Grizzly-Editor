//! The build project the level compiler reads (`build.prj`).

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_base::{Finding, Findings};
use gz_fmt_level::{
    Aabb, BuildFace, BuildGlow, BuildLight, BuildLightDynamic, BuildLod, BuildMaterial, BuildParams,
    BuildProject, BuildTexture, Light, LightControl, LodFace, MuModel, MuReference, Portal,
    XRCL_CURRENT_VERSION, i_floor,
};
use gz_fmt_object::{Mesh, ObjectFlags, Surface, SurfaceFlags, VMapKind};
use gz_scene::library::LibraryObject;
use gz_scene::math::{Mat4, xform_box};
use gz_scene::objects::{LT_POINT, SO_USE_SURFACE, light_flags};
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
        for (i, o) in scene.of_class(ObjClass::Sector).enumerate() {
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

    /// `PortalUtils.FindSector`.
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
/// shader, material, texture and vertex map where it keeps them (`m_Surfaces`).
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
    log: Vec<Finding>,
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
        }) {
            return i as u32;
        }
        self.materials.push(m);
        (self.materials.len() - 1) as u32
    }

    /// `BuildMaterial(surf, sector, allow_draft)`.
    fn material(&mut self, cx: &Ctx<'_>, s: &Surface, sector: u16, allow_draft: bool) -> Result<u32> {
        self.material_of(cx, &s.shader, &s.shader_xrlc, &s.texture, sector, allow_draft)
    }

    /// `BuildMaterial(esh_name, csh_name, tx_name, 1, sector, allow_draft)`.
    fn material_of(
        &mut self,
        cx: &Ctx<'_>,
        esh: &str,
        csh: &str,
        tex: &str,
        sector: u16,
        allow_draft: bool,
    ) -> Result<u32> {
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
            internal_max_area: 0,
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
        let mut dyn_sectors: Vec<u16> = Vec::new();
        if e.flags & light_flags::AFFECT_DYNAMIC != 0 {
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
        if e.kind != LT_POINT {
            return Err(BuildError::new("Invalid light type."));
        }
        if e.flags & light_flags::AFFECT_STATIC != 0 {
            let soft = e
                .fuzzy
                .as_ref()
                .filter(|_| e.flags & light_flags::POINT_FUZZY != 0)
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
        if e.flags & light_flags::AFFECT_DYNAMIC != 0 {
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

/// The scene objects' box.
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
    b.is_valid().then_some(b)
}

/// `CompileStatic(false)`: the whole scene. `lods` gives a LOD picture by texture name.
pub fn compile(
    scene: &SceneState,
    lib: Lookup<'_>,
    engine: &Engine,
    lods: LodSource<'_>,
) -> Result<Compiled> {
    let level_box = level_box(scene, lib)
        .ok_or_else(|| BuildError::new("The level's bounding volume cannot be computed."))?;
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
            match class {
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
                    let Some(lo) = reference(o).and_then(lib) else { continue };
                    let surfaces = object_surfaces(o, &lo);
                    let world = o.world();
                    if is_static(&lo.object) {
                        let mut out = (
                            std::mem::take(&mut b.verts),
                            std::mem::take(&mut b.faces),
                            std::mem::take(&mut b.smgroups),
                        );
                        for m in &lo.object.meshes {
                            let sector = sectors.of_mesh(&o.name, &m.name);
                            let r = b.mesh(&cx, &world, &world, o, &lo, &surfaces, m, sector, &mut out);
                            if let Err(e) = r {
                                b.log.error(e.0.clone());
                                return Err(BuildError::new(format!(
                                    "{}\nThe object '{}' could not be built.",
                                    e.0, o.name
                                )));
                            }
                            for p in m.points.iter() {
                                stat.add_svert(world.transform_tiny(*p));
                            }
                        }
                        (b.verts, b.faces, b.smgroups) = out;
                    } else if is_mu_static(&lo.object) {
                        let first = lo.object.meshes.first().map_or(String::new(), |m| m.name.to_string());
                        let sector = sectors.of_mesh(&o.name, &first);
                        let model = match b.mu_models.iter().position(|m| m.name == lo.name) {
                            Some(i) => i,
                            None => {
                                let lod_id = b.lod(&cx, &lo, sector)?.unwrap_or(u16::MAX);
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
        if class == ObjClass::Wallmark
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
