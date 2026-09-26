#include "stdafx.h"
#include "UIDOOneColor.h"
#include "UIDOShuffle.h"
UIDOOneColor::UIDOOneColor()
{
	list_index = -1;
}

UIDOOneColor::~UIDOOneColor()
{
}

// One colour index: the mask colour and the objects that grow on it
void UIDOOneColor::Draw()
{
	if (ImGui::ColorEdit3("##colour", Color, ImGuiColorEditFlags_NoInputs | ImGuiColorEditFlags_NoLabel))
		DOShuffle->bModif = true;
	XUI::Tip("The colour of the detail mask these objects grow on");

	const int left = DOShuffle->m_list_selected;
	const bool left_picked = left >= 0 && left < int(DOShuffle->m_list.size());
	ImGui::SameLine(0.f, XUI::Pt(6.f));
	if (XUI::IconButton("##assign", XUI::Icon::Plus, left_picked ? "Add the object picked in Objects to this colour"
		: "Pick an object in Objects first", false, left_picked))
	{
		AppendItem(DOShuffle->m_list[left]);
		DOShuffle->bModif = true;
	}
	const bool picked = list_index >= 0 && list_index < int(list.size());
	ImGui::SameLine(0.f, XUI::Pt(2.f));
	if (XUI::IconButton("##unassign", XUI::Icon::Minus, picked ? "Take the object picked here out of this colour"
		: "Pick an object of this colour first", false, picked))
	{
		list.erase(list.begin() + list_index);
		list_index = -1;
		DOShuffle->bModif = true;
	}
	ImGui::SameLine(0.f, XUI::Pt(2.f));
	if (XUI::IconButton("##remove", XUI::Icon::Delete, "Remove this colour index; its objects stay in Objects"))
	{
		DOShuffle->bModif = true;
		bOpen = false;
	}

	if (list.empty())
		XUI::StatusText(XUI::Tone::Muted, "No objects grow on this colour yet.");
	else
	{
		const int rows = _max(1, _min(int(list.size()), 4));
		if (XUI::BeginList("##list", ImVec2(0.f, rows * 22.f + 8.f)))
		{
			for (int i = 0; i < int(list.size()); ++i)
			{
				ImGui::PushID(i);
				if (XUI::ListRow("##row", i == list_index))
					list_index = i;
				XUI::TextEllipsis(list[i].c_str(), -1.f, XUI::Clip::Start);
				ImGui::PopID();
			}
		}
		XUI::EndList();
	}
	ImGui::Spacing();
}

void UIDOOneColor::RemoveObject(const xr_string& str)
{
	for (auto b = list.begin(), e = list.end(); b != e; b++)
	{
		if (*b == str)
		{
			list.erase(b);
			return;
		}
	}
}

void UIDOOneColor::AppendItem(const xr_string& item)
{
	for (xr_string& i : list)
	{
		if (i == item)
			return;
	}
	list.push_back(item);
}
