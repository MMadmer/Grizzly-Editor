//! `build.prj`: what the editor's level build hands to the compilers - the `b_*` records of
//! `communicate.h`, one chunk per kind.
//!
//! The records are C structs dumped as they lie in memory. Their padding bytes and what follows
//! a name's terminator are whatever the C++ had there; they are written as zeros here.

use std::sync::Arc;

use glam::{Vec2, Vec3};
use gz_chunk::{Reader, Writer};
use rayon::prelude::*;
use smol_str::SmolStr;

use crate::bytes::{
    f32_at, fits, fixed_str, push_fixed_str, push_vec3s, read_vec3s, u16_at, u32_at, vec3_at,
};
use crate::{LevelError, Result};

/// `XRCL_CURRENT_VERSION`: the version the build writes.
pub const XRCL_CURRENT_VERSION: u32 = 18;
/// `XR_MAX_PORTAL_VERTS`.
pub const MAX_PORTAL_VERTS: usize = 6;

/// Chunk ids of `build.prj` (`EBUILD_CHUNKS`).
pub mod chunk {
    pub const VERSION: u32 = 0;
    pub const PARAMETERS: u32 = 1;
    pub const VERTICES: u32 = 2;
    pub const FACES: u32 = 3;
    pub const MATERIALS: u32 = 4;
    pub const SHADERS_RENDER: u32 = 5;
    pub const SHADERS_COMPILE: u32 = 6;
    pub const TEXTURES: u32 = 7;
    pub const GLOWS: u32 = 8;
    pub const PORTALS: u32 = 9;
    pub const LIGHT_CONTROL: u32 = 10;
    pub const LIGHT_STATIC: u32 = 11;
    pub const LIGHT_DYNAMIC: u32 = 12;
    pub const LOD_MODELS: u32 = 13;
    pub const MU_MODELS: u32 = 14;
    pub const MU_REFS: u32 = 15;
    pub const SMOOTH_GROUPS: u32 = 16;
}

const PARAMS: usize = 52;
const FACE: usize = 44;
const MATERIAL: usize = 16;
const SHADER: usize = 128;
const TEXTURE: usize = 144;
const GLOW: usize = 24;
pub(crate) const PORTAL: usize = 80;
pub(crate) const LIGHT: usize = 108;
const LIGHT_DYNAMIC: usize = 144;
const LOD: usize = 644;
const MU_REF: usize = 108;

/// `b_params`: the build settings of a level.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BuildParams {
    pub sm_angle: f32,
    pub weld_distance: f32,
    pub lm_pixels_per_meter: f32,
    pub lm_jitter_samples: u32,
    pub lm_rms_zero: u32,
    pub lm_rms: u32,
    /// `EBuildQuality`: 0 draft, 1 high, 2 custom.
    pub quality: u16,
    pub reserved: u16,
    pub f_reserved: [f32; 6],
}

impl Default for BuildParams {
    /// `b_params::Init`: high quality.
    fn default() -> Self {
        Self {
            sm_angle: 75.0,
            weld_distance: 0.005,
            lm_pixels_per_meter: 10.0,
            lm_jitter_samples: 9,
            lm_rms_zero: 4,
            lm_rms: 4,
            quality: 1,
            reserved: 0,
            f_reserved: [0.0; 6],
        }
    }
}

impl BuildParams {
    fn read(b: &[u8]) -> Self {
        let mut f_reserved = [0.0f32; 6];
        for (i, f) in f_reserved.iter_mut().enumerate() {
            *f = f32_at(b, 28 + i * 4);
        }
        Self {
            sm_angle: f32_at(b, 0),
            weld_distance: f32_at(b, 4),
            lm_pixels_per_meter: f32_at(b, 8),
            lm_jitter_samples: u32_at(b, 12),
            lm_rms_zero: u32_at(b, 16),
            lm_rms: u32_at(b, 20),
            quality: u16_at(b, 24),
            reserved: u16_at(b, 26),
            f_reserved,
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_f32(self.sm_angle);
        w.w_f32(self.weld_distance);
        w.w_f32(self.lm_pixels_per_meter);
        w.w_u32(self.lm_jitter_samples);
        w.w_u32(self.lm_rms_zero);
        w.w_u32(self.lm_rms);
        w.w_u16(self.quality);
        w.w_u16(self.reserved);
        for f in self.f_reserved {
            w.w_f32(f);
        }
    }
}

/// `b_face`: a triangle with its texture coordinates and materials.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildFace {
    pub v: [u32; 3],
    pub t: [Vec2; 3],
    /// Index into the materials.
    pub material: u16,
    /// The game material's id.
    pub material_game: u32,
}

