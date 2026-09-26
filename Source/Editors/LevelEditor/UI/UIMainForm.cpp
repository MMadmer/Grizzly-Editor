#include "stdafx.h"
#include "UITitleBar.h"
#include "..\XrECore\Editor\EditorChooseEvents.h"
#include "..\..\XrECore\Editor\EditorGameConfigs.h"
#include "..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\XrECore\Editor\EditorModPublish.h"
#include "..\..\XrECore\Editor\EditorCatalogSubmit.h"
#include "..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\XrECore\Editor\EditorWorld.h"
#include "..\..\XrECore\Editor\EditorPda.h"
#include "..\..\XrECore\Editor\EditorFileOps.h"
#include "..\..\XrECore\Editor\UILogForm.h"
#include "..\Editor\EditorModScene.h"
#include "..\Editor\Recovery\EditorRecovery.h"
#include "..\Editor\Utils\XrSEFactoryManager.h"
#include "..\Editor\Tools\Spawn\ESceneSpawnTools.h"
#include "QuestGraph\UIQuestGraph.h"
#include "..\..\XrECore\Editor\Nq\NqUtil.h"

UIMainForm* MainForm = nullptr;

namespace
{
// the project the status bar counts the log for; the Start page forgets it, so a project opened again counts afresh
xr_string s_CountedProject;

// the quit after the exit prompt waits behind a save it asked for (deferred like every save from the UI)
void QuitNow()
{
	// a Save As the author cancelled keeps the editor open
	if (Scene && Scene->IsUnsaved())
	{
		XUI::Notice(XUI::Tone::Warning, "Not closed", "The scene was not saved, so the editor stays open.");
		return;
	}
	ExecCommand(COMMAND_QUIT);
}

xr_string SceneName()
{
	if (!Tools || Tools->m_LastFileName.empty())
		return "Untitled";
	xr_string name = Tools->m_LastFileName.c_str();
	const size_t slash = name.find_last_of("\\/");
	if (slash != xr_string::npos)
		name = name.substr(slash + 1);
	const size_t dot = name.rfind('.');
	if (dot != xr_string::npos && dot)
		name.resize(dot);
	return name;
}

// Wide at 1707 x 912 pt: Tools 260 left, a 340 right column (the Outliner over Details and Level Settings), the
// browser and the log in a 240 strip under the viewport only. Compact: 240 and 300, the strip lives in drawers.
void BuildLevelLayout(unsigned root, int size_class)
{
	using namespace XUI::Shell;
	const bool wide = size_class == UIMainForm::ClassWide;
	unsigned centre = root;
	const unsigned left = Dock::Split(centre, Dock::Left, wide ? 260.f : 240.f);
	unsigned right = Dock::Split(centre, Dock::Right, wide ? 340.f : 300.f);
	const unsigned right_bottom = Dock::Split(right, Dock::Down, 0.f, .60f);
	Dock::Place("Tools###LeftBar", left);
	Dock::Place("World Outliner", right);
	Dock::Place("Details###Properties", right_bottom);
	Dock::Place("Level Settings###World Properties", right_bottom);
	if (wide)
	{
		const unsigned bottom = Dock::Split(centre, Dock::Down, 240.f);
		Dock::Place(UIContentBrowser::PanelName(), bottom);
		Dock::Place(UILogForm::PanelName(), bottom);
	}
	Dock::Place("Viewport###Render", centre);
}

// Swap pSettings for the linked game's configs, then rebuild everything that
// cached the old ones: the server-entity factory (its statics parse actor
// profiles out of pSettings) and the Spawn tool's class roster. Runs once
// per project/link context. Context switches are rejected while scene objects
// are live, so replacing the factory cannot invalidate their entity classes.
void SyncGameConfigs()
{
    static xr_string s_active;
    static xr_string s_active_context;
    if (!EditorProject::Active() || !EditorProject::GameLinked())
        return;

    xr_string desired_context = EditorProject::Root();
    desired_context += "|";
    desired_context += EditorProject::GameRoot();
    if (s_active_context == desired_context && s_active.size() &&
        s_active == EditorGameConfigs::ActiveFingerprint())
        return;

    xr_string err;
    if (!EditorGameConfigs::Activate(err))
    {
        static bool s_said = false;
        if (!s_said)
        {
            Msg("! game configs: %s - the editor keeps its bundled ones", err.c_str());
            s_said = true;
        }
        return;
    }

    // the factory's initialize() re-reads $game_config$, which now points at
    // the extracted tree; its destroy() also frees the pSettings Activate
    // just built, and initialize() creates the final one
    xr_delete(g_SEFactoryManager);
    g_SEFactoryManager = xr_new<XrSEFactoryManager>();

    if (ESceneSpawnTool* tool = Scene ? dynamic_cast<ESceneSpawnTool*>(Scene->GetTool(OBJCLASS_SPAWNPOINT)) : 0)
        tool->RefreshClasses();

    s_active = EditorGameConfigs::ActiveFingerprint();
    s_active_context = desired_context;
}

// the session's tabs come back through these: the project-relative path the tab remembered
void RegisterFactories()
{
	UIQuestGraph::RegisterFactories();
	XUI::Shell::RegisterFactory("pda", [](LPCSTR rel)
	{
		string_path abs;
		xr_string err;
		if (!rel || !*rel || !EditorFileOps::ResolveInProject(rel, abs, err))
			return false;
		EditorPda::Open(abs);
		return EditorPda::IsOpen();
	});
	XUI::Shell::RegisterFactory("characters", [](LPCSTR) { EditorCharacters::Show(); return true; });
	XUI::Shell::RegisterFactory("world_rules", [](LPCSTR) { EditorWorld::Show(); return true; });
	XUI::Shell::RegisterFactory("mod_settings", [](LPCSTR) { EditorMod::RequestEditManifest(); return true; });
	EditorPublish::RegisterDocuments();
}
}

