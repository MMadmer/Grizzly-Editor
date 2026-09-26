#include "stdafx.h"
#include "../../UIPropsLocal.h"

UIPropertiesItem::UIPropertiesItem(shared_str Name, UIPropertiesForm* propertiesFrom):UITreeItem(Name),PropertiesFrom(propertiesFrom)
{
	PItem = nullptr;
	m_FilterVisible = true;
	m_Expanded = true;
}

UIPropertiesItem::~UIPropertiesItem()
{
}

void UIPropertiesItem::Draw(int depth)
{
	if (!m_FilterVisible)
		return;

	// ids follow the key, not this node: a rebuild while a value is dragged or typed keeps the edit going
	ImGui::PushID(m_Path.c_str());
	const bool filtering = PropertiesFrom->IsFiltering();
	const bool open = filtering || m_Expanded;
	bool toggle = false;
	if (PItem)
	{
		UIPropertiesForm* F = PropertiesFrom;
		const bool read_only = F->IsReadOnly() || PItem->m_Flags.test(PropItem::flDisabled);
		const bool mixed = PItem->m_Flags.test(PropItem::flMixed) && !PItem->m_Flags.test(PropItem::flIgnoreMixed);
		// a switch is one box: a grid that puts labels above values keeps it on its label's line
		const EPropType type = PItem->Type();
		const bool box = (type == PROP_BOOLEAN || type == PROP_FLAG) && !PItem->m_Flags.test(PropItem::flShowCB);
		const int flags = (read_only ? XUI::PropReadOnly : 0) | (mixed ? XUI::PropMixed : 0) | (box ? XUI::PropInline : 0);
		const xr_string label = UIPropertiesForm::LabelOf(Name.c_str());
		const xr_string help = F->GetHelp(PItem);
		const int row_depth = Items.empty() ? depth : depth + 1;
		// a row with rows under it keeps its value; the chevron before its label opens it
		XUI::EndPropertySection();
		const ImVec2 top = ImGui::GetCursorScreenPos();
		// the grid's width: the row runs from its left edge to the end of the region
		const float grid_w = _max(1.f, ImGui::GetContentRegionAvail().x);
		if (XUI::PropertyRow(label.c_str(), help.c_str(), flags, row_depth))
		{
			ImVec2 value_pos = ImGui::GetCursorScreenPos();
			float value_w = ImGui::CalcItemWidth();
			const bool stacked = value_pos.y > top.y + XUI::Pt(20.f);
			if (!Items.empty())
			{
				const float cy = stacked ? top.y + ImGui::GetTextLineHeight() * .5f : top.y + XUI::Pt(13.f);
				const float x = top.x + XUI::Pt(12.f) * depth;
				ImGui::SetCursorScreenPos(ImVec2(x, cy - XUI::Pt(11.f)));
				toggle = ImGui::InvisibleButton("##toggle", ImVec2(XUI::Pt(22.f), XUI::Pt(22.f)));
				XUI::Tip(open ? "Collapse" : "Expand");
				XUI::DrawIcon(ImGui::GetWindowDrawList(), open ? XUI::Icon::ChevronDown : XUI::Icon::ChevronRight,
					ImVec2(XUI_Local::Round(x + XUI::Pt(5.f)), XUI_Local::Round(cy - XUI::Pt(6.f))), 12.f,
					XUI_Local::Col(ImGui::IsItemHovered() ? XUI_Local::Token::Text : XUI_Local::Token::TextSecondary));
			}
			// A value that needs more than its column on one line (X, Y and Z side by side, a row of choices) takes the
			// whole line under its label, the way a grid too narrow for two columns lays out every row
			const float row_h = XUI::Pt(26.f);
			const bool under = !stacked && OneLineWidth() > value_w + .5f;
			if (under)
			{
				value_pos = ImVec2(top.x + XUI::Pt(12.f) * row_depth + XUI::Pt(6.f), top.y + row_h + (row_h - ImGui::GetFrameHeight()) * .5f);
				value_w = _max(1.f, top.x + grid_w - XUI::Pt(2.f) - value_pos.x);
			}
			else if (F->m_ValueMaxEm > 0.f && !TakesTrailing())
				value_w = _min(value_w, XUI::Em(F->m_ValueMaxEm));
			ImGui::SetCursorScreenPos(value_pos);
			ImGui::SetNextItemWidth(value_w);
			DrawValue();
			// the line under the label keeps the grid's rhythm
			if (under && ImGui::GetCursorScreenPos().y - ImGui::GetStyle().ItemSpacing.y < top.y + row_h * 2.f)
			{
				ImGui::SetCursorScreenPos(ImVec2(top.x, top.y + row_h * 2.f));
				ImGui::Dummy(ImVec2(0.f, 0.f));
			}
		}
		// the row ends here, a tall value included: the footer speaks of the row the mouse is over
		XUI::EndPropertySection();
		const float bottom = ImGui::GetCursorScreenPos().y;
		if (bottom > top.y && ImGui::IsMouseHoveringRect(top, ImVec2(top.x + grid_w, bottom)) &&
			ImGui::IsWindowHovered(ImGuiHoveredFlags_AllowWhenBlockedByActiveItem))
			F->m_HoverKey = PItem->Key();
		if (!F->m_FooterShown && F->m_FooterKey == PItem->Key())
			F->m_FooterShown = true;
	}
	else if (!Items.empty())
		toggle = XUI_Local::CategoryRow(Name.c_str(), depth, open);

	// a search keeps every branch that matches open; the author's own state comes back after it
	if (toggle && !filtering)
	{
		m_Expanded = !m_Expanded;
		PropertiesFrom->RememberExpansion(m_Path.c_str(), m_Expanded);
	}
	if (open && !Items.empty())
		for (UITreeItem* Item : Items)
			static_cast<UIPropertiesItem*>(Item)->Draw(depth + 1);
	ImGui::PopID();
}