impl BuildFace {
    fn read(b: &[u8]) -> Self {
        let uv = |at: usize| Vec2::new(f32_at(b, at), f32_at(b, at + 4));
        Self {
            v: [u32_at(b, 0), u32_at(b, 4), u32_at(b, 8)],
            t: [uv(12), uv(20), uv(28)],
            material: u16_at(b, 36),
            material_game: u32_at(b, 40),
        }
    }

    fn write(&self, w: &mut Writer) {
        for v in self.v {
            w.w_u32(v);
        }
        for t in self.t {
            w.w_vec2(t);
        }
        w.w_u16(self.material);
        w.w_u16(0);
        w.w_u32(self.material_game);
    }
}

/// `b_material`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BuildMaterial {
    /// Index into the textures.
    pub surface: u16,
    /// Index into the render shaders.
    pub shader: u16,
    /// Index into the compiler shaders.
    pub shader_xrlc: u16,
    pub sector: u16,
    pub reserved: u16,
    pub internal_max_area: u32,
}

/// `b_texture_real`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BuildTexture {
    pub name: SmolStr,
    pub width: u32,
    pub height: u32,
    pub has_alpha: u32,
    pub reserve: u32,
}

/// `b_glow`.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildGlow {
    pub pos: Vec3,
    pub size: f32,
    /// 1 = not scalable.
    pub flags: u32,
    /// Index into the materials.
    pub material: u32,
}

/// `b_portal`: a polygon between two sectors.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Portal {
    pub sector_front: u16,
    pub sector_back: u16,
    /// All six slots as stored; only the first `count` mean anything.
    pub vertices: [Vec3; MAX_PORTAL_VERTS],
    pub count: u32,
}

impl Portal {
    /// A portal of up to six vertices; more are dropped.
    #[must_use]
    pub fn new(sector_front: u16, sector_back: u16, verts: &[Vec3]) -> Self {
        let mut vertices = [Vec3::ZERO; MAX_PORTAL_VERTS];
        let count = verts.len().min(MAX_PORTAL_VERTS);
        for (slot, v) in vertices.iter_mut().zip(verts) {
            *slot = *v;
        }
        Self { sector_front, sector_back, vertices, count: count as u32 }
    }

    /// The vertices that are in use.
    #[must_use]
    pub fn used(&self) -> &[Vec3] {
        self.vertices.get(..(self.count as usize).min(MAX_PORTAL_VERTS)).unwrap_or_default()
    }

    pub(crate) fn read(b: &[u8]) -> Self {
        let mut vertices = [Vec3::ZERO; MAX_PORTAL_VERTS];
        for (i, v) in vertices.iter_mut().enumerate() {
            *v = vec3_at(b, 4 + i * 12);
        }
        Self { sector_front: u16_at(b, 0), sector_back: u16_at(b, 2), vertices, count: u32_at(b, 76) }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_u16(self.sector_front);
        w.w_u16(self.sector_back);
        for v in self.vertices {
            w.w_vec3(v);
        }
        w.w_u32(self.count);
    }
}

/// `Flight`: a light as Direct3D 9 describes one.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Light {
    /// `D3DLIGHTTYPE`: 1 point, 2 spot, 3 directional.
    pub kind: u32,
    /// r, g, b, a.
    pub diffuse: [f32; 4],
    pub specular: [f32; 4],
    pub ambient: [f32; 4],
    pub position: Vec3,
    pub direction: Vec3,
    pub range: f32,
    pub falloff: f32,
    pub attenuation0: f32,
    pub attenuation1: f32,
    pub attenuation2: f32,
    pub theta: f32,
    pub phi: f32,
}

