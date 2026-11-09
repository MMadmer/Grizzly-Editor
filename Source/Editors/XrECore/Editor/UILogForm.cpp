#include "stdafx.h"
#include "ELog.h"
#include "UILogForm.h"
#include "..\XrCore\os_clipboard.h"
#include "..\XrEngine\XR_IOConsole.h"

extern bool bAllowLogCommands;

namespace
{
enum ELevel : u8 { lvPlain = 0, lvInfo, lvWarning, lvError, lvCount };

struct SLine
{
	xr_string text;
	ELevel level = lvPlain;
	u32 epoch = 0;				// the counting window it was written in (CountFromHere starts the next one)
	bool mark = false;			// the row where a counting window starts
	bool counted = false;		// one of the status bar's errors or warnings: the first of its text in its window
};

// written by any thread, under the guard; the UI thread moves the lines out when it draws
xr_vector<SLine>* s_List = nullptr;
xrCriticalSection s_Guard;
int s_Errors = 0;
int s_Warnings = 0;
xr_string s_Last;
u32 s_Epoch = 0;
// the texts counted in this window: a line that comes again is one thing to act on, not another
xr_set<u32> s_Counted;

// the UI thread's lines, the ones the filter shows, and the view
xr_vector<SLine> s_Lines;
xr_vector<u32> s_Visible;
u32 s_Filtered = 0;				// lines the filter has looked at
u32 s_Marks = 0;				// rows of s_Lines that are counting marks, not lines of the log
u32 s_ViewEpoch = 0;			// the window Warnings and Errors list
bool s_Refilter = false;
char s_Search[256] = "";
xr_string s_ShownSearch;
int s_Level = 0;				// 0 all, 1 warnings, 2 errors
int s_ShownLevel = 0;
bool s_AutoScroll = true;
bool s_ScrollDown = true;		// the next frame shows the end
bool s_ScrollDownAgain = false;	// and the one after it: the first request stops at the size the view had the frame before
bool s_Grew = false;
u32 s_MenuLine = u32(-1);
char s_Command[512] = "";
xr_vector<xr_string> s_History;
int s_HistoryAt = -1;

xr_vector<SLine>* GetList()
{
	if (!s_List)
		s_List = xr_new<xr_vector<SLine>>();
	return s_List;
}

ELevel LevelOf(const char* line)
{
	if (!strncmp(line, "! ", 2))
		return lvError;
	if (!strncmp(line, "~ ", 2))
		return lvWarning;
	if (!strncmp(line, "* ", 2))
		return lvInfo;
	return lvPlain;
}

// "! [LUA]  2 : [Lua] ...script(32) : ..." - a frame of the traceback under a script error, not an error of its own
bool IsTraceFrame(const char* line)
{
	if (strncmp(line, "! [LUA]", 7))
		return false;
	const char* p = line + 7;
	while (*p == ' ')
		++p;
	if (*p < '0' || *p > '9')
		return false;
	while (*p >= '0' && *p <= '9')
		++p;
	return !strncmp(p, " :", 2);
}

ImU32 ColorOf(ELevel level)
{
	switch (level)
	{
	case lvError:	return XUI::ToneColor(XUI::Tone::Error);
	case lvWarning:	return XUI::ToneColor(XUI::Tone::Warning);
	case lvInfo:	return GrizzlyTheme::U32(GrizzlyTheme::ColorToken::TextSecondary);
	default:		return GrizzlyTheme::U32(GrizzlyTheme::ColorToken::Text);
	}
}

bool Shows(const SLine& line)
{
	// a mark belongs to the whole, unfiltered log
	if (line.mark)
		return !s_Level && !s_Search[0];
	// Warnings and Errors list what the status bar counts: the lines since the last CountFromHere, each text once
	if (s_Level && (line.epoch != s_ViewEpoch || !line.counted))
		return false;
	if (s_Level == 1 && line.level != lvWarning)
		return false;
	if (s_Level == 2 && line.level != lvError)
		return false;
	return XUI::Match(s_Search, line.text.c_str());
}

// the lines written since the last frame come over
void Take()
{
	xrCriticalSection::raii guard(&s_Guard);
	if (s_ViewEpoch != s_Epoch)
	{
		s_ViewEpoch = s_Epoch;
		s_Refilter = true;
	}
	xr_vector<SLine>* list = GetList();
	s_Grew = !list->empty();
	for (SLine& line : *list)
	{
		s_Marks += line.mark;
		s_Lines.push_back(std::move(line));
	}
	list->clear();
}

// the new lines through the filter; a changed filter looks at every line again
void Refilter()
{
	if (s_Refilter || s_ShownSearch != s_Search || s_ShownLevel != s_Level)
	{
		s_Refilter = false;
		s_ShownSearch = s_Search;
		s_ShownLevel = s_Level;
		s_Visible.clear();
		s_Filtered = 0;
		s_ScrollDown = true;
	}
	for (; s_Filtered < s_Lines.size(); ++s_Filtered)
		if (Shows(s_Lines[s_Filtered]))
			s_Visible.push_back(s_Filtered);
}

void Clear()
{
	{
		xrCriticalSection::raii guard(&s_Guard);
		GetList()->clear();
		s_Errors = s_Warnings = 0;
		s_Counted.clear();
		s_Last.clear();
	}
	s_Lines.clear();
	s_Visible.clear();
	s_Filtered = 0;
	s_Marks = 0;
	s_MenuLine = u32(-1);
}

void CopyShown()
{
	xr_string text;
	for (u32 index : s_Visible)
		if (!s_Lines[index].mark)
			text.append(s_Lines[index].text).append("\r\n");
	os_clipboard::copy_to_clipboard(text.c_str());
}

// ---- the toolbar: search, the level, what the view does ----------------------------------------------------------
void DrawToolbar()
{
	XUI::Toolbar tb("output_log", XUI::ToolbarPanel);
	tb.Group(2);
	tb.Search(s_Search, sizeof(s_Search), "Search the log", 10.f);

	tb.Group(1);
	// Warnings and Errors are the status bar's own counts, and list the lines those count
	xr_string last;
	int error_count = 0, warning_count = 0;
	UILogForm::State(last, error_count, warning_count);
	char all[32], warnings[32], errors[32];
	sprintf_s(all, "All %u", u32(s_Lines.size() - s_Marks));
	sprintf_s(warnings, "Warnings %d", warning_count);
	sprintf_s(errors, "Errors %d", error_count);
	const char* labels[] = { all, warnings, errors };
	tb.Segmented("level", "Show", &s_Level, labels, 3,
		"Which lines the log shows: every line, or the warnings or errors since the project opened, each once - the ones the status bar counts");

	tb.Spring();
	tb.Group(0);
	if (tb.Action("autoscroll", XUI::Icon::MoveDown, "Auto-scroll", "Follow the new lines as they come", true, s_AutoScroll))
	{
		s_AutoScroll = !s_AutoScroll;
		s_ScrollDown |= s_AutoScroll;
	}
	if (tb.Action("copy", XUI::Icon::Copy, "Copy", s_Visible.empty() ? "No lines are shown" : "Copy the lines shown to the clipboard",
		!s_Visible.empty()))
		CopyShown();
	if (tb.Action("clear", XUI::Icon::Clear, "Clear", s_Lines.empty() ? "The log is empty" : "Clear the lines shown here; the log file keeps them",
		!s_Lines.empty()))
		Clear();
}

// ---- the lines: Mono, and only those on screen are drawn -----------------------------------------------------------
void DrawLines()
{
	const float console_h = ImGui::GetFrameHeight() + ImGui::GetStyle().ItemSpacing.y;
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::Inset));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(8.f), XUI::Pt(4.f)));
	const bool shown = ImGui::BeginChild("##lines", ImVec2(0.f, -console_h), ImGuiChildFlags_AlwaysUseWindowPadding, ImGuiWindowFlags_HorizontalScrollbar);
	ImGui::PopStyleVar();
	ImGui::PopStyleColor();
	if (shown)
	{
		// a view that comes up - the drawer opened, the panel's tab picked - shows the end
		if (ImGui::IsWindowAppearing())
			s_ScrollDown = true;
		if (s_Lines.empty())
			XUI::EmptyState(XUI::Icon::Log, "The log is empty");
		else if (s_Visible.empty())
		{
			if (XUI::EmptyState(XUI::Icon::Search, "No lines match", "Nothing in the log matches the search and the level.", "Show All"))
			{
				s_Search[0] = 0;
				s_Level = 0;
			}
		}
		else
		{
			const bool at_end = ImGui::GetScrollY() >= ImGui::GetScrollMaxY() - 1.f;
			XUI::FontScope mono(XUI::Font::Mono);
			ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(XUI::Pt(8.f), XUI::Pt(1.f)));
			ImGuiListClipper clipper;
			clipper.Begin(int(s_Visible.size()));
			while (clipper.Step())
				for (int i = clipper.DisplayStart; i < clipper.DisplayEnd; ++i)
				{
					const SLine& line = s_Lines[s_Visible[i]];
					if (line.mark)
					{
						// where the counts start: its reason, then a rule to the edge, in a line's height
						ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(GrizzlyTheme::ColorToken::TextMuted));
						ImGui::TextUnformatted(line.text.c_str(), line.text.c_str() + line.text.size());
						ImGui::PopStyleColor();
						const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
						const float y = floorf((min.y + max.y) * .5f + .5f);
						ImGui::GetWindowDrawList()->AddLine(ImVec2(max.x + XUI::Pt(8.f), y),
							ImVec2(ImGui::GetWindowPos().x + ImGui::GetWindowWidth(), y), GrizzlyTheme::U32(GrizzlyTheme::ColorToken::BorderSubtle));
						continue;
					}
					ImGui::PushStyleColor(ImGuiCol_Text, ColorOf(line.level));
					ImGui::TextUnformatted(line.text.c_str(), line.text.c_str() + line.text.size());
					ImGui::PopStyleColor();
					if (ImGui::IsItemHovered() && ImGui::IsMouseReleased(ImGuiMouseButton_Right))
					{
						s_MenuLine = s_Visible[i];
						ImGui::OpenPopup("##line_menu");
					}
				}
			ImGui::PopStyleVar();
			// the end follows new lines only while it is in view: reading further up is not interrupted
			if (s_ScrollDown || s_ScrollDownAgain || (s_AutoScroll && s_Grew && at_end))
				ImGui::SetScrollHereY(1.f);
			s_ScrollDownAgain = s_ScrollDown;
			s_ScrollDown = false;
		}
		if (ImGui::BeginPopup("##line_menu"))
		{
			if (XUI::MenuItem("Copy Line", nullptr, false, s_MenuLine < s_Lines.size()))
				os_clipboard::copy_to_clipboard(s_Lines[s_MenuLine].text.c_str());
			if (XUI::MenuItem("Copy Lines Shown", nullptr, false, !s_Visible.empty()))
				CopyShown();
			ImGui::EndPopup();
		}
	}
	ImGui::EndChild();
}

