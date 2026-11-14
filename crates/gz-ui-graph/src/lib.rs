//! The graph tab: where a quest (`.nqasset`) or a behaviour graph (`.behasset`) is edited
//! (docs/UI_DESIGN.md, section 6, "Quest and Behaviour graph"; docs/NQ_ARCHITECTURE.md 13.8-13.11).
//!
//! - [`tab`] - [`tab::GraphTab`], one [`gz_app::Document`] per file (`quest:<rel>` /
//!   `beh:<rel>`): the toolbar, the Graph panel | the canvas | Details (each a rail when the tab
//!   is narrow), the drawer (Problems, Find Results), the external-change banner, the Shortcuts
//!   card, the Graph menu, Preferences > Graph.
//! - [`scene`] - what the canvas draws, built once per graph revision (and wire options and
//!   names) on the CPU pool from `gz_nq::layout`'s one node box and `gz_nq::wires`' one wire
//!   builder: node geometry and texts in words, pins by role, wires' polylines, comment frames, a
//!   uniform grid index, the extent Frame All fits, the sorted edges `beyond` counts by, the
//!   minimap's mesh.
//! - [`canvas`] - the painter canvas: zoom levels, semantic zoom (full, header, block), hit tests
//!   through the grid, gestures (pan, zoom at the cursor, drag with grid and guides, wires out of
//!   and into pins, rewire, cut, reroutes, frames, the marquee, action drags), the minimap, the
//!   edge pills, the context menus and the kind picker (the kit's `PickList`, filtered by the pin).
//!   Every edit is a `gz_nq` document command or an action of the group Graph.
//! - [`actions`] - the actions of the group Graph: what the keys, the menus and the palette run.
//! - [`inspector`] and [`params`] - the Graph panel (one scroll: header, Tasks or Roles, Variables,
//!   Global) and the Details panel (the node, its action under a splitter), with the typed
//!   parameter editors.
//! - [`dialogs`] - Find in Project, Rename Task, Task References, Edit Text / Edit Lua.
//! - [`derived`] - what the tab reads besides the snapshot (the catalog, the problems as marks on
//!   their nodes, the pickers, the names of items and characters, the item capabilities, the
//!   globals, the project's ids), fetched off the UI thread: a frame never computes and never
//!   touches a file.
//! - [`cmd`] - `graph.view`, the tab's own command.
//!
//! The one widget the kit lacks, local to this crate ([`local`]): `local_pane_header` (an icon,
//! a title and a muted id at the top of a pane).

#![forbid(unsafe_code)]

pub mod actions;
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

/// Plugs the graph tab into the shell: the `quest` and `beh` document kinds.
pub fn install(b: &mut ShellBuilder) {
    tab::install(b);
}

/// `graph.view`, the tab's own command. Register after `gz_nq::register`.
pub fn register(reg: &mut Registry) {
    cmd::register(reg);
}
