#pragma once

// The editor's own MCP commands (docs/UI_DESIGN.md, section 7). XFinedMCP answers the core's (ping, state, the
// screenshots, projects, open_scene) and hands every other command to XFinedInspector, which asks these groups in
// turn. A group answers true when it took the command.
namespace LevelMcp
{
// LevelMcp.cpp: the frame - tabs, panels, drawers, dialogs, popups, scroll, splits, audit, scale, the window,
// theme, palette, progress, recovery, project_close, preferences, the command registry
bool HandleShell	(LPCSTR cmd, LPCSTR raw, xr_string& out);
bool HandleScene	(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpScene.cpp
bool HandleProps	(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpProps.cpp
bool HandleContent	(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpContent.cpp
bool HandleGraph	(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpGraph.cpp
bool HandlePreview	(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpPreview.cpp
bool HandleMod		(LPCSTR cmd, LPCSTR raw, xr_string& out);	// LevelMcpMod.cpp

// shared by the groups (LevelMcp.cpp)
xr_string		JsonStr				(LPCSTR s);
xr_string		JsonEscapePath		(LPCSTR text);
bool			GetArgBool			(LPCSTR raw, LPCSTR field, bool def);
int				GetArgInt			(LPCSTR raw, LPCSTR field, int def);
bool			GetArgFloat			(LPCSTR raw, LPCSTR field, float& out);
CCustomObject*	FindObjectRelaxed	(LPCSTR name);
}

// UI_LevelMain.cpp keeps the scene IO state these answer from
void	LevelMcpSceneLoadJson	(xr_string& out);
// COMMAND_SAVE to `file` with its dialog kept down; false with the reason
bool	LevelMcpSaveScene		(const xr_string& file, xr_string& err);
void	CollectSelectedObjects	(ObjectList& out);
u32		DropObjectsToGround		(ObjectList& objects, bool align_normal, bool line_trace);
void	RetrieveSceneObjPointAndNormal(Fvector& hitpoint, Fvector* hitnormal, const SRayPickInfo& pinf, int bSnap);
// How far down a sweep looks for support.
inline constexpr float kDropReach = 1000.f;
