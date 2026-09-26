#pragma once

// Unreal-style dockable asset browser.
//
// It does not enumerate anything itself: every asset kind is already registered
// in UIChooseForm's event table (see FillChooseEvents), which supplies both a
// fill function and a thumbnail adapter. This panel just renders that data as a
// folder tree plus a thumbnail grid, and can spawn what it shows into the scene.

// every write goes through the file-op layer, whose report type is part of the
// paste signature below
#include "../../../XrECore/Editor/EditorFileOps.h"

// payload id for viewport drag&drop
#define CB_DND_PAYLOAD "XRAY_ASSET"

class UIContentBrowser : public XrUI
{
public:
					UIContentBrowser	();
	virtual			~UIContentBrowser	();
	virtual void	Draw				();
	// Everything under the panel's title: the panel draws it on the Level tab, the Content Drawer over any tab.
	// One host a frame - while the drawer is open the panel only says so.
	void			DrawBody			();

	// the file watcher, every frame whatever shows the browser
	static void		Tick				();
	// the panel on the Level tab, when it is open
	static void		DrawPanel			();
	// the drawer's body: the same browser, created for it when the panel never was
	static void		DrawDrawer			();
	// the panel, focused - its tab raised when another one covers it - while Level draws it; the Content Drawer
	// otherwise
	static void		MakeVisible			();
	// Level draws the panel - docked in the strip (behind another tab too) or floating - so it is the host, and
	// a drawer over it is not needed
	static bool		PanelLive			();
	// where the browser is seen now: "panel", "drawer" or "none" (a panel raised by MakeVisible counts at once)
	static LPCSTR	VisibleHost			();
	static void		Show				();
	static void		Close				();
	static IC bool	IsOpen				() { return Form && Form->bOpen; }
	static LPCSTR	PanelName			() { return "Content Browser"; }

	// The viewport drop hands over the browser's own name for the asset; what
	// placement needs is a library reference. Translates per the ACTIVE source:
	// an Objects entry is already a ref, a game path resolves through
	// ResolvePlaceable, a project .object strips its rawdata prefix. False =
	// not placeable (textures, sounds, meshes without an .object, ...).
	static bool		ResolveDropRef		(LPCSTR name, string_path& ref);
	// a droppable engine model (.ogf / a library Visuals entry): the path the
	// raw-visual placement takes; false when the entry is not a model
	static bool		ResolveDropVisual	(LPCSTR name, string_path& path);

	// asset currently held by an in-flight drag, empty when nothing is dragged
	static LPCSTR	DraggedAsset		();
	static u32		DraggedCategory		();

	// The model preview window registers itself here instead of being referenced
	// directly, so the browser does not depend on it existing. Without a handler
	// a double click on a model just reports that no viewer is installed.
	typedef void (*TShowVisual)		(LPCSTR visual_name);
	typedef void (*TShowVisualMem)	(LPCSTR title, const void* data, u32 size);
	// `object_from_memory` takes the editor's .object format rather than an
	// engine .ogf - the project source hands over raw file bytes for both
	static void		SetVisualPreview	(TShowVisual by_name, TShowVisualMem from_memory,
										 TShowVisualMem object_from_memory);