UIMainForm::UIMainForm()
{
	// the ImGui bar is the caption from here on (see EditorTitleBar.h)
	UITitleBar::Install();

    EnableReceiveCommands();
    if (!ExecCommand(COMMAND_INITIALIZE, (u32)0, (u32)0))
    {
        FlushLog();
        exit(-1);
    }
    ExecCommand(COMMAND_UPDATE_GRID);
    ExecCommand(COMMAND_RENDER_FOCUS);
    FillChooseEvents();
    m_TopBar = xr_new<UITopBarForm>();
    m_Render = xr_new<UIRenderForm>();
    m_MainMenu = xr_new<UIMainMenuForm>();
    m_LeftBar = xr_new<UILeftBarForm>();
    m_Properties = xr_new<UILPropertiesFrom>();
    m_WorldProperties = xr_new<UIWorldPropertiesFrom>();
    m_Render->SetContextMenuEvent(TOnRenderContextMenu(this, &UIMainForm::DrawContextMenu));
    m_Render->SetToolBarEvent(TOnRenderToolBar(this, &UIMainForm::DrawRenderToolBar));
    m_Render->SetDropAssetEvent(TOnRenderDropAsset(this, &UIMainForm::DropAsset));
    // every build path mirrors the open scene into its spawn layer first -
    // "save scene, press Build" must ship what the viewport shows
    EditorMod::SetPreBuildBake(&EditorModScene::BakeCurrentSceneSpawn);
    if (dynamic_cast<CLevelPreferences*>(EPrefs)->OpenContentBrowser)
    {
        UIContentBrowser::Show();
    }
    if (dynamic_cast<CLevelPreferences*>(EPrefs)->OpenWorldOutliner)
    {
        UIWorldOutliner::Show();
    }
    if (!dynamic_cast<CLevelPreferences*>(EPrefs)->OpenProperties)
    {
        m_Properties->Close();
    }
	if (!dynamic_cast<CLevelPreferences*>(EPrefs)->OpenWorldProperties)
	{
        m_WorldProperties->Close();
	}

    // the frame: what the editor plugs into the shell's caption, toolbar, status bar and drawers
    {
        XUI::Shell::FrameHooks& hooks = XUI::Shell::Hooks();
        hooks.caption = [this](bool busy) { m_MainMenu->Draw(EditorProject::Active() && EditorProject::GameLinked(), busy); };
        hooks.project_toolbar = [this](XUI::Toolbar& tb) { m_TopBar->DrawProject(tb); };
        hooks.status_right = [this](bool busy) { DrawStatusRight(busy); };
        hooks.log_state = [](xr_string& last, int& errors, int& warnings) { UILogForm::State(last, errors, warnings); };
        hooks.busy_card = [] { UIProgressCenter::DrawCard(*UI); };
        hooks.palette = [] { ExecCommand(COMMAND_COMMAND_PALETTE); };
        hooks.quit = [] { UI->DeferUIWork(&QuitNow); };
        hooks.level_maximize = [this] { ToggleViewportMaximized(); };
        hooks.session_file = []
        {
            return EditorProject::Active() && EditorProject::GameLinked()
                ? xr_string(EditorProject::Root()) + "\\temp\\session.ltx" : xr_string();
        };
        hooks.reveal_in_browser = [](LPCSTR path)
        {
            xr_string err;
            if (UIContentBrowser::RevealAsset(NqUtil::ProjectRelative(path).c_str(), 0, false, err))
                UIContentBrowser::MakeVisible();
            else
                XUI::Toast(XUI::Tone::Warning, err.c_str());
        };
        hooks.tabs_locked = [] { return UI && UI->IsPlayInEditor(); };

        XUI::Shell::DrawerDesc content;
        content.label = "Content Browser";
        content.icon = XUI::Icon::Drawer;
        content.keys = "Ctrl+Space";
        content.body = [] { UIContentBrowser::DrawDrawer(); };
        content.panel_visible = [] { return UIContentBrowser::PanelLive(); };
        content.focus_panel = [] { UIContentBrowser::MakeVisible(); };
        XUI::Shell::SetDrawer(XUI::Shell::DrawerContent, content);

        XUI::Shell::DrawerDesc log;
        log.label = "Output Log";
        log.icon = XUI::Icon::Log;
        log.keys = "Ctrl+`";
        log.body = [] { UILogForm::DrawBody(); };
        log.panel_visible = [] { return XUI::Shell::IsActive("level") && UILogForm::PanelOpen() && XUI::Shell::WindowShown(UILogForm::PanelName()); };
        log.focus_panel = [] { XUI::Shell::FocusWindow(UILogForm::PanelName()); };
        XUI::Shell::SetDrawer(XUI::Shell::DrawerLog, log);

        // they answer their own keys: the scene's bindings stay away while one has focus
        XUI::Shell::RegisterKeyOwner(UIContentBrowser::PanelName());
        XUI::Shell::RegisterKeyOwner(UILogForm::PanelName());
        RegisterFactories();
    }
    RegisterLevel();

    extern bool XFinedInspector(LPCSTR, LPCSTR, xr_string&);
    extern void LevelMcpRegisterSurfaces();
    LevelMcpRegisterSurfaces();
    XFinedMCP::SetHandler(XFinedInspector);
    XFinedMCP::Start();

    LTools->GetGimzo()->SwitchStep(Gizmo::EType::Move, true);
    LTools->GetGimzo()->SwitchStep(Gizmo::EType::Scale, true);
    LTools->GetGimzo()->SwitchStep(Gizmo::EType::Rotate, true);

}

