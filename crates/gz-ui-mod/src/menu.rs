//! The Mod menu and the project toolbar's Build Mod (docs/UI_DESIGN.md, section 5), as actions.
//!
//! The menu's rows stand in the design's order - Mod Settings, Characters, Items, World Rules, Screens
//! & Music, PDA Interface | Build Into Game, Build to Folder..., Export Flat Gamedata..., Bake
//! Scene Layer > | Package Release..., Publish Release..., Releases | Submit to Mod Browser... -
//! as [`MOD_MENU`] gives each its group and order. This package adds the actions whose targets it
//! owns (Mod Settings, the builds, Package Release); the package that owns any other row adds its
//! action and the row at its slot (`ShellBuilder::menu_action("Mod", group, order, id, label)`).
//! Every row runs a command: the tabs open through `ui.tab`, Build Into Game through `mod.build`
//! (a person in the window is asked what the build needs), the dialogs of the other builds and of
//! the packaging through `mod.view dialog=`.

use gz_app::{Action, ActionCx, DocCx, Enabled, Scope, ShellBuilder, args};
use gz_ui::{Icon, Kind, MenuItem, Tx, menu_item, tr};

use crate::build::{build_running, build_target_text, can_build_into_game, job_status_text};

pub const MOD_SETTINGS: &str = "mod.settings";
pub const CHARACTERS: &str = "mod.characters";
pub const ITEMS: &str = "mod.items";
pub const WORLD_RULES: &str = "mod.world_rules";
pub const SCREENS: &str = "mod.screens";
pub const PDA: &str = "mod.pda";
pub const BUILD_INTO_GAME: &str = "mod.build_into_game";
pub const BUILD_TO_FOLDER: &str = "mod.build_to_folder";
pub const EXPORT_FLAT: &str = "mod.export_flat";
/// Not an action: the submenu its owner fills (Spawn Layer..., Collision Overlay..., Visual
/// Overlay..., Level Cut...).
pub const BAKE_SCENE_LAYER: &str = "Bake Scene Layer";
pub const PACKAGE: &str = "mod.package";
pub const PUBLISH: &str = "mod.publish";
pub const RELEASES: &str = "mod.releases";
pub const CATALOG: &str = "mod.submit";

/// Every row of the Mod menu: its action (or, for Bake Scene Layer, the submenu's name), its
/// group (a separator stands between groups) and its order in the group.
pub const MOD_MENU: [(&str, i32, i32); 14] = [
    (MOD_SETTINGS, 0, 0),
    (CHARACTERS, 0, 1),
    (ITEMS, 0, 2),
    (WORLD_RULES, 0, 3),
    (SCREENS, 0, 4),
    (PDA, 0, 5),
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

/// An action of the Mod menu is on with a project open.
#[must_use]
pub fn with_project(cx: &ActionCx<'_>) -> Enabled {
    Enabled::when(cx.editor.project_opt().is_some(), "Open a project first.")
}

/// A row of the Mod menu at its slot.
pub fn row(b: &mut ShellBuilder, id: &str, label: Option<&str>) {
    let (group, order) = mod_menu_slot(id).unwrap_or((9, 0));
    b.menu_action("Mod", group, order, id, label);
}

fn build_to(target: &'static str) -> gz_editor::Args {
    args(&[("target", target)])
}

/// The window's dialog of a build or of the packaging, put up the way the menu does.
fn dialog(name: &'static str) -> gz_editor::Args {
    args(&[("dialog", name)])
}

pub(crate) fn install(b: &mut ShellBuilder) {
    b.action(
        Action::new(MOD_SETTINGS, "Mod Settings", "Mod", "ui.tab")
            .icon(Icon::Package)
            .args(|_| args(&[("action", "open"), ("kind", crate::settings::TAB)]))
            .enabled(with_project),
    );
    b.action(
        Action::new(BUILD_INTO_GAME, "Build Into Game", "Mod", "mod.build")
            .icon(Icon::BuildMod)
            .scope(Scope::Tab(&["level"]))
            .args(|_| build_to("game"))
            .enabled(|cx| {
                Enabled::when(can_build_into_game(cx.editor), "Link a game and set a valid module id first.")
            }),
    );
    b.action(
        Action::new(BUILD_TO_FOLDER, "Build to Folder...", "Mod", "mod.view")
            .icon(Icon::GameFolder)
            .args(|_| dialog("build_to_folder"))
            .enabled(with_project),
    );
    b.action(
        Action::new(EXPORT_FLAT, "Export Flat Gamedata...", "Mod", "mod.view")
            .icon(Icon::FileArchive)
            .args(|_| dialog("export_flat"))
            .enabled(with_project),
    );
    b.action(
        Action::new(PACKAGE, "Package Release...", "Mod", "mod.view")
            .icon(Icon::Package)
            .args(|_| dialog("package_release"))
            .enabled(with_project),
    );
    for id in [MOD_SETTINGS, BUILD_INTO_GAME, BUILD_TO_FOLDER, EXPORT_FLAT, PACKAGE] {
        row(b, id, None);
    }
    b.project_toolbar(0, build_mod);
}

/// Shipping the mod is the most-used action of the editor: the one Primary button of the row, on
/// every tab. Its label keeps its width: what a running job does is in the tip, and a click brings
/// the job's card up.
fn build_mod(tb: &mut gz_ui::Toolbar<'_, '_>, cx: &mut DocCx<'_>) {
    crate::cards::ensure(cx.editor);
    let editor = cx.editor.clone();
    let busy = build_running(&editor);
    let target = build_target_text(&editor);
    let tip = if busy {
        tr!("{status}.\nClick to see the job.", status = job_status_text(&editor))
    } else if target.is_empty() {
        Tx::from("Link a game and set a valid module id first.")
    } else {
        tr!("A clean build into the linked game:\n{target}", target = target)
    };
    let mut picked = None;
    tb.group(100);
    let can = can_build_into_game(&editor);
    if tb.split("build_mod", Icon::BuildMod, "Build Mod", tip, can, Kind::Primary, |ui| {
        for (id, icon, label, on) in [
            (BUILD_INTO_GAME, Icon::BuildMod, "Build Into Game", !busy && can),
            (BUILD_TO_FOLDER, Icon::GameFolder, "Build to Folder...", !busy),
            (EXPORT_FLAT, Icon::FileArchive, "Export Flat Gamedata...", !busy),
        ] {
            if menu_item(ui, label, MenuItem { icon, enabled: on, ..MenuItem::default() }) {
                picked = Some(id);
            }
        }
        ui.separator();
        if menu_item(
            ui,
            "Package Release...",
            MenuItem { icon: Icon::Package, enabled: !busy, ..MenuItem::default() },
        ) {
            picked = Some(PACKAGE);
        }
    }) {
        picked = Some(BUILD_INTO_GAME);
    }
    if let Some(id) = picked {
        cx.requests.action(id);
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
