#include "stdafx.h"
#include "..\..\XrECore\Editor\EditorUiContract.h"
#include "..\..\XrEProps\UIPropsLocal.h"

namespace
{
// Level Settings rows the grid cannot draw from their values alone: Quality is three presets whose current one
// only the scene knows, and an unsaved scene has no name yet
bool DrawWorldValue(PropItem* item)
{
	if (!Scene || !item)
		return false;
	if (item->Type() == PROP_BUTTON && !xr_strcmp(item->Key(), "Scene\\Build options\\Quality"))
	{
		ButtonValue* B = dynamic_cast<ButtonValue*>(item->GetFrontValue());
		if (!B || B->value.size() != 3)
			return false;
		static const char* kPresets[] = { "Draft", "High", "Custom" };
		int current = clampr(int(Scene->m_LevelOp.m_BuildParams.m_quality), 0, 2);
		if (XUI::Segmented("##quality", &current, kPresets, 3))
		{
			B->btn_num = current;
			bool safe = false;
			B->OnBtnClick(safe);
		}
		XUI::Tip("Draft and High set every build option below; Custom lets them be edited one by one.");
		return true;
	}
	if (item->Type() == PROP_CAPTION && !xr_strcmp(item->Key(), "Scene\\Name") && item->GetDrawText().empty())
	{
		XUI_Local::MutedValue("Not saved yet");
		return true;
	}
	return false;
}

// what Level Settings says of its rows (the ones the label alone does not explain)
struct SWorldHelp { const char* key; const char* help; };
const SWorldHelp kWorldHelp[] =
{
	{ "Scene\\Name", "The scene file this level is saved to." },
	{ "Scene\\Build options\\Level path", "The folder of the built level under the game's levels folder." },
	{ "Scene\\Build options\\Custom data", "Lines the build adds to the level's level.ltx as they are." },
	{ "Scene\\Build options\\Quality", "Draft and High fill in the build options below; Custom lets them be set one by one." },
	{ "Scene\\Build options\\Lighting\\Hemisphere quality [0-3]", "How finely the build samples the sky light." },
	{ "Scene\\Build options\\Lighting\\Sun shadow quality [0-3]", "How finely the build samples the sun." },
	{ "Scene\\Build options\\Lighting\\Pixel per meter", "Lightmap pixels per metre of surface." },
	{ "Scene\\Build options\\Lighting\\Jitter samples", "Samples per lightmap pixel: more is smoother and slower to build." },
	{ "Scene\\Build options\\Optimizing\\Normal smooth angle", "Faces that meet at a smaller angle than this are shaded smooth." },
	{ "Scene\\Build options\\Optimizing\\Weld distance (m)", "Vertices closer than this are merged." },
	{ "Scene\\Map version", "The version the level's level.ltx gives in [map_usage]." },
};
}

UIWorldPropertiesFrom::UIWorldPropertiesFrom()
{
}

UIWorldPropertiesFrom::~UIWorldPropertiesFrom()
{
}

void UIWorldPropertiesFrom::Draw()
{
	if (!bOpen)
		return;
	if (XUI::BeginPanel(EditorUiContract::kWindowLevelSettings, &bOpen))
	{
		UIPropertiesForm* form = LTools->GetWorldProperties();
		// once per form: its drawer and its help outlive every rebuild of the rows
		static UIPropertiesForm* s_Hooked = nullptr;
		if (s_Hooked != form)
		{
			s_Hooked = form;
			form->SetValueDrawer(DrawWorldValue);
			for (const SWorldHelp& h : kWorldHelp)
				form->SetHelp(h.key, h.help);
			form->SetUnit("Scene\\Build options\\Optimizing\\Normal smooth angle", "\xC2\xB0");
		}
		// the build options nest three folders deep and their values are short: the labels get half the row
		form->SetRole("world", .5f);
		form->SetChrome(UIPropertiesForm::chromePanel);
		const xr_string& file = LTools->m_LastFileName;
		xr_string name = file.empty() ? xr_string("Untitled") : file;
		const size_t slash = name.find_last_of("\\/");
		if (slash != xr_string::npos)
			name = name.substr(slash + 1);
		XUI_Local::PanelHeader(XUI::Icon::Level, name.c_str(), "Level");
		form->Draw();
	}
	XUI::EndPanel();
}
