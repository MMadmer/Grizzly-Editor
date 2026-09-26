#pragma once
class UIItemListFormItem :public UITreeItem
{
public:
	UIItemListFormItem(shared_str Name);
	virtual ~UIItemListFormItem();

	bool bIsFavorite;

	class UIItemListForm* Form;
	int Index;
	ListItem* Object;
	shared_str Text;
	bool bSelected;
	// the item matches the search, or a folder holds one that does
	bool Visible;
	void Draw();
	void DrawRoot();
	void Sort();
	void ClearSelection();
	void Selected(int Start, int End);
	void SelectedToFavorite(bool Favorite);
	bool UpdateFilter(LPCSTR query);
	// a row under this folder (at any depth) that `name` names: a folder's name, a leaf's name or its whole key
	bool Holds(const char* name) const;

	void CheckFavorited();
	void SetFavorite(bool bFavorite);
	void FillFavorited(xr_vector< ListItem*>& selected);
	void CheckFavorited(xr_vector< ListItem*>& selected);
protected:
	virtual UITreeItem* CreateItem(shared_str Name);
	bool m_bIsMixed;
};
