#include "stdafx.h"

UIParticlesTool::UIParticlesTool()
{
    m_Current = nullptr;
    m_Search[0] = 0;
    m_ParticlesList = xr_new<UIItemListForm>();
    m_ParticlesList->SetOnItemFocusedEvent(TOnILItemFocused(this, &UIParticlesTool::OnItemFocused));
    ListItemsVec items;
    for (PS::PEDIt E = ::RImplementation.PSLibrary.FirstPED(); E != ::RImplementation.PSLibrary.LastPED(); E++) {
        ListItem* I = LHelper().CreateItem(items, *(*E)->m_Name, 0, 0, *E);
        I->SetIcon(1);
    }
    for (PS::PGDIt G = ::RImplementation.PSLibrary.FirstPGD(); G != ::RImplementation.PSLibrary.LastPGD(); G++) {
        ListItem* I = LHelper().CreateItem(items, *(*G)->m_Name, 0, 0, *G);
        I->SetIcon(2);
    }

    m_Items = items;
    m_ParticlesList->AssignItems(items);
}

UIParticlesTool::~UIParticlesTool()
{
    xr_delete(m_ParticlesList);
}

namespace
{
void PlaySelected(bool play)
{
    ObjectIt _F = Scene->FirstObj(OBJCLASS_PS);
    ObjectIt _E = Scene->LastObj(OBJCLASS_PS);
    for (; _F != _E; _F++)
        if ((*_F)->Visible() && (*_F)->Selected())
        {
            if (play)	((EParticlesObject*)(*_F))->Play();
            else		((EParticlesObject*)(*_F))->Stop();
        }
}
}

void UIParticlesTool::Draw()
{
    // the effects are what the tool adds: first, and the list takes the room the closed section leaves
    XUI_LocalFilteredList("particles", m_Search, sizeof(m_Search), m_ParticlesList);
    if (XUI::Section("Commands", false))
    {
        const bool picked = !!m_Current;
        const XUI_LocalButton same[] =
        {
            { "Select Same",	"Select every visible particle system made from the effect picked in the list", picked, "Pick an effect in the list first" },
            { "Deselect Same",	"Deselect every particle system made from the effect picked in the list", picked, "Pick an effect in the list first" },
        };
        switch (XUI_LocalButtonFlow(same, _countof(same)))
        {
        case 0: SelByRef(true); break;
        case 1: SelByRef(false); break;
        default: break;
        }
        if (XUI::IconButton("##play", XUI::Icon::Play, "Play the selected particle systems"))
            PlaySelected(true);
        ImGui::SameLine(0.f, XUI::Pt(2.f));
        if (XUI::IconButton("##stop", XUI::Icon::Stop, "Stop the selected particle systems"))
            PlaySelected(false);
        XUI::EndSection();
    }
}

void UIParticlesTool::SelByRef(bool flag)
{
    if (m_Current) 
    {
        ObjectIt _F = Scene->FirstObj(OBJCLASS_PS);
        ObjectIt _E = Scene->LastObj(OBJCLASS_PS);
        for (; _F != _E; _F++) {
            if ((*_F)->Visible()) {
                EParticlesObject* _O = (EParticlesObject*)(*_F);
                if (_O->RefCompare(m_Current)) _O->Select(flag);
            }
        }
    }
}

void UIParticlesTool::OnItemFocused(ListItem* item)
{
    m_Current = nullptr;
    if (item)
    {
        m_Current = item->Key();
    }
}
