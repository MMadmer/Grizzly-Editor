//! The sixteen kinds of things a scene holds: their names in files and commands, their labels and
//! what a gizmo may do to each.

use serde::Serialize;

/// Number of classes; `by_class` and the tool table are indexed by [`ObjClass::index`].
pub const CLASSES: usize = 16;

/// What the gizmo may do to an object of a class. A selection's is the intersection of its
/// objects' ([`Caps::and`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Caps {
    pub moves: bool,
    pub rotates: bool,
    pub scales: bool,
}

impl Caps {
    pub const ALL: Caps = Caps { moves: true, rotates: true, scales: true };
    pub const NONE: Caps = Caps { moves: false, rotates: false, scales: false };

    /// What both allow.
    #[must_use]
    pub fn and(self, o: Caps) -> Caps {
        Caps {
            moves: self.moves && o.moves,
            rotates: self.rotates && o.rotates,
            scales: self.scales && o.scales,
        }
    }
}

/// The kinds. The discriminant is the id the X-Ray SDK's scene files carry for the class.
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
    /// In id order.
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

    /// The class's name in the X-Ray SDK's scene files (`scene_object.part`).
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

    /// The kind's name in the scene's files and in commands (`classes`, `class`).
    #[must_use]
    pub fn key(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Glow => "glow",
            Self::SceneObject => "object",
            Self::Light => "light",
            Self::Shape => "shape",
            Self::SoundSrc => "sound",
            Self::SpawnPoint => "spawn",
            Self::Way => "way",
            Self::Sector => "sector",
            Self::Portal => "portal",
            Self::SoundEnv => "sound_env",
            Self::Particles => "particles",
            Self::DetailObjects => "details",
            Self::AiMap => "ai_map",
            Self::Wallmark => "wallmarks",
            Self::FogVolume => "fog",
        }
    }

    /// What the author calls one, Title Case.
    #[must_use]
    pub fn desc(self) -> &'static str {
        match self {
            Self::Group => "Group",
            Self::Glow => "Glow",
            Self::SceneObject => "Object",
            Self::Light => "Light",
            Self::Shape => "Shape",
            Self::SoundSrc => "Sound Source",
            Self::SpawnPoint => "Spawn Point",
            Self::Way => "Way Path",
            Self::Sector => "Sector",
            Self::Portal => "Portal",
            Self::SoundEnv => "Sound Environment",
            Self::Particles => "Particle Effect",
            Self::DetailObjects => "Detail Objects",
            Self::AiMap => "AI Map",
            Self::Wallmark => "Wallmarks",
            Self::FogVolume => "Fog Volume",
        }
    }

    /// One, in a sentence: `a light`, `this spawn point`.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Group => "group",
            Self::Glow => "glow",
            Self::SceneObject => "object",
            Self::Light => "light",
            Self::Shape => "shape",
            Self::SoundSrc => "sound source",
            Self::SpawnPoint => "spawn point",
            Self::Way => "way path",
            Self::Sector => "sector",
            Self::Portal => "portal",
            Self::SoundEnv => "sound environment",
            Self::Particles => "particle effect",
            Self::DetailObjects => "detail objects",
            Self::AiMap => "AI map",
            Self::Wallmark => "wallmarks",
            Self::FogVolume => "fog volume",
        }
    }

    /// Several, Title Case: a heading.
    #[must_use]
    pub fn label_plural(self) -> &'static str {
        match self {
            Self::Group => "Groups",
            Self::Glow => "Glows",
            Self::SceneObject => "Objects",
            Self::Light => "Lights",
            Self::Shape => "Shapes",
            Self::SoundSrc => "Sound Sources",
            Self::SpawnPoint => "Spawn Points",
            Self::Way => "Way Paths",
            Self::Sector => "Sectors",
            Self::Portal => "Portals",
            Self::SoundEnv => "Sound Environments",
            Self::Particles => "Particle Effects",
            Self::DetailObjects => "Detail Objects",
            Self::AiMap => "AI Map",
            Self::Wallmark => "Wallmarks",
            Self::FogVolume => "Fog Volumes",
        }
    }

    /// A kind by its key (`light`), its X-Ray SDK name (`ps`) or its label (`Sound Source`),
    /// any case.
    #[must_use]
    pub fn by_key(word: &str) -> Option<Self> {
        let w = word.trim();
        Self::ALL.iter().copied().find(|c| {
            c.key().eq_ignore_ascii_case(w)
                || c.name().eq_ignore_ascii_case(w)
                || c.desc().eq_ignore_ascii_case(w)
        })
    }

    /// [`ObjClass::by_key`].
    #[must_use]
    pub fn by_name(name: &str) -> Option<Self> {
        Self::by_key(name)
    }

    /// What the gizmo may do to one: a glow scales (its radius) and does not turn, a sound
    /// source and a way path only move, sectors and portals stand where their meshes are.
    #[must_use]
    pub fn caps(self) -> Caps {
        let (moves, rotates, scales) = match self {
            Self::SceneObject
            | Self::Shape
            | Self::SoundEnv
            | Self::FogVolume
            | Self::SpawnPoint
            | Self::Group => (true, true, true),
            Self::Light | Self::Particles => (true, true, false),
            Self::Glow => (true, false, true),
            Self::SoundSrc | Self::Way => (true, false, false),
            Self::Sector | Self::Portal | Self::DetailObjects | Self::AiMap | Self::Wallmark => {
                (false, false, false)
            }
        };
        Caps { moves, rotates, scales }
    }

    /// Whether the kind is of placed objects; detail objects, the AI map and wallmarks are
    /// painted layers.
    #[must_use]
    pub fn is_object_tool(self) -> bool {
        !matches!(self, Self::DetailObjects | Self::AiMap | Self::Wallmark)
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
            assert_eq!(ObjClass::by_key(c.key()), Some(c));
        }
        assert_eq!(ObjClass::from_id(16), None);
        assert_eq!(ObjClass::by_name("SPAWN"), Some(ObjClass::SpawnPoint));
        assert_eq!(ObjClass::by_key("ps"), Some(ObjClass::Particles));
        assert_eq!(ObjClass::by_key("Sound Source"), Some(ObjClass::SoundSrc));
        let keys: std::collections::BTreeSet<&str> = ObjClass::ALL.iter().map(|c| c.key()).collect();
        assert_eq!(keys.len(), ObjClass::ALL.len());
    }

    #[test]
    fn a_selections_gizmo_is_what_every_object_of_it_allows() {
        let light_and_glow = ObjClass::Light.caps().and(ObjClass::Glow.caps());
        assert_eq!(light_and_glow, Caps { moves: true, rotates: false, scales: false });
        assert_eq!(ObjClass::SceneObject.caps().and(ObjClass::Shape.caps()), Caps::ALL);
        assert_eq!(ObjClass::Sector.caps().and(ObjClass::SceneObject.caps()), Caps::NONE);
    }
}
