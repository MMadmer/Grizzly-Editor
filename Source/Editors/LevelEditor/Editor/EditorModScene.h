#pragma once

// XMS scene-side mod exports (level overlays).
//
// Bakes editor-placed scene objects into module deltas inside the active
// project, for the game engine to merge into a base level at load time - the
// base level files are never rewritten, so two modules can edit one map:
//   spawn ops  -> spawn\<level>.xspawn                     (preferred for props)
//   collision  -> levels\<level>\overlay.xcform            (XCF1 binary)
//   visuals    -> levels\<level>\visuals\<object>.ogf      (+ overlay_visuals.ltx)
// Menu entries run on the current selection and report in the result dialog every
// build shares; the MCP handlers take level/sector/selected_only arguments and
// answer in JSON. Mirrors EditorMod style.

namespace EditorModScene
{
	// Mod > Bake Scene Layer triggers (selected objects, sector 0)
	void RequestExportSpawn		();
	void RequestExportCut		();
	void RequestExportXCForm	();
	void RequestExportOGF		();

	// MCP command handlers: fill the full JSON response; raw = request line
	void McpExportSpawn			(LPCSTR raw, xr_string& out);	// mod_export_spawn
	void McpExportCut			(LPCSTR raw, xr_string& out);	// mod_export_cut
	void McpExportXCForm		(LPCSTR raw, xr_string& out);	// mod_export_xcform
	void McpExportOGF			(LPCSTR raw, xr_string& out);	// mod_export_ogf_probe

	// Visual checks shared with spawn_set_visual: a meshes-relative name (no
	// extension) resolved the way the game will - the project's gamedata first,
	// then the linked game content. DrawsNothing detects the glow_ghost skin
	// X-Ray uses for models that exist but are never rendered (stash boxes).
	bool VisualReachable		(LPCSTR visual);
	bool VisualDrawsNothing		(LPCSTR visual);

	// pre-build hook (EditorMod::SetPreBuildBake): mirrors the OPEN scene into
	// its level's spawn file, so "save scene, press Build" ships what you see -
	// nobody remembers a separate bake step, and stale ops of this level die.
	// No scene open = does nothing. Other levels' files are never touched.
	void BakeCurrentSceneSpawn	(xr_string& note);

	// the level the open scene bakes into (spawn\<level>.xspawn); false with no scene
	bool SceneLevelName			(xr_string& level);
}
