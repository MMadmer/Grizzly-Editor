#define IMGUI_DEFINE_MATH_OPERATORS
#include "stdafx.h"
#include "imgui_internal.h"
#include "imgui_impl_win32.h"
#include "XrUIKitInternal.h"
#include "..\Public\ShortcutText.h"
#include <objbase.h>
#include <shobjidl.h>
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "shell32.lib")

using namespace XUI::Internal;

// ===================================================================================================
// Scale, precedence
// ===================================================================================================
namespace
{
float s_Dpi = 1.f;
float s_PendingDpi = 0.f;
bool s_PendingStyle = false;
bool s_ProgressFrame = false;
bool s_SuppressHover = false;

bool s_FromMcp = false;
XUI::Answer s_McpUnsavedAnswer = XUI::Answer::Cancel;
bool s_McpUnsavedRefused = false;
XUI::Strings s_McpUnsavedWhat;

// -uiscale <f>: tests and screenshots at a fixed scale, whatever the monitor says
float CommandLineScale()
{
	static float s_Scale = -1.f;
	if (s_Scale < 0.f)
	{
		s_Scale = 0.f;
		if (const char* at = strstr(::GetCommandLineA(), "-uiscale"))
		{
			float value = 0.f;
			if (sscanf_s(at + 8, " %f", &value) == 1 && value >= .5f && value <= 4.f)
				s_Scale = value;
			else
				Msg("! -uiscale wants a scale from 0.5 to 4, e.g. -uiscale 1.5");
		}
	}
	return s_Scale;
}

void ApplyDpi(float scale)
{
	const float old = s_Dpi;
	s_Dpi = scale;
	s_PendingStyle = false;
	if (!ImGui::GetCurrentContext())
		return;
	XUI::Hooks::RebuildStyle();
	if (old > 0.f && old != scale)
		RescaleLayout(scale / old);
	RecentreDialogs();
}
}

float XUI::Dpi()
{
	return s_Dpi;
}

float XUI::Pt(float pt)
{
	return pt * s_Dpi;
}

float XUI::Em(float n)
{
	const float size = ImGui::GetCurrentContext() ? ImGui::GetFontSize() : 0.f;
	return n * (size > 0.f ? size : FontBaseSize * s_Dpi);
}

void XUI::SetDpi(float scale)
{
	if (!(scale > 0.f))
		return;
	scale = ImClamp(scale, .5f, 4.f);
	ImGuiContext* ctx = ImGui::GetCurrentContext();
	if (ctx && ctx->WithinFrameScope)
	{
		s_PendingDpi = scale;
		return;
	}
	s_PendingDpi = 0.f;
	ApplyDpi(scale);
}

float XUI::ScaleForDpi(unsigned dpi)
{
	const float forced = CommandLineScale();
	if (forced > 0.f)
		return forced;
	return dpi ? float(dpi) / 96.f : 1.f;
}

bool XUI::Unattended()
{
	static const bool s_NoDialogs = !!strstr(::GetCommandLineA(), "-nodlg");
	return s_NoDialogs;
}

bool XUI::FromMcp()
{
	return s_FromMcp;
}

XUI::McpScope::McpScope(Answer unsaved) : m_Previous(s_FromMcp), m_PreviousUnsaved(s_McpUnsavedAnswer)
{
	if (!m_Previous)
	{
		s_McpUnsavedRefused = false;
		s_McpUnsavedWhat.clear();
	}
	s_FromMcp = true;
	s_McpUnsavedAnswer = unsaved;
}

XUI::McpScope::~McpScope()
{
	s_FromMcp = m_Previous;
	s_McpUnsavedAnswer = m_PreviousUnsaved;
}

bool XUI::McpUnsaved(Strings* what)
{
	if (what)
		*what = s_McpUnsavedWhat;
	return s_McpUnsavedRefused;
}

XUI::Answer XUI::Internal::McpUnsavedAnswer(const Strings& what)
{
	if (s_McpUnsavedAnswer == XUI::Answer::Cancel)
	{
		s_McpUnsavedRefused = true;
		s_McpUnsavedWhat = what;
	}
	return s_McpUnsavedAnswer;
}

void XUI::SuppressHover(bool suppress)
{
	s_SuppressHover = suppress;
	// ImGui's own hover goes too: a legacy panel's SetTooltip or hover fill would land in the picture as well
	if (ImGui::GetCurrentContext())
	{
		ImGuiIO& io = ImGui::GetIO();
		io.ConfigFlags = suppress ? io.ConfigFlags | ImGuiConfigFlags_NoMouse : io.ConfigFlags & ~ImGuiConfigFlags_NoMouse;
	}
}

void XUI::Hooks::ContextCreated(HWND hwnd)
{
	ImGuiIO& io = ImGui::GetIO();
	// the scale is the kit's to set (SetDpi), never the backend's
	io.ConfigDpiScaleFonts = false;
	io.ConfigDpiScaleViewports = false;
	RegisterLayoutHandler();
	const float forced = CommandLineScale();
	SetDpi(forced > 0.f ? forced : ImGui_ImplWin32_GetDpiScaleForHwnd(hwnd));
}

void XUI::Hooks::BeforeNewFrame()
{
	if (!FontsLoaded())
		LoadFonts();
	if (s_PendingDpi > 0.f)
	{
		const float scale = s_PendingDpi;
		s_PendingDpi = 0.f;
		ApplyDpi(scale);
	}
	if (s_PendingStyle)
	{
		s_PendingStyle = false;
		RebuildStyle();
	}
}

void XUI::Hooks::SetProgressFrame(bool progress)
{
	s_ProgressFrame = progress;
}

void XUI::Hooks::RequestStyle()
{
	s_PendingStyle = true;
}

// ===================================================================================================
// Text
// ===================================================================================================
ImU32 XUI::ToneColor(Tone tone, float alpha)
{
	switch (tone)
	{
	case Tone::Info:		return Col(Token::AccentBright, alpha);
	case Tone::Success:		return Col(Token::Success, alpha);
	case Tone::Warning:		return Col(Token::Warning, alpha);
	case Tone::Error:		return Col(Token::Error, alpha);
	case Tone::Required:	return Col(Token::Required, alpha);
	default:				return Col(Token::TextMuted, alpha);
	}
}

XUI::Icon XUI::Internal::ToneIcon(Tone tone)
{
	switch (tone)
	{
	case Tone::Success:		return Icon::Success;
	case Tone::Warning:		return Icon::Warning;
	case Tone::Error:		return Icon::Error;
	case Tone::Required:	return Icon::Required;
	default:				return Icon::Info;
	}
}

ImVec2 XUI::Internal::TextSize(Font font, const char* begin, const char* end, float wrap)
{
	if (!begin)
		return ImVec2(0.f, FontPx(font));
	ImFont* face = GetFont(font);
	if (!face)
		face = ImGui::GetFont();
	ImVec2 size = face->CalcTextSizeA(FontPx(font), FLT_MAX, wrap, begin, end);
	size.x = IM_TRUNC(size.x + .99999f);
	return size;
}

void XUI::Internal::PaintText(ImDrawList* draw_list, Font font, const ImVec2& pos, ImU32 col, const char* begin, const char* end,
	float wrap, const ImVec4* clip)
{
	if (!begin || begin == end)
		return;
	ImFont* face = GetFont(font);
	if (!face)
		face = ImGui::GetFont();
	draw_list->AddText(face, FontPx(font), pos, col, begin, end, wrap, clip);
}

bool XUI::Internal::Ellipsize(Font font, const char* text, float avail, Clip clip, xr_string& out)
{
	ImFont* face = GetFont(font);
	return EllipsizeWith(face ? face : ImGui::GetFont(), FontPx(font), text, avail, clip, out);
}

bool XUI::Internal::EllipsizeWith(ImFont* face, float px, const char* text, float avail, Clip clip, xr_string& out)
{
	if (!text)
		text = "";
	const char* end = text + strlen(text);
	if (face->CalcTextSizeA(px, FLT_MAX, 0.f, text, end).x <= avail + .5f)
		return false;

	static const char* kEllipsis = "\xE2\x80\xA6";
	const float room = ImMax(0.f, avail - face->CalcTextSizeA(px, FLT_MAX, 0.f, kEllipsis).x);

	auto prefix_end = [&](float width)
	{
		const char* rest = text;
		face->CalcTextSizeA(px, width, 0.f, text, end, &rest);
		return rest;
	};
	// the longest tail that fits: widths only grow as the start moves left
	auto suffix_begin = [&](float width)
	{
		xr_vector<const char*> starts;
		for (const char* p = text; p < end;)
		{
			starts.push_back(p);
			unsigned c = 0;
			p += ImMax(1, ImTextCharFromUtf8(&c, p, end));
		}
		int lo = 0, hi = int(starts.size());
		while (lo < hi)
		{
			const int mid = (lo + hi) / 2;
			if (face->CalcTextSizeA(px, FLT_MAX, 0.f, starts[mid], end).x <= width)
				hi = mid;
			else
				lo = mid + 1;
		}
		return lo < int(starts.size()) ? starts[lo] : end;
	};

	switch (clip)
	{
	case Clip::Start:
		out = kEllipsis;
		out.append(suffix_begin(room), end);
		break;
	case Clip::Middle:
	{
		const char* head = prefix_end(room * .5f);
		const float head_w = face->CalcTextSizeA(px, FLT_MAX, 0.f, text, head).x;
		const char* tail = suffix_begin(room - head_w);
		out.assign(text, head);
		out += kEllipsis;
		out.append(tail, end);
		break;
	}
	default:
		out.assign(text, prefix_end(room));
		out += kEllipsis;
		break;
	}
	return true;
}

