#include "stdafx.h"
#include "UIChooseForm.h"
#include "../../UIPropsLocal.h"
UIChooseForm::EventsMap	UIChooseForm::m_Events;
UIChooseForm* UIChooseForm::Form = 0;
ImTextureID   UIChooseForm::NullTexture = nullptr;
bool UIChooseForm::s_Closing = false;

namespace
{
// the dialog's id; the title before it is the chooser's caption
constexpr const char* kDialogId = "###dlg.choose";
constexpr const char* kClosingTitle = "Select###dlg.choose";
}

void UIChooseForm::UpdateSelected(UIChooseFormItem*NewSelected)
{
	m_SelectedItem = NewSelected;
	if (m_SelectedItem)
	{
		PropItemVec Vec;
        if (E.on_sel)
        {
			E.on_sel(m_SelectedItem->Object, Vec);
			m_Props->AssignItems(Vec);
        }
		if (E.flags.test(SChooseEvents::flClearTexture))
		{
			if (m_Texture)
				m_Texture->Release();
			m_Texture = 0;
		}
		if (!E.on_get_texture.empty())
			E.on_get_texture(m_SelectedItem->Object->name.c_str(), m_Texture);
	}

}
void UIChooseForm::FillItems(u32 choose_id)
{
    m_SelectedItems.clear();
    m_RootItem = UIChooseFormItem("");
    m_RootItem.Form = this;

    m_ChooseID = choose_id;

    int Index = 0;
   for (auto&i:m_Items)
    {
	   xr_string Name = i.name.c_str();
	   Name.append("*");
       xr_string RealName;
       if (strrchr(i.name.c_str(), '\\'))
       {
           RealName = strrchr(i.name.c_str(), '\\') + 1;
      }
       else
       {
           RealName = i.name.c_str();
       }
       UIChooseFormItem* Item = static_cast<UIChooseFormItem*> (m_RootItem.AppendItem(Name.c_str()));
       Item->Object = &i;
       Item->Text = RealName.c_str();
       Item->Index = Index++;
    }

    if (m_Flags.is(cfAllowNone) && !m_Flags.is(cfMultiSelect))
    {
        m_ItemNone.name = NONE_CAPTION;
        xr_string Name = m_ItemNone.name.c_str();
		Name.append("*");
		UIChooseFormItem* Item = static_cast<UIChooseFormItem*> (m_RootItem.AppendItem(Name.c_str()));
		Item->Object = &m_ItemNone;
		Item->Text = NONE_CAPTION;
		Item->Index = Index++;
    }
    m_RootItem.Sort();
}

void UIChooseForm::CheckFavorite()
{
    m_RootItem.CheckFavorited();
    m_SelectedItems.clear();
    m_RootItem.FillFavorited(m_SelectedItems);
}

UIChooseForm::UIChooseForm():m_Texture(nullptr),m_SelectedItem(nullptr), m_RootItem(""), m_SelectedList(-1)
{
    m_Search[0] = 0;
    m_FocusSearch = true;
    m_ScrollToSelected = false;
    m_DrawnFrame = -1;
    m_Shown = false;
    m_ChooseID = 0;
    m_Result = R_Cancel;
    m_Props = xr_new<UIPropertiesForm>();
    m_Props->SetChrome(UIPropertiesForm::chromeInline);
    m_Props->SetRole("chooser");
}
UIChooseForm::~UIChooseForm()
{
    if (m_Texture)
        m_Texture->Release();
    if (!E.on_close.empty())
    {
        E.on_close();
    }
    xr_delete(m_Props);
}

void UIChooseForm::Confirm()
{
    if (!m_Flags.is(cfMultiSelect))
    {
        SChooseItem* item = GetSelectedItem();
        if (!item)
            return;
        m_SelectedItems.clear();
        m_SelectedItems.push_back(item);
    }
    m_Result = R_Ok;
    bOpen = false;
    ImGui::CloseCurrentPopup();
}