impl Light {
    fn read(b: &[u8]) -> Self {
        let color = |at: usize| [f32_at(b, at), f32_at(b, at + 4), f32_at(b, at + 8), f32_at(b, at + 12)];
        Self {
            kind: u32_at(b, 0),
            diffuse: color(4),
            specular: color(20),
            ambient: color(36),
            position: vec3_at(b, 52),
            direction: vec3_at(b, 64),
            range: f32_at(b, 76),
            falloff: f32_at(b, 80),
            attenuation0: f32_at(b, 84),
            attenuation1: f32_at(b, 88),
            attenuation2: f32_at(b, 92),
            theta: f32_at(b, 96),
            phi: f32_at(b, 100),
        }
    }

    fn write(&self, w: &mut Writer) {
        w.w_u32(self.kind);
        for c in [self.diffuse, self.specular, self.ambient] {
            for v in c {
                w.w_f32(v);
            }
        }
        w.w_vec3(self.position);
        w.w_vec3(self.direction);
        for v in [
            self.range,
            self.falloff,
            self.attenuation0,
            self.attenuation1,
            self.attenuation2,
            self.theta,
            self.phi,
        ] {
            w.w_f32(v);
        }
    }
}

/// `b_light`: a light on a controller ("layer"); 0 is the base layer.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildLight {
    pub controller: u32,
    pub light: Light,
}

impl BuildLight {
    pub(crate) fn read(b: &[u8]) -> Self {
        Self { controller: u32_at(b, 0), light: Light::read(b.get(4..).unwrap_or_default()) }
    }

    pub(crate) fn write(&self, w: &mut Writer) {
        w.w_u32(self.controller);
        self.light.write(w);
    }
}

/// `b_light_dynamic`: a light for dynamic models, with the sectors it reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildLightDynamic {
    pub light: BuildLight,
    /// All sixteen slots as stored; only the first `sector_count` mean anything.
    pub sectors: [u16; 16],
    pub sector_count: u32,
}

/// `b_light_control`: a named controller and its animation keys.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LightControl {
    /// Up to 63 bytes.
    pub name: SmolStr,
    pub data: Vec<u32>,
}

/// One quad of a LOD model.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct LodFace {
    pub v: [Vec3; 4],
    pub t: [Vec2; 4],
}

/// `b_lod`: the eight quads of a LOD model.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct BuildLod {
    pub faces: [LodFace; 8],
    /// Index into the materials.
    pub material: u32,
}

/// `b_mu_model`: a model that is placed many times.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MuModel {
    pub name: SmolStr,
    pub verts: Vec<Vec3>,
    pub faces: Vec<BuildFace>,
    /// Index into the LOD models; `u16::MAX` for none.
    pub lod_id: u16,
    /// One smoothing group word per face.
    pub smooth_groups: Vec<u32>,
}

/// `b_mu_reference`: one placement of a [`MuModel`].
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct MuReference {
    pub model: u32,
    /// The transform, row after row.
    pub transform: [[f32; 4]; 4],
    pub flags: u32,
    pub sector: u16,
    pub reserved: [u32; 8],
}

/// A whole `build.prj`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct BuildProject {
    pub version: u32,
    pub params: BuildParams,
    pub verts: Arc<[Vec3]>,
    pub faces: Arc<[BuildFace]>,
    /// One word per face. Empty in files older than version 18, which have no such chunk.
    pub smooth_groups: Arc<[u32]>,
    pub materials: Vec<BuildMaterial>,
    /// Render shader names (`b_shader`, up to 127 bytes).
    pub shaders: Vec<SmolStr>,
    /// Compiler shader names.
    pub shaders_xrlc: Vec<SmolStr>,
    pub textures: Vec<BuildTexture>,
    pub glows: Vec<BuildGlow>,
    pub portals: Vec<Portal>,
    pub light_controls: Vec<LightControl>,
    pub lights_static: Vec<BuildLight>,
    pub lights_dynamic: Vec<BuildLightDynamic>,
    pub lods: Vec<BuildLod>,
    pub mu_models: Vec<MuModel>,
    pub mu_refs: Vec<MuReference>,
}

