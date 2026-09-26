#pragma once
class CLAItem;

//------------------------------------------------------------------------------
// A kit widget still local to the libraries (docs/UI_DESIGN.md, section 8: "A widget the kit lacks becomes
// XUI_Local in the package"). Defined in UIEditLightAnim.cpp.
//------------------------------------------------------------------------------
// The item list of a library tab: a search field over a 22 pt one-column list, the leaf name with its
// folder muted after it, only the rows on screen drawn (a texture library holds thousands).
struct XUI_LocalLibraryList
{
	char			search[128] = {};
	// the rows the list shows: indices into the items, rebuilt when the search, the items or a filter change
	xr_vector<int>	shown;
	// the empty state's action was pressed in the last Draw
	bool			acted = false;

	// drops the cached rows (a filter changed the items' visibility)
	void			Invalidate		() { m_Count = size_t(-1); }
	// Draws into the rest of the current window. Returns the index of the item picked this frame (a click, or
	// Up / Down while the list has the focus), -1 for none. With nothing to show and no search, the empty state
	// says empty_title / empty_text and offers empty_action when there is one.
	int				Draw			(const char* id, const ListItemsVec& items, const char* selected,
									 const char* empty_title, const char* empty_text, const char* empty_action = nullptr);
	// the item a search would pick first: the one named exactly, else the first match
	int				Find			(const ListItemsVec& items, const char* name) const;
	// what the list shows now, counted without a frame (MCP asks between frames): the rows and the first one
	u32				Count			(const ListItemsVec& items, int* first = nullptr) const;

private:
	xr_string		m_Search;
	xr_string		m_Selected;
	size_t			m_Count = size_t(-1);
	int				m_Scroll = -1;		// the row to bring into view next frame
	void			Rebuild			(const ListItemsVec& items);
};

// Where a library reads from, for its empty state: "the editor's rawdata\textures\ folder" when the FS alias lies
// in the editor's folder, else the whole path.
xr_string LibraryFolderText(LPCSTR alias);

// MCP `library`: the three library tabs - light_animations, texture_library, sound_library
ECORE_API void EditorLibraryMcp(LPCSTR raw, xr_string& out);

class UIEditLightAnim :public XrUI
{
public:
	UIEditLightAnim();
	virtual ~UIEditLightAnim();
	virtual void Draw();
public:
	static void Update();
	static void Show();
	static bool IsOpen() { return !!Form; }
	// MCP library kind=light_animations: state, select item=, search=
	static void Mcp(const xr_string& action, LPCSTR raw, xr_string& out);
private:
	static UIEditLightAnim* Form;
private:
	void UpdateProperties();
	void InitializeItems();
	void Select(int index);
	void DrawToolbar(XUI::Toolbar& tb);
	void DrawPreview();
	void DrawTimeline(float height);
	void DrawKeyBar();
	void DrawStatus();
	void Save();
	void Reload();

	ListItemsVec			m_List;
	XUI_LocalLibraryList	m_ListView;
	UIPropertiesForm*		m_Props;
	CLAItem*				m_CurrentItem;
	bool					m_Modife;
	int						m_PointerValue;
	bool					m_RenderAlpha;
private:
	void OnCreateKeyClick();
	void OnModified();
	void OnItemFocused(ListItem*);
	bool OnFrameCountAfterEdit(PropValue* v, s32& val);
	void ClampPointer();
};