// the dialog's body: the list with its search on the left, what the picked item is on the right
void UIChooseForm::Draw()
{
    if (m_SelectedItem&& E.flags.test(SChooseEvents::flAnimated))
    {
        if (!E.on_get_texture.empty())E.on_get_texture(m_SelectedItem->Object->name.c_str(), m_Texture);
    }
    if (m_AppliedSearch != m_Search)
    {
        m_AppliedSearch = m_Search;
        m_RootItem.UpdateFilter(m_Search);
    }

    const ImGuiStyle& style = ImGui::GetStyle();
    // tall enough to browse, never taller than the window leaves the dialog (its title, footer and margins)
    const float body_h = _max(XUI::Pt(200.f), _min(XUI::Pt(440.f), ImGui::GetMainViewport()->WorkSize.y - XUI::Pt(48.f + 36.f + 52.f + 24.f)));
    const float gap = XUI::Pt(12.f);
    const float avail_w = ImGui::GetContentRegionAvail().x;
    const float info_w = _min(XUI::Pt(240.f), avail_w * .4f);
    const float list_w = _max(XUI::Em(10.f), avail_w - info_w - gap);

    ImGui::BeginGroup();
    string64 hint;
    xr_sprintf(hint, "Search %u items", u32(m_Items.size()));
    if (m_FocusSearch)
    {
        ImGui::SetKeyboardFocusHere();
        m_FocusSearch = false;
    }
    ImGui::SetNextItemWidth(list_w);
    XUI::SearchField("##choose_search", m_Search, sizeof(m_Search), hint);
    const float list_h = _max(XUI::Pt(80.f), body_h - ImGui::GetFrameHeight() - style.ItemSpacing.y);
    if (ImGui::BeginChild("##choose_list", ImVec2(list_w, list_h), ImGuiChildFlags_Borders))
    {
        if (m_RootItem.Items.empty())
            XUI::EmptyState(XUI::Icon::Search, "Nothing to choose from");
        else if (!m_RootItem.Visible)
            XUI::EmptyState(XUI::Icon::Search, "No matches", "Nothing in the list matches the search.");
        else
            m_RootItem.DrawRoot();
    }
    ImGui::EndChild();
    ImGui::EndGroup();

    ImGui::SameLine(0.f, gap);
    ImGui::PushStyleColor(ImGuiCol_ChildBg, ImVec4(0.f, 0.f, 0.f, 0.f));
    if (ImGui::BeginChild("##choose_info", ImVec2(0.f, body_h), ImGuiChildFlags_None))
        DrawInfo();
    ImGui::EndChild();
    ImGui::PopStyleColor();
}

void UIChooseForm::DrawInfo()
{
    const float w = ImGui::GetContentRegionAvail().x;
    const float side = _min(w, XUI::Pt(192.f));
    ImTextureID picture = m_Texture ? m_Texture : NullTexture;
    if (picture)
        ImGui::Image(picture, ImVec2(side, side));
    else
    {
        // no picture for this kind of item: the square says so instead of staying blank
        const ImVec2 pos = ImGui::GetCursorScreenPos();
        ImDrawList* dl = ImGui::GetWindowDrawList();
        dl->AddRectFilled(pos, ImVec2(pos.x + side, pos.y + side), XUI_Local::Col(XUI_Local::Token::Canvas), XUI::Pt(4.f));
        const float icon = XUI::Pt(32.f);
        XUI::DrawIcon(dl, XUI::Icon::Picture, ImVec2(XUI_Local::Round(pos.x + (side - icon) * .5f), XUI_Local::Round(pos.y + (side - icon) * .5f)), 32.f,
            XUI_Local::Col(XUI_Local::Token::TextDisabled));
        ImGui::Dummy(ImVec2(side, side));
    }
    ImGui::Spacing();
    if (m_SelectedItem && m_SelectedItem->Object)
    {
        const SChooseItem* item = m_SelectedItem->Object;
        {
            XUI::FontScope strong(XUI::Font::Strong);
            XUI::TextEllipsis(item->name.c_str() ? item->name.c_str() : "", w, XUI::Clip::Start);
        }
        if (item->hint.size())
        {
            ImGui::PushTextWrapPos(ImGui::GetCursorPosX() + w);
            ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::Vec4(XUI_Local::Token::TextSecondary));
            ImGui::TextUnformatted(item->hint.c_str());
            ImGui::PopStyleColor();
            ImGui::PopTextWrapPos();
        }
    }
    else
        XUI_Local::MutedValue("Nothing selected");

    if (m_Flags.is(cfMultiSelect))
        DrawSelectedList();
    else if (!E.on_sel.empty() && m_SelectedItem)
    {
        ImGui::Spacing();
        m_Props->DrawGrid();
    }
}