UIMainForm::~UIMainForm()
{
	dynamic_cast<CLevelPreferences*>(EPrefs)->OpenProperties = !m_Properties->IsClosed();
	dynamic_cast<CLevelPreferences*>(EPrefs)->OpenWorldProperties = !m_WorldProperties->IsClosed();
    dynamic_cast<CLevelPreferences*>(EPrefs)->OpenContentBrowser = UIContentBrowser::IsOpen();
    dynamic_cast<CLevelPreferences*>(EPrefs)->OpenWorldOutliner = UIWorldOutliner::IsOpen();
    // the project's open tabs, for the next start
    XUI::Shell::FlushSession();
    XFinedMCP::Stop();
    // an upload in flight is cut here; its draft stays on GitHub for the next attempt
    EditorPublish::Shutdown();
    // a Mod Browser job is cut the same way; the open author key is wiped
    EditorCatalogSubmit::Shutdown();
    EditorRecovery::Shutdown();
    EditorProject::Close();		// preview + manifest while the device is alive
    UIContentBrowser::Close();
    UIWorldOutliner::Close();
    ClearChooseEvents();
    xr_delete(m_WorldProperties);
    xr_delete(m_Properties);
    xr_delete(m_LeftBar);
    xr_delete(m_MainMenu);
    xr_delete(m_Render);
    xr_delete(m_TopBar);

    ExecCommand(COMMAND_DESTROY, (u32)0, (u32)0);
}

