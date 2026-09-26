#include "stdafx.h"
UISectorTool::UISectorTool()
{
	m_Edit = false;
    m_CreateNewMultiple = false;
    m_CreateNewSingle = false;
    m_MeshAdd = true;
    m_BoxPick = false;
}

UISectorTool::~UISectorTool()
{
}
void UISectorTool::Draw()
{
    if (XUI::Section("Commands"))
    {
        const bool has_default = !!Scene->FindObjectByName(DEFAULT_SECTOR_NAME, OBJCLASS_SECTOR);
        const XUI_LocalButton buttons[] =
        {
            { "Validate",				"Check the sectors and report what is wrong with them" },
            { "Capture Volume",			"Grow the selected sector over the objects inside it" },
            { "Distribute Objects",		"Hand the objects inside the selected sector to it" },
            { "Create Default",			"Create the sector that holds whatever no other sector does",
              !has_default, "The default sector is there already" },
            { "Remove Default...",		"Remove the default sector", has_default, "There is no default sector" },
        };
        switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
        {
        case 0:
            PortalUtils.Validate(true);
            break;
        case 1:
            if (CSector* S = PortalUtils.GetSelectedSector())
            {
                S->CaptureInsideVolume();
                Scene->UndoSave();
            }
            break;
        case 2:
            if (CSector* S = PortalUtils.GetSelectedSector())
            {
                S->DistributeInsideObjects();
                Scene->UndoSave();
            }
            break;
        case 3:
            if (!PortalUtils.CreateDefaultSector())
                XUI::Toast(XUI::Tone::Warning, "The default sector could not be created");
            break;
        case 4:
        {
            XUI::ConfirmSpec spec;
            spec.id = "remove_default_sector";
            spec.title = "Remove Default Sector";
            spec.headline = "Remove the default sector?";
            spec.items.push_back(DEFAULT_SECTOR_NAME);
            spec.consequence = "This cannot be undone.";
            spec.action = "Remove";
            spec.destructive = true;
            XUI::Confirm(spec, [](XUI::Answer answer)
            {
                if (answer == XUI::Answer::Yes && Scene)
                    PortalUtils.RemoveDefaultSector();
            });
            break;
        }
        default: break;
        }
        XUI::EndSection();
    }
    if (m_Edit && XUI::Section("Edit"))
    {
        if (ImGui::Checkbox("New Sector From Mesh", &m_CreateNewSingle) && m_CreateNewSingle)
            m_CreateNewMultiple = false;
        XUI::Tip("A mesh picked in the viewport becomes a sector of its own");
        if (ImGui::Checkbox("New Sectors From Objects", &m_CreateNewMultiple) && m_CreateNewMultiple)
            m_CreateNewSingle = false;
        XUI::Tip("Each object picked in the viewport becomes a sector of its own");
        if (XUI::BeginPropertyGrid("tools"))
        {
            if (XUI::PropertyRow("Meshes", "What a mesh picked in the viewport does to the selected sector"))
            {
                const char* modes[] = { "Add", "Remove" };
                int mode = m_MeshAdd ? 0 : 1;
                if (XUI::Segmented("##mesh_mode", &mode, modes, 2))
                    m_MeshAdd = mode == 0;
            }
            XUI::EndPropertyGrid();
        }
        const bool creating = m_CreateNewSingle || m_CreateNewMultiple;
        if (creating)
            m_BoxPick = false;
        ImGui::BeginDisabled(creating);
        ImGui::Checkbox("Box Pick", &m_BoxPick);
        ImGui::EndDisabled();
        XUI::Tip(creating ? "Not while a new sector is being made" : "Pick the meshes inside a dragged box");
        XUI::EndSection();
    }
}
