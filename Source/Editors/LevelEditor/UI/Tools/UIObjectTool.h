#pragma once
class ESceneObjectTool;
class UIObjectTool :public UIToolCustom
{
public:
	UIObjectTool();
	virtual ~UIObjectTool();
	virtual void Draw();
	void RefreshList();
	IC const char* Current() { return m_Current; }
	virtual void OnDrawUI();
	ESceneObjectTool* ParentTools;
private:
	void OnItemFocused(ListItem*item);
	void SelByRefObject(bool flag);
	void MultiSelByRefObject(bool flag);
	static void ClearSurface(bool selected);
	// the picked object: its picture (or the class icon), its facts, Browse and Refresh List
	void DrawPicked();
	const char* m_Current;
	UIItemListForm* m_ObjectList;
	ListItemsVec m_Items;
	char m_Search[64];
	bool m_MultiAppend;
	bool m_PropRandom;
	bool m_RandomAppend;
	float m_selPercent;
	bool m_AddPercent;
	bool m_Selection;
private:
	ImTextureID m_RealTexture;
	ImTextureID m_RemoveTexture;
	// what the picked object's thumbnail says about it
	bool m_HasInfo;
	int m_Faces;
	int m_Vertices;
};
