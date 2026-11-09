#include "stdafx.h"
#include "LevelMcp.h"
#include "..\..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\..\XrECore\Editor\EditorModPublish.h"
#include "..\..\..\XrECore\Editor\EditorCatalogSubmit.h"
#include "..\..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\..\XrECore\Editor\EditorWorld.h"
#include "..\..\..\XrECore\Editor\EditorScreens.h"
#include "..\..\..\XrECore\Editor\EditorGameModes.h"
#include "..\EditorModScene.h"

// The module over MCP: manifest, build, releases, the Mod Browser, characters, the mode's world, scene layers.
namespace LevelMcp
{
bool HandleMod(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
    // XMS mod commands are project-scoped and do not need an open scene
    if (0 == xr_strcmp(cmd, "mod_manifest"))		{ EditorMod::McpManifest(out);			return true; }
    if (0 == xr_strcmp(cmd, "mod_set_manifest"))	{ EditorMod::McpSetManifest(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_exclude"))		{ EditorMod::McpExclude(raw, out);		return true; }
    // the console commands the module adds: [commands] of mod.ltx
    if (0 == xr_strcmp(cmd, "mod_commands"))		{ EditorMod::McpCommands(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "mod_export"))			{ EditorMod::McpExport(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "mod_package"))			{ EditorMod::McpPackage(raw, out);		return true; }
    // the Mod Settings tab: which page it shows, what stops its Save
    if (0 == xr_strcmp(cmd, "mod_settings_view"))	{ EditorMod::McpSettingsView(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_publish"))			{ EditorPublish::McpPublish(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_releases"))		{ EditorPublish::McpReleases(raw, out);	return true; }
    // the Mod Browser: author key, listing checks, VirusTotal, the submission
    if (0 == xr_strcmp(cmd, "mod_catalog"))			{ EditorCatalogSubmit::McpCatalog(raw, out);	return true; }
    // the module's own NPCs - the asset is text, so this is the primary path
    if (0 == xr_strcmp(cmd, "characters_list"))		{ EditorCharacters::McpList(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "characters_get"))		{ EditorCharacters::McpGet(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "characters_write"))	{ EditorCharacters::McpWrite(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "characters_delete"))	{ EditorCharacters::McpDelete(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "characters_view"))		{ EditorCharacters::McpView(raw, out);	return true; }
    // the modes a module can target, scanned from the linked game install
    if (0 == xr_strcmp(cmd, "game_modes"))			{ EditorGameModes::McpList(out);		return true; }
    // the mode's world: what of the original game it keeps, the area, the transitions
    if (0 == xr_strcmp(cmd, "world_get"))			{ EditorWorld::McpGet(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "screens_get"))			{ EditorScreens::McpGet(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "screens_set"))			{ EditorScreens::McpSet(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "screens_view"))			{ EditorScreens::McpView(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "world_set"))			{ EditorWorld::McpSet(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "world_transitions"))	{ EditorWorld::McpTransitions(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "world_view"))			{ EditorWorld::McpView(raw, out);		return true; }
    if (0 == xr_strcmp(cmd, "mod_export_spawn"))	{ EditorModScene::McpExportSpawn(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_export_cut"))		{ EditorModScene::McpExportCut(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_export_xcform"))	{ EditorModScene::McpExportXCForm(raw, out);	return true; }
    if (0 == xr_strcmp(cmd, "mod_export_ogf_probe")){ EditorModScene::McpExportOGF(raw, out);		return true; }

    // project <-> game link and the read-only DARF content view: project
    // scoped, no scene required
    if (0 == xr_strcmp(cmd, "game_link"))			{ EditorProject::McpGameLink(raw, out);			return true; }

    return false;
}
}