/// What a chunk of the file holds, for a sentence about it, and the size of its records when it
/// is an array of them.
fn chunk_info(id: u32) -> Option<(&'static str, usize)> {
    Some(match id {
        chunk::VERSION => ("version", 4),
        chunk::PARAMETERS => ("parameters", 1),
        chunk::VERTICES => ("vertices", 12),
        chunk::FACES => ("faces", FACE),
        chunk::MATERIALS => ("materials", MATERIAL),
        chunk::SHADERS_RENDER => ("shaders", SHADER),
        chunk::SHADERS_COMPILE => ("compiler shaders", SHADER),
        chunk::TEXTURES => ("textures", TEXTURE),
        chunk::GLOWS => ("glows", GLOW),
        chunk::PORTALS => ("portals", PORTAL),
        chunk::LIGHT_CONTROL => ("light controllers", 1),
        chunk::LIGHT_STATIC => ("static lights", LIGHT),
        chunk::LIGHT_DYNAMIC => ("dynamic lights", LIGHT_DYNAMIC),
        chunk::LOD_MODELS => ("LOD models", LOD),
        chunk::MU_MODELS => ("multi-usage models", 1),
        chunk::MU_REFS => ("multi-usage references", MU_REF),
        chunk::SMOOTH_GROUPS => ("smoothing groups", 4),
        _ => return None,
    })
}

/// The chunks the C++ compiler cannot do without (`CBuild::Load` opens them unchecked).
const REQUIRED: [u32; 7] = [
    chunk::VERSION,
    chunk::PARAMETERS,
    chunk::VERTICES,
    chunk::FACES,
    chunk::TEXTURES,
    chunk::LIGHT_CONTROL,
    chunk::LIGHT_STATIC,
];

/// The file is chunks of `build.prj` end to end, each whole, the required ones there, every
/// array a whole number of records. The chunk reader stops at a header that does not fit, so
/// without this a file cut short would read as a build that has fewer chunks.
fn check_whole(bytes: &[u8]) -> Result<()> {
    let bad = |s: String| Err(LevelError::Invalid(s));
    if bytes.is_empty() {
        return bad("build.prj is empty.".into());
    }
    let mut at = 0usize;
    let mut seen = Vec::new();
    while at < bytes.len() {
        let Some(head) = bytes.get(at..at + 8) else {
            return bad(format!(
                "build.prj is cut short: it ends {} bytes into the header of a chunk.",
                bytes.len() - at
            ));
        };
        let id = u32_at(head, 0) & 0x7fff_ffff;
        let size = u32_at(head, 4) as usize;
        let Some((name, record)) = chunk_info(id) else {
            return bad(if at == 0 {
                "build.prj is not a build project: it does not start with a chunk of one.".into()
            } else {
                format!("build.prj is damaged: {} bytes into it stands no chunk of a build project.", at)
            });
        };
        let left = bytes.len() - at - 8;
        if size > left {
            return bad(format!(
                "build.prj is cut short: its {name} chunk holds {size} bytes and the file ends {left} bytes into it."
            ));
        }
        if !size.is_multiple_of(record) {
            return bad(format!(
                "build.prj is damaged: its {name} chunk holds {size} bytes, not a whole number of {record}-byte records."
            ));
        }
        seen.push(id);
        at += 8 + size;
    }
    if let Some(&missing) = REQUIRED.iter().find(|id| !seen.contains(id)) {
        let name = chunk_info(missing).map_or("", |c| c.0);
        return bad(format!("build.prj is incomplete: it has no {name} chunk."));
    }
    Ok(())
}

/// The records of a chunk whose body is an array of `size`-byte structs.
fn records<T>(r: &Reader<'_>, id: u32, size: usize, read: impl Fn(&[u8]) -> T) -> Result<Vec<T>> {
    let Some(chunk) = r.open_chunk(id)? else { return Ok(Vec::new()) };
    Ok(chunk.bytes().chunks_exact(size).map(read).collect())
}

fn read_faces(bytes: &[u8]) -> Vec<BuildFace> {
    bytes.par_chunks_exact(FACE).with_min_len(8192).map(BuildFace::read).collect()
}

fn read_mu_models(bytes: &[u8]) -> Result<Vec<MuModel>> {
    let mut r = Reader::new(bytes);
    let mut out = Vec::new();
    while !r.eof() {
        let name = SmolStr::new(r.r_stringz()?);
        let vert_count = r.r_u32()?;
        let nv = fits(r.elapsed(), u64::from(vert_count), 12, "multi-usage model vertices")?;
        let verts = read_vec3s(r.take(nv * 12)?);
        let face_count = r.r_u32()?;
        // the faces, the lod id and the smoothing groups after them
        let nf = fits(r.elapsed(), u64::from(face_count), FACE as u64 + 4, "multi-usage model faces")?;
        let faces = r.take(nf * FACE)?.as_chunks::<FACE>().0.iter().map(|c| BuildFace::read(c)).collect();
        let lod_id = r.r_u16()?;
        let smooth_groups = r.take(nf * 4)?.as_chunks::<4>().0.iter().map(|c| u32_at(c, 0)).collect();
        out.push(MuModel { name, verts, faces, lod_id, smooth_groups });
    }
    Ok(out)
}

