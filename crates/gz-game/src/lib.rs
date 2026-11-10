//! What the linked game's files mean: its configs, string tables, texture sheets, modes, spawn
//! roster and version. Read-only: nothing here writes a file of the game.
//!
//! The C++ specification is `EditorGameConfigs` (the `system.ltx` tree), `EditorGameModes`
//! (the modes), the Spawn tool's list (`UISpawnTool`), `EditorMod::LinkedGameVersion` and the
//! handlers named in [`register`]. The files themselves are `gz-vfs`'s [`gz_vfs::GameFiles`],
//! mounted and kept by `gz-project`'s [`gz_project::Project`].
//!
//! # API
//!
//! - [`GameLink`] (project scope, `session.services`; [`GameLink::of`]): [`GameLink::files`] /
//!   [`GameLink::snapshot`] (the index and its revision), [`GameLink::rev`] (moves when the link
//!   changes or [`GameLink::refresh`] is asked), [`GameLink::root`]; derived values, each a
//!   `Query` keyed by `rev()` and computed on first use - [`GameLink::configs`] /
//!   [`GameLink::config_tree`] (the `system.ltx` tree with its warnings), [`GameLink::language`],
//!   [`GameLink::strings`] / [`GameLink::strings_as`], [`GameLink::texture_sheets`],
//!   [`GameLink::modes`], [`GameLink::spawn_sections`], [`GameLink::game_version`]. An unlinked
//!   project answers [`GameError::NotLinked`]; nothing panics.
//! - [`Bundled`] (editor scope): the copies of game files the editor ships
//!   (`<editor root>\gamedata`), [`Bundled::read`], [`Bundled::config_tree`],
//!   [`Bundled::spawn_sections`]; [`read_game_or_bundled`] prefers the linked game and falls
//!   back to the bundled copy ([`Provenance`] says which); [`spawn_roster`] does the same for
//!   the Spawn tool's list.
//! - The domain, pure: [`configs::load_tree`] (breadth-first include discovery, parallel reads
//!   and parses, then the engine's assembly) over a [`configs::FileSource`]; [`modes`]
//!   ([`modes::scan`], [`modes::checkboxes`], [`modes::provides_mode`]); [`spawn`]
//!   ([`spawn::sections`], [`spawn::list_path`]); [`version::exe_version`].
//! - [`register`]: `game_link`, `game_modes`, `list_spawn_sections`.

#![forbid(unsafe_code)]

mod cmd;
pub mod configs;
pub mod modes;
mod service;
pub mod spawn;
pub mod version;

pub use cmd::register;
pub use configs::{ConfigTree, SYSTEM_LTX};
pub use modes::{GameMode, ModeScan};
pub use service::{Bundled, GameError, GameLink, Provenance, read_game_or_bundled, spawn_roster};
pub use spawn::SpawnSection;
