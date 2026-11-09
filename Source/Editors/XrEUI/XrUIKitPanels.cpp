#define IMGUI_DEFINE_MATH_OPERATORS
#include "stdafx.h"
#include "imgui_internal.h"
#include "XrUIKitInternal.h"

using namespace XUI::Internal;

// ===================================================================================================
// Property grid
// ===================================================================================================
namespace
{
struct Grid
{
	bool active = false;
	xr_string key;			// grid.<role>
	float x0 = 0.f, x1 = 0.f;
	float label_w = 0.f;
	float default_frac = .40f;
	bool stacked = false;
	float top = 0.f;
	// the row being drawn
	bool row_open = false;
	ImRect row;
	ImGuiID row_id = 0;
	float value_x = 0.f;
	int trailing = 0;		// slots used so far this frame
	int slots = 0;			// slots reserved (last frame's count)
	bool mixed = false;
	// the hovered row's help, for HelpFooter
	xr_string hovered_label, hovered_help;
};
Grid s_Grid;
xr_string s_FooterLabel, s_FooterHelp;
int s_FooterFrame = -10;

constexpr float kLabelGap = 8.f;	// pt between the label column and the value
constexpr float kSlot = 24.f;		// pt of a trailing icon button

void CloseRow()
{
	Grid& g = s_Grid;
	if (!g.row_open)
		return;
	g.row_open = false;
	if (g.mixed)
	{
		ImGui::PopItemFlag();
		SetRowMixed(false);
		g.mixed = false;
	}
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	window->StateStorage.SetInt(g.row_id, g.trailing);
	// rows keep their 26 pt rhythm; a taller value (a message, a stacked field) pushes the next one down
	const float bottom = ImMax(g.row.Max.y, window->DC.CursorPos.y - (window->DC.IsSameLine ? 0.f : ImGui::GetStyle().ItemSpacing.y));
	// what it took, for the frames it is scrolled out of view: the rows under it do not jump
	window->StateStorage.SetFloat(g.row_id + 1, bottom - g.row.Min.y);
	ImGui::SetCursorScreenPos(ImVec2(g.x0, bottom));
}
}

bool XUI::BeginPropertyGrid(cstr role, float label_frac)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	Grid& g = s_Grid;
	g = Grid();
	g.key = "grid.";
	g.key += role ? role : "default";
	g.default_frac = label_frac;
	if (window->SkipItems)
		return false;
	const float frac = ImClamp(Layout::Get(g.key.c_str(), label_frac), .1f, .9f);
	g.x0 = window->DC.CursorPos.x;
	const float width = ImMax(0.f, ImGui::GetContentRegionAvail().x);
	g.x1 = g.x0 + width;
	// below 18 em the label goes above its value
	g.stacked = width < Em(18.f);
	g.label_w = g.stacked ? width : ImClamp(width * frac, Em(6.f), ImMax(Em(6.f), width - Em(8.f)));
	g.top = window->DC.CursorPos.y;
	g.active = true;
	ImGui::PushID(g.key.c_str());
	return true;
}

void XUI::EndPropertyGrid()
{
	Grid& g = s_Grid;
	if (!g.active)
		return;
	CloseRow();
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	const float bottom = window->DC.CursorPos.y;

	// the label column's edge is a grip: its ratio is stored per role
	if (!g.stacked && bottom > g.top)
	{
		const float edge = g.x0 + g.label_w + Pt(kLabelGap) * .5f;
		const ImRect hit(ImVec2(edge - Pt(3.f), g.top), ImVec2(edge + Pt(3.f), bottom));
		const ImGuiID id = window->GetID("##label_edge");
		if (ImGui::ItemAdd(hit, id, nullptr, ImGuiItemFlags_NoNav))
		{
			bool hovered = false, held = false;
			ImGui::ButtonBehavior(hit, id, &hovered, &held);
			if (hovered || held)
				ImGui::SetMouseCursor(ImGuiMouseCursor_ResizeEW);
			const float width = g.x1 - g.x0;
			if (held && width > 0.f)
			{
				const float label = ImClamp(ImGui::GetIO().MousePos.x - g.x0 - Pt(kLabelGap) * .5f, Em(6.f), ImMax(Em(6.f), width - Em(8.f)));
				Layout::Set(g.key.c_str(), label / width);
			}
			if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
				Layout::Set(g.key.c_str(), g.default_frac);
			if (hovered || held)
				window->DrawList->AddLine(ImVec2(edge, g.top), ImVec2(edge, bottom), Col(held ? Token::Accent : Token::AccentBright, held ? 1.f : .6f),
					ImMax(1.f, Pt(1.f)));
		}
	}
	// several grids may share one footer (two panes of a panel): the one hovered this frame is what it shows
	if (!g.hovered_label.empty() || !g.hovered_help.empty())
	{
		s_FooterLabel = g.hovered_label;
		s_FooterHelp = g.hovered_help;
		s_FooterFrame = ImGui::GetFrameCount();
	}
	ImGui::Dummy(ImVec2(0.f, 0.f));
	ImGui::PopID();
	g.active = false;
}

bool XUI::PropertySection(cstr title, bool open, cstr badge)
{
	CloseRow();
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	Grid& g = s_Grid;
	const float x0 = g.active ? g.x0 : window->DC.CursorPos.x;
	const float x1 = g.active ? g.x1 : window->WorkRect.Max.x;
	const float h = Pt(28.f);
	const ImVec2 pos(x0, window->DC.CursorPos.y);
	const ImRect bb(pos, ImVec2(x1, pos.y + h));
	const ImGuiID id = window->GetID(title);
	ImGuiStorage* storage = ImGui::GetStateStorage();
	bool is_open = storage->GetBool(id, open);
	ImGui::ItemSize(bb.GetSize());
	ScrollToItem(title, bb);
	ImGui::SetNextItemAllowOverlap();
	if (!ImGui::ItemAdd(bb, id))
		return is_open;
	bool hovered = false, held = false;
	if (ImGui::ButtonBehavior(bb, id, &hovered, &held))
	{
		is_open = !is_open;
		storage->SetBool(id, is_open);
	}
	ImDrawList* dl = window->DrawList;
	dl->AddRectFilled(bb.Min, bb.Max, Col(hovered ? Token::Hover : Token::Raised));
	ImGui::RenderNavCursor(bb, id);
	const float cy = bb.GetCenter().y;
	DrawIcon(dl, is_open ? Icon::ChevronDown : Icon::ChevronRight, ImVec2(IM_ROUND(bb.Min.x + Pt(6.f)), IM_ROUND(cy - Pt(6.f))), 12.f,
		Col(Token::TextSecondary));
	float right = bb.Max.x - Pt(8.f);
	if (badge && *badge)
	{
		const float w = TextSize(Font::Small, badge).x + Pt(12.f);
		const ImRect pill(ImVec2(right - w, cy - Pt(9.f)), ImVec2(right, cy + Pt(9.f)));
		dl->AddRectFilled(pill.Min, pill.Max, Col(Token::Base), Pt(9.f));
		PaintText(dl, Font::Small, ImVec2(pill.Min.x + Pt(6.f), IM_ROUND(cy - FontPx(Font::Small) * .5f)), Col(Token::TextSecondary), badge);
		right = pill.Min.x - Pt(6.f);
	}
	const float text_x = bb.Min.x + Pt(24.f);
	xr_string shown;
	const char* end = LabelEnd(title);
	const xr_string label = Str(title, end);
	const bool cut = Ellipsize(Font::Strong, label.c_str(), right - text_x, Clip::End, shown);
	PaintText(dl, Font::Strong, ImVec2(IM_ROUND(text_x), IM_ROUND(cy - FontPx(Font::Strong) * .5f)), Col(Token::Text), cut ? shown.c_str() : label.c_str());
	if (cut)
		AuditAdd("truncated", label.c_str(), bb, shown.c_str());
	AuditAdd("control", label.c_str(), bb, nullptr, true, nullptr, nullptr);
	if (g.active)
		ImGui::SetCursorScreenPos(ImVec2(g.x0, bb.Max.y));
	return is_open;
}

