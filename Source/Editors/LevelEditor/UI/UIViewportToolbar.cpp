#include "stdafx.h"
#include "..\..\XrECore\Editor\UI_ViewportNavigation.h"

// The viewport's pills (docs/UI_DESIGN.md, section 6, Viewport): the view options, the camera view and what is shown
// on the left; the transform tool, object snap and the grid, scale and angle snaps on the right. One overlay toolbar
// over the top of the picture, so a narrow viewport folds the snaps into its overflow menu instead of cutting them.
namespace
{
// the menus MCP puts up (ui_popup viewport:<name>) and where each opens: under its button as last drawn
enum EMenu { mOptions = 0, mPerspective, mShow, mSnap, mGrid, mScale, mAngle, mCount };
const char* const kMenuName[mCount] = { "options", "perspective", "show", "snap", "grid", "scale", "angle" };
ImVec2 s_Anchor[mCount];

// A toolbar menu (Toolbar::Menu) asked for over MCP opens where a click opens it: its popup is "##menu" under the
// item's id, in the toolbar's own id scope.
void TakeMenuRequest(int menu)
{
	char popup[48];
	sprintf_s(popup, "viewport:%s", kMenuName[menu]);
	if (!XUI::Shell::TakePopup(popup))
		return;
	ImGui::PushID(kMenuName[menu]);
	XUI::Shell::OpenPopupAt("##menu", s_Anchor[menu]);
	ImGui::PopID();
}

// the item just drawn was this menu's button (not moved into the overflow): its menu opens under it
void NoteAnchor(int menu)
{
	if (ImGui::GetItemID() == ImGui::GetID(kMenuName[menu]))
		s_Anchor[menu] = ImVec2(ImGui::GetItemRectMin().x, ImGui::GetItemRectMax().y + XUI::Pt(2.f));
}

// Toolbar::Menu with the MCP request and the anchor around it; true while the menu is open (EndPopup follows)
bool PillMenu(XUI::Toolbar& tb, int menu, XUI::Icon icon, LPCSTR label, LPCSTR tip)
{
	TakeMenuRequest(menu);
	if (tb.Menu(kMenuName[menu], icon, label, tip))
		return true;
	NoteAnchor(menu);
	return false;
}

// The keys bound to a command now, for a menu item's keycaps (the main menu's way): never a literal that could drift
// from the binding. nullptr when nothing is bound.
LPCSTR BoundKeys(u32 command, u32 sub = 0)
{
	static xr_string s_Labels[4];
	static int s_Next = 0;
	ECommandVec& cmds = GetEditorCommands();
	if (command >= cmds.size() || !cmds[command] || sub >= cmds[command]->sub_commands.size())
		return nullptr;
	const xr_shortcut& shortcut = cmds[command]->sub_commands[sub]->shortcut;
	if (!shortcut.key)
		return nullptr;
	// a few labels live at once: one menu asks for several
	xr_string& label = s_Labels[s_Next++ & 3];
	label = XUI::ShortcutLabel(shortcut);
	return label.empty() ? nullptr : label.c_str();
}

bool StepItem(LPCSTR label, float value, float current)
{
	return XUI::MenuItem(label, nullptr, fabsf(value - current) < value * 1e-4f);
}

// a snap: its toggle and its step, the step picked from the steps the editor has always offered
void DrawSnap(XUI::Toolbar& tb, int menu, Gizmo::EType type, XUI::Icon icon, LPCSTR toggle_id, LPCSTR name, LPCSTR tip,
	const float* steps, int count, bool degrees)
{
	Gizmo* gizmo = LTools->GetGimzo();
	const bool on = gizmo->IsStepEnable(type);
	if (tb.Action(toggle_id, icon, "", tip, true, on))
		gizmo->SwitchStep(type, !on);

	char value[32];
	const float step = gizmo->GetStep(type);
	// %g: 5.625 reads 5.625 and 0.1 reads 0.1, never 5.62500
	sprintf_s(value, degrees ? "%g\xC2\xB0" : "%g", step);
	char step_tip[64];
	sprintf_s(step_tip, "%s step", name);
	if (PillMenu(tb, menu, XUI::Icon::None, value, step_tip))
	{
		for (int i = 0; i < count; ++i)
		{
			char label[32];
			sprintf_s(label, degrees ? "%g\xC2\xB0" : "%g", steps[i]);
			if (StepItem(label, steps[i], step))
				gizmo->SetStep(type, steps[i]);
		}
		ImGui::EndPopup();
	}
}

void DrawPerspectiveMenu()
{
	using namespace ViewportNavigation;
	const Target current = CurrentTarget();
	if (XUI::MenuItem(TargetName(Target::Perspective), nullptr, current == Target::Perspective))
		SetTarget(Target::Perspective);
	XUI::Tip("Back to the free view the camera had before an axis view");
	ImGui::Separator();
	const Target axes[] = { Target::Front, Target::Back, Target::Left, Target::Right, Target::Top, Target::Bottom };
	for (Target target : axes)
		if (XUI::MenuItem(TargetName(target), nullptr, current == target))
			SetTarget(target);
	ImGui::Separator();
	if (XUI::MenuItem("Frame All", BoundKeys(COMMAND_ZOOM_EXTENTS, 0)))
		FrameAll();
	XUI::Tip("Every visible object in view");
	if (XUI::MenuItem("Frame Selection", BoundKeys(COMMAND_ZOOM_EXTENTS, 1)))
		FrameSelection();
	XUI::Tip("The selected objects in view");
}

void DrawShowMenu()
{
	// `command` toggles the same flag and lends the item its keycaps; u32(-1): there is none
	struct SFlag { LPCSTR label; u32 mask; LPCSTR tip; u32 command; };
	const SFlag flags[] =
	{
		{ "Stats",		rsStatistic,	"Frame rate and what the last frame drew, over the picture",	u32(-1) },
		{ "Grid",		rsDrawGrid,		"The ground grid",											COMMAND_TOGGLE_GRID },
		{ "Safe Rect",	rsDrawSafeRect,	"The frame the game's screen shows",						COMMAND_TOGGLE_SAFE_RECT },
	};
	for (const SFlag& f : flags)
	{
		bool selected = !!psDeviceFlags.test(f.mask);
		if (XUI::MenuItem(f.label, BoundKeys(f.command), &selected))
		{
			psDeviceFlags.set(f.mask, selected);
			UI->RedrawScene();
		}
		XUI::Tip(f.tip);
	}
}

void DrawSnapMenu()
{
	struct SSetting { LPCSTR label; u32 mask; LPCSTR tip; };
	const SSetting settings[] =
	{
		{ "Snap to Objects",			etfOSnap,		"Placed and moved objects land on the surface of other objects" },
		{ "Snap to Vertices",			etfVSnap,		"On an object, to its nearest vertex" },
		{ "Align to Surface Normal",	etfNormalAlign,	"A placed object stands along the surface it lands on" },
	};
	for (const SSetting& s : settings)
	{
		const bool on = !!Tools->GetSettings(s.mask);
		if (XUI::MenuItem(s.label, nullptr, on))
			ExecCommand(COMMAND_SET_SETTINGS, s.mask, !on);
		XUI::Tip(s.tip);
	}
}
}