bool XUI::Internal::HoverBlocked()
{
	return s_SuppressHover || ImGui::GetIO().AppFocusLost || ImGui::GetDragDropPayload();
}

// "Ctrl+Shift+S" as keycaps; a '+' that ends a key ("Num +") stays in it
float XUI::Keycaps(ImDrawList* draw_list, const ImVec2& pos, cstr keys, float h, bool enabled)
{
	if (!keys || !*keys)
		return 0.f;
	if (h <= 0.f)
		h = FontPx(Font::Small) + Pt(4.f);
	const float text_y = IM_ROUND((h - FontPx(Font::Small)) * .5f);
	float x = pos.x;
	for (const char* p = keys; *p;)
	{
		const char* e = p;
		while (*e && !(*e == '+' && e > p && e[1] && e[1] != ' '))
			++e;
		if (x > pos.x)
			x += Pt(3.f);
		const float w = TextSize(Font::Small, p, e).x + Pt(10.f);
		if (draw_list)
		{
			const ImRect bb(ImVec2(x, pos.y), ImVec2(x + w, pos.y + h));
			draw_list->AddRectFilled(bb.Min, bb.Max, Col(Token::Raised), Pt(3.f));
			draw_list->AddRect(bb.Min, bb.Max, Col(enabled ? Token::BorderStrong : Token::BorderSubtle), Pt(3.f));
			PaintText(draw_list, Font::Small, ImVec2(bb.Min.x + Pt(5.f), bb.Min.y + text_y), Col(enabled ? Token::TextSecondary : Token::TextDisabled), p, e);
		}
		x += w;
		p = *e ? e + 1 : e;
	}
	return x - pos.x;
}

namespace
{
// keycaps as one item of the current window (a tooltip's last line)
void KeycapsItem(const char* keys)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	const float h = FontPx(XUI::Font::Small) + XUI::Pt(4.f);
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, pos + ImVec2(XUI::Keycaps(nullptr, pos, keys, h), h));
	ImGui::ItemSize(bb.GetSize());
	if (ImGui::ItemAdd(bb, 0))
		XUI::Keycaps(window->DrawList, pos, keys, h);
}
}

namespace
{
// spaces as wide as `w` px: ImGui lays a menu's columns out from text, so they reserve room a glyph is drawn over
void SpacesFor(float w, char* pad, int n)
{
	const int spaces = ImClamp(int(ceilf(w / ImMax(1.f, ImGui::CalcTextSize(" ").x))), 1, n - 1);
	memset(pad, ' ', size_t(spaces));
	pad[spaces] = 0;
}

bool MenuItemWith(const char* label, XUI::Icon icon, const char* keys, bool selected, bool enabled, bool danger)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems)
		return false;
	const bool has_keys = keys && *keys;
	const bool has_icon = icon != XUI::Icon::None;
	// a menu bar lays items out in a row and shows no shortcut
	if (window->DC.LayoutType == ImGuiLayoutType_Horizontal || (!has_keys && !has_icon && !danger))
		return ImGui::MenuItem(label, nullptr, selected, enabled);
	const float h = FontPx(XUI::Font::Small) + XUI::Pt(2.f);
	const float icon_px = XUI::Pt(16.f);
	// the shortcut column opens with a clear gap after the longest label; its keycaps stand flush right in it
	const float keys_gap = XUI::Pt(24.f);
	const float keys_w = has_keys ? XUI::Keycaps(nullptr, ImVec2(0.f, 0.f), keys, h) : 0.f;
	char keys_pad[96] = {}, icon_pad[32] = {};
	if (has_keys)
		SpacesFor(keys_gap + keys_w, keys_pad, int(sizeof(keys_pad)));
	if (has_icon)
		SpacesFor(icon_px, icon_pad, int(sizeof(icon_pad)));
	const ImVec2 pos = window->DC.CursorPos;
	const float base = window->DC.CurrLineTextBaseOffset;
	const float avail = ImGui::GetContentRegionAvail().x;
	const ImU32 tone = enabled && danger ? XUI::ToneColor(XUI::Tone::Error) : 0;
	if (tone)
		ImGui::PushStyleColor(ImGuiCol_Text, tone);
	const bool pressed = ImGui::MenuItemEx(label, has_icon ? icon_pad : nullptr, has_keys ? keys_pad : nullptr, selected, enabled);
	if (tone)
		ImGui::PopStyleColor();
	if (g.LastItemData.StatusFlags & ImGuiItemStatusFlags_Visible)
	{
		const ImGuiMenuColumns& columns = window->DC.MenuColumns;
		if (has_icon)
			XUI::DrawIcon(window->DrawList, icon, ImVec2(IM_ROUND(pos.x + columns.OffsetIcon), IM_ROUND(pos.y + base + (g.FontSize - icon_px) * .5f)),
				16.f, tone ? tone : Col(enabled ? Token::TextSecondary : Token::TextDisabled));
		if (has_keys)
		{
			// where MenuItemEx put the shortcut: its column, pushed right by what the menu is wider than its columns.
			// The column's width is last frame's widest shortcut (the mark column always follows it, a spacing apart).
			const float stretch = ImMax(0.f, avail - ImMax(float(columns.TotalWidth), float(columns.NextTotalWidth)));
			const float column_w = float(columns.OffsetMark) - float(columns.OffsetShortcut) - float(columns.Spacing);
			const float inset = ImMax(keys_gap, column_w - keys_w);
			const ImVec2 at(IM_ROUND(pos.x + columns.OffsetShortcut + stretch + inset), IM_ROUND(pos.y + base + (g.FontSize - h) * .5f));
			XUI::Keycaps(window->DrawList, at, keys, h, enabled);
		}
	}
	return pressed;
}
}

bool XUI::MenuItem(cstr label, cstr keys, bool selected, bool enabled)
{
	return MenuItemWith(label, Icon::None, keys, selected, enabled, false);
}

bool XUI::MenuItem(cstr label, Icon icon, cstr keys, bool enabled, bool danger)
{
	return MenuItemWith(label, icon, keys, false, enabled, danger);
}

bool XUI::MenuItem(cstr label, cstr keys, bool* selected, bool enabled)
{
	if (MenuItem(label, keys, selected && *selected, enabled))
	{
		if (selected)
			*selected = !*selected;
		return true;
	}
	return false;
}

void XUI::Internal::TooltipNow(const char* text, const char* keys, const char* title)
{
	if (HoverBlocked())
		return;
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(8.f, 6.f));
	if (ImGui::BeginTooltip())
	{
		// tooltips stay under 360 pt
		ImGui::PushTextWrapPos(Pt(352.f));
		if (title && *title)
		{
			FontScope strong(Font::Strong);
			ImGui::TextUnformatted(title);
		}
		if (text && *text)
		{
			if (title && *title)
				ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
			ImGui::TextUnformatted(text);
			if (title && *title)
				ImGui::PopStyleColor();
		}
		ImGui::PopTextWrapPos();
		if (keys && *keys)
			KeycapsItem(keys);
		ImGui::EndTooltip();
	}
	ImGui::PopStyleVar();
}

void XUI::Tip(cstr text, cstr keys)
{
	if ((!text || !*text) && (!keys || !*keys))
		return;
	if (ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
		TooltipNow(text, keys);
}

bool XUI::TextEllipsis(cstr text, float w, Clip clip)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	if (!text)
		text = "";
	const float avail = w < 0.f ? ImMax(0.f, ImGui::GetContentRegionAvail().x) : w;
	// in whatever face and size the caller has pushed
	xr_string shown;
	const bool cut = EllipsizeWith(ImGui::GetFont(), ImGui::GetFontSize(), text, avail, clip, shown);
	const char* draw = cut ? shown.c_str() : text;
	const ImVec2 size(ImMin(ImGui::CalcTextSize(draw, nullptr, true).x, avail), ImGui::GetTextLineHeight());
	const ImVec2 pos(window->DC.CursorPos.x, window->DC.CursorPos.y + window->DC.CurrLineTextBaseOffset);
	const ImRect bb(pos, pos + size);
	ImGui::ItemSize(size, 0.f);
	if (!ImGui::ItemAdd(bb, 0))
		return cut;
	window->DrawList->AddText(pos, ImGui::GetColorU32(ImGuiCol_Text), draw);
	if (cut)
	{
		AuditAdd("truncated", text, bb, draw);
		if (ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
			TooltipNow(text);
	}
	return cut;
}

bool XUI::TextAt(ImDrawList* draw_list, Font font, const ImVec2& pos, ImU32 col, cstr text, float max_w, Clip clip)
{
	if (!draw_list || !text || !*text)
		return false;
	xr_string shown;
	const bool cut = max_w >= 0.f && Ellipsize(font, text, max_w, clip, shown);
	PaintText(draw_list, font, pos, col, cut ? shown.c_str() : text);
	return cut;
}

float XUI::TextWidth(Font font, cstr text)
{
	return text && *text ? TextSize(font, text).x : 0.f;
}

bool XUI::EllipsizeText(cstr text, float w, Clip clip, xr_string& out)
{
	return EllipsizeWith(ImGui::GetFont(), ImGui::GetFontSize(), text, w, clip, out);
}

bool XUI::EllipsizeFace(ImFont* face, float px, cstr text, float w, Clip clip, xr_string& out)
{
	return EllipsizeWith(face ? face : ImGui::GetFont(), px > 0.f ? px : ImGui::GetFontSize(), text, w, clip, out);
}

void XUI::AuditTruncated(cstr full, cstr shown, const ImVec2& min, const ImVec2& max)
{
	if (full && *full)
		AuditAdd("truncated", full, ImRect(min, max), shown);
}

void XUI::Prose(cstr text)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	// prose wraps at 36 em, or at the column's end when that comes first
	const float wrap = ImMin(window->DC.CursorPos.x + Em(36.f), window->WorkRect.Max.x);
	ImGui::PushTextWrapPos(wrap - window->Pos.x + window->Scroll.x);
	ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
	ImGui::TextUnformatted(text ? text : "");
	ImGui::PopStyleColor();
	ImGui::PopTextWrapPos();
}

