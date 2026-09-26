#pragma once

// NQ - the graph editor (docs/NQ_ARCHITECTURE.md par. 13.7): every open quest or
// behaviour graph is a major tab of the editor (XUI::Shell) that owns the whole work
// area. The tab's toolbar is on the shell's toolbar row; below it the Graph panel (the
// graph's own settings), the canvas, the Details panel (the selection) and, when asked
// for, a drawer with the problems and the find results. UIQuestGraph is the registry
// (Open/Update/CloseAll) pumped from CLevelMain::OnDrawUI; the documents themselves live
// in NqDocs (XrECore) so MCP and the UI share them.

#include "../../../XrECore/Editor/Nq/NqDoc.h"
#include "../../../XrECore/Editor/Nq/NqProjectIndex.h"

class NqCanvas;
class NqInspector;

class UIQuestGraphWindow : public XrUI
{
public:
	explicit		UIQuestGraphWindow	(NqDoc* doc);
	virtual			~UIQuestGraphWindow	();
	virtual void	Draw				() { DrawBody(); }
	// the tab's work area; the shell draws it while the tab is active
	void			DrawBody			();
	// the tab's part of the shell's toolbar row
	void			DrawToolbar			(XUI::Toolbar& tb);
	// the centre of the status bar while the tab is active
	void			DrawStatus			();
	// the Graph menu of the caption while the tab is active
	void			DrawMenu			();
	// the Edit menu's actions on this graph
	bool			EditAction			(LPCSTR action, bool run);
	// keys the shell hands the tab (F3, Ctrl+F, ...); true when taken
	bool			Shortcut			(int key, int mods);

	NqDoc*			Doc					()	{ return m_Doc; }
	NqCanvas*		Canvas				()	{ return m_Canvas; }
	NqInspector*	Inspector			()	{ return m_Inspector; }
	// its major tab comes up at once: the next work area drawn is this graph's
	void			Focus				();
	void			Save				();
	void			McpFind				(LPCSTR raw, xr_string& out);
	void			McpBookmarks		(LPCSTR raw, xr_string& out);
	void			McpHistory			(LPCSTR raw, xr_string& out);
	void			McpMinimap			(LPCSTR raw, xr_string& out);

	// ---- panes, for the toolbar, the Graph menu and MCP quest_view --------------------
	enum EDrawer { DrawerNone = 0, DrawerProblems, DrawerFind };
	// The Graph panel is asked for (the toggle); it is drawn as a rail when the canvas would get
	// less than 400 pt, and the rail opens it over the canvas.
	void			SetGraphPanel		(bool on);
	bool			GraphPanel			() const	{ return m_ShowGraph; }
	// "open", "rail", "overlay" or "hidden", as the last frame drew it
	LPCSTR			GraphPanelState		() const;
	void			SetGraphWidth		(float pt);
	float			GraphWidth			() const;
	void			SetDetails			(bool on)	{ m_ShowDetails = on; }
	bool			Details				() const	{ return m_ShowDetails; }
	void			SetDetailsWidth		(float pt);
	float			DetailsWidth		() const;
	void			SetShowKeys			(bool on)	{ m_ShowKeys = on; }
	bool			ShowKeys			() const	{ return m_ShowKeys; }
	void			SetDrawer			(int which);
	int				Drawer				() const	{ return m_Drawer; }
	LPCSTR			DrawerName			() const;
	// the panes as the last frame drew them, pt of the main viewport
	void			AppendPanes			(xr_string& out) const;
	// whether `where` (quest_view scroll=) has a pane on screen to scroll; `why` says what is missing
	bool			CanScroll			(LPCSTR where, xr_string& why) const;
	// quest_view answers from the last frame drawn: a change it asked for owes `n` frames (a
	// scroll lands a frame after it is asked), and the answer is pending until they are drawn
	void			OweFrames			(int n)		{ m_ViewFrames = _max(m_ViewFrames, n); }
	bool			ViewPending			() const	{ return m_ViewFrames > 0; }
	// the tab was drawn in the frame being built or the one before
	bool			DrawnLately			() const;

private:
	struct SFindResult
	{
		xr_string	node;
		xr_string	kind;
		xr_string	title;
		xr_string	match;
	};

