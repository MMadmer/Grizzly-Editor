#pragma once

// The Command Palette (docs/UI_DESIGN.md, section 6): every command of the registry, the Level tab's panels and drawers
// and the open tabs, found by the words of their names. A command's id (COMMAND_*) is MCP's name for it and shows
// only in its tooltip.
namespace CommandPalette
{
struct SResult
{
	xr_string id;			// COMMAND_X, COMMAND_X."sub"; panel:<id>, drawer:<id>, tab:<id> for the rest
	xr_string kind;			// command | panel | drawer | tab
	xr_string category;
	xr_string path;			// the whole menu path: "Compile > Build"
	xr_string name;			// a command's COMMAND_* name; the id for the rest
	xr_string label;		// what the row shows: "Build"
	xr_string shortcut;
	u32 command = u32(-1);	// a command's registry index and preset; u32(-1) for the rest
	u32 subcommand = 0;
	int icon = 0;			// XUI::Icon
	int score = 0;			// lower ranks first
};

ECORE_API void Open(LPCSTR query = "");
ECORE_API void Close();
ECORE_API bool IsOpen();
ECORE_API void Draw();
ECORE_API void Query(LPCSTR query, xr_vector<SResult>& results, u32 limit = 100);
ECORE_API bool Execute(LPCSTR id);
ECORE_API bool Execute(u32 command, u32 subcommand);
}
