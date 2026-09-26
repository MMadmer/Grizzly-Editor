#include "stdafx.h"

UIItemListFormItem::UIItemListFormItem(shared_str Name) :Object(nullptr), UITreeItem(Name)
{
	bIsFavorite = false;
	m_bIsMixed = false;
	bSelected = false;
	Visible = true;
	Index = -1;
	Form = nullptr;
}

UIItemListFormItem::~UIItemListFormItem()
{
}

namespace
{
// the row's name cut to what is left of the row after the arrow's room; the audit hears of a cut one
LPCSTR RowLabel(LPCSTR name, xr_string& shown)
{
	const float w = ImGui::GetContentRegionAvail().x - ImGui::GetTreeNodeToLabelSpacing();
	if (!XUI::EllipsizeText(name, w, XUI::Clip::End, shown))
		return name;
	const ImVec2 at = ImGui::GetCursorScreenPos();
	XUI::AuditTruncated(name, shown.c_str(), at, ImVec2(at.x + ImGui::GetContentRegionAvail().x, at.y + ImGui::GetFrameHeight()));
	return shown.c_str();
}
}

void UIItemListFormItem::Draw()
{
	if (!Visible)
		return;
	xr_string shown;
	ImGui::PushID(this);
	const bool multi = Form->m_Flags.test(UIItemListForm::fMultiSelect);
	const float gap = ImGui::GetStyle().ItemInnerSpacing.x;
	if (Object)
	{
		if (multi)
		{
			if (ImGui::Checkbox("##checkbox", &bIsFavorite))
			{
				if (bSelected)
				{
					Form->m_RootItem.SelectedToFavorite(bIsFavorite);
				}
				else
				{
					Form->m_RootItem.ClearSelection();
					Form->m_SelectedItem = this;
					bSelected = true;
				}
				Form->m_RootItem.CheckFavorited();
			}
			ImGui::SameLine(0, gap);
		}
		ImGuiTreeNodeFlags Flags = ImGuiTreeNodeFlags_Leaf | ImGuiTreeNodeFlags_NoTreePushOnOpen | ImGuiTreeNodeFlags_SpanAvailWidth |
			ImGuiTreeNodeFlags_FramePadding;
		if (Form->m_SelectedItem == this || (multi && bSelected))
			Flags |= ImGuiTreeNodeFlags_Selected;
		ImGui::TreeNodeEx("##item", Flags, "%s", RowLabel(Text.c_str(), shown));
		// an MCP item:<name> scroll finds a row by its name or by its whole key
		XUI::ScrollTarget(Text.c_str());
		XUI::ScrollTarget(Object->Key());
		// the whole name (a leaf of a long path) and what the row is
		if (ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
		{
			xr_string tip = Object->Key() ? Object->Key() : "";
			if (Object->hint.size())
				tip = tip + "\n" + Object->hint.c_str();
			XUI::Tip(tip.c_str());
		}
		if (ImGui::IsItemClicked())
		{
			if (multi)
			{
				if (ImGui::GetIO().KeyShift)
				{
					if (Form->m_SelectedItem)
					{
						Form->m_RootItem.Selected(Index, Form->m_SelectedItem->Index);
					}
					bSelected = true;
				}
				else if (ImGui::GetIO().KeyCtrl)
				{
					bSelected = true;
					Form->UpdateSelected(this);
				}
				else
				{
					Form->m_RootItem.ClearSelection();
					bSelected = true;
					Form->UpdateSelected(this);
				}
			}
			else
			{
				Form->m_RootItem.ClearSelection();
				bSelected = true;
				Form->UpdateSelected(this);
			}
		}
	}
	else
	{
		if (multi)
		{
			ImGui::PushMixedValue(m_bIsMixed);
			bool CheckChange = ImGui::Checkbox("##checkbox", &bIsFavorite);
			ImGui::PopItemFlag();
			XUI::Tip("Everything in this folder");
			if (CheckChange)
			{
				Form->m_RootItem.ClearSelection();
				Form->m_SelectedItem = nullptr;
				bSelected = false;
				SetFavorite(bIsFavorite);
				Form->m_RootItem.CheckFavorited();
			}
			ImGui::SameLine(0, gap);
		}
		// a search opens every folder that holds a match, an MCP item:<name> scroll every folder on the way to it
		const char* wanted = XUI::ScrollTargetWanted();
		if (Form->m_Search[0] || (wanted && Holds(wanted)))
			ImGui::SetNextItemOpen(true);
		const bool open = ImGui::TreeNodeEx("##folder", ImGuiTreeNodeFlags_SpanAvailWidth | ImGuiTreeNodeFlags_FramePadding, "%s",
			RowLabel(Name.c_str(), shown));
		XUI::ScrollTarget(Name.c_str());
		// a cut folder name is whole in its tooltip
		if (shown.size() && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
			XUI::Tip(Name.c_str());
		if (open)
		{
			for (UITreeItem* Item : Items)
			{
				((UIItemListFormItem*)Item)->Draw();
			}
			ImGui::TreePop();
		}
	}

	ImGui::PopID();
}

bool UIItemListFormItem::Holds(const char* name) const
{
	const auto same = [name](const char* s) { return s && !xr_strcmp(s, name); };
	for (UITreeItem* item : Items)
	{
		const UIItemListFormItem* row = (const UIItemListFormItem*)item;
		if (!row->Visible)
			continue;
		// a leaf's Name is its path part plus '*' (AssignItems), hence Text
		if (row->Object ? (same(row->Text.c_str()) || same(row->Object->Key())) : (same(row->Name.c_str()) || row->Holds(name)))
			return true;
	}
	return false;
}

void UIItemListFormItem::DrawRoot()
{
	// a copy: a row's focus callback may give the list new rows while these are walked
	const xr_vector<UITreeItem*> rows = Items;
	for (UITreeItem* Item : rows)
	{
		((UIItemListFormItem*)Item)->Draw();
	}
}

bool UIItemListFormItem::UpdateFilter(LPCSTR query)
{
	// the hint names what the key does not: a spawn entry's section and class
	if (Object)
		Visible = XUI::Match(query, Object->Key()) || (Object->hint.size() && XUI::Match(query, Object->hint.c_str()));
	else
	{
		Visible = false;
		for (UITreeItem* Item : Items)
			Visible |= ((UIItemListFormItem*)Item)->UpdateFilter(query);
	}
	return Visible;
}

void UIItemListFormItem::Sort()
{
	std::sort(Items.begin(), Items.end(), [](UITreeItem* Right, UITreeItem* Left)->bool
		{
			UIItemListFormItem* pRight = ((UIItemListFormItem*)Right);
			UIItemListFormItem* pLeft = ((UIItemListFormItem*)Left);
			if (pRight->Object && !pLeft->Object)
			{
				return false;
			}
			if (!pRight->Object && pLeft->Object)
			{
				return true;
			}
			return xr_strcmp(pRight->Name.c_str(), pLeft->Name.c_str()) < 0;
		});

	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->Sort();
	}
}

void UIItemListFormItem::CheckFavorited()
{
	if (Items.size() == 0)return;
	if (!Form->m_Flags.test(UIItemListForm::fMultiSelect))
	{
		return;
	}
	m_bIsMixed = false;
	bool bIsLastFavorited = ((UIItemListFormItem*)Items.front())->bIsFavorite;
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->CheckFavorited();
		m_bIsMixed |= ((UIItemListFormItem*)Item)->m_bIsMixed;
		if (bIsLastFavorited != ((UIItemListFormItem*)Item)->bIsFavorite)
		{

			bIsLastFavorited = false;
			m_bIsMixed = true;
		}
	}
	bIsFavorite = bIsLastFavorited;
}