void XUI::EndPropertySection()
{
	CloseRow();
}

bool XUI::PropertyRow(cstr label, cstr help, int flags, int depth)
{
	CloseRow();
	Grid& g = s_Grid;
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems || !g.active)
		return false;
	if (!label)
		label = "";
	const float frame = ImGui::GetFrameHeight();
	const float row_h = Pt(26.f);
	// a switch stays on its label's line when labels go above values: the box first, the label after it
	const bool inline_value = g.stacked && (flags & PropInline);
	const float label_h = g.stacked && !inline_value ? ImGui::GetTextLineHeight() + Pt(2.f) : 0.f;
	const float y = window->DC.CursorPos.y;
	g.row = ImRect(ImVec2(g.x0, y), ImVec2(g.x1, y + label_h + row_h));
	g.row_id = ImHashData(&depth, sizeof(depth), window->GetID(label));
	g.slots = ImMax(window->StateStorage.GetInt(g.row_id, 0), (flags & PropModified) ? 1 : 0);
	g.trailing = 0;
	ScrollToItem(label, g.row);

	// off screen: keep the layout - the height the row took when it was drawn, a tall value included - and skip the work.
	// Never in a window that sizes itself to its content: every row is on screen there once it has, and a row skipped
	// while it grows holds its height back a frame per tall row (a tab opened half drawn)
	const float drawn_h = ImMax(g.row.GetHeight(), window->StateStorage.GetFloat(g.row_id + 1, 0.f));
	const bool fits_content = (window->Flags & ImGuiWindowFlags_AlwaysAutoResize) || (window->ChildFlags & ImGuiChildFlags_AutoResizeY);
	if (!fits_content && !ImGui::IsRectVisible(g.row.Min, ImVec2(g.row.Max.x, g.row.Min.y + drawn_h)))
	{
		ImGui::Dummy(ImVec2(g.row.GetWidth(), drawn_h));
		ImGui::SetCursorScreenPos(ImVec2(g.x0, g.row.Min.y + drawn_h));
		return false;
	}

	ImDrawList* dl = window->DrawList;
	const bool hovered = ImGui::IsMouseHoveringRect(g.row.Min, g.row.Max) && ImGui::IsWindowHovered(ImGuiHoveredFlags_AllowWhenBlockedByActiveItem);
	if (hovered)
	{
		g.hovered_label = Str(label, LabelEnd(label));
		g.hovered_help = help ? help : "";
	}
	if (flags & PropInvalid)
		dl->AddRectFilled(ImVec2(g.x0, g.row.Min.y + Pt(3.f)), ImVec2(g.x0 + Pt(2.f), g.row.Max.y - Pt(3.f)), Col(Token::Error));

	// the label: secondary text, cut with an ellipsis; its tooltip names it and says what it does
	const float indent = Pt(12.f) * depth + Pt(6.f);
	const float trailing_w = g.slots * (Pt(kSlot) + Pt(2.f));
	const float label_x = g.x0 + indent + (inline_value ? frame + ImGui::GetStyle().ItemInnerSpacing.x : 0.f);
	const float label_right = inline_value ? g.x1 - trailing_w : g.stacked ? g.x1 : g.x0 + g.label_w;
	const float dot = (flags & PropRequired) ? Pt(10.f) : 0.f;
	const float label_y = g.stacked && !inline_value ? y : y + (row_h - FontPx(Font::Body)) * .5f;
	const xr_string text = Str(label, LabelEnd(label));
	xr_string shown;
	const bool cut = Ellipsize(Font::Body, text.c_str(), ImMax(0.f, label_right - label_x - dot), Clip::End, shown);
	const char* drawn = cut ? shown.c_str() : text.c_str();
	PaintText(dl, Font::Body, ImVec2(IM_ROUND(label_x), IM_ROUND(label_y)), Col((flags & PropReadOnly) ? Token::TextMuted : Token::TextSecondary), drawn);
	const ImRect label_bb(ImVec2(label_x, label_y), ImVec2(label_right, label_y + FontPx(Font::Body)));
	if (flags & PropRequired)
	{
		const float dot_x = label_x + TextSize(Font::Body, drawn).x + Pt(5.f);
		dl->AddCircleFilled(ImVec2(dot_x, label_bb.GetCenter().y), Pt(2.5f), Col(Token::Required));
	}
	if (cut)
		AuditAdd("truncated", text.c_str(), label_bb, shown.c_str());
	// an item of its own, so its tooltip waits like every other one
	if (ImGui::ItemAdd(label_bb, 0) && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
		TooltipNow(help, nullptr, text.c_str());
	AuditAdd("control", text.c_str(), g.row, nullptr, !(flags & PropReadOnly), nullptr, help);

	// the value: the rest of the row, less the trailing icons
	g.value_x = g.stacked ? g.x0 + indent : g.x0 + g.label_w + Pt(kLabelGap);
	const float value_w = inline_value ? frame : ImMax(Em(3.f), g.x1 - g.value_x - trailing_w - Pt(2.f));
	const float value_y = g.stacked && !inline_value ? y + label_h + (row_h - frame) * .5f : y + (row_h - frame) * .5f;
	ImGui::SetCursorScreenPos(ImVec2(g.value_x, value_y));
	ImGui::SetNextItemWidth(value_w);
	if (flags & PropMixed)
	{
		ImGui::PushItemFlag(ImGuiItemFlags_MixedValue, true);
		SetRowMixed(true);
		g.mixed = true;
	}
	g.row_open = true;
	return true;
}

