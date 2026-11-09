#pragma once
// Widgets the property grid, the chooser and the lists need and the kit does not have yet (docs/UI_DESIGN.md,
// section 8: a widget the kit lacks stays local until the kit takes it). Public ImGui only - imgui_internal.h
// stays inside XrEUI.
namespace XUI_Local
{
using Token = GrizzlyTheme::ColorToken;

inline ImU32 Col(Token token, float alpha = 1.f) { return GrizzlyTheme::U32(token, alpha); }
inline float Round(float v) { return floorf(v + .5f); }

// `text` cut to `avail` px with an ellipsis at its end, in the font being drawn; false when it fits as it is
inline bool Ellipsize(const char* text, float avail, xr_string& out)
{
	return XUI::EllipsizeText(text ? text : "", avail, XUI::Clip::End, out);
}

// A folder of a property grid: a row as wide as the grid that toggles on a click. A top-level one is a band.
inline bool CategoryRow(const char* label, int depth, bool open)
{
	// the row above ends here: the cursor is at the grid's left edge under it
	XUI::EndPropertySection();
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const float w = _max(1.f, ImGui::GetContentRegionAvail().x);
	const float h = XUI::Pt(depth ? 26.f : 28.f);
	const ImVec2 corner(pos.x + w, pos.y + h);
	if (!ImGui::IsRectVisible(pos, corner))
	{
		ImGui::Dummy(ImVec2(w, h));
		ImGui::SetCursorScreenPos(ImVec2(pos.x, corner.y));
		return false;
	}
	const bool clicked = ImGui::InvisibleButton("##category", ImVec2(w, h));
	const bool hovered = ImGui::IsItemHovered();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	if (!depth)
		dl->AddRectFilled(pos, corner, Col(hovered ? Token::Hover : Token::Raised));
	else if (hovered)
		dl->AddRectFilled(pos, corner, Col(Token::Hover), XUI::Pt(3.f));
	const float cy = pos.y + h * .5f;
	const float x = pos.x + XUI::Pt(6.f) + XUI::Pt(12.f) * depth;
	XUI::DrawIcon(dl, open ? XUI::Icon::ChevronDown : XUI::Icon::ChevronRight, ImVec2(Round(x), Round(cy - XUI::Pt(6.f))), 12.f,
		Col(Token::TextSecondary));
	{
		XUI::FontScope font(depth ? XUI::Font::Body : XUI::Font::Strong);
		const float tx = x + XUI::Pt(18.f);
		xr_string shown;
		const bool cut = Ellipsize(label, _max(0.f, corner.x - XUI::Pt(8.f) - tx), shown);
		dl->AddText(ImVec2(Round(tx), Round(cy - ImGui::GetFontSize() * .5f)), Col(Token::Text), cut ? shown.c_str() : label);
		if (cut)
			XUI::Tip(label);
	}
	ImGui::SetCursorScreenPos(ImVec2(pos.x, corner.y));
	return clicked;
}

// A field-looking button: the text cut with an ellipsis, all of it in the tooltip; true on a click. Width: the
// next item's.
inline bool FrameText(const char* id, const char* text, bool muted, const char* tip = nullptr)
{
	const ImGuiStyle& style = ImGui::GetStyle();
	const float w = _max(1.f, ImGui::CalcItemWidth());
	const float h = ImGui::GetFrameHeight();
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const bool clicked = ImGui::InvisibleButton(id, ImVec2(w, h));
	const bool hovered = ImGui::IsItemHovered();
	const bool held = ImGui::IsItemActive();
	const ImVec2 corner(pos.x + w, pos.y + h);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(pos, corner, Col(held ? Token::Pressed : hovered ? Token::InputHovered : Token::Input), style.FrameRounding);
	if (style.FrameBorderSize > 0.f)
		dl->AddRect(pos, corner, ImGui::GetColorU32(ImGuiCol_Border), style.FrameRounding, 0, style.FrameBorderSize);
	xr_string shown;
	const bool cut = Ellipsize(text, w - style.FramePadding.x * 2.f, shown);
	dl->AddText(ImVec2(pos.x + style.FramePadding.x, pos.y + style.FramePadding.y), Col(muted ? Token::TextMuted : Token::Text),
		cut ? shown.c_str() : text);
	if (cut || (tip && *tip))
	{
		xr_string all = cut ? xr_string(text) : xr_string();
		if (tip && *tip)
			all += all.empty() ? tip : xr_string("\n\n") + tip;
		XUI::Tip(all.c_str());
	}
	return clicked;
}

// The row shows several objects at once and they differ: a click gives them all the first one's value.
inline bool MixedField()
{
	return FrameText("##mixed", "Multiple values", true, "The selected objects differ here. Click to give them all the first one's value.");
}

// Secondary text on a row's line, never cut silently
inline void MutedValue(const char* text)
{
	ImGui::AlignTextToFramePadding();
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(Token::TextMuted));
	XUI::TextEllipsis(text, ImGui::CalcItemWidth());
	ImGui::PopStyleColor();
}

