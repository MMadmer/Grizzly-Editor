#include "stdafx.h"
UITextForm* UITextForm::Form = nullptr;
bool UITextForm::s_Closing = false;

namespace
{
constexpr const char* kTitle = "Edit Text###dlg.text_editor";

int TextResize(ImGuiInputTextCallbackData* data)
{
    if (data->EventFlag == ImGuiInputTextFlags_CallbackResize)
    {
        xr_vector<char>* text = static_cast<xr_vector<char>*>(data->UserData);
        text->resize(size_t(data->BufSize), 0);
        data->Buf = text->data();
    }
    return 0;
}
}

UITextForm::UITextForm(const char* text) :m_Text(text ? text : "")
{
    m_Ok = false;
    m_Shown = false;
    m_EditText.assign(m_Text.begin(), m_Text.end());
    m_EditText.resize(_max(m_Text.size() + 1, size_t(1024)), 0);
}

UITextForm::~UITextForm()
{
}

void UITextForm::Draw()
{
    if (!XUI::BeginDialog(kTitle, &bOpen, XUI::DialogSize::M, true))
        return;
    m_Shown = true;
    XUI::BeginDialogBody();
    {
        XUI::FontScope mono(XUI::Font::Mono);
        if (ImGui::IsWindowAppearing())
            ImGui::SetKeyboardFocusHere();
        ImGui::InputTextMultiline("##text", m_EditText.data(), m_EditText.size(), ImVec2(ImGui::GetContentRegionAvail().x, XUI::Pt(260.f)),
            ImGuiInputTextFlags_CallbackResize, TextResize, &m_EditText);
    }
    if (XUI::BeginDialogFooter())
    {
        if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
            CLBOk();
        if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
            CLBCancel();
    }
    XUI::EndDialog();
}

void UITextForm::RunEditor(const char* str)
{
    VERIFY(!Form);
    Form = xr_new< UITextForm>(str);
}

void UITextForm::Update()
{
    if (!Form)
    {
        // the form went away while its dialog was up: the popup goes with it
        if (s_Closing)
        {
            s_Closing = false;
            bool open = false;
            if (XUI::BeginDialog(kTitle, &open, XUI::DialogSize::M))
                XUI::EndDialog();
        }
        return;
    }
    if (!Form->IsClosed())
        Form->Draw();
}

bool UITextForm::GetResult(bool& change, xr_string& result)
{
    if (Form && Form->IsClosed())
    {
        change = Form->m_Ok;
        result = Form->m_Text;
        s_Closing = Form->m_Shown;
        xr_delete(Form);
        return true;
    }

    return false;
}

void UITextForm::CLBOk()
{
    m_Text = m_EditText.data();
    bOpen = false;
    m_Ok = true;
    ImGui::CloseCurrentPopup();
}


void UITextForm::CLBCancel()
{
    bOpen = false;
    ImGui::CloseCurrentPopup();
}
