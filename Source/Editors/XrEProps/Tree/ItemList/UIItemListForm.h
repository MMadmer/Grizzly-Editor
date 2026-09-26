#pragma once
// A list of library items as a tree (the Tools panels): a filter over it opens the folders that hold a match. The
// search field is the host's (SetFilter), or the list's own with fSearch.
class XREPROPS_API UIItemListForm :public XrUI
{
	friend class UIItemListFormItem;
	TOnILItemsFocused	OnItemsFocusedEvent;
	TOnILItemFocused	OnItemFocusedEvent;
public:
	UIItemListForm();
	virtual ~UIItemListForm();
public:
	virtual void Draw() { Draw(0.f); }
	// `height` px is the list's outer height (the host sizes it); 0 = tall enough for its rows, 10 to 24 of them;
	// negative = as tall as all its rows, with no scroll of its own: the list flows in the host's scroll
	void Draw(float height);
	// one row of the tree, px
	static float RowHeight();
	void ClearList();
	void ClearSelected();
	void SelectItem(const char* name);
	// `name_selection`: the item selected once the list is in
	void AssignItems(ListItemsVec& items, const char* name_selection = nullptr, bool clear_floder = true, bool save_selected = false);
	IC const ListItemsVec& GetItems()const {return m_Items;}
	bool GetSelected(RStringVec& items)const;
	// every word in the item's key or its hint, any case (XUI::Match); empty shows every item
	void SetFilter(LPCSTR text);
	LPCSTR GetFilter() const { return m_Search; }
	//int GetSelected(LPCSTR pref, ListItemsVec& items, bool bOnlyObject);???
public:
	IC void		SetOnItemsFocusedEvent(TOnILItemsFocused e) { OnItemsFocusedEvent = e; }
	IC void		SetOnItemFocusedEvent(TOnILItemFocused e) { OnItemFocusedEvent = e; }
public:
	enum {
		fMenuEdit =			(1<<0),
		fMultiSelect =		(1<<1),
		// the list draws its own search field above the tree
		fSearch =			(1<<2),
	};
	Flags32 m_Flags;
private:
	void UpdateSelected(UIItemListFormItem* NewSelected);
	ListItemsVec m_Items;
	UIItemListFormItem m_RootItem;
	UIItemListFormItem* m_SelectedItem;
	// the rows assigned, and the filter over them (worked out once per query)
	u32 m_Count;
	char m_Search[128];
	xr_string m_AppliedSearch;
	// rows a focus callback replaced while the list was drawn: freed once the frame is through with them
	int m_Drawing;
	xr_vector<UITreeItem*> m_Graveyard;
};