void XUI::StatusText(Tone tone, cstr text)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	if (!text)
		text = "";
	const float line = ImGui::GetTextLineHeight();
	const float icon = Pt(14.f);
	ImGui::BeginGroup();
	const ImVec2 pos(window->DC.CursorPos.x, window->DC.CursorPos.y + window->DC.CurrLineTextBaseOffset);
	DrawIcon(window->DrawList, ToneIcon(tone), ImVec2(pos.x, pos.y + (line - icon) * .5f), 14.f, ToneColor(tone));
	ImGui::Dummy(ImVec2(icon, line));
	ImGui::SameLine(0.f, Pt(6.f));
	// prose wraps at 36 em, or at the row's end when that comes first
	const float wrap = ImMin(window->DC.CursorPos.x + Em(36.f), window->WorkRect.Max.x);
	ImGui::PushTextWrapPos(wrap - window->Pos.x + window->Scroll.x);
	ImGui::PushStyleColor(ImGuiCol_Text, tone == Tone::Muted ? Col(Token::TextMuted) : tone == Tone::Info ? Col(Token::Text) : ToneColor(tone));
	ImGui::TextUnformatted(text);
	ImGui::PopStyleColor();
	ImGui::PopTextWrapPos();
	ImGui::EndGroup();
}

// ===================================================================================================
// Buttons
// ===================================================================================================
namespace
{
struct BtnColors
{
	ImU32 bg, text, icon, border;
};

BtnColors ColorsFor(const BtnSpec& s, bool hovered, bool held)
{
	BtnColors c = {};
	const ImU32 label = ImGui::GetColorU32(ImGuiCol_Text);
	if (!s.on)
	{
		c.bg = s.kind == XUI::Kind::Ghost || s.kind == XUI::Kind::GhostDanger ? 0 : Col(Token::Input);
		c.text = c.icon = Col(Token::TextDisabled);
		c.border = s.kind == XUI::Kind::Secondary ? Col(Token::BorderSubtle) : 0;
		return c;
	}
	switch (s.kind)
	{
	case XUI::Kind::Primary:
		c.bg = Col(held ? Token::PrimaryPressed : hovered ? Token::PrimaryHovered : Token::Primary);
		c.text = c.icon = Col(Token::Text);
		break;
	case XUI::Kind::Danger:
		c.bg = Col(held ? Token::DangerPressed : hovered ? Token::DangerHovered : Token::Danger);
		c.text = c.icon = Col(Token::Text);
		break;
	case XUI::Kind::Ghost:
		c.bg = held ? Col(Token::Pressed) : hovered ? Col(Token::Hover) : 0;
		c.text = label;
		c.icon = hovered ? Col(Token::Text) : Col(Token::TextSecondary);
		break;
	case XUI::Kind::GhostDanger:
		c.bg = held ? Col(Token::Pressed) : hovered ? Col(Token::Hover) : 0;
		c.text = c.icon = XUI::ToneColor(XUI::Tone::Error);
		break;
	default:
		c.bg = Col(held ? Token::Pressed : hovered ? Token::ButtonHovered : Token::Button);
		c.text = label;
		c.icon = hovered ? Col(Token::Text) : Col(Token::TextSecondary);
		c.border = Col(Token::Border);
		break;
	}
	if (s.checked)
	{
		c.bg = Col(Token::SelectionFocused);
		c.icon = Col(Token::AccentBright);
		c.text = Col(Token::Text);
	}
	return c;
}

const char* ShownText(const BtnSpec& s, const char** end)
{
	if (s.text)
	{
		*end = s.text + strlen(s.text);
		return s.text;
	}
	*end = LabelEnd(s.id);
	return s.id;
}
}

float XUI::Internal::ButtonWidth(const BtnSpec& s)
{
	const char* end = nullptr;
	const char* text = ShownText(s, &end);
	const float text_w = ImMax(s.text_w, text != end ? TextSize(s.font, text, end).x : 0.f);
	const float icon = s.icon != Icon::None ? Pt(s.icon_pt) : 0.f;
	if (text_w <= 0.f && !s.caret)
		return s.size.x > 0.f ? s.size.x : (s.size.y > 0.f ? s.size.y : ImGui::GetFrameHeight());
	float w = Pt(icon > 0.f ? 8.f : 10.f) + icon + (icon > 0.f && text_w > 0.f ? Pt(6.f) : 0.f) + text_w + Pt(10.f);
	if (s.caret)
		w += Pt(4.f) + Pt(12.f);
	return ImMax(w, s.size.x);
}

bool XUI::Internal::ButtonEx(const BtnSpec& s, ImRect* out_bb, bool* out_hovered)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems)
		return false;

	const ImGuiID id = window->GetID(s.id);
	const float h = s.size.y > 0.f ? s.size.y : ImGui::GetFrameHeight();
	const float w = ButtonWidth(s);
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, pos + ImVec2(w, h));
	if (s.layout)
		ImGui::ItemSize(bb.GetSize(), ImMax(0.f, (h - ImGui::GetFontSize()) * .5f));
	if (!s.on)
		ImGui::PushItemFlag(ImGuiItemFlags_Disabled, true);
	const bool added = ImGui::ItemAdd(bb, id);
	bool hovered = false, held = false, pressed = false;
	if (added)
		pressed = ImGui::ButtonBehavior(bb, id, &hovered, &held);
	if (!s.on)
		ImGui::PopItemFlag();
	if (out_bb)
		*out_bb = bb;
	if (!added)
		return false;
	if (!s.on)
		hovered = ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenDisabled);

	// fill, border, and the mark of the button Enter presses when it is not the loud one anyway
	ImDrawList* dl = window->DrawList;
	const BtnColors c = ColorsFor(s, hovered && s.on, held && s.on);
	const float rounding = Pt(4.f);
	if (c.bg & IM_COL32_A_MASK)
		dl->AddRectFilled(bb.Min, bb.Max, c.bg, rounding, s.corners);
	if (c.border & IM_COL32_A_MASK)
		dl->AddRect(bb.Min, bb.Max, c.border, rounding, s.corners, ImMax(1.f, g.Style.FrameBorderSize));
	// one ring on a button: the keyboard's focus ring where it is, else the default's mark
	const bool nav_ring = g.NavId == id && g.NavCursorVisible;
	if (s.is_default && s.on && s.kind != Kind::Primary && !nav_ring)
		dl->AddRect(bb.Min, bb.Max, Col(Token::AccentBright), rounding, s.corners, ImMax(1.f, Pt(1.f)));
	ImGui::RenderNavCursor(bb, id);

	// icon, text and caret, centred as one block
	const char* end = nullptr;
	const char* text = ShownText(s, &end);
	const bool has_text = text != end;
	const float icon = s.icon != Icon::None ? Pt(s.icon_pt) : 0.f;
	const float text_w = has_text ? TextSize(s.font, text, end).x : 0.f;
	float content = icon + (icon > 0.f && has_text ? Pt(6.f) : 0.f) + text_w + (s.caret ? Pt(4.f) + Pt(12.f) : 0.f);
	float x = bb.Min.x + ImMax(0.f, (w - content) * .5f);
	const float cy = (bb.Min.y + bb.Max.y) * .5f;
	if (icon > 0.f)
	{
		DrawIcon(dl, s.icon, ImVec2(IM_ROUND(x), IM_ROUND(cy - icon * .5f)), s.icon_pt, c.icon);
		x += icon + (has_text ? Pt(6.f) : 0.f);
	}
	if (has_text)
	{
		const ImVec4 clip(bb.Min.x, bb.Min.y, bb.Max.x, bb.Max.y);
		PaintText(dl, s.font, ImVec2(IM_ROUND(x), IM_ROUND(cy - FontPx(s.font) * .5f)), c.text, text, end, 0.f, &clip);
		x += text_w;
	}
	if (s.caret)
		DrawIcon(dl, Icon::ChevronDown, ImVec2(IM_ROUND(x + Pt(4.f)), IM_ROUND(cy - Pt(6.f))), 12.f, c.icon);

	if (hovered && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
	{
		if (!s.on && s.why && *s.why)
			TooltipNow(s.why, nullptr, s.tip_title);
		else if ((s.tip && *s.tip) || (s.tip_title && *s.tip_title) || s.keys)
			TooltipNow(s.tip, s.keys, s.tip_title);
	}
	const char* audit = s.audit_label ? s.audit_label : has_text ? text : s.tip;
	const xr_string label = Str(audit, audit == text ? end : nullptr);
	AuditAdd("control", label.c_str(), bb, nullptr, s.on, s.why, s.tip);
	CheckEscape(bb, label.c_str());
	if (out_hovered)
		*out_hovered = hovered;
	return pressed && s.on;
}

