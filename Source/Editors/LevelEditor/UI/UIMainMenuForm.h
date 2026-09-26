#pragma once
// The caption row's menus (docs/UI_DESIGN.md, section 5): File, Edit, the active tab's own (Scene, Build and
// View on Level; Graph on a graph), Mod, Libraries, Window, Help. Under 1100 pt they fold into one button.
class UIMainMenuForm :public XrUI
{
public:
	UIMainMenuForm();
	virtual ~UIMainMenuForm();
	virtual void Draw() { Draw(true, false); }
	// the whole caption: `menus` false on the Start page, `busy` while a blocking job runs (all disabled)
	void Draw(bool menus, bool busy);
	// the Level tab's menus: Scene, Build, View
	static void DrawLevelMenus();
	// the render, draw and environment switches of the viewport (View menu)
	static void DrawViewportOptions();
	// the window is narrower than 1100 pt: the menus are one button (ui_popup menu:Menu)
	static bool Folded();
};
