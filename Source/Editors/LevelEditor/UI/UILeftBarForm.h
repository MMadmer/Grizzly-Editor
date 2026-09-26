#pragma once
// The Tools panel (Tools###LeftBar): a tile per object class to pick the active tool and show or hide its
// objects, the active tool's own form under a header that stays put, and the snap objects list.
class UILeftBarForm :
	public XrUI
{
public:
	UILeftBarForm();
	virtual ~UILeftBarForm();
	virtual void Draw();
	IC void Open() { bOpen = true; }
	IC void Close() { bOpen = false; }
	IC bool IsUseSnapList()const { return m_UseSnapList; }
	IC bool IsSnapListMode()const { return m_SnapListMode; }
	// MCP snap_objects: the section's two switches
	IC void SetUseSnapList(bool use) { m_UseSnapList = use; }
	IC void SetSnapListMode(bool pick) { m_SnapListMode = pick; }
private:
	void DrawTiles();
	void DrawToolHeader(u32 cls);
	// `fixed`: the body is a child of its own height (a list may fill it), not a part of the panel's scroll
	void DrawToolBody(bool fixed);
	void DrawSnapObjects();

	bool m_UseSnapList;
	bool m_SnapListMode;
	int m_SnapItem_Current;
};
