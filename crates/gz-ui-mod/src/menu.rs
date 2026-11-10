//! The Mod menu and the project toolbar's Build Mod (docs/UI_DESIGN.md, section 5).
//!
//! The menu's rows stand in the design's order - Mod Settings, Characters, World Rules, Screens
//! & Music, PDA Interface | Build Into Game, Build to Folder..., Export Flat Gamedata..., Bake
//! Scene Layer > | Package Release..., Publish Release..., Releases | Submit to Mod Browser... -
//! as [`MOD_MENU`] gives each its group and order. This package adds the rows whose targets it
//! owns (Mod Settings, the builds, Package Release); the package that owns any other row adds it
//! at its slot with `ShellBuilder::menu_item("Mod", group, order, id)` (a submenu by
//! `MenuEntry::submenu`) and registers the `UiCommand` of that id, so no row stands without a
//! command. The C++ specification is `MenuMod` of `UIMainMenuForm.cpp` and
//! `UITopBarForm::DrawProject`.

use gz_app::{DocCx, MenuContent, MenuEntry, OpenRequest, ShellBuilder, UiCommand};
use gz_editor::{Editor, SmolStr};
use gz_ui::{Icon, Kind, MenuItem, menu_item};

use crate::build::{self, build_running, build_target_text, can_build_into_game, job_status_text};

pub const MOD_SETTINGS: &str = "COMMAND_MOD_EDIT_MANIFEST";
pub const CHARACTERS: &str = "COMMAND_MOD_CHARACTERS";
pub const WORLD_RULES: &str = "COMMAND_MOD_WORLD";
pub const SCREENS: &str = "COMMAND_MOD_SCREENS";
pub const PDA: &str = "COMMAND_MOD_PDA";
pub const BUILD_INTO_GAME: &str = "COMMAND_MOD_BUILD";
pub const BUILD_TO_FOLDER: &str = "COMMAND_MOD_BUILD_TO_FOLDER";
pub const EXPORT_FLAT: &str = "COMMAND_MOD_EXPORT_FLAT";
/// Not a command: the submenu its owner fills (Spawn Layer..., Collision Overlay..., Visual
/// Overlay..., Level Cut...).
pub const BAKE_SCENE_LAYER: &str = "Bake Scene Layer";
pub const PACKAGE: &str = "COMMAND_MOD_PACKAGE";
pub const PUBLISH: &str = "COMMAND_MOD_PUBLISH";
pub const RELEASES: &str = "COMMAND_MOD_RELEASES";
pub const CATALOG: &str = "COMMAND_MOD_CATALOG";

/// Every row of the Mod menu: its command id (or, for Bake Scene Layer, the submenu's name), its
/// group (a separator stands between groups) and its order in the group.
pub const MOD_MENU: [(&str, i32, i32); 13] = [
    (MOD_SETTINGS, 0, 0),
    (CHARACTERS, 0, 1),
    (WORLD_RULES, 0, 2),
    (SCREENS, 0, 3),
    (PDA, 0, 4),
    (BUILD_INTO_GAME, 1, 0),
    (BUILD_TO_FOLDER, 1, 1),
    (EXPORT_FLAT, 1, 2),
    (BAKE_SCENE_LAYER, 1, 3),
    (PACKAGE, 2, 0),
    (PUBLISH, 2, 1),
    (RELEASES, 2, 2),
    (CATALOG, 3, 0),
];

/// The group and order of a Mod menu row.
#[must_use]
pub fn mod_menu_slot(id: &str) -> Option<(i32, i32)> {
    MOD_MENU.iter().find(|(i, _, _)| *i == id).map(|(_, g, o)| (*g, *o))
}

fn has_project(editor: &Editor, _tab: &str) -> bool {
    editor.project_opt().is_some()
}

