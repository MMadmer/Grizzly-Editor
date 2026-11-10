//! The PDA preview and the Screens & Music tab (docs/UI_DESIGN.md, section 6, "PDA Interface"
//! and "Screens & Music"; CLAUDE.md, "PDA interfaces" and "Screens and music").
//!
//! - **The PDA preview** ([`pda`]): the `pda` document - a singleton that switches to the next
//!   layout it is given, asking first while its Source pane holds edits. The layout is what
//!   `gz_pda::evaluate` makes of the text (the game's own reader, off the UI thread); the running
//!   interface is `gz_pda`'s `Preview`. This crate draws the flat item list and forwards clicks,
//!   keys and the wheel - it interprets nothing of its own. Drawing is egui meshes with the
//!   game's textures and BITMAP fonts ([`assets`], [`bitmap`]): a text is as wide here as in the
//!   game, canvas units scale per axis, the button placement rule is the game's. `pda_preview`
//!   answers in canvas units and waits for the frame it changed.
//! - **Screens & Music** ([`screens`]): the `screens` document - the screens and music of one
//!   surface, the selected one's preview, layers and settings - saved to `mod.ltx` as it is
//!   edited through `gz-mod`'s store; files arrive by drag and drop ([`drop`]) and go through
//!   `gz_screens::import` off the UI thread. `screens_view` opens or reports it.
//! - [`gamefile`]: what the preview reads of the game's own files, as the C++ read them -
//!   `pda.xml`, a level's map, plain ltx values; [`bitmap`]: the game's fonts and how a label
//!   lays out in them. Both pure.
//!
//! [`install`] plugs both tabs and their Mod menu commands (`COMMAND_MOD_SCREENS`,
//! `COMMAND_MOD_PDA`) into the shell, the rows at the slots `gz-ui-mod`'s `MOD_MENU` keeps; [`register`] adds `pda_preview` and `screens_view`.
//!
//! Widgets the kit lacks, local to this crate ([`local`], [`drop`]): `local_file_drop_zone` (a
//! place files are dropped on), `local_list_header` (a list's title with its +),
//! `local_section` (a folding header outside a grid), `local_tab_strip` (a panel's tabs),
//! `local_check` (a switch with its label), `local_multi_combo` and `local_toggle_row` (a set
//! picked from a list that stays open), `local_code_view` (Mono text with a gutter and an
//! error line), `local_drop_mark` (where a dragged row lands).
//!
//! Nothing here touches a file or computes inside a frame: files are read, decoded and written
//! on the pools; a frame draws from the model and the caches and starts what it needs.

#![forbid(unsafe_code)]

pub mod assets;
pub mod bitmap;
pub mod drop;
pub mod gamefile;
pub mod local;
pub mod pda;
pub mod screens;

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests;

use gz_app::ShellBuilder;
use gz_editor::Registry;

pub use drop::{begin_drops, local_file_drop_zone, simulate_drop};

/// The crate's name, as the registry records who registered a command.
const AREA: &str = "gz-ui-pda";

/// Plugs the `pda` and `screens` tabs and their Mod menu commands into the shell.
pub fn install(b: &mut ShellBuilder) {
    pda::doc::install(b);
    screens::doc::install(b);
    // the rows go where the Mod menu keeps their slots: Screens & Music, then PDA Interface
    for command in [gz_ui_mod::menu::SCREENS, gz_ui_mod::menu::PDA] {
        let (group, order) = gz_ui_mod::mod_menu_slot(command).unwrap_or((0, 9));
        b.menu_entry(gz_app::MenuEntry {
            menu: gz_editor::SmolStr::new("Mod"),
            submenu: None,
            group,
            order,
            content: gz_app::MenuContent::Command { command: gz_editor::SmolStr::new(command), label: None },
        });
    }
}

/// The commands of this package: `pda_preview`, `screens_view`.
pub fn register(reg: &mut Registry) {
    reg.add(AREA, "pda_preview", pda::cmd::pda_preview);
    reg.add(AREA, "screens_view", screens::cmd::screens_view);
}