void UIMainForm::DropAsset(LPCSTR name)
{
    // The payload is the browser's own name; placement needs a library ref.
    // The translation depends on which source the browser is showing.
    string_path ref;
    if (UIContentBrowser::ResolveDropRef(name, ref))
    {
        // p2 bit0 -> place under the cursor, the ray was refreshed by the drop target
        ExecCommand(COMMAND_CB_PLACE_ASSET, xr_string(ref), u32(1));
        return;
    }
    // not an editable reference - but a compiled model still places, as a
    // raw-visual scene object (bit1 picks that loader in the command)
    if (UIContentBrowser::ResolveDropVisual(name, ref))
    {
        ExecCommand(COMMAND_CB_PLACE_ASSET, xr_string(ref), u32(1|2));
        return;
    }
    ELog.Msg(mtError, "'%s' cannot be placed in the scene - only .object references and .ogf models can.", name);
}

// ---- the Level tab ---------------------------------------------------------------------------------------------
int UIMainForm::SizeClass()
{
	const ImVec2 size = ImGui::GetMainViewport()->Size;
	return size.x >= XUI::Pt(1400.f) && size.y >= XUI::Pt(640.f) ? ClassWide : ClassCompact;
}

void UIMainForm::ResetLayout(bool all)
{
	XUI::Shell::ResetWorkspace(LevelWorkspace());
	if (EPrefs)
		EPrefs->level_layout_custom = false;
	if (all)
		XUI::Layout::ResetAll();
	if (MainForm)
		MainForm->m_ViewportMaximized = false;
}

void UIMainForm::ToggleViewportMaximized()
{
	m_ViewportMaximized = !m_ViewportMaximized;
	UI->RedrawScene();
}

bool UIMainForm::LevelEdit(LPCSTR action, bool run)
{
	const xr_string a = action ? action : "";
	u32 command = u32(-1);
	if (a == "undo")			command = COMMAND_UNDO;
	else if (a == "redo")		command = COMMAND_REDO;
	else if (a == "cut")		command = COMMAND_CUT;
	else if (a == "copy")		command = COMMAND_COPY;
	else if (a == "paste")		command = COMMAND_PASTE;
	else if (a == "delete")		command = COMMAND_DELETE_SELECTION;
	else if (a == "duplicate")
	{
		// the scene has no duplicate of its own: a copy pasted where it was
		if (run)
		{
			ExecCommand(COMMAND_COPY);
			ExecCommand(COMMAND_PASTE);
		}
		return true;
	}
	else if (a == "find")
	{
		if (run)
			UIWorldOutliner::Focus(true);
		return true;
	}
	if (command == u32(-1))
		return false;
	if (run)
		ExecCommand(command);
	return true;
}

