#pragma once
class UIParticlesTool :public UIToolCustom
{
public:
	UIParticlesTool();
	virtual ~UIParticlesTool();
	virtual void Draw();
	IC const char* Current()const { return m_Current; }
private:
	void SelByRef(bool flag);
	void OnItemFocused(ListItem* item);
	UIItemListForm* m_ParticlesList;
	ListItemsVec m_Items;
	char m_Search[64];
	const char* m_Current;
};