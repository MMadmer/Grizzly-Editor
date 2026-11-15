//! The Characters, Items and World Rules tabs (docs/UI_DESIGN.md, section 6, "Characters",
//! "Items" and "World Rules"; CLAUDE.md, "Characters" and "The mode's world").
//!
//!
//! - **Characters** ([`characters`]): the `characters` singleton tab - a list of 260 pt with a
//!   search, dirty dots and Template badges, a splitter, and a form of up to 760 pt: Identity,
//!   Name and Bio (a row per language), Appearance, Profile, Dialogs, the Starting Kit table and
//!   Trade. The form binds to the RAW stored values (`gz_characters::text_raw`), never the
//!   resolved ones; what the base gives shows as a hint or a muted row. Ctrl+S saves through
//!   `Characters::save` (only what changed reaches the file), Delete... asks through
//!   `dlg.delete_character` and runs `file.delete`; unsaved edits ask before any switch.
//!   Validation runs on change, on the pools.
//! - **Items** ([`items`]): the `items` singleton tab - the module's items under New and
//!   Changes, a form of up to 760 pt whose key rows are the linked game's item catalog's keys of
//!   the item's family (`item_model`), and the item as the game shows it. The form binds to what
//!   the file STORES; Ctrl+S saves through `Items::save` (the difference), the + menu's choosers
//!   make the file at once, Delete... asks through `dlg.delete_item`. What the item resolves to
//!   and its problems are worked out on the pools for the item as it is edited
//!   (`Items::check_edit`); the lists the Characters tab lacks are [`item_lists`].
//! - **World Rules** ([`world`]): the `world_rules` singleton tab, saved to `mod.ltx` as it is
//!   edited through `world.set`: Original Game, Player Start and Starting Kit on the surface,
//!   Applies On, Playable Area and Transitions under one folded More; a problems bar and drawer
//!   only when there are problems. Empty World... asks through `dlg.empty_world`.
//! - **The lists** ([`lists`]): what the pickers offer - the linked game's items, communities,
//!   dialogs, portraits and ranks (`gz_nq::Pickers`), its trade categories, the models of
//!   `meshes\actors` of the game and the project - read on the pools into a [`lists::ListsView`]
//!   the frames only look at; a list that is still loading shows busy.
//! - [`item_picker`] - the one item picker both tabs use (a kit row, a stock row, the mode's
//!   start kit).
//! - [`register`] - `characters.view`, `items.view`, `world.view`; [`install`] - the two tabs and their Mod menu
//!   rows (at `gz_ui_mod::mod_menu_slot`).
//!
//! What comes from a list is picked with the kit's `Combo` and `PickList` ([`picker::list_picker`],
//! [`picker::row_combo`]). Widgets the kit lacks, local to this crate:
//! [`picker::local_multi_combo`] (ticks over names, a name the list lacks kept),
//! [`portraits::local_portrait_gallery`] (the faces of every portrait sheet),
//! [`local::local_number_text`] (a number that may be empty, with a hint),
//! [`local::local_value_text`] (an own value or the inherited one as its hint),
//! [`local::local_add_field`], [`local::local_value_combo`], [`local::local_dirty_dot`], [`local::local_problems_drawer`], [`local::local_cell`] and
//! [`local::drawer_height`]. The levels World Rules picks are the kit's `PickList`.
//!
//! Nothing here writes a file or computes inside a frame: every edit is a command or a service
//! call on the pools (`Characters::save`, `file.delete`, `world.set`, `scene.place`,
//! `viewport.camera`, `preview.model`); what a frame shows comes from snapshots and from data read
//! on the pools.

#![forbid(unsafe_code)]

mod call;
mod char_form;
pub mod characters;
mod item_form;
pub mod item_lists;
pub mod item_model;
mod item_view_cmd;
pub mod items;
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
    b.translation(include_str!("ru.lua"));
    b.translation(include_str!("items.ru.lua"));
    characters::install(b);
    items::install(b);
    world::install(b);
    b.action(
        Action::new(gz_ui_mod::menu::CHARACTERS, "Characters", "Mod", "ui.tab")
            .icon(Icon::Character)
            .args(|_| args(&[("action", "open"), ("kind", characters::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    b.action(
        Action::new(gz_ui_mod::menu::ITEMS, "Items", "Mod", "ui.tab")
            .icon(Icon::Item)
            .args(|_| args(&[("action", "open"), ("kind", items::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    b.action(
        Action::new(gz_ui_mod::menu::WORLD_RULES, "World Rules", "Mod", "ui.tab")
            .icon(Icon::World)
            .args(|_| args(&[("action", "open"), ("kind", world::TAB)]))
            .enabled(gz_ui_mod::menu::with_project),
    );
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::CHARACTERS, None);
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::ITEMS, None);
    gz_ui_mod::menu::row(b, gz_ui_mod::menu::WORLD_RULES, None);
}

/// The commands of this package: `characters.view`, `items.view`, `world.view`.
pub fn register(reg: &mut Registry) {
    view_cmd::register(reg);
    item_view_cmd::register(reg);
}

#[cfg(test)]
mod i18n_tests {
    #[test]
    fn the_catalog_reads_with_the_kits_and_the_shells() {
        let c = gz_ui::i18n::Catalog::parse(&[
            gz_ui::i18n::KIT_RU,
            gz_app::lang::SHELL_RU,
            include_str!("ru.lua"),
            include_str!("items.ru.lua"),
        ])
        .expect("the catalogs read");
        let p = gz_ui::i18n::problems(&c);
        assert!(p.is_empty(), "{}", p.join("\n"));
    }
}