void XUI::PropertyText(cstr text)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems)
		return;
	if (!text)
		text = "";
	// the rows of a grid share one id stack: a value takes its row's id, or every value of the grid is one widget
	struct SRowScope
	{
		const bool on;
		SRowScope(bool row, ImGuiID row_id) : on(row) { if (on) ImGui::PushOverrideID(row_id); }
		~SRowScope() { if (on) ImGui::PopID(); }
	} row_scope(s_Grid.row_open, s_Grid.row_id);
	// the first line only; a longer value is in the tooltip
	const char* eol = strchr(text, '\n');
	const xr_string line = Str(text, eol);
	const float width = ImGui::CalcItemWidth();
	const float h = ImGui::GetFrameHeight();
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, pos + ImVec2(width, h));
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID id = window->GetID("##property_text");
	const ImGuiID select_key = id + 1;
	const bool fits = TextSize(Font::Body, line.c_str()).x <= width - g.Style.FramePadding.x * 2.f && !eol;

	// selectable: a read-only field once clicked, or at once when the value fits
	if (fits || storage->GetBool(select_key))
	{
		static xr_vector<char> s_Buffer;
		s_Buffer.assign(text, text + strlen(text) + 1);
		ImGui::PushStyleColor(ImGuiCol_FrameBg, IM_COL32(0, 0, 0, 0));
		ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
		ImGui::PushStyleVar(ImGuiStyleVar_FrameBorderSize, 0.f);
		ImGui::SetNextItemWidth(width);
		if (!fits && storage->GetBool(select_key + 1))
		{
			storage->SetBool(select_key + 1, false);
			ImGui::SetKeyboardFocusHere();
		}
		ImGui::InputText("##property_text", s_Buffer.data(), s_Buffer.size(), ImGuiInputTextFlags_ReadOnly);
		if (!fits && ImGui::IsItemDeactivated())
			storage->SetBool(select_key, false);
		ImGui::PopStyleVar();
		ImGui::PopStyleColor(2);
		return;
	}
	ImGui::ItemSize(bb.GetSize(), g.Style.FramePadding.y);
	if (!ImGui::ItemAdd(bb, id))
		return;
	bool hovered = false, held = false;
	if (ImGui::ButtonBehavior(bb, id, &hovered, &held))
	{
		storage->SetBool(select_key, true);
		storage->SetBool(select_key + 1, true);
	}
	xr_string shown;
	Ellipsize(Font::Body, line.c_str(), width - g.Style.FramePadding.x * 2.f, Clip::End, shown);
	PaintText(window->DrawList, Font::Body, ImVec2(bb.Min.x + g.Style.FramePadding.x, bb.Min.y + g.Style.FramePadding.y), Col(Token::TextSecondary),
		shown.empty() ? line.c_str() : shown.c_str());
	AuditAdd("truncated", text, bb, shown.c_str());
	if (hovered)
		Tip(text);
}

bool XUI::PropertyTrailing(cstr id, Icon icon, cstr tip)
{
	Grid& g = s_Grid;
	if (!g.row_open)
		return IconButton(id, icon, tip);
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	const int slot = g.trailing++;
	const float size = Pt(kSlot);
	const float x = g.x1 - (slot + 1) * (size + Pt(2.f));
	const ImVec2 backup = window->DC.CursorPos;
	const float cy = g.stacked ? g.row.Max.y - Pt(13.f) : g.row.GetCenter().y;
	ImGui::SetCursorScreenPos(ImVec2(x, cy - size * .5f));
	BtnSpec s;
	s.id = id;
	s.text = "";
	s.icon = icon;
	s.icon_pt = 14.f;
	s.kind = Kind::Ghost;
	s.tip = tip;
	s.size = ImVec2(size, size);
	s.layout = false;
	s.audit_label = tip;
	const bool pressed = ButtonEx(s);
	window->DC.CursorPos = backup;
	return pressed;
}

void XUI::PropertyMessage(Tone tone, cstr text)
{
	Grid& g = s_Grid;
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems || !text)
		return;
	const float x = g.row_open ? g.value_x : window->DC.CursorPos.x;
	const float right = g.row_open ? g.x1 : window->WorkRect.Max.x;
	const float y = g.row_open ? ImMax(window->DC.CursorPos.y, g.row.Max.y - Pt(2.f)) : window->DC.CursorPos.y;
	ImGui::SetCursorScreenPos(ImVec2(x, y));
	FontScope font(Font::Small);
	ImGui::PushTextWrapPos(right - window->Pos.x + window->Scroll.x);
	ImGui::PushStyleColor(ImGuiCol_Text, tone == Tone::Muted ? Col(Token::TextMuted) : ToneColor(tone));
	ImGui::TextUnformatted(text);
	ImGui::PopStyleColor();
	ImGui::PopTextWrapPos();
	if (g.row_open)
		g.row.Max.y = ImMax(g.row.Max.y, window->DC.CursorPos.y);
}

void XUI::HelpFooter()
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	const float line = FontPx(Font::Small);
	const float h = Pt(8.f) * 2.f + line * 2.f + Pt(2.f);
	const ImVec2 pos = window->DC.CursorPos;
	const float right = window->WorkRect.Max.x;
	const ImRect bb(pos, ImVec2(right, pos.y + h));
	ImGui::ItemSize(bb.GetSize());
	if (!ImGui::ItemAdd(bb, 0))
		return;
	ImDrawList* dl = window->DrawList;
	dl->AddLine(bb.Min, ImVec2(bb.Max.x, bb.Min.y), Col(Token::BorderSubtle));
	// with no row hovered the band says what it is for, so the panel does not jump and is never blank; a footer drawn
	// before its grid reads the frame before
	const bool hovered = !(s_FooterHelp.empty() && s_FooterLabel.empty()) && s_FooterFrame >= ImGui::GetFrameCount() - 1;
	const ImVec4 clip(bb.Min.x, bb.Min.y, bb.Max.x, bb.Max.y - Pt(6.f));
	const xr_string text = !hovered ? xr_string("Point at a property to see what it does.") : s_FooterHelp.empty() ? s_FooterLabel : s_FooterHelp;
	PaintText(dl, Font::Small, ImVec2(bb.Min.x + Pt(4.f), bb.Min.y + Pt(8.f)), Col(Token::TextMuted), text.c_str(), nullptr,
		bb.GetWidth() - Pt(8.f), &clip);
}

// ===================================================================================================
// Sections, lists
// ===================================================================================================
namespace
{
// the row being drawn: RowActions sit on it, the next row starts under it
struct List
{
	bool row_open = false;
	ImRect row;
};
List s_List;
}

bool XUI::HeaderAction(cstr id, Icon icon, cstr tip)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const ImVec2 max = ImGui::GetItemRectMax(), min = ImGui::GetItemRectMin();
	const ImVec2 backup = window->DC.CursorPos;
	const float side = Pt(22.f);
	ImGui::SetCursorScreenPos(ImVec2(max.x - Pt(4.f) - side, (min.y + max.y - side) * .5f));
	BtnSpec s;
	s.id = id;
	s.text = "";
	s.icon = icon;
	s.icon_pt = 14.f;
	s.kind = Kind::Ghost;
	s.tip = tip;
	s.size = ImVec2(side, side);
	s.layout = false;
	const bool pressed = ButtonEx(s);
	window->DC.CursorPos = backup;
	window->DC.IsSetPos = false;
	return pressed;
}

bool XUI::Section(cstr title, bool open, cstr right)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const float h = Pt(28.f);
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, ImVec2(window->WorkRect.Max.x, pos.y + h));
	const ImGuiID id = window->GetID(title);
	ImGuiStorage* storage = ImGui::GetStateStorage();
	bool is_open = storage->GetBool(id, open);
	ImGui::ItemSize(bb.GetSize());
	ScrollToItem(title, bb);
	ImGui::SetNextItemAllowOverlap();
	if (!ImGui::ItemAdd(bb, id))
		return is_open;
	bool hovered = false, held = false;
	if (ImGui::ButtonBehavior(bb, id, &hovered, &held))
	{
		is_open = !is_open;
		storage->SetBool(id, is_open);
	}
	ImDrawList* dl = window->DrawList;
	dl->AddRectFilled(bb.Min, bb.Max, Col(hovered ? Token::Hover : Token::Raised), Pt(4.f));
	ImGui::RenderNavCursor(bb, id);
	const float cy = bb.GetCenter().y;
	DrawIcon(dl, is_open ? Icon::ChevronDown : Icon::ChevronRight, ImVec2(IM_ROUND(bb.Min.x + Pt(6.f)), IM_ROUND(cy - Pt(6.f))), 12.f,
		Col(Token::TextSecondary));
	const xr_string label = Str(title, LabelEnd(title));
	float text_right = bb.Max.x - Pt(8.f);
	if (right && *right)
	{
		xr_string shown;
		const float room = ImMax(Em(4.f), bb.GetWidth() * .4f);
		const bool cut = Ellipsize(Font::Small, right, room, Clip::End, shown);
		const char* drawn = cut ? shown.c_str() : right;
		const float w = TextSize(Font::Small, drawn).x;
		PaintText(dl, Font::Small, ImVec2(IM_ROUND(bb.Max.x - Pt(8.f) - w), IM_ROUND(cy - FontPx(Font::Small) * .5f)), Col(Token::TextMuted), drawn);
		text_right -= w + Pt(12.f);
	}
	xr_string shown;
	const float text_x = bb.Min.x + Pt(24.f);
	const bool cut = Ellipsize(Font::Strong, label.c_str(), text_right - text_x, Clip::End, shown);
	PaintText(dl, Font::Strong, ImVec2(IM_ROUND(text_x), IM_ROUND(cy - FontPx(Font::Strong) * .5f)), Col(Token::Text), cut ? shown.c_str() : label.c_str());
	if (cut)
		AuditAdd("truncated", label.c_str(), bb, shown.c_str());
	AuditAdd("control", label.c_str(), bb, nullptr, true, nullptr, right);
	if (is_open)
		ImGui::Dummy(ImVec2(0.f, Pt(2.f)));
	return is_open;
}

