//! Quests and behaviour graphs: the `.nqasset` / `.behasset` node graphs of the editor.
//!
//! The C++ specification is `cpp-editor-last:Source/Editors/XrECore/Editor/Nq/` (`NqAsset`, `NqLua`'s typed
//! half, `NqCatalog`, `NqDoc`, `NqValidate`, `NqLayout`, `NqReferences`, `NqProjectIndex`,
//! `NqPickers`, `NqItemCaps`, `NqExport`, `NqMcp`) and the canvas operations of the graph tab
//! that MCP reaches (`UIQuestGraph::McpEdit` / `McpFind` / `McpBookmarks` / `McpHistory` /
//! `McpMinimap`). The editor never compiles a graph: the asset travels into the module as it is
//! and the game's NQ runtime reads it.
//!
//! # The domain (pure)
//!
//! - [`asset`] - the graph as plain data: [`Graph`] (header, [`Node`]s with params, pins and
//!   links, [`Task`]s or [`Role`]s, [`Var`]s), read from a [`gz_luadata::Value`]
//!   ([`asset::graph_from_value`], [`asset::parse`]) and written back canonically
//!   ([`asset::write`], byte-equal to `NqLua::Write`), the outline ([`asset::outline`]), the
//!   topological order, the templates.
//! - [`catalog`] - the kind catalog: [`Catalog`], [`Kind`], [`Param`]; [`catalog::parse_text`]
//!   reads one catalog file the C++ way and [`Catalog::assemble`] folds the files in order.
//! - [`layout`] - [`layout::node_box`], the ONE source of a node's rectangle, and the auto
//!   layout ([`layout::run`], [`layout::ensure_positions`]).
//! - [`validate`] - [`validate::run`]: `(graph, catalog, context) -> problems`, the C++ codes and
//!   messages word for word; [`NqProblem`].
//! - [`references`] - catalog-typed reference discovery and the task rename.
//! - [`index`] - the project's graphs as one [`index::IndexSnapshot`] and the node search.
//! - [`pickers`] - the lookups of `quest_lookup` and of `W060` over the linked game's data
//!   ([`pickers::Pickers`]); [`caps`] - what an item section can carry (`W062`, `W063`).
//! - [`doc`] - an open document: [`doc::GraphDoc`], the state of one `Store`, with every edit
//!   operation of `NqDoc` and of the canvas as commands.
//! - [`text`] - the text helpers the outline, the pickers and the validator share.
//!
//! # The service and the commands
//!
//! - [`NqService`] (project scope, `session.services`): the open documents (one
//!   `Store<GraphDoc>` each), the catalog, the pickers, the item capabilities and the project
//!   index as `Query`s keyed by the revisions they read; the slots other areas fill
//!   ([`CharacterLookup`], [`SceneLookup`], [`WorldLookup`]).
//! - [`register`] - every `quest_*` command of the bridge except `quest_view` (the graph tab's
//!   own state; the UI registers it); [`install`] - the `content_create` templates, the rename
//!   hook, the build check and the unsaved-work source.

#![forbid(unsafe_code)]

pub mod asset;
pub mod caps;
pub mod catalog;
mod check;
mod cmd;
pub mod doc;
pub mod index;
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
    pub use crate::cmd::{quest_bookmarks, quest_edit, quest_find, quest_history, quest_minimap};
}
pub use pickers::{PickerEntry, PickerType, Pickers};
pub use service::{
    CharacterBrief, CharacterLookup, DocHandle, DocPath, NqService, SceneLookup, Transition, ValidateEnv,
    WorldLookup,
};
pub use validate::{NqProblem, Severity};
