//! Scene objects, lights, glows, sound sources and particle systems: their data and their part
//! of the files (`SceneObjectIO.cpp`, `ELight_IO.cpp`, `glow.cpp`, `ESound_Source.cpp`,
//! `EParticlesObject.cpp`).

use gz_base::{Blob, SmolStr};
use gz_chunk::{Reader, Writer};

use crate::SceneError;
use crate::ltx::{IniReader, IniWriter};
use crate::math::Vec3;

/// What a class body read says: understood, or of a version the C++ refuses (kept).
pub enum Body<T> {
    Read(T),
    Refused(String),
}

fn chunk_u16(r: &Reader<'_>, id: u32) -> Result<Option<u16>, SceneError> {
    Ok(match r.open_chunk(id)? {
        Some(c) => Some(c.reader().r_u16()?),
        None => None,
    })
}

// ---------------------------------------------------------------- scene objects

pub const SCENEOBJ_VERSION: u32 = 0x12;
/// `CSceneObject::flUseSurface`.
pub const SO_USE_SURFACE: u32 = 1 << 0;
/// `flVisualMode`: the reference names an `.ogf`.
pub const SO_VISUAL_MODE: u32 = 1 << 1;

/// A surface of the reference with the scene's own shader, compiler shader, material,
/// texture and vertex map (`flUseSurface`).
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SurfaceOverride {
    pub name: SmolStr,
    pub shader: SmolStr,
    pub shader_xrlc: SmolStr,
    pub game_mtl: SmolStr,
    pub texture: SmolStr,
    pub vmap: SmolStr,
}

/// `CSceneObject`.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SceneObjectData {
    /// The library object (`props\box_1`) or, in visual mode, the `.ogf` path.
    pub reference: SmolStr,
    pub flags: u32,
    pub surfaces: Vec<SurfaceOverride>,
}

impl SceneObjectData {
    #[must_use]
    pub fn library(reference: &str) -> Self {
        Self { reference: SmolStr::new(reference), flags: 0, surfaces: Vec::new() }
    }

    #[must_use]
    pub fn visual_mode(&self) -> bool {
        self.flags & SO_VISUAL_MODE != 0
    }

    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let reference = r.r_string(s, "reference_name")?;
        let flags = r.opt_u32(s, "flags").unwrap_or(0);
        let mut surfaces = Vec::new();
        if flags & SO_USE_SURFACE != 0 && r.line_exist(s, "000001") {
            let mut n = 1u32;
            let mut next = |r: &IniReader| -> Result<String, SceneError> {
                let k = format!("{n:06}");
                n += 1;
                r.r_string_wb(s, &k)
            };
            let count: u32 = next(r)?.trim().parse().unwrap_or(0);
            for _ in 0..count.min(4096) {
                surfaces.push(SurfaceOverride {
                    name: next(r)?.into(),
                    shader: next(r)?.into(),
                    shader_xrlc: next(r)?.into(),
                    game_mtl: next(r)?.into(),
                    texture: next(r)?.into(),
                    vmap: next(r)?.into(),
                });
            }
        }
        Ok(Self { reference: reference.into(), flags, surfaces })
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", SCENEOBJ_VERSION);
        w.w_string(s, "reference_name", &self.reference);
        w.w_u32(s, "flags", self.flags);
        if self.flags & SO_USE_SURFACE != 0 {
            let mut n = 0u32;
            let mut put = |w: &mut IniWriter, v: &str, quoted: bool| {
                n += 1;
                if quoted {
                    w.w_string(s, &format!("{n:06}"), &format!("\"{v}\""))
                } else {
                    w.w_string(s, &format!("{n:06}"), v)
                }
            };
            put(w, &(self.surfaces.len() as i32).to_string(), false);
            for sf in &self.surfaces {
                for v in [&sf.name, &sf.shader, &sf.shader_xrlc, &sf.game_mtl, &sf.texture, &sf.vmap] {
                    put(w, v, true);
                }
            }
        }
    }

    /// `LoadStream`; version `0x10` carries the placement in a chunk of its own, which
    /// `placement` receives.
    pub fn read_stream(r: &Reader<'_>, placement: &mut Option<[Vec3; 3]>) -> Result<Body<Self>, SceneError> {
        let Some(version) = chunk_u16(r, 0x0900)? else {
            return Ok(Body::Refused("CSceneObject: no version".into()));
        };
        if version == 0x10
            && let Some(c) = r.open_chunk(0x0904)?
        {
            let mut q = c.reader();
            *placement = Some([q.r_vec3()?, q.r_vec3()?, q.r_vec3()?]);
        }
        let Some(c) = r.open_chunk(0x0902)? else {
            return Ok(Body::Refused("CSceneObject: no reference".into()));
        };
        let mut q = c.reader();
        if version <= 0x11 {
            q.r_u32()?;
            q.r_u32()?;
        }
        let reference = q.r_stringz()?;
        let flags = match r.open_chunk(0x0905)? {
            Some(c) => c.reader().r_u32()?,
            None => 0,
        };
        let mut surfaces = Vec::new();
        if flags & SO_USE_SURFACE != 0
            && let Some(c) = r.open_chunk(0x0906)?
        {
            let mut q = c.reader();
            let n = q.r_u32()?;
            for _ in 0..n.min(4096) {
                surfaces.push(SurfaceOverride {
                    name: q.r_stringz()?.into(),
                    shader: q.r_stringz()?.into(),
                    shader_xrlc: q.r_stringz()?.into(),
                    game_mtl: q.r_stringz()?.into(),
                    texture: q.r_stringz()?.into(),
                    vmap: q.r_stringz()?.into(),
                });
            }
        }
        Ok(Body::Read(Self { reference: reference.into(), flags, surfaces }))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0x0900, |w| w.w_u16(SCENEOBJ_VERSION as u16));
        w.chunk(0x0902, |w| w.w_stringz(&self.reference));
        w.chunk(0x0905, |w| w.w_u32(self.flags));
        if self.flags & SO_USE_SURFACE != 0 {
            w.chunk(0x0906, |w| {
                w.w_u32(u32::try_from(self.surfaces.len()).unwrap_or(0));
                for sf in &self.surfaces {
                    for v in [&sf.name, &sf.shader, &sf.shader_xrlc, &sf.game_mtl, &sf.texture, &sf.vmap] {
                        w.w_stringz(v);
                    }
                }
            });
        }
    }
}