void XUI::EndSection()
{
	ImGui::Dummy(ImVec2(0.f, Pt(4.f)));
}

bool XUI::BeginList(cstr id, ImVec2 size_pt)
{
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	auto axis = [](float pt, float room) { return pt > 0.f ? Pt(pt) : pt < 0.f ? ImMax(1.f, room + Pt(pt)) : 0.f; };
	const ImVec2 size(axis(size_pt.x, avail.x), axis(size_pt.y, avail.y));
	s_List = List();
	ImGui::PushStyleColor(ImGuiCol_ChildBg, Col(Token::Inset));
	ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderSubtle));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(4.f, 4.f));
	const bool visible = ImGui::BeginChild(id, size, ImGuiChildFlags_Borders | ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleVar();
	ImGui::PopStyleColor(2);
	// rows touch: their 22 pt is the rhythm
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(ImGui::GetStyle().ItemSpacing.x, 0.f));
	return visible;
}

namespace
{
void CloseListRow()
{
	if (!s_List.row_open)
		return;
	s_List.row_open = false;
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	// nothing was drawn on the row: leave its line
	if (window->DC.CursorPos.y < s_List.row.Max.y - .5f)
		ImGui::NewLine();
}
}

bool XUI::ListRow(cstr stable_id, bool selected)
{
	CloseListRow();
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const float h = Pt(22.f);
	const ImVec2 pos = window->DC.CursorPos;
	const ImRect bb(pos, ImVec2(window->WorkRect.Max.x, pos.y + h));
	const ImGuiID id = window->GetID(stable_id);
	ImGui::ItemSize(bb.GetSize(), ImMax(0.f, (h - ImGui::GetFontSize()) * .5f));
	s_List.row = bb;
	ScrollToItem(stable_id, bb);
	if (!ImGui::ItemAdd(bb, id))
		return false;
	bool hovered = false, held = false;
	const bool pressed = ImGui::ButtonBehavior(bb, id, &hovered, &held, ImGuiButtonFlags_AllowOverlap);
	const bool focused = ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows);
	ImDrawList* dl = window->DrawList;
	if (selected)
		dl->AddRectFilled(bb.Min, bb.Max, Col(focused ? Token::SelectionFocused : Token::Selection), Pt(3.f));
	else if (hovered || (ImGui::IsMouseHoveringRect(bb.Min, bb.Max) && ImGui::IsWindowHovered()))
		dl->AddRectFilled(bb.Min, bb.Max, Col(held ? Token::Pressed : Token::Hover), Pt(3.f));
	ImGui::RenderNavCursor(bb, id);
	// the row's content follows on its line, inset from the edge
	ImGui::SameLine(0.f, 0.f);
	window->DC.CursorPos.x = bb.Min.x + Pt(6.f);
	s_List.row_open = true;
	return pressed;
}

void XUI::EndList()
{
	CloseListRow();
	ImGui::PopStyleVar();
	ImGui::EndChild();
	s_List = List();
}

int XUI::RowActions(const Icon* icons, const cstr* tips, int n)
{
	if (!s_List.row_open || !icons || n <= 0)
		return -1;
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	const ImRect row = s_List.row;
	if (!(ImGui::IsMouseHoveringRect(row.Min, row.Max) && ImGui::IsWindowHovered(ImGuiHoveredFlags_AllowWhenBlockedByActiveItem)))
		return -1;
	const float size = row.GetHeight();
	const ImVec2 backup = window->DC.CursorPos;
	int clicked = -1;
	for (int i = 0; i < n; ++i)
	{
		const float x = row.Max.x - (n - i) * (size + Pt(2.f));
		window->DC.CursorPos = ImVec2(x, row.Min.y);
		ImGui::PushID(i);
		BtnSpec s;
		s.id = "##row_action";
		s.text = "";
		s.icon = icons[i];
		s.icon_pt = 14.f;
		s.kind = Kind::Ghost;
		s.tip = tips ? tips[i] : nullptr;
		s.size = ImVec2(size, size);
		s.layout = false;
		s.audit_label = s.tip;
		if (ButtonEx(s))
			clicked = i;
		ImGui::PopID();
	}
	window->DC.CursorPos = backup;
	return clicked;
}

