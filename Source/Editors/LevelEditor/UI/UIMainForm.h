#pragma once
// The LevelEditor's frame (docs/UI_DESIGN.md, section 5): the caption, the active tab's toolbar and the status
// bar around the work area of XUI::Shell, and the Level tab itself - the XFinedLevel_v2 workspace, laid out per
// size class, that the scene's panels dock into.
class UIMainForm :public XrUI
{
public:
	UIMainForm();
	virtual ~UIMainForm();
	virtual void Draw();
	bool Frame();
	IC UILeftBarForm* GetLeftBarForm() {return m_LeftBar;}
	IC UITopBarForm* GetTopBarForm() { return m_TopBar; }
	IC UIRenderForm* GetRenderForm() { return m_Render; }
	IC UILPropertiesFrom* GetPropertiesFrom() { return m_Properties; }
	IC class UIWorldPropertiesFrom* GetWorldPropertiesFrom() { return m_WorldProperties; }

	// F11 on Level: the viewport over the whole work area, the panels kept where they are
	bool ViewportMaximized() const { return m_ViewportMaximized; }
	void ToggleViewportMaximized();
	// Window > Reset Layout, MCP reset_layout: the Level workspace built again for the window's size class with
	// every panel back; `all` also forgets the splits and grids of Layout
	static void ResetLayout(bool all);
	static LPCSTR LevelWorkspace() { return "XFinedLevel_v2"; }
	// Wide: W >= 1400 and H >= 640 pt; the browser and the log docked under the viewport. Compact: drawers only.
	enum ESizeClass { ClassCompact = 0, ClassWide = 1 };
	static int SizeClass();
	// the scene changed since a level builder last ran: the status bar's Rebuild chip is up (MCP ui_popup show=rebuild)
	static bool RebuildPending();
private:
	UITopBarForm *m_TopBar;
	UIRenderForm* m_Render;
	UIMainMenuForm* m_MainMenu;
	UILeftBarForm* m_LeftBar;
	UILPropertiesFrom* m_Properties;
	class UIWorldPropertiesFrom* m_WorldProperties;
	bool m_ViewportMaximized = false;
private:
	void DrawContextMenu();
	void DrawRenderToolBar(ImVec2 Size);
	void DropAsset(LPCSTR name);
	// the Level tab
	void RegisterLevel();
	void DrawLevel();
	void KeepLevelAlive();
	void DrawLevelStatus();
	void DrawStatusRight(bool busy);
	bool LevelEdit(LPCSTR action, bool run);
};
extern UIMainForm* MainForm;