	// MCP: reveal an asset in the browser - switches source and category when
	// needed, navigates to the folder holding it, selects it and optionally
	// opens its viewer (the double-click action). Returns false + a reason when
	// the asset is not in the current listing.
	//   source: -1 keeps the current one, else 0 project / 1 editor / 2 game
	static bool		RevealAsset			(LPCSTR name, int source, bool open_viewer, xr_string& err);
	// MCP: what is selected right now, and in which source/folder
	static void		GetSelection		(int& source, xr_string& folder, xr_vector<xr_string>& sel);
	// The icon the browser draws for an entry - one of the editor's own asset kinds
	// (EditorAssetIcons::Name) - or "" for any other file: what an agent reads instead of the tile.
	static LPCSTR	IconName			(LPCSTR entry);
	// The vector glyph a tile draws while it has no picture (XUI::IconName: "folder", "file_texture",
	// "quest", ...); a folder is named with a trailing '\', as GetSelection lists it.
	static LPCSTR	Glyph				(LPCSTR entry);
	// MCP: the tile size - a tile's side in pt, 48..192 (EditorUiContract), kept in the layout. The docked panel
	// and the drawer keep one each; these read and set the one of the host MakeVisible picks. A size asks for tiles.
	// TileSize is the side the tiles are drawn at: the size picked, or while none is (TileFit) the fitted one.
	static float	TileSize			();
	static bool		SetTileSize			(float size, xr_string& err);
	// MCP: whether the host's tiles fit its height (no size picked yet): the largest side up to the host's default
	// (panel 64, drawer 96) at which the grid shows two rows, 48 at least. Off keeps the side shown as picked.
	static bool		TileFit				();
	static void		SetTileFit			(bool on);
	// MCP: whether the toolbar's Add works here - only in the open project's own source
	static bool		CanAdd				();
	// MCP: how the grid shows its entries, "tiles" or "list" (one line each, in columns), per host like the tile
	// size: the docked panel starts as a list - its strip is too short for two rows of tiles - the drawer as tiles
	static LPCSTR	ViewMode			();
	static bool		SetViewMode			(LPCSTR mode, xr_string& err);
	// MCP: the search field; the grid lists the folder shown and everything below it that matches
	static void		SetSearch			(LPCSTR text);
	static LPCSTR	Search				();
	// MCP: the Sources pane (source, favorites, folder tree). `on` is the author's choice; it is
	// shown only while the browser is at least 480 pt wide.
	static void		SetSourcesPane		(bool on);
	static void		GetSourcesPane		(bool& on, bool& shown);
	// Show in Explorer for a project entry; `dry_run` only answers the path it would select
	static bool		RevealInExplorer	(LPCSTR entry, bool dry_run, xr_string& path, xr_string& err);
	// MCP: apply file-manager selection semantics to an exact entry in the
	// complete current view, including rows outside the virtualized viewport.
	static bool		ModifySelection		(LPCSTR action, LPCSTR name, xr_string& err);
	// the inline rename box, when one is open: the entry, the text it holds and how
	// much of it is selected
	static bool		GetRename			(xr_string& entry, xr_string& text, int& select);
	// MCP: the Create menu, without the menu. `kind` is quest / behaviour /
	// character (a folder has content_mkdir); the asset lands in the open folder,
	// under the same first-free name the menu would pick, and `made` comes back
	// project-relative. Nothing is renamed inline - that is the menu's own
	// flourish and a caller renames through content_rename.
	static bool		CreateAsset			(LPCSTR kind, xr_string& made, xr_string& err);
	// MCP: copy items - or a whole folder, subfolders included - out of a
	// read-only source into the project. Unlike the menu this never prompts;
	// `overwrite` decides. `names` is a ';'-separated list and may be empty when
	// `folder` is given. source/category of -1 keep whatever the browser is on.
	// `dst` empty mirrors each asset into the layout fs.ltx implies; give it a
	// project-relative folder and it goes through the browser's own clipboard
	// and paste instead, i.e. exactly where you asked for it.
	static bool		McpCopyToProject	(LPCSTR names, LPCSTR folder, LPCSTR dst,
										 int source, int category,
										 bool overwrite, int& files, xr_string& err);

	struct SLocation
	{
		int							source;
		int							category;
		xr_string					folder;

		SLocation() : source(0), category(0) {}
		SLocation(int src, int cat, LPCSTR path)
			: source(src), category(cat), folder(path ? path : "") {}
	};

	// History is returned oldest-to-newest; the last entry is the next one used.
	// Source/category -1 keep the current value; folders are source-relative.
	static void		GetHistory			(SLocation& current, xr_vector<SLocation>& back,
										 xr_vector<SLocation>& forward);
	static bool		NavigateToFolder	(int source, int category, LPCSTR folder, xr_string& err);
	static bool		NavigateBack		(xr_string& err);
	static bool		NavigateForward		(xr_string& err);
	static bool		NavigateUp			(xr_string& err);
	static bool		NavigateHome		(xr_string& err);
	// lists the source again now (View > Refresh, F5): what the file watcher does after a quiet moment
	static void		RefreshNow			();