bool XUI::NavRow(cstr stable_id, cstr label, bool selected, const NavRowSpec& spec, bool* action_clicked)
{
	if (action_clicked)
		*action_clicked = false;
	CloseListRow();
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const float h = Pt(22.f);
	const ImVec2 pos = window->DC.CursorPos;
	const float right = ImMax(pos.x + h * 2.f, window->WorkRect.Max.x);
	const ImRect bb(pos, ImVec2(right, pos.y + h));
	const ImGuiID id = window->GetID(stable_id);
	ImGui::ItemSize(bb.GetSize(), ImMax(0.f, (h - ImGui::GetFontSize()) * .5f));
	ScrollToItem(stable_id, bb);
	if (!ImGui::ItemAdd(bb, id))
		return false;
	bool hovered = false, held = false;
	const bool pressed = ImGui::ButtonBehavior(bb, id, &hovered, &held, ImGuiButtonFlags_AllowOverlap);
	if (spec.tip && *spec.tip)
		Tip(spec.tip);
	const bool row_hovered = ImGui::IsMouseHoveringRect(bb.Min, bb.Max) && ImGui::IsWindowHovered(ImGuiHoveredFlags_AllowWhenBlockedByActiveItem);
	const bool focused = ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows);
	ImDrawList* dl = window->DrawList;
	const float r = Pt(3.f);
	if (selected)
		dl->AddRectFilled(bb.Min, bb.Max, Col(focused ? Token::SelectionFocused : Token::Selection), r);
	else if (hovered || row_hovered)
		dl->AddRectFilled(bb.Min, bb.Max, Col(held ? Token::Pressed : Token::Hover), r);
	ImGui::RenderNavCursor(bb, id);

	// the right end: the hover action, else a dot or a count
	float text_right = bb.Max.x - Pt(6.f);
	const bool show_action = spec.action != Icon::None && row_hovered;
	if (show_action)
		text_right = bb.Max.x - h - Pt(2.f);
	else if (spec.dot)
	{
		dl->AddCircleFilled(ImVec2(bb.Max.x - Pt(10.f), bb.GetCenter().y), Pt(3.5f), ToneColor(spec.tone));
		text_right = bb.Max.x - Pt(18.f);
	}
	else if (spec.count && *spec.count)
	{
		const float cw = TextSize(Font::Small, spec.count).x;
		PaintText(dl, Font::Small, ImVec2(IM_ROUND(bb.Max.x - Pt(8.f) - cw), IM_ROUND(bb.GetCenter().y - FontPx(Font::Small) * .5f)),
			Col(Token::TextMuted), spec.count);
		text_right = bb.Max.x - Pt(8.f) - cw - Pt(8.f);
	}

	float x = bb.Min.x + Pt(6.f);
	if (spec.icon != Icon::None)
	{
		const float icon = Pt(16.f);
		DrawIcon(dl, spec.icon, ImVec2(IM_ROUND(x), IM_ROUND(bb.GetCenter().y - icon * .5f)), 16.f,
			Col(selected ? Token::AccentBright : spec.dim ? Token::TextDisabled : Token::TextSecondary));
		x += icon + Pt(6.f);
	}
	const xr_string text = Str(label, LabelEnd(label));
	xr_string shown;
	const bool cut = Ellipsize(Font::Body, text.c_str(), ImMax(0.f, text_right - x), Clip::End, shown);
	const ImVec2 text_pos(IM_ROUND(x), IM_ROUND(bb.GetCenter().y - FontPx(Font::Body) * .5f));
	PaintText(dl, Font::Body, text_pos, Col(spec.dim ? Token::TextDisabled : Token::Text), cut ? shown.c_str() : text.c_str());
	if (cut)
	{
		AuditAdd("truncated", text.c_str(), ImRect(text_pos, ImVec2(text_right, bb.Max.y)), shown.c_str());
		// the row is still the last item here: the action button comes after
		if (!(spec.tip && *spec.tip) && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
			TooltipNow(text.c_str());
	}
	if (show_action)
	{
		const ImVec2 backup = window->DC.CursorPos;
		window->DC.CursorPos = ImVec2(bb.Max.x - h, bb.Min.y);
		ImGui::PushID(stable_id);
		BtnSpec s;
		s.id = "##nav_action";
		s.text = "";
		s.icon = spec.action;
		s.icon_pt = 12.f;
		s.kind = Kind::Ghost;
		s.tip = spec.action_tip;
		s.size = ImVec2(h, h);
		s.layout = false;
		s.audit_label = spec.action_tip;
		if (ButtonEx(s) && action_clicked)
			*action_clicked = true;
		ImGui::PopID();
		window->DC.CursorPos = backup;
	}
	AuditAdd("control", text.c_str(), bb, nullptr, true, nullptr, spec.tip);
	return pressed;
}

void XUI::TableHeaders(u32 centred)
{
	ImGuiContext& g = *GImGui;
	ImGuiTable* table = g.CurrentTable;
	if (!table)
		return;
	FontScope caption_font(Font::Small);
	ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextSecondary));
	const float row_h = Pt(24.f);
	ImGui::TableNextRow(ImGuiTableRowFlags_Headers, row_h);
	// the names sit in the middle of the row's height
	const float drop = ImMax(0.f, (row_h - table->RowCellPaddingY * 2.f - g.FontSize) * .5f);
	for (int column = 0; column < table->ColumnsCount; ++column)
	{
		if (!ImGui::TableSetColumnIndex(column))
			continue;
		const char* name = ImGui::TableGetColumnName(column);
		const char* end = LabelEnd(name);
		const float w = name ? ImGui::CalcTextSize(name, end).x : 0.f;
		const float room = ImGui::GetContentRegionAvail().x;
		ImGui::SetCursorPosY(ImGui::GetCursorPosY() + drop);
		if (column < 32 && (centred & (1u << column)))
			ImGui::SetCursorPosX(ImGui::GetCursorPosX() + ImMax(0.f, (room - w) * .5f));
		const ImVec2 at = ImGui::GetCursorScreenPos();
		ImGui::PushID(column);
		ImGui::TableHeader(name);
		ImGui::PopID();
		if (name && name != end && w > room + .5f)
			AuditAdd("truncated", Str(name, end).c_str(), ImRect(at, ImVec2(at.x + room, at.y + g.FontSize)), nullptr);
	}
	ImGui::PopStyleColor();
}

// ===================================================================================================
// Splitter
// ===================================================================================================
bool XUI::Splitter(cstr key, bool vertical, float* size_pt, float min_a, float min_b, float def)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	const float stored = Layout::Get(key, def);
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float gap = vertical ? g.Style.ItemSpacing.x : g.Style.ItemSpacing.y;
	const float total_pt = (vertical ? avail.x : avail.y) / Dpi();
	const float max_a = ImMax(min_a, total_pt - gap / Dpi() - min_b);
	// clamped anew every frame, never written back unless dragged: no debt builds up
	float shown = ImClamp(stored, min_a, max_a);
	if (size_pt)
		*size_pt = shown;
	if (window->SkipItems || !key)
		return false;

	const ImVec2 start = window->DC.CursorPos;
	const float line = (vertical ? start.x : start.y) + Pt(shown) + gap * .5f;
	const float grip = Pt(3.f);
	const ImRect hit = vertical
		? ImRect(ImVec2(line - grip, start.y), ImVec2(line + grip, start.y + avail.y))
		: ImRect(ImVec2(start.x, line - grip), ImVec2(start.x + avail.x, line + grip));
	const ImGuiID id = window->GetID(key);
	if (!ImGui::ItemAdd(hit, id, nullptr, ImGuiItemFlags_NoNav))
		return false;
	bool hovered = false, held = false;
	ImGui::ButtonBehavior(hit, id, &hovered, &held, ImGuiButtonFlags_FlattenChildren);
	if (hovered || held)
		ImGui::SetMouseCursor(vertical ? ImGuiMouseCursor_ResizeEW : ImGuiMouseCursor_ResizeNS);

	bool changed = false;
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID grab_key = id + 1;
	const float mouse = vertical ? g.IO.MousePos.x : g.IO.MousePos.y;
	if (ImGui::IsItemActivated())
		storage->SetFloat(grab_key, mouse - line);
	if (held)
	{
		// from where the mouse is, not from how far it moved: past a limit and back, the line follows at once
		const float want = (mouse - storage->GetFloat(grab_key) - gap * .5f - (vertical ? start.x : start.y)) / Dpi();
		const float next = ImClamp(want, min_a, max_a);
		if (next != shown)
		{
			shown = next;
			Layout::Set(key, shown);
			changed = true;
		}
	}
	if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		shown = ImClamp(def, min_a, max_a);
		Layout::Set(key, def);
		changed = true;
	}
	if (size_pt)
		*size_pt = shown;

	const float at = (vertical ? start.x : start.y) + Pt(shown) + gap * .5f;
	const ImU32 col = held ? Col(Token::Accent) : hovered ? Col(Token::AccentBright, .6f) : Col(Token::BorderSubtle);
	const float thickness = ImMax(1.f, Pt(held || hovered ? 2.f : 1.f));
	if (vertical)
		window->DrawList->AddLine(ImVec2(at, start.y), ImVec2(at, start.y + avail.y), col, thickness);
	else
		window->DrawList->AddLine(ImVec2(start.x, at), ImVec2(start.x + avail.x, at), col, thickness);
	AuditAdd("control", key, hit, nullptr, true, nullptr, "Drag to resize, double-click to reset");
	return changed;
}