// ---- the console: a command of the engine's console, Up and Down recall the earlier ones ------------------------------
int ConsoleHistory(ImGuiInputTextCallbackData* data)
{
	if (data->EventFlag != ImGuiInputTextFlags_CallbackHistory || s_History.empty())
		return 0;
	const int last = int(s_History.size()) - 1;
	if (data->EventKey == ImGuiKey_UpArrow)
		s_HistoryAt = s_HistoryAt < 0 ? last : _max(0, s_HistoryAt - 1);
	else if (data->EventKey == ImGuiKey_DownArrow)
		s_HistoryAt = s_HistoryAt < 0 || s_HistoryAt >= last ? -1 : s_HistoryAt + 1;
	data->DeleteChars(0, data->BufTextLen);
	if (s_HistoryAt >= 0)
		data->InsertChars(0, s_History[s_HistoryAt].c_str());
	return 0;
}

void DrawConsole()
{
	ImGui::PushID("##console");
	const float icon = XUI::Pt(14.f);
	const ImVec2 at = ImGui::GetCursorScreenPos();
	bool run = false;
	{
		XUI::FontScope mono(XUI::Font::Mono);
		ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(8.f) + icon + XUI::Pt(6.f), ImGui::GetStyle().FramePadding.y));
		// the row's width: xrCore's FLT_MIN is -FLT_MAX, so ImGui's -FLT_MIN idiom does not hold here
		ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
		run = ImGui::InputTextWithHint("##command", "Console command", s_Command, sizeof(s_Command),
			ImGuiInputTextFlags_EnterReturnsTrue | ImGuiInputTextFlags_CallbackHistory, &ConsoleHistory);
		ImGui::PopStyleVar();
	}
	const float h = ImGui::GetItemRectSize().y;
	XUI::DrawIcon(ImGui::GetWindowDrawList(), XUI::Icon::Terminal, ImVec2(floorf(at.x + XUI::Pt(8.f) + .5f), floorf(at.y + (h - icon) * .5f + .5f)), 14.f,
		GrizzlyTheme::U32(ImGui::IsItemActive() ? GrizzlyTheme::ColorToken::Text : GrizzlyTheme::ColorToken::TextMuted));
	XUI::Tip("A command of the engine's console, as the game's console takes it.", "Up / Down");
	if (run && s_Command[0])
	{
		Msg("* > %s", s_Command);
		if (Console)
			Console->Execute(s_Command);
		if (s_History.empty() || s_History.back() != s_Command)
			s_History.push_back(s_Command);
		s_HistoryAt = -1;
		s_Command[0] = 0;
		s_ScrollDown = true;
		// the field keeps the focus for the next command
		ImGui::SetKeyboardFocusHere(-1);
	}
	ImGui::PopID();
}
}

