//! The full level build of a scene and the runs: the build project for the level compiler, the
//! game side files, the collision form, the AI map, the game graph, the spawn file, the level
//! compile in the editor's process and the game as a process.
//!
//! The C++ specification is `LevelEditor/Editor/Builder/*` (`SceneBuilder`: `CompileStatic`,
//! `BuildLTX`, `BuildGame`, the HOM / SOM models, the scene statistic, the tools' exports),
//! `Scene/CFromBuilder.cpp`, `Scene/scene_spawn.cpp`, `LevelEditor/AI/*` (the level graph of the
//! AI Map tool, `CGameGraphBuilder`, `CLevelSpawnConstructor` / `CGameSpawnConstructor`, the
//! patrol paths), the engine's `ILevelGraph` lookups they use, xrAI's `CGraphMerger` for a game
//! graph of several levels, `UI_LevelTools.cpp` (`RunGame`, `RunXrLC`, `RunXrDO`) and the Level
//! toolbar of `UITopBarForm.cpp`.
//!
//! # The domain (pure, over a scene snapshot)
//!
//! - [`libs::Engine`] - the engine libraries a build checks names against: `shaders.xr`
//!   (engine shaders), `shaders_xrlc.xr` (compiler shaders and their collision / render flags),
//!   `gamemtl.xr` (game materials by name, id and index).
//! - [`prj::compile`] - `build.prj` ([`gz_fmt_level::BuildProject`]): static objects welded
//!   into one vertex and face list with their materials, smoothing groups and sectors,
//!   multiple-usage objects as models and references (with their LOD quads and the merged
//!   `level_lods` textures), point lights (soft ones sampled), the hemisphere and the sun,
//!   glows, portals, light controls. [`prj::Compiled`] carries the scene statistic.
//! - [`breakable`] - the breakable and climable objects `BuildGame` adds to `level.spawn`
//!   (`GeometryPartExtractor`: parts, fragments, `meshes\brkbl#N.ogf`).
//! - [`files`] - `level.ltx`, the game files of `BuildGame` (`level.spawn`, `level_rs.spawn`,
//!   `level.game`, `level.env_mod`, `level.snd_static`, `level.ps_static`, `level.fog_vol`),
//!   `level.hom`, `level.som`, `build.aimap`, `level_stat.tga` / `.txt`, the collision form
//!   (`CFromBuilder`) as a [`gz_fmt_level::CForm`].
//! - [`details`] - `build.details` and `build_details.dds` (`EDetailManager::Export`).
//! - [`wallmarks`] - the Wallmark tool's marks: `level.wallmarks` and their faces in `build.prj`.
//! - [`merge`] - `CreateMergedTexture`'s packer (the LOD atlas) and its UV remap.
//! - [`nav::LevelNav`] - the engine's level graph lookups: a position's vertex, inside, plane
//!   height, the straight-line check, the A* distance, the flooder.
//! - [`graph`] - the level graph of the AI Map tool ([`graph::level_graph`]), the level's game
//!   graph with its cross table ([`graph::build_game_graph`]), and a game graph of several levels
//!   joined through their connection points ([`graph::merge_levels`]).
//! - [`spawn`] - the spawn file of the scene: every spawn element on its level and game vertex,
//!   the patrol paths of the way objects, the artefact points of the zones, the game graph
//!   embedded ([`spawn::build_spawn`]).
//!
//! # The editor side
//!
//! - [`compile`] - the level compilers in the editor's process: `gz_lc::compile` (geometry and
//!   light) and the detail compile on a level folder of the project, a job on the CPU pool with
//!   the compiler's phases as its stage, its log in the Output Log, its report as the result.
//! - [`play_root`] - what may be Play's game root ([`play_root::check_root`]: the questions of a
//!   project's deletion, a project anywhere on the way, only a missing or empty folder or one
//!   carrying Play's marker) and [`play`] - the root made and refreshed, clearing only what Play
//!   writes there, the level staged; the installed engine runs in place on the root's fs config
//!   when it says it can (`EngineAbilities`, [`play::PlayMode`]), an older one from copies of it
//!   and the libraries it needs (the game's runtime list and the import tables, `pe`).
//! - [`process`] - the game, the one program Play starts, as a `tokio::process` child: its
//!   output followed into the job's stage, Stop ends the whole process tree, one that does not
//!   start is a sentence.
//! - [`LevelBuild`] (`session.services`) - what is built of the scene and when it went stale
//!   ([`LevelBuild::rebuild`] answers the status bar's Rebuild chip), the derived data kept in
//!   memory (collision form, level graph, game graph), the running process.
//! - [`register`] - one command per Level toolbar button and Build menu row ([`COMMANDS`]:
//!   `build_cform`, `build_aimap`, `build_game_graph`, `build_all`, `play`, `play_game`,
//!   `play_clean`, `play_stop`, `reload_configs`, `make_all`, `make_game`, `make_details`,
//!   `make_hom`, `make_som`, `make_aimap`, `compile_geometry`, `compile_details`),
//!   `level_compile` (a level folder compiled on its own), `level_rebuild_status` (`{collision,
//!   ai_map, game_graph}` for the Rebuild chip) and `level_build` (the same by `action`, and
//!   `status`, `play_root` - Play's root made without a game started). Every build is the job `level_build` (one at a time) with stages and Stop; a
//!   compile or the game runs as the job `level_run` (group `game`). Play starts the real game on the baked
//!   scene (docs/RUST_MIGRATION.md D1): there is no game
//!   inside the editor's process.
//!
//! The build writes into the open project's `gamedata\levels\<level>` and `gamedata\spawns` (the
//! C++ `$game_levels$` / `$game_spawn$`, which `EditorProject` remounts into the project), never
//! into the editor's folder or the linked game. The level path is one folder name, and the folder is emptied only when it
//! really lies below the levels folder and is no link ([`folder`]).

#![forbid(unsafe_code)]

pub mod breakable;
mod cmd;
pub mod compile;
pub mod details;
pub mod files;
pub mod folder;
pub mod graph;
mod hemi;
pub mod libs;
pub mod merge;
pub mod nav;
mod pe;
pub mod play;
pub mod play_root;
pub mod prj;
pub mod process;
mod service;
pub mod spawn;
pub mod validate;
pub mod wallmarks;

pub use cmd::{COMMANDS, register};
pub use service::{LevelBuild, Rebuild, Running};

/// Why a build step stopped. The texts are the C++ messages.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BuildError(pub String);

impl BuildError {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}

pub type Result<T, E = BuildError> = std::result::Result<T, E>;
