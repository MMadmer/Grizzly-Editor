#include "stdafx.h"

UISpawnTool::UISpawnTool()
{
    m_selPercent = 50.f;
    m_AddPercent = false;
    m_Current = nullptr;
    m_Search[0] = 0;
    m_AttachObject = false;
    m_SpawnList = xr_new<UIItemListForm>();
    m_SpawnList->SetOnItemFocusedEvent(TOnILItemFocused(this, &UISpawnTool::OnItemFocused));
    RefreshList();
}

UISpawnTool::~UISpawnTool()
{
    xr_delete(m_SpawnList);
}

void UISpawnTool::Draw()
{
    // the list is what the tool is for: first, under the tool's header, and it takes the room the closed sections
    // leave
    XUI_LocalFilteredList("spawns", m_Search, sizeof(m_Search), m_SpawnList);
    // only zone entities use it, but a restrictor placed without a zone is worse than one more row
    if (XUI::BeginPropertyGrid("tools"))
    {
        if (XUI::PropertyRow("Zone Shape", "The shape a restrictor or another zone entity is placed with"))
        {
            const char* shapes[] = { "Sphere", "Box" };
            int shape = m_ShapeSphere ? 0 : 1;
            if (XUI::Segmented("##zone_shape", &shape, shapes, 2))
                m_ShapeSphere = shape == 0;
        }
        XUI::EndPropertyGrid();
    }

    if (XUI::Section("Select by Reference", false))
    {
        const bool picked = !!m_Current;
        const XUI_LocalButton same[] =
        {
            { "Select Same",	"Select every visible spawn point made from the entry picked in the list", picked, "Pick an entry in the list first" },
            { "Deselect Same",	"Deselect every spawn point made from the entry picked in the list", picked, "Pick an entry in the list first" },
        };
        switch (XUI_LocalButtonFlow(same, _countof(same)))
        {
        case 0: SelByRefObject(true); break;
        case 1: SelByRefObject(false); break;
        default: break;
        }
        if (XUI_LocalPercentSelect(&m_selPercent, &m_AddPercent))
            MultiSelByRefObject(!m_AddPercent);
        XUI::EndSection();
    }

    // closed, the header still says that a click in the viewport attaches
    if (XUI::Section("Attached Objects", false, m_AttachObject ? "Attaching" : nullptr))
    {
        if (ImGui::Checkbox("Attach on Click", &m_AttachObject) && m_AttachObject)
            ExecCommand(COMMAND_CHANGE_ACTION, etaAdd);
        XUI::Tip("A click on an object in the viewport attaches it to the selected spawn point");

        // the selected spawn points that hold an object: what Detach takes it from
        ObjectList selected;
        Scene->GetQueryObjects(selected, OBJCLASS_SPAWNPOINT, 1, 1, 0);
        XUI::Strings holders;
        for (CCustomObject* obj : selected)
        {
            CSpawnPoint* point = dynamic_cast<CSpawnPoint*>(obj);
            if (point && point->m_AttachedObject)
                holders.push_back(point->GetName());
        }
        const XUI_LocalButton buttons[] =
        {
            { "Detach From Selected...", "Take the attached object off the selected spawn points", !holders.empty(),
              "No selected spawn point holds an attached object" },
        };
        if (XUI_LocalButtonFlow(buttons, _countof(buttons)) == 0)
        {
            XUI::ConfirmSpec spec;
            spec.id = "detach_spawn_objects";
            spec.title = "Detach Objects";
            spec.headline = "Detach the attached objects from these spawn points?";
            spec.items = holders;
            spec.consequence = "This cannot be undone.";
            spec.action = "Detach";
            spec.destructive = true;
            XUI::Confirm(spec, [](XUI::Answer answer)
            {
                if (answer != XUI::Answer::Yes || !Scene)
                    return;
                ObjectList lst;
                if (Scene->GetQueryObjects(lst, OBJCLASS_SPAWNPOINT, 1, 1, 0))
                    for (CCustomObject* obj : lst)
                        if (CSpawnPoint* point = dynamic_cast<CSpawnPoint*>(obj))
                            point->DetachObject();
                UI->RedrawScene();
            });
        }
        XUI::EndSection();
    }
}

