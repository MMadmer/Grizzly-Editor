//---------------------------------------------------------------------------
#include "stdafx.h"
#pragma hdrstop

#include "ui_main.h"
#include "ui_toolscustom.h"
#include "..\..\XrEProps\UIPropsLocal.h"
//---------------------------------------------------------------------------
CCustomPreferences* EPrefs=0;
//---------------------------------------------------------------------------

namespace
{
constexpr const char* kKeyboardPage = "Keyboard";
// the nav list's icons for the pages this class fills; General is Mod Settings' General too
struct SPageIcon { const char* page; XUI::Icon icon; };
const SPageIcon kPageIcons[] =
{
	{ "General", XUI::Icon::Info },
	{ "Viewport", XUI::Icon::Perspective },
	{ "Tools", XUI::Icon::Move },
	{ "Objects", XUI::Icon::SceneObject },
	{ kKeyboardPage, XUI::Icon::Keyboard },
};
// the separator the Keyboard page puts between the parts of a command's name
constexpr const char* kPathDot = " \xC2\xB7 ";

// what the Preferences rows do: their tooltip, the help under the grid, and MCP; and the unit a number is shown in
// (angles are in degrees by themselves)
struct SPrefHelp { const char* key; const char* help; const char* unit; };
const SPrefHelp kPrefHelp[] =
{
	{ "General\\Theme", "The editor's colours.", nullptr },
	{ "General\\Allow Panels Outside the Window", "A panel can be dragged out of the editor into a window of its own. Takes effect after a restart.", nullptr },
	{ "General\\Frame Rate Limit", "The most frames a second the editor draws while its window is in front.", "fps" },
	{ "General\\Recent Files", "How many scenes File > Recent Scene remembers.", nullptr },
	{ "General\\Undo Steps", "How many scene changes Undo can take back.", nullptr },
	{ "Viewport\\Camera\\Move Sensitivity", "How far the camera moves for a mouse movement.", nullptr },
	{ "Viewport\\Camera\\Rotate Sensitivity", "How far the camera turns for a mouse movement.", nullptr },
	{ "Viewport\\Camera\\Fly Speed", "The camera's speed in fly mode.", "m/s" },
	{ "Viewport\\Camera\\Fly Altitude", "The height above the ground the camera keeps in fly mode.", "m" },
	{ "Viewport\\Field of View", "The camera's vertical field of view.", nullptr },
	{ "Viewport\\Near Plane", "Nothing closer to the camera than this is drawn.", "m" },
	{ "Viewport\\Far Plane", "Nothing farther from the camera than this is drawn.", "m" },
	{ "Viewport\\Render Radius", "Scene objects farther than this from the camera are not drawn.", "m" },
	{ "Viewport\\Background Color", "The colour behind everything the viewport draws.", nullptr },
	{ "Viewport\\Fog\\Color", "The colour distant things fade into.", nullptr },
	{ "Viewport\\Fog\\Density", "How quickly distant things fade into the fog.", nullptr },
	{ "Viewport\\Grid\\Cell Size", "The side of a grid cell.", "m" },
	{ "Viewport\\Grid\\Cell Count", "How many cells the grid has along each side.", nullptr },
	{ "Tools\\Gizmo\\Move Sensitivity", "How far an object moves for a mouse movement.", nullptr },
	{ "Tools\\Gizmo\\Rotate Sensitivity", "How far an object turns for a mouse movement.", nullptr },
	{ "Tools\\Gizmo\\Scale Sensitivity", "How much an object grows for a mouse movement.", nullptr },
	{ "Tools\\Gizmo\\Show Move Axes", "Draws the move gizmo's axes on the object being moved.", nullptr },
	{ "Tools\\Snap\\Grid Step", "How far a move goes per step while Grid Snap is on. The viewport toolbar and the status bar show this step.", "m" },
	{ "Tools\\Snap\\Angle Step", "How far a rotation turns per step while Angle Snap is on. The viewport toolbar and the status bar show this step.", nullptr },
	{ "Tools\\Snap\\Scale Step", "How much scaling changes per step while Scale Snap is on. The viewport toolbar shows this step.", nullptr },
	{ "Tools\\Snap\\Surface Search Distance", "How far above and below an object the move to a surface looks for the surface it lands on.", "m" },
	{ "Tools\\Box Selection\\Limited Depth", "A box selects only as deep as the nearest object under its corners.", nullptr },
	{ "Tools\\Box Selection\\Back Face Culling", "Faces turned away from the camera are not picked.", nullptr },
	{ "Tools\\Box Selection\\Depth Tolerance", "How far past that nearest object a box still selects.", "m" },
	{ "Tools\\Groups\\Select Whole Group", "Selecting a group selects every object in it too." },
	{ "Objects\\Library\\Unload Unused Objects", "A library object leaves memory when no scene object uses it any more." },
	{ "Objects\\Loading\\Load Render Buffers at Once", "Meshes build their render buffers as they load, not when they are first drawn." },
	{ "Objects\\Loading\\Load Collision Models at Once", "Meshes build their collision models as they load, not when they are first needed." },
	{ "Objects\\Display\\Show Pivot", "Draws the pivot of a selected object." },
	{ "Objects\\Display\\Show Animation Path", "Draws the path an animated object follows." },
	{ "Objects\\Display\\Use LODs", "An object that has a LOD draws it when it is small on screen." },
	{ "Objects\\Skeleton\\Show Joints", "Draws the joints of a skeleton." },
	{ "Objects\\Skeleton\\Show Bone Axes", "Draws the axes of every bone." },
	{ "Objects\\Skeleton\\Show Bone Names", "Draws the name of every bone." },
	{ "Objects\\Skeleton\\Show Bone Shapes", "Draws the collision shape of every bone." },
};

// The official names of the commands that take a shortcut, as the menus and the viewport say them, in the order the
// Keyboard page lists them. The id is the command's .ini name (COMMAND_X, or COMMAND_X."sub"), which stays what it was:
// the registry's own descriptions are engine wording and double as those names. A command missing here keeps its
// description, split at its last part.
struct SCommandName { const char* id; const char* group; const char* label; };
const SCommandName kCommandNames[] =
{
	{ "COMMAND_CLEAR", "File", "New Scene" },
	{ "COMMAND_LOAD", "File", "Open Scene" },
	{ "COMMAND_LOAD_FIRSTRECENT", "File", "Open Last Scene" },
	{ "COMMAND_SAVE.\"Save\"", "File", "Save" },
	{ "COMMAND_SAVE.\"Save As\"", "File", "Save As" },
	{ "COMMAND_FILE_MENU", "File", "File Menu" },
	{ "COMMAND_EXIT", "File", "Quit" },

	{ "COMMAND_UNDO", "Edit", "Undo" },
	{ "COMMAND_REDO", "Edit", "Redo" },
	{ "COMMAND_CUT", "Edit", "Cut" },
	{ "COMMAND_COPY", "Edit", "Copy" },
	{ "COMMAND_PASTE", "Edit", "Paste" },
	{ "COMMAND_DELETE_SELECTION", "Edit", "Delete" },
	{ "COMMAND_SHOW_OBJECTLIST", "Edit", "Find in World Outliner" },
	{ "COMMAND_COMMAND_PALETTE", "Edit", "Command Palette" },
	{ "COMMAND_EDITOR_PREF", "Edit", "Preferences" },
	{ "COMMAND_BREAK_LAST_OPERATION", "Edit", "Cancel Running Job" },

	{ "COMMAND_SELECT_ALL", "Selection", "Select All" },
	{ "COMMAND_DESELECT_ALL", "Selection", "Deselect All" },
	{ "COMMAND_INVERT_SELECTION_ALL", "Selection", "Invert Selection" },

	{ "COMMAND_HIDE_SEL", "Visibility", "Hide Selected" },
	{ "COMMAND_HIDE_UNSEL", "Visibility", "Hide Unselected" },
	{ "COMMAND_HIDE_ALL", "Visibility", "Hide All" },

	{ "COMMAND_GIZMO_MODE.\"Select\"", "Viewport Tools", "Select" },
	{ "COMMAND_CHANGE_ACTION.\"Add\"", "Viewport Tools", "Place" },
	{ "COMMAND_GIZMO_MODE.\"Move\"", "Viewport Tools", "Move" },
	{ "COMMAND_GIZMO_MODE.\"Rotate\"", "Viewport Tools", "Rotate" },
	{ "COMMAND_GIZMO_MODE.\"Scale\"", "Viewport Tools", "Scale" },
	{ "COMMAND_GIZMO_MODE.\"Cycle\"", "Viewport Tools", "Next Transform Tool" },
	{ "COMMAND_CHANGE_ACTION.\"Select\"", "Viewport Tools", "Stop Placing" },

	{ "COMMAND_ZOOM_EXTENTS.\"Extent\"", "View", "Frame All" },
	{ "COMMAND_ZOOM_EXTENTS.\"Selected\"", "View", "Frame Selection" },
	{ "COMMAND_MOVE_CAMERA_TO", "View", "Move Camera To" },
	{ "COMMAND_TOGGLE_RENDER_WIRE", "View", "Wireframe" },
	{ "COMMAND_TOGGLE_GRID", "View", "Show Grid" },
	{ "COMMAND_GRID_SLOT_SIZE.\"Decrease\"", "View", "Smaller Grid Cells" },
	{ "COMMAND_GRID_SLOT_SIZE.\"Increase\"", "View", "Larger Grid Cells" },
	{ "COMMAND_TOGGLE_SAFE_RECT", "View", "Show Safe Rect" },
	{ "COMMAND_TOGGLE_AIMAP_VISIBILITY", "View", "Show AI Map" },

	// a command names the tool as its header does, "<title> Tool" (UILeftBarForm.cpp kToolClasses); the ids stay
	{ "COMMAND_CHANGE_TARGET.\"Object\"", "Tools Panel", "Object Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Light\"", "Tools Panel", "Light Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Sound Source\"", "Tools Panel", "Sound Source Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Sound Env\"", "Tools Panel", "Sound Environment Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Glow\"", "Tools Panel", "Glow Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Shape\"", "Tools Panel", "Shape Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Spawn Point\"", "Tools Panel", "Spawn Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Way\"", "Tools Panel", "Way Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Way Point\"", "Tools Panel", "Way Tool: Points" },
	{ "COMMAND_CHANGE_TARGET.\"Toggle Way Mode\"", "Tools Panel", "Way Tool: Switch Ways / Points" },
	{ "COMMAND_CHANGE_TARGET.\"Sector\"", "Tools Panel", "Sector Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Portal\"", "Tools Panel", "Portal Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Group\"", "Tools Panel", "Group Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Particle System\"", "Tools Panel", "Particles Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Detail Objects\"", "Tools Panel", "Detail Objects Tool" },
	{ "COMMAND_CHANGE_TARGET.\"AI Map\"", "Tools Panel", "AI Map Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Static Wallmark\"", "Tools Panel", "Wallmark Tool" },
	{ "COMMAND_CHANGE_TARGET.\"Fog Volume\"", "Tools Panel", "Fog Volume Tool" },

	{ "COMMAND_DROP_TO_GROUND", "Object", "Drop to Ground" },
	{ "COMMAND_SIMULATE", "Object", "Simulate Physics" },
	{ "COMMAND_USE_SIMULATE_POSITIONS", "Object", "Apply Simulated Positions" },

	{ "COMMAND_BUILD", "Make", "Make All" },
	{ "COMMAND_MAKE_GAME", "Make", "Make Game" },
	{ "COMMAND_MAKE_DETAILS", "Make", "Make Details" },
	{ "COMMAND_MAKE_HOM", "Make", "Make HOM" },
	{ "COMMAND_MAKE_SOM", "Make", "Make SOM" },
	{ "COMMAND_MAKE_AIMAP", "Make", "Make AI Map" },

	{ "COMMAND_MOD_BUILD", "Mod", "Build Into Game" },
	{ "COMMAND_MOD_EDIT_MANIFEST", "Mod", "Mod Settings" },
	{ "COMMAND_MOD_CHARACTERS", "Mod", "Characters" },
	{ "COMMAND_MOD_WORLD", "Mod", "World Rules" },
	{ "COMMAND_MOD_PDA", "Mod", "PDA Interface" },
	{ "COMMAND_MOD_PACKAGE", "Mod", "Package Release" },
	{ "COMMAND_MOD_PUBLISH", "Mod", "Publish Release" },
	{ "COMMAND_MOD_RELEASES", "Mod", "Releases" },
	{ "COMMAND_MOD_CATALOG", "Mod", "Submit to Mod Browser" },

	{ "COMMAND_SHOW_PROPERTIES", "Window", "Details" },
	{ "COMMAND_OPTIONS", "Window", "Details (Scene Options)" },
	{ "COMMAND_SHOW_WORLD_OUTLINER", "Window", "World Outliner" },
	{ "COMMAND_SHOW_CONTENT_BROWSER", "Window", "Content Browser" },

	{ "COMMAND_RUN_MACRO.\"Slot #1\"", "Macros", "Run Macro 1" },
	{ "COMMAND_RUN_MACRO.\"Slot #2\"", "Macros", "Run Macro 2" },
	{ "COMMAND_RUN_MACRO.\"Slot #3\"", "Macros", "Run Macro 3" },
	{ "COMMAND_RUN_MACRO.\"Slot #4\"", "Macros", "Run Macro 4" },
	{ "COMMAND_RUN_MACRO.\"Slot #5\"", "Macros", "Run Macro 5" },
	{ "COMMAND_RUN_MACRO.\"Slot #6\"", "Macros", "Run Macro 6" },
	{ "COMMAND_RUN_MACRO.\"Slot #7\"", "Macros", "Run Macro 7" },
	{ "COMMAND_RUN_MACRO.\"Slot #8\"", "Macros", "Run Macro 8" },
};
constexpr u32 kUnlistedOrder = 0x10000;

// a row of the Keyboard page
struct SShortcutRow
{
	SESubCommand* sub;
	xr_string id, group, label, keys;
	u32 order;
};

// every command that takes a shortcut, in the page's order: the listed ones as listed, the rest after them by group
void ShortcutRows(xr_vector<SShortcutRow>& rows)
{
	rows.clear();
	for (SECommand* command : GetEditorCommands())
		if (command && command->editable)
			for (SESubCommand* sub : command->sub_commands)
			{
				SShortcutRow& row = rows.emplace_back();
				row.sub = sub;
				row.id = CCustomPreferences::ShortcutId(sub);
				row.order = CCustomPreferences::CommandName(sub, row.group, row.label);
				row.keys = XUI::ShortcutLabel(sub->shortcut);
			}
	std::stable_sort(rows.begin(), rows.end(), [](const SShortcutRow& a, const SShortcutRow& b)
	{
		if (a.order != b.order)
			return a.order < b.order;
		return a.order == kUnlistedOrder && a.group < b.group;
	});
}
}