// ---------------------------------------------------------------- lights

pub const LIGHT_VERSION: u32 = 0x11;
pub mod light_flags {
    pub const AFFECT_STATIC: u32 = 1 << 0;
    pub const AFFECT_DYNAMIC: u32 = 1 << 1;
    pub const PROCEDURAL: u32 = 1 << 2;
    pub const BREAKING: u32 = 1 << 3;
    pub const POINT_FUZZY: u32 = 1 << 4;
    pub const CAST_SHADOW: u32 = 1 << 5;
}
/// `D3DLIGHT_POINT`, `D3DLIGHT_SPOT`, `D3DLIGHT_DIRECTIONAL`.
pub const LT_POINT: u32 = 1;
pub const LT_SPOT: u32 = 2;
pub const LT_DIRECT: u32 = 3;

/// `CLight::SFuzzyData`.
#[derive(Clone, Debug, PartialEq)]
pub struct Fuzzy {
    /// `fstSphere` 0, `fstBox` 1.
    pub shape: u8,
    pub sphere_radius: f32,
    pub box_dim: Vec3,
    pub points: Vec<Vec3>,
}

impl Default for Fuzzy {
    fn default() -> Self {
        Self { shape: 0, sphere_radius: 0.1, box_dim: Vec3::splat(0.1), points: vec![Vec3::ZERO] }
    }
}

/// `CLight`.
#[derive(Clone, Debug, PartialEq)]
pub struct LightData {
    pub kind: u32,
    pub color: [f32; 4],
    pub brightness: f32,
    pub range: f32,
    pub attenuation: [f32; 3],
    pub cone: f32,
    pub virtual_size: f32,
    pub use_in_d3d: bool,
    pub flags: u32,
    pub lcontrol: u32,
    pub anim_ref: SmolStr,
    pub falloff_tex: SmolStr,
    pub fuzzy: Option<Fuzzy>,
}

impl Default for LightData {
    /// `CLight::Construct`.
    fn default() -> Self {
        Self {
            kind: LT_POINT,
            color: [1.0, 1.0, 1.0, 0.0],
            brightness: 1.0,
            range: 8.0,
            attenuation: [1.0, 0.0, 0.0],
            cone: std::f32::consts::PI / 8.0,
            virtual_size: 0.0,
            use_in_d3d: true,
            flags: light_flags::AFFECT_STATIC,
            lcontrol: 0,
            anim_ref: SmolStr::default(),
            falloff_tex: SmolStr::default(),
            fuzzy: None,
        }
    }
}

