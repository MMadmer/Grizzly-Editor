//! The graph tab: where a quest (`.nqasset`) or a behaviour graph (`.behasset`) is edited
//! (docs/UI_DESIGN.md, section 6, "Quest and Behaviour graph").
//!
//! The C++ specification is `cpp-editor-last:Source/Editors/LevelEditor/UI/QuestGraph/` (`UIQuestGraph`,
//! `NqCanvas`, `NqInspector`, `NqLook`) and `UIQuestGraph::McpView` for `quest_view`.
//!
//! - [`tab`] - [`tab::GraphTab`], one [`gz_app::Document`] per file (`quest:<rel>` /
//!   `beh:<rel>`): the toolbar, the Graph panel | the canvas | Details, the drawer (Problems,
//!   Find Results), the external-change banner, the Shortcuts card, the Graph menu, the keys.
//! - [`scene`] - what the canvas draws, built once per graph revision on the CPU pool from
//!   `gz_nq::layout`'s one node box: node geometry and texts, links (beziers and the back edges
//!   routed through channels, as `NqCanvas::RouteBack` lays them out), a uniform grid index, the
//!   extent Frame All fits, the sorted edges `beyond` counts by, the minimap's mesh.
//! - [`canvas`] - the painter canvas: zoom levels, level of detail, hit tests through the grid,
//!   gestures (pan, zoom at the cursor, drag, link, marquee, action drag), the minimap, the edge
//!   pills, the context menus and the kind pickers. Every edit is a `gz_nq` document command.
//! - [`inspector`] and [`params`] - the Graph panel (header, Tasks or Roles, Variables, Global)
//!   and the Details panel (the node, its action under a splitter), with the typed parameter
//!   editors of `NqInspector::DrawTyped`.
//! - [`dialogs`] - Find in Project, Rename Task, Task References, Edit Text / Edit Lua.
//! - [`derived`] - what the tab reads besides the snapshot (the catalog, the problems, the
//!   pickers, the item capabilities, the characters, the globals, the project's ids), fetched off
//!   the UI thread: a frame never computes and never touches a file.
//! - [`cmd`] - `quest_view` (the C++ wire), and `quest_open` that brings the tab up.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_fold_header` (a folding header
//! outside a property grid, with one action on it), `local_pane_header` (an icon, a title and a
//! muted id), `local_check` (a box with its label beside it), `local_kind_combo` (a combo of
//! kinds grouped by family) and `local_pick_list` (a search field over a list, for the pickers).

#![forbid(unsafe_code)]

pub mod canvas;
pub mod cmd;
pub mod derived;
pub mod dialogs;
pub mod inspector;
pub mod local;
pub mod look;
pub mod params;
pub mod scene;
pub mod tab;

use gz_app::ShellBuilder;
use gz_editor::Registry;

/// The crate's name, as the registry records who registered a command.
pub(crate) const AREA: &str = "gz-ui-graph";

/// Plugs the graph tab into the shell: the `quest` and `beh` document kinds.
pub fn install(b: &mut ShellBuilder) {
    tab::install(b);
}

/// `quest_view`, and `quest_open` that also brings the tab up. Register after `gz_nq::register`.
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
}
