#pragma once
// A property list on the kit's grid (docs/UI_DESIGN.md, section 4 "Property grid"): a label column and a value
// column per row, folders as rows that toggle, a search over every row and the hovered row's help at the bottom.
class XREPROPS_API UIPropertiesForm :public XrUI
{
	friend class UIPropertiesItem;
public:
	UIPropertiesForm();
	virtual ~UIPropertiesForm();
	// the whole form: search, the grid and the help footer, as SetChrome says
	virtual void Draw();
	// the rows alone, at the cursor of the window being drawn: the host lays out the rest
	void DrawGrid();
	// no rows drawn this frame (the host shows something else): the chooser or dialog a row opened stays up
	void Tick();
	void AssignItems(PropItemVec& items);
	PropItem* FindItem(const char* path);
	PropItem* FindItemOfName(shared_str name);
	void ClearProperties();
	// Opens the chooser bound to a PROP_CHOOSE row exactly like clicking that row
	// does - the same entry point the mouse uses, so both paths cannot drift apart.
	bool BeginChooseEdit(PropItem* item);
	bool HasPendingChooseEdit() const { return !!m_EditChooseValue; }
	void SetFilter(LPCSTR filter);
	LPCSTR GetFilter() const { return m_Filter; }
	void ExpandAll();
	void CollapseAll();
	u32 VisiblePropertyCount() const { return m_VisiblePropertyCount; }
	u32 PropertyCount() const { return m_PropertyCount; }
	IC void SetReadOnly(bool enable) { m_Flags.set(plReadOnly, enable); }
	IC bool IsModified() { return m_bModified;}
	// the host took the changes (saved them): IsModified() answers for what comes after
	IC void ClearModified() { m_bModified = false; }
	IC bool Empty() { return m_Items.size() == 0; }
	// the rows as assigned, and in the order the grid draws them (a level's own rows before its categories; MCP
	// property_inspector lists them so)
	const PropItemVec& Items() const { return m_Items; }
	void RowsInOrder(PropItemVec& out) const;
	void SetModifiedEvent(TOnModifiedEvent modif = 0) { OnModifiedEvent = modif; }

