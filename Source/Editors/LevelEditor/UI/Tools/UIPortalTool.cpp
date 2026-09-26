#include "stdafx.h"
UIPortalTool::UIPortalTool()
{
}

UIPortalTool::~UIPortalTool()
{
}

namespace
{
void ToastPortals(int count)
{
    char text[64];
    sprintf_s(text, "Computed %d portal%s", count, count == 1 ? "" : "s");
    XUI::Toast(count ? XUI::Tone::Success : XUI::Tone::Warning, text);
}

// `selected`: the portals of the selected sectors go, named by those sectors; otherwise every portal of the level
void AskPortals(LPCSTR id, LPCSTR title, LPCSTR headline, bool selected, void (*run)())
{
    XUI::ConfirmSpec spec;
    if (selected)
    {
        ObjectList sectors;
        Scene->GetQueryObjects(sectors, OBJCLASS_SECTOR, 1, 1, -1);
        for (CCustomObject* sector : sectors)
            spec.items.push_back(sector->GetName());
    }
    else
    {
        const u32 count = u32(Scene->ListObj(OBJCLASS_PORTAL).size());
        char item[64];
        sprintf_s(item, "%u portal%s in the level", count, count == 1 ? "" : "s");
        spec.items.push_back(item);
    }
    spec.id = id;
    spec.title = title;
    spec.headline = headline;
    spec.consequence = "This cannot be undone.";
    spec.action = "Compute";
    spec.destructive = true;
    XUI::Confirm(spec, [run](XUI::Answer answer)
    {
        if (answer == XUI::Answer::Yes && Scene)
            run();
    });
}
}

void UIPortalTool::Draw()
{
    if (XUI::Section("Commands"))
    {
        const bool any = Scene->ListObj(OBJCLASS_PORTAL).size() > 0;
        ObjectList sectors;
        // the same query CalculateSelectedPortals makes: it wants two sectors at least
        const int selected_sectors = Scene->GetQueryObjects(sectors, OBJCLASS_SECTOR, 1, 1, -1);
        const XUI_LocalButton buttons[] =
        {
            { "Invert Orientation",		"Turn the selected portals around" },
            { "Compute All...",			"Remove every portal and compute them from the sectors again" },
            { "Compute Selected...",	"Compute the portals of the selected sectors again", selected_sectors >= 2, "Select two sectors or more first" },
            { "Remove Duplicates...",	"Remove the portals that repeat another one", any, "There are no portals" },
        };
        switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
        {
        case 0:
        {
            ObjectList lst;
            if (Scene->GetQueryObjects(lst, OBJCLASS_PORTAL, 1, 1, 0))
                for (ObjectIt it = lst.begin(); it != lst.end(); it++)
                    ((CPortal*)*it)->InvertOrientation(true);
            break;
        }
        case 1:
            AskPortals("compute_all_portals", "Compute All Portals", "Remove every portal and compute them again?", false,
                [] { ToastPortals(PortalUtils.CalculateAllPortals()); });
            break;
        case 2:
            AskPortals("compute_selected_portals", "Compute Selected Portals",
                "Remove the portals of these sectors and compute them again?", true,
                [] { ToastPortals(PortalUtils.CalculateSelectedPortals()); });
            break;
        case 3:
        {
            EScenePortalTool* portals = tool;
            XUI::ConfirmSpec spec;
            spec.id = "remove_duplicate_portals";
            spec.title = "Remove Duplicate Portals";
            spec.headline = "Remove every portal that repeats another one?";
            spec.consequence = "This cannot be undone.";
            spec.action = "Remove";
            spec.destructive = true;
            XUI::Confirm(spec, [portals](XUI::Answer answer)
            {
                if (answer == XUI::Answer::Yes && Scene && Scene->GetTool(OBJCLASS_PORTAL) == portals)
                    portals->RemoveSimilar();
            });
            break;
        }
        default: break;
        }
        XUI::EndSection();
    }
}
