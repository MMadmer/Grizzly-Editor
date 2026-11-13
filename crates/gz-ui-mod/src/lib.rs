//! Mod Settings and the build dialogs (docs/UI_DESIGN.md, section 6, "Mod Settings" and "Build
//! dialogs"; section 5, the Mod menu and the project toolbar).
//!
//! The C++ specification is the drawing half of `EditorModManifest.cpp` (the Mod Settings tab and
//! its pages, Build to Folder, Export Flat, Package Release, the result dialogs, the clean-target
//! question, Disable Game Updates), `EditorCatalog::DrawKeyLine`, the Game Mode picker of
//! `EditorGameModes.cpp`, the Mod menu of `UIMainMenuForm.cpp` and the Build Mod split of
//! `UITopBarForm.cpp`.
//!
//! - **Mod Settings** ([`settings`]): the `mod_settings` singleton tab - a nav of seven pages
//!   (General, Mod Browser, Game Mode, Relations, Build Contents, PDA, Commands), each a row with
//!   its icon and a problem dot, and a form of up to 44 em with the problem of a field as a
//!   sentence under it. The form ([`form`]) follows `gz-mod`'s store; an edit is saved once the
//!   field being typed in lets go, through `mod_set_manifest`, `mod_commands` and `pda_use`.
//!   What the checks and the pickers read (the linked game's modes and modules, the project's
//!   script functions, its PDA layouts) is read when a page is picked, never per frame.
//! - **The builds** ([`build`]): Build Into Game, the Build to Folder and Export Flat dialogs, the
//!   clean-target question (`dlg.clean_target`) and the kit's one result dialog
//!   (`dlg.build_result`, `dlg.export_result`).
//! - **Package Release** ([`package`]): the dialog, the author key row ([`package::key_line`],
//!   shared with Submit), the `dlg.replace_release` question and `dlg.package_result`.
//! - **The Task Center** ([`cards`]): the build's and the packaging's steps, numbers and Stop,
//!   published into `gz_app::TaskCardTable` for every such job, the ones MCP started included.
//! - **The Mod menu and the toolbar** ([`menu`]): the rows in the design's order ([`MOD_MENU`]
//!   names every row's slot, so the packages that own Characters, World Rules, Screens & Music,
//!   PDA Interface, Bake Scene Layer, Publish Release, Releases and Submit add theirs in place),
//!   and the Build Mod split at the right end of every toolbar.
//! - **Bake Scene Layer** (`bake`): Spawn Layer..., Collision Overlay..., Visual Overlay...,
//!   Level Cut... - faces of `mod_export_spawn`, `mod_export_xcform`, `mod_export_ogf_probe`
//!   and `mod_export_cut`, reporting in `dlg.bake_result`.
//! - [`register`] - `mod_settings_view`.
//!
//! Widgets the kit lacks, local to this crate ([`local`]): `local_text_field` (a one-line field
//! with a hint), `local_password_field`, `local_check` (a switch with its label, the tags'
//! checkboxes), `local_value_column` (a block of rows inside one property row), `local_picture`
//! (the thumbnail's frame) and `local_pick_list` (a searchable list in a popup: Add Module...,
//! the Function picker).
//!
//! Nothing here writes a file or computes inside a frame: every action is a command that already
//! exists (`mod_set_manifest`, `mod_commands`, `pda_use`, the PDA tab's opener, `mod_catalog
//! action=key_unlock`) or the job the build and the packaging commands start, and what a frame
//! shows comes from snapshots and from data read on the pools.

#![forbid(unsafe_code)]

mod bake;
pub mod build;
mod call;
pub mod cards;
pub mod form;
pub mod local;
pub mod menu;
pub mod package;
pub mod settings;
mod view_cmd;

use gz_app::ShellBuilder;
use gz_editor::Registry;

pub use menu::{MOD_MENU, mod_menu_slot};
pub use package::KeyLine;

/// The crate's name, as the registry records who registered a command.
const AREA: &str = "gz-ui-mod";

/// Plugs Mod Settings, the build and package dialogs, the Mod menu's rows and the Build Mod split
/// into the shell.
pub fn install(b: &mut ShellBuilder) {
    settings::install(b);
    build::install(b);
    package::install(b);
    menu::install(b);
    bake::install(b);
}

/// The commands of this package: `mod_settings_view`.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "mod_settings_view", view_cmd::mod_settings_view);
    menu::register(reg);
}