	// What the form draws around its rows. Auto: a search once there is something to search, and the grid
	// scrolls with the help under it when the host gives it the room; Panel: all of it; Inline: the rows only.
	enum EChrome { chromeAuto = 0, chromePanel, chromeInline };
	void SetChrome(int chrome) { m_Chrome = chrome; }
	// the grid's name: its label column is kept per role (grid.<role> in the layout); `label_frac` is the column's share
	// until the author drags its edge (a grid of deep folders and short values wants more than the kit's .40)
	void SetRole(LPCSTR role, float label_frac = .40f) { m_Role = role && role[0] ? role : "default"; m_LabelFrac = label_frac; }
	// only this top-level folder's rows, as a page (Preferences); null shows them all. A search shows every match.
	void SetRootFolder(LPCSTR folder) { m_RootFolder = folder ? folder : ""; }
	// a document's form (Preferences): values are at most this many em wide instead of the whole value column, so a
	// number is not drawn far away from its label. Rows with a button at their end keep the column. 0: the column.
	void SetValueMaxWidth(float em) { m_ValueMaxEm = em; }
	// what a row is for: its label's tooltip, the help footer and MCP. Without one a number says its range.
	void SetHelp(LPCSTR key, LPCSTR help);
	xr_string GetHelp(PropItem* item) const;
	// the unit a number is shown in ("m", "fps"): drawn after the value and said by its range. Angles are in degrees
	// by themselves, and a key that ends in a unit ("Weld distance (m)") gives its own.
	void SetUnit(LPCSTR key, LPCSTR unit);
	xr_string GetUnit(PropItem* item) const;
	// the label a key is shown under: its last part without a trailing range or unit ("Quality [0-3]" -> "Quality",
	// "Interval (minutes)" -> "Interval")
	static xr_string LabelOf(LPCSTR key);
	// the unit a key's trailing "(m)", "(minutes)" names; empty for none
	static xr_string UnitOf(LPCSTR key);
	// the band under the rows: the help of the row under the mouse, or of the last one it was over while that row is
	// still shown. The panel chrome draws it; a host that lays out the grid itself draws it where it wants. `width` 0
	// takes the rest of the row.
	void DrawHelpFooter(float width = 0.f);
	static float HelpFooterHeight();
	// a value the host draws itself, in the row's value column (its width set); true when it did
	using TValueDrawer = std::function<bool(PropItem*)>;
	void SetValueDrawer(const TValueDrawer& drawer) { m_ValueDrawer = drawer; }
	// the value given as text, applied the way an edit of the row applies it (MCP property_set); false says why
	bool SetValueText(PropItem* item, LPCSTR text, xr_string& err);
	// the top-level folders with how many rows each holds (all of them, or the ones that match the search)
	void Folders(xr_vector<std::pair<xr_string, u32> >& out, bool matching) const;
public:
	enum {
		plReadOnly = (1 << 0),
	};
	Flags32 m_Flags;
	IC bool IsReadOnly()const { return m_Flags.is(plReadOnly); }
private:
	PropItemVec m_Items;
	PropItem* m_EditChooseValue;
	// the row a pending chooser edit belongs to, kept by key so a properties
	// rebuild while the modal is open can hand the edit back to the new item
	shared_str m_PendingChooseKey;
	PropItem* m_EditTextureValue;
	shared_str m_PendingTextureKey;
	PropItem* m_EditShortcutValue;
	shared_str m_PendingShortcutKey;
	// a chooser this form opened: it is drawn until it answers, even when its row went away meanwhile
	bool m_ChooserOwned;
	// a list assigned while the rows are drawn waits for the frame to be through
	int m_Drawing;
	bool m_HasDeferred;
	PropItemVec m_Deferred;
	void ApplyDeferred();
private:
	TOnModifiedEvent 	OnModifiedEvent;
private:
	// the Edit Text dialog: the row by key (a rebuild frees the item), the text being edited
	xr_string m_EditTextKey;
	const void* m_EditTextTarget;
	xr_vector<char> m_EditTextBuffer;
	bool m_EditTextOpen;
	void OpenEditText(PropItem* item);
	void DrawEditText();
	// the row whose text is typed in place, and what it holds so far
	xr_string m_InlineKey;
	const void* m_InlineTarget;
	xr_vector<char> m_InlineBuffer;
private:
	GameTypeChooser m_EditGameTypeChooser;
	xr_string m_EditGameTypeKey;
	bool m_EditGameTypeOpen;
	void OpenEditGameType(PropItem* item);
	void DrawEditGameType();
	bool m_bModified;
	void Modified() { m_bModified = true; if (!OnModifiedEvent.empty()) OnModifiedEvent(); }
	// a text row's new value through its AfterEdit, as typing it would
	bool ApplyText(PropItem* item, LPCSTR text);
private:
	void TakeResults();
	void DrawSearchRow();
	void DrawRows();
	void DrawDialogs();
	void RefreshFilter();
	bool IsFiltering() const { return m_Filter[0] != 0; }
	bool RestoreExpansion(LPCSTR path, bool fallback) const;
	void RememberExpansion(LPCSTR path, bool expanded);
	char m_Filter[256];
	u32 m_VisiblePropertyCount;
	u32 m_PropertyCount;
	bool m_ExpandByDefault;
	xr_flat_hash_map<xr_string, bool> m_ExpansionState;
	int m_Chrome;
	xr_string m_Role;
	float m_LabelFrac;
	float m_ValueMaxEm;
	xr_string m_RootFolder;
	xr_map<xr_string, xr_string> m_Help;
	xr_map<xr_string, xr_string> m_Units;
	TValueDrawer m_ValueDrawer;
	// the help footer: the row under the mouse this frame, the one the footer speaks of, and whether that one was drawn
	xr_string m_HoverKey;
	xr_string m_FooterKey;
	bool m_FooterShown;
	int m_RowsFrame;

	UIPropertiesItem m_Root;
};

