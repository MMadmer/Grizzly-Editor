//! The Mod menu and the project toolbar's Build Mod (docs/UI_DESIGN.md, section 5), as actions.
//!
//! The menu's rows stand in the design's order - Mod Settings, Characters, World Rules, Screens
//! & Music, PDA Interface | Build Into Game, Build to Folder..., Export Flat Gamedata..., Bake
//! Scene Layer > | Package Release..., Publish Release..., Releases | Submit to Mod Browser... -
//! as [`MOD_MENU`] gives each its group and order. This package adds the actions whose targets it
//! owns (Mod Settings, the builds, Package Release); the package that owns any other row adds its
//! action and the row at its slot (`ShellBuilder::menu_action("Mod", group, order, id, label)`).
//! Every row runs a command: the tabs open through `ui.tab`, the builds through `mod.build`.

use std::sync::Arc;

use gz_app::{Action, ActionCx, DocCx, Enabled, Scope, ShellBuilder, args};
use gz_editor::{Editor, Origin, Registry, Reply, Spec, json};
use gz_ui::{Icon, Kind, MenuItem, menu_item};
use schemars::JsonSchema;
use serde::Deserialize;

use crate::build::{self, build_running, build_target_text, can_build_into_game, job_status_text};

pub const MOD_SETTINGS: &str = "mod.settings";
pub const CHARACTERS: &str = "mod.characters";
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

fn build_to(to: &'static str) -> gz_editor::Args {
    args(&[("to", to)])
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
            .keys(const { &[gz_app::Chord::ctrl(egui::Key::B)] })
            // a graph keeps Ctrl+B for its bookmarks
            .scope(Scope::Tab(&["level"]))
            .args(|_| build_to("game"))
            .enabled(|cx| {
                Enabled::when(can_build_into_game(cx.editor), "Link a game and set a valid module id first.")
            }),
    );
    b.action(
        Action::new(BUILD_TO_FOLDER, "Build to Folder...", "Mod", "mod.build")
            .icon(Icon::GameFolder)
            .args(|_| build_to("folder"))
            .enabled(with_project),
    );
    b.action(
        Action::new(EXPORT_FLAT, "Export Flat Gamedata...", "Mod", "mod.build")
            .icon(Icon::FileArchive)
            .args(|_| build_to("flat"))
            .enabled(with_project),
    );
    b.action(
        Action::new(PACKAGE, "Package Release...", "Mod", "mod.build")
            .icon(Icon::Package)
            .args(|_| build_to("release"))
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum BuildTo {
    /// Into the linked game, `<game>\modules\<id>`.
    #[default]
    Game,
    /// Into a folder of the author's (`target`); the window asks for it.
    Folder,
    /// A flat `gamedata_<id>` in a folder (`target`); the window asks for it.
    Flat,
    /// The window's Package Release dialog (over MCP: `mod_package`).
    Release,
}

#[derive(Debug, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct BuildParams {
    /// Where the module goes.
    #[serde(default)]
    to: BuildTo,
    /// The folder for folder and flat over MCP (the window asks for it).
    #[serde(default)]
    target: Option<String>,
}

/// `mod.build`: from the window, the build flows with their card and result dialog (Build to
/// Folder, Export Flat and Package Release open their dialogs); over MCP the build itself, its
/// answer when it ended.
async fn mod_build(ed: Arc<Editor>, p: BuildParams, origin: Origin) -> Reply {
    ed.project()?;
    let window = origin != Origin::Mcp && gz_app::UiQueue::of(&ed).is_ok();
    if window {
        let to = p.to;
        gz_app::on_ui(&ed, move |shell, ctx| {
            shell.with_cx(ctx, |cx| match to {
                BuildTo::Game => build::request_build_into_game(cx),
                BuildTo::Folder => build::request_export(cx, false),
                BuildTo::Flat => build::request_export(cx, true),
                BuildTo::Release => crate::package::request(cx),
            });
            Ok(())
        })
        .await?;
        return Ok(json!({ "started": true }));
    }
    let mut a = gz_editor::Args::new();
    match p.to {
        BuildTo::Game => {}
        BuildTo::Folder | BuildTo::Flat => {
            let t = p
                .target
                .filter(|t| !t.is_empty())
                .ok_or_else(|| gz_editor::CmdError::invalid("folder and flat take target="))?;
            a.insert("target".into(), json!(t));
            if p.to == BuildTo::Flat {
                a.insert("flat".into(), json!(true));
            }
        }
        BuildTo::Release => {
            return Err(gz_editor::CmdError::new(
                "refused",
                "a release is packaged with mod_package over MCP",
            ));
        }
    }
    let reply = ed.call("mod_export", a, origin).await;
    if reply.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        Ok(reply)
    } else {
        let why =
            reply.get("error").and_then(serde_json::Value::as_str).unwrap_or("the build failed").to_owned();
        Err(gz_editor::CmdError::new("failed", why))
    }
}

/// Registers `mod.build`.
pub(crate) fn register(reg: &mut Registry) {
    reg.area("mod", "The module: its settings, building it, packaging it");
    reg.cmd(
        Spec::write("mod.build", "Builds the module into the linked game, a folder or a flat gamedata; the window asks what it needs.")
            .example("to=game"),
        mod_build,
    );
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