impl LightData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != LIGHT_VERSION {
            return Ok(Body::Refused("CLight: Unsupported version.".into()));
        }
        let flags = r.r_u32(s, "light_flags")?;
        let fuzzy = if flags & light_flags::POINT_FUZZY != 0 {
            let n = r.r_u32(s, "fuzzy_point_count")?;
            let mut points = Vec::new();
            for i in 0..n.min(1 << 16) {
                points.push(r.r_fvector3(s, &format!("fuzzy_point_{i}"))?);
            }
            Some(Fuzzy {
                shape: r.r_u8(s, "fuzzy_shape_type")?,
                sphere_radius: r.r_float(s, "fuzzy_sphere_radius")?,
                box_dim: r.r_fvector3(s, "fuzzy_box_dim")?,
                points,
            })
        } else {
            None
        };
        Ok(Body::Read(Self {
            kind: r.r_u32(s, "type")?,
            color: r.r_fcolor(s, "color")?,
            brightness: r.r_float(s, "brightness")?,
            range: r.r_float(s, "range")?,
            attenuation: [
                r.r_float(s, "attenuation0")?,
                r.r_float(s, "attenuation1")?,
                r.r_float(s, "attenuation2")?,
            ],
            cone: r.r_float(s, "cone")?,
            virtual_size: r.r_float(s, "virtual_size")?,
            use_in_d3d: r.r_bool(s, "use_in_d3d")?,
            flags,
            lcontrol: r.r_u32(s, "light_control")?,
            anim_ref: r.r_string(s, "anim_ref_name")?.into(),
            falloff_tex: r.r_string(s, "fallof_texture")?.into(),
            fuzzy,
        }))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u16(s, "version", LIGHT_VERSION as u16);
        w.w_u32(s, "type", self.kind);
        w.w_fcolor(s, "color", self.color);
        w.w_float(s, "brightness", self.brightness);
        w.w_float(s, "range", self.range);
        w.w_float(s, "attenuation0", self.attenuation[0]);
        w.w_float(s, "attenuation1", self.attenuation[1]);
        w.w_float(s, "attenuation2", self.attenuation[2]);
        w.w_float(s, "cone", self.cone);
        w.w_float(s, "virtual_size", self.virtual_size);
        w.w_bool(s, "use_in_d3d", self.use_in_d3d);
        w.w_u32(s, "light_flags", self.flags);
        w.w_u32(s, "light_control", self.lcontrol);
        w.w_string(s, "anim_ref_name", &self.anim_ref);
        w.w_string(s, "fallof_texture", &self.falloff_tex);
        if self.flags & light_flags::POINT_FUZZY != 0 {
            let f = self.fuzzy.clone().unwrap_or_default();
            w.w_u8(s, "fuzzy_shape_type", f.shape);
            w.w_float(s, "fuzzy_sphere_radius", f.sphere_radius);
            w.w_fvector3(s, "fuzzy_box_dim", f.box_dim);
            w.w_u32(s, "fuzzy_point_count", u32::try_from(f.points.len()).unwrap_or(0));
            for (i, p) in f.points.iter().enumerate() {
                w.w_fvector3(s, &format!("fuzzy_point_{i}"), *p);
            }
        }
    }

    /// `LoadStream`; the old `Flight` chunk carries the position, which `position` receives.
    pub fn read_stream(r: &Reader<'_>, position: &mut Option<Vec3>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 0xB411)?.unwrap_or(0);
        if version != 0x10 && version != LIGHT_VERSION as u16 {
            return Ok(Body::Refused("CLight: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xB442)? {
            let mut q = c.reader();
            d.kind = q.r_u32()?;
            d.color = [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?];
            d.brightness = q.r_f32()?;
            d.range = q.r_f32()?;
            d.attenuation = [q.r_f32()?, q.r_f32()?, q.r_f32()?];
            d.cone = q.r_f32()?;
            d.virtual_size = q.r_f32()?;
        } else {
            let Some(c) = r.open_chunk(0xB435)? else {
                return Ok(Body::Refused("CLight: no parameters".into()));
            };
            let mut q = c.reader();
            d.kind = q.r_u32()?;
            d.color = [q.r_f32()?, q.r_f32()?, q.r_f32()?, q.r_f32()?];
            q.advance(32)?;
            *position = Some(q.r_vec3()?);
            q.r_vec3()?;
            d.range = q.r_f32()?;
            q.r_f32()?;
            d.attenuation = [q.r_f32()?, q.r_f32()?, q.r_f32()?];
            q.r_f32()?;
            d.cone = q.r_f32()?;
            if let Some(b) = r.open_chunk(0xB425)? {
                d.brightness = b.reader().r_f32()?;
            }
        }
        if let Some(c) = r.open_chunk(0xB436)? {
            d.use_in_d3d = c.reader().r_u32()? != 0;
        }
        if let Some(c) = r.open_chunk(0xB413)? {
            d.flags = c.reader().r_u32()?;
        }
        if let Some(c) = r.open_chunk(0xB441)? {
            d.lcontrol = c.reader().r_u32()?;
        }
        if d.kind == LT_DIRECT {
            return Ok(Body::Refused("CLight: Can't load sun.".into()));
        }
        if let Some(c) = r.open_chunk(0xB438)? {
            d.anim_ref = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xB439)? {
            d.falloff_tex = c.reader().r_stringz()?.into();
        }
        if let Some(c) = r.open_chunk(0xB440)? {
            let mut q = c.reader();
            let shape = q.r_u8()?;
            let sphere_radius = q.r_f32()?;
            let box_dim = q.r_vec3()?;
            let n = usize::try_from(q.r_i16()?.max(0)).unwrap_or(0);
            if n * 12 > q.len() {
                return Err(SceneError::load("CLight: the fuzzy points are cut short"));
            }
            let mut points = Vec::with_capacity(n);
            for _ in 0..n {
                points.push(q.r_vec3()?);
            }
            d.fuzzy = Some(Fuzzy { shape, sphere_radius, box_dim, points });
            d.flags |= light_flags::POINT_FUZZY;
        } else {
            d.flags &= !light_flags::POINT_FUZZY;
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0xB411, |w| w.w_u16(LIGHT_VERSION as u16));
        w.chunk(0xB442, |w| {
            w.w_u32(self.kind);
            for c in self.color {
                w.w_f32(c);
            }
            w.w_f32(self.brightness);
            w.w_f32(self.range);
            for a in self.attenuation {
                w.w_f32(a);
            }
            w.w_f32(self.cone);
            w.w_f32(self.virtual_size);
        });
        w.chunk(0xB436, |w| w.w_u32(u32::from(self.use_in_d3d)));
        w.chunk(0xB413, |w| w.w_u32(self.flags));
        w.chunk(0xB441, |w| w.w_u32(self.lcontrol));
        if !self.anim_ref.is_empty() {
            w.chunk(0xB438, |w| w.w_stringz(&self.anim_ref));
        }
        if !self.falloff_tex.is_empty() {
            w.chunk(0xB439, |w| w.w_stringz(&self.falloff_tex));
        }
        if self.flags & light_flags::POINT_FUZZY != 0 {
            let f = self.fuzzy.clone().unwrap_or_default();
            w.chunk(0xB440, |w| {
                w.w_u8(f.shape);
                w.w_f32(f.sphere_radius);
                w.w_vec3(f.box_dim);
                w.w_i16(i16::try_from(f.points.len()).unwrap_or(i16::MAX));
                for p in &f.points {
                    w.w_vec3(*p);
                }
            });
        }
    }
}

// ---------------------------------------------------------------- glows

pub const GLOW_VERSION: u32 = 0x12;

/// `CGlow`.
#[derive(Clone, Debug, PartialEq)]
pub struct GlowData {
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub radius: f32,
    pub flags: u16,
}

impl Default for GlowData {
    fn default() -> Self {
        Self {
            shader: SmolStr::new_static("effects\\glow"),
            texture: SmolStr::default(),
            radius: 0.5,
            flags: 0,
        }
    }
}

impl GlowData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != GLOW_VERSION {
            return Ok(Body::Refused("CGlow: Unsupported version.".into()));
        }
        Ok(Body::Read(Self {
            shader: r.r_string(s, "shader_name")?.into(),
            texture: r.r_string(s, "texture_name")?.into(),
            radius: r.r_float(s, "radius")?,
            flags: r.r_u32(s, "flags")? as u16,
        }))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u16(s, "version", GLOW_VERSION as u16);
        w.w_float(s, "radius", self.radius);
        w.w_string(s, "shader_name", &self.shader);
        w.w_string(s, "texture_name", &self.texture);
        w.w_u16(s, "flags", self.flags);
    }

    pub fn read_stream(r: &Reader<'_>, position: &mut Option<Vec3>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 0xC411)?.unwrap_or(0);
        if version != 0x11 && version != GLOW_VERSION as u16 {
            return Ok(Body::Refused("CGlow: Unsupported version.".into()));
        }
        let mut d = Self::default();
        if let Some(c) = r.open_chunk(0xC414)? {
            d.shader = c.reader().r_stringz()?.into();
        }
        let (Some(t), Some(p)) = (r.open_chunk(0xC415)?, r.open_chunk(0xC413)?) else {
            return Ok(Body::Refused("CGlow: the texture or the parameters are missing".into()));
        };
        d.texture = t.reader().r_stringz()?.into();
        let mut q = p.reader();
        d.radius = q.r_f32()?;
        if version == 0x11 {
            *position = Some(q.r_vec3()?);
        }
        if let Some(c) = r.open_chunk(0xC416)? {
            d.flags = c.reader().r_u16()?;
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0xC411, |w| w.w_u16(GLOW_VERSION as u16));
        w.chunk(0xC413, |w| w.w_f32(self.radius));
        w.chunk(0xC414, |w| w.w_stringz(&self.shader));
        w.chunk(0xC415, |w| w.w_stringz(&self.texture));
        w.chunk(0xC416, |w| w.w_u16(self.flags));
    }
}

