#include "stdafx.h"
#include "UIRenderForm.h"
#include "UI_ViewportNavigation.h"
#include "ui_main.h"
#include "EditorUiContract.h"

namespace
{
// Stats (Show > Stats, off unless asked for): what the last pass drew, as a Mono pill under the left pills
void DrawStatsPill(const ImVec2& canvas_pos, const ImVec2& canvas_size)
{
	if (!psDeviceFlags.is(rsStatistic) || !EDevice || !EDevice->EStatistic)
		return;
	const CEStats::SShown& s = EDevice->EStatistic->Shown;
	char lines[6][64];
	sprintf_s(lines[0], "FPS         %.0f", s.fps);
	sprintf_s(lines[1], "Render FPS  %.0f", s.render_fps);
	sprintf_s(lines[2], "Triangles   %u", s.polys);
	sprintf_s(lines[3], "Vertices    %u", s.verts);
	sprintf_s(lines[4], "Draw calls  %u", s.calls);
	sprintf_s(lines[5], "Lights      %u of %u", s.lights_scene, s.lights_total);

	XUI::FontScope mono(XUI::Font::Mono);
	const float line = ImGui::GetTextLineHeight();
	const float pad = XUI::Pt(8.f);
	// under the left pills; as many lines as the view has room for, and none on a view too narrow for them
	const ImVec2 at(canvas_pos.x + XUI::Pt(8.f), canvas_pos.y + XUI::Pt(44.f));
	const int n = _max(0, _min(6, int((canvas_pos.y + canvas_size.y - XUI::Pt(8.f) - at.y - pad * 2.f) / line)));
	float w = 0.f;
	for (int i = 0; i < n; ++i)
		w = _max(w, ImGui::CalcTextSize(lines[i]).x);
	if (!n || w + pad * 2.f > canvas_size.x - XUI::Pt(16.f))
		return;
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + w + pad * 2.f, at.y + n * line + pad * 1.5f),
		XFinedTheme::U32(XFinedTheme::ColorToken::Overlay, .92f), XUI::Pt(6.f));
	for (int i = 0; i < n; ++i)
		dl->AddText(ImVec2(at.x + pad, at.y + pad * .75f + i * line), XFinedTheme::U32(XFinedTheme::ColorToken::TextSecondary), lines[i]);
}
}

UIRenderForm::UIRenderForm()
{
	m_mouse_down = false;
	m_mouse_move = false;
	m_shiftstate_down = false;
	m_overlay_mouse_down = false;
}

UIRenderForm::~UIRenderForm()
{

}

void UIRenderForm::Draw()
{
	// the centre of the Level workspace: no tab bar, nothing docks over it, the picture edge to edge
	XUI::Shell::NextWindowIsCanvas();
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, 0.f));
	const bool visible = ImGui::Begin(EditorUiContract::kWindowViewport, nullptr, ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	ImGui::PopStyleVar();
	// a hidden viewport keeps its render target's size: no resize, no device reset for nothing
	if (visible)
		DrawCanvas();
	ImGui::End();
}

