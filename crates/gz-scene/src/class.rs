//! The sixteen object classes (`ESceneClassList.h`) and their tools' names.

use serde::Serialize;

/// Number of classes; `by_class` and the tool table are indexed by [`ObjClass::index`].
pub const CLASSES: usize = 16;

/// `OBJCLASS_*`. The discriminant is the id the files carry (`clsid`, `CHUNK_TOOLS_DATA + id`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjClass {
    Group = 0,
    Glow = 1,
    SceneObject = 2,
    Light = 3,
    Shape = 4,
    SoundSrc = 5,
    SpawnPoint = 6,
    Way = 7,
    Sector = 8,
    Portal = 9,
    SoundEnv = 10,
    Particles = 11,
    DetailObjects = 12,
    AiMap = 13,
    Wallmark = 14,
    FogVolume = 15,
}

impl ObjClass {
    /// In id order: the order `m_SceneTools` (an `xr_map` by id) is walked in.
    pub const ALL: [ObjClass; CLASSES] = [
        Self::Group,
        Self::Glow,
        Self::SceneObject,
        Self::Light,
        Self::Shape,
        Self::SoundSrc,
        Self::SpawnPoint,
        Self::Way,
        Self::Sector,
        Self::Portal,
        Self::SoundEnv,
        Self::Particles,
        Self::DetailObjects,
        Self::AiMap,
        Self::Wallmark,
        Self::FogVolume,
    ];

    #[must_use]
    pub fn id(self) -> u32 {
        self as u32
    }

    #[must_use]
    pub fn index(self) -> usize {
        self as usize
    }

    #[must_use]
    pub fn from_id(id: u32) -> Option<Self> {
        Self::ALL.get(usize::try_from(id).ok()?).copied()
    }

    /// `ClassName()`: the part file's name and the class the wire names.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Glow => "glow",
            Self::SceneObject => "scene_object",
            Self::Light => "light",
            Self::Shape => "shape",
            Self::SoundSrc => "sound_src",
            Self::SpawnPoint => "spawn",
            Self::Way => "way",
            Self::Sector => "sector",
            Self::Portal => "portal",
            Self::SoundEnv => "sound_env",
            Self::Particles => "ps",
            Self::DetailObjects => "detail_object",
            Self::AiMap => "ai_map",
            Self::Wallmark => "wallmark",
            Self::FogVolume => "fog_volume",
        }
    }

    /// `ClassDesc()`.
    #[must_use]
    pub fn desc(self) -> &'static str {
        match self {
            Self::Group => "Group",
            Self::Glow => "Glow",
            Self::SceneObject => "Scene Object",
            Self::Light => "Light",
            Self::Shape => "Shape",
            Self::SoundSrc => "Sound Source",
            Self::SpawnPoint => "Spawn",
            Self::Way => "Way",
            Self::Sector => "Sector",
            Self::Portal => "Portal",
            Self::SoundEnv => "Sound Environment",
            Self::Particles => "Particle System",
            Self::DetailObjects => "Detail Objects",
            Self::AiMap => "AI Map",
            Self::Wallmark => "Wallmark",
            Self::FogVolume => "Scene fog volumes",
        }
    }

    /// A class by its `ClassName`, any case.
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::ALL.iter().copied().find(|c| c.name().eq_ignore_ascii_case(name))
    }

    /// Whether the tool holds placed objects (`GetOTool` is not null). Detail objects, the AI
    /// map and wallmarks are bulk data.
    #[must_use]
    pub fn is_object_tool(self) -> bool {
        !matches!(self, Self::DetailObjects | Self::AiMap | Self::Wallmark)
    }

    /// `can_use_inifile()`: the part is LTX; the bulk tools write a chunk stream.
    #[must_use]
    pub fn uses_ltx(self) -> bool {
        self.is_object_tool()
    }

    /// `SaveFileCount()`: groups split light presets into `group.part1`.
    #[must_use]
    pub fn save_file_count(self) -> usize {
        if self == Self::Group { 2 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_names_are_unique() {
        for c in ObjClass::ALL {
            assert_eq!(ObjClass::from_id(c.id()), Some(c));
            assert_eq!(ObjClass::by_name(c.name()), Some(c));
        }
        assert_eq!(ObjClass::from_id(16), None);
        assert_eq!(ObjClass::by_name("SPAWN"), Some(ObjClass::SpawnPoint));
    }
}
