//! A module's loading and menu screens and their music: `mod.ltx [screens]`, `[screen.<id>]`,
//! `[track.<id>]`, kept in the manifest store of `gz-mod` and saved as they are edited.
//!
//! The C++ specification is the model of `EditorScreens.cpp` and `EditorModManifest.cpp`
//! (`ParseScreen*Key`, `AppendScreens`), its `Import`, `Validate` and the MCP handlers; what a
//! screen means in the game is the game repo's `docs/dead-air/MODDING.md`, "Screens and music".
//!
//! A screen is a stack of layers, one file each: picture, GIF, video, shader or HTML. THE KIND
//! IS THE FILE'S EXTENSION ([`file_kind`]); nothing is declared.
//!
//! # API
//!
//! - [`model`] - [`Screens`], [`Screen`], [`Layer`], [`Track`], [`Fit`] and the surface / hide
//!   masks; [`Screens::parse`] reads them off a [`gz_mod::ManifestDoc`], [`Screens::edits`] gives
//!   the [`gz_mod::Edit`]s that write them back as the C++ wrote them (sections kept in place
//!   when their order holds).
//! - [`import`] - [`import()`]: the one way a file enters (inside the project it stays, from
//!   outside it is copied into `screens\`, PNG / JPG / TGA / BMP becomes a `.dds` through
//!   `gz-fmt-tex`, a `.ps` gets its `.s`); [`plan`] - the same decision without the disk;
//!   [`file_kind`], [`FileKind`]. Imports are blocking file work for the IO pool; a drop of
//!   several files imports them in parallel, and two imports at once never write over each
//!   other.
//! - [`validate()`] - `E401`..`W411` with the C++ texts.
//! - [`register`]: `screens.info`, `screens.set`, `screens.import`; [`install`]: the build's check (an `E4xx`
//!   refuses the build); [`GameLevels`]: where the world area hangs the linked game's levels
//!   for `W407`; [`answer`] - the reply both commands give.

mod cmd;
pub mod import;
pub mod model;
pub mod validate;

pub use cmd::{GameLevels, LevelsHook, answer, install, problems, register};
pub use import::{FileKind, Plan, file_kind, import, plan};
pub use model::{Fit, Layer, Screen, Screens, Track};
pub use validate::validate;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;
