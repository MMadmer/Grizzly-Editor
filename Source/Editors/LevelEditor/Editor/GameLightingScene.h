#pragma once

// View > Game Lighting, the level editor's half (EGameLighting in XrECore is the other): before every scene pass it
// gathers the lights the game would draw near the camera and keeps the roof map current, after it takes the lighting
// away again. ESceneLightTool::BeforeRender / AfterRender call these.
void	GameLighting_BeforeRender	();
void	GameLighting_AfterRender	();
// drops the roof map; the next scene pass bakes it again
void	GameLighting_Rebake			();
// the body of the View > Game Lighting menu
void	GameLighting_Menu			();
// MCP game_lighting (docs: tools/mcp/AI_SETUP.md)
void	GameLighting_Mcp			(LPCSTR raw, xr_string& out);
// the lighting as view_mode reports it
void	GameLighting_JsonState		(xr_string& out);