bool XUI::Button(cstr label, Kind kind, bool on, cstr why, const cstr* label_set, int n)
{
	BtnSpec s;
	s.id = label;
	s.kind = kind;
	s.on = on;
	s.why = why;
	s.font = kind == Kind::Primary || kind == Kind::Danger ? Font::Strong : Font::Body;
	for (int i = 0; i < n; ++i)
		if (label_set && label_set[i])
			s.text_w = ImMax(s.text_w, TextSize(s.font, label_set[i], LabelEnd(label_set[i])).x);
	if (InDialogFooter())
	{
		s.size = ImVec2(Pt(88.f), Pt(28.f));
		PlaceFooterButton(ButtonWidth(s));
		if (kind == Kind::Danger)
			MarkFooterDanger();
	}
	return ButtonEx(s);
}

bool XUI::IconButton(cstr id, Icon icon, cstr tip, bool checked, bool on)
{
	BtnSpec s;
	s.id = id;
	s.text = "";
	s.icon = icon;
	s.kind = Kind::Ghost;
	s.tip = tip;
	s.why = on ? nullptr : tip;
	s.checked = checked;
	s.on = on;
	s.size = ImVec2(Pt(24.f), Pt(24.f));
	s.audit_label = tip;
	return ButtonEx(s);
}

bool XUI::SplitButton(cstr id, Icon icon, cstr label, cstr tip, Kind kind)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const ImGuiID popup_id = window->GetID(id);
	ImGui::PushID(id);
	BtnSpec main;
	main.id = "##main";
	main.text = label;
	main.icon = icon;
	main.kind = kind;
	main.tip = tip;
	main.font = kind == Kind::Primary || kind == Kind::Danger ? Font::Strong : Font::Body;
	main.corners = ImDrawFlags_RoundCornersLeft;
	ImRect a;
	const bool pressed = ButtonEx(main, &a);

	// the seam between the halves is the window showing through
	ImGui::SameLine(0.f, ImMax(1.f, Pt(1.f)));
	BtnSpec more;
	more.id = "##more";
	more.text = "";
	more.icon = Icon::ChevronDown;
	more.icon_pt = 12.f;
	more.kind = kind;
	more.tip = "More options";
	more.audit_label = "More options";
	more.checked = ImGui::IsPopupOpen(popup_id, ImGuiPopupFlags_None) && kind != Kind::Primary && kind != Kind::Danger;
	more.size = ImVec2(Pt(20.f), a.GetHeight());
	more.corners = ImDrawFlags_RoundCornersRight;
	ImRect b;
	if (ButtonEx(more, &b))
		OpenPopupAt(popup_id, ImVec2(a.Min.x, b.Max.y + Pt(2.f)));
	ImGui::PopID();
	return pressed;
}

bool XUI::Segmented(cstr id, int* cur, const cstr* labels, int n, const Icon* icons)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems || !cur || !labels || n <= 0)
		return false;

	float seg_w = 0.f;
	for (int i = 0; i < n; ++i)
	{
		const float text = labels[i] ? TextSize(Font::Body, labels[i], LabelEnd(labels[i])).x : 0.f;
		seg_w = ImMax(seg_w, text + (icons && icons[i] != Icon::None ? Pt(16.f) + Pt(6.f) : 0.f) + Pt(20.f));
	}
	const float natural = seg_w * n + Pt(4.f);
	const bool has_width = (g.NextItemData.HasFlags & ImGuiNextItemDataFlags_HasWidth) != 0;
	const float width = has_width ? ImGui::CalcItemWidth() : ImMin(natural, ImMax(0.f, ImGui::GetContentRegionAvail().x));

	ImGui::PushID(id);
	bool changed = false;
	// too narrow for its segments: the same choice as a combo
	if (natural > width + .5f)
	{
		ImGui::SetNextItemWidth(width);
		const bool valid = *cur >= 0 && *cur < n;
		if (ImGui::BeginCombo("##segmented", valid ? labels[*cur] : ""))
		{
			for (int i = 0; i < n; ++i)
				if (ImGui::Selectable(labels[i], i == *cur))
				{
					changed = i != *cur;
					*cur = i;
				}
			ImGui::EndCombo();
		}
		ImGui::PopID();
		return changed;
	}

	const float h = ImGui::GetFrameHeight();
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect frame(pos, pos + ImVec2(width, h));
	ImDrawList* dl = window->DrawList;
	dl->AddRectFilled(frame.Min, frame.Max, Col(Token::Input), Pt(4.f));
	dl->AddRect(frame.Min, frame.Max, Col(Token::Border), Pt(4.f));

	const float inset = Pt(2.f);
	const float each = (width - inset * 2.f) / n;
	ImGui::BeginGroup();
	for (int i = 0; i < n; ++i)
	{
		const ImRect seg(ImVec2(frame.Min.x + inset + each * i, frame.Min.y + inset),
			ImVec2(frame.Min.x + inset + each * (i + 1), frame.Max.y - inset));
		ImGui::SetCursorScreenPos(ImVec2(i ? seg.Min.x : frame.Min.x, frame.Min.y));
		ImGui::ItemSize(ImVec2(i ? seg.GetWidth() : seg.GetWidth() + inset, h), g.Style.FramePadding.y);
		const ImGuiID sid = window->GetID(i);
		if (!ImGui::ItemAdd(seg, sid))
			continue;
		bool hovered = false, held = false;
		if (ImGui::ButtonBehavior(seg, sid, &hovered, &held) && *cur != i)
		{
			*cur = i;
			changed = true;
		}
		const bool on = *cur == i;
		if (on || hovered)
			dl->AddRectFilled(seg.Min, seg.Max, Col(on ? Token::SelectionFocused : Token::Hover), Pt(3.f));
		ImGui::RenderNavCursor(seg, sid);
		const char* label = labels[i] ? labels[i] : "";
		const char* end = LabelEnd(label);
		const float icon = icons && icons[i] != Icon::None ? Pt(16.f) : 0.f;
		const float content = icon + (icon > 0.f ? Pt(6.f) : 0.f) + TextSize(Font::Body, label, end).x;
		float x = seg.Min.x + ImMax(0.f, (seg.GetWidth() - content) * .5f);
		const float cy = seg.GetCenter().y;
		const ImU32 tc = on ? Col(Token::Text) : hovered ? Col(Token::Text) : Col(Token::TextSecondary);
		if (icon > 0.f)
		{
			DrawIcon(dl, icons[i], ImVec2(IM_ROUND(x), IM_ROUND(cy - icon * .5f)), 16.f, on ? Col(Token::AccentBright) : tc);
			x += icon + Pt(6.f);
		}
		const ImVec4 clip(seg.Min.x, seg.Min.y, seg.Max.x, seg.Max.y);
		PaintText(dl, Font::Body, ImVec2(IM_ROUND(x), IM_ROUND(cy - FontPx(Font::Body) * .5f)), tc, label, end, 0.f, &clip);
		AuditAdd("control", Str(label, end).c_str(), seg, nullptr, true, nullptr, nullptr);
	}
	ImGui::EndGroup();
	CheckEscape(frame, id);
	ImGui::PopID();
	return changed;
}

// ===================================================================================================
// Fields
// ===================================================================================================
namespace
{
int s_CtrlFFrame = -1;

float FieldWidth(float min_w)
{
	ImGuiContext& g = *GImGui;
	if (g.NextItemData.HasFlags & ImGuiNextItemDataFlags_HasWidth)
		return ImGui::CalcItemWidth();
	return ImMax(min_w, ImGui::GetContentRegionAvail().x);
}
}

bool XUI::SearchField(cstr id, char* buf, size_t n, cstr hint, float min_em)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems || !buf || !n)
		return false;
	const float width = FieldWidth(Em(min_em));
	ImGui::PushID(id);

	// Ctrl+F goes to the first search field of the focused panel
	if (s_CtrlFFrame != ImGui::GetFrameCount() && ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows) &&
		ImGui::IsKeyChordPressed(ImGuiMod_Ctrl | ImGuiKey_F))
	{
		s_CtrlFFrame = ImGui::GetFrameCount();
		ImGui::SetKeyboardFocusHere();
	}

	// the padding hosts the magnifier on the left and the clear cross on the right
	const float side = Pt(8.f) + Pt(14.f) + Pt(6.f);
	ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(side, ImGui::GetStyle().FramePadding.y));
	ImGui::SetNextItemWidth(width);
	ImGui::SetNextItemAllowOverlap();
	bool changed = ImGui::InputTextWithHint("##search", hint ? hint : "Search", buf, n, ImGuiInputTextFlags_EscapeClearsAll);
	ImGui::PopStyleVar();
	const ImRect bb(ImGui::GetItemRectMin(), ImGui::GetItemRectMax());
	const bool active = ImGui::IsItemActive();
	const float cy = bb.GetCenter().y;
	DrawIcon(window->DrawList, Icon::Search, ImVec2(IM_ROUND(bb.Min.x + Pt(8.f)), IM_ROUND(cy - Pt(7.f))), 14.f,
		Col(active ? Token::Text : Token::TextMuted));
	AuditAdd("control", hint ? hint : "Search", bb, buf, true, nullptr, "Esc clears, Ctrl+F focuses");
	CheckEscape(bb, hint ? hint : "Search");

	if (buf[0])
	{
		const ImVec2 backup = window->DC.CursorPos;
		const float size = Pt(20.f);
		ImGui::SetCursorScreenPos(ImVec2(bb.Max.x - size - Pt(2.f), cy - size * .5f));
		BtnSpec clear;
		clear.id = "##clear";
		clear.text = "";
		clear.icon = Icon::Clear;
		clear.icon_pt = 12.f;
		clear.kind = Kind::Ghost;
		clear.tip = "Clear";
		clear.size = ImVec2(size, size);
		clear.layout = false;
		if (ButtonEx(clear))
		{
			buf[0] = 0;
			changed = true;
		}
		window->DC.CursorPos = backup;
	}
	ImGui::PopID();
	return changed;
}

