#include "stdafx.h"
UIPropertiesModal* UIPropertiesModal::Form = nullptr;
bool UIPropertiesModal::s_Closing = false;

namespace
{
constexpr const char* kDialogId = "###dlg.properties";
constexpr const char* kClosingTitle = "Properties###dlg.properties";
}

UIPropertiesModal::UIPropertiesModal()
{
	m_Props = xr_new<UIPropertiesForm>();
	m_Props->SetRole("dialog");
	m_Props->SetChrome(UIPropertiesForm::chromeInline);
	m_Result = R_Cancel;
	m_Shown = false;
}

UIPropertiesModal::~UIPropertiesModal()
{
	xr_delete(m_Props);
}

void UIPropertiesModal::Draw()
{
	const xr_string title = m_Title + kDialogId;
	if (!XUI::BeginDialog(title.c_str(), &bOpen, XUI::DialogSize::M))
		return;
	m_Shown = true;
	XUI::BeginDialogBody();
	m_Props->DrawGrid();
	if (XUI::BeginDialogFooter())
	{
		if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
		{
			m_Result = R_Ok;
			bOpen = false;
			ImGui::CloseCurrentPopup();
		}
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
		{
			bOpen = false;
			ImGui::CloseCurrentPopup();
		}
	}
	XUI::EndDialog();
}

void UIPropertiesModal::Update()
{
	if (!Form)
	{
		// the form went away while its dialog was up: the popup goes with it
		if (s_Closing)
		{
			s_Closing = false;
			bool open = false;
			if (XUI::BeginDialog(kClosingTitle, &open, XUI::DialogSize::M))
				XUI::EndDialog();
		}
		return;
	}
	if (!Form->IsClosed())
		Form->Draw();
}

bool UIPropertiesModal::GetResult(bool& ok)
{
	if (Form && !Form->bOpen)
	{
		ok = Form->m_Result == R_Ok;
		s_Closing = Form->m_Shown;
		xr_delete(Form);
		return true;
	}
	return false;
}

void UIPropertiesModal::Show(PropItemVec& items, LPCSTR title)
{
	VERIFY(!Form);
	Form = xr_new<UIPropertiesModal>();
	Form->m_Title = title && title[0] ? title : "Properties";
	Form->m_Props->AssignItems(items);
}