void UILogForm::AddMessage( const xr_string& msg, Level level, bool own)
{
	SLine line;
	line.text.reserve(msg.size());
	for (const char ch : msg)
	{
		if (ch == '\r')
			continue;
		line.text += ch == '\n' ? ' ' : ch;
	}
	switch (level)
	{
	case Level::Info:		line.level = lvInfo;		break;
	case Level::Warning:	line.level = lvWarning;		break;
	case Level::Error:		line.level = lvError;		break;
	default:				line.level = LevelOf(line.text.c_str());	break;
	}

	xrCriticalSection::raii guard(&s_Guard);
	line.epoch = s_Epoch;
	// the status bar's counts and its line: what an author acts on, not the chatter of every render target made
	const bool serious = (line.level == lvError || line.level == lvWarning) && !IsTraceFrame(line.text.c_str());
	line.counted = serious && s_Counted.insert(crc32(line.text.data(), u32(line.text.size()))).second;
	if (line.counted)
		++(line.level == lvError ? s_Errors : s_Warnings);
	if (serious || own)
		s_Last = line.text;
	GetList()->push_back(std::move(line));
}

void UILogForm::State(xr_string& last, int& errors, int& warnings)
{
	xrCriticalSection::raii guard(&s_Guard);
	last = s_Last;
	errors = s_Errors;
	warnings = s_Warnings;
}