	// Favorites live for the editor process and are never written to a project.
	static void		GetFavorites		(xr_vector<SLocation>& out);
	static bool		AddFavorite		(SLocation location, xr_string& err);
	static bool		RemoveFavorite		(int index, xr_string& err);
	static bool		NavigateFavorite	(int index, xr_string& err);

	// asset categories, shared with the MCP asset commands
	static int		CategoryCount		();
	static LPCSTR	CategoryName		(int index);		// caption, e.g. "Objects"
	static u32		CategoryId			(int index);		// EChooseMode id
	static int		FindCategory		(LPCSTR name);		// by caption, case-insensitive; -1 = unknown
	// Levels have no EChooseMode - they are scenes, not library assets - so the
	// callers that enumerate categories have to special-case them
	static bool		IsLevelsCategory	(int index);

private:
	struct SFolder
	{
		xr_string					name;		// leaf name
		xr_string					path;		// full path, '\\'-separated
		xr_vector<SFolder>			children;
		xr_vector<int>				items;		// indices into m_Items
	};

	static UIContentBrowser*		Form;
	static xr_vector<SLocation>	s_Favorites;
	static xr_string				s_FavoriteProject;
	static xr_string				s_FavoriteGameRoot;

	// thumbnail cache — the choose-event adapters allocate a fresh D3D texture
	// per call, far too costly for a grid, so results are kept keyed by name
	struct SThumb
	{
		ImTextureID					tex;
		u32							last_used;
		float						aspect;		// width / height: a picture keeps its shape in the square
		// An image's picture is made for the size it is drawn at (ImGui samples one level): `side` is the long edge
		// it was asked for - 0 for a model render or a library thumbnail, which come in one size - `edge` the long
		// edge it got, `full` when that is the file's own size
		u32							side;
		u32							edge;
		bool						full;
		// what the tile says of an image: its transparency (STextureTile::alpha) and the file's own size
		u8							alpha;
		u32							src_w;
		u32							src_h;
									SThumb() : tex(0), last_used(0), aspect(1.f), side(0), edge(0), full(false),
										alpha(0), src_w(0), src_h(0) {}
	};
	using ThumbMap = xr_flat_hash_map<shared_str, SThumb>;
	using ThumbMapIt = ThumbMap::iterator;
	ThumbMap						m_Thumbs;
	u32								m_Tick;

	ChooseItemVec					m_Items;
	// Project source only: the folders that exist on disk. The tree is otherwise
	// derived from item names, which cannot describe an EMPTY folder - and a
	// freshly created one is exactly that.
	xr_vector<xr_string>			m_Dirs;
	SFolder							m_Root;
	xr_string						m_CurFolder;
	xr_vector<SLocation>			m_BackHistory;
	xr_vector<SLocation>			m_ForwardHistory;
	// what the grid drew last frame, so entering a folder can start at its top
	xr_string						m_ShownFolder;
	u32								m_Category;
	// UE-style split: 0 = project "Content" folder (disk scan), 1 = shared
	// "Editor Content" (UIChooseForm event table), 2 = "DARF Content" (the
	// linked game install, STRICTLY READ ONLY - see IsReadOnlySource)
	int								m_Source;
	bool							m_NeedRefresh;
	// A tile's side in pt (the square; its caption goes below it), one per host: the docked panel is a short
	// strip in the Wide layout and fits smaller tiles than the drawer. m_Host is the one being drawn. 0 = no size
	// picked: the tiles fit the host's height (ShownTile), measured at its last frame (m_GridHeight, pt).
	enum { kHostDrawer = 0, kHostPanel = 1 };
	float							m_TileSize[2];
	float							m_GridHeight[2];
	float			ShownTile			(int host) const;
	// the list instead of the tiles, per host as well
	bool							m_ListView[2];
	int								m_Host;
	// the last frame the panel was drawn; whether it was drawn at the last frame Level was up (Tick); frames a
	// raised panel counts as seen before its tab has come to the front
	int								m_PanelFrame;
	bool							m_PanelLive;
	int								m_RaiseFrames;
	// the host MakeVisible and the tile size go to: the panel while it is live, the drawer otherwise
	static int		TargetHost			();
	// the view settings live in the layout, which is read with the .ini at the first frame
	bool							m_ViewLoaded;
	// the Sources pane: the author's choice, whether the width has room for it (480 pt), and
	// whether it showed last frame
	bool							m_SourcesPane;
	bool							m_SourcesFit;
	bool							m_SourcesShown;
	char							m_Search[256];
	// what the view holds, counted when it is rebuilt: the status strip reads it every frame
	int								m_ViewFolders;
	int								m_ViewFiles;
	// One entry of the grid. A folder and an asset are THE SAME THING to the
	// selection and to copy/cut/paste/delete - only the code that finally has to
	// touch the disk looks at `folder`. Nothing else is allowed to branch on it,
	// or a mixed selection stops behaving like one selection.
	struct SEntry
	{
		xr_string					path;
		bool						folder;
									SEntry() : folder(false) {}
									SEntry(LPCSTR p, bool f) : path(p), folder(f) {}
		bool operator==				(const SEntry& o) const { return folder == o.folder && path == o.path; }
	};
	struct SEntryHash
	{
		size_t operator()(const SEntry& entry) const noexcept
		{
			const size_t path_hash = std::hash<xr_string>{}(entry.path);
			return path_hash ^ (entry.folder ? size_t(0x9e3779b9) : 0);
		}
	};