void UIMainForm::RegisterLevel()
{
	XUI::Shell::DocDesc d;
	d.id = "level";
	d.kind = "level";
	d.icon = XUI::Icon::Level;
	d.closable = false;
	d.padded = false;
	d.label = [] { return SceneName(); };
	d.tooltip = [] { return Tools && !Tools->m_LastFileName.empty() ? xr_string(Tools->m_LastFileName.c_str()) : xr_string("The scene has not been saved yet"); };
	d.dirty = [] { return Scene && Scene->IsUnsaved(); };
	d.menus = [] { UIMainMenuForm::DrawLevelMenus(); };
	d.toolbar = [this](XUI::Toolbar& tb) { m_TopBar->DrawLevel(tb); };
	d.body = [this] { DrawLevel(); };
	d.inactive = [this] { KeepLevelAlive(); };
	d.status = [this] { DrawLevelStatus(); };
	d.edit = [this](LPCSTR action, bool run) { return LevelEdit(action, run); };
	// the scene's save runs deferred, like every save the UI asks for; the quit after the exit prompt waits for it
	d.save = [](xr_string&) { ExecCommand(COMMAND_SAVE, xr_string(LTools->m_LastFileName.c_str())); return true; };
	d.discard = [] { if (Scene) Scene->m_RTFlags.set(EScene::flRT_Unsaved, FALSE); };
	// nothing was drawn of the scene while another tab was up
	d.on_activate = [] { if (UI) UI->RedrawScene(); };
	XUI::Shell::Open(d, true);
}

void UIMainForm::KeepLevelAlive()
{
	XUI::Shell::Workspace ws;
	ws.name = LevelWorkspace();
	// the panels keep their dock nodes while they are not drawn
	XUI::Shell::WorkspaceDockSpace(ws, false);
}

void UIMainForm::DrawLevel()
{
	const int size_class = SizeClass();
	XUI::Shell::Workspace ws;
	ws.name = LevelWorkspace();
	ws.size_class = size_class;
	ws.build = [](unsigned root, int cls) { BuildLevelLayout(root, cls); };
	ws.custom = EPrefs ? &EPrefs->level_layout_custom : nullptr;
	ws.built = [this, size_class]
	{
		// a layout built anew has every panel of its class
		m_LeftBar->Open();
		UIWorldOutliner::Show();
		m_Properties->Open();
		m_WorldProperties->Open();
		if (size_class == ClassWide)
		{
			UIContentBrowser::Show();
			UILogForm::Show();
		}
	};
	if (m_ViewportMaximized)
	{
		XUI::Shell::WorkspaceDockSpace(ws, false);
		m_Render->DrawCanvas();
		return;
	}
	XUI::Shell::WorkspaceDockSpace(ws, true);

	// a floating panel stays inside the editor's window
	XUI::Shell::ClampNextFloatingWindow("Tools###LeftBar");
	m_LeftBar->Draw();
	XUI::Shell::ClampNextFloatingWindow("World Outliner");
	UIWorldOutliner::Update();
	XUI::Shell::ClampNextFloatingWindow("Details###Properties");
	m_Properties->Draw();
	XUI::Shell::ClampNextFloatingWindow("Level Settings###World Properties");
	m_WorldProperties->Draw();
	// the Compact class keeps the browser and the log in drawers, unless the author docked them
	const bool strip = size_class == ClassWide || (EPrefs && EPrefs->level_layout_custom);
	if (strip)
	{
		XUI::Shell::ClampNextFloatingWindow(UIContentBrowser::PanelName());
		UIContentBrowser::DrawPanel();
		XUI::Shell::ClampNextFloatingWindow(UILogForm::PanelName());
		UILogForm::DrawPanel();
	}
	m_Render->Draw();
}

void UIMainForm::DrawLevelStatus()
{
	if (!Scene)
		return;
	const int selected = Scene->SelectionCount(true, OBJCLASS_DUMMY);
	xr_string text;
	text.sprintf("%d selected", selected);
	Gizmo* gizmo = LTools->GetGimzo();
	char part[96];
	sprintf_s(part, "  \xC2\xB7  Grid %g%s", gizmo->GetStep(Gizmo::EType::Move), gizmo->IsStepEnable(Gizmo::EType::Move) ? "" : " (off)");
	text += part;
	sprintf_s(part, "  \xC2\xB7  Angle %g\xC2\xB0%s", gizmo->GetStep(Gizmo::EType::Rotate), gizmo->IsStepEnable(Gizmo::EType::Rotate) ? "" : " (off)");
	text += part;
	ImGui::PushStyleColor(ImGuiCol_Text, XFinedTheme::U32(XFinedTheme::ColorToken::TextSecondary));
	ImGui::TextUnformatted(text.c_str());
	ImGui::PopStyleColor();
}