	NqDoc*			m_Doc;
	NqCanvas*		m_Canvas;
	NqInspector*	m_Inspector;
	bool			m_ShowGraph;
	bool			m_ShowDetails;
	bool			m_ShowKeys;			// the shortcut card over the canvas
	bool			m_GraphOverlay;		// the rail's panel, open over the canvas
	bool			m_Rail;				// last frame: the Graph panel was a rail
	bool			m_PendingRename;	// F2 while the Details panel was hidden
	int				m_Drawer;
	// the panes as the last frame drew them (x y w h, px)
	ImVec4			m_PaneGraph, m_PaneCanvas, m_PaneDetails, m_PaneDrawer;
	int				m_ViewFrames;		// frames a quest_view change still owes
	int				m_DrawnFrame;		// ImGui frame the tab was last drawn in

	// the file changed on disk behind the document: polled, not read per frame
	bool			m_External;
	double			m_ExternalAt;

	xr_string		m_Message;			// last save/reload result
	XUI::Tone		m_MessageTone;
	bool			m_FocusFind;
	char			m_Find[256];
	int				m_FindIndex;
	u32				m_FindRevision;
	u32				m_FindCatalogGeneration;
	xr_vector<SFindResult> m_FindResults;
	bool			m_ProjectFindOpen;
	bool			m_FocusProjectFind;
	char			m_ProjectFind[256];
	xr_vector<NqProjectIndex::SFindResult>	m_ProjectFindResults;
	xr_vector<NqProjectIndex::SDiagnostic>	m_ProjectFindDiagnostics;
	xr_string		m_ProjectFindError;
	bool			m_ProjectFindComplete;
	u32			m_ProjectFindGeneration;
	u64			m_ProjectFindFingerprint;

	void			DrawRail			(const ImVec2& at, float w, float h);
	void			DrawKeysCard		();		// what the keyboard does here, read off HandleKeys
	void			DrawDrawer			(const ImVec2& at, float w, float h);
	void			DrawProblemsTab		();
	void			DrawFindTab			();
	void			DrawProjectFind		();
	void			DrawExternalBanner	();
	void			PollExternal		();
	void			Reload				();		// asks first when there are unsaved edits
	void			Overwrite			();		// saves over a file that changed on disk, after asking
	void			Validate			();
	void			OpenFind			();
	void			OpenProjectFind		();
	void			RefreshProjectFind	();
	bool			NavigateProjectFind	(int index);
	void			RefreshFind			(bool force = false);
	bool			NavigateFind		(int index);
	void			SetFindQuery		(LPCSTR query);
	void			AppendFindState		(xr_string& out, int limit);
	void			AppendViewState		(xr_string& out);
	void			SetMessage			(LPCSTR text, XUI::Tone tone = XUI::Tone::Muted);
	LPCSTR			Noun				() const;	// "quest" / "graph"
	// the graph is big enough for a minimap (NqCanvas::MinimapMinNodes); the tip says why not
	bool			MinimapAvailable	() const;
	xr_string		MinimapTip			() const;
};

namespace UIQuestGraph
{
	// opens (or focuses) the tab for a project-relative or absolute path;
	// false + err when the file cannot be read (err = 0 -> message box)
	bool			Open				(LPCSTR path, xr_string* err);
	// the tab's id: quest:<rel> or beh:<rel>, rel project-relative with forward slashes
	xr_string		DocId				(LPCSTR abs_path, bool behaviour);
	// the session reopens graphs through these (XUI::Shell::RegisterFactory)
	void			RegisterFactories	();
	// drops the windows whose document was closed elsewhere; call from CLevelMain::OnDrawUI
	void			Update				();
	// closes every tab (dirty ones ask unless `discard`)
	void			CloseAll			(bool discard);
	int				Count				();
	UIQuestGraphWindow* Find			(LPCSTR path);
	// MCP quest_view: {"path","frame":"all"|"<node>","zoom_level":n,"center":{x,y}}
	void			McpClose			(LPCSTR raw, xr_string& out);
	void			McpView				(LPCSTR raw, xr_string& out);
	// every graph edit the canvas can do, for callers without a mouse
	void			McpEdit				(LPCSTR raw, xr_string& out);
	void			McpFind				(LPCSTR raw, xr_string& out);
	void			McpBookmarks		(LPCSTR raw, xr_string& out);
	void			McpHistory			(LPCSTR raw, xr_string& out);
	void			McpMinimap			(LPCSTR raw, xr_string& out);
	// forget every tab when the project changes
	void			OnProjectChanged	();
}