CCustomPreferences::CCustomPreferences()
{
    bOpen = false;
	m_ItemProps = nullptr;
	m_Search[0] = 0;
	m_Page = "General";
	m_SavePending = false;
	m_AssignFor = nullptr;
	ui_theme_preset = u32(GrizzlyTheme::Default());
	active_idle_fps = kEditorIdleFpsDefault;
	ui_layout_version = 0;
	level_layout_custom = false;
	ui_floating_panels = false;
	// view
    view_np				= 0.1f;
    view_fp				= 1500.f;
    view_fov			= deg2rad(60.f);
	// fog
    fog_color			= 0x00555555;
    fog_fogness			= 0.9;
    // camera
    cam_fly_speed		= 5.0f;
    cam_fly_alt			= 1.8f;
    cam_sens_rot		= 0.6f;
    cam_sens_move		= 0.6f;
	// tools mouse
    tools_sens_move		= 0.3f;
    tools_sens_rot		= 0.3f;
    tools_sens_scale	= 0.3f;
	tools_show_move_axis =false;
    // box pick
    bp_lim_depth		= TRUE;
    bp_cull				= TRUE;
    bp_depth_tolerance	= 0.1f;
    // snap: the viewport toolbar's steps (the gizmo reads them)
    snap_angle			= deg2rad(5.625f);
    snap_move			= 0.1f;
    snap_scale			= 0.1f;
    snap_moveto			= 0.5f;
    // grid
    grid_cell_size		= 1.f;
    grid_cell_count		= 100;
    // scene
    scene_undo_level	= 125;
    scene_recent_count	= 10;
    scene_clear_color	= DEFAULT_CLEARCOLOR;
    // objects
    object_flags.zero	();
}
//---------------------------------------------------------------------------

