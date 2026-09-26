#pragma once

// NQ - the graph canvas of a quest or behaviour document (docs/NQ_ARCHITECTURE.md
// par. 13.8). Own ImDrawList implementation: infinite world, right-button pan, ten
// fixed zoom levels, top-down flow (input on the top edge, one output pin per catalog
// pin on the bottom edge; an edge that goes back up is routed around the nodes). A node
// is a card sized by its content (NqLayout::Box): a header, up to three lines (its
// conditions, then "key: value"), up to four action rows and its pin names.
// Every edit goes through NqDoc so undo/redo and MCP see the same operations.
//
// World units are pt at zoom 1: the screen scale is the zoom level times the UI scale,
// applied once (Scale), so a node is as readable at 150% as at 100%.

#include "../../../XrECore/Editor/Nq/NqDoc.h"
#include "../../../XrECore/Editor/Nq/NqCatalog.h"
#include "../../../XrECore/Editor/Nq/NqLayout.h"

// How the graph names and colours a kind - shared by the canvas and the Details panel,
// and aware of the document: a behaviour graph never says "quest".
namespace NqLook
{
	XUI::Icon		KindIcon		(LPCSTR kind);
	// the family colour of a kind (a theme token), `alpha` times its own
	ImU32			FamilyColor		(LPCSTR kind, float alpha = 1.f);
	xr_string		KindTitle		(LPCSTR kind, bool behaviour);
	xr_string		KindDesc		(LPCSTR kind, bool behaviour);
	// "quest" as the document says it: "quest" or "graph"
	LPCSTR			DocNoun			(bool behaviour);
}

class NqCanvas
{
public:
	struct SViewState
	{
		float				cx, cy;
		int					zoom;
		xr_vector<xr_string>	selection;
		xr_string				slot;
	};

	explicit		NqCanvas		(NqDoc* doc);

	// draws the canvas into the current window at the cursor, `size` pixels
	void			Draw			(const ImVec2& size);

	// view control (also used by MCP quest_view)
	static int		ZoomLevels		();
	static float	ZoomOf			(int idx);
	// the lowest level Frame All picks: titles stay readable (12 px at 100%)
	static int		FrameMinLevel	();
	void			SetZoom			(int idx);				// clamps
	void			ZoomBy			(int delta, const ImVec2* anchor_screen = 0);
	void			FrameAll		();
	void			FrameSelection	();
	void			FrameNode		(LPCSTR id);
	// an explicit centre wins over a pending frame request (the coordinates are
	// clamped: quest_view takes them straight from the request line)
	void			Center			(float wx, float wy);
	// all/selection framing needs the canvas rect and waits for Draw when needed;
	// a single node can be centred immediately
	void			RequestFrameAll	();
	void			RequestFrameSel	();
	void			RequestFrameNode(LPCSTR id);
	// the rect changes before the next Draw (a pane shown or hidden): framing waits for that Draw
	void			Remeasure		()	{ m_DrawnFrame = -1; }
	bool			FramePending	() const { return m_WantFrameAll || m_WantFrameSel; }
	void			BeginNavigationBatch();
	void			EndNavigationBatch();
	bool			HistoryBack		();
	bool			HistoryForward	();
	void			ClearHistory	();
	SViewState		CurrentView		() const;
	const xr_vector<SViewState>& BackHistory() const	{ return m_BackHistory; }
	const xr_vector<SViewState>& ForwardHistory() const { return m_ForwardHistory; }

	// Bookmarks are transient editor state and never enter the quest asset.
	bool			SetBookmark		(LPCSTR id, bool enabled);
	bool			ToggleBookmark	(LPCSTR id);
	void			ClearBookmarks	();
	bool			JumpBookmark	(int delta);
	bool			IsBookmarked	(LPCSTR id) const;
	const xr_vector<xr_string>& Bookmarks() const		{ return m_Bookmarks; }
	// Ctrl+B: bookmarks the selection, or takes the bookmarks off when all of it has one
	void			ToggleSelectedBookmarks();

