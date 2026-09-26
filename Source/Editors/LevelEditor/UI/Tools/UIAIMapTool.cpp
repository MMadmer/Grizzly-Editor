#include "stdafx.h"

UIAIMapTool::UIAIMapTool()
{
	m_Mode = mdAppend;
	m_AutoLink = true;
	m_IgnoreConstraints = false;
	m_IgnoreMaterialsListSelected = -1;
	m_ChooseIgnoreMaterials = false;
}

UIAIMapTool::~UIAIMapTool()
{
}

void UIAIMapTool::Draw()
{
	if (XUI::Section("Commands"))
	{
		const u32 nodes = u32(tool->Nodes().size());
		const XUI_LocalButton buttons[] =
		{
			{ "Generate Full",		"Build the AI map over the whole level" },
			{ "Generate Selected",	"Build the AI map over the selected objects only" },
			{ "Smooth Selected",	"Even out the heights of the selected nodes" },
			{ "Reset Selected",		"Put the selected nodes back where generation placed them" },
			{ "Clear...",			"Remove every node of the AI map", nodes > 0, "The AI map is empty" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0: tool->GenerateMap(false); break;
		case 1: tool->GenerateMap(true); break;
		case 2: tool->SmoothNodes(); break;
		case 3: tool->ResetNodes(); break;
		case 4:
		{
			char item[64];
			sprintf_s(item, "%u node%s", nodes, nodes == 1 ? "" : "s");
			XUI::ConfirmSpec spec;
			spec.id = "clear_ai_map";
			spec.title = "Clear AI Map";
			spec.headline = "Remove every node of the AI map?";
			spec.items.push_back(item);
			spec.consequence = "Generate it again to get it back.";
			spec.action = "Clear";
			spec.destructive = true;
			ESceneAIMapTool* map = tool;
			XUI::Confirm(spec, [map](XUI::Answer answer)
			{
				if (answer != XUI::Answer::Yes || !Scene || Scene->GetTool(OBJCLASS_AIMAP) != map)
					return;
				map->Clear();
				Scene->UndoSave();
			});
			break;
		}
		default: break;
		}
		XUI::EndSection();
	}

	if (XUI::Section("Nodes"))
	{
		ImGui::Checkbox("Ignore Constraints", &m_IgnoreConstraints);
		XUI::Tip("Link and generate nodes even where the height or slope limits would refuse them");
		ImGui::Checkbox("Auto Link", &m_AutoLink);
		XUI::Tip("New nodes link to their neighbours at once");
		XUI::EndSection();
	}

	char count[16];
	sprintf_s(count, "%u", u32(m_IgnoreMaterialsList.size()));
	if (XUI::Section("Ignored Materials", true, m_IgnoreMaterialsList.empty() ? nullptr : count))
	{
		// its Clear... is not the AI map's
		ImGui::PushID("ignored");
		const bool any = !m_IgnoreMaterialsList.empty();
		const bool picked = m_IgnoreMaterialsListSelected >= 0 && m_IgnoreMaterialsListSelected < int(m_IgnoreMaterialsList.size());
		const XUI_LocalButton buttons[] =
		{
			{ "Add...",				"Pick a game material the AI map is not generated on" },
			{ "Remove",				"Take the material picked in the list out", picked, "Pick a material in the list first" },
			{ "Clear...",			"Take every material out of the list", any, "The list is empty" },
		};
		switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
		{
		case 0:
			UIChooseForm::SelectItem(smGameMaterial, 1, 0, 0, 0, 0, 0, 0);
			m_ChooseIgnoreMaterials = true;
			break;
		case 1:
			m_IgnoreMaterialsList.erase(m_IgnoreMaterialsList.begin() + m_IgnoreMaterialsListSelected);
			if (m_IgnoreMaterialsListSelected < int(tool->m_ignored_materials.size()))
				tool->m_ignored_materials.erase(tool->m_ignored_materials.begin() + m_IgnoreMaterialsListSelected);
			m_IgnoreMaterialsListSelected = -1;
			break;
		case 2:
		{
			XUI::ConfirmSpec spec;
			spec.id = "clear_ignored_materials";
			spec.title = "Clear Ignored Materials";
			spec.headline = "Take every material out of the ignored list?";
			for (const xr_string& name : m_IgnoreMaterialsList)
				spec.items.push_back(name);
			spec.consequence = "The AI map is generated on them again.";
			spec.action = "Clear";
			ESceneAIMapTool* map = tool;
			XUI::Confirm(spec, [map](XUI::Answer answer)
			{
				// the scene may have closed while the question was up
				if (answer != XUI::Answer::Yes || !Scene || Scene->GetTool(OBJCLASS_AIMAP) != map)
					return;
				if (UIAIMapTool* form = dynamic_cast<UIAIMapTool*>(map->pForm))
				{
					form->m_IgnoreMaterialsListSelected = -1;
					form->m_IgnoreMaterialsList.clear();
				}
				map->m_ignored_materials.clear();
			});
			break;
		}
		default: break;
		}
		if (any)
		{
			const int rows = _max(1, _min(int(m_IgnoreMaterialsList.size()), 7));
			if (XUI::BeginList("##ignored", ImVec2(0.f, rows * 22.f + 8.f)))
			{
				for (int i = 0; i < int(m_IgnoreMaterialsList.size()); ++i)
				{
					ImGui::PushID(i);
					if (XUI::ListRow("##row", i == m_IgnoreMaterialsListSelected))
						m_IgnoreMaterialsListSelected = i;
					XUI::TextEllipsis(m_IgnoreMaterialsList[i].c_str());
					ImGui::PopID();
				}
			}
			XUI::EndList();
		}
		ImGui::PopID();
		XUI::EndSection();
	}

	if (XUI::Section("Links"))
	{
		if (XUI::BeginPropertyGrid("tools"))
		{
			if (XUI::PropertyRow("Mode", "What a side button does to the links of the selected nodes"))
			{
				const char* modes[] = { "Add", "Remove", "Invert" };
				int mode = int(m_Mode);
				if (XUI::Segmented("##link_mode", &mode, modes, 3))
					m_Mode = EMode(mode);
			}
			XUI::EndPropertyGrid();
		}

		// the sides follow the camera: up is the side the view looks towards
		const float size = 24.f;
		const float cell = XUI::Pt(size + 2.f);
		const ImVec2 origin = ImGui::GetCursorScreenPos();
		struct SSide { int col, row, tag; XUI::Icon icon; LPCSTR tip; };
		const SSide sides[] =
		{
			{ 1, 0, 1, XUI::Icon::ChevronUp,	"The side away from the camera" },
			{ 0, 1, 0, XUI::Icon::ChevronLeft,	"The left side" },
			{ 1, 1, 4, XUI::Icon::Move,			"All four sides" },
			{ 2, 1, 2, XUI::Icon::ChevronRight,	"The right side" },
			{ 1, 2, 3, XUI::Icon::ChevronDown,	"The side towards the camera" },
		};
		for (const SSide& side : sides)
		{
			ImGui::SetCursorScreenPos(ImVec2(origin.x + side.col * cell, origin.y + side.row * cell));
			ImGui::PushID(side.tag);
			if (XUI::IconButton("##side", side.icon, side.tip))
				SideClick(side.tag);
			ImGui::PopID();
		}
		ImGui::SetCursorScreenPos(origin);
		ImGui::Dummy(ImVec2(cell * 3.f, cell * 3.f));

		const XUI_LocalButton buttons[] =
		{
			{ "Select 0-Link", "Select the nodes that have no links" },
			{ "Select 1-Link", "Select the nodes that have one link" },
			{ "Select 2-Link", "Select the nodes that have two links" },
		};
		const int pressed = XUI_LocalButtonFlow(buttons, _countof(buttons));
		if (pressed >= 0)
		{
			tool->SelectNodesByLink(pressed);
			Scene->UndoSave();
		}
		XUI::EndSection();
	}
}



static const int idx[5][4] = {
	{0,1,2,3},
	{1,2,3,0},
	{2,3,0,1},
	{3,0,1,2},
	{4,4,4,4},
};

int ConvertV2L(int side)
{
	if (side < 4) {
		const Fvector& HPB = EDevice->m_Camera.GetHPB();
		float h = angle_normalize(HPB.x) / PI; R_ASSERT((h >= 0.f) && (h <= 2.f));
		if (h > 0.25f && h <= 0.75f)		return idx[3][side];
		else if (h > 0.75f && h <= 1.25f)	return idx[2][side];
		else if (h > 1.25f && h <= 1.75f)	return idx[1][side];
		else return idx[0][side];
	}
	else return side;
}

static const u8 fl[5] = { SAINode::flN1,SAINode::flN2,SAINode::flN3,SAINode::flN4,
							 SAINode::flN1 | SAINode::flN2 | SAINode::flN3 | SAINode::flN4,
	//    					 	 SAINode::flN1|SAINode::flN2,SAINode::flN2|SAINode::flN3,
	//    					 	 SAINode::flN3|SAINode::flN4,SAINode::flN4|SAINode::flN1
};
void UIAIMapTool::SideClick(int tag)
{
	ESceneAIMapTool::EMode mode;
	switch (m_Mode)
	{
	case UIAIMapTool::mdAppend:
		mode = ESceneAIMapTool::mdAppend;
		break;
	case UIAIMapTool::mdRemove:
		mode = ESceneAIMapTool::mdRemove;
		break;
	case UIAIMapTool::mdInvert:
		mode = ESceneAIMapTool::mdInvert;
		break;
	}
	
	tool->MakeLinks(fl[ConvertV2L(tag)], mode, m_IgnoreConstraints);
	Scene->UndoSave();
	UI->RedrawScene();
}

void UIAIMapTool::OnDrawUI()
{
	if (m_ChooseIgnoreMaterials)
	{
		bool result = false;
		xr_string name;
		if (UIChooseForm::GetResult(result, name))
		{
			if (result)
			{
				// the two lists stay parallel: a name without its material is not added at all
				if (SGameMtl* mtl = GameMaterialLibraryEditors->GetMaterial(name.c_str()))
				{
					m_IgnoreMaterialsList.push_back(name);
					tool->m_ignored_materials.push_back(mtl->GetID());
				}
			}
			m_ChooseIgnoreMaterials = false;
		}
		UIChooseForm::Update();
	}
}
