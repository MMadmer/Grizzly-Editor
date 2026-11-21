//! The UI constants MCP answers and QA pins.
//!
//! Each one is defined here once; the code that draws it uses it from here and `ui.contract`
//! reports them all, so a test reads the value instead of hardcoding it.

/// Content Browser tiles (pt).
pub const TILE_MIN: f32 = 48.0;
pub const TILE_MAX: f32 = 192.0;
/// The most a fitted tile takes in the Content Drawer.
pub const TILE_DEFAULT: f32 = 96.0;
/// ... and in the docked panel, whose Wide strip fits 48.
pub const TILE_DEFAULT_PANEL: f32 = 64.0;
pub const TILE_RANGE_ERROR: &str = "tile must be 48..192";

/// The Start page: a project card's menu.
pub const CARD_SHOW_IN_EXPLORER: &str = "Show in Explorer";
pub const CARD_DELETE: &str = "Delete...";
pub const CARD_REMOVE_FROM_LIST: &str = "Remove from List";
pub const DELETE_PROJECT_DIALOG: &str = "Delete Project###dlg.delete_project";

/// The Level tab's panels: the MCP id and the window (its id after `###`).
pub const PANELS: [(&str, &str); 8] = [
    ("place", "Place###LeftBar"),
    ("world_outliner", "World Outliner"),
    ("details", "Details###Properties"),
    ("level_settings", "Level Settings###World Properties"),
    ("content_browser", "Content Browser"),
    ("output_log", "Output Log###Log"),
    ("problems", "Problems###level.problems"),
    ("viewport", "Viewport###Render"),
];

/// The page the project list, Link Game and Recovery draw in.
pub const WINDOW_START_PAGE: &str = "Start Page###start_page";

/// The drawers over the bottom of any tab.
pub const DRAWERS: [(&str, &str); 2] =
    [("content", "Content Drawer###drawer.content"), ("log", "Output Log###drawer.log")];

/// The dialogs `ui.dialog` opens and closes: the MCP id and the window's name.
pub const DIALOGS: [(&str, &str); 8] = [
    ("package_release", "Package Release###dlg.package_release"),
    ("publish_release", "Publish Release###dlg.publish_release"),
    ("build_to_folder", "Build to Folder###dlg.build_to_folder"),
    ("export_flat", "Export Flat Gamedata###dlg.export_flat"),
    ("import_base_scene", "Import Base Scene###dlg.import_base_scene"),
    ("submit", "Submit to Mod Browser###dlg.catalog_submit"),
    ("update", "Update###dlg.update"),
    ("feedback", "Send Feedback###dlg.feedback"),
];

/// The kinds of major tab.
pub const DOCUMENT_KINDS: [&str; 19] = [
    "level",
    "quest",
    "beh",
    "pda",
    "model",
    "texture",
    "sound",
    "characters",
    "items",
    "economy",
    "mod_settings",
    "world_rules",
    "screens",
    "releases",
    "preferences",
    "light_animations",
    "texture_library",
    "sound_library",
    "text",
];

/// Kit dialogs a row, a document or a question raises; none opens over `ui.dialog`, each closes
/// with it, as Esc.
pub const KIT_DIALOGS: [&str; 31] = [
    "dlg.disable_game_updates",
    "dlg.choose",
    "dlg.edit_text",
    "dlg.game_type",
    "dlg.assign_shortcut",
    "dlg.text_editor",
    "dlg.properties",
    "dlg.numeric_vector",
    "dlg.rename_object",
    "dlg.delete_object",
    "dlg.colour_indices",
    "dlg.nq_project_find",
    "dlg.nq_task_rename",
    "dlg.nq_task_references",
    "dlg.nq_text",
    "dlg.nq_reload",
    "dlg.nq_overwrite",
    "dlg.build_result",
    "dlg.export_result",
    "dlg.package_result",
    "dlg.bake_result",
    "dlg.clean_target",
    "dlg.replace_release",
    "dlg.delete_character",
    "dlg.delete_item",
    "dlg.delete_table",
    "dlg.empty_world",
    "dlg.edit_release",
    "dlg.catalog_withdraw",
    "dlg.delete_project",
    "dlg.clean_level",
];

/// The Task Center's card.
pub const TASK_CARD: &str = "Tasks###tasks_card";