bool XUI::SplitterTail(cstr key, bool vertical, float* tail_pt, float min_head, float min_tail, float def_tail)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float gap = vertical ? g.Style.ItemSpacing.x : g.Style.ItemSpacing.y;
	const float room = ((vertical ? avail.x : avail.y) - gap) / Dpi();
	const float max_tail = ImMax(min_tail, room - min_head);
	// clamped anew every frame, never written back unless dragged: no debt builds up
	float tail = ImClamp(Layout::Get(key, def_tail), min_tail, max_tail);
	if (tail_pt)
		*tail_pt = tail;
	if (window->SkipItems || !key)
		return false;

	const ImVec2 start = window->DC.CursorPos;
	const float end = vertical ? start.x + avail.x : start.y + avail.y;
	const float line = end - Pt(tail) - gap * .5f;
	const float grip = Pt(3.f);
	const ImRect hit = vertical
		? ImRect(ImVec2(line - grip, start.y), ImVec2(line + grip, start.y + avail.y))
		: ImRect(ImVec2(start.x, line - grip), ImVec2(start.x + avail.x, line + grip));
	const ImGuiID id = window->GetID(key);
	if (!ImGui::ItemAdd(hit, id, nullptr, ImGuiItemFlags_NoNav))
		return false;
	bool hovered = false, held = false;
	ImGui::ButtonBehavior(hit, id, &hovered, &held, ImGuiButtonFlags_FlattenChildren);
	if (hovered || held)
		ImGui::SetMouseCursor(vertical ? ImGuiMouseCursor_ResizeEW : ImGuiMouseCursor_ResizeNS);

	bool changed = false;
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const ImGuiID grab_key = id + 1;
	const float mouse = vertical ? g.IO.MousePos.x : g.IO.MousePos.y;
	if (ImGui::IsItemActivated())
		storage->SetFloat(grab_key, mouse - line);
	if (held)
	{
		// from where the mouse is, not from how far it moved: past a limit and back, the line follows at once
		const float next = ImClamp((end - (mouse - storage->GetFloat(grab_key)) - gap * .5f) / Dpi(), min_tail, max_tail);
		if (next != tail)
		{
			tail = next;
			Layout::Set(key, tail);
			changed = true;
		}
	}
	if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		tail = ImClamp(def_tail, min_tail, max_tail);
		Layout::Set(key, def_tail);
		changed = true;
	}
	if (tail_pt)
		*tail_pt = tail;

	const float at = end - Pt(tail) - gap * .5f;
	const ImU32 col = held ? Col(Token::Accent) : hovered ? Col(Token::AccentBright, .6f) : Col(Token::BorderSubtle);
	const float thickness = ImMax(1.f, Pt(held || hovered ? 2.f : 1.f));
	if (vertical)
		window->DrawList->AddLine(ImVec2(at, start.y), ImVec2(at, start.y + avail.y), col, thickness);
	else
		window->DrawList->AddLine(ImVec2(start.x, at), ImVec2(start.x + avail.x, at), col, thickness);
	AuditAdd("control", key, hit, nullptr, true, nullptr, "Drag to resize, double-click to reset");
	return changed;
}

bool XUI::SplitterAt(cstr key, bool vertical, const ImVec2& line, float length, float sign, float lo, float hi, float def)
{
	ImGuiContext& g = *GImGui;
	ImGuiWindow* window = g.CurrentWindow;
	if (window->SkipItems || !key)
		return false;
	const float grip = Pt(3.f);
	const ImRect hit = vertical
		? ImRect(ImVec2(line.x - grip, line.y), ImVec2(line.x + grip, line.y + ImMax(1.f, length)))
		: ImRect(ImVec2(line.x, line.y - grip), ImVec2(line.x + ImMax(1.f, length), line.y + grip));
	const ImGuiID id = window->GetID(key);
	if (!ImGui::ItemAdd(hit, id, nullptr, ImGuiItemFlags_NoNav))
		return false;
	bool hovered = false, held = false;
	ImGui::ButtonBehavior(hit, id, &hovered, &held, ImGuiButtonFlags_FlattenChildren);
	if (hovered || held)
		ImGui::SetMouseCursor(vertical ? ImGuiMouseCursor_ResizeEW : ImGuiMouseCursor_ResizeNS);

	bool changed = false;
	ImGuiStorage* storage = ImGui::GetStateStorage();
	const float mouse = vertical ? g.IO.MousePos.x : g.IO.MousePos.y;
	if (ImGui::IsItemActivated())
	{
		storage->SetFloat(id + 1, mouse);
		storage->SetFloat(id + 2, ImClamp(Layout::Get(key, def), lo, ImMax(lo, hi)));
	}
	// from where the mouse is, not from how far it moved: past a limit and back, the line follows at once
	if (held)
	{
		const float want = storage->GetFloat(id + 2, def) + sign * (mouse - storage->GetFloat(id + 1, mouse)) / Dpi();
		const float next = ImClamp(want, lo, ImMax(lo, hi));
		if (next != Layout::Get(key, def))
		{
			Layout::Set(key, next);
			changed = true;
		}
	}
	if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		Layout::Set(key, def);
		changed = true;
	}
	if (hovered && !held)
		Tip("Drag to resize, double-click to reset");
	const ImU32 col = held ? Col(Token::Accent) : hovered ? Col(Token::AccentBright, .6f) : Col(Token::BorderSubtle);
	const float thickness = ImMax(1.f, Pt(held || hovered ? 2.f : 1.f));
	if (vertical)
		window->DrawList->AddLine(line, ImVec2(line.x, line.y + length), col, thickness);
	else
		window->DrawList->AddLine(line, ImVec2(line.x + length, line.y), col, thickness);
	AuditAdd("control", key, hit, nullptr, true, nullptr, "Drag to resize, double-click to reset");
	return changed;
}

