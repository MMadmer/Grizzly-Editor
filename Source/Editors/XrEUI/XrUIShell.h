#pragma once
#include "XrUIKit.h"

// The editor's frame (docs/UI_DESIGN.md, section 5): the caption row with its menus and major tabs, the
// active tab's toolbar, the status bar, the work area and the drawers over it. Every asset editor is a
// document - a major tab that owns the whole work area. Level is tab 1 and never closes.
//
// A frame, in this order: BeginFrame; the caption (BeginCaption, the editor's menus, DrawTabStrip,
// EndCaption); DrawToolbar; DrawStatusBar; DrawWorkArea; the editor's modals; EndFrame. A blocking job draws
// DrawBusyFrame instead.
namespace XUI
{
namespace Shell
{
enum CloseMode { IfClean = 0, SaveFirst, Discard };

// What a tab is. Every callback is optional; the shell asks them each frame, so they must be cheap.
struct DocDesc
{
	xr_string id;							// the MCP name: "level", "quest:<rel>", "beh:<rel>", "pda", ...
	xr_string kind;							// "level", "quest", "beh", "pda", "model", "characters", ...
	xr_string path;							// the file it edits; "" for a tool without one
	Icon icon = Icon::File;
	bool closable = true;
	bool padded = true;						// the work area keeps 8 pt around the body
	std::function<xr_string()> label;		// the tab's name
	std::function<xr_string()> tooltip;
	std::function<bool()> dirty;
	std::function<bool()> missing;			// its file is gone
	std::function<bool()> alive;			// false: its owner closed it, the tab goes without a question
	std::function<void()> menus;			// its menus, after Edit
	std::function<void(Toolbar&)> toolbar;
	std::function<void()> body;				// the work area
	std::function<void()> status;			// the centre of the status bar
	std::function<bool(xr_string& err)> save;
	std::function<void()> discard;			// forget the unsaved changes; the tab closes after it
	std::function<bool(int key, int mods)> shortcut;	// ImGuiKey and ImGuiMod_ flags; true when taken
	// the Edit menu on this tab: "undo", "redo", "cut", "copy", "paste", "duplicate", "delete", "find";
	// asked with run = false it answers whether the action is there at all
	std::function<bool(cstr action, bool run)> edit;
	std::function<void()> on_activate;
	std::function<void()> on_deactivate;
	std::function<void()> on_close;			// the tab is gone: free what it holds
	std::function<void()> inactive;			// every frame it is not the active tab, inside the work area
	// what the session's factory is given to reopen it (a project-relative path); unset: the id after "<kind>:"
	std::function<xr_string()> restore;
};

struct DocInfo
{
	xr_string id, kind, label, path;
	Icon icon = Icon::File;
	bool dirty = false;
	bool active = false;
	bool missing = false;
	bool closable = true;
	bool loaded = true;						// false: restored from the session, opened when activated
};

// Opens a tab, or updates the one with that id in place (a restored tab gets its document here).
XREUI_API void Open(const DocDesc& desc, bool activate = true);
// Takes effect at once: the next work area drawn is this tab's.
XREUI_API bool Activate(cstr id);
XREUI_API cstr Active();
XREUI_API bool IsActive(cstr id);
// a tab with its document; one the session restored and nobody opened yet is not (List names it, loaded false)
XREUI_API bool IsOpen(cstr id);
// The UI's close: a tab with unsaved changes asks first.
XREUI_API void RequestClose(cstr id);
// MCP's close: IfClean refuses a dirty tab (err "unsaved"), SaveFirst saves it, Discard drops the changes.
XREUI_API bool Close(cstr id, CloseMode mode, xr_string* err = nullptr);
// A tab's save or its discard without closing it (MCP window_control close, project_close); true when it is clean
XREUI_API bool SaveTab(cstr id, xr_string* err = nullptr);
XREUI_API bool DiscardTab(cstr id);
XREUI_API void List(xr_vector<DocInfo>& out);
XREUI_API bool Info(cstr id, DocInfo& out);
// Ctrl+Tab order: +1 the next tab, -1 the previous one
XREUI_API void Cycle(int step);
// A tab restored from the session opens its document through the factory of its kind when it is activated,
// given what its restore() said (a project-relative path; "" for a tool); false leaves it "File not found".
using Factory = std::function<bool(cstr path)>;
XREUI_API void RegisterFactory(cstr kind, Factory fn);
// The active tab's Edit actions (run = false: is the action there at all) and its own menus, which the caption
// draws after Edit.
// Ctrl+S: the active tab's save (the scene on Level); a failure is a notice
XREUI_API void SaveActive();
XREUI_API bool Edit(cstr action, bool run);
XREUI_API void DrawDocMenus();
// Quit, after the unsaved prompt that names the scene and every dirty document.
XREUI_API void RequestExit();
// Writes the session now: an editor MCP drives in the background may never have drawn a frame to write it in.
XREUI_API void FlushSession();
// The project closes: its session is written, then every tab but Level goes with it and comes back when the project
// opens again. A tab with unsaved changes stays - the caller settles those first.
XREUI_API void CloseProjectTabs();

// ---------------------------------------------------------------------------------------------------
// What the editor plugs into the frame
// ---------------------------------------------------------------------------------------------------
struct FrameHooks
{
	std::function<void(bool busy)> caption;				// the caption row, drawn by the busy frame too
	std::function<void(Toolbar&)> project_toolbar;		// the right end of every toolbar
	std::function<void(bool busy)> status_right;		// rebuild chip, tasks, counts, the MCP dot
	std::function<void(xr_string& last, int& errors, int& warnings)> log_state;
	std::function<void()> busy_card;					// the task card of a blocking job
	std::function<void()> palette;						// Ctrl+Shift+P
	std::function<void()> quit;							// the exit prompt is through
	std::function<void()> level_maximize;				// F11 on Level
	std::function<xr_string()> session_file;			// <project>\temp\session.ltx; "" without a project
	std::function<void(cstr path)> reveal_in_browser;	// a tab's file in the Content Browser
	std::function<bool()> tabs_locked;					// play in editor holds Level
};
XREUI_API FrameHooks& Hooks();

// ---------------------------------------------------------------------------------------------------
// The frame
// ---------------------------------------------------------------------------------------------------
XREUI_API void BeginFrame();
// The caption row (32 pt): a main menu bar. False when it is not drawn; EndCaption() follows a true return.
XREUI_API bool BeginCaption();
XREUI_API void EndCaption();
XREUI_API float CaptionHeight();
// The major tabs from the cursor up to max_x (px); the x where the strip ends is returned
XREUI_API float DrawTabStrip(float max_x, bool locked = false);
// Where the strip's tabs are (px, x0 y0 x1 y1), for the caption's hit test and titlebar_info
XREUI_API void TabRects(xr_vector<ImVec4>& out, xr_vector<xr_string>* ids = nullptr);
XREUI_API void DrawToolbar();
XREUI_API void DrawStatusBar(bool busy = false);
XREUI_API void DrawWorkArea();
XREUI_API void EndFrame();
// A blocking job: the caption, the status bar and a dimmed work area with the task card.
XREUI_API void DrawBusyFrame();
// The rect the work area has this frame (px, x y w h)
XREUI_API ImVec4 WorkRect();

// ---------------------------------------------------------------------------------------------------
// Drawers: the Content Browser and the Output Log over the bottom of any tab, one at a time
// ---------------------------------------------------------------------------------------------------
enum DrawerId { DrawerNone = 0, DrawerContent, DrawerLog };
struct DrawerDesc
{
	cstr label = nullptr;
	Icon icon = Icon::None;
	cstr keys = nullptr;
	std::function<void()> body;
	std::function<bool()> panel_visible;	// its panel is on screen: the drawer shows the panel instead
	std::function<void()> focus_panel;
};
XREUI_API void SetDrawer(int which, const DrawerDesc& desc);
XREUI_API void OpenDrawer(int which);
XREUI_API void ToggleDrawer(int which);
XREUI_API void CloseDrawer();
XREUI_API int OpenedDrawer();

// ---------------------------------------------------------------------------------------------------
// Keys, first taker wins: a text field, the shell, the document, then the scene (ApplyShortCut)
// ---------------------------------------------------------------------------------------------------
// true for a chord the shell owns (Ctrl+Shift+P, Ctrl+Tab, Ctrl+F4, Ctrl+Space, Ctrl+`, Ctrl+S, F11)
XREUI_API bool ShellChord(unsigned vk, bool ctrl, bool shift, bool alt);
// the scene's bindings may act: Level is active, nothing modal is up and no panel that owns its keys has focus
XREUI_API bool SceneKeysAllowed();
// a window that handles its keys itself (the Content Browser, the Output Log)
XREUI_API void RegisterKeyOwner(cstr window_name);

// ---------------------------------------------------------------------------------------------------
// A workspace: a dockspace the Level tab's panels dock into, laid out per size class
// ---------------------------------------------------------------------------------------------------
namespace Dock
{
enum Dir { Left = 0, Right, Up, Down };
// Splits the side of `node` off: size_pt along the split axis, or `ratio` of the node when size_pt is 0.
// `node` becomes the remainder; the side is returned.
XREUI_API unsigned Split(unsigned& node, int dir, float size_pt, float ratio = 0.f);
XREUI_API void Place(cstr window_name, unsigned node);
}
struct Workspace
{
	cstr name = nullptr;					// the dockspace's name, e.g. XFinedLevel_v2
	int size_class = 0;						// a change rebuilds the layout unless it is customized
	std::function<void(unsigned root, int size_class)> build;
	bool* custom = nullptr;					// the user's own arrangement; kept by the caller
	std::function<void()> built;			// after a (re)build
};
// Submits the dockspace over the rest of the current window, or keeps it alive only when `visible` is false.
XREUI_API void WorkspaceDockSpace(const Workspace& ws, bool visible);
XREUI_API void ResetWorkspace(cstr name);
// The canvas of a workspace (the viewport): no tab bar, nothing docks over it.
XREUI_API void NextWindowIsCanvas();
// A floating panel whose saved place is outside the work area is moved back inside it.
XREUI_API void ClampNextFloatingWindow(cstr window_name);
// true when the window was drawn docked and visible last frame
XREUI_API bool WindowShown(cstr window_name);
XREUI_API void FocusWindow(cstr window_name);
// a popup asked for over MCP (ShowPopup) is taken by the owner that draws it; true once
XREUI_API bool TakePopup(cstr name);
// a popup whose name starts with `prefix` is asked for and nobody took it yet (a folded caption opens for "menu:")
XREUI_API bool PopupRequested(cstr prefix);
// A menu of the caption - the editor's own, or the one a document adds in its menus() (Graph): BeginMenu that MCP
// ui_popup menu:<label> puts up the way a click does. EndMenu() follows a true return.
XREUI_API bool BeginCaptionMenu(cstr label, bool enabled = true);
// OpenPopup(str_id) of the current window, put up at `pos` (px) instead of where the mouse happens to be: under the
// button that opens it, which is also where a popup asked for over MCP belongs
XREUI_API void OpenPopupAt(cstr str_id, const ImVec2& pos);
// The popup begun next stands above the item at [min, max] (px): a status bar chip's popover opens over the bar,
// never over the chip. Its edge follows the side of the window the item is on. Before BeginPopup, every frame.
XREUI_API void NextPopupAbove(const ImVec2& item_min, const ImVec2& item_max);

// ---------------------------------------------------------------------------------------------------
// What MCP reads of the frame and asks of it (px of the main viewport, as the last frame left them)
// ---------------------------------------------------------------------------------------------------
struct WindowState
{
	bool exists = false;
	bool shown = false;			// drawn last frame, and the visible tab of its dock node
	bool docked = false;
	bool focused = false;
	ImVec4 rect;				// x, y, w, h
	float scroll_y = 0.f;		// of the window, or of its child that scrolls the most
	float scroll_max_y = 0.f;
};
XREUI_API bool GetWindowState(cstr window_name, WindowState& out);
// top | bottom | <pt>: the window, or its child that scrolls the most, moves at the next frame
XREUI_API bool ScrollWindow(cstr window_name, cstr target);
struct PopupState
{
	xr_string name;				// the ImGui window's name
	xr_string title;			// what its title shows
	xr_string id;				// what follows ### (dlg.<name>), else the title
	bool modal = false;
	ImVec4 rect;
};
// every popup open now, the outermost first
XREUI_API void OpenPopups(xr_vector<PopupState>& out);
// takes the popups down that are only looked at (not the modals) at the next frame, and a ShowPopup not taken yet
XREUI_API void ClosePopups();

// ---------------------------------------------------------------------------------------------------
// Settings read before the ImGui context exists
// ---------------------------------------------------------------------------------------------------
// "Allow panels outside the editor window": platform windows (a restart takes effect)
XREUI_API void SetFloatingPanels(bool allow);
XREUI_API bool FloatingPanels();
// A stored layout of another version is not loaded (the ImGui ini of an older editor).
XREUI_API void SetSkipLayoutOnce(bool skip);
XREUI_API bool SkipLayoutOnce();
}
}