void UIChooseForm::DrawSelectedList()
{
    ImGui::Spacing();
    const int count = int(m_SelectedItems.size());
    string64 caption;
    xr_sprintf(caption, "Selected (%d)", count);
    {
        XUI::FontScope caption_font(XUI::Font::Small);
        ImGui::AlignTextToFramePadding();
        ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::Vec4(XUI_Local::Token::TextSecondary));
        ImGui::TextUnformatted(caption);
        ImGui::PopStyleColor();
    }
    // the row's order is the order the result lists them in
    const float buttons = XUI::Pt(24.f) * 4.f + XUI::Pt(2.f) * 3.f;
    ImGui::SameLine();
    ImGui::SetCursorPosX(ImGui::GetCursorPosX() + _max(0.f, ImGui::GetContentRegionAvail().x - buttons));
    const bool has = m_SelectedList >= 0 && m_SelectedList < count;
    if (XUI::IconButton("##up", XUI::Icon::MoveUp, "Move up", false, has && m_SelectedList > 0))
    {
        std::swap(m_SelectedItems[m_SelectedList - 1], m_SelectedItems[m_SelectedList]);
        --m_SelectedList;
        m_ScrollToSelected = true;
    }
    ImGui::SameLine(0.f, XUI::Pt(2.f));
    if (XUI::IconButton("##down", XUI::Icon::MoveDown, "Move down", false, has && m_SelectedList < count - 1))
    {
        std::swap(m_SelectedItems[m_SelectedList], m_SelectedItems[m_SelectedList + 1]);
        ++m_SelectedList;
        m_ScrollToSelected = true;
    }
    ImGui::SameLine(0.f, XUI::Pt(2.f));
    if (XUI::IconButton("##remove", XUI::Icon::Minus, "Take it out of the selection", false, has))
    {
        m_SelectedItems.erase(m_SelectedItems.begin() + m_SelectedList);
        m_SelectedList = -1;
        m_RootItem.CheckFavorited(m_SelectedItems);
        CheckFavorite();
    }
    ImGui::SameLine(0.f, XUI::Pt(2.f));
    if (XUI::IconButton("##clear", XUI::Icon::Clear, "Clear the selection", false, count > 0))
    {
        m_SelectedItems.clear();
        m_RootItem.CheckFavorited(m_SelectedItems);
        CheckFavorite();
        m_SelectedList = -1;
    }
    if (XUI::BeginList("##selected", ImVec2(0.f, 0.f)))
    {
        if (!count)
            XUI_Local::MutedValue("Tick items in the list");
        for (int i = 0; i < count; ++i)
        {
            ImGui::PushID(i);
            if (XUI::ListRow("##row", m_SelectedList == i))
                m_SelectedList = i;
            if (m_ScrollToSelected && m_SelectedList == i)
            {
                ImGui::SetScrollHereY();
                m_ScrollToSelected = false;
            }
            XUI::TextEllipsis(m_SelectedItems[i]->name.c_str(), -1.f, XUI::Clip::Start);
            ImGui::PopID();
        }
    }
    XUI::EndList();
}