// The playback controls a row of buttons may hold (an animation's First Frame ... Last Frame). They are drawn as icons named
// by their tooltips; an engine's glyph caption ("|<<", "||") stands for the word.
enum class Transport { None, FirstFrame, PreviousKey, PreviousFrame, Play, Pause, Stop, NextFrame, NextKey, LastFrame };

// The control a caption names, and its name (the caption itself when it names none)
inline Transport TransportOf(const char* caption, const char** name = nullptr)
{
	struct SName { const char* caption; Transport glyph; const char* name; };
	static const SName kNames[] =
	{
		{ "|<<", Transport::FirstFrame, "First Frame" },		{ "First Frame", Transport::FirstFrame, "First Frame" },
		{ "+<<", Transport::PreviousKey, "Previous Key" },		{ "Previous Key", Transport::PreviousKey, "Previous Key" },
		{ "<<", Transport::PreviousFrame, "Previous Frame" },	{ "Previous Frame", Transport::PreviousFrame, "Previous Frame" },
		{ ">", Transport::Play, "Play" },						{ "Play", Transport::Play, "Play" },
		{ "||", Transport::Pause, "Pause" },					{ "Pause", Transport::Pause, "Pause" },
		{ "Stop", Transport::Stop, "Stop" },
		{ ">>", Transport::NextFrame, "Next Frame" },			{ "Next Frame", Transport::NextFrame, "Next Frame" },
		{ ">>+", Transport::NextKey, "Next Key" },				{ "Next Key", Transport::NextKey, "Next Key" },
		{ ">>|", Transport::LastFrame, "Last Frame" },			{ "Last Frame", Transport::LastFrame, "Last Frame" },
	};
	if (name)
		*name = caption;
	if (!caption)
		return Transport::None;
	// the engine pads its captions with spaces
	const xr_string text = caption;
	const size_t first = text.find_first_not_of(' ');
	if (first == xr_string::npos)
		return Transport::None;
	const xr_string trimmed = text.substr(first, text.find_last_not_of(' ') - first + 1);
	for (const SName& n : kNames)
		if (!_stricmp(trimmed.c_str(), n.caption))
		{
			if (name)
				*name = n.name;
			return n.glyph;
		}
	return Transport::None;
}

