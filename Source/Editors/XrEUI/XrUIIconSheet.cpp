#include "stdafx.h"
#include "XrUIIconSheet.h"
#include "XrUIIcons.h"
#include "XrUIKit.h"
#include "imgui_internal.h"
#include "XrUIKitInternal.h"

void XUI::DrawIconSheet(ImU32 color, int first)
{
	first = ImClamp(first, 1, (int)Icon::Count - 1);
	const float dpi			= Dpi();
	const float sizes[]		= { 16.f, 24.f, 48.f };
	const float gap			= 8.f * dpi;
	const float big			= 48.f * dpi;
	const int	count		= (int)Icon::Count - first;

	float name_w = 0.f;
	for (int i = 1; i < (int)Icon::Count; ++i)
		name_w = std::max(name_w, ImGui::CalcTextSize(IconName((Icon)i)).x);
	const float glyphs_w	= (16.f + 24.f + 48.f) * dpi + gap * 2.f;
	const float cell_w		= std::max(glyphs_w, name_w) + gap * 2.f;
	const float cell_h		= big + gap * 2.f + ImGui::GetTextLineHeight();
	const int	cols		= std::max(1, (int)(ImGui::GetContentRegionAvail().x / cell_w));
	const int	rows		= (count + cols - 1) / cols;

	ImDrawList* dl		= ImGui::GetWindowDrawList();
	const ImU32 ink		= color ? color : ImGui::GetColorU32(ImGuiCol_Text);
	const ImU32 muted	= ImGui::GetColorU32(ImGuiCol_TextDisabled);
	const ImU32 frame	= ImGui::GetColorU32(ImGuiCol_Border, 0.8f);
	const ImU32 margin	= ImGui::GetColorU32(ImGuiCol_Border, 0.35f);

	ImGuiListClipper clipper;
	clipper.Begin(rows, cell_h + ImGui::GetStyle().ItemSpacing.y);
	while (clipper.Step())
	{
		for (int row = clipper.DisplayStart; row < clipper.DisplayEnd; ++row)
		{
			const ImVec2 at = ImGui::GetCursorScreenPos();
			for (int col = 0; col < cols; ++col)
			{
				const int id = row * cols + col + first;
				if (id >= (int)Icon::Count)
					break;
				const Icon	icon = (Icon)id;
				const float	x0	 = at.x + col * cell_w + gap;
				const float	base = at.y + gap + big;
				float x = x0;
				for (float pt : sizes)
				{
					const float side = pt * dpi;
					const ImVec2 min(IM_ROUND(x), IM_ROUND(base - side));
					if (pt == 48.f)
					{
						const float unit = side / 16.f;
						dl->AddRect(min, ImVec2(min.x + side, min.y + side), frame);
						dl->AddRect(ImVec2(min.x + unit, min.y + unit), ImVec2(min.x + side - unit, min.y + side - unit), margin);
					}
					DrawIcon(dl, icon, min, pt, ink);
					x += side + gap;
				}
				dl->AddText(ImVec2(x0, base + gap * 0.5f), muted, IconName(icon));
			}
			ImGui::Dummy(ImVec2(cols * cell_w, cell_h));
		}
	}
	clipper.End();
}

// -iconsheet [first]: a floating window with every glyph (from icon `first` on) in the colours a
// state gives it - the developer's view of the icon set
void XUI::Internal::DrawIconSheetWindow()
{
	static int s_First = 0;
	static bool s_Open = [] {
		const char* at = strstr(::GetCommandLineA(), "-iconsheet");
		if (at && sscanf_s(at + 10, " %d", &s_First) != 1)
			s_First = 1;
		return !!at;
	}();
	static int s_Colour = 0;
	if (!s_Open)
		return;
	const ImGuiViewport* main = ImGui::GetMainViewport();
	ImGui::SetNextWindowViewport(main->ID);
	const ImVec2 at = main->WorkPos, room = main->WorkSize;
	ImGui::SetNextWindowPos(ImVec2(at.x + room.x * 0.04f, at.y + room.y * 0.04f), ImGuiCond_Appearing);
	ImGui::SetNextWindowSize(ImVec2(room.x * 0.92f, room.y * 0.92f), ImGuiCond_Appearing);
	if (ImGui::Begin("Icon Sheet###icon_sheet", &s_Open, ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_NoSavedSettings))
	{
		// in front of the panels the level keeps focusing, never in front of a popup
		if (GImGui->OpenPopupStack.empty())
			ImGui::BringWindowToDisplayFront(ImGui::GetCurrentWindow());
		const char* labels[] = { "Text", "Secondary", "Checked", "Disabled" };
		Segmented("colour", &s_Colour, labels, IM_ARRAYSIZE(labels));
		const ImU32 inks[] = { Col(Token::Text), Col(Token::TextSecondary), Col(Token::AccentBright), Col(Token::TextDisabled) };
		ImGui::BeginChild("glyphs", ImVec2(0, 0), ImGuiChildFlags_None);
		// a checked icon sits on selection.focused
		if (s_Colour == 2)
		{
			ImGuiWindow* child = ImGui::GetCurrentWindow();
			child->DrawList->AddRectFilled(child->InnerRect.Min, child->InnerRect.Max, Col(Token::SelectionFocused));
		}
		DrawIconSheet(inks[s_Colour], s_First);
		ImGui::EndChild();
	}
	ImGui::End();
}
