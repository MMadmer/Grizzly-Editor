#include "stdafx.h"
UIObjectTool::UIObjectTool()
{
    m_selPercent = 50.f;
    m_AddPercent = false;
    m_MultiAppend = false;
    m_PropRandom = false;
    m_Current = nullptr;
    m_RandomAppend = false;
    m_Selection = false;
    m_RealTexture = nullptr;
    m_RemoveTexture = nullptr;
    m_HasInfo = false;
    m_Faces = m_Vertices = 0;
    m_Search[0] = 0;
    m_ObjectList = xr_new<UIItemListForm>();
    m_ObjectList->SetOnItemFocusedEvent(TOnILItemFocused(this, &UIObjectTool::OnItemFocused));
    RefreshList();
}

UIObjectTool::~UIObjectTool()
{
    if (m_RemoveTexture)m_RemoveTexture->Release();
    if (m_RealTexture)m_RealTexture->Release();
    xr_delete(m_ObjectList);
}

void UIObjectTool::DrawPicked()
{
    const float avail = ImGui::GetContentRegionAvail().x;
    // the picture gives way to the list when the body is short
    const float side = floorf(_min(avail, clampr(ImGui::GetContentRegionAvail().y * .2f, XUI::Pt(56.f), XUI::Pt(96.f))));
    XUI_LocalThumbnail(m_RealTexture, side, XUI::Icon::SceneObject,
        m_Current ? "This object has no thumbnail" : "Pick an object in the list, or browse the library");
    // the facts and the buttons beside the picture, under it when the panel is too narrow for both
    if (avail - side - XUI::Pt(8.f) >= XUI::Em(8.f))
        ImGui::SameLine(0.f, XUI::Pt(8.f));
    ImGui::BeginGroup();
    LPCSTR leaf = m_Current ? strrchr(m_Current, '\\') : nullptr;
    XUI::TextEllipsis(m_Current ? (leaf ? leaf + 1 : m_Current) : "Nothing picked");
    if (m_HasInfo)
    {
        char line[64];
        ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(XFinedTheme::ColorToken::TextSecondary));
        sprintf_s(line, "Faces: %d", m_Faces);
        XUI::TextEllipsis(line);
        sprintf_s(line, "Vertices: %d", m_Vertices);
        XUI::TextEllipsis(line);
        ImGui::PopStyleColor();
    }
    const XUI_LocalButton buttons[] =
    {
        { "Browse...",		"Pick the object from the library" },
        { "Refresh List",	"Read the object library again" },
    };
    switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
    {
    case 0:
        UIChooseForm::SelectItem(smObject, 1, m_Current, 0, 0, 0, 0, 0);
        m_Selection = true;
        break;
    case 1:
        RefreshList();
        break;
    default: break;
    }
    ImGui::EndGroup();
}