// A playback glyph on the icon set's 16-unit grid, filled like its Play and Stop; `min` is the square's top-left corner (px),
// `size_pt` its side. Next Frame, Next Key and Last Frame are Previous Frame, Previous Key and First Frame mirrored.
inline void DrawTransport(ImDrawList* dl, Transport glyph, const ImVec2& min, float size_pt, ImU32 col)
{
	if (glyph == Transport::Play || glyph == Transport::Stop)
	{
		XUI::DrawIcon(dl, glyph == Transport::Play ? XUI::Icon::Play : XUI::Icon::Stop, min, size_pt, col);
		return;
	}
	const float u = XUI::Pt(size_pt) / 16.f;
	const bool mirror = glyph == Transport::NextFrame || glyph == Transport::NextKey || glyph == Transport::LastFrame;
	auto at = [&](float x, float y) { return ImVec2(min.x + (mirror ? 16.f - x : x) * u, min.y + y * u); };
	auto bar = [&](float x0, float x1)
	{
		const ImVec2 a = at(x0, 3.f), b = at(x1, 13.f);
		dl->AddRectFilled(ImVec2(_min(a.x, b.x), a.y), ImVec2(_max(a.x, b.x), b.y), col, .5f * u);
	};
	// anti-aliased filling wants the points clockwise: a mirrored shape takes them the other way round
	auto fill = [&](const float* xy, int n)
	{
		ImVec2 points[4];
		for (int i = 0; i < n; ++i)
			points[mirror ? n - 1 - i : i] = at(xy[i * 2], xy[i * 2 + 1]);
		dl->AddConvexPolyFilled(points, n, col);
	};
	// a triangle pointing left, its tip at `tip`, its base at `base`
	auto left = [&](float tip, float base)
	{
		const float xy[] = { tip, 8.f, base, 3.f, base, 13.f };
		fill(xy, 3);
	};
	switch (glyph)
	{
	case Transport::Pause:
		bar(4.f, 6.75f);
		bar(9.25f, 12.f);
		break;
	case Transport::FirstFrame:
	case Transport::LastFrame:
		bar(2.f, 3.75f);
		left(4.25f, 9.f);
		left(9.25f, 14.f);
		break;
	case Transport::PreviousFrame:
	case Transport::NextFrame:
		bar(3.5f, 5.25f);
		left(6.f, 12.5f);
		break;
	case Transport::PreviousKey:
	case Transport::NextKey:
	{
		// the key is a diamond, the way the timeline marks one
		const float key[] = { 4.5f, 5.f, 7.5f, 8.f, 4.5f, 11.f, 1.5f, 8.f };
		fill(key, 4);
		left(8.5f, 14.f);
	}
	break;
	default:
		break;
	}
}

// A button of `size` showing a playback glyph; true on a click. `tip` names it.
inline bool TransportButton(const char* id, Transport glyph, const char* tip, const ImVec2& size)
{
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const bool pressed = ImGui::Button(id, size);
	XUI::Tip(tip);
	const float glyph_pt = 16.f;
	const float side = XUI::Pt(glyph_pt);
	// the text colour dims with the row when it is disabled
	DrawTransport(ImGui::GetWindowDrawList(), glyph, ImVec2(Round(pos.x + (size.x - side) * .5f), Round(pos.y + (size.y - side) * .5f)),
		glyph_pt, ImGui::GetColorU32(ImGuiCol_Text));
	return pressed;
}

// A colour: its swatch, which opens the picker, and its hex (#RRGGBB) to type; true while it changes
inline bool ColorField(const char* id, float rgb[3])
{
	ImGui::PushID(id);
	const float w = _max(1.f, ImGui::CalcItemWidth());
	const float h = ImGui::GetFrameHeight();
	const float gap = ImGui::GetStyle().ItemInnerSpacing.x;
	bool changed = false;
	if (ImGui::ColorButton("##swatch", ImVec4(rgb[0], rgb[1], rgb[2], 1.f), ImGuiColorEditFlags_NoAlpha | ImGuiColorEditFlags_NoTooltip,
		ImVec2(h, h)))
		ImGui::OpenPopup("##picker");
	XUI::Tip("Pick a colour");
	if (ImGui::BeginPopup("##picker"))
	{
		ImGui::SetNextItemWidth(XUI::Pt(200.f));
		changed |= ImGui::ColorPicker3("##pick", rgb, ImGuiColorEditFlags_NoSidePreview | ImGuiColorEditFlags_DisplayRGB |
			ImGuiColorEditFlags_InputRGB | ImGuiColorEditFlags_PickerHueBar);
		ImGui::EndPopup();
	}
	ImGui::SameLine(0.f, gap);
	char hex[16];
	sprintf_s(hex, "#%02X%02X%02X", int(Round(clampr(rgb[0], 0.f, 1.f) * 255.f)), int(Round(clampr(rgb[1], 0.f, 1.f) * 255.f)),
		int(Round(clampr(rgb[2], 0.f, 1.f) * 255.f)));
	ImGui::SetNextItemWidth(_max(1.f, w - h - gap));
	if (ImGui::InputText("##hex", hex, sizeof(hex), ImGuiInputTextFlags_CharsUppercase | ImGuiInputTextFlags_AutoSelectAll))
	{
		const char* s = hex[0] == '#' ? hex + 1 : hex;
		unsigned v = 0;
		if (strlen(s) == 6 && sscanf_s(s, "%06X", &v) == 1)
		{
			rgb[0] = float((v >> 16) & 0xff) / 255.f;
			rgb[1] = float((v >> 8) & 0xff) / 255.f;
			rgb[2] = float(v & 0xff) / 255.f;
			changed = true;
		}
	}
	XUI::Tip("The colour as #RRGGBB");
	ImGui::PopID();
	return changed;
}