impl BuildProject {
    /// The file; one cut short, damaged or lacking a chunk the compiler needs is refused with a
    /// sentence saying which.
    pub fn read(bytes: &[u8]) -> Result<Self> {
        check_whole(bytes)?;
        let r = Reader::new(bytes);
        let version = r.need_chunk(chunk::VERSION)?.reader().r_u32()?;
        if !(17..=XRCL_CURRENT_VERSION).contains(&version) {
            return Err(LevelError::Version { what: "build.prj", found: version });
        }
        let params = r.need_chunk(chunk::PARAMETERS)?;
        if params.bytes().len() < PARAMS {
            return Err(LevelError::Invalid("build.prj: the parameters chunk is too short".into()));
        }

        let mut out = Self { version, params: BuildParams::read(params.bytes()), ..Self::default() };
        if let Some(c) = r.open_chunk(chunk::VERTICES)? {
            out.verts = read_vec3s(c.bytes()).into();
        }
        if let Some(c) = r.open_chunk(chunk::FACES)? {
            out.faces = read_faces(c.bytes()).into();
        }
        if let Some(c) = r.open_chunk(chunk::SMOOTH_GROUPS)? {
            out.smooth_groups = c.bytes().as_chunks::<4>().0.iter().map(|b| u32_at(b, 0)).collect();
        }
        out.materials = records(&r, chunk::MATERIALS, MATERIAL, |b| BuildMaterial {
            surface: u16_at(b, 0),
            shader: u16_at(b, 2),
            shader_xrlc: u16_at(b, 4),
            sector: u16_at(b, 6),
            reserved: u16_at(b, 8),
            internal_max_area: u32_at(b, 12),
        })?;
        out.shaders = records(&r, chunk::SHADERS_RENDER, SHADER, |b| SmolStr::new(fixed_str(b)))?;
        out.shaders_xrlc = records(&r, chunk::SHADERS_COMPILE, SHADER, |b| SmolStr::new(fixed_str(b)))?;
        out.textures = records(&r, chunk::TEXTURES, TEXTURE, |b| BuildTexture {
            name: SmolStr::new(fixed_str(b.get(..128).unwrap_or_default())),
            width: u32_at(b, 128),
            height: u32_at(b, 132),
            has_alpha: u32_at(b, 136),
            reserve: u32_at(b, 140),
        })?;
        out.glows = records(&r, chunk::GLOWS, GLOW, |b| BuildGlow {
            pos: vec3_at(b, 0),
            size: f32_at(b, 12),
            flags: u32_at(b, 16),
            material: u32_at(b, 20),
        })?;
        out.portals = records(&r, chunk::PORTALS, PORTAL, Portal::read)?;
        if let Some(c) = r.open_chunk(chunk::LIGHT_CONTROL)? {
            let mut c = c.reader();
            while !c.eof() {
                let name = SmolStr::new(fixed_str(c.take(64)?));
                let count = c.r_u32()?;
                let n = fits(c.elapsed(), u64::from(count), 4, "light controller keys")?;
                let data = c.take(n * 4)?.as_chunks::<4>().0.iter().map(|b| u32_at(b, 0)).collect();
                out.light_controls.push(LightControl { name, data });
            }
        }
        out.lights_static = records(&r, chunk::LIGHT_STATIC, LIGHT, BuildLight::read)?;
        out.lights_dynamic = records(&r, chunk::LIGHT_DYNAMIC, LIGHT_DYNAMIC, |b| {
            let mut sectors = [0u16; 16];
            for (i, s) in sectors.iter_mut().enumerate() {
                *s = u16_at(b, LIGHT + i * 2);
            }
            BuildLightDynamic { light: BuildLight::read(b), sectors, sector_count: u32_at(b, LIGHT + 32) }
        })?;
        out.lods = records(&r, chunk::LOD_MODELS, LOD, |b| {
            let mut faces = [LodFace::default(); 8];
            for (i, face) in faces.iter_mut().enumerate() {
                let at = i * 80;
                for (k, v) in face.v.iter_mut().enumerate() {
                    *v = vec3_at(b, at + k * 12);
                }
                for (k, t) in face.t.iter_mut().enumerate() {
                    *t = Vec2::new(f32_at(b, at + 48 + k * 8), f32_at(b, at + 52 + k * 8));
                }
            }
            BuildLod { faces, material: u32_at(b, 640) }
        })?;
        if let Some(c) = r.open_chunk(chunk::MU_MODELS)? {
            out.mu_models = read_mu_models(c.bytes())?;
        }
        out.mu_refs = records(&r, chunk::MU_REFS, MU_REF, |b| {
            let mut transform = [[0.0f32; 4]; 4];
            for (i, row) in transform.iter_mut().enumerate() {
                for (k, v) in row.iter_mut().enumerate() {
                    *v = f32_at(b, 4 + (i * 4 + k) * 4);
                }
            }
            let mut reserved = [0u32; 8];
            for (i, v) in reserved.iter_mut().enumerate() {
                *v = u32_at(b, 76 + i * 4);
            }
            MuReference {
                model: u32_at(b, 0),
                transform,
                flags: u32_at(b, 68),
                sector: u16_at(b, 72),
                reserved,
            }
        })?;
        Ok(out)
    }