/// The Command Palette: the modal's id (`shot.window` `modal.id`).
pub const PALETTE_ID: &str = "grizzly_command_palette";
/// Its window.
pub const PALETTE_WINDOW: &str = "Command Palette###grizzly_command_palette";

/// The popups `ui.popup show=` puts up to be looked at.
pub const POPUPS: [&str; 15] = [
    "menu:<name>",
    "tab_menu:<tab>",
    "tab_overflow",
    "toolbar_overflow",
    "toolbar_split:<id>",
    "cb_context:<path>",
    "cb_add",
    "cb_view",
    "cb_favorites",
    "cb_grid",
    "outliner_more",
    "outliner_filters",
    "viewport:<menu>",
    "tasks",
    "rebuild",
];

/// The pick lists of a surface, each put up by its own view command (written as the call:
/// `<command> show=<value>`), which opens the tab, the page and the fold that hold the list. A
/// kit pick list is no menu, so `ui.list` does not name it: `ui.audit` lists its rows and
/// `shot.window` shows it.
pub const VIEW_POPUPS: [&str; 24] = [
    "world.view show=add_level",
    "world.view show=add_area_level",
    "mod.view show=add_requires",
    "mod.view show=add_load_after",
    "mod.view show=add_load_before",
    "mod.view show=add_conflicts",
    "mod.view show=function",
    "characters.view show=base",
    "characters.view show=visual",
    "characters.view show=icon",
    "characters.view show=community",
    "characters.view show=rank",
    "characters.view show=dialog",
    "characters.view show=sells",
    "characters.view show=buys",
    "items.view show=add",
    "items.view show=new",
    "items.view show=change",
    "items.view show=base",
    "library.view kind=texture_library show=detail",
    "library.view kind=texture_library show=bump",
    "library.view kind=texture_library show=normal",
    "preview.sound show=streamer",
    "text.view show=symbols",
];

/// The caption's menus, in their order; Graph is a graph tab's own, Menu the folded button.
pub const MENUS: [&str; 11] =
    ["File", "Edit", "Scene", "Build", "View", "Graph", "Mod", "Libraries", "Window", "Help", "Menu"];

/// The viewport's menus (`viewport:<menu>`).
pub const VIEWPORT_MENUS: [&str; 7] =
    ["perspective", "view_mode", "show", "surface", "grid", "angle", "scale"];

/// `ui.scale`: tests only, never saved.
pub const UI_SCALE_MIN: f32 = 1.0;
pub const UI_SCALE_MAX: f32 = 2.5;

/// Preferences > Interface > Interface Scale: Auto, then these percentages.
pub const INTERFACE_SCALES: [&str; 6] = ["auto", "100", "125", "150", "175", "200"];

/// The kinds of the Content Browser's Library, as it names them.
pub const CATEGORIES: [&str; 7] =
    ["Objects", "Groups", "Textures", "Sounds", "Particles", "Spawn Sections", "Base Scenes"];

/// The window of a panel, a drawer or a dialog by its MCP id.
#[must_use]
pub fn window_of_known(id: &str) -> Option<&'static str> {
    PANELS
        .iter()
        .chain(DRAWERS.iter())
        .chain(DIALOGS.iter())
        .find(|(i, _)| *i == id)
        .map(|(_, w)| *w)
        .or((id == "start_page").then_some(WINDOW_START_PAGE))
}

/// The panel window of an MCP panel id, `""` when it is none.
#[must_use]
pub fn panel_window(id: &str) -> &'static str {
    PANELS.iter().find(|(i, _)| *i == id).map_or("", |(_, w)| *w)
}

#[cfg(test)]
mod tests {
    use super::{DIALOGS, PANELS, window_of_known};

    #[test]
    fn dialogs_are_kit_named() {
        assert!(DIALOGS.iter().all(|(_, w)| w.contains("###dlg.")));
    }

    #[test]
    fn windows_resolve() {
        assert_eq!(window_of_known("details"), Some("Details###Properties"));
        assert_eq!(window_of_known("log"), Some("Output Log###drawer.log"));
        assert_eq!(window_of_known("start_page"), Some("Start Page###start_page"));
        assert_eq!(window_of_known("nothing"), None);
        assert_eq!(PANELS.len(), 8);
    }
}