void UIObjectTool::Draw()
{
    if (m_RemoveTexture)m_RemoveTexture->Release();
    m_RemoveTexture = nullptr;

    // what the tool adds comes first, and its list takes the room the closed sections leave
    DrawPicked();
    XUI_LocalFilteredList("objects", m_Search, sizeof(m_Search), m_ObjectList);

    m_RandomAppend = !!ParentTools->IsAppendRandomActive();
    // closed, the header still says that added objects are randomized
    if (XUI::Section("Add Objects", false, m_RandomAppend ? "Randomized" : nullptr))
    {
        if (ImGui::Checkbox("Randomize Added Objects", &m_RandomAppend))
            ParentTools->ActivateAppendRandom(m_RandomAppend);
        XUI::Tip("Each object added gets a random scale and rotation, picked from what Random Settings holds");
        const XUI_LocalButton buttons[] =
        {
            { "Add Several...",		"Pick several objects and add them all at the origin" },
            { "Random Settings...",	"The scale and rotation ranges and the objects Randomize picks from" },
        };
        switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
        {
        case 0:
            UIChooseForm::SelectItem(smObject, 512, 0);
            m_MultiAppend = true;
            break;
        case 1:
            m_PropRandom = true;
            ParentTools->FillAppendRandomPropertiesBegin();
            break;
        default: break;
        }
        XUI::EndSection();
    }

    if (XUI::Section("Select by Reference", false))
    {
        const bool picked = !!m_Current;
        const XUI_LocalButton same[] =
        {
            { "Select Same",	"Select every visible object made from the object picked in the list", picked, "Pick an object in the list first" },
            { "Deselect Same",	"Deselect every object made from the object picked in the list", picked, "Pick an object in the list first" },
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

    if (XUI::Section("Surfaces", false))
    {
        const XUI_LocalButton buttons[] =
        {
            { "Reset Selected",	"Give the selected objects the surfaces of their reference again" },
            { "Reset All...",	"Give every object in the level the surfaces of its reference again" },
        };
        switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
        {
        case 0:
            Scene->UndoSave();
            ClearSurface(true);
            break;
        case 1:
        {
            u32 count = 0;
            for (ObjectIt it = Scene->FirstObj(OBJCLASS_SCENEOBJECT); it != Scene->LastObj(OBJCLASS_SCENEOBJECT); ++it)
                count += (*it)->Visible() ? 1 : 0;
            char item[64];
            sprintf_s(item, "%u visible scene object%s", count, count == 1 ? "" : "s");
            XUI::ConfirmSpec spec;
            spec.id = "reset_surfaces";
            spec.title = "Reset Surfaces";
            spec.headline = "Reset the surfaces of every object in the level?";
            spec.items.push_back(item);
            spec.consequence = "Surface changes made on these objects in this level are lost.";
            spec.action = "Reset All";
            spec.destructive = true;
            XUI::Confirm(spec, [](XUI::Answer answer)
            {
                if (answer != XUI::Answer::Yes || !Scene)
                    return;
                Scene->UndoSave();
                ClearSurface(false);
            });
            break;
        }
        default: break;
        }
        XUI::EndSection();
    }
}

void UIObjectTool::RefreshList()
{
    ListItemsVec items;
    FS_FileSet lst;
    if (Lib.GetObjects(lst)) {
        FS_FileSetIt	it = lst.begin();
        FS_FileSetIt	_E = lst.end();
        for (; it != _E; it++)
            LHelper().CreateItem(items, it->name.c_str(), 0, ListItem::flDrawThumbnail, 0);
    }
    const xr_string picked = m_Current ? m_Current : "";
    m_Items = items;
    m_ObjectList->AssignItems(items);
    // the tree was built anew: the object picked before is picked again in it, when it is still there
    m_ObjectList->ClearSelected();
    ListItem* again = nullptr;
    for (ListItem* item : m_Items)
        if (item && !picked.empty() && !xr_strcmp(item->Key(), picked.c_str()))
            again = item;
    if (again)
        m_ObjectList->SelectItem(again->Key());
    OnItemFocused(again);
}

void UIObjectTool::OnDrawUI()
{
    if (m_Selection)
    {
        bool change = false;
        xr_string lst;
        if (UIChooseForm::GetResult(change, lst))
        {
            if (change)
            {
                m_ObjectList->SelectItem(lst.c_str());
                for (ListItem* item : m_Items)
                    if (item && !xr_strcmp(item->Key(), lst.c_str()))
                        OnItemFocused(item);
            }
            m_Selection = false;
        }

        UIChooseForm::Update();
    }
    if (m_MultiAppend)
    {
        bool change = false;
        SStringVec lst;
        if (UIChooseForm::GetResult(change, lst))
        {
            if (change)
            {
                Fvector pos = { 0.f,0.f,0.f };
                Fvector up = { 0.f,1.f,0.f };
                Scene->SelectObjects(false, OBJCLASS_SCENEOBJECT);

                SPBItem* pb = UI->ProgressStart(lst.size(), "Adding objects");
                for (AStringIt it = lst.begin(); it != lst.end(); it++)
                {
                    string256 namebuffer;
                    Scene->GenObjectName(OBJCLASS_SCENEOBJECT, namebuffer, it->c_str());
                    CSceneObject* obj = xr_new<CSceneObject>((LPVOID)0, namebuffer);
                    CEditableObject* ref = obj->SetReference(it->c_str());
                    if (!ref)
                    {
                        // one object that does not load must not leave the rest out, nor the progress open
                        ELog.Msg(mtError, "Cannot load the object '%s'; it was not added.", it->c_str());
                        xr_delete(obj);
                        continue;
                    }
                    obj->MoveTo(pos, up);
                    Scene->AppendObject(obj);
                }
                UI->ProgressEnd(pb);
            }
            m_MultiAppend = false;
        }
        UIChooseForm::Update();
    }
    if (m_PropRandom)
    {
        if (ParentTools->FillAppendRandomPropertiesEnd())
        {
            m_PropRandom = false;
        }
        UIPropertiesModal::Update();
    }
}
void UIObjectTool::OnItemFocused(ListItem* item)
{
    if (m_RealTexture)m_RemoveTexture = m_RealTexture;
    m_RealTexture = nullptr;
    m_HasInfo = false;
    m_Current = nullptr;
    if (item)
    {
        m_Current = item->Key();
        EImageThumbnail* thm = ImageLib.CreateThumbnail(m_Current, EImageThumbnail::ETObject);
        if (thm)
        {
            thm->Update(m_RealTexture);
            if (EObjectThumbnail* object = dynamic_cast<EObjectThumbnail*>(thm))
            {
                m_Faces = object->_FaceCount();
                m_Vertices = object->_VertexCount();
                m_HasInfo = true;
            }
            xr_delete(thm);
        }
    }
}

void UIObjectTool::SelByRefObject(bool flag)
{
    LPCSTR N = Current();
    if (N) {
        ObjectIt _F = Scene->FirstObj(OBJCLASS_SCENEOBJECT);
        ObjectIt _E = Scene->LastObj(OBJCLASS_SCENEOBJECT);
        for (; _F != _E; _F++) {
            if ((*_F)->Visible()) {
                CSceneObject* _O = (CSceneObject*)(*_F);
                if (_O->RefCompare(N)) _O->Select(flag);
            }
        }
    }
}

void UIObjectTool::MultiSelByRefObject(bool clear_prev)
{
    ObjectList 	objlist;
    LPU32Vec 	sellist;
    if (Scene->GetQueryObjects(objlist, OBJCLASS_SCENEOBJECT, 1, 1, -1)) {
        for (ObjectIt it = objlist.begin(); it != objlist.end(); it++) {
            LPCSTR N = ((CSceneObject*)*it)->RefName();
            ObjectIt _F = Scene->FirstObj(OBJCLASS_SCENEOBJECT);
            ObjectIt _E = Scene->LastObj(OBJCLASS_SCENEOBJECT);
            for (; _F != _E; _F++) {
                CSceneObject* _O = (CSceneObject*)(*_F);
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
            CSceneObject* _O = (CSceneObject*)(*o_it);
            _O->Select(true);
        }
    }
}

void UIObjectTool::ClearSurface(bool selected)
{
    {
        ObjectIt _F = Scene->FirstObj(OBJCLASS_SCENEOBJECT);
        ObjectIt _E = Scene->LastObj(OBJCLASS_SCENEOBJECT);
        for (; _F != _E; _F++) {
            if ((*_F)->Visible()) {
                CSceneObject* _O = (CSceneObject*)(*_F);
                if ((_O->Selected() && _O->Visible())||!selected)
                {
                    _O->ClearSurface();
                }
            }
        }
    }
}