namespace
{
unsigned Fold(unsigned c)
{
	if (c >= 'A' && c <= 'Z')
		return c + 32;
	if (c < 0x80)
		return c;
	if (c >= 0xC0 && c <= 0xDE && c != 0xD7)
		return c + 32;
	if (c >= 0x410 && c <= 0x42F)
		return c + 32;
	if (c >= 0x400 && c <= 0x40F)
		c += 80;
	// yo is written as ye as often as not
	if (c == 0x451)
		return 0x435;
	if (c == 0x490)
		return 0x491;
	return c;
}

void FoldUtf8(const char* text, xr_string& out)
{
	out.clear();
	const char* end = text + strlen(text);
	for (const char* p = text; p < end;)
	{
		unsigned c = 0;
		p += ImMax(1, ImTextCharFromUtf8(&c, p, end));
		char utf8[5];
		out.append(utf8, ImTextCharToUtf8(utf8, Fold(c)));
	}
}
}

bool XUI::Match(cstr query, cstr text)
{
	if (!query)
		return true;
	while (*query == ' ' || *query == '\t')
		++query;
	if (!*query)
		return true;
	if (!text)
		return false;

	// the query repeats across every row of a list: fold it once
	static xr_string s_Query, s_Folded, s_Text;
	static xr_vector<std::pair<size_t, size_t>> s_Terms;
	if (s_Query != query)
	{
		s_Query = query;
		FoldUtf8(query, s_Folded);
		s_Terms.clear();
		for (size_t i = 0; i < s_Folded.size();)
		{
			while (i < s_Folded.size() && (s_Folded[i] == ' ' || s_Folded[i] == '\t'))
				++i;
			const size_t start = i;
			while (i < s_Folded.size() && s_Folded[i] != ' ' && s_Folded[i] != '\t')
				++i;
			if (i > start)
				s_Terms.emplace_back(start, i - start);
		}
	}
	FoldUtf8(text, s_Text);
	for (const auto& [start, length] : s_Terms)
		if (s_Text.find(s_Folded.c_str() + start, 0, length) == xr_string::npos)
			return false;
	return true;
}

namespace
{
bool BrowseNative(bool folder, char* buf, size_t n)
{
	if (XUI::Unattended() || XUI::FromMcp())
		return false;
	const HRESULT com = ::CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE);
	bool picked = false;
	IFileOpenDialog* dialog = nullptr;
	if (SUCCEEDED(::CoCreateInstance(CLSID_FileOpenDialog, nullptr, CLSCTX_INPROC_SERVER, IID_PPV_ARGS(&dialog))))
	{
		DWORD options = 0;
		dialog->GetOptions(&options);
		dialog->SetOptions(options | FOS_FORCEFILESYSTEM | FOS_NOCHANGEDIR | (folder ? FOS_PICKFOLDERS : FOS_FILEMUSTEXIST));
		// start where the field points: the folder itself, or the folder of the file
		if (buf[0])
		{
			wchar_t path[MAX_PATH * 2] = {};
			if (::MultiByteToWideChar(CP_UTF8, 0, buf, -1, path, int(sizeof(path) / sizeof(path[0]))))
			{
				if (!folder)
					if (wchar_t* slash = wcsrchr(path, L'\\'))
						*slash = 0;
				IShellItem* start = nullptr;
				if (SUCCEEDED(::SHCreateItemFromParsingName(path, nullptr, IID_PPV_ARGS(&start))))
				{
					dialog->SetFolder(start);
					start->Release();
				}
			}
		}
		const HWND owner = (HWND)ImGui::GetMainViewport()->PlatformHandleRaw;
		IShellItem* result = nullptr;
		if (SUCCEEDED(dialog->Show(owner)) && SUCCEEDED(dialog->GetResult(&result)))
		{
			PWSTR path = nullptr;
			if (SUCCEEDED(result->GetDisplayName(SIGDN_FILESYSPATH, &path)))
			{
				char utf8[MAX_PATH * 4];
				const int bytes = ::WideCharToMultiByte(CP_UTF8, 0, path, -1, utf8, int(sizeof(utf8)), nullptr, nullptr);
				if (bytes > 0 && size_t(bytes) <= n)
				{
					memcpy(buf, utf8, size_t(bytes));
					picked = true;
				}
				else if (bytes > 0)
					Msg("! The picked path is longer than the field takes: %s", utf8);
				::CoTaskMemFree(path);
			}
			result->Release();
		}
		dialog->Release();
	}
	if (SUCCEEDED(com))
		::CoUninitialize();
	return picked;
}
}

bool XUI::BrowsePath(char* buf, size_t n, bool folder)
{
	return buf && n && BrowseNative(folder, buf, n);
}

bool XUI::PathField(cstr id, char* buf, size_t n, bool folder)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems || !buf || !n)
		return false;
	const float button = ImGui::GetFrameHeight();
	const float spacing = g.Style.ItemInnerSpacing.x;
	const float width = ImMax(Em(4.f), FieldWidth(Em(12.f)) - button - spacing);

	ImGui::PushID(id);
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID editing_key = ImGui::GetID("##editing");
	const ImGuiID focus_key = ImGui::GetID("##focus");
	bool changed = false;
	if (storage->GetBool(editing_key))
	{
		if (storage->GetBool(focus_key))
		{
			storage->SetBool(focus_key, false);
			ImGui::SetKeyboardFocusHere();
		}
		ImGui::SetNextItemWidth(width);
		changed = ImGui::InputText("##path", buf, n, ImGuiInputTextFlags_AutoSelectAll);
		if (ImGui::IsItemDeactivated())
			storage->SetBool(editing_key, false);
	}
	else
	{
		// a path shows its end: the start is what the ellipsis drops
		const ImVec2 pos = window->DC.CursorPos;
		const ImRect bb(pos, pos + ImVec2(width, button));
		ImGui::ItemSize(bb.GetSize(), g.Style.FramePadding.y);
		const ImGuiID view = ImGui::GetID("##view");
		if (ImGui::ItemAdd(bb, view))
		{
			bool hovered = false, held = false;
			if (ImGui::ButtonBehavior(bb, view, &hovered, &held))
			{
				storage->SetBool(editing_key, true);
				storage->SetBool(focus_key, true);
			}
			ImGui::RenderFrame(bb.Min, bb.Max, Col(hovered ? Token::InputHovered : Token::Input), true, g.Style.FrameRounding);
			ImGui::RenderNavCursor(bb, view);
			const float room = bb.GetWidth() - g.Style.FramePadding.x * 2.f;
			const ImVec2 text_pos(bb.Min.x + g.Style.FramePadding.x, bb.Min.y + g.Style.FramePadding.y);
			if (buf[0])
			{
				xr_string shown;
				const bool cut = Ellipsize(Font::Body, buf, room, Clip::Start, shown);
				PaintText(window->DrawList, Font::Body, text_pos, Col(Token::Text), cut ? shown.c_str() : buf);
				if (cut)
					AuditAdd("truncated", buf, bb, shown.c_str());
				if (hovered)
					Tip(buf);
			}
			else
				PaintText(window->DrawList, Font::Body, text_pos, Col(Token::TextMuted), folder ? "No folder selected" : "No file selected");
		}
	}
	ImGui::SameLine(0.f, spacing);
	if (IconButton("##browse", folder ? Icon::FolderOpen : Icon::Open, folder ? "Browse for a folder" : "Browse for a file"))
		changed |= BrowseNative(folder, buf, n);
	ImGui::PopID();
	return changed;
}

namespace
{
bool s_RowMixed = false;
}

