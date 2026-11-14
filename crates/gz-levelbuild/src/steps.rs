//! The steps of a level build: what each makes, what it reads, what it needs first.
//!
//! ```text
//! game          level.ltx and the game files (spawn packets, ways, sounds, particles, fog)
//! spawn_layer   the module's spawn layer (spawn\<level>.xspawn), the whole scene
//! ai_map        level.ai, from the AI map - generated over the ground when the build made it
//! game_graph    level.graph: the level's game graph and cross table, from level.ai
//! spawn         gamedata\spawns\editor.spawn: every spawn point on the level graph, the game graph
//!               joined with the linked game's (what Play runs)
//! geometry      build.prj (the visibility cells completed), the LOD atlas, HOM, SOM, wallmarks
//! details       build.details and its texture, from the painted detail slots
//! light         the compiler on build.prj: level, level.geom, level.cform, the light maps
//! detail_light  the detail compiler: level.details
//! ```
//!
//! [`Step::ORDER`] is one order in which every step comes after what it needs: the quick steps
//! first, the compiler last.

use gz_scene::ObjClass;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// One step of the level build.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Step {
    /// `level.ltx` and the game files: spawn packets, patrol paths, sounds, particles, fog.
    Game,
    /// The module's spawn layer of the whole scene (`spawn\<level>.xspawn`).
    SpawnLayer,
    /// `level.ai`, the AI map the game's NPCs walk on.
    AiMap,
    /// `level.graph`, the level's game graph and its cross table.
    GameGraph,
    /// The spawn file Play runs (`gamedata\spawns\editor.spawn`): the level's spawn points on
    /// the level graph, its game graph joined with the linked game's.
    Spawn,
    /// `build.prj` - the static geometry, lights, glows, visibility cells - and the files beside it.
    Geometry,
    /// `build.details` and its texture.
    Details,
    /// The level compiler: geometry, collision and light maps.
    Light,
    /// The detail compiler: `level.details`.
    DetailLight,
}

impl Step {
    /// Every step, each after what it needs.
    pub const ORDER: [Step; 9] = [
        Step::Game,
        Step::SpawnLayer,
        Step::AiMap,
        Step::GameGraph,
        Step::Spawn,
        Step::Geometry,
        Step::Details,
        Step::Light,
        Step::DetailLight,
    ];

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Game => "game",
            Self::SpawnLayer => "spawn_layer",
            Self::AiMap => "ai_map",
            Self::GameGraph => "game_graph",
            Self::Spawn => "spawn",
            Self::Geometry => "geometry",
            Self::Details => "details",
            Self::Light => "light",
            Self::DetailLight => "detail_light",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        Self::ORDER.into_iter().find(|s| s.name() == word)
    }

    /// The name a person reads.
    #[must_use]
    pub fn title(self) -> &'static str {
        match self {
            Self::Game => "Game Files",
            Self::SpawnLayer => "Spawn Layer",
            Self::AiMap => "AI Map",
            Self::GameGraph => "Game Graph",
            Self::Spawn => "Spawn",
            Self::Geometry => "Geometry",
            Self::Details => "Details",
            Self::Light => "Light",
            Self::DetailLight => "Detail Light",
        }
    }

    /// The job's stage while the step runs.
    #[must_use]
    pub fn stage(self) -> &'static str {
        match self {
            Self::Game => "Writing the game files",
            Self::SpawnLayer => "Baking the spawn layer",
            Self::AiMap => "Building the AI map",
            Self::GameGraph => "Building the game graph",
            Self::Spawn => "Building the spawn",
            Self::Geometry => "Building the geometry",
            Self::Details => "Writing the details",
            Self::Light => "Compiling the geometry and light",
            Self::DetailLight => "Lighting the details",
        }
    }

    /// The steps whose output this one reads.
    #[must_use]
    pub fn needs(self) -> &'static [Step] {
        match self {
            Self::GameGraph => &[Step::AiMap],
            Self::Spawn => &[Step::GameGraph],
            Self::Light => &[Step::Geometry],
            Self::DetailLight => &[Step::Details, Step::Light],
            Self::Game | Self::SpawnLayer | Self::AiMap | Self::Geometry | Self::Details => &[],
        }
    }

    /// The object classes the step reads (its other inputs are layers and settings).
    #[must_use]
    pub fn classes(self) -> &'static [ObjClass] {
        match self {
            Self::Geometry => {
                &[ObjClass::SceneObject, ObjClass::Light, ObjClass::Glow, ObjClass::Sector, ObjClass::Portal]
            }
            Self::Game => &[
                ObjClass::SceneObject,
                ObjClass::Shape,
                ObjClass::SoundSrc,
                ObjClass::SpawnPoint,
                ObjClass::Way,
                ObjClass::SoundEnv,
                ObjClass::Particles,
                ObjClass::FogVolume,
            ],
            Self::AiMap => &[ObjClass::SceneObject, ObjClass::Shape],
            Self::GameGraph => &[ObjClass::SpawnPoint],
            Self::Spawn => &[ObjClass::SpawnPoint, ObjClass::Way],
            Self::SpawnLayer => &ObjClass::ALL,
            Self::Details | Self::Light | Self::DetailLight => &[],
        }
    }

    /// The files the step writes into the level folder (the spawn step writes its file into
    /// `gamedata\spawns`, the spawn layer into the module's `spawn` folder).
    #[must_use]
    pub fn files(self) -> &'static [&'static str] {
        match self {
            Self::Game => &["level.ltx", "level.spawn", "level.game"],
            Self::AiMap => &["level.ai"],
            Self::GameGraph => &["level.graph"],
            Self::Geometry => &["build.prj"],
            Self::Details => &["build.details"],
            Self::Light => &["level", "level.geom", "level.cform", "build.lights", "build.cform"],
            Self::DetailLight => &["level.details"],
            Self::Spawn | Self::SpawnLayer => &[],
        }
    }

    /// Whether the step runs only when the scene has what it is made of.
    #[must_use]
    pub fn optional(self) -> bool {
        matches!(self, Self::Details | Self::DetailLight)
    }

    /// The steps that need this one, directly or not.
    #[must_use]
    pub fn dependents(self) -> Vec<Step> {
        let mut out: Vec<Step> = Vec::new();
        for s in Self::ORDER {
            if s.needs().iter().any(|n| *n == self || out.contains(n)) {
                out.push(s);
            }
        }
        out
    }
}