	// The minimap is asked for (the toggle) and shown only when the graph is big enough to
	// need one: 12 nodes and more, and a canvas that has room for it.
	void			SetMinimapVisible(bool visible)
	{
		m_ShowMinimap = visible;
		if (!visible) m_MinimapHovered = m_MinimapPanning = false;
	}
	bool			MinimapVisible	() const					{ return m_ShowMinimap; }
	bool			MinimapShown	() const					{ return m_MinimapShown; }
	static int		MinimapMinNodes	()							{ return 12; }
	// Nodes wholly beyond an edge of the view on the last frame drawn: 0 above, 1 below, 2 left,
	// 3 right. Without a minimap the canvas shows them at that edge (DrawBeyond).
	int				Beyond			(int side) const			{ return side >= 0 && side < 4 ? m_Beyond[side] : 0; }
	bool			GraphBounds		(float& min_x, float& min_y, float& max_x, float& max_y) const;
	bool			ViewportBounds	(float& min_x, float& min_y, float& max_x, float& max_y) const;

	// the inspector wants the id field focused (F2 / rename)
	bool			TakeRenameRequest()						{ bool r = m_WantRename; m_WantRename = false; return r; }
	// the inspector wants the action section focused (a double click on an action row)
	bool			TakeFocusAction	()						{ bool r = m_WantFocusAction; m_WantFocusAction = false; return r; }
	// the add-action picker of a node, put up at `screen` (the Details panel's Add button)
	void			RequestAddAction(LPCSTR node, LPCSTR slot, const ImVec2& screen);

	// clipboard: selected nodes as a Lua fragment
	void			CopySelection	();
	void			PasteClipboard	();
	void			DeleteSelection	();
	void			SelectAll		();
	// Replaces the selection with exactly these node ids (empty = clear). The one
	// way in from outside the canvas; the piecewise helpers below are its internals.
	void			SelectNodes		(const xr_vector<xr_string>& ids);
	void			DuplicateSelection();
	// Q: wires the one selected node to the nearest free pin of another one
	void			ConnectNearest	();
	// Alt+D: the node stays put and the chain closes over it (BlueprintAssist's Alt+D)
	void			BypassSelected	();
	// the single gate every new edge passes: false + `why` when the validator
	// would reject it on the spot (E007)
	bool			CanLink			(LPCSTR from, LPCSTR to, xr_string& why) const;
	LPCSTR			Status			() const { return m_Status.c_str(); }	// last refusal / result

	// world <-> screen of the last Draw
	ImVec2			ToScreen		(const ImVec2& w) const;
	ImVec2			ToWorld			(const ImVec2& s) const;
	// the zoom level's factor (1.0 = 1:1), as MCP reports it
	float			Zoom			() const				{ return ZoomOf(m_Doc->zoom_idx); }
	// screen px per world unit: the zoom level times the UI scale
	float			Scale			() const;
	// the px size node titles are drawn at on this level
	float			TitlePx			() const;
	bool			Hovered			() const				{ return m_Hovered; }

private:
	struct SNodeGeom
	{
		ImVec2	pos, size;			// world, top-left + size
		bool	trigger;
		NqLayout::SBox box;			// where the header, lines, actions and pin names sit
		xr_vector<NqLayout::SLine> lines;
		xr_vector<xr_string> pins;	// output pins in draw order
		int		declared;			// pins[0..declared) come from the catalog, the rest only from the file
	};
	struct SLinkGeom
	{
		int			from;			// node index
		xr_string	pin;
		int			to;
		ImVec2		p0, p1, p2, p3;	// screen: the bezier of an edge that runs down
		// an edge that goes back up is routed around the nodes instead: this polyline,
		// p0 to p3 (screen); empty for a bezier
		xr_vector<ImVec2> path;
		ImVec2		bmin, bmax;		// screen bounds of either shape
	};
	// the vertical run of a routed edge (world): the next one keeps clear of it. Its pins and the
	// side each end slants to: a second edge through the same pin to that side slants steeper.
	struct SLane
	{
		float		x, top, bottom;
		ImVec2		from, to;
		float		side_from, side_to;		// -1 left, +1 right
	};
	// an action row under the cursor
	struct SActionHit
	{
		int			node = -1;
		int			row = -1;		// the row of the node, top to bottom
		xr_string	slot;			// "enter" / "exit"
		int			index = -1;		// in that slot
		bool		more = false;	// the "+N more" row
	};

	NqDoc*			m_Doc;
	ImVec2			m_Origin;		// screen top-left of the canvas rect
	ImVec2			m_Size;
	bool			m_Hovered;
	bool			m_ShowMinimap;
	bool			m_MinimapShown;
	bool			m_MinimapHovered;
	bool			m_MinimapPanning;
	ImVec2			m_MinimapMin, m_MinimapMax;
	ImVec2			m_MinimapWorldMin, m_MinimapWorldMax;
	float			m_MinimapScale;
	// what lies past each edge of the view (DrawBeyond): the count, the nearest node, the pill under the cursor
	int				m_Beyond[4];
	int				m_BeyondNear[4];
	int				m_BeyondHot;