CCustomPreferences::~CCustomPreferences()
{
}

void CCustomPreferences::ApplyValues()
{
	SetThemePreset(ui_theme_preset, false);

    EDevice->m_Camera.SetViewport(view_np, view_fp, rad2deg(view_fov));
    Tools->SetFog	(fog_color,fog_fogness);

    UI->m_MouseSM	= 0.2f*tools_sens_move*tools_sens_move;
    UI->m_MouseSR	= 0.02f*tools_sens_rot*tools_sens_rot;
    UI->m_MouseSS	= 0.02f*tools_sens_scale*tools_sens_scale;

    EDevice->m_Camera.SetSensitivity	(cam_sens_move, cam_sens_rot);
    EDevice->m_Camera.SetFlyParams	(cam_fly_speed, cam_fly_alt);

    ExecCommand		(COMMAND_UPDATE_GRID);
}
//---------------------------------------------------------------------------

void  CCustomPreferences::OnClose	()
{
	ApplyValues	();
}
//---------------------------------------------------------------------------

void CCustomPreferences::OnThemeChanged(PropValue*)
{
	SetThemePreset(ui_theme_preset, false);
}

void CCustomPreferences::SetThemePreset(u32 preset, bool persist)
{
	ui_theme_preset = GrizzlyTheme::IsValid(preset) ? preset : u32(GrizzlyTheme::Default());
	GrizzlyTheme::Apply(GrizzlyTheme::Preset(ui_theme_preset));
	if (persist)
		Save();
}

// a change on the tab takes effect at once; the file is written when the field being typed in lets go
void CCustomPreferences::OnModified()
{
	ApplyValues();
	m_SavePending = true;
}

//---------------------------------------------------------------------------
// Shortcuts
//---------------------------------------------------------------------------
xr_string CCustomPreferences::ShortcutId(SESubCommand* sub)
{
	if (!sub || !sub->parent)
		return {};
	xr_string id = sub->parent->Name();
	if (sub->desc.size())
		id += ".\"" + sub->desc + "\"";
	return id;
}

u32 CCustomPreferences::CommandName(SESubCommand* sub, xr_string& group, xr_string& label)
{
	group.clear();
	label.clear();
	if (!sub || !sub->parent)
		return kUnlistedOrder;
	const xr_string id = ShortcutId(sub);
	for (u32 i = 0; i < u32(std::size(kCommandNames)); ++i)
		if (id == kCommandNames[i].id)
		{
			group = kCommandNames[i].group;
			label = kCommandNames[i].label;
			return i;
		}
	// not listed: the description's last part names it, the parts before it (or the command, for a preset) its group
	xr_string path = sub->parent->Desc();
	if (sub->desc.size())
		path += (path.empty() ? "" : "\\") + xr_string(sub->desc.c_str());
	const size_t last = path.rfind('\\');
	if (last == xr_string::npos)
	{
		group = "General";
		label = path;
	}
	else
	{
		for (size_t i = 0; i < last; ++i)
		{
			if (path[i] == '\\')
				group += kPathDot;
			else
				group += path[i];
		}
		label = path.substr(last + 1);
	}
	return kUnlistedOrder;
}