// ---------------------------------------------------------------- sound sources

pub const SOUND_SOURCE_VERSION: u32 = 0x14;

/// `ESoundSource`.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundSrcData {
    pub kind: u32,
    pub wav: SmolStr,
    pub flags: u32,
    /// `m_Params.position`: what `GetPosition` answers.
    pub position: Vec3,
    pub volume: f32,
    pub freq: f32,
    pub min_distance: f32,
    pub max_distance: f32,
    pub max_ai_distance: f32,
    pub random_pause: [f32; 2],
    pub active_time: [f32; 2],
    pub play_time: [f32; 2],
}

impl Default for SoundSrcData {
    fn default() -> Self {
        Self {
            kind: 0,
            wav: SmolStr::default(),
            flags: 1,
            position: Vec3::ZERO,
            volume: 1.0,
            freq: 1.0,
            min_distance: 1.0,
            max_distance: 300.0,
            max_ai_distance: 300.0,
            random_pause: [0.0; 2],
            active_time: [0.0; 2],
            play_time: [0.0; 2],
        }
    }
}

impl SoundSrcData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Body<Self>, SceneError> {
        if r.r_u32(s, "version")? != SOUND_SOURCE_VERSION {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        Ok(Body::Read(Self {
            kind: r.r_u32(s, "snd_type")?,
            wav: r.r_string(s, "snd_name")?.into(),
            flags: r.r_u32(s, "flags")?,
            position: r.r_fvector3(s, "snd_position")?,
            volume: r.r_float(s, "volume")?,
            freq: r.r_float(s, "freq")?,
            min_distance: r.r_float(s, "min_dist")?,
            max_distance: r.r_float(s, "max_dist")?,
            max_ai_distance: r.r_float(s, "max_ai_dist")?,
            random_pause: r.r_fvector2(s, "random_pause")?,
            active_time: r.r_fvector2(s, "active_time")?,
            play_time: r.r_fvector2(s, "play_time")?,
        }))
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", SOUND_SOURCE_VERSION);
        w.w_u32(s, "snd_type", self.kind);
        w.w_string(s, "snd_name", &self.wav);
        w.w_u32(s, "flags", self.flags);
        w.w_fvector3(s, "snd_position", self.position);
        w.w_float(s, "volume", self.volume);
        w.w_float(s, "freq", self.freq);
        w.w_float(s, "min_dist", self.min_distance);
        w.w_float(s, "max_dist", self.max_distance);
        w.w_float(s, "max_ai_dist", self.max_ai_distance);
        w.w_fvector2(s, "random_pause", self.random_pause);
        w.w_fvector2(s, "active_time", self.active_time);
        w.w_fvector2(s, "play_time", self.play_time);
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        if chunk_u16(r, 0x1001)? != Some(SOUND_SOURCE_VERSION as u16) {
            return Ok(Body::Refused("ESoundSource: Unsupported version.".into()));
        }
        let mut d = Self::default();
        let (Some(t), Some(n)) = (r.open_chunk(0x1002)?, r.open_chunk(0x1003)?) else {
            return Ok(Body::Refused("ESoundSource: the type or the name is missing".into()));
        };
        d.kind = t.reader().r_u32()?;
        d.wav = n.reader().r_stringz()?.into();
        let params = match (r.open_chunk(0x1007)?, r.open_chunk(0x1006)?, r.open_chunk(0x1004)?) {
            (Some(c), _, _) | (None, Some(c), _) => Some((c, true)),
            (None, None, Some(c)) => Some((c, false)),
            _ => None,
        };
        let Some((c, ai)) = params else {
            return Ok(Body::Refused("ESoundSource: Can't load Sound Source. Unsupported version.".into()));
        };
        let mut q = c.reader();
        d.position = q.r_vec3()?;
        d.volume = q.r_f32()?;
        d.freq = q.r_f32()?;
        d.min_distance = q.r_f32()?;
        d.max_distance = q.r_f32()?;
        d.max_ai_distance = if ai { q.r_f32()? } else { d.max_distance };
        if let Some(c) = r.open_chunk(0x1005)? {
            d.flags = c.reader().r_u32()?;
        }
        if let Some(c) = r.open_chunk(0x1008)? {
            let mut q = c.reader();
            d.random_pause = [q.r_f32()?, q.r_f32()?];
            d.active_time = [q.r_f32()?, q.r_f32()?];
            d.play_time = [q.r_f32()?, q.r_f32()?];
        }
        if d.kind != 0 {
            return Ok(Body::Refused("ESoundSource: unknown type".into()));
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(0x1001, |w| w.w_u16(SOUND_SOURCE_VERSION as u16));
        w.chunk(0x1002, |w| w.w_u32(self.kind));
        w.chunk(0x1003, |w| w.w_stringz(&self.wav));
        w.chunk(0x1005, |w| w.w_u32(self.flags));
        w.chunk(0x1007, |w| {
            w.w_vec3(self.position);
            w.w_f32(self.volume);
            w.w_f32(self.freq);
            w.w_f32(self.min_distance);
            w.w_f32(self.max_distance);
            w.w_f32(self.max_ai_distance);
        });
        w.chunk(0x1008, |w| {
            for v in [self.random_pause, self.active_time, self.play_time] {
                w.w_f32(v[0]);
                w.w_f32(v[1]);
            }
        });
    }
}

// ---------------------------------------------------------------- particle systems

pub const PS_VERSION: u32 = 0x13;

/// `EParticlesObject`.
#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesData {
    pub reference: SmolStr,
    /// `m_GameType` (`SetDefaults` is every bit).
    pub game_type: u16,
}