bool UIMainForm::RebuildPending()
{
	return Scene && Scene->ObjCount() && EditorProject::Active() && (!Scene->m_RTFlags.is(EScene::flIsBuildedCForm) ||
		!Scene->m_RTFlags.is(EScene::flIsBuildedAIMap) || !Scene->m_RTFlags.is(EScene::flIsBuildedGameGraph));
}

void UIMainForm::DrawStatusRight(bool busy)
{
	const float h = XUI::Pt(20.f);
	const ImVec2 row = ImGui::GetCursorScreenPos();
	// every part is placed at `x`: a part that draws nothing (no job runs) leaves no item to line up with
	float x = row.x;

	// the rebuild chip (#30): what the scene's edits left for the level builders
	if (RebuildPending())
	{
		struct SItem { const char* name; u32 flag; void (*build)(); };
		static const SItem items[] =
		{
			{ "Collision",	EScene::flIsBuildedCForm,		[] { UI->DeferUIWork([] { SProgressOperation progress(*UI, false); Scene->BuildCForm(); }); } },
			{ "AI Map",		EScene::flIsBuildedAIMap,		[] { UI->DeferUIWork([] { SProgressOperation progress(*UI, false); Scene->BuildAIMap(); }); } },
			{ "Game Graph",	EScene::flIsBuildedGameGraph,	[] { UI->DeferUIWork([] { SProgressOperation progress(*UI, false); Scene->BuildGameGraph(); }); } },
		};
		xr_string label;
		for (const SItem& it : items)
			if (!Scene->m_RTFlags.is(it.flag))
			{
				label += label.empty() ? "Rebuild: " : " \xC2\xB7 ";
				label += it.name;
			}
		if (!label.empty())
		{
			bool clicked = false;
			ImVec2 chip_min, chip_max;
			// the chip is in the bar's small type; its popover is not
			{
				XUI::FontScope small_font(XUI::Font::Small);
				ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(6.f), (h - ImGui::GetFontSize()) * .5f));
				ImGui::PushStyleColor(ImGuiCol_Button, XFinedTheme::U32(XFinedTheme::ColorToken::Chrome, 0.f));
				ImGui::PushStyleColor(ImGuiCol_Text, XUI::ToneColor(XUI::Tone::Warning));
				ImGui::BeginDisabled(busy);
				ImGui::SetCursorScreenPos(ImVec2(x, row.y));
				clicked = ImGui::Button((label + "###rebuild_chip").c_str());
				chip_min = ImGui::GetItemRectMin();
				chip_max = ImGui::GetItemRectMax();
				x = chip_max.x + XUI::Pt(8.f);
				ImGui::EndDisabled();
				ImGui::PopStyleColor(2);
				ImGui::PopStyleVar();
				XUI::Tip("The scene changed since these were built. The game needs them rebuilt before it can play the level.");
			}
			if (clicked || XUI::Shell::TakePopup("rebuild"))
				XUI::Shell::OpenPopupAt("##rebuild", chip_min);
			// over the status bar, never over its own chip
			XUI::Shell::NextPopupAbove(chip_min, chip_max);
			if (ImGui::BeginPopup("##rebuild"))
			{
				// the Build buttons stand in one column, a clear gap after the longest name
				float name_w = 0.f;
				for (const SItem& it : items)
					if (!Scene->m_RTFlags.is(it.flag))
						name_w = _max(name_w, ImGui::CalcTextSize(it.name).x);
				const float column = ImGui::GetCursorPosX() + name_w + XUI::Pt(24.f);
				for (const SItem& it : items)
				{
					if (Scene->m_RTFlags.is(it.flag))
						continue;
					ImGui::PushID(it.name);
					ImGui::AlignTextToFramePadding();
					ImGui::TextUnformatted(it.name);
					ImGui::SameLine(column);
					const bool running = LTools->IsCompilerRunning() || LTools->IsGameRunning();
					if (XUI::Button("Build", XUI::Kind::Secondary, !running, "A compiler or the game is running."))
					{
						it.build();
						ImGui::CloseCurrentPopup();
					}
					ImGui::PopID();
				}
				ImGui::EndPopup();
			}
		}
	}

	// the jobs that run now
	{
		SProgressTaskInfoVec tasks;
		UIProgressCenter::Snapshot(*UI, tasks);
		ImGui::SetCursorScreenPos(ImVec2(x, row.y));
		UIProgressCenter::DrawChip(*UI, busy);
		if (!tasks.empty())
			x = ImGui::GetItemRectMax().x + XUI::Pt(8.f);
	}

	// the log's errors and warnings since the project opened (the start's own are under All): a click lists them
	xr_string last;
	int errors = 0, warnings = 0;
	UILogForm::State(last, errors, warnings);
	{
		XUI::FontScope small_font(XUI::Font::Small);
		char text[64];
		sprintf_s(text, "%d###log_errors", errors);
		ImGui::SetCursorScreenPos(ImVec2(x, row.y));
		const ImVec2 at = ImGui::GetCursorScreenPos();
		const float icon = XUI::Pt(12.f);
		ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(6.f) + icon + XUI::Pt(4.f), (h - ImGui::GetFontSize()) * .5f));
		ImGui::PushStyleColor(ImGuiCol_Button, XFinedTheme::U32(XFinedTheme::ColorToken::Chrome, 0.f));
		ImGui::BeginDisabled(busy);
		const bool err_clicked = ImGui::Button(text);
		XUI::DrawIcon(ImGui::GetWindowDrawList(), XUI::Icon::Error, ImVec2(at.x + XUI::Pt(6.f), at.y + (h - icon) * .5f), 12.f,
			errors ? XUI::ToneColor(XUI::Tone::Error) : XFinedTheme::U32(XFinedTheme::ColorToken::TextMuted));
		XUI::Tip("Errors since the project opened, a repeated one once. A click lists them in the Output Log.");
		ImGui::SetCursorScreenPos(ImVec2(ImGui::GetItemRectMax().x + XUI::Pt(2.f), row.y));
		sprintf_s(text, "%d###log_warnings", warnings);
		const ImVec2 at2 = ImGui::GetCursorScreenPos();
		const bool warn_clicked = ImGui::Button(text);
		x = ImGui::GetItemRectMax().x + XUI::Pt(8.f);
		XUI::DrawIcon(ImGui::GetWindowDrawList(), XUI::Icon::Warning, ImVec2(at2.x + XUI::Pt(6.f), at2.y + (h - icon) * .5f), 12.f,
			warnings ? XUI::ToneColor(XUI::Tone::Warning) : XFinedTheme::U32(XFinedTheme::ColorToken::TextMuted));
		XUI::Tip("Warnings since the project opened, a repeated one once. A click lists them in the Output Log.");
		ImGui::EndDisabled();
		ImGui::PopStyleColor();
		ImGui::PopStyleVar();
		if (err_clicked || warn_clicked)
		{
			// a chip opens the log on its own lines; a second click on the same chip closes it again
			const UILogForm::Level want = err_clicked ? UILogForm::Level::Error : UILogForm::Level::Warning;
			const bool drawer = XUI::Shell::OpenedDrawer() == XUI::Shell::DrawerLog;
			if (drawer && UILogForm::ShownLevel() == want)
				XUI::Shell::CloseDrawer();
			else
			{
				UILogForm::ShowLevel(want);
				// the log panel when it is on screen, the drawer otherwise
				if (!drawer)
					XUI::Shell::ToggleDrawer(XUI::Shell::DrawerLog);
			}
		}
	}

	// the MCP endpoint agents drive the editor through
	ImGui::SetCursorScreenPos(ImVec2(x, row.y + (h - XUI::Pt(12.f)) * .5f));
	const bool mcp = !!XFinedMCP::WakeEvent();
	XUI::StatusDot(mcp ? XUI::Tone::Success : XUI::Tone::Muted,
		mcp ? "MCP: agents drive the editor on 127.0.0.1:28016" : "MCP is not running");
}