int XUI::PickField(cstr id, cstr value, bool clearable)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems)
		return PickNone;
	const float h = ImGui::GetFrameHeight();
	const float width = FieldWidth(Em(6.f));
	ImGui::PushID(id);
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, pos + ImVec2(width, h));
	ImGui::ItemSize(bb.GetSize(), g.Style.FramePadding.y);
	const ImGuiID fid = ImGui::GetID("##pick");
	int result = PickNone;
	if (ImGui::ItemAdd(bb, fid))
	{
		bool hovered = false, held = false;
		if (ImGui::ButtonBehavior(bb, fid, &hovered, &held, ImGuiButtonFlags_AllowOverlap))
			result = PickBrowse;
		ImGui::RenderFrame(bb.Min, bb.Max, Col(hovered ? Token::InputHovered : Token::Input), true, g.Style.FrameRounding);
		ImGui::RenderNavCursor(bb, fid);

		const bool has_value = value && *value;
		const float chevron = Pt(20.f);
		const float clear = clearable && has_value ? Pt(20.f) : 0.f;
		const float room = bb.GetWidth() - g.Style.FramePadding.x * 2.f - chevron - clear;
		const ImVec2 text_pos(bb.Min.x + g.Style.FramePadding.x, bb.Min.y + g.Style.FramePadding.y);
		if (s_RowMixed)
			PaintText(window->DrawList, Font::Body, text_pos, Col(Token::TextMuted), "Multiple values");
		else if (has_value)
		{
			// the leaf names the thing; the tooltip has all of it
			const char* leaf = value;
			for (const char* p = value; *p; ++p)
				if (*p == '\\' || *p == '/')
					leaf = p + 1;
			if (!*leaf)
				leaf = value;
			xr_string shown;
			const bool cut = Ellipsize(Font::Body, leaf, room, Clip::Start, shown);
			PaintText(window->DrawList, Font::Body, text_pos, Col(Token::Text), cut ? shown.c_str() : leaf);
			if (cut)
				AuditAdd("truncated", value, bb, shown.c_str());
			if (hovered)
				Tip(value);
		}
		else
			PaintText(window->DrawList, Font::Body, text_pos, Col(Token::TextMuted), "None");
		DrawIcon(window->DrawList, Icon::ChevronDown, ImVec2(IM_ROUND(bb.Max.x - chevron + Pt(4.f)), IM_ROUND(bb.GetCenter().y - Pt(6.f))), 12.f,
			Col(hovered ? Token::Text : Token::TextSecondary));
		AuditAdd("control", id, bb, value, true, nullptr, nullptr);
		CheckEscape(bb, id);

		if (clear > 0.f)
		{
			const ImVec2 backup = window->DC.CursorPos;
			ImGui::SetCursorScreenPos(ImVec2(bb.Max.x - chevron - clear, bb.Min.y + (h - clear) * .5f));
			BtnSpec s;
			s.id = "##clear";
			s.text = "";
			s.icon = Icon::Clear;
			s.icon_pt = 12.f;
			s.kind = Kind::Ghost;
			s.tip = "Clear";
			s.size = ImVec2(clear, clear);
			s.layout = false;
			if (ButtonEx(s))
				result = PickClear;
			window->DC.CursorPos = backup;
		}
	}
	ImGui::PopID();
	return result;
}

bool XUI::Vec3Field(cstr id, float v[3], float speed)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems || !v)
		return false;
	static const char* kAxis[] = { "X", "Y", "Z" };
	static const Token kTone[] = { Token::AxisX, Token::AxisY, Token::AxisZ };

	const float width = FieldWidth(Em(9.f));
	// three numbers side by side need 15 em; narrower, they stack
	const bool stacked = width < Em(15.f);
	const float gap = Pt(4.f);
	const float each = stacked ? width : (width - gap * 2.f) / 3.f;
	const float tag = Pt(18.f);
	const float h = ImGui::GetFrameHeight();
	const float rounding = g.Style.FrameRounding;

	bool changed = false;
	ImGui::PushID(id);
	ImGui::BeginGroup();
	for (int i = 0; i < 3; ++i)
	{
		if (i && !stacked)
			ImGui::SameLine(0.f, gap);
		const ImVec2 pos = window->DC.CursorPos;
		// the drag starts under the tag, so its rounded left corners hide beneath it
		ImGui::SetCursorScreenPos(ImVec2(pos.x + tag - rounding, pos.y));
		ImGui::SetNextItemWidth(each - tag + rounding);
		ImGui::PushID(i);
		changed |= ImGui::DragFloat("##v", &v[i], speed, 0.f, 0.f, "%.3f");
		ImGui::PopID();
		const ImRect bb(pos, ImVec2(pos.x + tag, pos.y + h));
		window->DrawList->AddRectFilled(bb.Min, bb.Max, Col(Token::Raised), rounding, ImDrawFlags_RoundCornersLeft);
		window->DrawList->AddRectFilled(bb.Min, bb.Max, Col(kTone[i], .22f), rounding, ImDrawFlags_RoundCornersLeft);
		const ImVec2 letter = TextSize(Font::Strong, kAxis[i]);
		PaintText(window->DrawList, Font::Strong, ImVec2(IM_ROUND(bb.GetCenter().x - letter.x * .5f), IM_ROUND(bb.GetCenter().y - FontPx(Font::Strong) * .5f)),
			Col(kTone[i]), kAxis[i]);
	}
	ImGui::EndGroup();
	ImGui::PopID();
	return changed;
}

int XUI::Chips(cstr id, const Strings& items, bool removable)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems)
		return -1;
	int removed = -1;
	const float h = Pt(22.f);
	const float right = window->WorkRect.Max.x;
	float last_max = -FLT_MAX;
	ImGui::PushID(id);
	ImGui::BeginGroup();
	for (int i = 0; i < int(items.size()); ++i)
	{
		const char* text = items[i].c_str();
		const float text_w = TextSize(Font::Small, text).x;
		const float close = removable ? Pt(16.f) : 0.f;
		const float w = Pt(8.f) + text_w + (removable ? Pt(4.f) + close : 0.f) + Pt(8.f);
		// wrap before a chip that would pass the row's end
		if (i > 0 && last_max + Pt(4.f) + w <= right)
			ImGui::SameLine(0.f, Pt(4.f));
		const ImVec2 pos = window->DC.CursorPos;
		const ImRect bb(pos, pos + ImVec2(w, h));
		ImGui::ItemSize(bb.GetSize());
		last_max = bb.Max.x;
		if (!ImGui::ItemAdd(bb, 0))
			continue;
		window->DrawList->AddRectFilled(bb.Min, bb.Max, Col(Token::Raised), h * .5f);
		window->DrawList->AddRect(bb.Min, bb.Max, Col(Token::BorderSubtle), h * .5f);
		PaintText(window->DrawList, Font::Small, ImVec2(bb.Min.x + Pt(8.f), IM_ROUND(bb.GetCenter().y - FontPx(Font::Small) * .5f)),
			Col(Token::Text), text);
		if (removable)
		{
			const ImVec2 backup = window->DC.CursorPos;
			ImGui::SetCursorScreenPos(ImVec2(bb.Max.x - Pt(6.f) - close, bb.GetCenter().y - close * .5f));
			ImGui::PushID(i);
			BtnSpec s;
			s.id = "##remove";
			s.text = "";
			s.icon = Icon::Close;
			s.icon_pt = 10.f;
			s.kind = Kind::Ghost;
			s.tip = "Remove";
			s.size = ImVec2(close, close);
			s.layout = false;
			if (ButtonEx(s))
				removed = i;
			ImGui::PopID();
			window->DC.CursorPos = backup;
		}
	}
	ImGui::EndGroup();
	ImGui::PopID();
	return removed;
}

void XUI::BlockLabel(cstr label, cstr help)
{
	ImGui::SetCursorPosX(ImGui::GetCursorPosX() + Pt(6.f));
	ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
	TextEllipsis(label);
	ImGui::PopStyleColor();
	Tip(help);
}

bool XUI::TextBlock(cstr label, cstr help, cstr id, char* buf, size_t n, int lines)
{
	BlockLabel(label, help);
	const float h = ImGui::GetTextLineHeight() * float(ImMax(1, lines)) + ImGui::GetStyle().FramePadding.y * 2.f;
	const bool changed = ImGui::InputTextMultiline(id, buf, n, ImVec2(ImMax(1.f, ImGui::GetContentRegionAvail().x), h));
	const char* shown = LabelEnd(label);
	AuditAdd("control", Str(label, shown).c_str(), ImRect(ImGui::GetItemRectMin(), ImGui::GetItemRectMax()), nullptr, true, nullptr, help);
	ImGui::Dummy(ImVec2(0.f, Pt(6.f)));
	return changed;
}

void XUI::Internal::SetRowMixed(bool mixed)
{
	s_RowMixed = mixed;
}

