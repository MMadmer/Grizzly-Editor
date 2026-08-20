#pragma once

// The LevelEditor half of the custom chrome: draws the icon, the centered
// title and the window buttons inside the main menu bar, and answers the
// WndProc hit-test with what it drew last frame.
namespace UITitleBar
{
// call once after the UI exists: registers the hit-test with EditorTitleBar
void	Install();
// inside ImGui::BeginMainMenuBar(), BEFORE the first menu: the icon
void	DrawLeft();
// inside the bar, AFTER the last menu: title + minimize/maximize/close
void	DrawRight();
// MCP: geometry and state of the band, JSON object appended to out
void	McpInfo(xr_string& out);
}
