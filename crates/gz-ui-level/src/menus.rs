//! The Level tab's caption menus - Scene, Build, View - as lists of actions ([`crate::actions`]):
//! every row runs an action, shows the keys bound to it now, and is off with its reason when it
//! cannot run. The File menu's rows of the scene are the shell's menu entries
//! ([`crate::actions::FILE_MENU`]); the Edit menu is the shell's and acts on the selection,
//! whatever kinds it holds (the Level document's `edit`).

use egui::Ui;
use gz_app::chrome::{MenuBar, local_submenu};
use gz_app::{DocCx, menu_row};
use gz_levelbuild::{LevelBuild, StepState};
use gz_ui::{Icon, MenuItem, Tx, menu_item};

use crate::actions::{self as a, step_action};

/// The Scene menu, in its groups.
pub const SCENE_MENU: &[&[&str]] = &[
    &[a::SELECT_ALL, a::DESELECT_ALL, a::INVERT_SELECTION],
    &[a::HIDE_SELECTED, a::HIDE_UNSELECTED, a::SHOW_ALL],
    &[a::GROUP, a::UNGROUP, a::SAVE_GROUP],
    &[a::MARK_MOD, a::MARK_BASE],
    &[a::BROWSE_TO_ASSET, a::SUMMARY],
];

/// The Build menu above its submenus, in its groups.
pub const BUILD_MENU: &[&[&str]] = &[
    &[a::BUILD_LEVEL, a::REBUILD_ALL],
    &[a::PLAY, a::PLAY_CLEAN, a::PLAY_WITHOUT_BUILDING, a::STOP],
    &[a::VALIDATE, a::SHOW_PROBLEMS, a::CLEAR_PROBLEMS],
];

/// Build > Advanced and Build Level's More: the mod's level layers, the exports, the compiler of a
/// folder, the game's configs and folder.
pub const ADVANCED: &[&[&str]] = &[
    &[a::BUILD_DRAFT, a::BUILD_FINAL],
    &a::LAYER_ACTIONS,
    &[a::EXPORT_OBJ, a::EXPORT_SELECTION_OBJ, a::COMPILE_FOLDER],
    &[a::REMOVE_BUILD_FILES],
    &[a::RELOAD_CONFIGS, a::GAME_FOLDER],
];

/// The rows of groups of actions, a separator between the groups.
pub fn rows(ui: &mut Ui, cx: &mut DocCx<'_>, groups: &[&[&str]]) {
    // an action's title is a verb where the same word is a noun elsewhere ("Group")
    let ctx = ui.ctx().clone();
    gz_ui::i18n::scope(&ctx, "action", || {
        for (i, g) in groups.iter().enumerate() {
            if i > 0 {
                ui.separator();
            }
            for id in *g {
                menu_row(ui, cx, id, None);
            }
        }
    });
}

/// A step's state as its row says it: "Fresh", "1 object moved.", "Failed: ...".
#[must_use]
pub fn state_word(state: StepState) -> &'static str {
    match state {
        StepState::Fresh => "Fresh",
        StepState::Stale => "Stale",
        StepState::Never => "Never Built",
        StepState::Failed => "Failed",
        StepState::NotNeeded => "Not Needed",
    }
}

/// A step's title as the window shows it: the steps' words have a context of their own ("Light"
/// the build step, not the object; "Details" the detail objects, not the panel).
#[must_use]
pub fn step_tx(title: &'static str) -> Tx<'static> {
    Tx::ctx(STEP_CONTEXT, title)
}

/// The catalog's context of the build steps' titles.
pub const STEP_CONTEXT: &str = "build_step";

/// The steps of the build, each with its state, building it when picked.
pub fn step_rows(ui: &mut Ui, cx: &mut DocCx<'_>) {
    let status = LevelBuild::status_now(cx.editor);
    let Some(status) = status else {
        menu_item(ui, "Reading the build's state...", MenuItem { enabled: false, ..MenuItem::default() });
        return;
    };
    for s in &status.steps {
        // the row's keys column holds the state; its tip says why it is not fresh
        let state = gz_ui::i18n::t(ui.ctx(), state_word(s.state));
        let item = MenuItem {
            keys: &state,
            enabled: !crate::actions::running(cx.editor),
            tip: if s.why.is_empty() {
                Tx::from("Built from what the scene holds now.")
            } else {
                Tx::ui(&s.why)
            },
            icon: match s.state {
                StepState::Failed => Icon::Error,
                StepState::Stale | StepState::Never => Icon::Warning,
                StepState::Fresh => Icon::Success,
                StepState::NotNeeded => Icon::None,
            },
            ..MenuItem::default()
        };
        if menu_item(ui, step_tx(s.step.title()), item) {
            cx.requests.action(step_action(s.step));
        }
    }
}

/// Draws Scene, Build and View.
pub fn draw(bar: &mut MenuBar<'_, '_>, cx: &mut DocCx<'_>) {
    bar.menu("Scene", |ui| rows(ui, cx, SCENE_MENU));
    bar.menu("Build", |ui| {
        rows(ui, cx, &BUILD_MENU[..1]);
        local_submenu(ui, "Build Step", Icon::None, true, |ui| step_rows(ui, cx));
        ui.separator();
        rows(ui, cx, &BUILD_MENU[1..]);
        ui.separator();
        local_submenu(ui, "Advanced", Icon::None, true, |ui| rows(ui, cx, ADVANCED));
    });
    bar.menu("View", |ui| {
        gz_ui_viewport::view_menu(ui, cx);
        ui.separator();
        menu_row(ui, cx, a::MAXIMIZE, None);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_row_is_an_action_of_the_level() {
        let ids: Vec<&str> = crate::actions::actions()
            .iter()
            .chain(gz_ui_viewport::actions::actions().iter())
            .map(|x| x.id)
            .collect();
        for g in SCENE_MENU.iter().chain(BUILD_MENU).chain(ADVANCED) {
            for id in *g {
                assert!(ids.contains(id) || a::LAYER_ACTIONS.contains(id), "{id} is no action");
            }
        }
        // the Scene menu stays short: it is not a drawer of everything
        assert!(SCENE_MENU.iter().map(|g| g.len()).sum::<usize>() <= 13);
    }
}
