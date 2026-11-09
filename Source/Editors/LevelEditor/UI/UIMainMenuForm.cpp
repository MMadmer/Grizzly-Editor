#include "stdafx.h"
#include "../Editor/GameLightingScene.h"
#include "..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\XrECore\Editor\EditorModPublish.h"
#include "..\..\XrECore\Editor\EditorCatalogSubmit.h"
#include "..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\XrECore\Editor\EditorWorld.h"
#include "..\..\XrECore\Editor\EditorScreens.h"
#include "..\..\XrECore\Editor\EditorPda.h"
#include "..\..\XrECore\Editor\UILogForm.h"
#include "..\Editor\EditorModScene.h"
#include "..\Editor\Recovery\EditorRecovery.h"
#include "UITitleBar.h"

// The caption's menus (docs/UI_DESIGN.md, section 5). The labels are the ones the author reads; the commands behind
// them keep their ids and menu paths, which are MCP names (list_commands, exec_command). A shortcut is shown as the
// keycaps of what is bound now, never a literal that could drift from the binding.
namespace
{
bool s_Folded = false;

// the shortcut bound to a command, as the menu shows it; `fallback` when none is
const char* Keys(u32 command, const char* fallback = nullptr, u32 sub = 0)
{
	static xr_string s_Labels[4];
	static int s_Next = 0;
	ECommandVec& cmds = GetEditorCommands();
	if (command >= cmds.size() || !cmds[command] || sub >= cmds[command]->sub_commands.size())
		return fallback;
	const xr_shortcut& shortcut = cmds[command]->sub_commands[sub]->shortcut;
	if (!shortcut.key)
		return fallback;
	// a few labels live at once: one menu item may ask for two
	xr_string& label = s_Labels[s_Next++ & 3];
	label = XUI::ShortcutLabel(shortcut);
	return label.empty() ? fallback : label.c_str();
}

bool LevelActive()
{
	return XUI::Shell::IsActive("level");
}

bool Menu(const char* label, bool enabled = true)
{
	return XUI::Shell::BeginCaptionMenu(label, enabled);
}

// ---- File ----------------------------------------------------------------------------------------------------
// Back to the Start page. Ask BEFORE closing anything - the unsaved tabs, then the scene: COMMAND_CLEAR's result
// was once ignored here, so cancelling the scene's prompt still closed the project and took the scene with it. The
// project's tabs close with it and come back from its session when it opens again.
void SwitchProject()
{
	xr_vector<XUI::Shell::DocInfo> tabs;
	XUI::Shell::List(tabs);
	XUI::Strings what;
	xr_vector<xr_string> ids;
	for (const XUI::Shell::DocInfo& tab : tabs)
		if (tab.dirty && tab.id != "level")
		{
			what.push_back(tab.label);
			ids.push_back(tab.id);
		}
	// Out of the UI pass: the card's picture renders the scene before it is cleared, and a Yes to the scene's
	// own question saves at once instead of being queued behind the clear
	auto close = []
	{
		UI->DeferUIWork([]
		{
			if (!EditorProject::Active() || !Scene->IfModified())
				return;
			EditorProject::CapturePreview();
			if (!(BOOL)ExecCommand(COMMAND_CLEAR))
				return;
			XUI::Shell::CloseProjectTabs();
			EditorProject::Close();
		});
	};
	if (what.empty())
	{
		close();
		return;
	}
	XUI::UnsavedPrompt("switch_project", what, [ids, close](XUI::Answer answer)
	{
		if (answer == XUI::Answer::Cancel)
			return;
		for (const xr_string& id : ids)
		{
			xr_string err;
			if (answer == XUI::Answer::No)
				XUI::Shell::DiscardTab(id.c_str());
			else if (!XUI::Shell::SaveTab(id.c_str(), &err))
			{
				XUI::Notice(XUI::Tone::Error, "Save failed", err.c_str());
				return;
			}
		}
		close();
	});
}

void MenuFile()
{
	if (!Menu("File"))
		return;
	if (XUI::MenuItem("New Scene", Keys(COMMAND_CLEAR)))			{ XUI::Shell::Activate("level"); ExecCommand(COMMAND_CLEAR); }
	if (XUI::MenuItem("Open Scene...", Keys(COMMAND_LOAD)))			{ XUI::Shell::Activate("level"); ExecCommand(COMMAND_LOAD); }
	if (ImGui::BeginMenu("Recent Scene", !EPrefs->scene_recent_list.empty()))
	{
		for (auto& str : EPrefs->scene_recent_list)
			if (ImGui::MenuItem(str.c_str()))
			{
				XUI::Shell::Activate("level");
				ExecCommand(COMMAND_LOAD, str);
			}
		ImGui::EndMenu();
	}
	ImGui::Separator();
	// Save is the active tab's: the scene on Level, the document elsewhere (Ctrl+S is the shell's)
	if (XUI::MenuItem("Save", "Ctrl+S"))
		XUI::Shell::SaveActive();
	if (XUI::MenuItem("Save As...", Keys(COMMAND_SAVE, nullptr, 1), false, LevelActive()))
		ExecCommand(COMMAND_SAVE, 0, 1);
	if (!LevelActive())
		XUI::Tip("Saves the scene under another name: on the Level tab.");
	ImGui::Separator();
	if (ImGui::BeginMenu("Project"))
	{
		if (XUI::MenuItem("Import Base Scene..."))	EditorProject::RequestImportScene();
		if (XUI::MenuItem("Open Project Folder"))	EditorProject::OpenProjectFolder();
		ImGui::Separator();
		if (XUI::MenuItem("Switch Project..."))
			SwitchProject();
		XUI::Tip("Closes this project and goes back to the Start page.");
		ImGui::EndMenu();
	}
	ImGui::Separator();
	// Quit is the Exit command's binding, as Preferences and the palette name it
	if (XUI::MenuItem("Quit", Keys(COMMAND_EXIT)))
		XUI::Shell::RequestExit();
	ImGui::EndMenu();
}

// ---- Edit ----------------------------------------------------------------------------------------------------
// the active tab's action; Level shows the scene's own binding, a document the one every editor uses
void EditItem(const char* label, const char* action, u32 level_command, const char* keys)
{
	const char* shown = LevelActive() && level_command != u32(-1) ? Keys(level_command, keys) : keys;
	if (XUI::MenuItem(label, shown, false, XUI::Shell::Edit(action, false)))
		XUI::Shell::Edit(action, true);
}

void MenuEdit()
{
	if (!Menu("Edit"))
		return;
	EditItem("Undo", "undo", COMMAND_UNDO, "Ctrl+Z");
	EditItem("Redo", "redo", COMMAND_REDO, "Ctrl+Y");
	ImGui::Separator();
	EditItem("Cut", "cut", COMMAND_CUT, "Ctrl+X");
	EditItem("Copy", "copy", COMMAND_COPY, "Ctrl+C");
	EditItem("Paste", "paste", COMMAND_PASTE, "Ctrl+V");
	EditItem("Duplicate", "duplicate", u32(-1), "Ctrl+D");
	EditItem("Delete", "delete", COMMAND_DELETE_SELECTION, "Del");
	ImGui::Separator();
	EditItem("Find", "find", COMMAND_SHOW_OBJECTLIST, "Ctrl+F");
	ImGui::Separator();
	if (XUI::MenuItem("Command Palette...", Keys(COMMAND_COMMAND_PALETTE, "Ctrl+Shift+P")))	ExecCommand(COMMAND_COMMAND_PALETTE);
	if (XUI::MenuItem("Preferences", Keys(COMMAND_EDITOR_PREF)))							ExecCommand(COMMAND_EDITOR_PREF);
	ImGui::EndMenu();
}

// ---- Scene, Build (Level) ------------------------------------------------------------------------------------------
void MarkSelection(bool mod_content)
{
	// objects placed before this flag existed (or intentionally traded between mod and base) are re-labelled
	// here; the build ships only mod content, an imported base level stays the game's
	int n = 0;
	ObjectList sel;
	if (Scene && Scene->GetQueryObjects(sel, OBJCLASS_DUMMY, 1, -1, -1))
		for (ObjectIt it = sel.begin(); it != sel.end(); ++it)
			if (!!(*it)->IsAuthorPlaced() != mod_content)
			{
				(*it)->SetAuthorPlaced(mod_content ? TRUE : FALSE);
				++n;
			}
	if (n) Scene->UndoSave();
	ELog.Msg(mtInformation, mod_content ? "Mod content: %d object(s) marked." : "Base level: %d object(s) unmarked.", n);
}

void MenuScene()
{
	if (!Menu("Scene"))
		return;
	if (XUI::MenuItem("Validate", Keys(COMMAND_VALIDATE_SCENE)))		ExecCommand(COMMAND_VALIDATE_SCENE);
	if (XUI::MenuItem("Summary Info"))
	{
		ExecCommand(COMMAND_CLEAR_SCENE_SUMMARY);
		ExecCommand(COMMAND_COLLECT_SCENE_SUMMARY);
		ExecCommand(COMMAND_SHOW_SCENE_SUMMARY);
	}
	if (XUI::MenuItem("Highlight Texture..."))						ExecCommand(COMMAND_SCENE_HIGHLIGHT_TEXTURE);
	if (XUI::MenuItem("Find in World Outliner", Keys(COMMAND_SHOW_OBJECTLIST)))	ExecCommand(COMMAND_SHOW_OBJECTLIST);
	if (XUI::MenuItem("Clear Debug Draw"))							ExecCommand(COMMAND_CLEAR_DEBUG_DRAW);
	ImGui::Separator();
	if (XUI::MenuItem("Open Selection..."))							ExecCommand(COMMAND_LOAD_SELECTION);
	if (XUI::MenuItem("Save Selection As..."))						ExecCommand(COMMAND_SAVE_SELECTION);
	if (XUI::MenuItem("Export Scene as OBJ..."))					Scene->ExportObj(false);
	if (XUI::MenuItem("Export Selection as OBJ..."))				Scene->ExportObj(true);
	ImGui::Separator();
	if (XUI::MenuItem("Mark Selection as Mod Content"))				MarkSelection(true);
	XUI::Tip("The build ships these objects: they are the module's own.");
	if (XUI::MenuItem("Mark Selection as Base Level"))				MarkSelection(false);
	XUI::Tip("The build leaves these objects out: they belong to the imported base level.");
	ImGui::Separator();
	{
		bool any_class = !!Tools->GetSettings(etfPickAnyClass);
		if (XUI::MenuItem("Pick Any Class", nullptr, &any_class))
			ExecCommand(COMMAND_SET_SETTINGS, etfPickAnyClass, any_class);
		XUI::Tip("A click selects an object of any class and switches the active tool to it.");
	}
	if (XUI::MenuItem("Create Recovery Snapshot", nullptr, false, EditorRecovery::CanCreateSnapshot()))
		EditorRecovery::RequestSnapshot();
	ImGui::EndMenu();
}

void MenuBuild()
{
	if (!Menu("Build"))
		return;
	if (ImGui::BeginMenu("Make"))
	{
		if (XUI::MenuItem("Make All..."))		ExecCommand(COMMAND_BUILD);
		XUI::Tip("Empties the level folder and builds every level file of this scene into it.");
		ImGui::Separator();
		if (XUI::MenuItem("Make Game..."))		ExecCommand(COMMAND_MAKE_GAME);
		if (XUI::MenuItem("Make Details..."))	ExecCommand(COMMAND_MAKE_DETAILS);
		if (XUI::MenuItem("Make HOM..."))		ExecCommand(COMMAND_MAKE_HOM);
		if (XUI::MenuItem("Make SOM..."))		ExecCommand(COMMAND_MAKE_SOM);
		if (XUI::MenuItem("Make AI Map..."))	ExecCommand(COMMAND_MAKE_AIMAP);
		ImGui::EndMenu();
	}
	const bool running = LTools->IsCompilerRunning() || LTools->IsGameRunning();
	if (ImGui::BeginMenu("Compile", !running))
	{
		if (XUI::MenuItem("Geometry & Light"))		LTools->RunXrLC();
		if (XUI::MenuItem("Detail Object Light"))	LTools->RunXrDO();
		if (ImGui::BeginMenu("AI Map"))
		{
			if (XUI::MenuItem("High"))		LTools->RunXrAI_AIMap(true);
			if (XUI::MenuItem("Low"))		LTools->RunXrAI_AIMap(false);
			if (XUI::MenuItem("Verify"))	LTools->RunXrAI_Verify();
			ImGui::EndMenu();
		}
		if (ImGui::BeginMenu("Spawn"))
		{
			if (XUI::MenuItem("This Level"))	LTools->RunXrAI_Spawn(true);
			if (XUI::MenuItem("All Levels"))	LTools->RunXrAI_Spawn(false);
			ImGui::EndMenu();
		}
		ImGui::EndMenu();
	}
	if (running)
		XUI::Tip("A compiler or the game is running.");
	ImGui::Separator();
	if (XUI::MenuItem("Import Error List..."))	ExecCommand(COMMAND_IMPORT_COMPILER_ERROR);
	if (XUI::MenuItem("Export Error List..."))	ExecCommand(COMMAND_EXPORT_COMPILER_ERROR);
	if (XUI::MenuItem("Clear Error List"))		ExecCommand(COMMAND_CLEAR_DEBUG_DRAW);
	ImGui::EndMenu();
}

// ---- Mod ---------------------------------------------------------------------------------------------------
void MenuMod()
{
	if (!Menu("Mod"))
		return;
	if (XUI::MenuItem("Mod Settings", Keys(COMMAND_MOD_EDIT_MANIFEST)))	EditorMod::RequestEditManifest();
	XUI::Tip("The module's manifest: its name, id, version, mode, relations and what the build leaves out.");
	if (XUI::MenuItem("Characters", Keys(COMMAND_MOD_CHARACTERS)))		EditorCharacters::Show();
	XUI::Tip("The module's own NPCs: name, model, community and starting kit.");
	if (XUI::MenuItem("World Rules", Keys(COMMAND_MOD_WORLD)))			EditorWorld::Show();
	XUI::Tip("What the mode keeps of the original game, the levels the player reaches and the Player Start.");
	if (XUI::MenuItem("Screens & Music"))									EditorScreens::Show();
	XUI::Tip("The module's loading and menu screens and the music that goes with them.");
	if (XUI::MenuItem("PDA Interface", Keys(COMMAND_MOD_PDA)))			EditorPda::ShowModuleLayout();
	XUI::Tip("The module's own PDA screens, in a preview that plays them.");
	ImGui::Separator();

	char build_target[MAX_PATH] = {};
	EditorMod::BuildTargetText(build_target, sizeof(build_target));
	if (XUI::MenuItem("Build Into Game", Keys(COMMAND_MOD_BUILD), false, EditorMod::CanBuildIntoGame()))
		EditorMod::RequestBuildIntoGame();
	{
		const xr_string tip = build_target[0]
			? xr_string("A clean build into the linked game:\n") + build_target + "\nFiles the project no longer has are removed there."
			: xr_string("Link a game and set a valid module id first.");
		XUI::Tip(tip.c_str());
	}
	if (XUI::MenuItem("Build to Folder..."))			EditorMod::RequestExportModule();
	if (XUI::MenuItem("Export Flat Gamedata..."))		EditorMod::RequestExportFlat();
	if (ImGui::BeginMenu("Bake Scene Layer"))
	{
		// level deltas baked from the current selection: spawn ops first, they are the composable way to
		// put something on a base level
		if (XUI::MenuItem("Spawn Layer..."))			EditorModScene::RequestExportSpawn();
		if (XUI::MenuItem("Collision Overlay..."))		EditorModScene::RequestExportXCForm();
		if (XUI::MenuItem("Visual Overlay..."))			EditorModScene::RequestExportOGF();
		// the selection used as volumes: removes base collision and visuals
		if (XUI::MenuItem("Level Cut..."))				EditorModScene::RequestExportCut();
		ImGui::EndMenu();
	}
	ImGui::Separator();
	if (XUI::MenuItem("Package Release...", Keys(COMMAND_MOD_PACKAGE)))	EditorMod::RequestPackageRelease();
	XUI::Tip("The packages and the update descriptor of a GitHub release.");
	if (XUI::MenuItem("Publish Release...", Keys(COMMAND_MOD_PUBLISH), false, EditorPublish::CanPublish()))	EditorPublish::RequestPublish();
	XUI::Tip(EditorPublish::CanPublish() ? "Packages the module and publishes it as a GitHub release. Players receive it through the game's Mods menu."
		: "Set the GitHub repository in Mod Settings first.");
	if (XUI::MenuItem("Releases", Keys(COMMAND_MOD_RELEASES), false, EditorPublish::CanPublish()))	EditorPublish::RequestReleases();
	XUI::Tip(EditorPublish::CanPublish() ? "The releases the repository holds, drafts included." : "Set the GitHub repository in Mod Settings first.");
	ImGui::Separator();
	if (XUI::MenuItem("Submit to Mod Browser...", Keys(COMMAND_MOD_CATALOG), false, EditorCatalogSubmit::CanRequest()))
		EditorCatalogSubmit::Request();
	XUI::Tip("Lists the module in the in-game Mod Browser.");
	ImGui::EndMenu();
}

// ---- Libraries -------------------------------------------------------------------------------------------------
void RewriteThumbnailsNow()
{
	SProgressOperation progress(*UI, false);
	FS_FileSet files;
	FS.file_list(files, _textures_, FS_ListFiles, "*.thm");
	for (FS_FileSet::iterator it = files.begin(); it != files.end(); ++it)
	{
		ETextureThumbnail* thm = xr_new<ETextureThumbnail>(it->name.c_str(), false);
		thm->Load(it->name.c_str(), _textures_);
		thm->Save();
		xr_delete(thm);
	}
	ELog.Msg(mtInformation, "Texture descriptions rewritten: %d.", int(files.size()));
}

// rewrites every .thm of the texture library in place: it names how many, the neighbouring Synchronize commands ask too
void RewriteThumbnails()
{
	FS_FileSet files;
	FS.file_list(files, _textures_, FS_ListFiles, "*.thm");
	if (files.empty())
	{
		XUI::Notice(XUI::Tone::Info, "Rewrite Texture Descriptions", "The texture library holds no texture descriptions.");
		return;
	}
	XUI::ConfirmSpec spec;
	spec.id = "rewrite_thumbnails";
	spec.title = "Rewrite Texture Descriptions";
	spec.headline = "Rewrite every texture description (.thm) of the texture library?";
	for (FS_FileSet::iterator it = files.begin(); it != files.end() && spec.items.size() < 10; ++it)
		spec.items.push_back(it->name.c_str());
	spec.total = u32(files.size());
	spec.consequence = "Each file is read and written again in place, in the format this editor writes.";
	spec.action = "Rewrite";
	XUI::Confirm(spec, [](XUI::Answer answer)
	{
		if (answer == XUI::Answer::Yes)
			UI->DeferUIWork(&RewriteThumbnailsNow);
	});
}

void MenuLibraries()
{
	if (!Menu("Libraries"))
		return;
	if (XUI::MenuItem("Light Animations", Keys(COMMAND_LIGHTANIM_EDITOR)))	ExecCommand(COMMAND_LIGHTANIM_EDITOR);
	if (XUI::MenuItem("Texture Library", Keys(COMMAND_IMAGE_EDITOR)))		ExecCommand(COMMAND_IMAGE_EDITOR);
	if (XUI::MenuItem("Sound Library", Keys(COMMAND_SOUND_EDITOR)))			ExecCommand(COMMAND_SOUND_EDITOR);
	ImGui::Separator();
	if (XUI::MenuItem("Check New Textures"))		ExecCommand(COMMAND_CHECK_TEXTURES);
	if (XUI::MenuItem("Synchronize Textures..."))	ExecCommand(COMMAND_REFRESH_TEXTURES);
	if (XUI::MenuItem("Synchronize Sounds..."))		ExecCommand(COMMAND_SYNC_SOUNDS);
	ImGui::Separator();
	if (XUI::MenuItem("Reload Objects"))			ExecCommand(COMMAND_RELOAD_OBJECTS);
	if (XUI::MenuItem("Reload Textures"))			ExecCommand(COMMAND_RELOAD_TEXTURES);
	if (XUI::MenuItem("Rewrite Texture Descriptions..."))	RewriteThumbnails();
	if (XUI::MenuItem("Refresh Sound Environment Geometry"))	ExecCommand(COMMAND_REFRESH_SOUND_ENV_GEOMETRY);
	ImGui::EndMenu();
}

// ---- Window, Help ------------------------------------------------------------------------------------------------
void PanelItem(const char* label, bool open, void (*show)(), void (*hide)(), const char* window)
{
	bool selected = open;
	if (XUI::MenuItem(label, nullptr, &selected))
	{
		if (selected)
		{
			// a panel lives on the Level tab: opening one shows it there
			XUI::Shell::Activate("level");
			show();
			XUI::Shell::FocusWindow(window);
		}
		else
			hide();
	}
}

void MenuWindow()
{
	if (!Menu("Window"))
		return;
	PanelItem("Tools", !MainForm->GetLeftBarForm()->IsClosed(), [] { MainForm->GetLeftBarForm()->Open(); },
		[] { MainForm->GetLeftBarForm()->Close(); }, "Tools###LeftBar");
	PanelItem("World Outliner", UIWorldOutliner::IsOpen(), [] { UIWorldOutliner::Show(); }, [] { UIWorldOutliner::Close(); }, "World Outliner");
	PanelItem("Details", !MainForm->GetPropertiesFrom()->IsClosed(), [] { MainForm->GetPropertiesFrom()->Open(); },
		[] { MainForm->GetPropertiesFrom()->Close(); }, "Details###Properties");
	PanelItem("Level Settings", !MainForm->GetWorldPropertiesFrom()->IsClosed(), [] { MainForm->GetWorldPropertiesFrom()->Open(); },
		[] { MainForm->GetWorldPropertiesFrom()->Close(); }, "Level Settings###World Properties");
	PanelItem("Content Browser", UIContentBrowser::IsOpen(), [] { UIContentBrowser::Show(); }, [] { UIContentBrowser::Close(); },
		UIContentBrowser::PanelName());
	PanelItem("Output Log", UILogForm::PanelOpen(), [] { UILogForm::Show(); }, [] { UILogForm::Hide(); }, UILogForm::PanelName());
	ImGui::Separator();
	if (XUI::MenuItem("Content Drawer", "Ctrl+Space", XUI::Shell::OpenedDrawer() == XUI::Shell::DrawerContent))
		XUI::Shell::ToggleDrawer(XUI::Shell::DrawerContent);
	if (XUI::MenuItem("Log Drawer", "Ctrl+`", XUI::Shell::OpenedDrawer() == XUI::Shell::DrawerLog))
		XUI::Shell::ToggleDrawer(XUI::Shell::DrawerLog);
	ImGui::Separator();
	// the open documents, the way Ctrl+Tab walks them
	xr_vector<XUI::Shell::DocInfo> tabs;
	XUI::Shell::List(tabs);
	for (const XUI::Shell::DocInfo& tab : tabs)
	{
		ImGui::PushID(tab.id.c_str());
		if (XUI::MenuItem(tab.dirty ? (tab.label + " *").c_str() : tab.label.c_str(), nullptr, tab.active))
			XUI::Shell::Activate(tab.id.c_str());
		if (!tab.path.empty())
			XUI::Tip(tab.path.c_str());
		ImGui::PopID();
	}
	ImGui::Separator();
	// the Level layout for the window's size, every panel back
	if (XUI::MenuItem("Reset Layout"))
		UIMainForm::ResetLayout(false);
	XUI::Tip("Puts the Level tab's panels back where the layout for this window size has them.");
	ImGui::EndMenu();
}

void MenuHelp()
{
	if (!Menu("Help"))
		return;
	if (XUI::MenuItem("Keyboard Shortcuts"))
	{
		EPrefs->Edit();
		EPrefs->ShowPage("Keyboard");
	}
	XUI::Tip("Every shortcut, listed and rebound: the Keyboard page of Preferences.");
	if (XUI::MenuItem("About Grizzly Editor"))
		XUI::Notice(XUI::Tone::Info, "About Grizzly Editor",
			"Grizzly Editor - the level and mod editor of Dead Air Refined.\nAgents drive it over MCP on 127.0.0.1:28016.");
	ImGui::EndMenu();
}

void DrawMenus()
{
	MenuFile();
	MenuEdit();
	XUI::Shell::DrawDocMenus();
	MenuMod();
	MenuLibraries();
	MenuWindow();
	MenuHelp();
}

// Under 1100 pt the menus fold into one button, so the tabs keep their room: its popup holds every menu as a submenu.
// A menu asked for over MCP (ui_popup menu:<name>) opens the fold first, then itself inside it.
void DrawFolded()
{
	const ImVec2 bar_pos = ImGui::GetWindowPos(), bar_size = ImGui::GetWindowSize();
	const ImVec2 cursor = ImGui::GetCursorScreenPos();
	const float side = XUI::Pt(24.f);
	const ImVec2 at(cursor.x, bar_pos.y + (bar_size.y - side) * .5f);
	ImGui::SetCursorScreenPos(at);
	const bool open = ImGui::IsPopupOpen("##caption_menus");
	const bool clicked = XUI::IconButton("##caption_menu", XUI::Icon::Menu, "Menus", open);
	if (clicked || XUI::Shell::TakePopup("menu:Menu") || (!open && XUI::Shell::PopupRequested("menu:")))
		XUI::Shell::OpenPopupAt("##caption_menus", ImVec2(at.x, bar_pos.y + bar_size.y));
	if (ImGui::BeginPopup("##caption_menus", ImGuiWindowFlags_NoSavedSettings))
	{
		DrawMenus();
		ImGui::EndPopup();
	}
	ImGui::SetCursorScreenPos(ImVec2(at.x + side + XUI::Pt(4.f), cursor.y));
}
}