impl Default for ParticlesData {
    fn default() -> Self {
        Self { reference: SmolStr::default(), game_type: 0xffff }
    }
}

impl ParticlesData {
    pub fn read_ltx(r: &IniReader, s: &str) -> Result<Self, SceneError> {
        let version = r.r_u32(s, "version")?;
        let mut d = Self { reference: r.r_string(s, "ref_name")?.into(), ..Self::default() };
        if version >= 0x12 {
            d.game_type = r.r_u16(s, "game_type")?;
        }
        Ok(d)
    }

    pub fn write_ltx(&self, w: &mut IniWriter, s: &str) {
        w.w_u32(s, "version", PS_VERSION);
        w.w_string(s, "ref_name", &self.reference);
        w.w_u16(s, "game_type", self.game_type);
    }

    pub fn read_stream(r: &Reader<'_>) -> Result<Body<Self>, SceneError> {
        let version = chunk_u16(r, 1)?.unwrap_or(0);
        if version < 0x11 {
            return Ok(Body::Refused("PSObject: Unsupported version.".into()));
        }
        let Some(c) = r.open_chunk(2)? else {
            return Ok(Body::Refused("PSObject: no reference".into()));
        };
        let mut d = Self { reference: c.reader().r_stringz()?.into(), ..Self::default() };
        if version >= 0x13 {
            if let Some(c) = r.open_chunk(4)? {
                d.game_type = c.reader().r_u16()?;
            } else {
                return Ok(Body::Refused("PSObject: no game type".into()));
            }
        }
        Ok(Body::Read(d))
    }

    pub fn write_stream(&self, w: &mut Writer) {
        w.chunk(1, |w| w.w_u16(PS_VERSION as u16));
        w.chunk(2, |w| w.w_stringz(&self.reference));
        w.chunk(4, |w| w.w_u16(self.game_type));
    }
}

/// An object body kept as it was read.
#[derive(Clone, Debug, PartialEq)]
pub enum KeptObject {
    /// The object's section and the sections under it (`<section>_...`), by their suffix.
    Ltx { why: String, sections: Vec<(String, Vec<(String, String)>)> },
    /// The object's whole body chunk (`CHUNK_OBJECT_BODY`).
    Stream { why: String, body: Blob },
}
