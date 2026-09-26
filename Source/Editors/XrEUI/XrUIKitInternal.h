#pragma once
#include "XrUIKitHooks.h"

// Shared by the kit's translation units (XrUIKit*.cpp), after imgui_internal.h. XrEUI only.
namespace XUI
{
namespace Internal
{
using Token = XFinedTheme::ColorToken;

inline ImU32 Col(Token token, float alpha = 1.f) { return XFinedTheme::U32(token, alpha); }
inline ImVec2 PtV(float x, float y) { return ImVec2(Pt(x), Pt(y)); }
// the end of the part of a label that is shown ("Name##id" -> "Name")
inline const char* LabelEnd(const char* label) { return label ? ImGui::FindRenderedTextEnd(label) : label; }
// [begin, end) as a string; a null end runs to the terminator
inline xr_string Str(const char* begin, const char* end = nullptr)
{
	xr_string out;
	if (begin)
		out.assign(begin, end ? end : begin + strlen(begin));
	return out;
}
// px size of a type style at the current scale
inline float FontPx(Font font) { return FontSize(font) * Dpi(); }
ImVec2 TextSize(Font font, const char* begin, const char* end = nullptr, float wrap = 0.f);
void PaintText(ImDrawList* draw_list, Font font, const ImVec2& pos, ImU32 col, const char* begin, const char* end = nullptr,
	float wrap = 0.f, const ImVec4* clip = nullptr);
// `text` cut to `avail` px with an ellipsis where `clip` says; false when it fits as it is
bool Ellipsize(Font font, const char* text, float avail, Clip clip, xr_string& out);
bool EllipsizeWith(ImFont* face, float px, const char* text, float avail, Clip clip, xr_string& out);
Icon ToneIcon(Tone tone);

// tooltips
bool HoverBlocked();
void TooltipNow(const char* text, const char* keys = nullptr, const char* title = nullptr);

// audit
void AuditAdd(const char* kind, const char* label, const ImRect& bb, const char* detail = nullptr, bool enabled = true,
	const char* why = nullptr, const char* help = nullptr);
void CheckEscape(const ImRect& bb, const char* label);
void AuditFrameEnd(bool keep);
// a window hidden after it was drawn (a dialog measuring itself): what it and its children reported this frame goes
void AuditHideWindow(const char* name);

// the one button renderer
struct BtnSpec
{
	const char* id = nullptr;		// the ImGui id; its visible part is the text unless `text` is set
	const char* text = nullptr;
	const char* tip = nullptr;
	const char* tip_title = nullptr;	// the tooltip's first line (an icon-only button names itself)
	const char* keys = nullptr;
	const char* why = nullptr;		// the tooltip while disabled
	Icon icon = Icon::None;
	float icon_pt = 16.f;
	Kind kind = Kind::Secondary;
	bool on = true;
	bool checked = false;
	bool is_default = false;
	bool caret = false;				// a small chevron after the text
	bool layout = true;				// false: ItemAdd only, the cursor does not move
	Font font = Font::Body;
	ImVec2 size = ImVec2(0.f, 0.f);	// px; 0 = measured / frame height
	float text_w = 0.f;				// px reserved for the text (the widest of a label set)
	ImDrawFlags corners = ImDrawFlags_RoundCornersAll;
	const char* audit_label = nullptr;
};
bool ButtonEx(const BtnSpec& spec, ImRect* out_bb = nullptr, bool* out_hovered = nullptr);
float ButtonWidth(const BtnSpec& spec);

// popups
void OpenPopupAt(ImGuiID id, const ImVec2& pos);
bool TakePopupRequest(const char* name);
bool PeekPopupRequest(const char* prefix);

// the panel being drawn (BeginPanel) and a pending "item:<name>" scroll into it
void ScrollToItem(const char* name, const ImRect& bb);
void PanelBegun(const char* title_id, bool visible);
void PanelEnded();

// layout
void RescaleLayout(float ratio);
void RegisterLayoutHandler();

// dialogs and toasts
void RecentreDialogs();
void DrawQueuedDialogs();
void DrawToasts();
bool InDialogFooter();
void PlaceFooterButton(float width);
void MarkFooterDanger();
bool InDialogScope();
// -iconsheet (XrUIIconSheet.cpp)
void DrawIconSheetWindow();

// the precedence: what an UnsavedPrompt answers inside an MCP request; a Cancel is remembered for McpUnsaved()
Answer McpUnsavedAnswer(const Strings& what);
// the property row being drawn shows several values (PickField says so)
void SetRowMixed(bool mixed);
}
}