void UISpawnTool::SelByRefObject(bool flag)
{
    ObjectList objlist;
    LPCSTR N = Current();
    if (N) {
        ObjectIt _F = Scene->FirstObj(OBJCLASS_SPAWNPOINT);
        ObjectIt _E = Scene->LastObj(OBJCLASS_SPAWNPOINT);
        for (; _F != _E; _F++) {
            if ((*_F)->Visible()) {
                CSpawnPoint* _O = (CSpawnPoint*)(*_F);
                if (_O->RefCompare(N)) _O->Select(flag);
            }
        }
    }
}
void UISpawnTool::MultiSelByRefObject(bool clear_prev)
{
    ObjectList 	objlist;
    LPU32Vec 	sellist;
    if (Scene->GetQueryObjects(objlist, OBJCLASS_SPAWNPOINT, 1, 1, -1)) {
        for (ObjectIt it = objlist.begin(); it != objlist.end(); it++) {
            LPCSTR N = ((CSpawnPoint*)*it)->RefName();
            ObjectIt _F = Scene->FirstObj(OBJCLASS_SPAWNPOINT);
            ObjectIt _E = Scene->LastObj(OBJCLASS_SPAWNPOINT);
            for (; _F != _E; _F++) {
                CSpawnPoint* _O = (CSpawnPoint*)(*_F);
                if ((*_F)->Visible() && _O->RefCompare(N)) {
                    if (clear_prev) {
                        _O->Select(false);
                        sellist.push_back((u32*)_O);
                    }
                    else {
                        if (!_O->Selected())
                            sellist.push_back((u32*)_O);
                    }
                }
            }
        }
        std::sort(sellist.begin(), sellist.end());
        sellist.erase(std::unique(sellist.begin(), sellist.end()), sellist.end());
        std::shuffle(sellist.begin(), sellist.end(), xray::legacy_rand_urbg{});
        int max_k = iFloor(float(sellist.size()) / 100.f * float(m_selPercent) + 0.5f);
        int k = 0;
        for (LPU32It o_it = sellist.begin(); k < max_k; o_it++, k++) {
            CSpawnPoint* _O = (CSpawnPoint*)(*o_it);
            _O->Select(true);
        }
    }
}

// What a spawn class actually is. The list shows the leaf of the $spawn path, so a
// row reads "spawn group" or "graph point" and tells the author nothing about whether
// it is a zone, a marker, or engine bookkeeping they should not be placing at all.
static LPCSTR SpawnClassMeaning(LPCSTR cls)
{
	if (!cls) return 0;
	if (0 == _stricmp(cls, "SMRTTRRN"))	return "Smart terrain: owns NPCs and hands them jobs. Place one to get a smart of your own instead of hunting numbered ones";
	if (0 == _stricmp(cls, "SPC_RS_S"))	return "Zone with a shape. Quests use it as a place, a marker target, or a pen NPCs cannot leave";
	if (0 == _stricmp(cls, "SCRIPTZN"))	return "Script zone: a shape the level's own logic reacts to";
	if (0 == _stricmp(cls, "LVL_CHNG"))	return "Level changer: walking into it sends the player to another level";
	if (0 == _stricmp(cls, "AI_SPGRP"))	return "Spawn group (legacy): a respawn point of the old kind, not a smart terrain";
	if (0 == _stricmp(cls, "AI_GRAPH"))	return "Graph point: a vertex of the game graph, used when the level is compiled - not a runtime object";
	if (0 == _stricmp(cls, "SO_HLAMP"))	return "Hanging lamp";
	return 0;
}

