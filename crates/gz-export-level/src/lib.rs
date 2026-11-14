//! The module's level layer: what a scene adds to, takes from and lays over a level of the
//! game, without a single base file of the game rewritten.
//!
//! The files are part of the contract with the game (`docs/ARCHITECTURE.md` section 9) and are
//! written as the game reads them: `\r\n` line ends, a fixed key order, `%f` / `%g` / `%u` /
//! `%08x` as the C runtime prints them ([`cfmt`]).
//!
//! # The domain (pure)
//!
//! Every export reads a SNAPSHOT of the scene - the author goes on editing while it runs - plus
//! what was gathered from the disk beforehand, and returns the files to write and the report:
//!
//! - [`spawn::bake`] - `spawn\<level>.xspawn`: `[obj:<name>]` with `op = add` (props, spawn
//!   elements with their real section, visual, fixed bones, smart cover, lamp, zone shapes and
//!   the `@spawn/custom_data/<level>_<op>.ltx` logic with a stalker's `[xms_order]`),
//!   `op = start` (the Player Start), `op = remove` (a base spawn the author took out) and
//!   `[path:<name>]` (way paths); the merge with the ops already in the file
//!   ([`spawn::merge_spawn_text`]) or the mirror pass of the build. [`spawn::SpawnInput`] names
//!   what it reads, [`spawn::SpawnBake`] what it gives.
//! - [`overlay::collision`] - `levels\<level>\overlay.xcform`: the static objects' triangles in
//!   world space with their game materials, the cut list of the file kept.
//! - [`overlay::cut`] - the subtractive delta: object bounds as cut boxes in `overlay.xcform` and
//!   `[cut_N]` hide entries in `overlay_visuals.ltx`, accumulating, never repeated.
//! - [`overlay::visuals`] - `levels\<level>\visuals\<name>.ogf` (each static object exported in
//!   world space through `gz-fmt-ogf`, in parallel) and their `[v_N]` registry.
//! - [`starts`] - the open scene's Player Starts, which `gz-world`'s `E201` counts for the
//!   scene's level.
//! - [`visuals`] - how a model is found the way the game will find it ([`visuals::Visuals`]),
//!   X-Ray's "there, but not drawn" ([`visuals::ogf_is_ghost`]) and the fixed bones a model
//!   really has ([`visuals::checked_fixed_bones`]).
//!
//! # The editor side
//!
//! - [`gather`] - the reads an export needs, on the IO pool, in parallel ([`gather::Gathered`]),
//!   and [`gather::apply`] - the writes, after the CPU work. Nothing of the game is ever
//!   written, and nothing of another module is read or written.
//! - [`export_spawn`], [`export_collision`], [`export_cut`], [`export_visuals`] - the layers of a
//!   selection (or of objects named, or of the whole scene, [`Layer`]) as `level.export` runs
//!   them; [`bake_layer`] - the spawn layer of the whole scene, a step of the level build;
//!   [`install`] fills `gz-build`'s bake slot (the same pass, run before a module build) and
//!   `gz-world`'s `SceneStarts` slot.
//! - [`lookup::LiveScene`] - `gz-nq`'s `SceneLookup` (the scene's restrictors, story ids, way
//!   paths and level for the quest pickers), set when a project opens.
//! - [`ExportService`] - the linked game's patrol names (`gz-world` reads `all.spawn`).

#![forbid(unsafe_code)]

pub mod cfmt;
mod cmd;
pub mod gather;
pub mod lookup;
pub mod names;
pub mod overlay;
mod service;
pub mod spawn;
pub mod starts;
pub mod visuals;

pub use cmd::{Layer, bake_layer, export_collision, export_cut, export_spawn, export_visuals, install};
pub use service::ExportService;

/// Why an export did not write its files: a sentence for the author.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ExportError(pub String);

impl ExportError {
    pub(crate) fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }
}
