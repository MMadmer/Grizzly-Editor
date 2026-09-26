#include "stdafx.h"
#pragma hdrstop

#include "UI_ViewportNavigation.h"
#include "UI_MainCommand.h"
#include "UI_ToolsCustom.h"
#include "ui_main.h"

namespace
{
ViewportNavigation::Target g_CurrentTarget = ViewportNavigation::Target::Perspective;
Fvector g_PerspectiveHPB;
Fvector g_PerspectivePosition;
Fvector g_PerspectiveTarget;
bool g_HasPerspectivePose = false;

bool MatchesOrientation(ViewportNavigation::Target target)
{
	if (!EDevice || target == ViewportNavigation::Target::Perspective)
		return true;

	Fvector expected;
	switch (target)
	{
	case ViewportNavigation::Target::Front:	expected.set(0.f, 0.f, 0.f); break;
	case ViewportNavigation::Target::Back:	expected.set(M_PI, 0.f, 0.f); break;
	case ViewportNavigation::Target::Left:	expected.set(-PI_DIV_2, 0.f, 0.f); break;
	case ViewportNavigation::Target::Right:	expected.set(PI_DIV_2, 0.f, 0.f); break;
	case ViewportNavigation::Target::Top:	expected.set(0.f, -PI_DIV_2, 0.f); break;
	case ViewportNavigation::Target::Bottom:	expected.set(0.f, PI_DIV_2, 0.f); break;
	default:							return false;
	}

	const Fvector& actual = EDevice->m_Camera.GetHPB();
	const float tolerance = deg2rad(.1f);
	return angle_difference(actual.x, expected.x) <= tolerance &&
		angle_difference(actual.y, expected.y) <= tolerance &&
		angle_difference(actual.z, expected.z) <= tolerance;
}

void RefreshCurrentTarget()
{
	if (!MatchesOrientation(g_CurrentTarget))
		g_CurrentTarget = ViewportNavigation::Target::Perspective;
}

// One end of an axis of the gizmo: the view a click on it gives
struct SAxisEnd
{
	ViewportNavigation::Target	target;
	ImVec2						pos;
	float						depth;		// along the camera's direction: the far ends are drawn first
	XFinedTheme::ColorToken		color;
	LPCSTR						label;		// the positive end carries its letter
};
}

bool ViewportNavigation::SetTarget(Target target)
{
	if (!EDevice || !UI || UI->IsPlayInEditor() || u32(target) >= u32(Target::Count))
		return false;

	RefreshCurrentTarget();
	const Target previous_target = g_CurrentTarget;

	if (target != Target::Perspective && previous_target == Target::Perspective)
	{
		g_PerspectiveHPB = EDevice->m_Camera.GetHPB();
		g_PerspectivePosition = EDevice->m_Camera.GetPosition();
		g_PerspectiveTarget = EDevice->m_Camera.GetTarget();
		g_HasPerspectivePose = true;
	}

	switch (target)
	{
	case Target::Perspective:
		if (previous_target != Target::Perspective && g_HasPerspectivePose)
			EDevice->m_Camera.Set(g_PerspectiveHPB, g_PerspectivePosition, g_PerspectiveTarget);
		break;
	case Target::Front:		EDevice->m_Camera.ViewFront(); break;
	case Target::Back:		EDevice->m_Camera.ViewBack(); break;
	case Target::Left:		EDevice->m_Camera.ViewLeft(); break;
	case Target::Right:		EDevice->m_Camera.ViewRight(); break;
	case Target::Top:			EDevice->m_Camera.ViewTop(); break;
	case Target::Bottom:		EDevice->m_Camera.ViewBottom(); break;
	default:					return false;
	}

	g_CurrentTarget = target;
	UI->RedrawScene();
	return true;
}

ViewportNavigation::Target ViewportNavigation::CurrentTarget()
{
	RefreshCurrentTarget();
	return g_CurrentTarget;
}

void ViewportNavigation::ResetState()
{
	g_CurrentTarget = Target::Perspective;
	g_HasPerspectivePose = false;
}

bool ViewportNavigation::TryParseTarget(LPCSTR value, Target& target)
{
	if (!value || !value[0])
		return false;
	if (0 == _stricmp(value, "reset"))
	{
		target = Target::Perspective;
		return true;
	}

	for (u32 i = 0; i < u32(Target::Count); ++i)
	{
		const Target candidate = Target(i);
		if (0 == _stricmp(value, TargetKey(candidate)) || 0 == _stricmp(value, TargetName(candidate)))
		{
			target = candidate;
			return true;
		}
	}
	return false;
}

LPCSTR ViewportNavigation::TargetKey(Target target)
{
	switch (target)
	{
	case Target::Perspective:	return "perspective";
	case Target::Front:		return "front";
	case Target::Back:		return "back";
	case Target::Left:		return "left";
	case Target::Right:		return "right";
	case Target::Top:			return "top";
	case Target::Bottom:		return "bottom";
	default:					return "perspective";
	}
}

LPCSTR ViewportNavigation::TargetName(Target target)
{
	switch (target)
	{
	case Target::Perspective:	return "Perspective";
	case Target::Front:		return "Front";
	case Target::Back:		return "Back";
	case Target::Left:		return "Left";
	case Target::Right:		return "Right";
	case Target::Top:			return "Top";
	case Target::Bottom:		return "Bottom";
	default:					return "Perspective";
	}
}

bool ViewportNavigation::FrameAll()
{
	if (!UI || !Tools || UI->IsPlayInEditor())
		return false;
	return ExecCommand(COMMAND_ZOOM_EXTENTS, FALSE);
}