UIMainMenuForm::UIMainMenuForm()
{
}

UIMainMenuForm::~UIMainMenuForm()
{
}

bool UIMainMenuForm::Folded()
{
	return s_Folded;
}

void UIMainMenuForm::DrawLevelMenus()
{
	MenuScene();
	MenuBuild();
	if (Menu("View"))
	{
		DrawViewportOptions();
		ImGui::Separator();
		if (XUI::MenuItem("Maximize Viewport", "F11", MainForm->ViewportMaximized()))
			MainForm->ToggleViewportMaximized();
		ImGui::EndMenu();
	}
}

void UIMainMenuForm::DrawViewportOptions()
{
	if (ImGui::BeginMenu("Render"))
	{
		if (ImGui::BeginMenu("GPU"))
		{
			{
				xr_string active = "Active: ";
				active += GetActiveGpuName();
				ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::TextSecondary));
				ImGui::TextUnformatted(active.c_str());
				ImGui::PopStyleColor();
			}
			ImGui::Separator();

			// Real adapters, by name, as the runtime reports them - the device is created on the one picked here.
			// The driver-level preference below only nudges Windows and cannot pick a specific card, which is
			// why both exist.
			CLevelPreferences* prefs = dynamic_cast<CLevelPreferences*>(EPrefs);
			{
				bool is_default = !prefs || prefs->GpuAdapter.empty();
				if (XUI::MenuItem("System Default", nullptr, &is_default) && is_default && prefs)
					prefs->GpuAdapter = "";
			}
			const int gpu_count = GpuAdapterCount();
			for (int i = 0; i < gpu_count; ++i)
			{
				LPCSTR name = GpuAdapterName(i);
				bool selected = prefs && (0 == xr_strcmp(prefs->GpuAdapter.c_str(), name));
				ImGui::PushID(i);
				if (XUI::MenuItem(name, nullptr, &selected) && selected && prefs)
					prefs->GpuAdapter = name;
				ImGui::PopID();
			}
			if (!gpu_count)
				XUI::MenuItem("No adapters reported", nullptr, false, false);

			ImGui::Separator();
			const EGpuPreference current = GetGpuPreference();
			struct { EGpuPreference pref; const char* title; } items[] = {
				{ gpuAuto,         "Driver Hint: Automatic" },
				{ gpuPerformance,  "Driver Hint: High Performance" },
				{ gpuPowerSaving,  "Driver Hint: Power Saving" },
			};
			for (auto& it : items)
			{
				bool selected = (current == it.pref);
				if (XUI::MenuItem(it.title, nullptr, &selected) && selected)
					SetGpuPreference(it.pref);
			}
			ImGui::Separator();
			XUI::MenuItem("Takes effect after a restart", nullptr, false, false);
			ImGui::EndMenu();
		}
		if (ImGui::BeginMenu("Quality"))
		{
			// the device's own scale, so this menu and the viewport's never disagree
			struct { const char* label; float q; } qualities[] = { { "25%", .25f }, { "50%", .5f }, { "100%", 1.f }, { "200%", 2.f } };
			for (auto& it : qualities)
				if (XUI::MenuItem(it.label, nullptr, fabsf(EDevice->m_ScreenQuality - it.q) < .01f))
				{
					UI->SetRenderQuality(it.q);
					UI->RedrawScene();
				}
			ImGui::EndMenu();
		}
		if (ImGui::BeginMenu("Fill Mode"))
		{
			if (XUI::MenuItem("Point", nullptr, EDevice->dwFillMode == D3DFILL_POINT))			{ EDevice->dwFillMode = D3DFILL_POINT; UI->RedrawScene(); }
			if (XUI::MenuItem("Wireframe", nullptr, EDevice->dwFillMode == D3DFILL_WIREFRAME))	{ EDevice->dwFillMode = D3DFILL_WIREFRAME; UI->RedrawScene(); }
			if (XUI::MenuItem("Solid", nullptr, EDevice->dwFillMode == D3DFILL_SOLID))			{ EDevice->dwFillMode = D3DFILL_SOLID; UI->RedrawScene(); }
			ImGui::EndMenu();
		}
		if (ImGui::BeginMenu("Shade Mode"))
		{
			if (XUI::MenuItem("Flat", nullptr, EDevice->dwShadeMode == D3DSHADE_FLAT))			{ EDevice->dwShadeMode = D3DSHADE_FLAT; UI->RedrawScene(); }
			if (XUI::MenuItem("Gouraud", nullptr, EDevice->dwShadeMode == D3DSHADE_GOURAUD))	{ EDevice->dwShadeMode = D3DSHADE_GOURAUD; UI->RedrawScene(); }
			ImGui::EndMenu();
		}
		{
			bool selected = psDeviceFlags.test(rsEdgedFaces);
			if (XUI::MenuItem("Edged Faces", nullptr, &selected))		{ psDeviceFlags.set(rsEdgedFaces, selected); UI->RedrawScene(); }
		}
		ImGui::Separator();
		{
			bool selected = !HW.Caps.bForceGPU_SW;
			if (XUI::MenuItem("Hardware Rendering", nullptr, &selected))	{ HW.Caps.bForceGPU_SW = !selected; UI->Resize(); }
		}
		ImGui::Separator();
		{
			bool selected = psDeviceFlags.test(rsFilterLinear);
			if (XUI::MenuItem("Linear Filtering", nullptr, &selected))	{ psDeviceFlags.set(rsFilterLinear, selected); UI->RedrawScene(); }
		}
		{
			bool selected = psDeviceFlags.test(rsRenderTextures);
			if (XUI::MenuItem("Textures", nullptr, &selected))			{ psDeviceFlags.set(rsRenderTextures, selected); UI->RedrawScene(); }
		}
		ImGui::EndMenu();
	}
	ImGui::Separator();
	{
		bool selected = psDeviceFlags.test(rsDrawSafeRect);
		if (XUI::MenuItem("Safe Rect", Keys(COMMAND_TOGGLE_SAFE_RECT), &selected))	{ psDeviceFlags.set(rsDrawSafeRect, selected); UI->RedrawScene(); }
	}
	{
		bool selected = psDeviceFlags.test(rsDrawGrid);
		if (XUI::MenuItem("Grid", Keys(COMMAND_TOGGLE_GRID), &selected))			{ psDeviceFlags.set(rsDrawGrid, selected); UI->RedrawScene(); }
	}
	ImGui::Separator();
	{
		bool selected = psDeviceFlags.test(rsFog);
		if (XUI::MenuItem("Fog", nullptr, &selected))				{ psDeviceFlags.set(rsFog, selected); UI->RedrawScene(); }
	}
	if (ImGui::BeginMenu("Environment"))
	{
		if (XUI::MenuItem("None", nullptr, !psDeviceFlags.test(rsEnvironment)))
		{
			psDeviceFlags.set(rsEnvironment, false);
			UI->RedrawScene();
		}
		ImGui::Separator();
		for (auto& i : g_pGamePersistent->Environment().WeatherCycles)
		{
			const bool selected = psDeviceFlags.test(rsEnvironment) && i.first == g_pGamePersistent->Environment().CurrentCycleName;
			if (XUI::MenuItem(i.first.c_str(), nullptr, selected))
			{
				psDeviceFlags.set(rsEnvironment, true);
				g_pGamePersistent->Environment().SetWeather(i.first.c_str(), true);
				UI->RedrawScene();
			}
		}
		ImGui::EndMenu();
	}
	ImGui::Separator();
	{
		bool selected = psDeviceFlags.test(rsLighting);
		if (XUI::MenuItem("Lighting", nullptr, &selected))		{ psDeviceFlags.set(rsLighting, selected); UI->RedrawScene(); }
	}
	// the scene lit the way the game lights it at an hour and weather of the linked game
	if (ImGui::BeginMenu("Game Lighting"))
	{
		GameLighting_Menu();
		ImGui::EndMenu();
	}
	{
		bool selected = psDeviceFlags.test(rsMuteSounds);
		if (XUI::MenuItem("Mute Sounds", nullptr, &selected))		psDeviceFlags.set(rsMuteSounds, selected);
	}
	{
		bool selected = psDeviceFlags.test(rsRenderRealTime);
		if (XUI::MenuItem("Real Time", nullptr, &selected))		psDeviceFlags.set(rsRenderRealTime, selected);
	}
	ImGui::Separator();
	{
		bool selected = psDeviceFlags.test(rsStatistic);
		if (XUI::MenuItem("Stats", nullptr, &selected))			{ psDeviceFlags.set(rsStatistic, selected); UI->RedrawScene(); }
	}
}

void UIMainMenuForm::Draw(bool menus, bool busy)
{
	if (!UITitleBar::Begin())
		return;
	UITitleBar::DrawLeft();
	if (menus)
	{
		ImGui::BeginDisabled(busy);
		// a narrow window folds the menus into one button, so the tabs keep their room
		s_Folded = ImGui::GetMainViewport()->Size.x < XUI::Pt(1100.f);
		if (s_Folded)
			DrawFolded();
		else
			DrawMenus();
		ImGui::EndDisabled();
		XUI::Shell::DrawTabStrip(UITitleBar::TabsMaxX(), busy);
	}
	UITitleBar::DrawRight(busy);
	UITitleBar::End();
}
