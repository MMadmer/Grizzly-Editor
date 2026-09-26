#include "stdafx.h"

UIChooseFormItem::UIChooseFormItem(shared_str Name):Object(nullptr),UITreeItem(Name)
{
	bIsFavorite = false;
	m_bIsMixed = false;
	bSelected = false;
	Visible = true;
	Reveal = false;
	Index = -1;
}

UIChooseFormItem::~UIChooseFormItem()
{
}

void UIChooseFormItem::Draw()
{
	if (!Visible)
		return;
	ImGui::PushID(this);
	const bool multi = Form->m_Flags.test(cfMultiSelect);
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
				Form->CheckFavorite();
			}
			XUI::Tip(bIsFavorite ? "Take it out of the selection" : "Add it to the selection");
			ImGui::SameLine(0, gap);
		}
		ImGuiTreeNodeFlags Flags = ImGuiTreeNodeFlags_Leaf | ImGuiTreeNodeFlags_NoTreePushOnOpen | ImGuiTreeNodeFlags_SpanAvailWidth;
		if (Form->m_SelectedItem == this || (multi && bSelected))
			Flags |= ImGuiTreeNodeFlags_Selected;
		ImGui::TreeNodeEx("##item", Flags, "%s", Text.c_str());
		if (Reveal)
		{
			ImGui::SetScrollHereY(.5f);
			Reveal = false;
		}
		// the whole name (a leaf of a long path) and what the item is
		if (ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip))
		{
			xr_string tip = Object->name.c_str() ? Object->name.c_str() : "";
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
					Form->m_SelectedItem = this;
					bSelected = true;
					Form->UpdateSelected(this);
				}
			}
			else
			{
				Form->m_RootItem.ClearSelection();
				Form->m_SelectedItem = this;
				bSelected = true;
				Form->UpdateSelected(this);
			}
		}
		// a double click picks it, as OK would
		if (!multi && ImGui::IsItemHovered() && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
			Form->Confirm();
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
				Form->CheckFavorite();
			}
			ImGui::SameLine(0, gap);
		}
		// a search opens every folder that holds a match
		if (Form->m_Search[0] || Reveal)
			ImGui::SetNextItemOpen(true);
		else if (Form->m_Flags.test(cfFullExpand))
			ImGui::SetNextItemOpen(true, ImGuiCond_Once);
		Reveal = false;
		if (ImGui::TreeNodeEx("##folder", ImGuiTreeNodeFlags_SpanAvailWidth, "%s", Name.c_str()))
		{
			for (UITreeItem* Item : Items)
			{
				((UIChooseFormItem*)Item)->Draw();
			}
			ImGui::TreePop();
		}
	}
	ImGui::PopID();
}

void UIChooseFormItem::DrawRoot()
{
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->Draw();
	}
}

bool UIChooseFormItem::UpdateFilter(LPCSTR query)
{
	if (Object)
		Visible = XUI::Match(query, Object->name.c_str());
	else
	{
		Visible = false;
		for (UITreeItem* Item : Items)
			Visible |= ((UIChooseFormItem*)Item)->UpdateFilter(query);
	}
	return Visible;
}

void UIChooseFormItem::Sort()
{
	std::sort(Items.begin(), Items.end(), [](UITreeItem* Right, UITreeItem* Left)->bool
		{
			UIChooseFormItem* pRight = ((UIChooseFormItem*)Right);
			UIChooseFormItem* pLeft = ((UIChooseFormItem*)Left);
			if(pRight->Object&& !pLeft->Object)
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
		((UIChooseFormItem*)Item)->Sort();
	}
}

void UIChooseFormItem::CheckFavorited()
{
	if (Items.size() == 0)return;
	if (!Form->m_Flags.test(cfMultiSelect))
	{
		return;
	}
	m_bIsMixed = false;
	bool bIsLastFavorited = ((UIChooseFormItem*)Items.front())->bIsFavorite;
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->CheckFavorited();
		m_bIsMixed |= ((UIChooseFormItem*)Item)->m_bIsMixed;
		if (bIsLastFavorited != ((UIChooseFormItem*)Item)->bIsFavorite)
		{

			bIsLastFavorited = false;
			m_bIsMixed = true;
		}
	}
	bIsFavorite = bIsLastFavorited;
}

void UIChooseFormItem::SetFavorite(bool bFavorited)
{
	if (!Form->m_Flags.test(cfMultiSelect))
	{
		return;
	}
	bIsFavorite =  bFavorited;
	m_bIsMixed = false;
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->SetFavorite(bFavorited);
	}
}

void UIChooseFormItem::FillFavorited(xr_vector<SChooseItem*>& Favorited)
{
	if (!Form->m_Flags.test(cfMultiSelect))
	{
		return;
	}
	if (Object&&bIsFavorite)
	{
		Favorited.push_back(Object);
	}
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->FillFavorited(Favorited);
	}
}

void UIChooseFormItem::CheckFavorited(xr_vector<SChooseItem*>& Favorited)
{
	if (!Form->m_Flags.test(cfMultiSelect))
	{
		return;
	}
	if (Object)
	{
		bIsFavorite = std::find_if(Favorited.begin(), Favorited.end(), [&](SChooseItem* I) {return Object == I; })!= Favorited.end();
	}
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->CheckFavorited(Favorited);
	}
}

void UIChooseFormItem::ClearSelection()
{
	bSelected = false;
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->ClearSelection();
	}
}

void UIChooseFormItem::Selected(int Start, int End)
{

	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->Selected(Start,End);
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

void UIChooseFormItem::SelectedToFavorite(bool Favorite)
{
	if (!Form->m_Flags.test(cfMultiSelect))
	{
		return;
	}
	for (UITreeItem* Item : Items)
	{
		((UIChooseFormItem*)Item)->SelectedToFavorite(Favorite);
	}
	if (bSelected)
	{
		SetFavorite(Favorite);
	}
}

UITreeItem* UIChooseFormItem::CreateItem(shared_str Name)
{
    auto Item =  xr_new<UIChooseFormItem>(Name);
	Item->Form = Form;
	return Item;
}