bool ViewportNavigation::FrameSelection()
{
	if (!UI || !Tools || UI->IsPlayInEditor())
		return false;
	return ExecCommand(COMMAND_ZOOM_EXTENTS, TRUE);
}

bool ViewportNavigation::Draw(const ImVec2& canvas_position, const ImVec2& canvas_size)
{
	// a narrow view keeps its picture: the gizmo goes under 480 pt, the Perspective menu still has every view
	if (!EDevice || !UI || UI->IsPlayInEditor() || canvas_size.x < XUI::Pt(480.f))
		return false;
	const float size = XUI::Pt(64.f);
	// top right, under the row of pills
	const ImVec2 min(canvas_position.x + canvas_size.x - size - XUI::Pt(12.f), canvas_position.y + XUI::Pt(44.f));
	if (min.y + size > canvas_position.y + canvas_size.y - XUI::Pt(8.f))
		return false;
	RefreshCurrentTarget();

	// one item for the whole square, submitted before the canvas: it takes the mouse over itself
	ImGui::SetCursorScreenPos(min);
	const bool clicked = ImGui::InvisibleButton("##axis_gizmo", ImVec2(size, size));
	const bool hovered = ImGui::IsItemHovered();
	const bool owns_mouse = hovered || ImGui::IsItemActive();

	using Token = XFinedTheme::ColorToken;
	const ImVec2 centre(min.x + size * .5f, min.y + size * .5f);
	const float reach = size * .5f - XUI::Pt(9.f);
	const Fvector& right = EDevice->m_Camera.GetRight();
	const Fvector& up = EDevice->m_Camera.GetNormal();
	const Fvector& forward = EDevice->m_Camera.GetDirection();
	struct SAxis { Fvector dir; Target positive, negative; Token color; LPCSTR label; };
	// a view looks from the end it is named by: Right from +X, Top from +Y, Back from +Z
	const SAxis axes[] =
	{
		{ {1.f, 0.f, 0.f}, Target::Right,	Target::Left,	Token::AxisX, "X" },
		{ {0.f, 1.f, 0.f}, Target::Top,		Target::Bottom,	Token::AxisY, "Y" },
		{ {0.f, 0.f, 1.f}, Target::Back,	Target::Front,	Token::AxisZ, "Z" },
	};
	SAxisEnd ends[6];
	int n = 0;
	for (const SAxis& axis : axes)
	{
		// the world axis on the camera's screen basis
		const ImVec2 screen(axis.dir.dotproduct(right) * reach, -axis.dir.dotproduct(up) * reach);
		const float depth = axis.dir.dotproduct(forward);
		ends[n++] = { axis.positive, ImVec2(centre.x + screen.x, centre.y + screen.y), depth, axis.color, axis.label };
		ends[n++] = { axis.negative, ImVec2(centre.x - screen.x, centre.y - screen.y), -depth, axis.color, nullptr };
	}
	std::sort(ends, ends + n, [](const SAxisEnd& a, const SAxisEnd& b) { return a.depth > b.depth; });

	// the end under the mouse; the nearer one wins where two overlap
	const ImVec2 mouse = ImGui::GetIO().MousePos;
	const float grab = XUI::Pt(8.f);
	int hot = -1;
	if (hovered)
		for (int i = 0; i < n; ++i)
		{
			const float dx = mouse.x - ends[i].pos.x, dy = mouse.y - ends[i].pos.y;
			if (dx * dx + dy * dy <= grab * grab)
				hot = i;
		}

	ImDrawList* dl = ImGui::GetWindowDrawList();
	if (owns_mouse)
		dl->AddCircleFilled(centre, size * .5f, XFinedTheme::U32(Token::Overlay, .75f));
	ImFont* font = XUI::GetFont(XUI::Font::Small);
	if (!font)
		font = ImGui::GetFont();
	const float font_px = XUI::FontSize(XUI::Font::Small) * XUI::Dpi();
	for (int i = 0; i < n; ++i)
	{
		const SAxisEnd& end = ends[i];
		const ImU32 color = XFinedTheme::U32(end.color);
		if (end.label)
		{
			dl->AddLine(centre, end.pos, color, _max(1.f, XUI::Pt(2.f)));
			dl->AddCircleFilled(end.pos, XUI::Pt(7.f), color);
			const ImVec2 ts = font->CalcTextSizeA(font_px, FLT_MAX, 0.f, end.label);
			dl->AddText(font, font_px, ImVec2(floorf(end.pos.x - ts.x * .5f + .5f), floorf(end.pos.y - ts.y * .5f + .5f)),
				XFinedTheme::U32(Token::Canvas), end.label);
		}
		else
			dl->AddCircleFilled(end.pos, XUI::Pt(5.f), XFinedTheme::U32(end.color, .45f));
		if (i == hot)
			dl->AddCircle(end.pos, XUI::Pt(end.label ? 8.5f : 6.5f), XFinedTheme::U32(Token::Text), 0, _max(1.f, XUI::Pt(1.5f)));
	}
	dl->AddCircleFilled(centre, XUI::Pt(2.5f), XFinedTheme::U32(Token::TextSecondary));

	// a click on an end looks from it; anywhere else on the gizmo goes back to the free view
	const Target target = hot >= 0 ? ends[hot].target : Target::Perspective;
	char tip[64];
	if (target == Target::Perspective)
		xr_strcpy(tip, "Perspective: back to the free view");
	else
		sprintf_s(tip, "%s view", TargetName(target));
	XUI::Tip(tip);
	if (clicked)
		SetTarget(target);
	return owns_mouse;
}
