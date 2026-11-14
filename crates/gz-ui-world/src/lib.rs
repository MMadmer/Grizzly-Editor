//! The Characters and World Rules tabs (docs/UI_DESIGN.md, section 6, "Characters" and "World
//! Rules"; CLAUDE.md, "Characters" and "The mode's world").
//!
//! The C++ specification is the drawing half of `EditorCharacters.cpp` (the list, the form and
//! its sections, the pickers, the trade section, `dlg.delete_character`, `McpView`) and of
//! `EditorWorld.cpp` (the rules table, Applies On, Playable Area, Transitions, Player Start, the
//! Starting Kit, `dlg.empty_world`, the problems drawer, `McpView`).
//!
//! - **Characters** ([`characters`]): the `characters` singleton tab - a list of 260 pt with a
//!   search, dirty dots and Template badges, a splitter, and a form of up to 760 pt: Identity,
//!   Name and Bio (a row per language), Appearance, Profile, Dialogs, the Starting Kit table and
//!   Trade. The form binds to the RAW stored values (`gz_characters::text_raw`), never the
//!   resolved ones; what the base gives shows as a hint or a muted row. Ctrl+S saves through
//!   `Characters::save` (only what changed reaches the file), Delete... asks through
//!   `dlg.delete_character` and runs `file.delete`; unsaved edits ask before any switch.
//!   Validation runs on change, on the pools.
//! - **World Rules** ([`world`]): the `world_rules` singleton tab, saved to `mod.ltx` as it is
//!   edited through `world_set`: Original Game, Player Start and Starting Kit on the surface,
//!   Applies On, Playable Area and Transitions under one folded More; a problems bar and drawer
//!   only when there are problems. Empty World... asks through `dlg.empty_world`.
//! - **The lists** ([`lists`]): what the pickers offer - the linked game's items, communities,
//!   dialogs, portraits and ranks (`gz_nq::Pickers`), its trade categories, the models of
//!   `meshes\actors` of the game and the project - read on the pools into a [`lists::ListsView`]
//!   the frames only look at; a list that is still loading shows busy.
//! - [`item_picker`] - the one item picker both tabs use (a kit row, a stock row, the mode's
//!   start kit).
//! - [`register`] - `characters.view`, `world.view`; [`install`] - the two tabs and their Mod menu
//!   rows (at `gz_ui_mod::mod_menu_slot`).
//!
//! Widgets the kit lacks, local to this crate: [`picker::local_list_picker`] (a long list in a
//! popup with a search, only the rows in view laid out), [`picker::local_combo`] (a short list
//! with a free preview), [`picker::local_multi_combo`] (ticks over names, a name the list lacks
//! kept), [`portraits::local_portrait_gallery`] (the faces of every portrait sheet),
//! [`local::local_section`] (a folding header outside a property grid),
//! [`local::local_hint_field`] (a one-line field with a hint), [`local::local_number_text`] (a
//! number that may be empty, with a hint), [`local::local_check`] (a box with its label),
//! [`local::local_dirty_dot`], [`local::local_problems_drawer`] and
//! [`local::local_level_picker`] (a searchable list of the linked game's levels).
//!
//! Nothing here writes a file or computes inside a frame: every edit is a command or a service
//! call on the pools (`Characters::save`, `file.delete`, `world_set`, `place_spawn`,
//! `viewport.camera`, `preview_model`); what a frame shows comes from snapshots and from data read
//! on the pools.

#![forbid(unsafe_code)]

mod call;
mod char_form;
pub mod characters;
pub mod lists;
pub mod local;
pub mod picker;
pub mod portraits;
mod view_cmd;
pub mod world;

use gz_app::{Action, ShellBuilder, args};
use gz_editor::Registry;
use gz_ui::Icon;

pub use picker::item_picker;

/// Plugs the Characters and World Rules tabs and their Mod menu rows into the shell.
pub fn install(b: &mut ShellBuilder) {
    characters::install(b);
    world::install(b);
    b.action(
        Action::new(gz_ui_mod::menu::CHARACTERS, "Characters", "Mod", "ui.tab")
            .icon(Icon::Character)
            .args(|_| args(&[("action", "open"), ("kind", characters::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    b.action(
        Action::new(gz_ui_mod::menu::WORLD_RULES, "World Rules", "Mod", "ui.tab")
            .icon(Icon::World)
            .args(|_| args(&[("action", "open"), ("kind", world::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::CHARACTERS, None);
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::WORLD_RULES, None);
}

/// The commands of this package: `characters.view`, `world.view`.
pub fn register(reg: &mut Registry) {
    view_cmd::register(reg);
}
