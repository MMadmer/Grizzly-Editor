#pragma once

// The UI constants that MCP answers and QA pin (docs/UI_DESIGN.md, section 7). Each one is defined here once, the
// code that draws it uses it from here, and MCP ui_contract reports them all, so a test reads the value instead of
// hardcoding it. A change here changes what agents and QA see.
namespace EditorUiContract
{
// ---- Content Browser tiles (pt) -----------------------------------------------------------------------------
inline constexpr float	kTileMin			= 48.f;
inline constexpr float	kTileMax			= 192.f;
// the most a fitted tile takes (a host with no size picked fits two rows, 48 pt at least): the Content Drawer ...
inline constexpr float	kTileDefault		= 96.f;
// ... and the docked panel, whose Wide-layout strip fits 48
inline constexpr float	kTileDefaultPanel	= 64.f;
inline constexpr const char* kTileRangeError	= "tile must be 48..192";

// ---- Start page: a project card's menu ----------------------------------------------------------------------
inline constexpr const char* kCardShowInExplorer	= "Show in Explorer";
inline constexpr const char* kCardDelete			= "Delete...";
inline constexpr const char* kCardRemoveFromList	= "Remove from List";
inline constexpr const char* kDeleteProjectDialog	= "Delete Project###dlg.delete_project";

// ---- the Level tab's panels: the MCP id and the window (its id after ###) ---------------------------------------
inline constexpr const char* kWindowTools			= "Tools###LeftBar";
inline constexpr const char* kWindowOutliner		= "World Outliner";
inline constexpr const char* kWindowDetails			= "Details###Properties";
inline constexpr const char* kWindowLevelSettings	= "Level Settings###World Properties";
inline constexpr const char* kWindowContentBrowser	= "Content Browser";
inline constexpr const char* kWindowOutputLog		= "Output Log###Log";
inline constexpr const char* kWindowViewport		= "Viewport###Render";

struct SPanel { const char* id; const char* window; };
inline constexpr SPanel kPanels[] =
{
	{ "tools",				kWindowTools },
	{ "world_outliner",		kWindowOutliner },
	{ "details",			kWindowDetails },
	{ "level_settings",		kWindowLevelSettings },
	{ "content_browser",	kWindowContentBrowser },
	{ "output_log",			kWindowOutputLog },
	{ "viewport",			kWindowViewport },
};

// ---- the Start page: the page window the project list, Link Game and Recovery draw in ------------------------------
inline constexpr const char* kWindowStartPage		= "Start Page###start_page";

// ---- drawers over the bottom of any tab ------------------------------------------------------------------------
struct SDrawer { const char* id; const char* window; };
inline constexpr SDrawer kDrawers[] =
{
	{ "content",	"Content Drawer###drawer.content" },
	{ "log",		"Output Log###drawer.log" },
};

// ---- dialogs ui_dialog opens and closes: the MCP id and the window's name ---------------------------------------
// "Title###dlg.<name>" (section 4): the title the author reads, the id MCP and the kit know it by. Mod Settings and
// Releases are documents (kDocumentKinds), not dialogs.
inline constexpr const char* kDialogPackageRelease	= "Package Release###dlg.package_release";
inline constexpr const char* kDialogPublishRelease	= "Publish Release###dlg.publish_release";
inline constexpr const char* kDialogBuildToFolder	= "Build to Folder###dlg.build_to_folder";
inline constexpr const char* kDialogExportFlat		= "Export Flat Gamedata###dlg.export_flat";
inline constexpr const char* kDialogImportBaseScene	= "Import Base Scene###dlg.import_base_scene";
inline constexpr const char* kDialogSubmit			= "Submit to Mod Browser###dlg.catalog_submit";
// raised by a job or a row, never opened over ui_dialog (kKitDialogs lists their names)
inline constexpr const char* kDialogEditRelease		= "Edit Release###dlg.edit_release";
inline constexpr const char* kDialogCatalogWithdraw	= "Withdraw from Mod Browser###dlg.catalog_withdraw";

struct SDialog { const char* id; const char* window; };
inline constexpr SDialog kDialogs[] =
{
	{ "package_release",	kDialogPackageRelease },
	{ "publish_release",	kDialogPublishRelease },
	{ "build_to_folder",	kDialogBuildToFolder },
	{ "export_flat",		kDialogExportFlat },
	{ "import_base_scene",	kDialogImportBaseScene },
	{ "submit",				kDialogSubmit },
};

// ---- documents: the kinds of major tab (their ids: "level", "<kind>:<rel>" for files, "<kind>" for singletons) --
inline constexpr const char* kDocumentKinds[] =
{
	"level", "quest", "beh", "pda", "model", "texture", "characters", "mod_settings", "world_rules", "screens", "releases",
	"preferences", "light_animations", "texture_library", "sound_library",
};

// ---- kit dialogs a row, a document or a question raises (none opens over ui_dialog; each closes with it, as Esc) ---
inline constexpr const char* kKitDialogs[] =
{
	// Details and Preferences (B)
	"dlg.disable_game_updates", "dlg.choose", "dlg.edit_text", "dlg.game_type", "dlg.assign_shortcut", "dlg.text_editor", "dlg.properties",
	"dlg.numeric_vector",
	// the scene (C)
	"dlg.rename_object", "dlg.delete_object", "dlg.detail_objects",
	// the graph tab (E)
	"dlg.nq_project_find", "dlg.nq_task_rename", "dlg.nq_task_references", "dlg.nq_text", "dlg.nq_reload", "dlg.nq_overwrite",
	// builds, characters, World Rules (G)
	"dlg.build_result", "dlg.export_result", "dlg.package_result", "dlg.bake_result", "dlg.clean_target", "dlg.replace_release",
	"dlg.delete_character", "dlg.empty_world",
	// releases and the Mod Browser (H)
	"dlg.edit_release", "dlg.catalog_withdraw",
	// the Start page (I)
	"dlg.delete_project",
};

// ---- the Task Center's card: the window of the jobs that run beside the editor (ui_popup show=tasks) ----------------
inline constexpr const char* kTaskCard			= "Tasks###tasks_card";

// ---- the Command Palette: the modal's id (screenshot_editor modal.id) ---------------------------------------------
inline constexpr const char* kPaletteId			= "grizzly_command_palette";

// ---- popups ui_popup puts up to be looked at ---------------------------------------------------------------------
inline constexpr const char* kPopups[] =
{
	"menu:<name>", "tab_menu:<tab>", "tab_overflow", "toolbar_overflow", "toolbar_split:<id>", "cb_context:<path>", "cb_add",
	"cb_view", "cb_favorites", "cb_grid", "outliner_more",
	"outliner_filters", "viewport:<menu>", "tasks", "rebuild",
};
// the caption's menus, in their order (menu:<name>); Graph is a graph tab's own
inline constexpr const char* kMenus[] =
{
	"File", "Edit", "Scene", "Build", "View", "Graph", "Mod", "Libraries", "Window", "Help", "Menu",
};
// the viewport's menus (viewport:<menu>)
inline constexpr const char* kViewportMenus[] = { "options", "perspective", "show", "snap", "grid", "scale", "angle" };

// ---- ui_scale: tests only, never saved ----------------------------------------------------------------------------
inline constexpr float	kUiScaleMin	= 1.f;
inline constexpr float	kUiScaleMax	= 2.5f;
}