void UILogForm::CountFromHere(LPCSTR mark)
{
	xrCriticalSection::raii guard(&s_Guard);
	s_Errors = s_Warnings = 0;
	s_Counted.clear();
	// the last line belongs to the new window, like the counts beside it: a note of the start never outlives them
	s_Last.clear();
	++s_Epoch;
	// the log shows where the counting starts, unless nothing came since the last mark
	xr_vector<SLine>* list = GetList();
	const SLine* prev = !list->empty() ? &list->back() : !s_Lines.empty() ? &s_Lines.back() : nullptr;
	if (!mark || !*mark || !prev || prev->mark)
		return;
	SLine line;
	line.text = mark;
	line.level = lvInfo;
	line.epoch = s_Epoch;
	line.mark = true;
	list->push_back(std::move(line));
}

void UILogForm::ShowLevel(Level level)
{
	s_Level = Level::Error == level ? 2 : Level::Warning == level ? 1 : 0;
	s_ScrollDown = true;
}

UILogForm::Level UILogForm::ShownLevel()
{
	return 2 == s_Level ? Level::Error : 1 == s_Level ? Level::Warning : Level::ByPrefix;
}

bool UILogForm::PanelOpen()
{
	return bAllowLogCommands;
}

void UILogForm::DrawPanel()
{
	if (!bAllowLogCommands)
	{
		// the end is what shows when it opens again
		s_ScrollDown = true;
		return;
	}
	if (XUI::BeginPanel(PanelName(), &bAllowLogCommands, ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse))
		DrawBody();
	XUI::EndPanel();
}

void UILogForm::Show()
{
	bAllowLogCommands = true;
}

void UILogForm::Hide()
{
	bAllowLogCommands = false;
}

void UILogForm::DrawBody()
{
	Take();
	DrawToolbar();
	// after the toolbar, which may have changed the search or the level
	Refilter();
	DrawLines();
	DrawConsole();
}

void UILogForm::Destroy()
{
	{
		xrCriticalSection::raii guard(&s_Guard);
		xr_delete(s_List);
		s_Last.clear();
	}
	s_Lines.clear();
	s_Visible.clear();
	s_Filtered = 0;
	s_Marks = 0;
}