	// interaction
	bool			m_Panning;
	bool			m_RmbMoved;
	ImVec2			m_RmbDown;
	bool			m_Dragging;
	bool			m_DragMoved;
	ImVec2			m_DragStartMouse;
	xr_vector<std::pair<xr_string, ImVec2> > m_DragStart;	// node id -> start pos (world)
	bool			m_Marquee;
	ImVec2			m_MarqueeFrom;	// screen
	bool			m_Linking;
	xr_string		m_LinkFrom, m_LinkPin;
	xr_string		m_Status;		// last refusal / result of a link gesture (drawn briefly)
	double			m_StatusAt;
	int				m_HotNode;		// node whose pin the cursor is on (-1 none)
	int				m_HotPin;		// output pin index, or -1 for the input pin
	int				m_HotLink;		// link under the cursor (-1 none) - lit, Alt+LMB or D removes it
	SActionHit		m_HotAction;	// action row under the cursor
	// pending "create node and connect" after dropping a link on empty space
	bool			m_PendingLink;
	ImVec2			m_MenuWorld;	// world position for node creation from menus
	xr_string		m_CtxNode;		// node under the context menu
	xr_string		m_CtxLinkFrom, m_CtxLinkPin, m_CtxLinkTo;
	char			m_Filter[64];	// add-node menu filter
	int				m_FilterSel;	// highlighted row of the filtered list (keyboard)
	// an action being dragged: a press only nominates it, the move starts once the mouse
	// has travelled (ImGui's own drag threshold), so a click stays a click
	xr_string		m_ActDragNode, m_ActDragSlot;
	int				m_ActDragIndex;
	bool			m_ActDragging;
	xr_string		m_AddSlot;		// the slot the add-action picker adds to
	bool			m_OpenAddAction;
	bool			m_AddActionAt;	// the picker goes up at m_AddActionPos, not at the mouse
	ImVec2			m_AddActionPos;
	bool			m_WantFrameAll, m_WantFrameSel, m_WantRename, m_WantFocusAction;
	int				m_WantZoom;		// zoom asked for explicitly (-1 = none); applied after framing
	xr_vector<SViewState> m_BackHistory, m_ForwardHistory;
	xr_vector<xr_string> m_Bookmarks;
	xr_flat_hash_map<xr_string, u8> m_BookmarkLookup;
	bool			m_ReplayingHistory;
	bool			m_HistoryReady;
	int				m_HistoryBatchDepth;
	bool			m_HistoryBatchRemembered;
	double			m_LastWheelHistory;
	bool			m_WheelZooming;
	bool			m_MiddlePanning;
	// the hover tooltip: worked out during the frame, shown on the next one once the same
	// thing has been under the cursor for the kit's delay
	xr_string		m_TipText, m_TipNext;
	double			m_TipSince;
	void			CancelFraming	()	{ m_WantFrameAll = m_WantFrameSel = false; }
	bool			Measured		() const { return m_Size.x > 32.f && m_Size.y > 32.f; }
	// the rect of the frame being built or the one before: a tab behind another one keeps the size of
	// the last frame that drew it, which a resize or a UI scale since then made wrong
	bool			MeasuredNow		() const { return Measured() && m_DrawnFrame >= ImGui::GetFrameCount() - 1; }
	int				m_DrawnFrame;
	u32				m_ReachRevision;
	xr_vector<bool>	m_Reachable;

	// render caches
	xr_vector<SNodeGeom> m_Geom;
	xr_vector<SLinkGeom> m_Links;
	u32				m_GeomRevision;
	u32				m_GeomCatalogGeneration;
	u32				m_LinkRevision;
	u32				m_LinkCatalogGeneration;
	ImVec2				m_LinkOrigin;
	ImVec2				m_LinkSize;
	float				m_LinkViewX;
	float				m_LinkViewY;
	float				m_LinkScale;

