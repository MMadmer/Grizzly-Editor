#include "stdafx.h"
#include "..\..\XrECore\Editor\EditorUiContract.h"
#include "..\..\XrEProps\UIPropsLocal.h"

namespace
{
XUI::Icon ClassIcon(ObjClassID cls)
{
	switch (cls)
	{
	case OBJCLASS_GROUP:		return XUI::Icon::Group;
	case OBJCLASS_GLOW:			return XUI::Icon::Glow;
	case OBJCLASS_SCENEOBJECT:	return XUI::Icon::SceneObject;
	case OBJCLASS_LIGHT:		return XUI::Icon::Light;
	case OBJCLASS_SHAPE:		return XUI::Icon::Shape;
	case OBJCLASS_SOUND_SRC:	return XUI::Icon::SoundSource;
	case OBJCLASS_SPAWNPOINT:	return XUI::Icon::Spawn;
	case OBJCLASS_WAY:			return XUI::Icon::Way;
	case OBJCLASS_SECTOR:		return XUI::Icon::Sector;
	case OBJCLASS_PORTAL:		return XUI::Icon::Portal;
	case OBJCLASS_SOUND_ENV:	return XUI::Icon::SoundEnv;
	case OBJCLASS_PS:			return XUI::Icon::Particles;
	case OBJCLASS_DO:			return XUI::Icon::DetailObjects;
	case OBJCLASS_AIMAP:		return XUI::Icon::AIMap;
	case OBJCLASS_WM:			return XUI::Icon::Wallmark;
	case OBJCLASS_FOG_VOL:		return XUI::Icon::FogVolume;
	default:					return XUI::Icon::Settings;
	}
}

// What the Details rows belong to. The scene is asked again when the rows change (a new selection rebuilds them)
// and twice a second besides, never once a frame: a level holds thousands of objects.
struct SSelectionHeader
{
	const void* items = nullptr;
	size_t count = 0;
	double time = -1.0;
	XUI::Icon icon = XUI::Icon::Settings;
	xr_string name, kind;
};
SSelectionHeader s_Header;

void RefreshHeader(const PropItemVec& items)
{
	if (!Scene)
		return;
	const double now = ImGui::GetTime();
	if (s_Header.items == (items.empty() ? nullptr : items.front()) && s_Header.count == items.size() && now - s_Header.time < .5)
		return;
	s_Header.items = items.empty() ? nullptr : items.front();
	s_Header.count = items.size();
	s_Header.time = now;

	const ObjClassID cls = LTools->CurrentClassID();
	ObjectList selected;
	Scene->GetQueryObjects(selected, cls, 1, -1, -1);
	ObjClassID common = OBJCLASS_DUMMY;
	bool uniform = true;
	for (CCustomObject* object : selected)
	{
		if (common == OBJCLASS_DUMMY)
			common = object->FClassID;
		else if (object->FClassID != common)
			uniform = false;
	}
	ESceneToolBase* tool = common != OBJCLASS_DUMMY ? Scene->GetTool(common) : (cls != OBJCLASS_DUMMY ? Scene->GetTool(cls) : nullptr);
	s_Header.kind = uniform && tool ? tool->ClassDesc() : "";
	if (selected.size() == 1)
	{
		s_Header.icon = ClassIcon(common);
		s_Header.name = selected.front()->GetName();
	}
	else if (!selected.empty())
	{
		s_Header.icon = uniform ? ClassIcon(common) : XUI::Icon::Group;
		string64 text;
		xr_sprintf(text, "%u objects", u32(selected.size()));
		s_Header.name = text;
		if (!uniform)
			s_Header.kind = "Several kinds";
	}
	else
	{
		// no object picked: the rows are the tool's own settings
		s_Header.icon = ClassIcon(cls);
		s_Header.name = tool ? tool->ClassDesc() : "Settings";
		s_Header.kind = "Tool";
	}
}
}

UILPropertiesFrom::UILPropertiesFrom()
{
}

UILPropertiesFrom::~UILPropertiesFrom()
{
}

void UILPropertiesFrom::Draw()
{
	if (!bOpen)
		return;
	if (XUI::BeginPanel(EditorUiContract::kWindowDetails, &bOpen))
	{
		UIPropertiesForm* form = LTools->GetProperties();
		form->SetRole("selection");
		form->SetChrome(UIPropertiesForm::chromePanel);
		if (form->Empty())
		{
			XUI::EmptyState(XUI::Icon::Select, "Nothing selected", "Select an object in the viewport or the World Outliner to see its properties here.");
			form->Tick();
		}
		else
		{
			RefreshHeader(form->Items());
			XUI_Local::PanelHeader(s_Header.icon, s_Header.name.c_str(), s_Header.kind.c_str());
			form->Draw();
		}
	}
	XUI::EndPanel();
}