void UIItemListFormItem::SetFavorite(bool bFavorited)
{
	if (!Form->m_Flags.test(UIItemListForm::fMultiSelect))
	{
		return;
	}
	bIsFavorite = bFavorited;
	m_bIsMixed = false;
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->SetFavorite(bFavorited);
	}
}

void UIItemListFormItem::FillFavorited(xr_vector<ListItem*>& Favorited)
{
	if (!Form->m_Flags.test(UIItemListForm::fMultiSelect))
	{
		return;
	}
	if (Object && bIsFavorite)
	{
		Favorited.push_back(Object);
	}
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->FillFavorited(Favorited);
	}
}

void UIItemListFormItem::CheckFavorited(xr_vector<ListItem*>& Favorited)
{
	if (!Form->m_Flags.test(UIItemListForm::fMultiSelect))
	{
		return;
	}
	if (Object)
	{
		bIsFavorite = std::find_if(Favorited.begin(), Favorited.end(), [&](ListItem* I) {return Object == I; }) != Favorited.end();
	}
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->CheckFavorited(Favorited);
	}
}

void UIItemListFormItem::ClearSelection()
{
	bSelected = false;
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->ClearSelection();
	}
}

void UIItemListFormItem::Selected(int Start, int End)
{

	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->Selected(Start, End);
	}
	if (!Object || !Visible)
	{
		return;
	}

	if (Start > End)
		std::swap(Start, End);
	if (Index >= Start && Index <= End)
		bSelected = true;
	else
		bSelected = false;
}

void UIItemListFormItem::SelectedToFavorite(bool Favorite)
{
	if (!Form->m_Flags.test(UIItemListForm::fMultiSelect))
	{
		return;
	}
	for (UITreeItem* Item : Items)
	{
		((UIItemListFormItem*)Item)->SelectedToFavorite(Favorite);
	}
	if (bSelected)
	{
		SetFavorite(Favorite);
	}
}

UITreeItem* UIItemListFormItem::CreateItem(shared_str Name)
{
	auto Item = xr_new<UIItemListFormItem>(Name);
	Item->Form = Form;
	return Item;
}