// A pick from a fixed list in a combo; the preview says "Multiple values" where the rows differ, and any pick counts then.
// It ends in the kit's chevron, as PickField does, not in ImGui's solid arrow.
template <typename Name>
bool PickFromList(const char* preview, int count, int* index, bool mixed, Name name)
{
	const ImGuiStyle& style = ImGui::GetStyle();
	const float w = _max(1.f, ImGui::CalcItemWidth());
	const float h = ImGui::GetFrameHeight();
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const ImVec2 corner(pos.x + w, pos.y + h);
	// the frame's own draw list and font: once the list is open the popup is the current window
	ImDrawList* dl = ImGui::GetWindowDrawList();
	ImFont* font = ImGui::GetFont();
	const float font_size = ImGui::GetFontSize();
	const bool hovered = ImGui::IsMouseHoveringRect(pos, corner) && ImGui::IsWindowHovered();
	const char* shown = mixed ? "Multiple values" : preview ? preview : "";
	const float chevron = XUI::Pt(20.f);
	xr_string cut;
	const bool clipped = Ellipsize(shown, _max(0.f, w - style.FramePadding.x * 2.f - chevron), cut);

	bool changed = false;
	ImGui::SetNextItemWidth(w);
	const bool open = ImGui::BeginCombo("##value", "", ImGuiComboFlags_NoArrowButton);
	dl->AddText(font, font_size, ImVec2(pos.x + style.FramePadding.x, pos.y + style.FramePadding.y), Col(mixed ? Token::TextMuted : Token::Text),
		clipped ? cut.c_str() : shown);
	XUI::DrawIcon(dl, XUI::Icon::ChevronDown, ImVec2(Round(corner.x - chevron + XUI::Pt(4.f)), Round(pos.y + h * .5f - XUI::Pt(6.f))), 12.f,
		Col(hovered || open ? Token::Text : Token::TextSecondary));
	if (!open && clipped)
		XUI::Tip(shown);
	if (open)
	{
		for (int i = 0; i < count; ++i)
		{
			const char* text = name(i);
			ImGui::PushID(i);
			const bool selected = i == *index;
			if (ImGui::Selectable(text ? text : "", selected))
			{
				changed = mixed || *index != i;
				*index = i;
			}
			if (selected)
				ImGui::SetItemDefaultFocus();
			ImGui::PopID();
		}
		ImGui::EndCombo();
	}
	return changed;
}

