//! Scene objects, lights, glows, sound sources and particle effects: their data.

use gz_base::{Blob, SmolStr};

use crate::math::Vec3;

// ---------------------------------------------------------------- scene objects

/// The flag that makes a scene object use its surface.
pub const SO_USE_SURFACE: u32 = 1 << 0;
/// The reference names an `.ogf`.
pub const SO_VISUAL_MODE: u32 = 1 << 1;

/// A surface of the reference with the scene's own shader, compiler shader, material,
/// texture and vertex map.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct SurfaceOverride {
    pub name: SmolStr,
    pub shader: SmolStr,
    pub shader_xrlc: SmolStr,
    pub game_mtl: SmolStr,
    pub texture: SmolStr,
    pub vmap: SmolStr,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct SceneObjectData {
    /// The library object (`props\box_1`) or, in visual mode, the `.ogf` path.
    pub reference: SmolStr,
    pub flags: u32,
    pub surfaces: Vec<SurfaceOverride>,
    /// The object is a stash: the bake puts an `inventory_box` over it (`super::stash`).
    pub stash: Option<super::Stash>,
}

impl SceneObjectData {
    #[must_use]
    pub fn library(reference: &str) -> Self {
        Self { reference: SmolStr::new(reference), ..Self::default() }
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
/// The light kinds: point, spot, directional.
pub const LT_POINT: u32 = 1;
pub const LT_SPOT: u32 = 2;
pub const LT_DIRECT: u32 = 3;

/// The fuzzy light data.
#[derive(Clone, Debug, PartialEq)]
pub struct Fuzzy {
    /// The fuzzy light shapes: sphere 0, box 1.
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

#[derive(Clone, Debug, PartialEq)]
pub struct LightData {
    pub kind: u32,
    pub color: [f32; 4],
    pub brightness: f32,
    pub range: f32,
    pub attenuation: [f32; 3],
    pub cone: f32,
    pub virtual_size: f32,
    pub flags: u32,
    pub lcontrol: u32,
    pub anim_ref: SmolStr,
    pub falloff_tex: SmolStr,
    pub fuzzy: Option<Fuzzy>,
}

impl Default for LightData {
    /// A new light.
    fn default() -> Self {
        Self {
            kind: LT_POINT,
            color: [1.0, 1.0, 1.0, 0.0],
            brightness: 1.0,
            range: 8.0,
            attenuation: [1.0, 0.0, 0.0],
            cone: std::f32::consts::PI / 8.0,
            virtual_size: 0.0,
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

#[derive(Clone, Debug, PartialEq)]
pub struct GlowData {
    pub shader: SmolStr,
    pub texture: SmolStr,
    pub radius: f32,
    pub flags: u16,
}

/// The shaders the game draws a glow with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GlowShader {
    /// The lens flares': the picture added to the frame. What a glow that names no shader, the
    /// game's own glow shader or the flares' is drawn with.
    Flare,
    /// The sun's: the same picture, hidden by depth as well.
    Sun,
    /// A shader no glow is drawn with: the game takes the flares' in its place.
    Other,
}

/// Which of the game's glow shaders a name is. The game's own list (its renderer's glows): a
/// change of one side is a change of both.
#[must_use]
pub fn glow_shader(name: &str) -> GlowShader {
    match name.trim().replace('/', "\\").to_ascii_lowercase().as_str() {
        "" | "effects\\glow" | "effects\\flare" => GlowShader::Flare,
        "effects\\sun" => GlowShader::Sun,
        _ => GlowShader::Other,
    }
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

#[derive(Clone, Debug, PartialEq)]
pub struct SoundSrcData {
    pub kind: u32,
    pub wav: SmolStr,
    pub flags: u32,
    /// The sound source's position.
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

#[derive(Clone, Debug, PartialEq)]
pub struct ParticlesData {
    pub reference: SmolStr,
    /// The particles' game type; the default is every bit.
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