// Dead Air's quest_items.ltx quotes "<tab><39 blanks>;quest_items\..." - an indent and a comment mark that ended up
// inside the value - which made a folder named ";quest_items" at the right edge of the list, cut off by it.
xr_string UISpawnTool::ListPath(LPCSTR spawn)
{
    xr_string path;
    for (LPCSTR p = spawn; p && *p;)
    {
        LPCSTR end = strchr(p, '\\');
        if (!end)
            end = p + xr_strlen(p);
        LPCSTR a = p, b = end;
        while (a < b && (*a == ' ' || *a == '\t' || *a == ';'))
            ++a;
        while (b > a && (b[-1] == ' ' || b[-1] == '\t'))
            --b;
        if (a < b)
        {
            if (!path.empty())
                path += '\\';
            path.append(a, b);
        }
        p = *end ? end + 1 : end;
    }
    return path;
}

void UISpawnTool::RefreshList()
{
    ListItemsVec items;
    // The points the editor makes itself are listed by what they are; the item carries the factory name ("$rpoint"),
    // which is what placing it hands the tool
    if (ListItem* start = LHelper().CreateItem(items, "Player Start", 0, 0, (LPVOID)PLAYERSTART_CHOOSE_NAME))
        start->hint = "Where a new game of the module's mode begins: position and facing.\n"
                      "One per module. The module's world (Mod > World Rules) says what else the game keeps.";
    if (ListItem* env = LHelper().CreateItem(items, "Environment Modifier", 0, 0, (LPVOID)ENVMOD_CHOOSE_NAME))
        env->hint = "A sphere that changes the weather while the camera is inside it: fog colour and density, view distance,\n"
                    "ambient, sky and hemisphere colours, strongest at its centre. For a cave, a cellar or a foggy hollow.";
    if (ListItem* rpoint = LHelper().CreateItem(items, "Respawn Point", 0, 0, (LPVOID)RPOINT_CHOOSE_NAME))
        rpoint->hint = "Where a player, an artefact or an item appears in a multiplayer game, by team and game type.\n"
                       "A single-player game does not read it: a new game begins at the Player Start.";
    CInifile::Root& data = ((CInifile*)pSettings)->sections();
    for (CInifile::RootIt it = data.begin(); it != data.end(); it++) {
        LPCSTR val;
        if ((*it)->line_exist("$spawn", &val))
        {
            const xr_string caption = ListPath(pSettings->r_string_wb((*it)->Name, "$spawn").c_str());
            shared_str sect = (*it)->Name;
            if (!caption.empty())
            {
                ListItem* I = LHelper().CreateItem(items, caption.c_str(), 0, ListItem::flDrawThumbnail, (LPVOID) * (*it)->Name);
                if (I)
                {
                    // the section is the name everything else refers to, and the class
                    // is what decides whether this row is any use to a quest
                    LPCSTR cls = 0;
                    (*it)->line_exist("class", &cls);
                    string512 tip;
                    LPCSTR meaning = SpawnClassMeaning(cls);
                    sprintf_s(tip, "%s%s%s%s%s", sect.c_str(),
                        (cls && cls[0]) ? "  (class " : "", (cls && cls[0]) ? cls : "", (cls && cls[0]) ? ")" : "",
                        meaning ? "" : "");
                    if (meaning)
                    {
                        xr_strcat(tip, sizeof(tip), "\n");
                        xr_strcat(tip, sizeof(tip), meaning);
                    }
                    I->hint = tip;
                }
            }
        }
    }
    m_Items = items;
    m_SpawnList->AssignItems(items);
    // the entries were read anew: nothing of the old list stays picked
    m_SpawnList->ClearSelected();
    m_Current = nullptr;
}

void UISpawnTool::OnItemFocused(ListItem* item)
{
    m_Current = 0;
    if (item)
    {
        m_Current = (LPCSTR)item->m_Object;
    }
    ExecCommand(COMMAND_RENDER_FOCUS);
}
