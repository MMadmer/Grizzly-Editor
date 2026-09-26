#pragma once
class UIToolCustom:public XrUI
{
public :
	virtual void OnDrawUI() {}
};

// ---- shared by the Tools panel, its tool forms and the World Outliner (docs/UI_DESIGN.md, section 6) ----------
// A scene object class (an ObjClassID): its icon, its short name for a tile and its full name for a header
XUI::Icon	ToolClassIcon			(u32 cls);
LPCSTR		ToolClassLabel			(u32 cls);
LPCSTR		ToolClassTitle			(u32 cls);

// One button of a row that wraps
struct XUI_LocalButton
{
	LPCSTR		label;
	LPCSTR		tip		= nullptr;
	bool		on		= true;
	LPCSTR		why		= nullptr;		// the tooltip while it is off
};
// Secondary buttons left to right, a new row when the next one does not fit: nothing is ever cut.
// The index of the one pressed, or -1.
int			XUI_LocalButtonFlow		(const XUI_LocalButton* buttons, int count);
// [Select % of Same] [percent] and "Add to Selection" under them: the Object, Spawn and Group tools share it.
// True when the button is pressed.
bool		XUI_LocalPercentSelect	(float* percent, bool* add);
// A square eye of `size_pt` at the cursor: open while visible, crossed out while hidden. True on click.
bool		XUI_LocalEyeToggle		(LPCSTR id, bool visible, LPCSTR tip, float size_pt = 22.f);
// The search field over a tool's list and the list itself: the list filters its tree by the key and the hint,
// and opens every folder that holds a match while the search is on. The list is the tool's canvas, and the body has
// one scroll at a time: the list takes the room the tool body leaves under it (what follows it stays in view, the
// body does not scroll), and with less room than a few rows it flows in the body's scroll at its full height. One per
// tool body.
void		XUI_LocalFilteredList	(LPCSTR id, char* search, size_t search_size, UIItemListForm* list);
// A thumbnail square of `side` px, or - with no texture (nothing picked, no thumbnail made) - the same square on the
// inset with `icon` in it, never a placeholder texture. `tip` is the empty square's tooltip.
void		XUI_LocalThumbnail		(ImTextureID texture, float side, XUI::Icon icon, LPCSTR tip);
