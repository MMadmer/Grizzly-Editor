//! The module's inventory items: one `.itemasset` per item, declarative Lua the game reads as
//! it is - a NEW item (`base`), or a CHANGE of one that exists (`edit`). The game makes the
//! sections, the strings and the use scene itself; the editor reads what an item resolves
//! to, checks it, and bakes the pictures the items name into the module's icon atlas. The
//! format is `docs/assets/ITEM.md`.
//!
//! **An item says what the item is and nothing of another thing** (`docs/ASSETS.md`, "One
//! thing, one place"). Who sells it, who carries it and what makes it are rows of the side that
//! owns them, each kind a file of its own ([`tables`]): a trader's shelf (`.tradeasset`), what
//! the dead carry (`.lootasset`), recipes (`.recipeasset`) - `docs/assets/TRADER.md`,
//! `LOOT.md`, `RECIPES.md`. The item only shows them as a fact ([`Roster::found`]).
//!
//! # The domain (pure)
//!
//! - [`fields`] - the keys of every level of the file, one table each: the reader, the known
//!   keys and the reference read them.
//! - [`model`] - [`Item`] (unset stays unset) with [`Changes`], [`SetValue`], [`Icon`], [`Hud`],
//!   [`OwnedSection`], [`Scene`]; [`model::from_value`]
//!   / [`load_text`] read a file with the problems of its own shape ([`Issue`], `I427`),
//!   [`model::to_value`] is the item as a save compares it, [`write`] the canonical text of a
//!   new file, [`template`] the start of one, [`model::diff_rules`] how a save writes the
//!   difference.
//! - [`catalog`] - [`Catalog`]: the game's `configs\item_catalog.ltx` (families by engine class,
//!   groups, every key's type, title, unit, whether the game stops without it, help);
//!   [`catalog::check_value`].
//! - [`compose`] - [`compose::compose`]: the module laid over the game's tree in the game's
//!   order, every change reaching what inherits; [`Composition`], [`compose::Applied`].
//! - [`resolve`] - [`Resolution`]: an item's section, HUD section and owned sections as the game
//!   holds them, every key with its value and where it comes from; a change's values before.
//! - [`validate`] - the check over [`validate::Lookups`] (`I401`..`I426`).
//! - [`atlas`] - the icon atlas: [`atlas::plan`] (deterministic shelf packing by section),
//!   [`atlas::bake`] (DXT5), [`atlas::shipped_text`] (the asset with `icon` rewritten).
//! - [`tables`] - [`TableFile`] with its [`Rows`] ([`Stock`], [`Drop`], [`Recipe`]): the three
//!   table kinds read, written and saved like an item.
//! - [`roster`] - [`Roster`]: every item and table file of the project read, composed and
//!   planned; [`Roster::found`] - where an item is found.
//! - [`doc`] - the references (`docs/assets/ITEM.md`, `TRADER.md`, `LOOT.md`, `RECIPES.md`) and
//!   the linked game's catalog after the item's.
//!
//! # The service
//!
//! - [`Items`] (project scope, `session.services`): [`Items::roster`] / [`Items::fresh_roster`]
//!   (a `Query` keyed by the watcher's changes of the items, the module's configs and patches
//!   and the pictures, the manifest and the game), [`Items::catalog`], [`Items::lookups`]
//!   ([`GameLookups`]), [`Items::resolution`], [`Items::problems`], [`Items::save`] (an edited
//!   item written as the difference into its file).
//! - [`install`] - the `file.create` template, the item kind of `gz-asset` (`asset.check`,
//!   `asset.reference kind=item`) and the three table kinds, the build's check and its publisher
//!   of the atlas, the roster made on project open. [`register`] - `items.list`, `items.info`,
//!   `economy.list`.

#![forbid(unsafe_code)]

pub mod atlas;
pub mod catalog;
mod cmd;
pub mod compose;
pub mod doc;
pub mod fields;
pub mod model;
pub mod names;
pub mod resolve;
pub mod roster;
mod service;
pub mod tables;
pub mod validate;

#[cfg(test)]
mod tests;

pub use catalog::{Catalog, Family, Group, KeyInfo, KeyType, ScopeInfo};
pub use cmd::{EconomyParams, InfoParams, ListParams, install, kind, publication, register, table_kind};
pub use compose::{Composition, Origin};
pub use model::{
    Changes, Count, Hud, Icon, Issue, Item, Kind, OwnedSection, Scene, SetValue, load_text, template, write,
};
pub use resolve::{From, PatchLine, Resolution, Row, SectionView};
pub use roster::{Entry, Found, Roster, TableEntry};
pub use service::{CatalogRead, GameLookups, Items, build_errors, issues_of, table_issues_of};
pub use tables::{Drop, Recipe, Rows, Stock, TableFile, TableKind};