xr_string CCustomPreferences::ShortcutLabel(SESubCommand* sub)
{
	xr_string group, label;
	CommandName(sub, group, label);
	return label.empty() ? label : group + kPathDot + label;
}

SESubCommand* CCustomPreferences::ShortcutOwner(const xr_shortcut& shortcut, SESubCommand* except)
{
	if (!shortcut.key)
		return nullptr;
	for (SECommand* command : GetEditorCommands())
		if (command && command->editable)
			for (SESubCommand* sub : command->sub_commands)
				if (sub != except && sub->shortcut.similar(shortcut))
					return sub;
	return nullptr;
}

void CCustomPreferences::AssignShortcut(SESubCommand* sub, const xr_shortcut& shortcut)
{
	if (!sub)
		return;
	// one shortcut, one command: the one that had it lets it go
	if (SESubCommand* owner = ShortcutOwner(shortcut, sub))
	{
		Msg("* Shortcut %s moved from %s to %s", XUI::ShortcutLabel(shortcut).c_str(), ShortcutLabel(owner).c_str(), ShortcutLabel(sub).c_str());
		owner->shortcut = xr_shortcut();
	}
	sub->shortcut = shortcut;
	Save();
}

void CCustomPreferences::ShowPage(LPCSTR page, LPCSTR search)
{
	if (page && page[0])
		m_Page = page;
	strncpy_s(m_Search, sizeof(m_Search), search ? search : "", _TRUNCATE);
	if (m_ItemProps)
		m_ItemProps->SetFilter(m_Search);
}

void CCustomPreferences::AddHelp(LPCSTR key, LPCSTR help)
{
	if (key && help)
		m_ExtraHelp.emplace_back(key, help);
}

void CCustomPreferences::AddUnit(LPCSTR key, LPCSTR unit)
{
	if (key && unit)
		m_ExtraUnits.emplace_back(key, unit);
}

void CCustomPreferences::AddPageIcon(LPCSTR page, XUI::Icon icon)
{
	if (!page || !page[0])
		return;
	for (auto& [name, known] : m_PageIcons)
		if (name == page)
		{
			known = icon;
			return;
		}
	m_PageIcons.emplace_back(page, icon);
}

XUI::Icon CCustomPreferences::PageIcon(const xr_string& page) const
{
	for (const auto& [name, icon] : m_PageIcons)
		if (name == page)
			return icon;
	for (const SPageIcon& p : kPageIcons)
		if (page == p.page)
			return p.icon;
	return XUI::Icon::Settings;
}

void CCustomPreferences::ImportShortcuts()
{
    xr_string fn;
    if (EFS.GetOpenName(EDevice->m_hWnd, "$import$", fn, false, NULL, 6))
    {
        CInifile* I = xr_new<CInifile>(fn.c_str(), TRUE, TRUE, TRUE);
        LoadShortcuts(I);
        xr_delete(I);
        Save();
    }
}

void CCustomPreferences::ExportShortcuts()
{
    xr_string fn;
    if (EFS.GetSaveName("$import$", fn, NULL, 6))
    {
        CInifile* I = xr_new<CInifile>(fn.c_str(), FALSE, TRUE, TRUE);
        SaveShortcuts(I);
        xr_delete(I);
    }
}

void CCustomPreferences::FillProp(PropItemVec& props)
{
	static xr_token theme_presets[] = {
		{ "Grizzly Purple", int(GrizzlyTheme::Preset::GrizzlyPurple) },
		{ "Graphite", int(GrizzlyTheme::Preset::Graphite) },
		{ nullptr, 0 },
	};
	Token32Value* theme = PHelper().CreateToken32(props, "General\\Theme", &ui_theme_preset, theme_presets);
	theme->OnChangeEvent.bind(this, &CCustomPreferences::OnThemeChanged);
	PHelper().CreateBOOL(props, "General\\Allow Panels Outside the Window", &ui_floating_panels);
	PHelper().CreateU32(props, "General\\Frame Rate Limit", &active_idle_fps, kEditorIdleFpsMinimum, kEditorIdleFpsMaximum);
    PHelper().CreateU32		(props,"General\\Recent Files",					&scene_recent_count,0,		25);
    PHelper().CreateU32		(props,"General\\Undo Steps",					&scene_undo_level,	0,		125);

    PHelper().CreateFloat	(props,"Viewport\\Camera\\Move Sensitivity",	&cam_sens_move);
    PHelper().CreateFloat	(props,"Viewport\\Camera\\Rotate Sensitivity",	&cam_sens_rot);
    PHelper().CreateFloat	(props,"Viewport\\Camera\\Fly Speed",			&cam_fly_speed,		0.01f,	100.f);
    PHelper().CreateFloat	(props,"Viewport\\Camera\\Fly Altitude",		&cam_fly_alt,		0.f,	1000.f);
    PHelper().CreateAngle	(props,"Viewport\\Field of View",				&view_fov,			deg2rad(0.1f), deg2rad(170.f));
    PHelper().CreateFloat	(props,"Viewport\\Near Plane",					&view_np,			0.01f,	10.f);
    PHelper().CreateFloat	(props,"Viewport\\Far Plane",					&view_fp,			10.f,	10000.f);
    PHelper().CreateFloat	(props,"Viewport\\Render Radius",				&EDevice->RadiusRender,10.f,100000.f);
    PHelper().CreateColor	(props,"Viewport\\Background Color",			&scene_clear_color);
    PHelper().CreateColor	(props,"Viewport\\Fog\\Color",					&fog_color);
    PHelper().CreateFloat	(props,"Viewport\\Fog\\Density",				&fog_fogness,		0.f,	100.f);
    PHelper().CreateFloat	(props,"Viewport\\Grid\\Cell Size",				&grid_cell_size,	0.1f,	10.f);
    PHelper().CreateU32		(props,"Viewport\\Grid\\Cell Count",			&grid_cell_count,	10,		1000);

    PHelper().CreateFloat	(props,"Tools\\Gizmo\\Move Sensitivity",		&tools_sens_move);
    PHelper().CreateFloat	(props,"Tools\\Gizmo\\Rotate Sensitivity",		&tools_sens_rot);
    PHelper().CreateFloat	(props,"Tools\\Gizmo\\Scale Sensitivity",		&tools_sens_scale);
    PHelper().CreateBOOL	(props,"Tools\\Gizmo\\Show Move Axes",			&tools_show_move_axis);
    PHelper().CreateFloat	(props,"Tools\\Snap\\Grid Step",				&snap_move,			0.01f,	1000.f);
    // three decimals: the toolbar's 5.625 reads the same here
    PHelper().CreateAngle	(props,"Tools\\Snap\\Angle Step",				&snap_angle,		deg2rad(0.1f),	deg2rad(180.f),	0.01f,	3);
    PHelper().CreateFloat	(props,"Tools\\Snap\\Scale Step",				&snap_scale,		0.01f,	1000.f);
    PHelper().CreateFloat	(props,"Tools\\Snap\\Surface Search Distance",	&snap_moveto,		0.01f,	1000.f);
    PHelper().CreateBOOL	(props,"Tools\\Box Selection\\Limited Depth",		&bp_lim_depth);
    PHelper().CreateBOOL	(props,"Tools\\Box Selection\\Back Face Culling",	&bp_cull);
    PHelper().CreateFloat	(props,"Tools\\Box Selection\\Depth Tolerance",		&bp_depth_tolerance,0.f,	10000.f);
    PHelper().CreateFlag32	(props,"Tools\\Groups\\Select Whole Group",		&object_flags,		epoSelectInGroup);

    PHelper().CreateFlag32	(props,"Objects\\Library\\Unload Unused Objects",			&object_flags, epoDiscardInstance);
    PHelper().CreateFlag32	(props,"Objects\\Loading\\Load Render Buffers at Once",		&object_flags, epoEagerLoadRenderBuffers);
    PHelper().CreateFlag32	(props,"Objects\\Loading\\Load Collision Models at Once",	&object_flags, epoEagerLoadCollisionModels);
    PHelper().CreateFlag32	(props,"Objects\\Display\\Show Pivot",					&object_flags, epoDrawPivot);
    PHelper().CreateFlag32	(props,"Objects\\Display\\Show Animation Path",			&object_flags, epoDrawAnimPath);
    PHelper().CreateFlag32	(props,"Objects\\Display\\Use LODs",					&object_flags, epoDrawLOD);
    PHelper().CreateFlag32	(props,"Objects\\Skeleton\\Show Joints",				&object_flags, epoDrawJoints);
    PHelper().CreateFlag32	(props,"Objects\\Skeleton\\Show Bone Axes",				&object_flags, epoDrawBoneAxis);
    PHelper().CreateFlag32	(props,"Objects\\Skeleton\\Show Bone Names",			&object_flags, epoDrawBoneNames);
    PHelper().CreateFlag32	(props,"Objects\\Skeleton\\Show Bone Shapes",			&object_flags, epoDrawBoneShapes);
}

