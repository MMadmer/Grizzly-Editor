//! The application's own surfaces: what exists before and beside any content
//! (docs/UI_DESIGN.md, section 6, "Start page", "Output Log", "Preferences").
//!
//! - **The Start page** ([`start`]): the projects shown while no project is open - Open
//!   Folder..., Open Project File..., the search and the sort, New Project in the first slot, the
//!   cards with their previews, the card menu and the `Delete Project###dlg.delete_project`
//!   confirmation.
//! - **Link Game** ([`link`]): the page card that takes the page over while no game is known.
//! - **The Output Log** ([`log`]): one body for the Level panel and the Log drawer, over
//!   `gz_log::Log` with a cursor; the counts and the line the status bar shows.
//! - **Preferences** ([`prefs`], [`keyboard`]): the `preferences` singleton tab - every
//!   preference with its label, help and unit, over the shell's `EditorSettings`; the Keyboard
//!   page over the actions' keys (`app.keys`).
//!
//! [`install`] plugs all of it into the shell; [`register`] adds `project.page`.
//!
//! Widget the kit lacks, local to this crate: [`local::local_rule`] (a muted rule to the edge of
//! a row).
//!
//! Nothing here touches a file or computes inside a frame: lists, counts and pictures come from
//! snapshots and queries filled on the pools; every action is an existing command
//! (`project.open`, `project.create`, `project.delete`, `project.forget`, `project.reveal`,
//! `project.close`, `game.link`, `shortcuts`), so the window and MCP share one code path.

#![forbid(unsafe_code)]

mod browser_cmd;
pub mod cards;
pub mod collect;
mod folder;
pub mod game_prefs;
pub mod keyboard;
pub mod launch;
pub mod link;
pub mod local;
pub mod log;
pub mod prefs;
mod reply;
pub mod start;
pub mod update;

use gz_app::ShellBuilder;
use gz_editor::Registry;

pub use log::{LogHost, LogModel, output_log};

/// Plugs the Start page, the Link Game card, the Output Log (drawer, panel, status line) and
/// the Preferences tab into the shell.
pub fn install(b: &mut ShellBuilder) {
    start::install(b);
    link::install(b);
    collect::install(b);
    log::install(b);
    prefs::install(b);
    update::install(b);
}

/// The commands of this package: `project.page`.
pub fn register(reg: &mut Registry) {
    browser_cmd::register(reg);
}
