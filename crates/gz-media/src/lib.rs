//! Media assets: a model (`*.meshasset`), a texture (`*.texasset`) and a sound (`*.sndasset`) -
//! text files beside a source of a common format (`docs/ASSETS.md` section 6; the references
//! `docs/assets/MODEL.md`, `TEXTURE.md`, `SOUND.md`).
//!
//! The asset is what everything refers to; the source is its companion. The editor reads the
//! asset for its own picture, its level compiler and its checks; the module build publishes the
//! data baked from the source and a copy of the text where the game looks for the asset's name,
//! and the game reads the description over the data.
//!
//! # The domain
//!
//! - [`fields`] - the key tables of the three kinds and their word lists; [`doc`] makes the
//!   references of them.
//! - [`model`], [`texture`], [`sound`] - each kind's file as a typed value. [`TextureAsset`]
//!   turns into a [`gz_fmt_tex::TextureParams`] and back without a loss.
//! - [`parts`] - the geometry a model writes instead of a source.
//! - [`resolve`] - an asset made into what the editor uses: a model into its
//!   `gz_fmt_object::Object` and the scene's `LibraryObject`, a texture into its pixels and its
//!   description, a sound into its parameters.
//! - [`index`] - every media asset of a project by kind, path and published name.
//! - [`publish`] - the bake the module build ships ([`publish::publish`] into a
//!   [`publish::Sink`], [`index::Index::published_paths`], [`index::Index::is_source`]).
//! - [`import`] - `asset.import`'s plan.
//! - [`check`] - `asset.check` of the three kinds, coded `M1xx`.
//!
//! # The service and the commands
//!
//! - [`Media`] (project scope): the index kept fresh by the project's watcher, the textures and
//!   sounds by name for the editor's readers, what the checks read of the linked game. The same
//!   watcher tells the scene's library which models to make again.
//! - [`install`] - the kinds of `gz-asset`, the `file.create` templates, the scene's provider of
//!   model assets, the index made when a project opens. [`register`] - `asset.import`,
//!   `model.info`, `model.list`.

#![forbid(unsafe_code)]

pub mod check;
mod cmd;
pub mod doc;
pub mod fields;
pub mod files;
pub mod import;
pub mod index;
pub mod kind;
pub mod model;
pub mod parts;
pub mod publish;
pub mod read;
pub mod resolve;
mod service;
pub mod sound;
pub mod texture;

#[cfg(test)]
mod tests;

pub use cmd::{ImportParams, InfoParams, ListParams, ModelSource, install, register};
pub use index::{Entry, Index, scan};
pub use model::ModelAsset;
pub use publish::{Payload, Published, Sink, publish};
pub use service::{GameLists, GameModelSource, GameModels, Media, forget_game_models, provider};
pub use sound::SoundAsset;
pub use texture::TextureAsset;