// Preferences is one of the editor's major tabs (XUI::Shell): a change takes effect at once and is saved when the
// field being typed in lets go, when the tab is left and when it closes
void CCustomPreferences::Edit()
{
    if (bOpen)
    {
        XUI::Shell::Activate("preferences");
        return;
    }
    bOpen = true;
	PropItemVec props;
	m_ExtraHelp.clear();
	m_ExtraUnits.clear();
    FillProp						(props);
	m_ItemProps->AssignItems		(props);
	for (const SPrefHelp& h : kPrefHelp)
	{
		m_ItemProps->SetHelp(h.key, h.help);
		m_ItemProps->SetUnit(h.key, h.unit);
	}
	for (const auto& [key, help] : m_ExtraHelp)
		m_ItemProps->SetHelp(key.c_str(), help.c_str());
	for (const auto& [key, unit] : m_ExtraUnits)
		m_ItemProps->SetUnit(key.c_str(), unit.c_str());

	XUI::Shell::DocDesc d;
	d.id = "preferences";
	d.kind = "preferences";
	d.icon = XUI::Icon::Settings;
	d.label = [] { return xr_string("Preferences"); };
	d.tooltip = [] { return xr_string("The editor's settings. A change takes effect at once and is saved."); };
	d.alive = [this] { return bOpen; };
	d.body = [this] { Draw(); };
	d.on_deactivate = [this]
	{
		if (m_SavePending)
		{
			m_SavePending = false;
			Save();
		}
	};
	d.on_close = [this]
	{
		OnClose();
		Save();
		m_SavePending = false;
		bOpen = false;
	};
	XUI::Shell::Open(d, true);
}
//---------------------------------------------------------------------------
extern bool bAllowLogCommands;
void CCustomPreferences::Load(CInifile* I)
{
	ui_theme_preset		= R_U32_SAFE	("editor_prefs", "ui_theme_preset", u32(GrizzlyTheme::Default()));
	if (!GrizzlyTheme::IsValid(ui_theme_preset))
		ui_theme_preset = u32(GrizzlyTheme::Default());
	active_idle_fps = R_U32_SAFE("editor_prefs", "active_idle_fps", kEditorIdleFpsDefault);
	active_idle_fps = clampr(active_idle_fps, kEditorIdleFpsMinimum, kEditorIdleFpsMaximum);

    psDeviceFlags.flags		= R_U32_SAFE	("editor_prefs","device_flags",	psDeviceFlags.flags);
    psSoundFlags.flags		= R_U32_SAFE	("editor_prefs","sound_flags",	psSoundFlags.flags)

    Tools->m_Settings.flags	= R_U32_SAFE	("editor_prefs","tools_settings",Tools->m_Settings.flags);

    view_np				= R_FLOAT_SAFE	("editor_prefs","view_np"			,view_np		 	);
    view_fp				= R_FLOAT_SAFE	("editor_prefs","view_fp"			,view_fp		 	);
    view_fov			= R_FLOAT_SAFE	("editor_prefs","view_fov"			,view_fov			);

    fog_color			= R_U32_SAFE	("editor_prefs","fog_color"			,fog_color			);
    fog_fogness			= R_FLOAT_SAFE	("editor_prefs","fog_fogness"		,fog_fogness	 	);

    cam_fly_speed		= R_FLOAT_SAFE	("editor_prefs","cam_fly_speed"		,cam_fly_speed		);
    cam_fly_alt			= R_FLOAT_SAFE	("editor_prefs","cam_fly_alt"		,cam_fly_alt	 	);
    cam_sens_rot		= R_FLOAT_SAFE	("editor_prefs","cam_sens_rot"		,cam_sens_rot		);
    cam_sens_move		= R_FLOAT_SAFE	("editor_prefs","cam_sens_move"		,cam_sens_move		);

    tools_sens_move		= R_FLOAT_SAFE	("editor_prefs","tools_sens_move"	,tools_sens_move  	);
    tools_sens_rot		= R_FLOAT_SAFE	("editor_prefs","tools_sens_rot"	,tools_sens_rot		);
    tools_sens_scale	= R_FLOAT_SAFE	("editor_prefs","tools_sens_scale"	,tools_sens_scale	);
	tools_show_move_axis= R_BOOL_SAFE	("editor_prefs","tools_show_move_axis"	,tools_show_move_axis);

    // each box-pick setting under its own name (they used to share bp_lim_depth and tools_sens_rot)
    bp_lim_depth		= R_BOOL_SAFE	("editor_prefs","bp_lim_depth"		,bp_lim_depth		);
    bp_cull				= R_BOOL_SAFE	("editor_prefs","bp_cull"			,bp_cull		  	);
    bp_depth_tolerance	= R_FLOAT_SAFE	("editor_prefs","bp_depth_tolerance",bp_depth_tolerance	);

    // the snap steps under their own names, the angle in degrees as the toolbar shows it; a step is never 0
    const float snap_deg = R_FLOAT_SAFE	("editor_prefs","snap_angle_step"	,rad2deg(snap_angle));
    snap_angle			= deg2rad(clampr(snap_deg, 0.1f, 180.f));
    snap_move			= R_FLOAT_SAFE	("editor_prefs","snap_grid_step"	,snap_move			);
    snap_move			= clampr(snap_move, 0.01f, 1000.f);
    snap_scale			= R_FLOAT_SAFE	("editor_prefs","snap_scale_step"	,snap_scale			);
    snap_scale			= clampr(snap_scale, 0.01f, 1000.f);
    snap_moveto			= R_FLOAT_SAFE	("editor_prefs","snap_moveto"		,snap_moveto	   	);

    grid_cell_size		= R_FLOAT_SAFE	("editor_prefs","grid_cell_size"	,grid_cell_size		);
    grid_cell_count		= R_U32_SAFE	("editor_prefs","grid_cell_count"	,grid_cell_count   	);

    scene_undo_level	= R_U32_SAFE	("editor_prefs","scene_undo_level"	,scene_undo_level	);
    scene_recent_count	= R_U32_SAFE	("editor_prefs","scene_recent_count",scene_recent_count	);
    scene_clear_color	= R_U32_SAFE	("editor_prefs","scene_clear_color"	,scene_clear_color	);

    object_flags.flags	= R_U32_SAFE	("editor_prefs","object_flags"		,object_flags.flags );
    EDevice->RadiusRender = R_FLOAT_SAFE("render", "render_radius", EDevice->RadiusRender);


    start_w = R_U32_SAFE("render", "w", 1280);
    start_h = R_U32_SAFE("render", "h",800);
    start_maximized = R_BOOL_SAFE("render", "maximized", false);

	// the frame's own settings, read before the UI exists: a layout of another version is not loaded,
	// panels outside the editor window only when asked for (both take a restart)
	ui_layout_version = R_U32_SAFE("ui", "layout_version", 0);
	level_layout_custom = R_BOOL_SAFE("ui", "level_layout_custom", false);
	ui_floating_panels = R_BOOL_SAFE("ui", "floating_panels", false);
	XUI::Shell::SetSkipLayoutOnce(ui_layout_version != kUiLayoutVersion);
	XUI::Shell::SetFloatingPanels(ui_floating_panels);

    bAllowLogCommands = R_BOOL_SAFE("windows", "log", false);
	// read recent list
    for (u32 i=0; i<scene_recent_count; i++){
    	shared_str fn  	= R_STRING_SAFE	("editor_prefs",xr_string().sprintf("recent_files_%d",i).c_str(),shared_str("") );
        if (fn.size())
        {
        	AStringIt it =   std::find(scene_recent_list.begin(), scene_recent_list.end(), fn.c_str() ) ;
            if (it==scene_recent_list.end())
	        	scene_recent_list.push_back(*fn);
        }
    }
    sWeather = R_STRING_SAFE	("editor_prefs", "weather", shared_str("") );
    // load shortcuts

    LoadShortcuts		(I);

    UI->LoadSettings	(I);
}

