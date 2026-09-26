#pragma once
class UIChooseFormItem :public UITreeItem
{
public:
	UIChooseFormItem(shared_str Name);
	virtual ~UIChooseFormItem();

	bool bIsFavorite;

	class UIChooseForm* Form;
	int Index;
	SChooseItem* Object;
	shared_str Text;
	bool bSelected;
	// the item matches the search, or a folder holds one that does
	bool Visible;
	// the item the chooser opened on: its folders open and the list scrolls to it once
	bool Reveal;
	void Draw();
	void DrawRoot();
	void Sort();
	// worked out once per query: a library holds thousands of items
	bool UpdateFilter(LPCSTR query);
	void CheckFavorited();
	void SetFavorite(bool bFavorite);
	void FillFavorited(xr_vector< SChooseItem*>& selected);
	void CheckFavorited(xr_vector< SChooseItem*>& selected);
	void ClearSelection();
	void Selected(int Start, int End);
	void SelectedToFavorite(bool Favorite);
protected:
	virtual UITreeItem* CreateItem(shared_str Name);
	bool m_bIsMixed;
};
