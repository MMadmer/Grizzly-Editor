//! The build: what turns a project into a module of the game.
//!
//! A build generates nothing: it ships the author's files.
//!
//! - [`walk`] - [`BuildWalk`]: ONE list of what a build writes, made from the project's
//!   [`gz_project::FileTree`] before anything is written, and [`LeftOut`] - what it passed by
//!   and why (tooling, a link, `[exclude]`, a key file, a path too long).
//! - [`check`] - [`BuildCheck`]: what an area that knows quests, characters, the PDA or the
//!   screens registers in [`BuildChecks`] (`editor.services`); the build runs them in parallel
//!   after the manifest's own check, and an `Error` stops it. [`BuildChecks::set_bake`] is the
//!   scene's bake that runs first; [`BuildChecks::set_media`] the media assets' publisher and
//!   [`BuildChecks::add_publisher`] any other area's ([`MediaPublisher`]: the items' icon atlas):
//!   what they are made of never ships where it lies, what they bake is added to the walk
//!   ([`BuildWalk::publish`]).
//! - [`target`] - where a build goes and the refusals made before a folder of it is created:
//!   inside the game's gamedata, a link, a target that is / holds / sits in the project.
//! - [`install`] - the clean build written beside the last one in `<target>\modules\.new\<id>`
//!   and swapped in with two renames, unchanged files hard-linked, [`install::wipe_tree`] that
//!   never follows a link; the flat export (`gamedata_<id>`) and its report.
//! - [`job`] - [`BuildRequest`] / [`BuildResult`], [`run_build`] (what `gz-release` stages a
//!   package through), [`Builds`]: the `build` job (one at a time), the results, the Task
//!   Center's [`BuildStatus`] and the clean-target question as data ([`CleanQuestion`]).
//! - [`register`] - `mod.info` (the manifest, its problems, what a build ships) and `mod.build`
//!   (a job the command waits for unless `wait=false`); [`WindowBuilds`] - the window's Build
//!   Into Game for a person, who is asked what the build needs.

pub mod check;
mod cmd;
pub mod install;
pub mod job;
pub mod target;
pub mod walk;

pub use check::{BakeHook, BuildCheck, BuildChecks, MediaPlan, MediaPublisher, Publication, problem_line};
pub use cmd::{BuildTarget, WindowBuilds, build_answer, register};
pub use job::{BuildKind, BuildRequest, BuildResult, BuildStatus, Builds, CleanQuestion, run_build};
pub use walk::{BuildFile, BuildWalk, FileFrom, LeftOut, LeftOutReason, Sources, WalkScope, source_key};