// ===================================================================================================
// Layout store: [XFinedLayout][Data] in the .ini, sizes in pt; [XFinedLayout][Dpi] the scale they
// were saved at, so a layout moved to another monitor keeps its panels' pt size.
// ===================================================================================================
namespace
{
xr_map<xr_string, float> s_Layout;
float s_LayoutDpi = 0.f;

void* LayoutReadOpen(ImGuiContext*, ImGuiSettingsHandler*, const char* name)
{
	if (!strcmp(name, "Data"))
		return (void*)1;
	if (!strcmp(name, "Dpi"))
		return (void*)2;
	return nullptr;
}

void LayoutReadLine(ImGuiContext*, ImGuiSettingsHandler*, void* entry, const char* line)
{
	const char* eq = strrchr(line, '=');
	float value = 0.f;
	if (!eq || eq == line || sscanf_s(eq + 1, "%f", &value) != 1)
		return;
	if (entry == (void*)2)
	{
		if (eq - line == 5 && !strncmp(line, "Scale", 5))
			s_LayoutDpi = value;
		return;
	}
	s_Layout[Str(line, eq)] = value;
}

void LayoutApplyAll(ImGuiContext*, ImGuiSettingsHandler*)
{
	if (s_LayoutDpi > 0.f && fabsf(s_LayoutDpi - s_Dpi) > .001f)
		RescaleLayout(s_Dpi / s_LayoutDpi);
	s_LayoutDpi = 0.f;
}

void LayoutWriteAll(ImGuiContext*, ImGuiSettingsHandler* handler, ImGuiTextBuffer* buf)
{
	buf->appendf("[%s][Dpi]\nScale=%.3f\n\n", handler->TypeName, s_Dpi);
	buf->appendf("[%s][Data]\n", handler->TypeName);
	for (const auto& [key, value] : s_Layout)
		buf->appendf("%s=%g\n", key.c_str(), value);
	buf->append("\n");
}

void LayoutClearAll(ImGuiContext*, ImGuiSettingsHandler*)
{
	s_Layout.clear();
}

bool FloatingWindow(const ImGuiWindow* window)
{
	const ImGuiWindowFlags skip = ImGuiWindowFlags_ChildWindow | ImGuiWindowFlags_Popup | ImGuiWindowFlags_Tooltip |
		ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoResize | ImGuiWindowFlags_DockNodeHost;
	return !(window->Flags & skip) && !window->DockIsActive && !window->DockNode && !window->IsFallbackWindow;
}
}

void XUI::Internal::RegisterLayoutHandler()
{
	ImGuiSettingsHandler handler;
	handler.TypeName = "XFinedLayout";
	handler.TypeHash = ImHashStr("XFinedLayout");
	handler.ClearAllFn = LayoutClearAll;
	handler.ReadOpenFn = LayoutReadOpen;
	handler.ReadLineFn = LayoutReadLine;
	handler.ApplyAllFn = LayoutApplyAll;
	handler.WriteAllFn = LayoutWriteAll;
	ImGui::AddSettingsHandler(&handler);
}

void XUI::Internal::RescaleLayout(float ratio)
{
	if (!(ratio > 0.f) || ratio == 1.f || !ImGui::GetCurrentContext())
		return;
	ImGuiContext& g = *GImGui;
	bool changed = false;
	// docked side nodes keep their pt size; the central node takes what they give up (dock rule 3)
	for (ImGuiStoragePair& pair : g.DockContext.Nodes.Data)
	{
		ImGuiDockNode* node = (ImGuiDockNode*)pair.val_p;
		if (!node)
			continue;
		const ImGuiDockNode* root = node;
		while (root->ParentNode)
			root = root->ParentNode;
		if (!(root->LocalFlags & ImGuiDockNodeFlags_DockSpace) && !root->IsDockSpace())
			continue;
		node->SizeRef = node->SizeRef * ratio;
		node->Size = node->Size * ratio;
		changed = true;
	}
	// floating windows too, alive or still only in the .ini
	for (ImGuiWindow* window : g.Windows)
		if (FloatingWindow(window))
		{
			ImGui::SetWindowSize(window, window->SizeFull * ratio, ImGuiCond_None);
			changed = true;
		}
	for (ImGuiWindowSettings* settings = g.SettingsWindows.begin(); settings; settings = g.SettingsWindows.next_chunk(settings))
		if (!settings->DockId)
		{
			settings->Size = ImVec2ih(short(settings->Size.x * ratio), short(settings->Size.y * ratio));
			changed = true;
		}
	if (changed)
		ImGui::MarkIniSettingsDirty();
}

float XUI::Layout::Get(cstr key, float def)
{
	if (!key)
		return def;
	const auto it = s_Layout.find(key);
	return it != s_Layout.end() ? it->second : def;
}

void XUI::Layout::Set(cstr key, float value)
{
	if (!key || !*key || strchr(key, '\n') || strchr(key, '='))
		return;
	float& slot = s_Layout[key];
	if (slot == value)
		return;
	slot = value;
	if (ImGui::GetCurrentContext())
		ImGui::MarkIniSettingsDirty();
}

bool XUI::Layout::Has(cstr key)
{
	return key && s_Layout.count(key);
}

void XUI::Layout::ResetAll()
{
	s_Layout.clear();
	if (ImGui::GetCurrentContext())
		ImGui::MarkIniSettingsDirty();
}

void XUI::Layout::Keys(Strings& out)
{
	out.clear();
	for (const auto& entry : s_Layout)
		out.push_back(entry.first);
}

// ===================================================================================================
// Surfaces, scroll and popup requests
// ===================================================================================================
namespace
{
struct Surface
{
	xr_string id, label;
	int kind = 0;
	std::function<void(int)> control;
	std::function<bool()> is_open;
};
xr_vector<Surface> s_Surfaces;

struct Request
{
	xr_string surface, target;
	int age = 0;
};
Request s_Scroll;
Request s_Popup;
xr_string s_ScrollItem;

// the panels being drawn, innermost last: the index of the surface each belongs to, or -1
xr_vector<int> s_PanelStack;
// the surface an "item:<name>" scroll waits in
int s_ScrollItemSurface = -1;

Surface* FindSurface(const char* id)
{
	if (!id)
		return nullptr;
	for (Surface& s : s_Surfaces)
		if (s.id == id)
			return &s;
	return nullptr;
}
}

void XUI::RegisterSurface(const SurfaceDesc& desc)
{
	if (!desc.id || !*desc.id)
		return;
	Surface* s = FindSurface(desc.id);
	if (!s)
		s = &s_Surfaces.emplace_back();
	s->id = desc.id;
	s->label = desc.label ? desc.label : "";
	s->kind = desc.kind;
	s->control = desc.control;
	s->is_open = desc.is_open;
}

void XUI::Surfaces(Strings& ids)
{
	ids.clear();
	for (const Surface& s : s_Surfaces)
		ids.push_back(s.id);
}

bool XUI::SurfaceControl(cstr id, int op)
{
	Surface* s = FindSurface(id);
	if (!s || !s->control)
		return false;
	s->control(op);
	return true;
}

bool XUI::SurfaceIsOpen(cstr id)
{
	const Surface* s = FindSurface(id);
	return s && s->is_open && s->is_open();
}

bool XUI::SurfaceInfo(cstr id, xr_string* label, int* kind)
{
	const Surface* s = FindSurface(id);
	if (!s)
		return false;
	if (label)
		*label = s->label;
	if (kind)
		*kind = s->kind;
	return true;
}

void XUI::RequestScroll(cstr surface, cstr target)
{
	s_Scroll.surface = surface ? surface : "";
	s_Scroll.target = target ? target : "";
	s_Scroll.age = 0;
	s_ScrollItem.clear();
}

void XUI::ShowPopup(cstr name)
{
	s_Popup.target = name ? name : "";
	s_Popup.age = 0;
}

bool XUI::Internal::TakePopupRequest(const char* name)
{
	if (s_Popup.target.empty() || !name)
		return false;
	// "toolbar_overflow" is taken by any toolbar that overflows, "toolbar_overflow:<id>" by that one
	const size_t colon = s_Popup.target.find(':');
	const bool hit = s_Popup.target == name ||
		(colon == xr_string::npos && !strncmp(name, s_Popup.target.c_str(), s_Popup.target.size()) && name[s_Popup.target.size()] == ':');
	if (hit)
		s_Popup.target.clear();
	return hit;
}

bool XUI::Internal::PeekPopupRequest(const char* prefix)
{
	return prefix && !s_Popup.target.empty() && !s_Popup.target.compare(0, strlen(prefix), prefix);
}

void XUI::Internal::OpenPopupAt(ImGuiID id, const ImVec2& pos)
{
	ImGuiContext& g = *GImGui;
	ImGui::OpenPopupEx(id, ImGuiPopupFlags_None);
	const int level = g.BeginPopupStack.Size;
	if (level < g.OpenPopupStack.Size && g.OpenPopupStack[level].PopupId == id)
		g.OpenPopupStack[level].OpenPopupPos = pos;
}

void XUI::Internal::PanelBegun(const char* title_id, bool visible)
{
	int index = -1;
	for (int i = 0; title_id && i < int(s_Surfaces.size()); ++i)
		if (s_Surfaces[i].label == title_id)
			index = i;
	s_PanelStack.push_back(index);
	if (!visible || index < 0 || s_Scroll.target.empty() || s_Scroll.surface != s_Surfaces[index].id)
		return;
	const xr_string target = s_Scroll.target;
	s_Scroll.target.clear();
	if (target == "top")
		ImGui::SetScrollY(0.f);
	else if (target == "bottom")
		ImGui::SetScrollY(ImGui::GetScrollMaxY());
	else if (!target.compare(0, 5, "item:"))
	{
		s_ScrollItem = target.substr(5);
		s_ScrollItemSurface = index;
	}
	else
	{
		float pt = 0.f;
		if (sscanf_s(target.c_str(), "%f", &pt) == 1)
			ImGui::SetScrollY(Pt(pt));
	}
}

void XUI::Internal::PanelEnded()
{
	if (!s_PanelStack.empty())
		s_PanelStack.pop_back();
}

void XUI::Internal::ScrollToItem(const char* name, const ImRect& bb)
{
	if (s_ScrollItem.empty() || !name || s_ScrollItem != name || s_PanelStack.empty() || s_PanelStack.back() != s_ScrollItemSurface)
		return;
	// the parents as well: a list that flows in a scrolling body cannot scroll by itself
	ImGui::ScrollToRectEx(ImGui::GetCurrentWindow(), bb, ImGuiScrollFlags_KeepVisibleCenterY);
	s_ScrollItem.clear();
}

