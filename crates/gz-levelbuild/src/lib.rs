//! Building and running a level: the pipeline of steps that makes a scene a level the game loads,
//! what each step makes, what is stale, Play.
//!
//! # The pipeline
//!
//! - [`steps`] - the steps ([`Step`]: game files, spawn layer, AI map, game graph, spawn,
//!   geometry, details, light, detail light), what each reads and needs, [`Quality`].
//! - [`fingerprint`] - what a step was made from, as hashes of content that survive a restart.
//! - [`state`] - `build.state` beside the build output: every step's fingerprint, time and
//!   quality, the objects as the last build saw them, the AI map the build generated.
//! - [`service`] - [`LevelBuild`]: the record, the status ([`Status`], a query keyed on the
//!   scene's revision - the status bar reads it, nothing polls), the problems of the last build.
//! - `pipeline` - the build as one job of the group `level_build`: what a later step would refuse
//!   checked first, then the stale steps in order, each recorded as it ends; the first error ends
//!   it, Stop ends it within a moment in any step (every long loop asks a [`Stop`]).
//! - [`stock`] - a level the linked game has: its compile steps not needed for the module, what an
//!   earlier build left in its folder (`level.clean` removes it).
//!
//! # The domain (pure, over a scene snapshot)
//!
//! - [`libs::Engine`] - the engine libraries a build checks names against: `shaders.xr`,
//!   `shaders_xrlc.xr` and `gamemtl.xr`.
//! - [`prj::compile`] - `build.prj` ([`gz_fmt_level::BuildProject`]): static objects welded into
//!   one vertex and face list with their materials, smoothing groups and visibility cells,
//!   multiple-usage objects as models and references (with their LOD quads and the merged
//!   `level_lods` textures), point lights (soft ones sampled), the hemisphere and the sun, glows,
//!   portals, light controls. The cells are completed first (`gz_scene::objtools::zones`).
//! - [`breakable`] - the breakable and climable objects added to `level.spawn`.
//! - [`files`] - `level.ltx`, the game files (`level.spawn`, `level_rs.spawn`, `level.game`,
//!   `level.env_mod`, `level.snd_static`, `level.ps_static`, `level.fog_vol`), `level.hom`,
//!   `level.som`, `level_stat.tga` / `.txt`.
//! - [`details`] - `build.details` and `build_details.dds`.
//! - [`wallmarks`] - the wallmarks: `level.wallmarks` and their faces in `build.prj`.
//! - [`merge`] - the LOD atlas's packer and its UV remap.
//! - [`nav::LevelNav`] - the level graph's lookups: a position's vertex, the straight-line check,
//!   the A* distance, the flooder.
//! - [`graph`] - the level graph of the AI map, the level's game graph with its cross table, and
//!   a game graph of several levels joined through their connection points.
//! - [`spawn`] - the spawn file of the scene: every spawn element on its level and game vertex,
//!   the patrol paths of the way objects, the game graph embedded.
//! - [`validate`] - what a scene must hold before it is built.
//!
//! # Running
//!
//! - [`compile`] - the level compilers in the editor's process (`gz_lc::compile`,
//!   `gz_lc::compile_details`) on a level folder, every core, the compiler's phases as the
//!   job's stage.
//! - [`play_root`] - what may be Play's game root; [`play`] - the root made and refreshed (the
//!   game's loose gamedata mirrored as hard links), the level staged - or, on a level the game has,
//!   the module installed into its `modules\` ([`module_play`]: Play's Player Start, the new game's
//!   mode); the installed engine runs in place on the root's fs config, and an engine that does not
//!   say it can is refused before anything is built.
//! - [`process`] - the game as a `tokio::process` child: its output followed into the job's
//!   stage, Stop ends the whole process tree.
//! - [`register`] - the `level` area: `level.build`, `level.status`, `level.problems`, `level.clean`,
//!   `level.validate`, `level.play`, `level.export`, `level.compile`.
//!
//! The build writes into the open project's `gamedata\levels\<level>` and `gamedata\spawns`,
//! never into the editor's folder or the linked game. The level path is one folder name.

#![forbid(unsafe_code)]

pub mod breakable;
mod cmd;
pub mod compile;
pub mod details;
pub mod files;
pub mod fingerprint;
pub mod folder;
pub mod graph;
mod hemi;
mod inspect;
pub mod libs;
pub mod merge;
pub mod module_play;
pub mod nav;
mod pipeline;
pub mod play;
pub mod play_root;
pub mod prj;
pub mod process;
pub mod report;
pub mod service;
pub mod spawn;
pub mod state;
pub mod steps;
pub mod stock;
pub mod validate;
pub mod wallmarks;

pub use cmd::{JOB_BUILD, JOB_PLAY_MODULE, JOB_RUN, register};
pub use service::{LevelBuild, LevelProblem, Running, Status, StepState, StepStatus};
pub use steps::{Quality, Step};

/// What the level build plugs into the module build once the editor exists: the module's piece
/// of a level of the game built first when it is due (`gz_build::PrepareHook`).
pub fn install(editor: &gz_editor::Editor) {
    gz_build::BuildChecks::of(editor)
        .add_prepare(|ed, session, ctx, flat| Box::pin(cmd::before_module_build(ed, session, ctx, flat)));
}

/// Why a build step stopped: a sentence for the author.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct BuildError(pub String);

impl BuildError {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// The answer of work that was stopped (the job's Stop).
    #[must_use]
    pub fn stopped() -> Self {
        Self(STOPPED.to_owned())
    }

    #[must_use]
    pub fn is_stopped(&self) -> bool {
        self.0 == STOPPED
    }
}

/// What a stopped step answers: the job's own word for it.
pub const STOPPED: &str = "Stopped.";

/// Asked by every long loop of a step, per item or per chunk: `true` once the job is stopped.
pub type Stop<'a> = &'a (dyn Fn() -> bool + Sync);

/// Never stops: what a caller without a job passes.
pub const NEVER: Stop<'static> = &|| false;

pub type Result<T, E = BuildError> = std::result::Result<T, E>;
