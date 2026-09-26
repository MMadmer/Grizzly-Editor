#pragma once
class XREPROPS_API UIPropertiesItem :public UITreeItem
{
	friend class UIPropertiesForm;
public:
	UIPropertiesItem(shared_str Name, UIPropertiesForm* PropertiesFrom);
	virtual ~UIPropertiesItem();
	PropItem* PItem;
	UIPropertiesForm* PropertiesFrom;
	// a row of the grid and, when open, the rows under it
	void Draw(int depth);
	void DrawRoot();
	// the value column of the row being drawn: one widget, its width already set
	void DrawValue();
protected:
	virtual UITreeItem* CreateItem(shared_str Name);
private:
	void RemoveMixed();
	void DrawTextValue();
	void DrawButtons();
	void DrawCanvas();
	// how wide the value has to be to sit on one line (X, Y and Z side by side, every choice of a row of them); 0 when
	// it fits any width
	float OneLineWidth() const;
	// the value ends in a button of its own at the row's right end (edit the whole text, clear a shortcut)
	bool TakesTrailing() const;
	// a category: a header with rows under it and no value of its own
	bool IsCategory() const { return !PItem && !Items.empty(); }
	// every level lists its own rows first and its categories after them, each kept in the order it was given: a row
	// listed after a category would be drawn under that category's rows and read as one of them
	void Arrange();
	// the rows in the order the grid draws them
	void CollectRows(PropItemVec& out) const;
	bool RefreshFilter(LPCSTR filter, const xr_string& parentPath, bool parentMatches, u32& visibleCount, u32& totalCount);
	void SetExpandedRecursive(bool expanded);
	void Reset(bool expanded);
	void CountRows(u32& total, u32& visible) const;
	bool m_FilterVisible;
	bool m_Expanded;
	xr_string m_Path;
};
