//! Scene objects, lights, glows, sound sources and particle effects: their data.

use gz_base::{Blob, SmolStr};

use crate::math::Vec3;

// ---------------------------------------------------------------- scene objects

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
}

// ---------------------------------------------------------------- lights

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

impl LightData {}

// ---------------------------------------------------------------- glows

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

impl GlowData {}

// ---------------------------------------------------------------- sound sources

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

impl SoundSrcData {}

// ---------------------------------------------------------------- particle systems

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

impl ParticlesData {}

/// An object body kept as it was read.
#[derive(Clone, Debug, PartialEq)]
pub enum KeptObject {
    /// The object's section and the sections under it (`<section>_...`), by their suffix.
    Ltx { why: String, sections: Vec<(String, Vec<(String, String)>)> },
    /// The object's whole body chunk (`CHUNK_OBJECT_BODY`).
    Stream { why: String, body: Blob },
}