void CCustomPreferences::Save(CInifile* I)
{
	if (ui_theme_preset == u32(GrizzlyTheme::Default()))
	{
		if (I->line_exist("editor_prefs", "ui_theme_preset"))
			I->remove_line("editor_prefs", "ui_theme_preset");
	}
	else
	{
		I->w_u32("editor_prefs", "ui_theme_preset", ui_theme_preset);
	}
	if (active_idle_fps == kEditorIdleFpsDefault)
	{
		if (I->line_exist("editor_prefs", "active_idle_fps"))
			I->remove_line("editor_prefs", "active_idle_fps");
	}
	else
	{
		I->w_u32("editor_prefs", "active_idle_fps", active_idle_fps);
	}

    I->w_u32("editor_prefs", "device_flags", psDeviceFlags.flags);
    I->w_u32("editor_prefs", "sound_flags", psSoundFlags.flags);

    I->w_u32("editor_prefs", "tools_settings", Tools->m_Settings.flags);

    I->w_float("editor_prefs", "view_np", view_np);
    I->w_float("editor_prefs", "view_fp", view_fp);
    I->w_float("editor_prefs", "view_fov", view_fov);

    I->w_u32("editor_prefs", "fog_color", fog_color);
    I->w_float("editor_prefs", "fog_fogness", fog_fogness);

    I->w_float("editor_prefs", "cam_fly_speed", cam_fly_speed);
    I->w_float("editor_prefs", "cam_fly_alt", cam_fly_alt);
    I->w_float("editor_prefs", "cam_sens_rot", cam_sens_rot);
    I->w_float("editor_prefs", "cam_sens_move", cam_sens_move);

    I->w_float("editor_prefs", "tools_sens_rot", tools_sens_rot);
    I->w_float("editor_prefs", "tools_sens_move", tools_sens_move);
    I->w_float("editor_prefs", "tools_sens_scale", tools_sens_scale);
    I->w_bool("editor_prefs", "tools_show_move_axis", tools_show_move_axis);

    I->w_bool("editor_prefs", "bp_lim_depth", bp_lim_depth);
    I->w_bool("editor_prefs", "bp_cull", bp_cull);
    I->w_float("editor_prefs", "bp_depth_tolerance", bp_depth_tolerance);

    I->w_float("editor_prefs", "snap_angle_step", rad2deg(snap_angle));
    I->w_float("editor_prefs", "snap_grid_step", snap_move);
    I->w_float("editor_prefs", "snap_scale_step", snap_scale);
    I->w_float("editor_prefs", "snap_moveto", snap_moveto);

    I->w_float("editor_prefs", "grid_cell_size", grid_cell_size);
    I->w_u32("editor_prefs", "grid_cell_count", grid_cell_count);

    I->w_u32("editor_prefs", "scene_undo_level", scene_undo_level);
    I->w_u32("editor_prefs", "scene_recent_count", scene_recent_count);
    I->w_u32("editor_prefs", "scene_clear_color", scene_clear_color);

    I->w_u32("editor_prefs", "object_flags", object_flags.flags);
    for (AStringIt it = scene_recent_list.begin(); it != scene_recent_list.end(); it++) {
        xr_string L; L.sprintf("recent_files_%d", it - scene_recent_list.begin());
        xr_string V; V.sprintf("\"%s\"", it->c_str());
        I->w_string("editor_prefs", L.c_str(), V.c_str());
    }
    I->w_string("editor_prefs", "weather", sWeather.c_str());
    I->w_bool("render", "maximized", EDevice->dwMaximized);
    I->w_u32("render", "w", EDevice->dwRealWidth);
    I->w_u32("render", "h", EDevice->dwRealHeight);
    I->w_bool("windows", "log", bAllowLogCommands);
    I->w_float("render", "render_radius", EDevice->RadiusRender);
    I->w_u32("ui", "layout_version", kUiLayoutVersion);
    I->w_bool("ui", "level_layout_custom", level_layout_custom);
    I->w_bool("ui", "floating_panels", ui_floating_panels);

    // load shortcuts
    SaveShortcuts(I);
    UI->SaveSettings(I);
}

