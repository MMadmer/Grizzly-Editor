#include "stdafx.h"

UILightTool::UILightTool()
{
}

UILightTool::~UILightTool()
{
}

void UILightTool::Draw()
{
	if (XUI::Section("Viewport Lighting"))
	{
		const XUI_LocalButton buttons[] =
		{
			{ "Enable Selected",	"The selected lights light the viewport" },
			{ "Enable All",			"Every light lights the viewport" },
			{ "Disable Selected",	"The selected lights stop lighting the viewport" },
			{ "Disable All",		"No light lights the viewport" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0: UseInD3D(false, true); break;
		case 1: UseInD3D(true, true); break;
		case 2: UseInD3D(false, false); break;
		case 3: UseInD3D(true, false); break;
		default: break;
		}
		XUI::EndSection();
	}
}

void UILightTool::UseInD3D(bool bAll, bool bFlag)
{
	ObjectIt _F = Scene->FirstObj(OBJCLASS_LIGHT);
	ObjectIt _E = Scene->LastObj(OBJCLASS_LIGHT);
	for (; _F != _E; _F++) {
		CLight* L = (CLight*)*_F;
		if (bAll) {
			L->AffectD3D(bFlag);
		}
		else {
			if (L->Selected() && L->Visible()) L->AffectD3D(bFlag);
		}
	}
	UI->RedrawScene();
}