	void			BuildGeometry	();
	void			BuildLinks		();
	// the route of an edge from pin `p0` up to input `p3` (world): around the nodes, clear of `lanes`
	void			RouteBack		(const ImVec2& p0, const ImVec2& p3, xr_vector<SLane>& lanes, xr_vector<ImVec2>& out) const;
	const SNodeGeom* GeomOf			(LPCSTR id, int* index = 0) const;
	ImVec2			InputPin		(const SNodeGeom& g) const;			// world
	ImVec2			OutputPin		(const SNodeGeom& g, int pin) const;	// world
	// the level of detail: 2 everything, 1 titles only, 0 blocks
	int				Lod				() const;
	// how far from a pin the mouse may be and still grab it (screen pixels)
	float			PinGrab			() const;
	bool			NearPin			(const ImVec2& screen, const ImVec2& c, float inward, float& d2) const;
	int				HitNode			(const ImVec2& screen) const;			// index or -1
	// nearest node within the grab radius of its box (`except` skipped), or -1
	int				HitNodeNear		(const ImVec2& screen, int except = -1) const;
	// nearest output pin of one node / of every node; `d2` gets its distance
	int				HitPin			(int node, const ImVec2& screen, float* d2 = 0) const;
	bool			HitAnyPin		(const ImVec2& screen, int& node, int& pin) const;
	bool			HitAction		(int node, const ImVec2& screen, SActionHit& hit) const;
	bool			HitHeader		(int node, const ImVec2& screen) const;
	// where a dragged action would land: the slot and the index it is inserted at
	bool			DropTarget		(const ImVec2& screen, int& node, xr_string& slot, int& index, float* line_y = 0) const;
	int				HitLink			(const ImVec2& screen) const;			// link index or -1
	bool			MenuOpen		() const;								// a popup of this canvas is up
	void			UpdateHot		();
	void			DrawGrid		(ImDrawList* dl);
	void			DrawLinks		(ImDrawList* dl);
	void			DrawNode		(ImDrawList* dl, int index);
	void			DrawActions		(ImDrawList* dl, int index, const ImVec2& a, float alpha);
	void			DrawMarquee		(ImDrawList* dl);
	void			DrawLinking		(ImDrawList* dl);
	void			DrawActionDrag	(ImDrawList* dl);
	void			DrawMinimap		(ImDrawList* dl);
	void			DrawBeyond		(ImDrawList* dl);
	// pans just far enough to bring the nearest node beyond that edge into view
	void			RevealBeyond	(int side);
	void			DrawStatus		(ImDrawList* dl);
	void			HandleInput		();
	void			HandleKeys		();
	void			OpenContextMenus();
	void			DrawContextMenus();
	void			DrawAddNodeMenu	(LPCSTR popup);
	// filter box + list of the search popups; the chosen kind, or 0
	const NqCatalog::SKind* PickKind(LPCSTR hint, u32 use_mask);
	void			InsertNodes		(xr_vector<SNqNode>& in, const ImVec2& place, bool anchor);
	void			CreateNode		(const NqCatalog::SKind& k, const ImVec2& world, bool connect_pending);
	void			MoveAction		(const xr_string& from_node, const xr_string& from_slot, int from,
									 const xr_string& to_node, const xr_string& to_slot, int at);
	void			SetStatus		(LPCSTR text);
	void			DeleteHotLink	();										// Alt+LMB and D share this
	void			SelectOnly		(LPCSTR id);
	bool			IsSelected		(LPCSTR id) const;
	void			ToggleSelected	(LPCSTR id);
	void			EndDrag			();
	void			EndLink			();
	void			EndMarquee		();
	void			EnsureReachable	();
	xr_string		NodeTitle		(const SNqNode& n) const;
	xr_string		NodeSummary		(const SNqNode& n) const;
	int				Severity		(LPCSTR node_id) const;		// 0 none, 1 warning, 2 error
	xr_string		ProblemText		(LPCSTR node_id) const;
	xr_string		ActionText		(const SNqAction& a, LPCSTR slot, int index) const;
	void			FrameRect		(const ImVec2& wmin, const ImVec2& wmax);
	// what Frame All has to show (world): the node boxes and the edges routed around them; `only`
	// limits it to those nodes and the edges between them. False for nothing to show.
	bool			Extent			(const xr_vector<bool>* only, ImVec2& mn, ImVec2& mx);
	void			RememberView	();
	void			ApplyView		(const SViewState& state);
	void			PruneTransient	();
	ImVec2			MinimapToWorld	(const ImVec2& screen) const;
	// the action at a row of a node, top to bottom: on_enter, then on_exit
	bool			ActionAtRow		(const SNqNode& n, int row, xr_string& slot, int& index) const;
	bool			Behaviour		() const { return m_Doc && m_Doc->quest.IsBehaviour(); }
};