// ---- the frame -----------------------------------------------------------------------------------------------
void UIMainForm::Draw()
{
    bOpen = true;
    // the caption in every state; the Start page, Link Game and Recovery have no menus and no tabs
    const bool linked = EditorProject::Active() && EditorProject::GameLinked() && !EditorRecovery::OfferPending();
    m_MainMenu->Draw(linked, false);
    // UE-style: until a project is opened, the browser page is the whole UI
    // (the MCP pump lives in TUI::Idle so it runs for background windows too)
    if (EditorProject::DrawBrowser())
    {
        s_CountedProject.clear();
        return;
    }
    // a project with no valid game link blocks the editor exactly the same way
    if (EditorProject::DrawGameLink())
        return;
    // The game is linked from here on - make its configs the editor's own.
    // Order matters: this must happen BEFORE the pending scene loads, or the
    // scene's spawn entities would be built by a factory reading the wrong
    // game's sections. Fingerprinted, so every later frame is a no-op.
    SyncGameConfigs();
    if (EditorRecovery::DrawStartupRecovery())
        return;
    // deferred last-scene autoload — first clean frame after the browser closed
    if (LPCSTR pending = EditorProject::PopPendingScene())
        ExecCommand(COMMAND_LOAD, xr_string(pending));
    // The status bar counts what came after the project opened, the scene it opens with included: the engine's notes
    // and the editor's own scripts as that scene loads are in the log, not in the counts. Deferred work runs after the
    // deferred load above.
    if (s_CountedProject != EditorProject::Root())
    {
        s_CountedProject = EditorProject::Root();
        UI->DeferUIWork([] { UILogForm::CountFromHere("The project opened - warnings and errors are counted from here"); });
    }
    XUI::Shell::DrawToolbar();
    XUI::Shell::DrawStatusBar();
    XUI::Shell::DrawWorkArea();
}