//---------------------------------------------------------------------------
// The tab: a search and the pages on the left, the page on the right with the help of the row under the mouse
//---------------------------------------------------------------------------
namespace
{
// a search finds a command by its name, its group, its keys and its id
bool ShortcutMatches(const char* search, const SShortcutRow& row)
{
	const xr_string text = row.group + " " + row.label + " " + row.keys + " " + row.id;
	return XUI::Match(search, text.c_str());
}
}

u32 CCustomPreferences::KeyboardMatches() const
{
	xr_vector<SShortcutRow> rows;
	ShortcutRows(rows);
	u32 count = 0;
	for (const SShortcutRow& row : rows)
		count += ShortcutMatches(m_Search, row);
	return count;
}

void CCustomPreferences::DrawNav(float width)
{
	ImGui::SetNextItemWidth(width);
	if (XUI::SearchField("##prefs_search", m_Search, sizeof(m_Search), "Search settings"))
		m_ItemProps->SetFilter(m_Search);
	const bool searching = m_Search[0] != 0;
	xr_vector<std::pair<xr_string, u32> > pages;
	m_ItemProps->Folders(pages, searching);
	pages.emplace_back(kKeyboardPage, searching ? KeyboardMatches() : 0);
	if (XUI::BeginList("##prefs_nav", ImVec2(width / XUI::Dpi(), 0.f)))
	{
		for (const auto& [name, count] : pages)
		{
			// while searching, a page without a match stays in its place, dimmed, and one with matches counts them
			string16 text = {};
			XUI::NavRowSpec row;
			row.icon = PageIcon(name);
			row.dim = searching && !count;
			if (searching && count)
			{
				xr_sprintf(text, "%u", count);
				row.count = text;
			}
			row.tip = searching ? "Clears the search and opens the page" : nullptr;
			if (XUI::NavRow(name.c_str(), name.c_str(), !searching && m_Page == name, row))
			{
				m_Page = name;
				if (searching)
				{
					m_Search[0] = 0;
					m_ItemProps->SetFilter("");
				}
			}
		}
	}
	XUI::EndList();
}

void CCustomPreferences::DrawKeyboard(bool searching)
{
	if (!searching)
	{
		if (XUI::Button("Import..."))
			ImportShortcuts();
		XUI::Tip("Load every shortcut from a file");
		ImGui::SameLine();
		if (XUI::Button("Export..."))
			ExportShortcuts();
		XUI::Tip("Save every shortcut to a file");
	}
	xr_vector<SShortcutRow> rows;
	ShortcutRows(rows);
	if (searching)
		rows.erase(std::remove_if(rows.begin(), rows.end(), [this](const SShortcutRow& row) { return !ShortcutMatches(m_Search, row); }),
			rows.end());

	const ImGuiStyle& style = ImGui::GetStyle();
	const float keys_h = XUI::FontSize(XUI::Font::Small) * XUI::Dpi() + XUI::Pt(4.f);
	// the Shortcut column is as wide as the widest keycaps
	float keys_w = XUI::Em(6.f);
	for (const SShortcutRow& row : rows)
		keys_w = _max(keys_w, XUI::Keycaps(nullptr, ImVec2(0.f, 0.f), row.keys.c_str(), keys_h) + XUI::Pt(4.f));
	const float assign_w = ImGui::CalcTextSize("Assign...").x + XUI::Pt(20.f);
	const float clear_w = ImGui::CalcTextSize("Clear").x + XUI::Pt(20.f);
	const float actions_w = assign_w + clear_w + style.ItemSpacing.x;
	const ImGuiTableFlags flags = ImGuiTableFlags_RowBg | ImGuiTableFlags_BordersInnerH | ImGuiTableFlags_PadOuterX;
	if (!ImGui::BeginTable("##shortcuts", 3, flags))
		return;
	ImGui::TableSetupColumn("Command", ImGuiTableColumnFlags_WidthStretch);
	ImGui::TableSetupColumn("Shortcut", ImGuiTableColumnFlags_WidthFixed, keys_w);
	ImGui::TableSetupColumn("##actions", ImGuiTableColumnFlags_WidthFixed, actions_w);
	XUI::TableHeaders();
	const xr_string* group = nullptr;
	for (const SShortcutRow& row : rows)
	{
		// the group's name over its commands, as the menus hold them
		if (!group || *group != row.group)
		{
			group = &row.group;
			ImGui::TableNextRow(ImGuiTableRowFlags_None, XUI::Pt(28.f));
			ImGui::TableNextColumn();
			ImGui::AlignTextToFramePadding();
			XUI::FontScope strong(XUI::Font::Strong);
			XUI::TextEllipsis(group->c_str());
		}
		SESubCommand* sub = row.sub;
		ImGui::PushID(sub);
		ImGui::TableNextRow(ImGuiTableRowFlags_None, XUI::Pt(26.f));
		ImGui::TableNextColumn();
		ImGui::AlignTextToFramePadding();
		ImGui::SetCursorPosX(ImGui::GetCursorPosX() + XUI::Pt(12.f));
		// the id MCP and level.ini know the command by is its tooltip
		if (!XUI::TextEllipsis(row.label.c_str()))
			XUI::Tip(row.id.c_str());
		ImGui::TableNextColumn();
		if (row.keys.empty())
		{
			ImGui::AlignTextToFramePadding();
			XUI_Local::MutedValue("None");
		}
		else
		{
			const ImVec2 at = ImGui::GetCursorScreenPos();
			const float frame = ImGui::GetFrameHeight();
			const float w = XUI::Keycaps(ImGui::GetWindowDrawList(), ImVec2(at.x, XUI_Local::Round(at.y + (frame - keys_h) * .5f)),
				row.keys.c_str(), keys_h);
			ImGui::Dummy(ImVec2(w, frame));
		}
		ImGui::TableNextColumn();
		if (XUI::Button("Assign..."))
		{
			m_AssignFor = sub;
			UIKeyPressForm::Show(ShortcutLabel(sub).c_str(), [](const xr_shortcut& shortcut) -> xr_string
			{
				SESubCommand* owner = ShortcutOwner(shortcut, EPrefs ? EPrefs->m_AssignFor : nullptr);
				return owner ? ShortcutLabel(owner) : xr_string();
			});
		}
		XUI::Tip("Press the new shortcut in the dialog that opens");
		ImGui::SameLine();
		if (XUI::Button("Clear", XUI::Kind::Secondary, sub->shortcut.key != 0, "This command has no shortcut"))
		{
			sub->shortcut = xr_shortcut();
			Save();
		}
		ImGui::PopID();
	}
	ImGui::EndTable();
}