/// How much time the compiler spends on the light.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Quality {
    /// Quick light maps: a tenth of a texel per metre, one sample, the coarsest sky and sun.
    Draft = 0,
    /// The level's own build settings (Level Settings).
    #[default]
    Standard = 1,
    /// The high-quality preset: ten texels per metre, nine samples, the finest sky and sun.
    Final = 2,
}

impl Quality {
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Standard => "standard",
            Self::Final => "final",
        }
    }

    #[must_use]
    pub fn parse(word: &str) -> Option<Self> {
        [Self::Draft, Self::Standard, Self::Final].into_iter().find(|q| q.name() == word)
    }

    /// The level options a build of this quality compiles with.
    #[must_use]
    pub fn apply(self, opts: &gz_scene::tools::LevelOptions) -> gz_scene::tools::LevelOptions {
        let mut o = opts.clone();
        match self {
            Self::Draft => o.set_quality(0),
            Self::Final => o.set_quality(1),
            Self::Standard => {}
        }
        o
    }
}

/// What a build is asked to make the level with, besides the scene.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Options {
    pub quality: Quality,
    /// The compiler's flags (`-nosun -noise ...`), as one line.
    pub flags: String,
    /// The linked game's folder: the spawn joins its game graph.
    pub game: String,
}

/// The steps a request names, with what they need, in [`Step::ORDER`].
#[must_use]
pub fn with_needs(asked: &[Step]) -> Vec<Step> {
    let mut want: Vec<Step> = asked.to_vec();
    let mut i = 0;
    while i < want.len() {
        for n in want[i].needs() {
            if !want.contains(n) {
                want.push(*n);
            }
        }
        i += 1;
    }
    Step::ORDER.into_iter().filter(|s| want.contains(s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_step_comes_after_what_it_needs() {
        for (i, s) in Step::ORDER.iter().enumerate() {
            for n in s.needs() {
                let at = Step::ORDER.iter().position(|x| x == n).unwrap_or(usize::MAX);
                assert!(at < i, "{} needs {} but comes first", s.name(), n.name());
            }
        }
        for s in Step::ORDER {
            assert_eq!(Step::parse(s.name()), Some(s));
        }
    }

    #[test]
    fn a_step_brings_what_it_needs_in_order() {
        assert_eq!(
            with_needs(&[Step::DetailLight]),
            [Step::Geometry, Step::Details, Step::Light, Step::DetailLight]
        );
        assert_eq!(with_needs(&[Step::Spawn]), [Step::AiMap, Step::GameGraph, Step::Spawn]);
        assert_eq!(Step::Geometry.dependents(), [Step::Light, Step::DetailLight]);
        assert_eq!(Step::AiMap.dependents(), [Step::GameGraph, Step::Spawn]);
    }
}