	// UE-style multi-selection: click replaces, Ctrl toggles, Shift takes the
	// range from the anchor. m_ViewOrder is the complete filtered view, while
	// m_DrawnOrder only holds the virtualized rows visible this frame.
	xr_vector<SEntry>				m_Selection;
	xr_flat_hash_map<SEntry, u8, SEntryHash> m_SelectionLookup;
	SEntry							m_Anchor;
	bool							m_HasAnchor;
	xr_vector<SEntry>				m_ViewOrder;
	bool							m_ViewDirty;
	xr_vector<SEntry>				m_DrawnOrder;
	// screen rect of each drawn tile, parallel to m_DrawnOrder - the rubber-band
	// needs to know what it swept over
	xr_vector<ImVec4>				m_DrawnRects;
	bool							m_Marquee;
	ImVec2							m_MarqueeStart;
	xr_vector<SEntry>				m_MarqueeBase;	// selection when the drag began
	SEntry							m_PendingRange;
	bool							m_HasPendingRange;
	xr_string						m_Dragged;

	// A selection made from OUTSIDE the panel (a reveal over MCP) has to end up
	// on screen, the way the outliner scrolls to a viewport pick: the tree opens
	// the chain down to the current folder, the grid scrolls to the LAST
	// selected tile in grid order. Nobody scrolls a thousand tiles by hand. The
	// panel's own clicks never raise it - their tile is already visible.
	// Cleared once both panes have had their frame, so a reveal into a collapsed
	// window still lands when the window comes back.
	bool							m_ScrollToSelection;

	// Clipboard. Copy/Cut put things HERE; nothing is written until a paste says
	// where. Folders coming out of a read-only source are expanded at copy time,
	// because the listing that turns a folder into its items is the SOURCE's -
	// and by paste time the browser is looking at the project instead.
	struct SClip
	{
		xr_string					src;	// how the source names it
		xr_string					rel;	// where it goes under the paste target
		bool						folder;	// only ever true for project entries
	};
	xr_vector<SClip>				m_Clipboard;
	int								m_ClipSource;
	// ...and the category it was copied from: an Editor Content name is only
	// resolvable against the aliases of ITS category, and the user is free to
	// switch categories between the copy and the paste
	u32								m_ClipCategory;
	bool							m_ClipCut;		// Cut, so the paste moves

	// DARF source state
	bool							m_DarfReady;	// mounted and browsable
	xr_string						m_DarfStatus;	// why not, when it is not
	// Pictures are decoded inline, but a folder of large textures is spread over frames: a frame decodes until its
	// budget is spent, one picture at least (TakeDecodeTurn). The clock runs from the frame's first decode.
	u32								m_ThumbsThisFrame;
	CTimerBase						m_ThumbClock;

	struct SDirectoryWatcher;
	SDirectoryWatcher*				m_Watcher;

