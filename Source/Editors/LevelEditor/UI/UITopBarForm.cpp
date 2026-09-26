#include "stdafx.h"
#include "UITopBarForm.h"
#include "../../XrECore/Editor/EditorModManifest.h"
#include "../../XrECore/Editor/EditorModPublish.h"

namespace
{
// the shortcut bound to a command now, as a menu shows it; "" when none is
xr_string BoundKeys(u32 command)
{
	ECommandVec& cmds = GetEditorCommands();
	if (command >= cmds.size() || !cmds[command] || cmds[command]->sub_commands.empty())
		return xr_string();
	return XUI::ShortcutLabel(cmds[command]->sub_commands[0]->shortcut);
}
}

UITopBarForm::UITopBarForm()
{
	m_VerifySpaceRestrictors = false;
	RefreshBar();
}

UITopBarForm::~UITopBarForm()
{
}

// Lowest priority first: what has a key and a menu item gives way first when the row is narrow.
void UITopBarForm::DrawLevel(XUI::Toolbar& tb)
{
	const bool running = LTools->IsCompilerRunning() || LTools->IsGameRunning();
	const char* busy = "A compiler or the game is running.";

	tb.Group(0);
	if (tb.Action("save", XUI::Icon::Save, "", "Save the scene (Ctrl+S)"))
		ClickSave();
	tb.Separator();
	if (tb.Action("undo", XUI::Icon::Undo, "", "Undo the last change (Ctrl+Z)"))
		ClickUndo();
	if (tb.Action("redo", XUI::Icon::Redo, "", "Redo the change that was undone (Ctrl+Y)"))
		ClickRedo();

	tb.Group(1);
	if (tb.Action("build_cform", XUI::Icon::BuildCollision, "", running ? busy : "Build the collision geometry (CForm) of this scene", !running))
		ClickCForm();
	if (tb.Action("build_aimap", XUI::Icon::BuildAIMap, "", running ? busy : "Build the AI map of this scene", !running))
		ClickAIMap();
	if (tb.Action("build_ggraph", XUI::Icon::BuildGameGraph, "", running ? busy : "Build the game graph of this scene", !running))
		ClickGGraph();
	if (tb.Action("build_all", XUI::Icon::BuildAll, "", running ? busy : "Compile the level and run the level builder (xrLC)", !running))
		ClickBuildAndMake();

	tb.Group(2);
	if (tb.Action("play", XUI::Icon::Play, "Play", running ? busy : "Play the scene inside the editor", !running))
		ClickPlayInEditor();
	if (tb.Menu("play_options", XUI::Icon::None, "", "Options for playing in the editor"))
	{
		XUI::MenuItem("Verify Space Restrictors", nullptr, &m_VerifySpaceRestrictors);
		XUI::Tip("Play checks the space restrictors of the scene before it starts.");
		ImGui::EndPopup();
	}
	if (tb.Action("play_game", XUI::Icon::PlayGame, "", running ? busy : "Build the scene for play and launch the game on it", !running))
		ClickPlayPC();
	if (tb.Action("play_clean", XUI::Icon::PlayClean, "", running ? busy : "Launch the game as it is, without building anything", !running))
		ClickPlayCleanGame();
	if (tb.Action("stop", XUI::Icon::Stop, "", running ? "Stop the compiler or the game that is running" : "Nothing is running", running))
		ClickTerminated();

	tb.Group(3);
	if (tb.Action("reload_configs", XUI::Icon::ReloadConfigs, "", running ? busy : "Re-read the game configs from disk (system.ltx, game.ltx)", !running))
		ClickReloadConfigs();
	if (tb.Action("game_folder", XUI::Icon::GameFolder, "", "Open the gamedata folder in Explorer"))
		ClickOpenGameData();
}

