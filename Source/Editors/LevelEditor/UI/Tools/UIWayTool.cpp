#include "stdafx.h"

UIWayTool::UIWayTool()
{
	m_WayMode = true;
	m_AutoLink = true;
}

UIWayTool::~UIWayTool()
{
}

namespace
{
// the selected way objects, each edited by `edit`; true when any changed
template <class F>
bool EditSelectedWays(F edit)
{
    ObjectList lst;
    Scene->GetQueryObjects(lst, OBJCLASS_WAY, 1, 1, 0);
    bool changed = false;
    for (CCustomObject* obj : lst)
        changed |= edit(static_cast<CWayObject*>(obj));
    if (changed)
        Scene->UndoSave();
    ExecCommand(COMMAND_UPDATE_PROPERTIES);
    return changed;
}
}

void UIWayTool::Draw()
{
    if (XUI::Section("Edit"))
    {
        if (XUI::BeginPropertyGrid("tools"))
        {
            if (XUI::PropertyRow("Select", "Pick whole ways, or the points of a way"))
            {
                const char* modes[] = { "Ways", "Points" };
                int mode = m_WayMode ? 0 : 1;
                if (XUI::Segmented("##way_mode", &mode, modes, 2))
                {
                    m_WayMode = mode == 0;
                    LTools->SetTarget(OBJCLASS_WAY, m_WayMode ? 0 : 1);
                }
            }
            XUI::EndPropertyGrid();
        }
        XUI::EndSection();
    }
    if (XUI::Section("Links"))
    {
        ImGui::Checkbox("Auto Link", &m_AutoLink);
        XUI::Tip("A point added to a way links to the selected point at once");
        // the point commands need points to act on
        LPCSTR why = m_WayMode ? "Switch Select to Points first" : nullptr;
        const XUI_LocalButton buttons[] =
        {
            { "Create 1-Link",		"Link the selected points one way, each to the next",	!m_WayMode, why },
            { "Create 2-Link",		"Link the selected points both ways",					!m_WayMode, why },
            { "Convert to 1-Link",	"Make the links of the selected ways one-way" },
            { "Convert to 2-Link",	"Make the links of the selected ways two-way" },
            { "Invert Link",		"Turn the links of the selected points around",			!m_WayMode, why },
            { "Remove Link",		"Remove the links of the selected points",				!m_WayMode, why },
        };
        switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
        {
        case 0:
            EditSelectedWays([](CWayObject* way) { way->RemoveLink(); return way->Add1Link(); });
            break;
        case 1:
            EditSelectedWays([](CWayObject* way) { return way->Add2Link(); });
            break;
        case 2:
            EditSelectedWays([](CWayObject* way) { way->Convert1Link(); return true; });
            break;
        case 3:
            EditSelectedWays([](CWayObject* way) { way->Convert2Link(); return true; });
            break;
        case 4:
            EditSelectedWays([](CWayObject* way) { way->InvertLink(); return true; });
            break;
        case 5:
            EditSelectedWays([](CWayObject* way) { way->RemoveLink(); return true; });
            break;
        default: break;
        }
        XUI::EndSection();
    }
}