	void			Refresh				();
	// Refresh(), plus the project check that has to precede it: the panel can
	// be drawn (and listed empty) before -project has opened anything, and
	// nothing invalidated that listing afterwards.
	void			EnsureListing		();
	void			UpdateWatcher		();
	void			CloseWatcherHandles	();
	void			StopWatcher			();
	bool			ArmWatcher			();
	bool			WatchPathIsRelevant	(LPCSTR relative_path) const;
	void			PruneSelection		();
	static void		SyncFavoriteScopes	();
	// Project and game roots that the current listing state was validated against.
	xr_string						m_ListedProject;
	xr_string						m_ListedGameRoot;
	void			BuildTree			();
	// creates the folder chain for `path` and returns its node
	SFolder*		EnsureFolder		(LPCSTR path);
	void			DrawFolder			(SFolder& f);
	void			DrawTiles			();
	void			InvalidateView		();
	void			RebuildView			();
	void			CollectItems		(SFolder& f, xr_vector<int>& out, bool recursive);
	SFolder*		FindFolder			(LPCSTR path);
	// The picture of `name` for a tile that draws it `px` px wide: null while there is none (yet). The entry lives
	// in the cache - copy what is needed before the next call, which may add to it.
	const SThumb*	GetThumb			(LPCSTR name, float px);
	// an image file's picture of `side` px: from the game's archives in the Game source, from disk in the Project one;
	// false (and no texture) when it cannot be read
	bool			MakeImageThumb		(LPCSTR name, u32 side, SThumb& t);
	// into the cache, evicting the coldest entry not drawn this frame once it is full
	const SThumb*	StoreThumb			(const shared_str& key, const SThumb& t);
	// whether this frame may decode one more picture
	bool			TakeDecodeTurn		();
	// queues an offscreen model render; the result is picked up on a later frame
	void			RequestModelThumbnail(LPCSTR name);
	// Double-click handler: opens the asset's viewer, never touches the scene.
	// `err` non-null returns the reason instead of popping a dialog, which is
	// what MCP needs - a modal on an unattended machine is a hang.
	bool			OpenAsset			(LPCSTR name, xr_string* err = 0);
	// game source only: an entry under levels\ belongs to one of the game's
	// levels, folder and compiled file alike
	bool			IsGameLevelEntry	(LPCSTR name) const;
	// a listing entry -> the .level file COMMAND_LOAD can open. `source` is the
	// browser's: the project names files relative to its root, the library by
	// a name under $maps$ with the extension clamped off.
	static bool		ResolveLevelFile	(LPCSTR name, int source, string_path& out);
	void			DropCache			();
	void			SwitchSource		(int src);
	SLocation		CurrentLocation		() const;
	bool			ApplyLocation		(SLocation location, bool record, xr_string& err);
	bool			GoBack				(xr_string& err);
	bool			GoForward			(xr_string& err);
	bool			GoUp				(xr_string& err);
	bool			GoHome				(xr_string& err);
	void			PushHistory			(xr_vector<SLocation>& history, const SLocation& location);
	void			RememberNavigation	(const SLocation& origin);
	int				FindFavorite			(const SLocation& location) const;
	bool			ValidateFavorite		(SLocation& location, xr_string& err);
	// [+ Add] | Back Forward Up | breadcrumb | search | View | star | Favorites
	void			DrawToolbar			();
	// the crumbs in `width` px: the leading ones fold into a menu when they do not fit
	void			DrawBreadcrumbs		(float width);
	float			BreadcrumbWidth		() const;
	LPCSTR			RootLabel			() const;
	void			DrawAddMenu			();
	// why Add is off here, or nullptr while it works: content is only ever added to the open project
	LPCSTR			AddBlocked			() const;
	void			DrawViewMenu		();
	void			DrawFavoritesMenu	();
	// Segmented source, the category of the Editor source, favorites and the folder tree
	void			DrawSources			();
	// counts of the folder shown, the selection, the clipboard; Read only
	void			DrawStatusStrip		();
	// the selected entries that are tiles of the view (a folder picked in the tree is not one)
	int				SelectedTiles		() const;
	void			LoadView			();
	void			StoreView			();
	void			LinkGameFolder		();
	// The project's own Content is the only writable root. Editor Content is a
	// shared SDK library and the game install is someone else's install - both
	// are browse-and-copy-out sources, never edited in place.
	IC bool			IsReadOnlySource	() const { return m_Source != 0; }
	// the linked game install specifically: it owns the copy machinery and the
	// per-frame thumbnail decode budget its size demands
	IC bool			IsGameSource		() const { return m_Source == 2; }

