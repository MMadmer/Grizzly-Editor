#include "stdafx.h"
UIFogVolTool::UIFogVolTool()
{
}

UIFogVolTool::~UIFogVolTool()
{
}

void UIFogVolTool::Draw()
{
	if (XUI::Section("Commands"))
	{
		const XUI_LocalButton buttons[] =
		{
			{ "Group Selected",		"Make one fog volume group of the selected volumes" },
			{ "Ungroup Current",	"Break the current fog volume group up" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0: ParentTools->GroupSelected(); break;
		case 1: ParentTools->UnGroupCurrent(); break;
		default: break;
		}
		XUI::EndSection();
	}
}
