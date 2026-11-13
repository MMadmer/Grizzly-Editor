//! Quests and behaviour graphs: the `.nqasset` / `.behasset` node graphs of the editor.
//!
//! The editor never compiles a graph: the asset travels into the module as it is and the game's
//! NQ runtime reads it. A graph's file is the author's text: the editor reads it through the one
//! read path of the assets (`gz-asset`) and writes back only what an edit changed.
//!
//! # The domain (pure)
//!
//! - [`asset`] - the graph as plain data: [`Graph`] (header, [`Node`]s with params, pins and
//!   links, [`Task`]s or [`Role`]s, [`Var`]s), read from a [`gz_luadata::Value`]
//!   ([`asset::graph_from_value`], [`asset::parse`]), its value for a save
//!   ([`asset::model_value`], [`asset::diff_rules`]), the text of a new file ([`asset::write`]),
//!   the outline ([`asset::outline`]), the topological order, the templates.
//! - [`catalog`] - the kind catalog: [`Catalog`], [`Kind`], [`Param`]; [`catalog::parse_text`]
//!   reads one catalog file and [`Catalog::assemble`] folds the files in order.
//! - [`layout`] - [`layout::node_box`], the ONE source of a node's rectangle, and the auto
//!   layout ([`layout::run`], [`layout::ensure_positions`]).
//! - [`validate`] - [`validate::run`]: `(graph, catalog, context) -> problems`; [`NqProblem`].
//! - [`references`] - catalog-typed reference discovery and the task rename.
//! - [`index`] - the project's graphs as one [`index::IndexSnapshot`] and the node search.
//! - [`pickers`] - the lists of `game.lookup` and of `W060` over the linked game's data
//!   ([`pickers::Pickers`]); [`caps`] - what an item section can carry (`W062`, `W063`).
//! - [`doc`] - an open document: [`doc::GraphDoc`], the state of one `Store`, with every edit
//!   operation of the canvas as commands.
//! - [`kind`] - quests and behaviour graphs as kinds of `gz-asset`: their positioned check, their
//!   reference (`docs/nq/NQ_FORMAT.md` and the linked game's node kinds), the keys they know.
//! - [`text`] - the text helpers the outline, the pickers and the validator share.
//!
//! # The service and the commands
//!
//! - [`NqService`] (project scope, `session.services`): the open documents (one
//!   `Store<GraphDoc>` each, with the file's lossless document, [`DiskState`] and a follower of
//!   the file through the project's watcher), the catalog, the pickers, the item capabilities
//!   and the project index as `Query`s keyed by the revisions they read; the slots other areas
//!   fill ([`CharacterLookup`], [`SceneLookup`], [`WorldLookup`]).
//! - [`register`] - the `quest_*` commands of open documents (`quest_view` is the graph tab's
//!   own; the UI registers it); [`install`] - the `file.create` templates, the rename hook, the
//!   build check, the kinds of `gz-asset` and the unsaved-work source.

#![forbid(unsafe_code)]

pub mod asset;
pub mod caps;
pub mod catalog;
mod check;
mod cmd;
mod disk;
pub mod doc;
pub mod index;
pub mod kind;
pub mod layout;
pub mod pickers;
pub mod references;
mod service;
pub mod text;
pub mod validate;

pub use asset::{Action, Cond, DocKind, Graph, Node, Objective, Pin, Role, Task, Var};
pub use catalog::{Catalog, Kind, Param};
pub use cmd::{install, register};
/// The tab commands whose C++ handlers bring the graph's tab up (`OpenMcpWindow`): `gz-ui-graph`
/// registers over them and calls these after it did.
pub mod tab_handlers {
    pub use crate::cmd::{
        project_find, quest_bookmarks, quest_edit, quest_find, quest_history, quest_minimap, quest_open,
        references,
    };
}
pub use disk::DiskState;
pub use pickers::{PickerEntry, PickerType, Pickers};
pub use service::{
    CharacterBrief, CharacterLookup, DocHandle, DocPath, NqService, SceneLookup, Transition, ValidateEnv,
    WorldLookup,
};
pub use validate::{NqProblem, Severity};
