//! The application's own surfaces: what exists before and beside any content
//! (docs/UI_DESIGN.md, section 6, "Start page", "Output Log", "Preferences").
//!
//! - **The Start page** ([`start`]): the project browser shown while no project is open - New
//!   Project and Open Folder..., the search and the sort, the cards with their previews, the card
//!   menu and the `Delete Project###dlg.delete_project` confirmation. The C++ specification is
//!   `EditorProject.cpp` (`DrawBrowser`, `DrawCard`, `DrawDeleteConfirm`, `McpBrowserMenu`).
//! - **Link Game** ([`link`]): the page card that takes the page over while the open project has
//!   no game linked (`EditorProject::DrawGameLink`).
//! - **The Output Log** ([`log`]): one body for the Level panel and the Log drawer, over
//!   `gz_log::Log` with a cursor; the counts and the line the status bar shows
//!   (`UILogForm.cpp`).
//! - **Preferences** ([`prefs`], [`keyboard`]): the `preferences` singleton tab - every
//!   preference of `EditorPreferences.cpp` and `LevelPreferences.cpp` with its official label,
//!   help and unit, over the shell's `EditorSettings`; the Keyboard page over the shell's
//!   binding table (the `shortcuts` command).
//!
//! [`install`] plugs all of it into the shell; [`register`] adds `project_browser_menu`.
//!
//! Widgets the kit lacks, local to this crate: [`local::local_section`] (a folding header outside
//! a property grid), [`local::local_text_field`] (a one-line field with a hint) and
//! [`local::local_rule`] (a muted rule to the edge of a row).
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

/// The crate's name, as the registry records who registered a command.
const AREA: &str = "gz-ui-app";

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

/// The commands of this package: `project_browser_menu`.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "project_browser_menu", browser_cmd::project_browser_menu);
}
