#pragma once
class UISpawnTool :public UIToolCustom
{
public:
	// Shape entities (a restrictor and friends) are placed with a shape attached; the
	// SDK let you pick which before spawning, so this does too.
	bool			m_ShapeSphere = true;
	bool			IsSphereShape() const { return m_ShapeSphere; }

	UISpawnTool();
	virtual ~UISpawnTool();
	virtual void Draw();
	void SelByRefObject(bool flag);
	void MultiSelByRefObject(bool clear_prev);
	IC const char* Current() { return m_Current; }
	IC void SetAttachObject(bool AttachObject) {m_AttachObject = AttachObject;}
	IC bool IsAttachObject()const { return m_AttachObject; }
	// pSettings is swapped when a project links a game - the tool calls this
	void RefreshList();
	// A $spawn path as the list (and list_spawn_sections) shows it: every part trimmed of blanks and of a leading
	// ';', empty parts dropped. "" when nothing is left.
	static xr_string ListPath(LPCSTR spawn);
private:
	void OnItemFocused(ListItem* item);
	const char* m_Current;
	UIItemListForm* m_SpawnList;
	ListItemsVec m_Items;
	char m_Search[64];
	float m_selPercent;
	bool m_AddPercent;
	bool m_AttachObject;
};