void UIChooseForm::SetNullTexture(ImTextureID Texture)
{
    NullTexture = Texture;
}
void UIChooseForm::Update()
{
    if (!Form)
    {
        // the form went away while its dialog was up (MCP picked or cancelled): the popup goes with it
        if (s_Closing)
        {
            s_Closing = false;
            bool open = false;
            if (XUI::BeginDialog(kClosingTitle, &open, XUI::DialogSize::L))
                XUI::EndDialog();
        }
        return;
    }
    if (Form->IsClosed())
        return;
    // several owners call Update: one dialog a frame
    const int frame = ImGui::GetFrameCount();
    if (Form->m_DrawnFrame == frame)
        return;
    Form->m_DrawnFrame = frame;
    const xr_string title = (Form->m_Title.empty() ? xr_string("Select") : Form->m_Title) + kDialogId;
    if (!XUI::BeginDialog(title.c_str(), &Form->bOpen, XUI::DialogSize::L))
        return;
    Form->m_Shown = true;
    XUI::BeginDialogBody();
    Form->Draw();
    xr_string status;
    if (Form->m_Flags.is(cfMultiSelect))
    {
        string64 text;
        xr_sprintf(text, "%u selected", u32(Form->m_SelectedItems.size()));
        status = text;
    }
    else if (SChooseItem* item = Form->GetSelectedItem())
        status = item->name.c_str() ? item->name.c_str() : "";
    if (XUI::BeginDialogFooter(status.c_str()))
    {
        const bool can_pick = Form->m_Flags.is(cfMultiSelect) || Form->GetSelectedItem();
        if (XUI::DialogButton("OK", XUI::Kind::Primary, true, can_pick, "Pick an item first"))
            Form->Confirm();
        if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
        {
            Form->m_Result = R_Cancel;
            Form->bOpen = false;
            ImGui::CloseCurrentPopup();
        }
    }
    XUI::EndDialog();
}
bool UIChooseForm::IsActive()
{
    return Form;
}

// The tree stores a leaf under "<path>*", so that is what the caller passes.
bool UIChooseForm::Pick(LPCSTR name)
{
    if (!Form || !name || !name[0]) return false;
    UITreeItem* Item = Form->m_RootItem.FindItem(name);
    if (!Item) return false;
    Form->m_RootItem.ClearSelection();
    ((UIChooseFormItem*)Item)->bSelected = true;
    Form->UpdateSelected((UIChooseFormItem*)Item);
    return true;
}

LPCSTR UIChooseForm::SelectedName()
{
    if (!Form || !Form->m_SelectedItem || !Form->m_SelectedItem->Object) return "";
    return Form->m_SelectedItem->Object->name.c_str();
}

bool UIChooseForm::CancelForm()
{
    if (!Form) return false;
    Form->m_Result = R_Cancel;
    Form->bOpen = false;
    return true;
}

bool UIChooseForm::PickAndConfirm(LPCSTR name)
{
    if (!Form) return false;
    if (name && name[0] && !Pick(name)) return false;
    if (!Form->GetSelectedItem()) return false;
    if (!Form->m_Flags.is(cfMultiSelect))
    {
        Form->m_SelectedItems.clear();
        Form->m_SelectedItems.push_back(Form->GetSelectedItem());
    }
    Form->m_Result = R_Ok;
    Form->bOpen = false;
    return true;
}