void UIRenderForm::DrawCanvas()
{
	if (UI && UI->RT->pSurface)
	{
		int ShiftState = ssNone;

		if (ImGui::GetIO().KeyShift)ShiftState |= ssShift;
		if (ImGui::GetIO().KeyCtrl)ShiftState |= ssCtrl;
		if (ImGui::GetIO().KeyAlt)ShiftState |= ssAlt;


		if (ImGui::IsMouseDown(ImGuiMouseButton_Left))ShiftState |= ssLeft;
		if (ImGui::IsMouseDown(ImGuiMouseButton_Right))ShiftState |= ssRight;
		if (ImGui::IsMouseDown(ImGuiMouseButton_Middle))ShiftState |= ssMiddle;
		//VERIFY(!(ShiftState & ssLeft && ShiftState & ssRight));
		ImDrawList* draw_list = ImGui::GetWindowDrawList();
		ImVec2 canvas_pos = ImGui::GetCursorScreenPos();
		ImVec2 canvas_size = ImGui::GetContentRegionAvail();
		ImVec2 mouse_pos = ImGui::GetIO().MousePos;
		bool cursor_in_zone = true;
		if (mouse_pos.x < canvas_pos.x)
		{
			cursor_in_zone = false;
			mouse_pos.x = canvas_pos.x;
		}
		if (mouse_pos.y < canvas_pos.y)
		{
			cursor_in_zone = false;
			mouse_pos.y = canvas_pos.y;
		}

		if (mouse_pos.x > canvas_pos.x + canvas_size.x)
		{
			cursor_in_zone = false;
			mouse_pos.x = canvas_pos.x + canvas_size.x;
		}
		if (mouse_pos.y > canvas_pos.y + canvas_size.y)
		{
			cursor_in_zone = false;
			mouse_pos.y = canvas_pos.y + canvas_size.y;
		}

		bool curent_shiftstate_down = m_shiftstate_down;


		if (canvas_size.x < 32.0f) canvas_size.x = 32.0f;
		if (canvas_size.y < 32.0f) canvas_size.y = 32.0f;
		UI->RTSize.set(canvas_size.x, canvas_size.y);

		// The picture is the render size's corner of the target: a target allocated larger (grown in 64 px steps, so a
		// splitter drag does not reset the device every pixel) shows only what was rendered.
		const float target_w = float(UI->RT->dwWidth), target_h = float(UI->RT->dwHeight);
		const ImVec2 uv1(target_w > 0.f ? _min(1.f, float(UI->GetRenderWidth()) / target_w) : 1.f,
			target_h > 0.f ? _min(1.f, float(UI->GetRenderHeight()) / target_h) : 1.f);

		ImGui::SetCursorScreenPos(canvas_pos);
#if defined(USE_DX11)
		// the RT texture is not drawable on its own - CRT keeps a CTexture around
		// it, and that is where the shader resource view ImGui wants lives
		draw_list->AddImage(UI->RT->pTexture->get_SRView(), canvas_pos, ImVec2(canvas_pos.x + canvas_size.x, canvas_pos.y + canvas_size.y),
			ImVec2(0.f, 0.f), uv1);
#else
		draw_list->AddImage(UI->RT->pSurface, canvas_pos, ImVec2(canvas_pos.x + canvas_size.x, canvas_pos.y + canvas_size.y),
			ImVec2(0.f, 0.f), uv1);
#endif

		if(m_OnToolBar)
			m_OnToolBar(canvas_size);
		const bool navigation_owns_mouse = ViewportNavigation::Draw(canvas_pos, canvas_size);
		DrawStatsPill(canvas_pos, canvas_size);

		ImGui::SetCursorScreenPos(canvas_pos);
		// accept every button: the default left-only button never activates on
		// RMB/MMB, which used to leave the viewport unfocused and swallow
		// camera navigation until the user left-clicked it first
		ImGui::InvisibleButton("canvas", canvas_size,
			ImGuiButtonFlags_MouseButtonLeft | ImGuiButtonFlags_MouseButtonRight | ImGuiButtonFlags_MouseButtonMiddle);

		const bool any_btn_down =
			ImGui::IsMouseDown(ImGuiMouseButton_Left) ||
			ImGui::IsMouseDown(ImGuiMouseButton_Right) ||
			ImGui::IsMouseDown(ImGuiMouseButton_Middle);
		const bool any_btn_released =
			ImGui::IsMouseReleased(ImGuiMouseButton_Left) ||
			ImGui::IsMouseReleased(ImGuiMouseButton_Right) ||
			ImGui::IsMouseReleased(ImGuiMouseButton_Middle);
		if (navigation_owns_mouse && any_btn_down)
			m_overlay_mouse_down = true;
		const bool overlay_owns_interaction = navigation_owns_mouse || m_overlay_mouse_down;

		// gate on hover, not focus - a drag started here keeps running even
		// after the cursor leaves the viewport
		const bool canvas_hovered = ImGui::IsItemHovered() && !overlay_owns_interaction;
		if (canvas_hovered || m_mouse_down)
		{

			if (any_btn_down && !m_mouse_down&& cursor_in_zone)
			{
				ImGui::SetWindowFocus();
				UI->MousePress(TShiftState(ShiftState), mouse_pos.x - canvas_pos.x, mouse_pos.y - canvas_pos.y);
				m_mouse_down = true;
			}

			else  if (any_btn_released && m_mouse_down)
			{
				if (!any_btn_down)
				{
					UI->MouseRelease(TShiftState(ShiftState), mouse_pos.x - canvas_pos.x, mouse_pos.y - canvas_pos.y);
					m_mouse_down = false;
					m_mouse_move = false;
					m_shiftstate_down = false;
				}
			}
			else if (m_mouse_down)
			{
				UI->MouseMove(TShiftState(ShiftState), mouse_pos.x - canvas_pos.x, mouse_pos.y - canvas_pos.y);
				m_mouse_move = true;
				m_shiftstate_down = m_shiftstate_down||( ShiftState & (ssShift | ssCtrl | ssAlt));
			}
		}
		else  if (m_mouse_down)
		{
			if (!any_btn_down)
			{
				UI->MouseRelease(TShiftState(ShiftState), mouse_pos.x - canvas_pos.x, mouse_pos.y - canvas_pos.y);
				m_mouse_down = false;
				m_mouse_move = false;
				m_shiftstate_down = false;
			}
		}
		m_mouse_position.set(mouse_pos.x - canvas_pos.x, mouse_pos.y - canvas_pos.y);

		// asset dropped from the content browser - refresh the pick ray first so
		// the handler can raycast against whatever is under the cursor
		if (!overlay_owns_interaction && ImGui::BeginDragDropTarget())
		{
			if (const ImGuiPayload* pl = ImGui::AcceptDragDropPayload("XRAY_ASSET"))
			{
				if (!m_OnDropAsset.empty() && pl->Data)
				{
					EDevice->m_Camera.MouseRayFromPoint(UI->m_CurrentRStart, UI->m_CurrentRDir, m_mouse_position);
					m_OnDropAsset((LPCSTR)pl->Data);
				}
			}
			ImGui::EndDragDropTarget();
		}

		// wheel over the viewport: dolly, or speed tune while flying
		const float wheel = ImGui::GetIO().MouseWheel;
		if (wheel != 0.f && (canvas_hovered || m_mouse_down))
			UI->MouseWheel(TShiftState(ShiftState), wheel);

		// no context menu when the RMB press was camera navigation
		if (!overlay_owns_interaction && !m_OnContextMenu.empty()&& !curent_shiftstate_down && !EDevice->m_Camera.WasNavInput())
		{
			if (ImGui::BeginPopupContextItem("Menu"))
			{
				m_OnContextMenu();
				ImGui::EndPopup();
			}
		}
		if (!any_btn_down)
			m_overlay_mouse_down = false;


	}
}
