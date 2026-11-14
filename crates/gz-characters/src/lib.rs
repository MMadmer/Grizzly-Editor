//! The module's own NPCs: one `.npcasset` per character, declarative Lua the game reads as it
//! is. The editor generates nothing for a character - the game makes the specific character,
//! the profile, the strings and a trader's config itself; the build only refuses a module whose
//! characters would not work. The format is `docs/assets/CHARACTER.md`.
//!
//! Its drawing is the Characters tab (`gz-ui-world`), which binds to the RAW stored values
//! ([`text_raw`]).
//!
//! # The domain (pure)
//!
//! - [`model`] - [`Character`] with [`Supply`], [`Trade`], [`TradeRow`]: **unset stays unset**
//!   (an empty string, a `has_*` flag), or a child would override its base with a default it
//!   never asked for. [`parse`] / [`load_text`] / [`model::from_value`] read a file (a newer
//!   `npc` version refused), [`model::to_value`] is the character as a save compares it,
//!   [`write`] the canonical text of a new file (a `{ rus = ... }` text alone collapses back to
//!   a plain string), [`template`] the start of a new character, [`game_id`] `<module>_<id>`.
//!   The text helpers [`text_in`] (with the game's fallback), [`text_raw`] (what is stored),
//!   [`text_set`], [`text_collapse`].
//! - [`resolve`] - [`resolve::resolve`]: every `base` chain flattened once (unset fields
//!   inherited, kits merged, a shop inherited or replaced whole), `E111` / `E112`.
//! - [`validate`] - [`validate::validate`] over a [`validate::Cx`] (`E102`..`E110`, `W105`,
//!   `W107`, `W114`..`W117`, `E118`, `E119`, `W120`..`W122`, `E123`); the lookups come from
//!   `gz_nq::Pickers`, the models through [`validate::Models`].
//! - [`game`] - what is read of the linked game: [`game::trade_categories`]
//!   (`misc\trade\trade_presets.ltx`), [`game::ogf_bone_names`],
//!   [`game::stalker_damage_bones`].
//!
//! # The service
//!
//! - [`Characters`] (project scope, `session.services`): the project's characters as one
//!   [`Roster`] - a `Query` keyed by the watcher's changes of the `.npcasset` files (their
//!   tree stamps without a watcher) and `[exclude]`: every file read in parallel, parsed,
//!   resolved. [`Characters::save`] writes an edited character as the difference into its
//!   file. [`Characters::briefs`] comes from the roster, and it fills `gz_nq`'s
//!   `CharacterLookup` slot when it is first made (on project open). [`Characters::checker`]
//!   gathers what validation reads from the linked game.
//! - [`install`] - the `file.create` template, the build check, the character kind of
//!   `gz-asset` (`asset.check`, `asset.reference kind=character`), the roster made on project
//!   open.

mod cmd;
pub mod game;
pub mod model;
pub mod resolve;
mod service;
pub mod validate;

#[cfg(test)]
mod tests;

pub use cmd::{character_json, install, kind};
pub use model::{
    Character, Supply, Trade, TradeRow, game_id, load_text, parse, template, text_collapse, text_empty,
    text_in, text_languages, text_raw, text_set, valid_id, write,
};
pub use service::{Characters, Checker, Roster};
