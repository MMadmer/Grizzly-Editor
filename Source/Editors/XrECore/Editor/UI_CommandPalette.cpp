#include "stdafx.h"
#pragma hdrstop

#include "UI_CommandPalette.h"
#include "UI_MainCommand.h"
#include "ui_main.h"
#include "../../Public/ShortcutText.h"
#include "EditorUiContract.h"

namespace
{
// the window's id is MCP's (screenshot_editor modal.id, ui_contract palette)
const xr_string& PopupId()
{
	static const xr_string s_Id = xr_string("Command Palette###") + EditorUiContract::kPaletteId;
	return s_Id;
}
constexpr int kNoMatch = 0x3fffffff;
// a match that is only every letter in order: never better than any real one
constexpr int kLetters = 160;

using Icon = XUI::Icon;
using Token = GrizzlyTheme::ColorToken;

struct SEntry
{
	CommandPalette::SResult result;
	// lower case, for the match: what the row shows
	xr_string label, path, shortcut;
};

struct SPaletteState
{
	bool open{};
	bool open_requested{};
	bool close_requested{};
	bool focus_search{};
	bool filter_dirty{};
	bool scroll_to_selected{};
	char query[256]{};
	xr_string filtered_query;
	xr_vector<CommandPalette::SResult> filtered;
	int selected{};
};

SPaletteState g_State;

ImU32 Col(Token token, float alpha = 1.f)
{
	return GrizzlyTheme::U32(token, alpha);
}

xr_string Lower(LPCSTR value)
{
	xr_string result = value ? value : "";
	for (char& c : result)
		if (c >= 'A' && c <= 'Z')
			c = char(c - 'A' + 'a');
	return result;
}

xr_vector<xr_string> Tokenize(LPCSTR query)
{
	xr_vector<xr_string> tokens;
	const xr_string lower = Lower(query);
	for (size_t begin = 0; begin < lower.size();)
	{
		while (begin < lower.size() && (lower[begin] == ' ' || lower[begin] == '\t'))
			++begin;
		size_t end = begin;
		while (end < lower.size() && lower[end] != ' ' && lower[end] != '\t')
			++end;
		if (end > begin)
			tokens.emplace_back(lower.substr(begin, end - begin));
		begin = end;
	}
	return tokens;
}

bool IsBreak(char c)
{
	return c == ' ' || c == '_' || c == '-' || c == '/' || c == '\\' || c == '>' || c == '.' || c == '(' || c == ':';
}

// Lower is better. The whole name, then its start, then the start of one of its words ("log" finds Output Log before
// Catalog), then anywhere in it, then every letter in order.
int ScoreIn(const xr_string& token, const xr_string& hay, bool letters)
{
	if (token.empty() || hay.empty())
		return kNoMatch;
	if (hay == token)
		return 0;
	if (!hay.compare(0, token.size(), token))
		return 4 + int(hay.size() - token.size()) / 8;
	for (size_t at = hay.find(token); at != xr_string::npos; at = hay.find(token, at + 1))
		if (IsBreak(hay[at - 1]))
		{
			int word = 0;
			for (size_t i = 0; i < at; ++i)
				word += IsBreak(hay[i]) && i + 1 < hay.size() && !IsBreak(hay[i + 1]);
			return 16 + word * 2;
		}
	const size_t sub = hay.find(token);
	if (sub != xr_string::npos)
		return 48 + int(sub);
	if (!letters)
		return kNoMatch;
	// the letters in order, each one starting a word or following the one before ("big" finds Build Into Game) - never
	// letters scattered through the middle of words, which read as no match at all ("log" in "buiLd intO Game")
	size_t matched = 0, first = xr_string::npos, previous = 0;
	int gaps = 0;
	for (size_t i = 0; i < hay.size() && matched < token.size(); ++i)
	{
		if (hay[i] != token[matched])
			continue;
		const bool follows = first != xr_string::npos && i == previous + 1;
		if (!follows && i && !IsBreak(hay[i - 1]))
			continue;
		if (first == xr_string::npos)
			first = i;
		else
			gaps += int(i - previous - 1);
		previous = i;
		++matched;
	}
	return matched == token.size() ? kLetters + int(first) + gaps * 3 : kNoMatch;
}

// every word of the query has to match what a row shows: the name counts most, the menu path less, the keys least. The
// COMMAND_* id is MCP's name and shows nowhere, so it matches nothing ("log" once found Submit to Mod Browser by
// COMMAND_MOD_CATALOG)
int ScoreEntry(const SEntry& entry, const xr_vector<xr_string>& tokens)
{
	int total = 0;
	for (const xr_string& token : tokens)
	{
		int best = ScoreIn(token, entry.label, true);
		best = _min(best, ScoreIn(token, entry.path, false) + 12);
		best = _min(best, ScoreIn(token, entry.shortcut, false) + 30);
		if (best >= kNoMatch)
			return kNoMatch;
		total += best;
	}
	return total;
}

// The command's picture: its own, else its menu's, else the one of a command. Labels and menus are the registry's
// (UI_MainCommand, UI_LevelMain) - a new command with a new menu gets the generic picture, never a placeholder.
Icon IconFor(const xr_string& path, const xr_string& label)
{
	const xr_string menu = path.substr(0, path.find(" > "));
	// the official names (CCustomPreferences::CommandName); a command it does not list keeps its registry name
	static const std::pair<const char*, Icon> kLabels[] =
	{
		{ "New Scene", Icon::New }, { "Open Scene", Icon::Open }, { "Open Last Scene", Icon::History }, { "Save", Icon::Save },
		{ "Save As", Icon::Save }, { "File Menu", Icon::Menu }, { "Quit", Icon::Close },
		{ "Undo", Icon::Undo }, { "Redo", Icon::Redo }, { "Cut", Icon::Cut }, { "Copy", Icon::Copy }, { "Paste", Icon::Paste },
		{ "Delete", Icon::Delete }, { "Find in World Outliner", Icon::Search }, { "Command Palette", Icon::Search },
		{ "Preferences", Icon::Settings }, { "Cancel Running Job", Icon::Stop },
		{ "Select All", Icon::Select }, { "Deselect All", Icon::Select }, { "Invert Selection", Icon::Select },
		{ "Hide Unselected", Icon::EyeOff }, { "Hide Selected", Icon::EyeOff }, { "Hide All", Icon::EyeOff },
		{ "Select", Icon::Select }, { "Place", Icon::Place }, { "Move", Icon::Move }, { "Rotate", Icon::Rotate },
		{ "Scale", Icon::Scale }, { "Next Transform Tool", Icon::Refresh }, { "Stop Placing", Icon::Select },
		{ "Frame All", Icon::FrameAll }, { "Frame Selection", Icon::FrameSelection }, { "Move Camera To", Icon::Perspective },
		{ "Wireframe", Icon::Wireframe }, { "Show Grid", Icon::SnapGrid }, { "Smaller Grid Cells", Icon::Minus },
		{ "Larger Grid Cells", Icon::Plus }, { "Show Safe Rect", Icon::Aspect }, { "Show AI Map", Icon::AIMap },
		{ "Drop to Ground", Icon::MoveDown }, { "Simulate Physics", Icon::Play }, { "Apply Simulated Positions", Icon::Place },
		{ "Make All", Icon::BuildAll }, { "Make Game", Icon::BuildGameGraph }, { "Make AI Map", Icon::BuildAIMap },
		{ "Make Details", Icon::DetailObjects }, { "Make HOM", Icon::Sector }, { "Make SOM", Icon::SoundEnv },
		{ "Build Into Game", Icon::BuildMod }, { "Mod Settings", Icon::Settings }, { "Characters", Icon::Character },
		{ "World Rules", Icon::World }, { "PDA Interface", Icon::PdaInterface }, { "Package Release", Icon::Package },
		{ "Publish Release", Icon::Publish }, { "Releases", Icon::Releases }, { "Submit to Mod Browser", Icon::Globe },
		{ "Details", Icon::Info }, { "Details (Scene Options)", Icon::Settings }, { "World Outliner", Icon::SidebarRight },
		{ "Content Browser", Icon::Folder },
		// Tools Panel: the scene's object classes
		{ "Object Tool", Icon::SceneObject }, { "Light Tool", Icon::Light }, { "Sound Source Tool", Icon::SoundSource },
		{ "Sound Environment Tool", Icon::SoundEnv }, { "Glow Tool", Icon::Glow }, { "Shape Tool", Icon::Shape },
		{ "Spawn Tool", Icon::Spawn }, { "Way Tool", Icon::Way }, { "Way Tool: Points", Icon::Way },
		{ "Way Tool: Switch Ways / Points", Icon::Way }, { "Sector Tool", Icon::Sector }, { "Portal Tool", Icon::Portal },
		{ "Group Tool", Icon::Group }, { "Particles Tool", Icon::Particles }, { "Detail Objects Tool", Icon::DetailObjects },
		{ "AI Map Tool", Icon::AIMap }, { "Wallmark Tool", Icon::Wallmark }, { "Fog Volume Tool", Icon::FogVolume },
	};
	for (const auto& [name, icon] : kLabels)
		if (label == name)
			return icon;
	static const std::pair<const char*, Icon> kMenus[] =
	{
		{ "File", Icon::File }, { "Edit", Icon::Rename }, { "Selection", Icon::Select }, { "Visibility", Icon::Eye },
		{ "Viewport Tools", Icon::Select }, { "View", Icon::Perspective }, { "Tools Panel", Icon::SidebarLeft },
		{ "Object", Icon::SceneObject }, { "Make", Icon::BuildAll }, { "Mod", Icon::BuildMod }, { "Window", Icon::SidebarRight },
		{ "Macros", Icon::Terminal }, { "Scene", Icon::Level }, { "Particles", Icon::Particles },
	};
	for (const auto& [name, icon] : kMenus)
		if (menu == name)
			return icon;
	return Icon::Terminal;
}

void Add(xr_vector<SEntry>& catalog, CommandPalette::SResult&& result)
{
	SEntry& entry = catalog.emplace_back();
	entry.result = std::move(result);
	entry.label = Lower(entry.result.label.c_str());
	entry.path = Lower(entry.result.path.c_str());
	entry.shortcut = Lower(entry.result.shortcut.c_str());
}

void BuildCatalog(xr_vector<SEntry>& catalog)
{
	catalog.clear();
	// ---- the open tabs, then the Level tab's panels and the drawers: they are opened by name as often as commands
	xr_vector<XUI::Shell::DocInfo> tabs;
	XUI::Shell::List(tabs);
	for (const XUI::Shell::DocInfo& tab : tabs)
	{
		CommandPalette::SResult r;
		r.kind = "tab";
		r.id = "tab:" + tab.id;
		r.name = r.id;
		r.label = tab.label;
		r.category = "Tabs";
		r.path = tab.path.empty() ? xr_string("Tab") : "Tab > " + tab.path;
		r.icon = int(tab.icon != Icon::None ? tab.icon : Icon::File);
		Add(catalog, std::move(r));
	}
	XUI::Strings surfaces;
	XUI::Surfaces(surfaces);
	for (const xr_string& id : surfaces)
	{
		xr_string window;
		int kind = -1;
		if (!XUI::SurfaceInfo(id.c_str(), &window, &kind) || (kind != XUI::SurfacePanel && kind != XUI::SurfaceDrawer))
			continue;
		CommandPalette::SResult r;
		const bool drawer = kind == XUI::SurfaceDrawer;
		r.kind = drawer ? "drawer" : "panel";
		r.id = r.kind + ":" + id;
		r.name = r.id;
		// the window's name up to its ###id is what its tab says
		r.label = window.substr(0, window.find("###"));
		if (drawer)
			r.label = id == "content" ? "Content Drawer" : id == "log" ? "Log Drawer" : r.label;
		r.category = drawer ? "Drawers" : "Panels";
		r.path = drawer ? "Window > Drawer" : "Window > Panel";
		r.shortcut = drawer ? (id == "content" ? "Ctrl+Space" : id == "log" ? "Ctrl+`" : "") : "";
		static const std::pair<const char*, Icon> kPanels[] =
		{
			{ "tools", Icon::SidebarLeft }, { "world_outliner", Icon::SidebarRight }, { "details", Icon::Info },
			{ "level_settings", Icon::Settings }, { "content_browser", Icon::Folder }, { "output_log", Icon::Log },
			{ "viewport", Icon::Perspective }, { "content", Icon::Drawer }, { "log", Icon::Log },
		};
		r.icon = int(drawer ? Icon::Drawer : Icon::SidebarRight);
		for (const auto& [panel, icon] : kPanels)
			if (id == panel)
				r.icon = int(icon);
		Add(catalog, std::move(r));
	}

	// ---- the command registry: every preset of every command that has a menu path
	const ECommandVec& commands = GetEditorCommands();
	for (u32 command_index = 0; command_index < commands.size(); ++command_index)
	{
		SECommand* command = commands[command_index];
		if (!command || !command->Desc()[0] || command->command.empty())
			continue;
		for (u32 sub = 0; sub < command->sub_commands.size(); ++sub)
		{
			SESubCommand* subcommand = command->sub_commands[sub];
			if (!subcommand)
				continue;
			CommandPalette::SResult r;
			r.kind = "command";
			r.command = command->idx;
			r.subcommand = sub;
			r.name = command->Name();
			xr_string group, label;
			CCustomPreferences::CommandName(subcommand, group, label);
			r.category = group;
			r.label = label;
			r.path = group + " > " + label;
			r.shortcut = ShortcutText(subcommand->shortcut);
			r.id = command->Name();
			if (!subcommand->desc.empty())
				r.id += ".\"" + subcommand->desc + "\"";
			r.icon = int(IconFor(r.path, r.label));
			Add(catalog, std::move(r));
		}
	}
}

// the order of an empty query: the tabs, the panels, the drawers, then the commands by their menus
int KindRank(const xr_string& kind)
{
	return kind == "tab" ? 0 : kind == "panel" ? 1 : kind == "drawer" ? 2 : 3;
}

void RefreshFiltered()
{
	CommandPalette::Query(g_State.query, g_State.filtered, 250);
	g_State.filtered_query = g_State.query;
	g_State.selected = g_State.filtered.empty() ? 0 : std::clamp(g_State.selected, 0, int(g_State.filtered.size()) - 1);
}

// the part of the menu path before the command's own name: "Scene > Build" of "Scene > Build > Make All"
xr_string Parent(const CommandPalette::SResult& r)
{
	if (r.kind != "command")
		return r.path;
	const size_t at = r.path.rfind(" > ");
	return at == xr_string::npos ? xr_string() : r.path.substr(0, at);
}

// one row: the icon, the name, where it lives (cut at its start: the end says most), the keys at the right
bool Row(const CommandPalette::SResult& r, int index, bool selected, float row_h)
{
	ImGui::PushID(index);
	const bool pressed = ImGui::Selectable("##row", selected, 0, ImVec2(0.f, row_h));
	const ImVec2 a = ImGui::GetItemRectMin(), b = ImGui::GetItemRectMax();
	const bool hovered = ImGui::IsItemHovered();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	const float cy = (a.y + b.y) * .5f;
	const float icon = XUI::Pt(16.f);
	XUI::DrawIcon(dl, XUI::Icon(r.icon), ImVec2(floorf(a.x + XUI::Pt(10.f) + .5f), floorf(cy - icon * .5f + .5f)), 16.f,
		Col(selected ? Token::AccentBright : Token::TextSecondary));

	const float keys_h = XUI::FontSize(XUI::Font::Small) * XUI::Dpi() + XUI::Pt(4.f);
	const float keys_w = XUI::Keycaps(nullptr, a, r.shortcut.c_str(), keys_h);
	const float right = b.x - XUI::Pt(10.f) - (keys_w > 0.f ? keys_w + XUI::Pt(12.f) : 0.f);
	const float name_x = a.x + XUI::Pt(36.f);
	const float body_px = XUI::FontSize(XUI::Font::Body) * XUI::Dpi();
	const float small_px = XUI::FontSize(XUI::Font::Small) * XUI::Dpi();
	const xr_string parent = Parent(r);
	// the name keeps what it needs; where it lives takes what is left, at least 8 em of it
	const float room = _max(0.f, right - name_x);
	const float name_w = _min(XUI::TextWidth(XUI::Font::Body, r.label.c_str()), parent.empty() ? room : _max(room * .6f, room - XUI::Em(8.f)));
	XUI::TextAt(dl, XUI::Font::Body, ImVec2(name_x, floorf(cy - body_px * .5f + .5f)), Col(Token::Text), r.label.c_str(), name_w);
	const float path_x = name_x + name_w + XUI::Pt(12.f);
	if (!parent.empty() && right - path_x > XUI::Pt(24.f))
		XUI::TextAt(dl, XUI::Font::Small, ImVec2(path_x, floorf(cy - small_px * .5f + .5f)), Col(Token::TextMuted), parent.c_str(),
			right - path_x, XUI::Clip::Start);
	if (keys_w > 0.f)
		XUI::Keycaps(dl, ImVec2(floorf(b.x - XUI::Pt(10.f) - keys_w + .5f), floorf(cy - keys_h * .5f + .5f)), r.shortcut.c_str(), keys_h);

	// what the row runs, by the name MCP knows it by
	if (hovered)
	{
		xr_string tip = r.path;
		tip += "\n";
		tip += r.id;
		XUI::Tip(tip.c_str());
	}
	ImGui::PopID();
	return pressed;
}
}

