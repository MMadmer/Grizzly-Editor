#pragma once

// The LevelEditor half of the custom chrome (docs/UI_DESIGN.md, section 5): the caption row is a main menu
// bar drawn in every state - the logo, the menus, the major tabs, a drag band with the project's name and the
// window buttons - and the WndProc hit test replays what it drew last frame: menus, tabs and buttons are client
// area, the rest of the band drags the window.
namespace UITitleBar
{
// call once after the UI exists: registers the hit test with EditorTitleBar
void	Install();
// the caption row (32 pt); false when it is not drawn, End() follows a true return
bool	Begin();
void	End();
// the logo, before the first menu
void	DrawLeft();
// where the menus and the major tabs must stop: the drag band and the window buttons come after them
float	TabsMaxX();
// after the last tab: the drag band with the project's name, the window buttons
void	DrawRight(bool busy);
// MCP titlebar_info: the band, the menus' end, the buttons, the tabs, what is interactive
void	McpInfo(xr_string& out);
}