/// A row of the Mod menu at its slot, with the menu's own label ("Build to Folder...").
fn row(b: &mut ShellBuilder, command: &str, label: Option<&str>) {
    let (group, order) = mod_menu_slot(command).unwrap_or((9, 0));
    b.menu_entry(MenuEntry {
        menu: SmolStr::new("Mod"),
        submenu: None,
        group,
        order,
        content: MenuContent::Command { command: SmolStr::new(command), label: label.map(SmolStr::new) },
    });
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.command(UiCommand::new(MOD_SETTINGS, "Mod", "Mod Settings", Icon::Settings, |cx| cx.requests.open(OpenRequest::kind(crate::settings::TAB))).enabled(has_project));
    b.command(UiCommand::new(BUILD_INTO_GAME, "Mod", "Build Into Game", Icon::BuildMod, build::request_build_into_game).keys("Ctrl+B").enabled(|ed, _| can_build_into_game(ed)));
    b.command(UiCommand::new(BUILD_TO_FOLDER, "Mod", "Build to Folder", Icon::GameFolder, |cx| build::request_export(cx, false)).enabled(has_project));
    b.command(UiCommand::new(EXPORT_FLAT, "Mod", "Export Flat Gamedata", Icon::FileArchive, |cx| build::request_export(cx, true)).enabled(has_project));
    b.command(UiCommand::new(PACKAGE, "Mod", "Package Release", Icon::Package, crate::package::request).enabled(has_project));
    row(b, MOD_SETTINGS, None);
    row(b, BUILD_INTO_GAME, None);
    row(b, BUILD_TO_FOLDER, Some("Build to Folder..."));
    row(b, EXPORT_FLAT, Some("Export Flat Gamedata..."));
    row(b, PACKAGE, Some("Package Release..."));
    b.project_toolbar(0, build_mod);
}

/// What the Build Mod split's menu picked.
enum Picked {
    IntoGame,
    ToFolder,
    Flat,
    Package,
}

/// Shipping the mod is the most-used action of the editor: the one Primary button of the row, on
/// every tab (`UITopBarForm::DrawProject`). Its label keeps its width: what a running job does is
/// in the tip, and a click brings the job's card up.
fn build_mod(tb: &mut gz_ui::Toolbar<'_, '_>, cx: &mut DocCx<'_>) {
    crate::cards::ensure(cx.editor);
    let editor = cx.editor.clone();
    let busy = build_running(&editor);
    let target = build_target_text(&editor);
    let tip = if busy {
        format!("{}.\nClick to see the job.", job_status_text(&editor))
    } else if target.is_empty() {
        "Link a game and set a valid module id first.".to_owned()
    } else {
        format!("A clean build into the linked game:\n{target}")
    };
    let mut picked = None;
    tb.group(100);
    let can = can_build_into_game(&editor);
    if tb.split("build_mod", Icon::BuildMod, "Build Mod", &tip, can, Kind::Primary, |ui| {
        if menu_item(ui, "Build Into Game", MenuItem { icon: Icon::BuildMod, enabled: !busy && can, ..MenuItem::default() }) {
            picked = Some(Picked::IntoGame);
        }
        if menu_item(ui, "Build to Folder...", MenuItem { enabled: !busy, ..MenuItem::default() }) {
            picked = Some(Picked::ToFolder);
        }
        if menu_item(ui, "Export Flat Gamedata...", MenuItem { enabled: !busy, ..MenuItem::default() }) {
            picked = Some(Picked::Flat);
        }
        ui.separator();
        if menu_item(ui, "Package Release...", MenuItem { icon: Icon::Package, enabled: !busy, ..MenuItem::default() }) {
            picked = Some(Picked::Package);
        }
    }) {
        picked = Some(Picked::IntoGame);
    }
    match picked {
        Some(Picked::IntoGame) => build::request_build_into_game(cx),
        Some(Picked::ToFolder) => build::request_export(cx, false),
        Some(Picked::Flat) => build::request_export(cx, true),
        Some(Picked::Package) => crate::package::request(cx),
        None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_has_a_slot_in_the_design_order() {
        let mut slots: Vec<(i32, i32)> = MOD_MENU.iter().map(|(_, g, o)| (*g, *o)).collect();
        let before = slots.clone();
        slots.sort_unstable();
        assert_eq!(slots, before);
        assert_eq!(mod_menu_slot(PACKAGE), Some((2, 0)));
        assert_eq!(mod_menu_slot("nothing"), None);
    }
}