// The panels' header (docs/UI_DESIGN.md, section 6 "Details and Level Settings"): what the rows below belong to,
// its icon and its name, and on the right what kind of thing it is
inline void PanelHeader(XUI::Icon icon, const char* name, const char* kind)
{
	const float h = XUI::Pt(28.f);
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const float w = _max(1.f, ImGui::GetContentRegionAvail().x);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	XUI::DrawIcon(dl, icon, ImVec2(Round(pos.x + XUI::Pt(2.f)), Round(pos.y + (h - XUI::Pt(16.f)) * .5f)), 16.f,
		Col(Token::TextSecondary));
	float right = pos.x + w;
	if (kind && *kind)
	{
		XUI::FontScope caption(XUI::Font::Small);
		xr_string shown;
		// the kind gives way first: at most a third of the row
		const bool cut = Ellipsize(kind, w / 3.f, shown);
		const float kw = ImGui::CalcTextSize(cut ? shown.c_str() : kind).x;
		right -= kw;
		dl->AddText(ImVec2(Round(right), Round(pos.y + (h - ImGui::GetFontSize()) * .5f)), Col(Token::TextMuted),
			cut ? shown.c_str() : kind);
		right -= XUI::Pt(8.f);
	}
	{
		XUI::FontScope strong(XUI::Font::Strong);
		const float x = pos.x + XUI::Pt(24.f);
		ImGui::SetCursorScreenPos(ImVec2(x, Round(pos.y + (h - ImGui::GetFontSize()) * .5f)));
		XUI::TextEllipsis(name && *name ? name : "", _max(1.f, right - x));
	}
	ImGui::SetCursorScreenPos(ImVec2(pos.x, pos.y + h));
}

// The help band under a grid, as tall as the kit's HelpFooter: two Small lines
inline float HelpFooterHeight()
{
	return XUI::Pt(18.f) + XUI::FontSize(XUI::Font::Small) * XUI::Dpi() * 2.f;
}

// The kit's HelpFooter with its text given by the grid's owner (which keeps the last row pointed at, so the band is never
// blank): `title` in secondary text, then as much of `text` as fits after it and on the line below, the rest in the
// tooltip. No title: `text` alone, muted. `width` 0 takes the rest of the row.
inline void HelpFooter(const char* title, const char* text, float width = 0.f)
{
	const float h = HelpFooterHeight();
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const float w = _max(1.f, width > 0.f ? width : ImGui::GetContentRegionAvail().x);
	ImGui::Dummy(ImVec2(w, h));
	if (!ImGui::IsItemVisible())
		return;
	const bool hovered = ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddLine(pos, ImVec2(pos.x + w, pos.y), Col(Token::BorderSubtle));

	XUI::FontScope small_font(XUI::Font::Small);
	ImFont* font = ImGui::GetFont();
	const float size = ImGui::GetFontSize();
	const float left = pos.x + XUI::Pt(4.f);
	const float right = pos.x + w - XUI::Pt(4.f);
	float x = left;
	const float y = pos.y + XUI::Pt(8.f);
	bool cut = false;
	if (title && *title)
	{
		xr_string shown;
		const bool clipped = Ellipsize(title, _max(0.f, right - x), shown);
		const char* drawn = clipped ? shown.c_str() : title;
		dl->AddText(font, size, ImVec2(Round(x), Round(y)), Col(Token::TextSecondary), drawn);
		x += ImGui::CalcTextSize(drawn).x + XUI::Pt(8.f);
		cut |= clipped;
	}
	const char* p = text ? text : "";
	const char* end = p + strlen(p);
	// the first line after the title takes the words that fit there, the second one the rest
	const char* first_blank = strchr(p, ' ');
	if (*p && ImGui::CalcTextSize(p, first_blank ? first_blank : end).x <= right - x)
	{
		const char* brk = font->CalcWordWrapPosition(size, p, end, right - x);
		dl->AddText(font, size, ImVec2(Round(x), Round(y)), Col(Token::TextMuted), p, brk);
		for (p = brk; *p == ' ' || *p == '\n'; ++p)
		{
		}
	}
	if (*p)
	{
		xr_string shown;
		const bool clipped = Ellipsize(p, _max(0.f, right - left), shown);
		dl->AddText(font, size, ImVec2(Round(left), Round(y + size + XUI::Pt(2.f))), Col(Token::TextMuted), clipped ? shown.c_str() : p);
		cut |= clipped;
	}
	if (cut && hovered)
	{
		xr_string all = title && *title ? xr_string(title) : xr_string();
		if (text && *text)
			all += all.empty() ? xr_string(text) : xr_string("\n\n") + text;
		XUI::Tip(all.c_str());
	}
}
}