void UIPropertiesItem::DrawRoot()
{
	VERIFY(!PItem);
	// a page of one top-level folder: its rows start at the left, its name is the page's title
	const xr_string& root = PropertiesFrom->m_RootFolder;
	if (!root.empty() && !PropertiesFrom->IsFiltering())
	{
		for (UITreeItem* Item : Items)
			if (!xr_strcmp(Item->Name.c_str(), root.c_str()))
				for (UITreeItem* Child : Item->Items)
					static_cast<UIPropertiesItem*>(Child)->Draw(0);
		return;
	}
	for (UITreeItem* Item : Items)
		static_cast<UIPropertiesItem*>(Item)->Draw(0);
}

UITreeItem* UIPropertiesItem::CreateItem(shared_str Name)
{
	return xr_new< UIPropertiesItem>(Name,PropertiesFrom);
}

bool UIPropertiesItem::RefreshFilter(LPCSTR filter, const xr_string& parentPath, bool parentMatches,
	u32& visibleCount, u32& totalCount)
{
	xr_string path = parentPath;
	if (Name.size())
	{
		if (!path.empty())
			path += '\\';
		path += Name.c_str();
	}
	m_Path = path;
	if (!m_Path.empty())
		m_Expanded = PropertiesFrom->RestoreExpansion(m_Path.c_str(), m_Expanded);

	// every word of the search in the row's path, any case, Cyrillic too (the kit's search rule)
	const bool selfMatches = parentMatches || XUI::Match(filter, path.c_str());
	bool childMatches = false;
	for (UITreeItem* item : Items)
	{
		UIPropertiesItem* propertyItem = static_cast<UIPropertiesItem*>(item);
		childMatches |= propertyItem->RefreshFilter(filter, path, selfMatches, visibleCount, totalCount);
	}

	m_FilterVisible = selfMatches || childMatches;
	if (PItem)
	{
		++totalCount;
		if (m_FilterVisible)
			++visibleCount;
	}
	return m_FilterVisible;
}

void UIPropertiesItem::Arrange()
{
	std::stable_partition(Items.begin(), Items.end(),
		[](UITreeItem* item) { return !static_cast<UIPropertiesItem*>(item)->IsCategory(); });
	for (UITreeItem* item : Items)
		static_cast<UIPropertiesItem*>(item)->Arrange();
}

void UIPropertiesItem::CollectRows(PropItemVec& out) const
{
	if (PItem)
		out.push_back(PItem);
	for (UITreeItem* item : Items)
		static_cast<const UIPropertiesItem*>(item)->CollectRows(out);
}

void UIPropertiesItem::CountRows(u32& total, u32& visible) const
{
	if (PItem)
	{
		++total;
		if (m_FilterVisible)
			++visible;
	}
	for (UITreeItem* item : Items)
		static_cast<const UIPropertiesItem*>(item)->CountRows(total, visible);
}

void UIPropertiesItem::SetExpandedRecursive(bool expanded)
{
	m_Expanded = expanded;
	PropertiesFrom->RememberExpansion(m_Path.c_str(), expanded);
	for (UITreeItem* item : Items)
		static_cast<UIPropertiesItem*>(item)->SetExpandedRecursive(expanded);
}

void UIPropertiesItem::Reset(bool expanded)
{
	for (UITreeItem* item : Items)
		xr_delete(item);
	Items.clear();
	PItem = nullptr;
	m_FilterVisible = true;
	m_Expanded = expanded;
	m_Path.clear();
}