bool UIMainForm::Frame()
{
    EditorProject::Startup();
    // Idle pumps MCP before drawing, so linked configs must be ready before a remote scene load.
    SyncGameConfigs();
    if (!UI)
        return false;
    const bool keep_running = UI->Idle();
    if (keep_running)
        EditorRecovery::Tick();
    return keep_running;
}

void UIMainForm::DrawContextMenu()
{
    if (ImGui::BeginMenu("Edit"))
    {
        if (ImGui::MenuItem("Copy"))
        {
            ExecCommand(COMMAND_COPY);
        }
        if (ImGui::MenuItem("Paste"))
        {
            ExecCommand(COMMAND_PASTE);
        }
        ImGui::Separator();
        if (ImGui::MenuItem("Cut"))
        {
            ExecCommand(COMMAND_CUT);
        }
        ImGui::Separator();
        if (ImGui::MenuItem("Delete"))
        {
            ExecCommand(COMMAND_DELETE_SELECTION);
        }
        ImGui::EndMenu();
    }
    if (ImGui::BeginMenu("Visibility"))
    {
        if (ImGui::MenuItem("Hide Selected"))
        {
            ExecCommand(COMMAND_HIDE_SEL, FALSE);
        }
        if (ImGui::MenuItem("Hide Unselected"))
        {
            ExecCommand(COMMAND_HIDE_UNSEL);
        }
        if (ImGui::MenuItem("Hide All"))
        {
            ExecCommand(COMMAND_HIDE_ALL, FALSE);
        }
        ImGui::Separator();
        if (ImGui::MenuItem("Unhide All"))
        {
            ExecCommand(COMMAND_HIDE_ALL, TRUE);
        }
        ImGui::EndMenu();
    }
    ImGui::Separator();
    if (ImGui::MenuItem("Properties"))
    {
        ExecCommand(COMMAND_SHOW_PROPERTIES);
    }
}
