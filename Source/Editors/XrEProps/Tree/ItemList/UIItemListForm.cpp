#include "stdafx.h"
#include "../../UIPropsLocal.h"


UIItemListForm::UIItemListForm():m_RootItem("")
{
	m_Flags.zero();
	m_SelectedItem = nullptr;
	m_Count = 0;
	m_Search[0] = 0;
	m_Drawing = 0;
	m_RootItem.Form = this;
}

UIItemListForm::~UIItemListForm()
{
	ClearList();
	for (UITreeItem* item : m_Graveyard)
		xr_delete(item);
}

float UIItemListForm::RowHeight()
{
	return XUI::Pt(22.f);
}

void UIItemListForm::Draw(float height)
{
	ImGui::PushID(this);
	++m_Drawing;
	if (m_Flags.test(fSearch))
	{
		string64 hint;
		xr_sprintf(hint, "Search %u items", m_Count);
		XUI::SearchField("##list_search", m_Search, sizeof(m_Search), hint);
	}
	if (m_AppliedSearch != m_Search)
	{
		m_AppliedSearch = m_Search;
		m_RootItem.UpdateFilter(m_Search);
	}

	// Without a host's height: tall enough for its rows, between 10 and 24 of them - at the bottom of a packed panel a
	// list that took what is left could be two rows tall and scroll by itself, out of reach; the panel scrolls instead.
	const float line = RowHeight();
	ImGuiChildFlags child_flags = ImGuiChildFlags_Borders;
	if (height < 0.f)
	{
		// every row in the host's scroll; measured even while the list is scrolled out of view, so a scroll to one of
		// its rows (ui_scroll item:) finds it
		child_flags |= ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysAutoResize;
		height = 0.f;
	}
	else if (height == 0.f)
	{
		const float wanted = float(_max(m_Count, 1u)) * line + ImGui::GetStyle().WindowPadding.y * 2.f;
		height = clampr(wanted, line * 10.f, line * 24.f);
	}
	if (ImGui::BeginChild("##list", ImVec2(0.f, height), child_flags))
	{
		if (!m_Count)
			XUI_Local::MutedValue("Nothing here");
		else if (!m_RootItem.Visible)
			XUI_Local::MutedValue("Nothing matches the search");
		else
		{
			// rows touch: the row's own padding makes it 22 pt, and its highlight fills it
			ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(ImGui::GetStyle().ItemSpacing.x, 0.f));
			ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(ImGui::GetStyle().FramePadding.x, _max(0.f, (line - ImGui::GetFontSize()) * .5f)));
			m_RootItem.DrawRoot();
			ImGui::PopStyleVar(2);
		}
	}
	ImGui::EndChild();
	if (!--m_Drawing)
	{
		for (UITreeItem* item : m_Graveyard)
			xr_delete(item);
		m_Graveyard.clear();
	}
	ImGui::PopID();
}

void UIItemListForm::AssignItems(ListItemsVec& items, const char* name_selection , bool clear_floder, bool save_selected)
{
	ClearList();
	int Index = 0;
	for (auto& i : items)
	{
		xr_string Name = i->key.c_str();

		xr_string RealName;
		if (strrchr(Name.c_str(), '\\'))
		{
			RealName = strrchr(Name.c_str(), '\\') + 1;
		}
		else
		{
			RealName = Name.c_str();
		}
		Name.append("*");
		auto Item = (UIItemListFormItem*)m_RootItem.AppendItem(Name.c_str());
		Item->Object = i;
		Item->Text = RealName.c_str();
		Item->Index = Index++;
	}
	m_Count = u32(items.size());
	m_RootItem.Sort();
	m_RootItem.UpdateFilter(m_Search);
	m_AppliedSearch = m_Search;
	if (name_selection && name_selection[0])
		SelectItem(name_selection);
}

void UIItemListForm::SetFilter(LPCSTR text)
{
	strncpy_s(m_Search, sizeof(m_Search), text ? text : "", _TRUNCATE);
}
void UIItemListForm::ClearList()
{

	for (ListItem* item : m_Items)
	{
		xr_delete(item);
	}
	m_Items.clear();
	for (UITreeItem* item : m_RootItem.Items)
	{
		if (m_Drawing)
			m_Graveyard.push_back(item);
		else
			xr_delete(item);
	}
	m_RootItem.Items.clear();
	m_RootItem = UIItemListFormItem("");
	m_RootItem.Form = this;
	m_SelectedItem = nullptr;
	m_Count = 0;
}

void UIItemListForm::SelectItem(const char* name)
{
	xr_string Name = name;
	Name.append("*");
	UIItemListFormItem *Item = (UIItemListFormItem*)m_RootItem.FindItem(Name.c_str());
	if (Item)
	{

		m_RootItem.ClearSelection();
		Item->bSelected = true;
		m_SelectedItem = Item;
	}

}
void UIItemListForm::ClearSelected()
{
	m_SelectedItem = nullptr;
}
bool UIItemListForm::GetSelected(RStringVec& items) const
{
	if (m_SelectedItem)
	{
		items.push_back(m_SelectedItem->Object->Key());
		return true;
	}
	return false;
}

void UIItemListForm::UpdateSelected(UIItemListFormItem* NewSelected)
{
	m_SelectedItem = NewSelected;
	if (!OnItemFocusedEvent.empty())
		OnItemFocusedEvent(m_SelectedItem ? m_SelectedItem->Object : nullptr);
}