void XUI::ScrollTarget(cstr name)
{
	Internal::ScrollToItem(name, ImRect(ImGui::GetItemRectMin(), ImGui::GetItemRectMax()));
}

XUI::cstr XUI::ScrollTargetWanted()
{
	if (s_ScrollItem.empty() || s_PanelStack.empty() || s_PanelStack.back() != s_ScrollItemSurface)
		return nullptr;
	return s_ScrollItem.c_str();
}

bool XUI::BeginPanel(cstr title_id, bool* open, int flags)
{
	const bool visible = ImGui::Begin(title_id, open, flags | ImGuiWindowFlags_NoCollapse);
	PanelBegun(title_id, visible);
	return visible;
}

void XUI::EndPanel()
{
	PanelEnded();
	ImGui::End();
}

xr_string XUI::ShortcutLabel(const xr_shortcut& shortcut)
{
	return ShortcutText(shortcut);
}

// ===================================================================================================
// Audit
// ===================================================================================================
namespace
{
xr_vector<XUI::AuditEntry> s_AuditCur, s_AuditLast;
xr_vector<xr_string> s_AuditHidden;
constexpr size_t kAuditMax = 8192;

void AuditPush(const char* window, const char* kind, const char* label, const ImRect& bb, const char* detail, bool enabled,
	const char* why, const char* help);

void ScanWindows()
{
	ImGuiContext& g = *GImGui;
	char detail[64];
	for (ImGuiWindow* w : g.Windows)
	{
		if (!w->Active || w->Hidden || w->IsFallbackWindow || (w->Flags & (ImGuiWindowFlags_Tooltip | ImGuiWindowFlags_AlwaysAutoResize)))
			continue;
		// a child of a window nobody sees this frame (a dialog measuring itself) is not on screen either
		if (w->RootWindow && w->RootWindow != w && w->RootWindow->Hidden)
			continue;
		const float room_x = w->InnerRect.GetWidth() - w->WindowPadding.x * 2.f;
		const float room_y = w->InnerRect.GetHeight() - w->WindowPadding.y * 2.f;
		const ImRect bb(w->Pos, w->Pos + w->Size);
		// what THIS frame laid out, not ContentSize: ImGui fills that at Begin from the frame before, so a window that had
		// just changed size (a dialog fitting itself once it has measured) read as escaped by the height it had before
		const ImVec2 content(w->ContentSizeExplicit.x != 0.f ? w->ContentSizeExplicit.x : IM_TRUNC(w->DC.CursorMaxPos.x - w->DC.CursorStartPos.x),
			w->ContentSizeExplicit.y != 0.f ? w->ContentSizeExplicit.y : IM_TRUNC(w->DC.CursorMaxPos.y - w->DC.CursorStartPos.y));
		if (!w->ScrollbarX && room_x > 0.f && content.x > room_x + 1.5f)
		{
			sprintf_s(detail, "x by %.0f px", content.x - room_x);
			AuditPush(w->Name, "escaped", w->Name, bb, detail, true, nullptr, nullptr);
		}
		if (!w->ScrollbarY && room_y > 0.f && content.y > room_y + 1.5f)
		{
			sprintf_s(detail, "y by %.0f px", content.y - room_y);
			AuditPush(w->Name, "escaped", w->Name, bb, detail, true, nullptr, nullptr);
		}
	}
}

void JsonString(xr_string& out, const char* text)
{
	out += '"';
	for (const char* p = text ? text : ""; *p; ++p)
	{
		const unsigned char c = (unsigned char)*p;
		if (c == '"' || c == '\\')
		{
			out += '\\';
			out += char(c);
		}
		else if (c < 0x20)
		{
			char esc[8];
			sprintf_s(esc, "\\u%04x", c);
			out += esc;
		}
		else
			out += char(c);
	}
	out += '"';
}

bool AuditMatches(const XUI::AuditEntry& e, const char* filter)
{
	if (!filter || !*filter)
		return true;
	if (const Surface* s = FindSurface(filter))
		if (!s->label.empty() && !e.window.compare(0, s->label.size(), s->label))
			return true;
	return e.window.find(filter) != xr_string::npos;
}
}

void XUI::Internal::AuditAdd(const char* kind, const char* label, const ImRect& bb, const char* detail, bool enabled,
	const char* why, const char* help)
{
	if (!ImGui::GetCurrentContext())
		return;
	ImGuiWindow* window = GImGui->CurrentWindow;
	AuditPush(window ? window->Name : "", kind, label, bb, detail, enabled, why, help);
}

namespace
{
void AuditPush(const char* window, const char* kind, const char* label, const ImRect& bb, const char* detail, bool enabled,
	const char* why, const char* help)
{
	if (s_AuditCur.size() >= kAuditMax)
		return;
	XUI::AuditEntry& e = s_AuditCur.emplace_back();
	e.window = window;
	e.kind = kind;
	if (label)
		e.label.assign(label, LabelEnd(label));
	if (detail)
		e.detail = detail;
	if (why)
		e.why = why;
	if (help)
		e.help = help;
	e.enabled = enabled;
	const ImVec2 origin = ImGui::GetMainViewport()->Pos;
	e.rect = ImVec4(bb.Min.x - origin.x, bb.Min.y - origin.y, bb.GetWidth(), bb.GetHeight());
}
}

void XUI::Internal::CheckEscape(const ImRect& bb, const char* label)
{
	ImGuiWindow* window = GImGui->CurrentWindow;
	if (!window || window->ScrollbarX || (window->Flags & ImGuiWindowFlags_HorizontalScrollbar))
		return;
	const float past = bb.Max.x - window->InnerRect.Max.x;
	if (past > 1.f)
	{
		char detail[48];
		sprintf_s(detail, "x by %.0f px", past);
		AuditAdd("escaped", label, bb, detail);
	}
}

void XUI::Internal::AuditHideWindow(const char* name)
{
	if (name && *name)
		s_AuditHidden.push_back(name);
}

void XUI::Internal::AuditFrameEnd(bool keep)
{
	if (keep)
	{
		// a child's name starts with its parent's and a '/'
		for (const xr_string& hidden : s_AuditHidden)
			s_AuditCur.erase(std::remove_if(s_AuditCur.begin(), s_AuditCur.end(), [&hidden](const XUI::AuditEntry& e)
			{
				return !e.window.compare(0, hidden.size(), hidden) && (e.window.size() == hidden.size() || e.window[hidden.size()] == '/');
			}), s_AuditCur.end());
		ScanWindows();
		s_AuditLast.swap(s_AuditCur);
	}
	s_AuditCur.clear();
	s_AuditHidden.clear();
}

void XUI::AuditSnapshot(xr_vector<AuditEntry>& out, cstr window)
{
	out.clear();
	for (const AuditEntry& e : s_AuditLast)
		if (AuditMatches(e, window))
			out.push_back(e);
}

void XUI::AuditJson(xr_string& out, cstr window)
{
	static const char* kKinds[] = { "truncated", "escaped", "control" };
	static const char* kKeys[] = { "truncated", "escaped", "controls" };
	out = "{";
	for (int k = 0; k < 3; ++k)
	{
		if (k)
			out += ',';
		out += '"';
		out += kKeys[k];
		out += "\":[";
		bool first = true;
		for (const AuditEntry& e : s_AuditLast)
		{
			if (e.kind != kKinds[k] || !AuditMatches(e, window))
				continue;
			if (!first)
				out += ',';
			first = false;
			out += "{\"window\":";
			JsonString(out, e.window.c_str());
			out += ",\"label\":";
			JsonString(out, e.label.c_str());
			if (k == 2)
			{
				out += e.enabled ? ",\"enabled\":true" : ",\"enabled\":false";
				out += ",\"why\":";
				JsonString(out, e.why.c_str());
				out += ",\"help\":";
				JsonString(out, e.help.c_str());
			}
			else
			{
				out += k == 0 ? ",\"shown\":" : ",\"by\":";
				JsonString(out, e.detail.c_str());
			}
			char rect[96];
			sprintf_s(rect, ",\"rect\":[%.0f,%.0f,%.0f,%.0f]}", e.rect.x, e.rect.y, e.rect.z, e.rect.w);
			out += rect;
		}
		out += ']';
	}
	char tail[48];
	sprintf_s(tail, ",\"dpi\":%.3f}", s_Dpi);
	out += tail;
}

// ===================================================================================================
// The end of a frame
// ===================================================================================================
void XUI::DrawOverlays()
{
	if (!ImGui::GetCurrentContext() || !GImGui->WithinFrameScope)
		return;
	// a progress frame draws no panels, so a dialog opened now could close a modal nobody drew
	if (!s_ProgressFrame)
	{
		DrawIconSheetWindow();
		DrawQueuedDialogs();
	}
	DrawToasts();
	// a request its owner did not take within a few frames is dropped
	if (!s_Popup.target.empty() && ++s_Popup.age > 3)
		s_Popup.target.clear();
	if (!s_Scroll.target.empty() && ++s_Scroll.age > 3)
		s_Scroll.target.clear();
	if (!s_ScrollItem.empty() && ++s_Scroll.age > 6)
		s_ScrollItem.clear();
	AuditFrameEnd(!s_ProgressFrame);
}