namespace CommandPalette
{
void Open(LPCSTR query)
{
	xr_strcpy(g_State.query, sizeof(g_State.query), query ? query : "");
	g_State.selected = 0;
	g_State.close_requested = false;
	g_State.open_requested = true;
	g_State.focus_search = true;
	g_State.filter_dirty = true;
}

void Close()
{
	g_State.open_requested = false;
	g_State.close_requested = true;
}

bool IsOpen()
{
	return !g_State.close_requested && (g_State.open || g_State.open_requested);
}

void Query(LPCSTR query, xr_vector<SResult>& results, u32 limit)
{
	xr_vector<SEntry> catalog;
	BuildCatalog(catalog);
	const xr_vector<xr_string> tokens = Tokenize(query);

	results.clear();
	results.reserve(std::min<size_t>(catalog.size(), limit));
	for (SEntry& entry : catalog)
	{
		entry.result.score = tokens.empty() ? KindRank(entry.result.kind) : ScoreEntry(entry, tokens);
		if (entry.result.score < kNoMatch)
			results.emplace_back(std::move(entry.result));
	}

	const bool ranked = !tokens.empty();
	std::stable_sort(results.begin(), results.end(), [ranked](const SResult& left, const SResult& right)
	{
		if (left.score != right.score)
			return left.score < right.score;
		// of two equal matches, the shorter name is the closer one; with no query it is the menus' order
		if (ranked && left.label.size() != right.label.size())
			return left.label.size() < right.label.size();
		const int path_order = stricmp(left.path.c_str(), right.path.c_str());
		if (path_order)
			return path_order < 0;
		return stricmp(left.id.c_str(), right.id.c_str()) < 0;
	});
	if (results.size() > limit)
		results.resize(limit);
}

bool Execute(u32 command_index, u32 subcommand_index)
{
	ECommandVec& commands = GetEditorCommands();
	if (command_index >= commands.size())
		return false;
	SECommand* command = commands[command_index];
	if (!command || command->command.empty() || subcommand_index >= command->sub_commands.size())
		return false;
	SESubCommand* subcommand = command->sub_commands[subcommand_index];
	if (!subcommand)
		return false;
	const bool executed = ExecCommand(command->idx, subcommand->p0, subcommand->p1);
	if (command->idx == COMMAND_EXIT && executed)
		ExecCommand(COMMAND_QUIT);
	return executed;
}

bool Execute(LPCSTR id)
{
	if (!id || !id[0])
		return false;
	// a panel comes up on Level (or in its drawer), a drawer over the tab, a tab in front
	if (!strncmp(id, "panel:", 6))
		return XUI::SurfaceControl(id + 6, XUI::SurfaceFocus);
	if (!strncmp(id, "drawer:", 7))
		return XUI::SurfaceControl(id + 7, XUI::SurfaceOpen);
	if (!strncmp(id, "tab:", 4))
		return XUI::Shell::Activate(id + 4);
	xr_vector<SEntry> catalog;
	BuildCatalog(catalog);
	const auto found = std::find_if(catalog.begin(), catalog.end(), [id](const SEntry& entry)
	{
		return entry.result.kind == "command" && 0 == stricmp(entry.result.id.c_str(), id);
	});
	return found != catalog.end() && Execute(found->result.command, found->result.subcommand);
}

void Draw()
{
	if (g_State.close_requested && !g_State.open)
	{
		g_State.close_requested = false;
		return;
	}
	if (g_State.open_requested)
	{
		g_State.open_requested = false;
		g_State.open = true;
		g_State.filter_dirty = true;
		ImGui::OpenPopup(PopupId().c_str());
	}
	if (!g_State.open)
		return;

	// Top-centred, min(720 pt, 70 % of the window) wide. The height is what the results take, up to 480 pt: the field
	// never moves, the list ends under its last row. The rows are counted before the frame, from last frame's query.
	const ImGuiViewport* viewport = ImGui::GetMainViewport();
	const float width = _min(XUI::Pt(720.f), viewport->WorkSize.x * .7f);
	const float top = XUI::Pt(40.f);
	const float margin = XUI::Pt(12.f);
	const float footer_h = XUI::Pt(32.f);
	const float row_h = XUI::Pt(30.f);
	if (g_State.filter_dirty || g_State.filtered_query != g_State.query)
	{
		if (g_State.filtered_query != g_State.query)
			g_State.selected = 0;
		RefreshFiltered();
		g_State.filter_dirty = false;
		g_State.scroll_to_selected = true;
	}
	// the field (its text and 8 pt above and below it), the gap under it; the list's own 6 pt padding at both ends and
	// 2 pt the rounding of the rows to px may take (a list that fits must not get a scrollbar)
	const float field_h = XUI::FontSize(XUI::Font::Body) * XUI::Dpi() + XUI::Pt(16.f);
	const float head_h = margin + field_h + XUI::Pt(8.f);
	const float list_h = g_State.filtered.empty() ? XUI::Pt(168.f) : float(g_State.filtered.size()) * row_h + XUI::Pt(14.f);
	const float height = _max(head_h + row_h + XUI::Pt(12.f) + footer_h,
		_min(head_h + list_h + footer_h, _min(XUI::Pt(480.f), viewport->WorkSize.y - top - XUI::Pt(24.f))));
	ImGui::SetNextWindowPos(ImVec2(floorf(viewport->WorkPos.x + (viewport->WorkSize.x - width) * .5f + .5f), viewport->WorkPos.y + top), ImGuiCond_Always);
	ImGui::SetNextWindowSize(ImVec2(width, height), ImGuiCond_Always);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, XUI::Pt(8.f));
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Overlay));
	ImGui::PushStyleColor(ImGuiCol_Border, Col(Token::BorderStrong));
	bool keep_open = true;
	const bool visible = ImGui::BeginPopupModal(PopupId().c_str(), &keep_open, ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoResize |
		ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	ImGui::PopStyleColor(2);
	ImGui::PopStyleVar(2);
	if (!visible)
	{
		if (!ImGui::IsPopupOpen(PopupId().c_str()))
			g_State.open = false;
		return;
	}

	// ---- the query: a magnifier and the field, the keyboard's the whole time
	const float icon = XUI::Pt(16.f);
	ImGui::SetCursorPos(ImVec2(margin, margin));
	if (g_State.focus_search)
	{
		ImGui::SetKeyboardFocusHere();
		g_State.focus_search = false;
	}
	ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(10.f) + icon + XUI::Pt(8.f), XUI::Pt(8.f)));
	ImGui::SetNextItemWidth(width - margin * 2.f);
	// the field owns Up and Down (a history callback that does nothing): they move the selection, never the focus
	const bool submitted = ImGui::InputTextWithHint("##command_palette_query", "Run a command, open a panel or go to a tab",
		g_State.query, sizeof(g_State.query), ImGuiInputTextFlags_EnterReturnsTrue | ImGuiInputTextFlags_AutoSelectAll |
		ImGuiInputTextFlags_CallbackHistory, [](ImGuiInputTextCallbackData*) { return 0; });
	ImGui::PopStyleVar();
	{
		const ImVec2 a = ImGui::GetItemRectMin(), b = ImGui::GetItemRectMax();
		XUI::DrawIcon(ImGui::GetWindowDrawList(), Icon::Search, ImVec2(floorf(a.x + XUI::Pt(10.f) + .5f), floorf((a.y + b.y - icon) * .5f + .5f)), 16.f,
			Col(Token::TextSecondary));
	}
	if (g_State.filter_dirty || g_State.filtered_query != g_State.query)
	{
		if (g_State.filtered_query != g_State.query)
			g_State.selected = 0;
		RefreshFiltered();
		g_State.filter_dirty = false;
		g_State.scroll_to_selected = true;
	}
	const int count = int(g_State.filtered.size());
	if (count)
	{
		int move = 0;
		if (ImGui::IsKeyPressed(ImGuiKey_DownArrow))	move = 1;
		if (ImGui::IsKeyPressed(ImGuiKey_UpArrow))		move = -1;
		if (ImGui::IsKeyPressed(ImGuiKey_PageDown))		move = 10;
		if (ImGui::IsKeyPressed(ImGuiKey_PageUp))		move = -10;
		if (move)
		{
			g_State.selected = std::abs(move) == 1 ? (g_State.selected + move + count) % count : std::clamp(g_State.selected + move, 0, count - 1);
			g_State.scroll_to_selected = true;
		}
	}
	ImGui::SetCursorPosY(ImGui::GetCursorPosY() + XUI::Pt(8.f) - ImGui::GetStyle().ItemSpacing.y);
	{
		const ImVec2 at = ImGui::GetCursorScreenPos();
		ImGui::GetWindowDrawList()->AddLine(ImVec2(at.x, at.y), ImVec2(at.x + width, at.y), Col(Token::BorderSubtle));
	}

	// ---- the results, drawn where they are on screen
	int execute_index = submitted && count ? g_State.selected : -1;
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(6.f), XUI::Pt(6.f)));
	ImGui::PushStyleColor(ImGuiCol_ChildBg, 0);
	if (ImGui::BeginChild("##command_palette_results", ImVec2(0.f, -footer_h), ImGuiChildFlags_AlwaysUseWindowPadding))
	{
		if (!count)
			XUI::EmptyState(Icon::Search, "No matches", "No command, panel or tab has these words in its name.");
		else
		{
			ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(0.f, 0.f));
			ImGuiListClipper clipper;
			clipper.Begin(count, row_h);
			if (g_State.scroll_to_selected)
				clipper.IncludeItemByIndex(g_State.selected);
			while (clipper.Step())
				for (int index = clipper.DisplayStart; index < clipper.DisplayEnd; ++index)
				{
					if (Row(g_State.filtered[index], index, index == g_State.selected, row_h))
					{
						g_State.selected = index;
						execute_index = index;
					}
					if (index == g_State.selected && g_State.scroll_to_selected)
					{
						// keep the selected row in view, nudged no further than needed
						const float y0 = ImGui::GetItemRectMin().y, y1 = ImGui::GetItemRectMax().y;
						const ImVec2 wmin = ImGui::GetWindowPos();
						const float view0 = wmin.y, view1 = wmin.y + ImGui::GetWindowHeight();
						if (y0 < view0)
							ImGui::SetScrollHereY(0.f);
						else if (y1 > view1)
							ImGui::SetScrollHereY(1.f);
						g_State.scroll_to_selected = false;
					}
				}
			ImGui::PopStyleVar();
		}
	}
	ImGui::EndChild();
	ImGui::PopStyleColor();
	ImGui::PopStyleVar();

	// ---- the footer: how many, and the keys
	{
		ImDrawList* dl = ImGui::GetWindowDrawList();
		const ImVec2 wpos = ImGui::GetWindowPos();
		const float y0 = wpos.y + height - footer_h;
		dl->AddLine(ImVec2(wpos.x, y0), ImVec2(wpos.x + width, y0), Col(Token::BorderSubtle));
		const float small_px = XUI::FontSize(XUI::Font::Small) * XUI::Dpi();
		const float cy = y0 + footer_h * .5f;
		char found[48];
		sprintf_s(found, count == 1 ? "1 result" : "%d results", count);
		XUI::TextAt(dl, XUI::Font::Small, ImVec2(wpos.x + margin, floorf(cy - small_px * .5f + .5f)), Col(Token::TextMuted), found);
		// right to left: Esc Close, Enter Run
		const float keys_h = small_px + XUI::Pt(2.f);
		float x = wpos.x + width - margin;
		const std::pair<const char*, const char*> hints[] = { { "Esc", "Close" }, { "Enter", "Run" } };
		for (const auto& [keys, what] : hints)
		{
			x -= XUI::TextWidth(XUI::Font::Small, what);
			XUI::TextAt(dl, XUI::Font::Small, ImVec2(x, floorf(cy - small_px * .5f + .5f)), Col(Token::TextMuted), what);
			const float kw = XUI::Keycaps(nullptr, ImVec2(0.f, 0.f), keys, keys_h);
			x -= XUI::Pt(6.f) + kw;
			XUI::Keycaps(dl, ImVec2(x, floorf(cy - keys_h * .5f + .5f)), keys, keys_h);
			x -= XUI::Pt(16.f);
		}
	}

	const bool cancel = g_State.close_requested || ImGui::IsKeyPressed(ImGuiKey_Escape, false);
	if (cancel || !keep_open || execute_index >= 0)
	{
		ImGui::CloseCurrentPopup();
		g_State.open = false;
		g_State.close_requested = false;
	}
	ImGui::EndPopup();

	if (!cancel && keep_open && execute_index >= 0)
	{
		const SResult selected = g_State.filtered[execute_index];
		if (selected.kind == "command")
			Execute(selected.command, selected.subcommand);
		else
			Execute(selected.id.c_str());
	}
}
}
