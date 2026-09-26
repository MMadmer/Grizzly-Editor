#include "stdafx.h"

UINumericVectorForm::UINumericVectorForm(const char* title, Fvector* data , Fvector* Reset, int decimal, Fvector* Min, Fvector* Max)
{
	m_Title = title ? title : "Vector";
	m_Title += "###dlg.numeric_vector";
	m_Out = data;
	m_Max = Max;
	m_Min = Min;
	m_Reset = Reset;
	m_Edit = *m_Out;
	m_Decimal = decimal;
}

UINumericVectorForm::~UINumericVectorForm()
{
}

void UINumericVectorForm::Draw()
{
	if (!XUI::BeginDialog(m_Title.c_str(), &bOpen, XUI::DialogSize::S))
		return;
	XUI::BeginDialogBody();
	float v[3] = { m_Edit.x, m_Edit.y, m_Edit.z };
	// the drag steps by the last decimal the vector keeps
	const float speed = m_Decimal > 0 ? powf(10.f, -float(_min(m_Decimal, 6))) : .1f;
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	if (XUI::Vec3Field("##vector", v, speed))
		m_Edit.set(v[0], v[1], v[2]);
	if (m_Max)
	{
		if (m_Max->x < m_Edit.x)
			m_Edit.x = m_Max->x;
		if (m_Max->y < m_Edit.y)
			m_Edit.y = m_Max->y;
		if (m_Max->z < m_Edit.z)
			m_Edit.z = m_Max->z;
	}
	if (m_Min)
	{
		if (m_Min->x > m_Edit.x)
			m_Edit.x = m_Min->x;
		if (m_Min->y > m_Edit.y)
			m_Edit.y = m_Min->y;
		if (m_Min->z > m_Edit.z)
			m_Edit.z = m_Min->z;
	}
	if (XUI::BeginDialogFooter())
	{
		if (m_Reset && XUI::DialogButton("Reset", XUI::Kind::Secondary))
			CLBReset();
		if (XUI::DialogButton("OK", XUI::Kind::Primary, true))
			CLBOk();
		if (XUI::DialogButton("Cancel", XUI::Kind::Secondary))
			CLBCancel();
	}
	XUI::EndDialog();
}

void UINumericVectorForm::CLBOk()
{
	*m_Out = m_Edit;
	bOpen = false;
	ImGui::CloseCurrentPopup();
}

void UINumericVectorForm::CLBCancel()
{
	bOpen = false;
	ImGui::CloseCurrentPopup();
}

void UINumericVectorForm::CLBReset()
{
	m_Edit = *m_Reset;
}