	// selection - folders and assets go through exactly the same calls
	bool			IsSelected			(LPCSTR path, bool folder) const;
	void			AddToSelection		(const SEntry& entry);
	void			RebuildSelectionLookup();
	void			SelectAllViewEntries();
	void			SelectEntry			(LPCSTR path, bool folder, bool additive, bool range);
	void			ClearSelection		();
	void			ApplyPendingRange	();
	// rubber-band selection over the tile grid
	void			UpdateMarquee		();

	// Editor Content -> disk. The item is a library reference, so the files
	// behind it are resolved through the fs.ltx aliases first; see LibFilesFor.
	// Returns how many files actually landed.
	int				CopyLibraryItem		(u32 category, LPCSTR ref, LPCSTR dst_dir,
										 bool overwrite, xr_string& err);
	// the copy MCP runs: mirrors each asset into the layout fs.ltx implies
	int				CopyRefsToProject	(u32 category, const xr_vector<xr_string>& names,
										 bool overwrite, xr_string& err);

	// folder authoring, project source only
	void			CreateFolder		();
	void			CreateQuest			();				// NewQuest.nqasset in the folder now open
	void			CreateCharacter		();				// NewCharacter.npcasset in the folder now open
	void			CreateBehaviour		();				// NewBehaviour.behasset in the folder now open
	void			CreatePda			();				// NewPda.pdaasset in the folder now open
	void			BeginRename			(LPCSTR path);	// inline edit on that tile
	void			DrawBuildToggle		();				// Exclude from / Include in Build for the selection
	void			CommitRename		();
	// Inline rename of a folder tile: the created folder starts here with its
	// name fully selected, so typing replaces it - the usual file-manager flow.
	xr_string						m_Rename;		// folder path being renamed, "" = none
	char							m_RenameBuf[512];
	bool							m_RenameFocus;	// grab the keyboard on the next frame
	int								m_RenameSelect;	// chars to select once the box is active, -1 = done
	int								m_RenameStem;	// the same number, kept for whoever asks

	// clipboard - one set of commands for whatever is selected
	void			ClipboardCopy		(bool cut);
	void			ClipboardPaste		();
	// the paste itself, into an explicit absolute folder: shared with MCP so a
	// scripted paste goes through exactly the code the menu does
	int				PasteClipboardInto	(LPCSTR dst_root, bool overwrite, xr_string& err);
	// Deleting content has no undo of its own, so it always asks first: this
	// only raises the confirmation (XUI::Confirm, naming what goes), and
	// DeleteEntries does the work on what was named. Everything destructive in
	// this panel goes through that pair.
	void			DeleteSelection		();
	void			DeleteEntries		(const xr_vector<SEntry>& entries);
	// materialises one clipboard entry under `dst_dir`
	void			PasteEntry			(const SClip& c, LPCSTR dst_dir, bool overwrite,
										 EditorFileOps::SReport& rep);
	// expands a folder of a read-only source into the clipboard entries of the
	// items under it, each keeping its place below the folder's own name
	void			ClipExpandFolder	(LPCSTR path);
	// true when the current source/folder can receive a paste
	bool			CanPaste			() const;
	LPCSTR			PasteBlockedReason	() const;
	// Ctrl+C / Ctrl+V while the grid has focus
	void			HandleShortcuts		();
	// the double click: into a folder (a game level opens), an asset's viewer
	void			OpenEntry			(const SEntry& entry);
	// right-click on empty grid space
	void			DrawGridContextMenu	();
	// Right-click surface for ANY tile. One menu for folders and assets alike -
	// they are the same thing to every command on it.
	void			DrawEntryContextMenu(LPCSTR path, bool folder);
};