bool UIChooseForm::GetResult(bool& change, shared_str& result)
{
    if (!Form)return false;
    if (!Form->bOpen)
    {
        s_Closing = Form->m_Shown;
        if (Form->m_Result == R_Ok)
        {
            if (!Form->m_Flags.test(cfMultiSelect))
            {
                VERIFY(Form->GetSelectedItem());
            }
            xr_string reuslt_temp;
            int i = 0;
            for (auto& item : Form->m_SelectedItems)
            {
                if (i != 0)
                {
                    reuslt_temp.append(",");
                }
                reuslt_temp.append(item->name.c_str());
                i++;
            }
            result = reuslt_temp.c_str();
            change = true;
            xr_delete(Form);
            return true;
        }
        change = false;
        xr_delete(Form);
        return true;
   }

	return false;
}
SChooseItem* UIChooseForm::GetSelectedItem()
{
    if (m_Flags.test(cfMultiSelect))
    {
        if (m_SelectedItems.size())
            return m_SelectedItems.back();
        return nullptr;
    }
    if (m_SelectedItem)
        return m_SelectedItem->Object;
    return nullptr;
}
bool UIChooseForm::GetResult(bool& change, xr_string& result)
{
    if (!Form) return false;
    if (!Form->bOpen)
    {
        s_Closing = Form->m_Shown;
        if (Form->m_Result == R_Ok)
        {
            int i = 0;
            for (auto& item : Form->m_SelectedItems)
            {
                if (i != 0)
                {
                    result.append(",");
                }
                result.append(item->name.c_str());
                i++;
            }
            change = true;
            xr_delete(Form);
            return true;
        }
        change = false;
        xr_delete(Form);
        return true;
    }

    return false;
}
bool UIChooseForm::GetResult(bool& change, xr_vector<xr_string>& result)
{
    if (!Form) return false;
    if (!Form->bOpen)
    {
        s_Closing = Form->m_Shown;
        if (Form->m_Result == R_Ok)
        {
            for (auto& item : Form->m_SelectedItems)
            {
                result.push_back(item->name.c_str());
            }
            change = true;
            xr_delete(Form);
            return true;
        }
        change = false;
        xr_delete(Form);
        return true;
    }

    return false;
}
void UIChooseForm::SelectItem(u32 choose_ID, int sel_cnt, LPCSTR init_name, TOnChooseFillItems item_fill, void* fill_param, TOnChooseSelectItem item_select, ChooseItemVec* items, u32 mask)
{
    VERIFY(!Form);

    Form = xr_new<UIChooseForm>();
    Form->m_Flags.assign(mask);
    Form->m_Flags.set(cfMultiSelect, sel_cnt > 1);

    // fill items
    if (items)
    {
        VERIFY2(item_fill.empty(), "ChooseForm: Duplicate source.");
        Form->m_Items = *items;
        Form->E.Set("Select Item", 0, item_select, 0, 0, 0);
    }
    else if (!item_fill.empty())
    {
        // custom
        Form->E.Set("Select Item", item_fill, item_select, 0, 0, 0);
    }
    else
    {
        SChooseEvents* e = GetEvents(choose_ID); VERIFY2(e, "Can't find choose event.");
        Form->E = *e;
    }
    // set & fill


    Form->m_Title = Form->E.caption.c_str();
    if (!Form->E.on_fill.empty())
        Form->E.on_fill(Form->m_Items, fill_param);

    Form->FillItems(choose_ID);

    if (sel_cnt > 1)
    {
        int cnt = _GetItemCount(init_name);
        xr_string result;
        for (int i = 0; i < cnt; i++)
        {
            _GetItem(init_name, i, result);
            for (auto& item : Form->m_Items)
            {
                if (item.name.c_str() == result)
                {
                    Form->m_SelectedItems.push_back(&item);
                }
            }
        }
        Form->m_RootItem.CheckFavorited(Form->m_SelectedItems);
        Form->CheckFavorite();
    }
    else
    {
        if (init_name && init_name[0])
        {
            xr_string leaf = init_name;
            leaf.append("*");
            // the item the row holds now is picked, and its folders open to show it
            auto Item = Form->m_RootItem.FindItem(leaf.c_str());
            if (!Item)
                Item = Form->m_RootItem.FindItem(init_name);
            if (Item)
            {
                ((UIChooseFormItem*)Item)->bSelected = true;
                Form->UpdateSelected((UIChooseFormItem*)Item);
                for (UITreeItem* Shown = Item; Shown; Shown = Shown->Owner)
                    ((UIChooseFormItem*)Shown)->Reveal = true;
            }
        }
    }
}

void UIChooseForm::AppendEvents(u32 choose_ID, LPCSTR caption, TOnChooseFillItems on_fill, TOnChooseSelectItem on_sel, TGetTexture on_thm, TOnChooseClose on_close, u32 flags)
{
	EventsMapIt it = m_Events.find(choose_ID); VERIFY(it == m_Events.end());
	m_Events.insert(std::make_pair(choose_ID, SChooseEvents(caption, on_fill, on_sel, on_thm, on_close, flags)));

}

void UIChooseForm::ClearEvents()
{
    if (NullTexture)
        NullTexture->Release();
    NullTexture = nullptr;
    m_Events.clear();
}

SChooseEvents* UIChooseForm::GetEvents(u32 choose_ID)
{
	EventsMapIt it = m_Events.find(choose_ID);
	if (it != m_Events.end()) {
		return &it->second;
	}
	else return 0;
}