void CCustomPreferences::DrawPage()
{
	const bool searching = m_Search[0] != 0;
	const bool keyboard = !searching && m_Page == kKeyboardPage;
	// a page reads as a form, 44 em at most like Mod Settings: a value is never a screen away from its label
	const float width = _max(1.f, _min(ImGui::GetContentRegionAvail().x, XUI::Em(44.f)));
	{
		XUI::FontScope title(XUI::Font::Title);
		XUI::TextEllipsis(searching ? "Search Results" : m_Page.c_str(), width);
	}
	ImGui::Spacing();
	// the rows scroll; the help of the row under the mouse stays at the bottom
	const float footer = keyboard ? 0.f : UIPropertiesForm::HelpFooterHeight() + ImGui::GetStyle().ItemSpacing.y;
	const float height = _max(XUI::Pt(80.f), ImGui::GetContentRegionAvail().y - footer);
	ImGui::PushStyleColor(ImGuiCol_ChildBg, ImVec4(0.f, 0.f, 0.f, 0.f));
	if (ImGui::BeginChild("##prefs_rows", ImVec2(width, height), ImGuiChildFlags_None))
	{
		if (keyboard)
			DrawKeyboard(false);
		else if (searching)
		{
			const u32 keys = KeyboardMatches();
			m_ItemProps->SetRootFolder(nullptr);
			if (m_ItemProps->VisiblePropertyCount() || !keys)
				m_ItemProps->DrawGrid();
			if (keys)
			{
				ImGui::Spacing();
				if (XUI::Section(kKeyboardPage))
				{
					DrawKeyboard(true);
					XUI::EndSection();
				}
			}
		}
		else
		{
			m_ItemProps->SetRootFolder(m_Page.c_str());
			m_ItemProps->DrawGrid();
		}
	}
	ImGui::EndChild();
	ImGui::PopStyleColor();
	if (!keyboard)
		m_ItemProps->DrawHelpFooter(width);
}

// the tab's work area
void CCustomPreferences::Draw()
{
    if (!bOpen)return;
	// the shortcut the Assign dialog took
	if (m_AssignFor)
	{
		bool ok = false;
		xr_shortcut shortcut;
		if (UIKeyPressForm::GetResult(ok, shortcut))
		{
			if (ok)
				AssignShortcut(m_AssignFor, shortcut);
			m_AssignFor = nullptr;
		}
	}

	const float nav = XUI::Pt(200.f);
	const float gap = XUI::Pt(16.f);
	ImGui::BeginGroup();
	DrawNav(nav);
	ImGui::EndGroup();
	ImGui::SameLine(0.f, gap);
	ImGui::BeginGroup();
	DrawPage();
	ImGui::EndGroup();

	// written once the field being typed in or dragged lets go, not on every step
	if (m_SavePending && !ImGui::IsAnyItemActive())
	{
		m_SavePending = false;
		Save();
	}
}

void CCustomPreferences::Load()
{
	string_path			fn;
	INI_NAME			(fn);
    CInifile* I			= xr_new<CInifile>(fn, TRUE, TRUE, TRUE);
    Load				(I);
    xr_delete			(I);
    ApplyValues			();
}
void CCustomPreferences::Save()
{
	string_path			fn;
	INI_NAME			(fn);
    CInifile* I 		= xr_new<CInifile>(fn, FALSE, TRUE, TRUE);
    I->set_override_names(TRUE);
	Save				(I);
	xr_delete			(I);
}

void CCustomPreferences::AppendRecentFile(LPCSTR name)
{
    for (AStringIt it=scene_recent_list.begin(); it!=scene_recent_list.end(); it++){
    	if (*it==name){
        	scene_recent_list.erase	(it);
            break;
        }
    }
	scene_recent_list.insert(scene_recent_list.begin(),name);
	while (scene_recent_list.size()>=EPrefs->scene_recent_count)
    	scene_recent_list.pop_back();

    ExecCommand				(COMMAND_REFRESH_UI_BAR);
}
//---------------------------------------------------------------------------

void CCustomPreferences::OnCreate()
{
	Load				();
    m_ItemProps = xr_new<UIPropertiesForm>();
	m_ItemProps->SetRole("preferences");
	m_ItemProps->SetChrome(UIPropertiesForm::chromeInline);
	// a number, a list or a colour is as wide as it needs to be, next to its label, not the rest of the page
	m_ItemProps->SetValueMaxWidth(16.f);
	m_ItemProps->SetModifiedEvent(TOnModifiedEvent(this, &CCustomPreferences::OnModified));
}
//---------------------------------------------------------------------------

void CCustomPreferences::OnDestroy()
{

    xr_delete(m_ItemProps);
    Save				();
}
//---------------------------------------------------------------------------
