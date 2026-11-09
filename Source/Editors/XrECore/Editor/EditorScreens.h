#pragma once

#include "EditorModManifest.h"

//------------------------------------------------------------------------------
// The module's loading and menu screens and their music: mod.ltx [screens], [screen.<id>], [track.<id>]. What it means
// in the game is the game repo's docs/dead-air/MODDING.md, "Screens and music"; the tab is docs/UI_DESIGN.md.
//
// A screen is a stack of layers, as many as the author wants, each holding one file: a picture, a GIF, a video, a shader
// or an HTML page. Files dropped from outside the project are copied into <project>\screens.
//------------------------------------------------------------------------------
namespace EditorScreens
{
	// E4nn stop a build, W4nn are warnings:
	//   E401 a layer's or a track's file does not exist      E402 a layer's file is no kind the game shows
	//   E403 a screen names a track the module does not have E404 a shader has no .ps beside it
	//   E408 the build leaves out a file a screen needs      E409 a track is not an .ogg
	//   W405 a layer nothing was put on                      W406 a screen with no layers
	//   W407 a level the linked game lacks                   W410 a track that plays nowhere
	//   W411 a screen asks for a video's sound and has no video
	ECORE_API void	Validate	(LPCSTR project_root, const EditorMod::SManifest& m, xr_vector<xr_string>& problems);

	ECORE_API void	DrawUI		();
	ECORE_API void	Show		();
	ECORE_API bool	IsOpen		();

	ECORE_API void	McpGet		(LPCSTR raw, xr_string& out);		// screens_get
	ECORE_API void	McpSet		(LPCSTR raw, xr_string& out);		// screens_set
	ECORE_API void	McpView		(LPCSTR raw, xr_string& out);		// screens_view
}
