#include "stdafx.h"
#include "UIGroupTool.h"
UIGroupTool::UIGroupTool()
{
	m_ChooseGroup = false;
	m_selPercent = 50.f;
	m_AddPercent = false;
}

UIGroupTool::~UIGroupTool()
{
}
void UIGroupTool::Draw()
{
	if (XUI::Section("Commands"))
	{
		const XUI_LocalButton buttons[] =
		{
			{ "Group",			"Make one group of the selected objects" },
			{ "Ungroup",		"Break the selected groups into their objects" },
			{ "Make Thumbnail",	"Take the library thumbnail of the selected group from the viewport" },
			{ "Save As...",		"Save the selected group to the library" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0: ParentTools->GroupObjects(); break;
		case 1: ParentTools->UngroupObjects(); break;
		case 2: ParentTools->MakeThumbnail(); break;
		case 3: ParentTools->SaveSelectedObject(); break;
		default: break;
		}
		XUI::EndSection();
	}
	if (XUI::Section("Group to Add"))
	{
		if (XUI::BeginPropertyGrid("tools"))
		{
			if (XUI::PropertyRow("Current", "The library group the viewport adds"))
			{
				if (XUI::PickField("##current", m_Current.c_str(), false) == XUI::PickBrowse)
				{
					UIChooseForm::SelectItem(smGroup, 1, m_Current.c_str());
					m_ChooseGroup = true;
				}
			}
			XUI::EndPropertyGrid();
		}
		const XUI_LocalButton buttons[] =
		{
			{ "Reload References", "Read the library groups of the selected groups again" },
		};
		if (XUI_LocalButtonFlow(buttons, _countof(buttons)) == 0)
		{
			ParentTools->ReloadRefsSelectedObject();
			Tools->UpdateProperties(TRUE);
		}
		XUI::EndSection();
	}
	if (XUI::Section("Select by Reference"))
	{
		const bool picked = !m_Current.empty();
		const XUI_LocalButton same[] =
		{
			{ "Select Same",	"Select every visible group made from the current library group", picked, "Pick the current group first" },
			{ "Deselect Same",	"Deselect every group made from the current library group", picked, "Pick the current group first" },
		};
		switch (XUI_LocalButtonFlow(same, _countof(same)))
		{
		case 0: SelByRefObject(true); break;
		case 1: SelByRefObject(false); break;
		default: break;
		}
		if (XUI_LocalPercentSelect(&m_selPercent, &m_AddPercent))
			MultiSelByRefObject(!m_AddPercent);
		XUI::EndSection();
	}
	if (XUI::Section("Pivot"))
	{
		const XUI_LocalButton buttons[] =
		{
			{ "Center to Group",	"Move the pivot of the selected group to its centre" },
			{ "Align to Object...",	"Move the pivot of the selected group to one of its objects" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0: ParentTools->CenterToGroup(); break;
		case 1: ParentTools->AlignToObject(); break;
		default: break;
		}
		XUI::EndSection();
	}
}

void UIGroupTool::OnDrawUI()
{
	if (m_ChooseGroup)
	{
		xr_string in;
		bool resutl;
		if (UIChooseForm::GetResult(resutl,in))
		{
			if (resutl)
			{
				m_Current = in;
				ParentTools->SetCurrentObject(m_Current.c_str());
			}
			m_ChooseGroup = false;
		}
		UIChooseForm::Update();
	}
}

void UIGroupTool::MultiSelByRefObject(bool clear_prev)
{
	ObjectList 	objlist;
	LPU32Vec 	sellist;
	if (Scene->GetQueryObjects(objlist, OBJCLASS_GROUP, 1, 1, -1)) {
		for (ObjectIt it = objlist.begin(); it != objlist.end(); it++) {
			LPCSTR N = ((CGroupObject*)*it)->RefName();
			ObjectIt _F = Scene->FirstObj(OBJCLASS_GROUP);
			ObjectIt _E = Scene->LastObj(OBJCLASS_GROUP);
			for (; _F != _E; _F++) {
				CGroupObject* _O = (CGroupObject*)(*_F);
				if ((*_F)->Visible() && _O->RefCompare(N)) {
					if (clear_prev) {
						_O->Select(false);
						sellist.push_back((u32*)_O);
					}
					else {
						if (!_O->Selected())
							sellist.push_back((u32*)_O);
					}
				}
			}
		}
		std::sort(sellist.begin(), sellist.end());
		sellist.erase(std::unique(sellist.begin(), sellist.end()), sellist.end());
		std::shuffle(sellist.begin(), sellist.end(), xray::legacy_rand_urbg{});
		int max_k = iFloor(float(sellist.size()) / 100.f * float(m_selPercent) + 0.5f);
		int k = 0;
		for (LPU32It o_it = sellist.begin(); k < max_k; o_it++, k++) {
			CGroupObject* _O = (CGroupObject*)(*o_it);
			_O->Select(true);
		}
	}
}

void UIGroupTool::SelByRefObject(bool flag)
{
	ObjectList objlist;
	
	if (!m_Current.empty()) {
		LPCSTR N = m_Current.c_str();
		ObjectIt _F = Scene->FirstObj(OBJCLASS_GROUP);
		ObjectIt _E = Scene->LastObj(OBJCLASS_GROUP);
		for (; _F != _E; _F++) {
			if ((*_F)->Visible()) {
				CGroupObject* _O = (CGroupObject*)(*_F);
				if (_O->RefCompare(N)) _O->Select(flag);
			}
		}
	}
}
