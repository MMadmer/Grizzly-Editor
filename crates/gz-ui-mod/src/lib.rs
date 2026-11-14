//! Mod Settings and the build dialogs (docs/UI_DESIGN.md, section 6, "Mod Settings" and "Build
//! dialogs"; section 5, the Mod menu and the project toolbar).
//!
//! - **Mod Settings** ([`settings`]): the `mod_settings` singleton tab - a nav of seven pages
//!   (General, Mod Browser, Game Mode, Relations, Build Contents, PDA, Commands), each a row with
//!   its icon and a problem dot, and a form of up to 44 em with the problem of a field as a
//!   sentence under it. The form ([`form`]) follows `gz-mod`'s store; an edit is saved once the
//!   field being typed in lets go, as one `mod.set`.
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
//!   Level Cut... - faces of `level.export` (`kind=spawn`, `collision`, `visuals`, `cut`),
//!   reporting in `dlg.bake_result`.
//! - [`register`] - `mod.view` (the tab put up to be looked at).
//!
//! What this crate draws that no other surface does ([`local`]): `local_value_column` (a block of
//! rows inside one property row) and `local_picture` (the thumbnail's frame). The text fields are
//! the kit's `form::row_field`, the module and function pickers its `PickList`.
//!
//! Nothing here writes a file or computes inside a frame: every action is a command that already
//! exists (`mod.set`, `mod.thumbnail`, the PDA tab's opener, `release.key action=open`) or the
//! job the build and the packaging commands start, and what a frame
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

/// Plugs Mod Settings, the build and package dialogs, the Mod menu's rows and the Build Mod split
/// into the shell.
pub fn install(b: &mut ShellBuilder) {
    b.translation(include_str!("ru.lua"));
    settings::install(b);
    build::install(b);
    package::install(b);
    menu::install(b);
    bake::install(b);
}

/// The commands of this package: `mod.view`.
pub fn register(reg: &mut Registry) {
    view_cmd::register(reg);
}

#[cfg(test)]
mod i18n_tests {
    #[test]
    fn the_catalog_reads_with_the_kits_and_the_shells() {
        let c = gz_ui::i18n::Catalog::parse(&[
            gz_ui::i18n::KIT_RU,
            gz_app::lang::SHELL_RU,
            include_str!("ru.lua"),
        ])
        .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }
}