void UIMainForm::DrawRenderToolBar(ImVec2 Size)
{
	// the tool's controls tell what it can do; a transform it cannot do falls back to plain selection
	bool bIsSupportMove = false;
	bool bIsSupportRotate = false;
	bool bIsSupportScale = false;
	const ETAction Action = LTools->GetAction();
	if (ESceneToolBase* tool = Scene->GetTool(LTools->CurrentClassID()))
		if (auto* control = tool->FindControl(0, etaSelect))
		{
			bIsSupportMove = control->IsSupportMove();
			bIsSupportRotate = control->IsSupportRotate();
			bIsSupportScale = control->IsSupportScale();
		}
	Gizmo* gizmo = LTools->GetGimzo();
	if (Action == etaSelect && ((!bIsSupportMove && gizmo->GetType() == Gizmo::EType::Move) ||
		(!bIsSupportRotate && gizmo->GetType() == Gizmo::EType::Rotate) || (!bIsSupportScale && gizmo->GetType() == Gizmo::EType::Scale)))
		gizmo->SetType(Gizmo::EType::None);

	// a one-cell table gives the row the canvas less a margin on either side: the pills never touch the edges
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const float margin = XUI::Pt(8.f);
	ImGui::SetCursorScreenPos(ImVec2(origin.x + margin, origin.y + XUI::Pt(4.f)));
	ImGui::PushStyleVar(ImGuiStyleVar_CellPadding, ImVec2(0.f, 0.f));
	const ImGuiTableFlags table_flags = ImGuiTableFlags_NoClip | ImGuiTableFlags_NoSavedSettings | ImGuiTableFlags_NoPadOuterX |
		ImGuiTableFlags_NoPadInnerX;
	if (ImGui::BeginTable("##viewport_pills", 1, table_flags, ImVec2(_max(XUI::Pt(24.f), Size.x - margin * 2.f), 0.f)))
	{
		ImGui::TableNextRow();
		ImGui::TableNextColumn();
		// the toolbar lays itself out when it goes out of scope: inside the cell
		{
			XUI::Toolbar tb("viewport", XUI::ToolbarOverlay);

			tb.Group(3);
			if (PillMenu(tb, mOptions, XUI::Icon::ViewOptions, "", "View options"))
			{
				UIMainMenuForm::DrawViewportOptions();
				ImGui::Separator();
				if (XUI::MenuItem("Maximize Viewport", "F11", ViewportMaximized()))
					ToggleViewportMaximized();
				ImGui::EndPopup();
			}
			const ViewportNavigation::Target view = ViewportNavigation::CurrentTarget();
			if (PillMenu(tb, mPerspective, view == ViewportNavigation::Target::Perspective ? XUI::Icon::Perspective : XUI::Icon::Ortho,
				ViewportNavigation::TargetName(view), "Camera view, Frame All and Frame Selection"))
			{
				DrawPerspectiveMenu();
				ImGui::EndPopup();
			}
			if (PillMenu(tb, mShow, XUI::Icon::Eye, "Show", "What the viewport draws over the scene"))
			{
				DrawShowMenu();
				ImGui::EndPopup();
			}

			tb.Spring();

			tb.Group(2);
			const Gizmo::EType type = gizmo->GetType();
			char tool_name[96];
			sprintf_s(tool_name, "The %s tool", ToolClassTitle(LTools->CurrentClassID()));
			if (tb.Action("tool_select", XUI::Icon::Select, "", "Select", true, Action == etaSelect && type == Gizmo::EType::None))
			{
				LTools->SetAction(etaSelect);
				gizmo->SetType(Gizmo::EType::None);
			}
			if (tb.Action("tool_place", XUI::Icon::Place, "", "Place: a click in the viewport adds what the Tools panel has picked",
				true, Action == etaAdd))
			{
				LTools->SetAction(etaAdd);
				gizmo->SetType(Gizmo::EType::None);
			}
			struct STransform { LPCSTR id; XUI::Icon icon; LPCSTR tip; LPCSTR cannot; Gizmo::EType type; bool on; };
			const STransform transforms[] =
			{
				{ "tool_move",	XUI::Icon::Move,	"Move the selection",	"cannot move objects",		Gizmo::EType::Move,		bIsSupportMove },
				{ "tool_rotate",	XUI::Icon::Rotate,	"Rotate the selection",	"cannot rotate objects",	Gizmo::EType::Rotate,	bIsSupportRotate },
				{ "tool_scale",	XUI::Icon::Scale,	"Scale the selection",	"cannot scale objects",		Gizmo::EType::Scale,	bIsSupportScale },
			};
			for (const STransform& t : transforms)
			{
				// a disabled button's tooltip says why it is off
				char tip[128];
				if (t.on)
					xr_strcpy(tip, t.tip);
				else
					sprintf_s(tip, "%s %s", tool_name, t.cannot);
				if (tb.Action(t.id, t.icon, "", tip, t.on, Action == etaSelect && type == t.type))
				{
					LTools->SetAction(etaSelect);
					gizmo->SetType(t.type);
				}
			}

			tb.Group(1);
			const bool osnap = !!Tools->GetSettings(etfOSnap);
			if (tb.Action("osnap", XUI::Icon::SnapObject, "", "Object Snap: placed and moved objects land on other objects", true, osnap))
				ExecCommand(COMMAND_SET_SETTINGS, etfOSnap, !osnap);
			if (PillMenu(tb, mSnap, XUI::Icon::None, "", "Snap options"))
			{
				DrawSnapMenu();
				ImGui::EndPopup();
			}

			static const float kMoveSteps[] = { .01f, .05f, .1f, 1.f, 2.f, 5.f, 50.f, 200.f, 1000.f };
			static const float kScaleSteps[] = { .01f, .05f, .1f, 1.f, 2.f, 5.f, 50.f, 200.f, 1000.f };
			static const float kAngleSteps[] = { 1.40625f, 2.8125f, 5.625f, 11.25f, 22.5f, 45.f, 90.f };
			tb.Group(0);
			DrawSnap(tb, mGrid, Gizmo::EType::Move, XUI::Icon::SnapGrid, "grid_snap", "Grid",
				"Grid Snap: movement goes in steps", kMoveSteps, _countof(kMoveSteps), false);
			tb.Group(0);
			DrawSnap(tb, mScale, Gizmo::EType::Scale, XUI::Icon::SnapScale, "scale_snap", "Scale",
				"Scale Snap: scaling goes in steps", kScaleSteps, _countof(kScaleSteps), false);
			tb.Group(0);
			DrawSnap(tb, mAngle, Gizmo::EType::Rotate, XUI::Icon::SnapAngle, "angle_snap", "Angle",
				"Angle Snap: rotation goes in steps", kAngleSteps, _countof(kAngleSteps), true);
		}
		ImGui::EndTable();
	}
	ImGui::PopStyleVar();
}
