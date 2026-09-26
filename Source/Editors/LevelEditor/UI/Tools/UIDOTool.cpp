#include "stdafx.h"
UIDOTool::UIDOTool()
{
	m_DOShuffle = false;
}

UIDOTool::~UIDOTool()
{
}

void UIDOTool::Draw()
{
	if (XUI::Section("Commands"))
	{
		const XUI_LocalButton build[] =
		{
			{ "First Initialize",		"Build the detail slots over the level for the first time" },
			{ "Reinitialize All",		"Build the detail slots and their objects again" },
			{ "Reinitialize Objects",	"Place the detail objects again, keeping the slots" },
			{ "Reinitialize Selected",	"Place the objects of the selected slots again" },
			{ "Update Renderer",		"Show the current details in the viewport" },
		};
		switch (XUI_LocalButtonFlow(build, _countof(build)))
		{
		case 0: if (DM->Initialize()) Scene->UndoSave(); break;
		case 1: if (DM->Reinitialize()) Scene->UndoSave(); break;
		case 2: if (DM->UpdateObjects(true, false)) Scene->UndoSave(); break;
		case 3: if (DM->UpdateObjects(false, true)) Scene->UndoSave(); break;
		case 4: DM->InvalidateCache(); Scene->UndoSave(); break;
		default: break;
		}
		ImGui::Spacing();
		const u32 slots = DM->dtH.size_x * DM->dtH.size_z;
		const u32 objects = u32(DM->objects.size());
		const XUI_LocalButton clear[] =
		{
			{ "Detail Objects...",	"The objects the details are made of, and their colour indices" },
			{ "Clear Slots...",		"Empty every detail slot of the level", slots > 0, "There are no slots" },
			{ "Clear Details...",	"Remove the detail objects and every slot", slots > 0 || objects > 0, "There are no details" },
		};
		const int clear_pressed = XUI_LocalButtonFlow(clear, _countof(clear));
		switch (clear_pressed)
		{
		case 0:
			m_DOShuffle = true;
			UIDOShuffle::Show(DM);
			break;
		case 1:
		case 2:
		{
			const bool all = clear_pressed == 2;
			char slot_line[64], object_line[64];
			sprintf_s(slot_line, "%u detail slot%s", slots, slots == 1 ? "" : "s");
			sprintf_s(object_line, "%u detail object%s", objects, objects == 1 ? "" : "s");
			XUI::ConfirmSpec spec;
			spec.id = all ? "clear_details" : "clear_detail_slots";
			spec.title = all ? "Clear Details" : "Clear Detail Slots";
			spec.headline = all ? "Remove every detail of the level?" : "Empty every detail slot of the level?";
			spec.items.push_back(slot_line);
			if (all)
				spec.items.push_back(object_line);
			spec.consequence = "Initialize the details again to get them back.";
			spec.action = "Clear";
			spec.destructive = true;
			EDetailManager* dm = DM;
			XUI::Confirm(spec, [dm, all](XUI::Answer answer)
			{
				// the scene may have closed while the question was up
				if (answer != XUI::Answer::Yes || !Scene || Scene->GetTool(OBJCLASS_DO) != dm)
					return;
				if (all)
				{
					ExecCommand(COMMAND_UPDATE_PROPERTIES);
					dm->Clear();
				}
				else
					dm->ClearSlots();
				Scene->UndoSave();
			});
			break;
		}
		default: break;
		}
		XUI::EndSection();
	}
}

void UIDOTool::OnDrawUI()
{
	if (m_DOShuffle)
	{
		if (UIDOShuffle::GetResult())
		{
			m_DOShuffle = false;
		}
		UIDOShuffle::Update();
	}
}
