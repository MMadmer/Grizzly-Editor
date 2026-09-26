#include "stdafx.h"

UIShapeTool::UIShapeTool()
{
    Tool = nullptr;
    m_AttachShape = false;
}

UIShapeTool::~UIShapeTool()
{
}

void UIShapeTool::Draw()
{
    if (XUI::Section("Add Shapes"))
    {
        if (XUI::BeginPropertyGrid("tools"))
        {
            if (XUI::PropertyRow("Shape", "What a click in the viewport adds"))
            {
                const char* shapes[] = { "Sphere", "Box" };
                int shape = m_SphereMode ? 0 : 1;
                if (XUI::Segmented("##shape", &shape, shapes, 2))
                    m_SphereMode = shape == 0;
            }
            XUI::EndPropertyGrid();
        }
        XUI::EndSection();
    }
    if (XUI::Section("Attach"))
    {
        if (ImGui::Checkbox("Attach on Click", &m_AttachShape) && m_AttachShape)
            ExecCommand(COMMAND_CHANGE_ACTION, etaAdd);
        XUI::Tip("A shape clicked in the viewport joins the selected shape");

        ObjectList selected;
        Scene->GetQueryObjects(selected, OBJCLASS_SHAPE, 1, 1, 0);
        const XUI_LocalButton buttons[] =
        {
            { "Detach Selected...", "Split the selected shapes into their parts", !selected.empty(), "Select shapes first" },
        };
        if (XUI_LocalButtonFlow(buttons, _countof(buttons)) == 0)
        {
            XUI::ConfirmSpec spec;
            spec.id = "detach_shapes";
            spec.title = "Detach Shapes";
            spec.headline = "Split these shapes into their parts?";
            for (CCustomObject* obj : selected)
                spec.items.push_back(obj->GetName());
            spec.consequence = "This cannot be undone.";
            spec.action = "Detach";
            spec.destructive = true;
            XUI::Confirm(spec, [](XUI::Answer answer)
            {
                ObjectList lst;
                if (answer != XUI::Answer::Yes || !Scene || !Scene->GetQueryObjects(lst, OBJCLASS_SHAPE, 1, 1, 0))
                    return;
                Scene->SelectObjects(false, OBJCLASS_SHAPE);
                for (ObjectIt it = lst.begin(); it != lst.end(); it++)
                    ((CEditShape*)*it)->Detach();
            });
        }
        XUI::EndSection();
    }
    if (XUI::Section("Level Bound"))
    {
        if (ImGui::Checkbox("Edit Level Bound", &EditLevelBound) && EditLevelBound)
            Tool->OnEditLevelBounds(false);
        XUI::Tip("Show the box the level is bound by and edit it");
        const XUI_LocalButton buttons[] =
        {
            { "Recalculate", "Fit the level bound to the level again", EditLevelBound, "Turn Edit Level Bound on first" },
        };
        if (XUI_LocalButtonFlow(buttons, _countof(buttons)) == 0)
            Tool->OnEditLevelBounds(true);
        XUI::EndSection();
    }
}
