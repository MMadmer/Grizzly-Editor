//! The mode's world: what a module's mode keeps of the ORIGINAL game, where the player may go,
//! which ways start closed and what a new game begins with.
//!
//! The C++ specification is `EditorWorld.h/.cpp` (the reader of the linked game's `all.spawn`,
//! `Starts`, `Validate`, the JSON of `[world]` / `[start_kit]`, `GameBaseKit`, the MCP
//! handlers) and the `[world]` / `[start_kit]` parts of `EditorModManifest.cpp` (`Load`,
//! `Save`). Its drawing is the World Rules tab, a later package.
//!
//! **The game is never written.** The linked game's `all.spawn` is read - its levels, its level
//! changers, its patrol names - and nothing else; the engine lays a module's world over the
//! game in memory when a new game is composed.
//!
//! # The domain (pure)
//!
//! - [`model`] - [`World`] (`[world]`: the five [`RULES`], `levels`, `player_choice` - three at
//!   most, `area`, `locked_levels`, `locked`, unknown lines kept) and [`StartKit`] (`[start_kit]`:
//!   money, perks, items). Both are read from `gz_mod`'s [`gz_mod::ManifestDoc`] (its raw section
//!   lines) and written back as [`gz_mod::Edit`]s in the C++ layout; their JSON;
//!   [`model::base_kit`] (`[shared]` of the game's `plugins\faction_loadouts.ltx`).
//! - [`game`] - [`GameWorld`]: [`game::read_all_spawn`] over the bytes of `all.spawn` -
//!   `gz-fmt-spawn` for the container, the game graph and the patrols (indexed in parallel on
//!   the caller's pool), `gz-entities` for a level changer's destination.
//! - [`starts`] - [`Start`], [`starts::read_starts`]: the `op = start` entries of a spawn layer.
//! - [`validate`] - [`validate::validate`]: `E201`, `E202`, `W203`..`W206`, `E207`, `W208` with
//!   the C++ messages.
//!
//! # The service and the commands
//!
//! - [`WorldService`] (project scope): the linked game's world as a `Query` keyed by the game
//!   link's revision, parsed once on the CPU pool; the captions, the Player Starts (the baked
//!   spawn layers and the open scene's, through [`SceneStarts`]), the problems. When it is first
//!   made (on project open) it fills `gz_nq`'s `WorldLookup` slot.
//! - [`SceneStarts`] (editor scope) - the slot the scene area fills: its level and the Player
//!   Starts placed in it.
//! - [`register`] - `world_get`, `world_set`, `world_transitions` (`world_view` is the tab's: not
//!   here); [`install`] - the `world` field of `mod_manifest`, `gz-screens`' `GameLevels`, the
//!   build check, the service made on project open.

mod cmd;
pub mod game;
pub mod model;
mod service;
pub mod starts;
pub mod validate;

#[cfg(test)]
mod tests;

pub use cmd::{install, register, world_answer};
pub use game::{GameTransition, GameWorld, read_all_spawn};
pub use model::{KEY_WIDTH, KitItem, MAX_PLAYER_CHOICES, RULES, StartKit, World, rule_index};
pub use service::{SceneStarts, WorldService};
pub use starts::{Start, read_starts};