    /// The file, chunks in the order the build writes them.
    #[must_use]
    pub fn write(&self) -> Vec<u8> {
        let mut w = Writer::with_capacity(4096 + self.verts.len() * 12 + self.faces.len() * (FACE + 4));
        w.chunk(chunk::VERSION, |w| w.w_u32(self.version));
        w.chunk(chunk::PARAMETERS, |w| self.params.write(w));
        w.chunk(chunk::VERTICES, |w| {
            let mut raw = Vec::new();
            push_vec3s(&mut raw, &self.verts);
            w.w(&raw);
        });
        w.chunk(chunk::FACES, |w| {
            for f in self.faces.iter() {
                f.write(w);
            }
        });
        if self.version >= 18 {
            w.chunk(chunk::SMOOTH_GROUPS, |w| {
                for g in self.smooth_groups.iter() {
                    w.w_u32(*g);
                }
            });
        }
        w.chunk(chunk::MATERIALS, |w| {
            for m in &self.materials {
                for v in [m.surface, m.shader, m.shader_xrlc, m.sector, m.reserved, 0] {
                    w.w_u16(v);
                }
                w.w_u32(m.internal_max_area);
            }
        });
        for (id, names) in
            [(chunk::SHADERS_RENDER, &self.shaders), (chunk::SHADERS_COMPILE, &self.shaders_xrlc)]
        {
            w.chunk(id, |w| {
                let mut raw = Vec::with_capacity(names.len() * SHADER);
                for name in names {
                    push_fixed_str(&mut raw, name, SHADER);
                }
                w.w(&raw);
            });
        }
        w.chunk(chunk::TEXTURES, |w| {
            for t in &self.textures {
                let mut raw = Vec::with_capacity(128);
                push_fixed_str(&mut raw, &t.name, 128);
                w.w(&raw);
                for v in [t.width, t.height, t.has_alpha, t.reserve] {
                    w.w_u32(v);
                }
            }
        });
        w.chunk(chunk::GLOWS, |w| {
            for g in &self.glows {
                w.w_vec3(g.pos);
                w.w_f32(g.size);
                w.w_u32(g.flags);
                w.w_u32(g.material);
            }
        });
        w.chunk(chunk::PORTALS, |w| {
            for p in &self.portals {
                p.write(w);
            }
        });
        w.chunk(chunk::LIGHT_CONTROL, |w| {
            for c in &self.light_controls {
                let mut raw = Vec::with_capacity(64);
                push_fixed_str(&mut raw, &c.name, 64);
                w.w(&raw);
                w.w_u32(c.data.len() as u32);
                for v in &c.data {
                    w.w_u32(*v);
                }
            }
        });
        w.chunk(chunk::LIGHT_STATIC, |w| {
            for l in &self.lights_static {
                l.write(w);
            }
        });
        w.chunk(chunk::LIGHT_DYNAMIC, |w| {
            for l in &self.lights_dynamic {
                l.light.write(w);
                for s in l.sectors {
                    w.w_u16(s);
                }
                w.w_u32(l.sector_count);
            }
        });
        w.chunk(chunk::LOD_MODELS, |w| {
            for lod in &self.lods {
                for face in lod.faces {
                    for v in face.v {
                        w.w_vec3(v);
                    }
                    for t in face.t {
                        w.w_vec2(t);
                    }
                }
                w.w_u32(lod.material);
            }
        });
        w.chunk(chunk::MU_MODELS, |w| {
            for m in &self.mu_models {
                w.w_stringz(&m.name);
                w.w_u32(m.verts.len() as u32);
                for v in &m.verts {
                    w.w_vec3(*v);
                }
                w.w_u32(m.faces.len() as u32);
                for f in &m.faces {
                    f.write(w);
                }
                w.w_u16(m.lod_id);
                // one word per face, whatever the list holds
                for i in 0..m.faces.len() {
                    w.w_u32(m.smooth_groups.get(i).copied().unwrap_or(0));
                }
            }
        });
        w.chunk(chunk::MU_REFS, |w| {
            for r in &self.mu_refs {
                w.w_u32(r.model);
                w.w_matrix(&r.transform);
                w.w_u32(r.flags);
                w.w_u16(r.sector);
                w.w_u16(0);
                for v in r.reserved {
                    w.w_u32(v);
                }
            }
        });
        w.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn face(i: u32) -> BuildFace {
        BuildFace {
            v: [i, i + 1, i + 2],
            t: [Vec2::new(0.0, 0.0), Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0)],
            material: (i % 2) as u16,
            material_game: 100 + i,
        }
    }

