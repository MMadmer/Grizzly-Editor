//! The game as the editor's library (`docs/GAME_LIBRARY.md`): everything of the linked game a
//! level designer names, described in text in the editor's own data and kept current.
//!
//! - [`sync`] - the pass that brings a library up to date: per kind what is new, changed and
//!   gone, described on every core ([`models`], [`media`], [`lists`]), written as cards and
//!   catalogs ([`lua`], [`layout`]) with what they were made from ([`state`]).
//! - [`service`] - [`GameLibrary`]: which game, the state, the sync as a job of the editor,
//!   started when the editor is up and when another game takes the place of the one there was.
//! - [`index`] - the catalogs read back for searching.
//! - [`provider`] - a compiled model as an object of the scene's library (`ref = "<name>.ogf"`).
//! - [`levels`] - the game's levels as scenes, their geometry and their passports.
//! - [`base`] - a level of the game as what a module's scene lies over (`base = "<level>"`).
//! - `game.library`, `game.read`, `game.search`, `asset.find`, `asset.card` ([`register`]).

#![forbid(unsafe_code)]

pub mod base;
mod cmd;
pub mod index;
pub mod layout;
pub mod levels;
pub mod lists;
pub mod lua;
pub mod media;
pub mod models;
pub mod names;
pub mod provider;
pub mod service;
pub mod shots;
pub mod state;
pub mod sync;

use std::sync::Arc;

pub use service::GameLibrary;

/// Hangs the library on the editor. With `follow` the library is kept current by itself: a
/// run that serves commands for as long as it lives (a window, `-headless`) follows the game; a
/// commandlet brings the library up to date when a command of it asks.
pub fn install(editor: &Arc<gz_editor::Editor>, follow: bool) {
    provider::install(editor);
    base::install(editor);
    names::install(editor);
    if follow {
        service::watch(editor);
    }
}

/// Registers the library's commands.
pub fn register(reg: &mut gz_editor::Registry) {
    cmd::register(reg);
    shots::register(reg);
}