// ===================================================================================================
// States
// ===================================================================================================
bool XUI::EmptyState(Icon icon, cstr title, cstr text, cstr action)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return false;
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const ImVec2 origin = window->DC.CursorPos;
	const float icon_px = Pt(40.f);
	const float wrap = ImMax(Em(8.f), ImMin(avail.x - Pt(32.f), Em(36.f)));

	// measure first, then centre the block in what is left of the panel
	const float title_h = title && *title ? TextSize(Font::Title, title).y : 0.f;
	const float text_h = text && *text ? TextSize(Font::Body, text, nullptr, wrap).y : 0.f;
	const float button_h = action && *action ? ImGui::GetFrameHeight() : 0.f;
	// a short region drops the icon first, then the margins, and keeps the words; one that grows to fit takes it all
	const bool grows = (window->Flags & ImGuiWindowFlags_AlwaysAutoResize) || (window->ChildFlags & ImGuiChildFlags_AutoResizeY);
	const float icon_block = icon_px + Pt(12.f);
	const float words = title_h + (text_h > 0.f ? Pt(6.f) + text_h : 0.f) + (button_h > 0.f ? Pt(16.f) + button_h : 0.f);
	const bool with_icon = grows || avail.y >= icon_block + words + Pt(32.f);
	const float block = (with_icon ? icon_block : 0.f) + words;
	const float margin = grows ? Pt(16.f) : ImClamp((avail.y - block) * .5f, 0.f, Pt(16.f));
	const float top = origin.y + ImMax(margin, (avail.y - block) * .4f);
	const float cx = origin.x + avail.x * .5f;
	ImDrawList* dl = window->DrawList;

	float y = top;
	if (with_icon)
	{
		DrawIcon(dl, icon, ImVec2(IM_ROUND(cx - icon_px * .5f), IM_ROUND(y)), 40.f, Col(Token::TextMuted));
		y += icon_block;
	}
	if (title_h > 0.f)
	{
		xr_string shown;
		const bool cut = Ellipsize(Font::Title, title, avail.x - Pt(16.f), Clip::End, shown);
		const char* drawn = cut ? shown.c_str() : title;
		PaintText(dl, Font::Title, ImVec2(IM_ROUND(cx - TextSize(Font::Title, drawn).x * .5f), IM_ROUND(y)), Col(Token::Text), drawn);
		y += title_h;
	}
	if (text_h > 0.f)
	{
		// each wrapped line centred on its own
		y += Pt(6.f);
		ImFont* face = GetFont(Font::Body) ? GetFont(Font::Body) : ImGui::GetFont();
		const float px = FontPx(Font::Body);
		const char* end = text + strlen(text);
		for (const char* p = text; p < end;)
		{
			const char* brk = face->CalcWordWrapPosition(px, p, end, wrap);
			if (brk == p)
				brk = p + 1;
			const char* line_end = brk;
			while (line_end > p && (line_end[-1] == ' ' || line_end[-1] == '\n'))
				--line_end;
			const float w = TextSize(Font::Body, p, line_end).x;
			PaintText(dl, Font::Body, ImVec2(IM_ROUND(cx - w * .5f), IM_ROUND(y)), Col(Token::TextSecondary), p, line_end);
			y += px;
			p = brk;
			while (p < end && (*p == ' ' || *p == '\n'))
				++p;
		}
	}
	bool pressed = false;
	if (button_h > 0.f)
	{
		y += Pt(16.f);
		BtnSpec s;
		s.id = action;
		s.kind = Kind::Primary;
		s.font = Font::Strong;
		const float w = ButtonWidth(s);
		ImGui::SetCursorScreenPos(ImVec2(IM_ROUND(cx - w * .5f), y));
		pressed = ButtonEx(s);
		y += button_h;
	}
	ImGui::SetCursorScreenPos(origin);
	ImGui::Dummy(ImVec2(avail.x, ImMax(0.f, y - origin.y + margin)));
	return pressed;
}

void XUI::Badge(cstr text, Tone tone)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems || !text)
		return;
	const float h = Pt(18.f);
	const ImVec2 size(TextSize(Font::Small, text).x + Pt(12.f), h);
	const ImVec2 pos(window->DC.CursorPos.x, window->DC.CursorPos.y + ImMax(0.f, window->DC.CurrLineTextBaseOffset - (h - ImGui::GetFontSize()) * .5f));
	const ImRect bb(pos, pos + size);
	ImGui::ItemSize(size, ImMax(0.f, (h - ImGui::GetFontSize()) * .5f));
	if (!ImGui::ItemAdd(bb, 0))
		return;
	const bool muted = tone == Tone::Muted;
	window->DrawList->AddRectFilled(bb.Min, bb.Max, muted ? Col(Token::Raised) : ToneColor(tone, .16f), h * .5f);
	PaintText(window->DrawList, Font::Small, ImVec2(bb.Min.x + Pt(6.f), IM_ROUND(bb.GetCenter().y - FontPx(Font::Small) * .5f)),
		muted ? Col(Token::TextSecondary) : ToneColor(tone), text);
}

void XUI::StatusDot(Tone tone, cstr tip)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	const float line = ImGui::GetTextLineHeight();
	const ImVec2 size(Pt(10.f), line);
	const ImVec2 pos(window->DC.CursorPos.x, window->DC.CursorPos.y + window->DC.CurrLineTextBaseOffset);
	const ImRect bb(pos, pos + size);
	ImGui::ItemSize(size, 0.f);
	if (!ImGui::ItemAdd(bb, 0))
		return;
	window->DrawList->AddCircleFilled(bb.GetCenter(), Pt(4.f), ToneColor(tone));
	Tip(tip);
}

void XUI::Spinner()
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	const float size = Pt(16.f);
	const float line = ImMax(size, ImGui::GetTextLineHeight());
	const ImVec2 pos(window->DC.CursorPos.x, window->DC.CursorPos.y + window->DC.CurrLineTextBaseOffset - (line - ImGui::GetTextLineHeight()) * .5f);
	const ImRect bb(pos, pos + ImVec2(size, line));
	ImGui::ItemSize(bb.GetSize(), 0.f);
	if (!ImGui::ItemAdd(bb, 0))
		return;
	// three quarters of a ring, one turn a second
	const ImVec2 c = bb.GetCenter();
	const float r = size * .5f - Pt(1.5f);
	const float start = float(ImGui::GetTime()) * IM_PI * 2.f;
	ImDrawList* dl = window->DrawList;
	dl->PathClear();
	dl->PathArcTo(c, r, start, start + IM_PI * 1.5f, 24);
	dl->PathStroke(Col(Token::AccentBright), ImDrawFlags_None, ImMax(1.f, Pt(2.f)));
}

void XUI::BusyLine(cstr text)
{
	Spinner();
	ImGui::SameLine(0.f, Pt(8.f));
	TextEllipsis(text ? text : "");
}

int XUI::Banner(Tone tone, cstr text, cstr action1, cstr action2)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return 0;
	const ImVec2 origin = window->DC.CursorPos;
	const float width = ImMax(Em(8.f), ImGui::GetContentRegionAvail().x);
	const float pad = Pt(10.f);
	const float icon = Pt(16.f);

	BtnSpec b1, b2;
	b1.id = action1;
	b2.id = action2;
	const float w1 = action1 && *action1 ? ButtonWidth(b1) : 0.f;
	const float w2 = action2 && *action2 ? ButtonWidth(b2) : 0.f;
	const float buttons = w1 + w2 + (w1 > 0.f && w2 > 0.f ? Pt(6.f) : 0.f);
	const float text_x = origin.x + pad + icon + Pt(8.f);
	const float wrap = ImMax(Em(6.f), origin.x + width - pad - (buttons > 0.f ? buttons + Pt(12.f) : 0.f) - text_x);
	const ImVec2 text_size = TextSize(Font::Body, text ? text : "", nullptr, wrap);
	const float h = ImMax(text_size.y, ImGui::GetFrameHeight()) + pad * 2.f;
	const ImRect bb(origin, origin + ImVec2(width, h));
	ImDrawList* dl = window->DrawList;
	dl->AddRectFilled(bb.Min, bb.Max, ToneColor(tone, .16f), Pt(6.f));
	dl->AddRectFilled(bb.Min, ImVec2(bb.Min.x + Pt(3.f), bb.Max.y), ToneColor(tone), Pt(6.f), ImDrawFlags_RoundCornersLeft);
	const float cy = bb.GetCenter().y;
	DrawIcon(dl, ToneIcon(tone), ImVec2(IM_ROUND(origin.x + pad), IM_ROUND(bb.Min.y + pad + (FontPx(Font::Body) - icon) * .5f)), 16.f, ToneColor(tone));
	PaintText(dl, Font::Body, ImVec2(IM_ROUND(text_x), IM_ROUND(bb.Min.y + pad)), Col(Token::Text), text, nullptr, wrap);

	int clicked = 0;
	float x = bb.Max.x - pad - buttons;
	const float by = IM_ROUND(cy - ImGui::GetFrameHeight() * .5f);
	if (w1 > 0.f)
	{
		ImGui::SetCursorScreenPos(ImVec2(x, by));
		if (ButtonEx(b1))
			clicked = 1;
		x += w1 + Pt(6.f);
	}
	if (w2 > 0.f)
	{
		ImGui::SetCursorScreenPos(ImVec2(x, by));
		if (ButtonEx(b2))
			clicked = 2;
	}
	ImGui::SetCursorScreenPos(origin);
	ImGui::Dummy(bb.GetSize());
	return clicked;
}