    fn project() -> BuildProject {
        let light = Light {
            kind: 1,
            diffuse: [1.0, 0.9, 0.8, 1.0],
            position: Vec3::new(1.0, 2.0, 3.0),
            range: 12.0,
            attenuation1: 1.0,
            ..Light::default()
        };
        let mut lod = BuildLod { material: 1, ..BuildLod::default() };
        lod.faces[7].v[3] = Vec3::splat(9.0);
        lod.faces[7].t[3] = Vec2::new(0.25, 0.75);
        BuildProject {
            version: XRCL_CURRENT_VERSION,
            params: BuildParams::default(),
            verts: (0..5).map(|i| Vec3::new(i as f32, 0.0, 1.0)).collect(),
            faces: vec![face(0), face(1), face(2)].into(),
            smooth_groups: vec![1, 2, 0xffff_ffff].into(),
            materials: vec![
                BuildMaterial {
                    surface: 0,
                    shader: 1,
                    shader_xrlc: 0,
                    sector: 3,
                    reserved: 0,
                    internal_max_area: 7,
                },
                BuildMaterial { surface: 1, ..BuildMaterial::default() },
            ],
            shaders: vec!["default".into(), "def_shaders\\def_vertex".into()],
            shaders_xrlc: vec!["default".into()],
            textures: vec![
                BuildTexture {
                    name: "grnd\\grnd_zemlya".into(),
                    width: 1024,
                    height: 512,
                    has_alpha: 0,
                    reserve: 0,
                },
                BuildTexture {
                    name: "trees\\trees_list".into(),
                    width: 256,
                    height: 256,
                    has_alpha: 1,
                    reserve: 0,
                },
            ],
            glows: vec![BuildGlow { pos: Vec3::Y, size: 0.5, flags: 1, material: 1 }],
            portals: vec![Portal::new(0, 1, &[Vec3::ZERO, Vec3::X, Vec3::Y, Vec3::Z])],
            light_controls: vec![
                LightControl { name: "$hemi".into(), data: vec![] },
                LightControl { name: "flicker".into(), data: vec![0xff00_0000, 0xffff_ffff] },
            ],
            lights_static: vec![BuildLight { controller: 1, light }],
            lights_dynamic: vec![BuildLightDynamic {
                light: BuildLight { controller: 0, light },
                sectors: [5; 16],
                sector_count: 2,
            }],
            lods: vec![lod],
            mu_models: vec![MuModel {
                name: "trees\\new_trees\\trees_topol".into(),
                verts: vec![Vec3::ZERO, Vec3::X, Vec3::Y],
                faces: vec![face(0)],
                lod_id: u16::MAX,
                smooth_groups: vec![4],
            }],
            mu_refs: vec![MuReference {
                model: 0,
                transform: [
                    [1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [5.0, 6.0, 7.0, 1.0],
                ],
                flags: 3,
                sector: 2,
                reserved: [0; 8],
            }],
        }
    }

    #[test]
    fn records_have_the_sizes_of_the_c_structs() {
        let bytes = project().write();
        let r = Reader::new(&bytes);
        let len = |id: u32| r.sub(id).unwrap().unwrap().len();
        assert_eq!(
            r.chunks().map(|c| c.0).collect::<Vec<_>>(),
            [0, 1, 2, 3, 16, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15]
        );
        assert_eq!(len(chunk::PARAMETERS), 52);
        assert_eq!(len(chunk::VERTICES), 5 * 12);
        assert_eq!(len(chunk::FACES), 3 * 44);
        assert_eq!(len(chunk::SMOOTH_GROUPS), 3 * 4);
        assert_eq!(len(chunk::MATERIALS), 2 * 16);
        assert_eq!(len(chunk::SHADERS_RENDER), 2 * 128);
        assert_eq!(len(chunk::TEXTURES), 2 * 144);
        assert_eq!(len(chunk::GLOWS), 24);
        assert_eq!(len(chunk::PORTALS), 80);
        assert_eq!(len(chunk::LIGHT_CONTROL), 68 + 68 + 8);
        assert_eq!(len(chunk::LIGHT_STATIC), 108);
        assert_eq!(len(chunk::LIGHT_DYNAMIC), 144);
        assert_eq!(len(chunk::LOD_MODELS), 644);
        assert_eq!(len(chunk::MU_MODELS), 28 + 4 + 36 + 4 + 44 + 2 + 4);
        assert_eq!(len(chunk::MU_REFS), 108);
    }

    #[test]
    fn the_file_round_trips() {
        let file = project();
        let bytes = file.write();
        let back = BuildProject::read(&bytes).unwrap();
        assert_eq!(back, file);
        assert_eq!(back.write(), bytes);
        assert_eq!(back.portals[0].used().len(), 4);
    }

    #[test]
    fn hostile_input_is_an_error() {
        let bytes = project().write();
        assert!(BuildProject::read(&[]).is_err());
        assert!(BuildProject::read(&bytes[..16]).is_err());
        let mut version = bytes.clone();
        version[8] = 3;
        assert!(matches!(BuildProject::read(&version), Err(LevelError::Version { found: 3, .. })));
        // cut short anywhere, a chunk missing, a record cut in two, a size past the end: refused
        // with a sentence, never read as a build with fewer chunks
        let says = |b: &[u8], what: &str| {
            let e = BuildProject::read(b).unwrap_err().to_string();
            assert!(e.contains(what), "{e}");
        };
        for cut in [bytes.len() - 1, bytes.len() / 2, 100, 14, 9] {
            says(&bytes[..cut], "cut short");
        }
        says(&bytes[..0], "empty");
        says(&[0xAB; 64], "not a build project");
        let mut walk = Reader::new(&bytes).chunks().map(|c| c.2.len() + 8);
        let first_two = walk.next().unwrap() + walk.next().unwrap();
        says(&bytes[..first_two], "incomplete");
        let mut odd = bytes.clone();
        // the vertices chunk (third) one byte short of whole records, the file still whole
        let at = first_two + 4;
        let size = u32::from_le_bytes(odd[at..at + 4].try_into().unwrap());
        odd[at..at + 4].copy_from_slice(&(size - 1).to_le_bytes());
        odd.remove(at + 4);
        says(&odd, "whole number");
        let mut past = bytes.clone();
        past[4..8].copy_from_slice(&0x7fff_fff0u32.to_le_bytes());
        says(&past, "cut short");
        // a multi-usage model that claims the whole address space
        let mut models = Writer::new();
        models.w_stringz("m");
        models.w_u32(u32::MAX);
        assert!(read_mu_models(models.bytes()).is_err());
        let mut models = Writer::new();
        models.w_stringz("m");
        models.w_u32(0);
        models.w_u32(0x1000_0000);
        assert!(read_mu_models(models.bytes()).is_err());
    }
}