// Shipping the mod is the single most-used action in this editor, so it is the one Primary button of the row,
// on every tab: the level builds next to it are a different kind of thing.
void UITopBarForm::DrawProject(XUI::Toolbar& tb)
{
	if (!EditorProject::Active())
		return;
	char build_target[MAX_PATH] = {};
	EditorMod::BuildTargetText(build_target, sizeof(build_target));
	const xr_string keys = BoundKeys(COMMAND_MOD_BUILD);
	const xr_string build_tip = build_target[0]
		? xr_string("A clean build into the linked game") + (keys.empty() ? xr_string() : " (" + keys + ")") + ":\n" + build_target
		: xr_string("Link a game and set a valid module id first.");
	bool more = false;
	if (tb.Split("build_mod", XUI::Icon::BuildMod, "Build Mod", build_tip.c_str(), &more, EditorMod::CanBuildIntoGame()))
		ExecCommand(COMMAND_MOD_BUILD);
	if (more)
	{
		if (XUI::MenuItem("Build Into Game", keys.empty() ? nullptr : keys.c_str(), false, EditorMod::CanBuildIntoGame()))	ExecCommand(COMMAND_MOD_BUILD);
		if (XUI::MenuItem("Build to Folder..."))		EditorMod::RequestExportModule();
		if (XUI::MenuItem("Export Flat Gamedata..."))	EditorMod::RequestExportFlat();
		ImGui::Separator();
		if (XUI::MenuItem("Package Release..."))		EditorMod::RequestPackageRelease();
		ImGui::EndPopup();
	}
	// the other half of shipping: the same module, out to the players. The label keeps its width (a label that follows
	// the state would push the row around): what a running release does is in the tip, and the button is lit meanwhile
	char state[64] = {};
	EditorPublish::ButtonText(state, sizeof(state));
	const bool running = xr_strcmp(state, "Publish Release") != 0;
	xr_string publish_tip = running ? xr_string(state) + ".\n" : xr_string();
	publish_tip += "Package the module and publish it as a GitHub release.\nPlayers receive it through the game's Mods menu.";
	if (tb.Action("publish_release", XUI::Icon::Publish, "Publish Release", publish_tip.c_str(), EditorPublish::CanPublish(), running))
		ExecCommand(COMMAND_MOD_PUBLISH);
	if (tb.Action("releases", XUI::Icon::Releases, "Releases", "The releases already on GitHub.\nDeletes one together with its git tag.",
		EditorPublish::CanPublish()))
		ExecCommand(COMMAND_MOD_RELEASES);
}

void UITopBarForm::RefreshBar()
{/*
	{
		m_bSelect = false;
		m_bAdd = false;

		switch (Tools->GetAction()) {
		case etaSelect: 	m_bSelect = true; 	break;
		case etaAdd:    m_bAdd = true; 		break;
		default: THROW;
		}
	}
	// settings
	m_bCsLocal = Tools->GetSettings(etfCSParent);
	m_bNuScale = Tools->GetSettings(etfNUScale);
	m_bNSnap = Tools->GetSettings(etfNormalAlign);
	m_bGSnap = Tools->GetSettings(etfGSnap);
	m_bOSnap = Tools->GetSettings(etfOSnap);
	m_bMoveToSnap = Tools->GetSettings(etfMTSnap);
	m_bVSnap = Tools->GetSettings(etfVSnap);
	m_bASnap = Tools->GetSettings(etfASnap);
	m_bMSnap = Tools->GetSettings(etfMSnap);*/
}

void UITopBarForm::ClickUndo()
{
	ExecCommand(COMMAND_UNDO);
}

void UITopBarForm::ClickRedo()
{
	ExecCommand(COMMAND_REDO);
}

void UITopBarForm::ClickSave()
{
	ExecCommand(COMMAND_SAVE, xr_string(LTools->m_LastFileName.c_str()));
}
void UITopBarForm::ClickReloadConfigs()
{
	UI->DeferUIWork([]
	{
		SProgressOperation progress(*UI, false);
		xr_delete(pSettings);
		string_path si_name;
		FS.update_path(si_name, "$game_config$", "system.ltx");
		pSettings = xr_new<CInifile>(si_name, TRUE);
		xr_delete(pGameIni);
		string_path fname;
		FS.update_path(fname, "$game_config$", "game.ltx");
		pGameIni = xr_new<CInifile>(fname, TRUE);
		g_SEFactoryManager->reload();
		g_pGamePersistent->OnAppEnd();
		g_pGamePersistent->OnAppStart();
		Tools->UpdateProperties();
	});
}
void UITopBarForm::ClickOpenGameData()
{
	string_path GameDataPath;
	FS.update_path(GameDataPath, "$game_data$", "");
	ShellExecuteA(NULL, "open", GameDataPath, NULL, NULL, SW_SHOWDEFAULT);
}
void UITopBarForm::ClickCForm()
{
	UI->DeferUIWork([]
	{
		SProgressOperation progress(*UI, false);
		Scene->BuildCForm();
	});
}
void UITopBarForm::ClickAIMap()
{
	UI->DeferUIWork([]
	{
		SProgressOperation progress(*UI, false);
		Scene->BuildAIMap();
	});
}
void UITopBarForm::ClickGGraph()
{
	UI->DeferUIWork([]
	{
		SProgressOperation progress(*UI, false);
		Scene->BuildGameGraph();
	});
}
void UITopBarForm::ClickPlayInEditor()
{
	UI->DeferUIWork([] { Scene->Play(); });
}
void UITopBarForm::ClickBuildAndMake()
{
	UI->DeferUIWork([]
	{
		if (Builder.Compile(false, false))
			LTools->RunXrLC();
	});
}
void UITopBarForm::ClickTerminated()
{
	LTools->Terminated();
}
void UITopBarForm::ClickPlayPC()
{
	UI->DeferUIWork([]
	{
		SProgressOperation progress(*UI, false);
		if (Scene->BuildForPCPlay())
			LTools->RunGame("-editor_scene -start server(editor/single/alife/new) client(localhost) -nointro -noprefetch");
	});
}
void UITopBarForm::ClickPlayCleanGame()
{
	LTools->RunGame("");
}