// ===================================================================================================
// Cards and pages
// ===================================================================================================
void XUI::BeginCard(cstr id, cstr title, Tone tone, bool attention, bool tone_title)
{
	ImGui::PushStyleColor(ImGuiCol_ChildBg, Col(Token::Raised));
	ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderSubtle));
	ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, Pt(6.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(12.f, 10.f));
	ImGui::BeginChild(id, ImVec2(0.f, 0.f), ImGuiChildFlags_Borders | ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysUseWindowPadding,
		ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	ImGui::PopStyleVar(2);
	ImGui::PopStyleColor(2);
	if (attention)
	{
		const ImVec2 pos = ImGui::GetWindowPos(), size = ImGui::GetWindowSize();
		ImGui::GetWindowDrawList()->AddRectFilled(pos, ImVec2(pos.x + Pt(3.f), pos.y + size.y), ToneColor(tone), Pt(6.f),
			ImDrawFlags_RoundCornersLeft);
	}
	if (title && *title)
	{
		FontScope strong(Font::Strong);
		ImGui::PushStyleColor(ImGuiCol_Text, tone_title ? ToneColor(tone) : Col(Token::Text));
		ImGui::AlignTextToFramePadding();
		TextEllipsis(title);
		ImGui::PopStyleColor();
	}
}

void XUI::EndCard()
{
	ImGui::EndChild();
	ImGui::Dummy(ImVec2(0.f, Pt(4.f)));
}

bool XUI::BeginPage(cstr title_id)
{
	// the work area under the caption, as the last frame left it
	const ImGuiViewport* vp = ImGui::GetMainViewport();
	ImGui::SetNextWindowPos(vp->WorkPos);
	ImGui::SetNextWindowSize(vp->WorkSize);
	ImGui::SetNextWindowViewport(vp->ID);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(24.f, 20.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 0.f);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Base));
	const bool visible = ImGui::Begin(title_id, nullptr, ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoMove |
		ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoBringToFrontOnFocus);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar(3);
	return visible;
}

void XUI::EndPage()
{
	ImGui::End();
}

namespace
{
struct PageCard
{
	float height = 0.f;		// last frame's: the card is centred by it
	int last_frame = -10;
	int shown = 0;			// frames in a row; the key that brought the card up must not also answer it
};
xr_map<xr_string, PageCard> s_PageCards;
PageCard* s_PageCard = nullptr;
}

void XUI::BeginPageCard(cstr id, cstr title, float width_pt)
{
	PageCard& card = s_PageCards[id ? id : ""];
	s_PageCard = &card;
	const int frame = ImGui::GetFrameCount();
	card.shown = card.last_frame >= frame - 1 ? card.shown + 1 : 1;
	card.last_frame = frame;

	// centred a little above the middle; a card taller than the page makes the page scroll
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const float w = ImMin(Pt(width_pt), avail.x);
	ImGui::SetCursorScreenPos(ImVec2(IM_ROUND(origin.x + (avail.x - w) * .5f), IM_ROUND(origin.y + ImMax(0.f, (avail.y - card.height) * .4f))));
	ImGui::PushStyleColor(ImGuiCol_ChildBg, Col(Token::Raised));
	ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderSubtle));
	ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, Pt(6.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(20.f, 16.f));
	ImGui::BeginChild(id, ImVec2(w, 0.f), ImGuiChildFlags_Borders | ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysUseWindowPadding,
		ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	ImGui::PopStyleVar(2);
	ImGui::PopStyleColor(2);
	{
		FontScope font(Font::Title);
		TextEllipsis(title ? title : "");
	}
	ImGui::Dummy(ImVec2(0.f, Pt(8.f)));
}

int XUI::CardFooter(cstr status, Tone tone, const CardAction* actions, int n)
{
	const float gap = Pt(8.f), h = Pt(28.f);
	ImGui::Dummy(ImVec2(0.f, Pt(8.f)));
	// the line runs across the whole card, its padding included
	const ImVec2 wpos = ImGui::GetWindowPos();
	const float line_y = IM_ROUND(ImGui::GetCursorScreenPos().y);
	ImGui::GetWindowDrawList()->AddLine(ImVec2(wpos.x, line_y), ImVec2(wpos.x + ImGui::GetWindowWidth(), line_y), Col(Token::BorderSubtle),
		ImMax(1.f, Pt(1.f)));
	ImGui::Dummy(ImVec2(0.f, Pt(12.f)));

	// the buttons' widths as DialogButton makes them: at least 88, the label and its padding
	float widths[8] = {};
	float total = 0.f;
	n = ImClamp(n, 0, 8);
	for (int i = 0; i < n; ++i)
	{
		BtnSpec s;
		s.id = actions[i].label;
		s.kind = actions[i].kind;
		s.font = actions[i].kind == Kind::Primary || actions[i].kind == Kind::Danger ? Font::Strong : Font::Body;
		s.size = ImVec2(Pt(88.f), h);
		widths[i] = ButtonWidth(s);
		total += widths[i] + (i ? gap : 0.f);
	}
	const ImVec2 row = ImGui::GetCursorScreenPos();
	const float room = ImGui::GetContentRegionAvail().x;
	// the status on the left, in what the buttons leave
	const float status_w = room - total - Pt(12.f);
	if (status && *status && status_w > Em(2.f))
	{
		ImGui::SetCursorScreenPos(ImVec2(row.x, IM_ROUND(row.y + (h - ImGui::GetTextLineHeight()) * .5f)));
		ImGui::PushStyleColor(ImGuiCol_Text, tone == Tone::Muted ? Col(Token::TextSecondary) : ToneColor(tone));
		TextEllipsis(status, status_w);
		ImGui::PopStyleColor();
	}
	int clicked = -1;
	float x = row.x + ImMax(0.f, room - total);
	for (int i = 0; i < n; ++i)
	{
		const CardAction& a = actions[i];
		ImGui::SetCursorScreenPos(ImVec2(IM_ROUND(x), row.y));
		if (DialogButton(a.label, a.kind, a.is_default, a.on, a.why))
			clicked = i;
		x += widths[i] + gap;
	}
	// Enter presses the default - never a Danger one, and never while a field, a popup or another window has the keys
	const bool keys = s_PageCard && s_PageCard->shown > 1 && !FromMcp() && !ImGui::IsAnyItemActive() &&
		!ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopupId | ImGuiPopupFlags_AnyPopupLevel) &&
		(ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows) || !ImGui::IsWindowFocused(ImGuiFocusedFlags_AnyWindow));
	if (clicked < 0 && keys && (ImGui::IsKeyPressed(ImGuiKey_Enter, false) || ImGui::IsKeyPressed(ImGuiKey_KeypadEnter, false)))
		for (int i = 0; i < n; ++i)
			if (actions[i].is_default && actions[i].on && actions[i].kind != Kind::Danger)
				clicked = i;
	ImGui::SetCursorScreenPos(row);
	ImGui::Dummy(ImVec2(room, h));
	return clicked;
}

void XUI::EndPageCard()
{
	ImGui::EndChild();
	if (s_PageCard)
		s_PageCard->height = ImGui::GetItemRectSize().y;
	s_PageCard = nullptr;
}
