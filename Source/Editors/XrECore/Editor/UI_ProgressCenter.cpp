#include "stdafx.h"
#pragma hdrstop

#include "UI_ProgressCenter.h"
#include "EditorUiContract.h"
#include "ui_main.h"

// The Task Center is a chip in the status bar and a card in the window's corner; a blocking job shows its rows on the busy
// frame's card (docs/UI_DESIGN.md, section 5).
namespace
{
xr_vector<std::pair<xr_string, UIProgressCenter::Source>> s_Sources;
// the jobs of the sources seen so far: a new one the UI started opens the card once
xr_vector<xr_string> s_Seen;
bool s_CardOpen = false;
// how far each job's bar has come on screen: it follows the job, it does not jump with it
xr_vector<std::pair<xr_string, float>> s_BarShown;

ImU32 Token(GrizzlyTheme::ColorToken token, float alpha = 1.f)
{
	return GrizzlyTheme::U32(token, alpha);
}

// a 4 pt track, filled as far as the job has come; a job of unknown length leaves it empty - only the spinner moves
void Bar(const SProgressTaskInfo& task, float width)
{
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float h = XUI::Pt(4.f);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + width, at.y + h), Token(GrizzlyTheme::ColorToken::Inset), h * .5f);
	if (task.determinate && task.fraction > 0.f)
		dl->AddRectFilled(at, ImVec2(at.x + width * task.fraction, at.y + h), Token(GrizzlyTheme::ColorToken::Accent), h * .5f);
	ImGui::Dummy(ImVec2(width, h));
}

void Rows(TUI& ui, const SProgressTaskInfoVec& tasks, float width)
{
	for (u32 i = 0; i < tasks.size(); ++i)
	{
		const SProgressTaskInfo& task = tasks[i];
		ImGui::PushID(int(i));
		const float indent = XUI::Pt(12.f) * float(task.depth);
		if (indent > 0.f)
			ImGui::Indent(indent);
		const float room = width - indent;

		// the title, and [Cancel] at its right: the blocking job's first row, or a job its source can stop
		const bool blocking_head = task.key.empty() && i == 0;
		const bool can_cancel = !task.cancel_requested && ((blocking_head && task.cancelable) || (!task.key.empty() && task.cancel));
		const char* cancel_label = "Cancel";
		const float cancel_w = can_cancel ? XUI::TextWidth(XUI::Font::Body, cancel_label) + XUI::Pt(20.f) + XUI::Pt(8.f) : 0.f;
		const ImVec2 row = ImGui::GetCursorScreenPos();
		{
			XUI::FontScope strong(XUI::Font::Strong);
			XUI::TextEllipsis(task.text.c_str(), _max(0.f, room - cancel_w));
		}
		if (can_cancel)
		{
			const ImVec2 after = ImGui::GetCursorScreenPos();
			ImGui::SetCursorScreenPos(ImVec2(row.x + room - cancel_w + XUI::Pt(8.f), row.y));
			if (XUI::Button(cancel_label))
			{
				if (!task.key.empty())
					task.cancel();
				else if (ui.NeedBreak())
					ELog.Progress("Cancellation requested: the job stops at its next checkpoint.");
			}
			XUI::Tip(task.key.empty() ? "The job stops the next time it yields to the editor." : "Stops this job.");
			ImGui::SetCursorScreenPos(ImVec2(row.x, _max(after.y, ImGui::GetCursorScreenPos().y)));
		}

		// the stage it is in
		if (!task.detail.empty())
		{
			ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::TextSecondary));
			ImGui::PushTextWrapPos(ImGui::GetCursorPosX() + room);
			ImGui::TextUnformatted(task.detail.c_str());
			ImGui::PopTextWrapPos();
			ImGui::PopStyleColor();
		}
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
		Bar(task, room);

		// how far, and for how long
		xr_string elapsed, line;
		UIProgressCenter::FormatElapsed(task.elapsed_ms, elapsed);
		if (task.determinate)
			line.sprintf("%.0f%%  \xC2\xB7  %s", task.fraction * 100.f, elapsed.c_str());
		else
			line = elapsed;
		{
			XUI::FontScope small_font(XUI::Font::Small);
			ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::TextMuted));
			ImGui::TextUnformatted(line.c_str());
			ImGui::PopStyleColor();
		}
		if (task.cancel_requested)
			XUI::StatusText(XUI::Tone::Warning, "Stopping at the next checkpoint.");
		else if (blocking_head && !task.cancelable)
		{
			XUI::FontScope small_font(XUI::Font::Small);
			ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::TextMuted));
			ImGui::TextUnformatted("This job cannot be cancelled safely.");
			ImGui::PopStyleColor();
		}
		if (indent > 0.f)
			ImGui::Unindent(indent);
		ImGui::PopID();
		if (i + 1 < tasks.size())
			ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));
	}
}
}

namespace
{
float ShownFraction(const SProgressTaskInfo& task)
{
	const float dt = ImGui::GetIO().DeltaTime;
	for (auto& [key, shown] : s_BarShown)
		if (key == task.key)
		{
			// a new stage starts from nothing, and that is not to be animated
			shown = task.fraction < shown ? task.fraction : shown + (task.fraction - shown) * _min(1.f, dt * 10.f);
			return shown;
		}
	s_BarShown.emplace_back(task.key, task.fraction);
	return task.fraction;
}

float LineHeight(XUI::Font font)
{
	XUI::FontScope scope(font);
	return ImGui::GetTextLineHeight();
}

// a 6 pt track: filled as far as the job has come, or crossed by a light while nobody can tell how long it takes
void CardBar(const SProgressTaskInfo& task, float width)
{
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float h = XUI::Pt(6.f);
	const float r = h * .5f;
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + width, at.y + h), Token(GrizzlyTheme::ColorToken::Inset), r);
	if (task.determinate)
	{
		const float fill = ShownFraction(task) * width;
		if (fill > 0.f)
			dl->AddRectFilled(at, ImVec2(at.x + _max(fill, h), at.y + h), Token(GrizzlyTheme::ColorToken::Accent), r);
	}
	else
	{
		const float period = 1.4f;
		const float phase = fmodf(float(ImGui::GetTime()), period) / period;
		const float eased = phase * phase * (3.f - 2.f * phase);
		const float light = width * .32f;
		const float x = at.x - light + (width + light) * eased;
		dl->PushClipRect(at, ImVec2(at.x + width, at.y + h), true);
		dl->AddRectFilled(ImVec2(x, at.y), ImVec2(x + light, at.y + h), Token(GrizzlyTheme::ColorToken::Accent), r);
		dl->PopClipRect();
	}
	ImGui::Dummy(ImVec2(width, h));
}

// o Check --- O Copy --- . Install: where the job is on its way
void CardSteps(const SProgressTaskInfo& task, float width)
{
	const int count = int(task.steps.size());
	if (!count)
		return;
	const float dot = XUI::Pt(3.f);
	const float gap = XUI::Pt(5.f);
	float labels = 0.f;
	for (const xr_string& step : task.steps)
		labels += XUI::TextWidth(XUI::Font::Small, step.c_str()) + dot * 2.f + gap;
	const float link = count > 1 ? _max(XUI::Pt(6.f), (width - labels) / float(count - 1)) : 0.f;

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float text_h = LineHeight(XUI::Font::Small);
	const float line = text_h * 1.3f;
	const float cy = at.y + line * .5f;
	float x = at.x;
	for (int i = 0; i < count; ++i)
	{
		const bool done = i < task.step;
		const bool now = i == task.step;
		const ImU32 tone = done || now ? Token(GrizzlyTheme::ColorToken::Accent) : Token(GrizzlyTheme::ColorToken::TextMuted);
		const ImVec2 centre(x + dot, cy);
		if (done || now)
			dl->AddCircleFilled(centre, dot, tone, 16);
		else
			dl->AddCircle(centre, dot - XUI::Pt(.5f), tone, 16, XUI::Pt(1.f));
		if (now)
			dl->AddCircle(centre, dot + XUI::Pt(2.f), Token(GrizzlyTheme::ColorToken::Accent, .45f), 20, XUI::Pt(1.f));
		x += dot * 2.f + gap;
		const char* name = task.steps[i].c_str();
		XUI::TextAt(dl, XUI::Font::Small, ImVec2(x, at.y + (line - text_h) * .5f),
			now ? Token(GrizzlyTheme::ColorToken::Text) : done ? Token(GrizzlyTheme::ColorToken::TextSecondary) : Token(GrizzlyTheme::ColorToken::TextMuted), name);
		x += XUI::TextWidth(XUI::Font::Small, name);
		if (i + 1 < count)
		{
			const float a = x + gap;
			const float b = x + link - gap;
			if (b > a)
				dl->AddLine(ImVec2(a, cy), ImVec2(b, cy), done ? Token(GrizzlyTheme::ColorToken::Accent, .6f) : Token(GrizzlyTheme::ColorToken::BorderStrong), XUI::Pt(1.f));
			x += link;
		}
	}
	ImGui::Dummy(ImVec2(width, line));
}

void SmallLine(GrizzlyTheme::ColorToken tone, const char* text, float width, XUI::Clip clip = XUI::Clip::End)
{
	XUI::FontScope small_font(XUI::Font::Small);
	ImGui::PushStyleColor(ImGuiCol_Text, Token(tone));
	XUI::TextEllipsis(text, width, clip);
	ImGui::PopStyleColor();
}

void CardRow(const SProgressTaskInfo& task, float width, bool first)
{
	ImGui::PushID(task.key.c_str());
	const float hide_w = first ? XUI::Pt(24.f) : 0.f;
	const ImVec2 head = ImGui::GetCursorScreenPos();
	const float title_h = XUI::Pt(24.f);
	ImGui::SetCursorScreenPos(ImVec2(head.x, head.y + (title_h - XUI::Pt(16.f)) * .5f));
	XUI::Spinner();
	ImGui::SetCursorScreenPos(ImVec2(head.x + XUI::Pt(24.f), head.y + (title_h - LineHeight(XUI::Font::Strong)) * .5f));
	{
		XUI::FontScope strong(XUI::Font::Strong);
		XUI::TextEllipsis(task.text.c_str(), _max(0.f, width - XUI::Pt(24.f) - hide_w));
	}
	if (first)
	{
		ImGui::SetCursorScreenPos(ImVec2(head.x + width - hide_w, head.y));
		if (XUI::IconButton("##hide_card", XUI::Icon::ChevronDown, "Hide. The job goes on, and the chip in the status bar brings this back."))
			s_CardOpen = false;
	}
	ImGui::SetCursorScreenPos(ImVec2(head.x, head.y + title_h));
	if (!task.subtitle.empty())
		SmallLine(GrizzlyTheme::ColorToken::TextSecondary, task.subtitle.c_str(), width, XUI::Clip::Middle);

	ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
	CardSteps(task, width);
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
	CardBar(task, width);
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));

	// what it is doing, and for how long: "Copying files        00:42 - about 01:08 left"
	xr_string elapsed, time_text;
	UIProgressCenter::FormatElapsed(task.elapsed_ms, elapsed);
	time_text = elapsed;
	if (task.eta_ms)
	{
		xr_string left;
		UIProgressCenter::FormatElapsed(task.eta_ms, left);
		time_text += " \xC2\xB7 about " + left + " left";
	}
	const float time_w = XUI::TextWidth(XUI::Font::Small, time_text.c_str());
	const ImVec2 line = ImGui::GetCursorScreenPos();
	{
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, Token(GrizzlyTheme::ColorToken::Text));
		XUI::TextEllipsis(task.detail.c_str(), _max(0.f, width - time_w - XUI::Pt(12.f)));
		ImGui::PopStyleColor();
	}
	const ImVec2 after_line = ImGui::GetCursorScreenPos();
	XUI::TextAt(ImGui::GetWindowDrawList(), XUI::Font::Small, ImVec2(line.x + width - time_w, line.y), Token(GrizzlyTheme::ColorToken::TextMuted), time_text.c_str());
	ImGui::SetCursorScreenPos(after_line);

	// how far: "38% - 2.1 of 4.6 GB - 3412 of 7146 files"
	if (task.determinate)
	{
		xr_string numbers;
		numbers.sprintf("%.0f%%", task.fraction * 100.f);
		if (!task.numbers.empty())
			numbers += "  \xC2\xB7  " + task.numbers;
		SmallLine(GrizzlyTheme::ColorToken::TextSecondary, numbers.c_str(), width);
	}
	if (!task.file.empty())
		SmallLine(GrizzlyTheme::ColorToken::TextMuted, task.file.c_str(), width, XUI::Clip::Middle);

	if (task.cancel)
	{
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
		const char* label = task.cancel_requested ? "Stopping..." : task.stop_label.empty() ? "Stop" : task.stop_label.c_str();
		const float button_w = XUI::TextWidth(XUI::Font::Body, label) + XUI::Pt(24.f);
		ImGui::SetCursorScreenPos(ImVec2(head.x + width - button_w, ImGui::GetCursorScreenPos().y));
		if (XUI::Button(label, XUI::Kind::Secondary, !task.cancel_requested, "It stops at the next step."))
			task.cancel();
		if (!task.cancel_requested && !task.stop_tip.empty())
			XUI::Tip(task.stop_tip.c_str());
	}
	ImGui::PopID();
}
}

void UIProgressCenter::ShowCard()
{
	s_CardOpen = true;
}

bool UIProgressCenter::CancelBackground()
{
	SProgressTaskInfoVec tasks;
	Background(tasks);
	bool any = false;
	for (const SProgressTaskInfo& task : tasks)
		if (task.cancel)
		{
			task.cancel();
			any = true;
		}
	return any;
}

void UIProgressCenter::DrawFloating()
{
	SProgressTaskInfoVec tasks;
	Background(tasks);
	// the toasts stand on the card while it is up
	XUI::SetToastInset(0.f);
	if (tasks.empty())
	{
		s_CardOpen = false;
		s_BarShown.clear();
		return;
	}
	// a dialog is the author's work now: the card waits behind it, the chip goes on saying what runs
	if (!s_CardOpen || XUI::ModalUp())
		return;

	const ImGuiViewport* vp = ImGui::GetMainViewport();
	const float padding = XUI::Pt(14.f);
	const float width = XUI::Pt(400.f);
	ImGui::SetNextWindowPos(ImVec2(vp->WorkPos.x + vp->WorkSize.x - XUI::Pt(16.f), vp->WorkPos.y + vp->WorkSize.y - XUI::Pt(16.f)), ImGuiCond_Always, ImVec2(1.f, 1.f));
	ImGui::SetNextWindowViewport(vp->ID);
	ImGui::SetNextWindowSizeConstraints(ImVec2(width, 0.f), ImVec2(width, vp->WorkSize.y));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(padding, XUI::Pt(10.f)));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, XUI::Pt(8.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, XUI::Pt(1.f));
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(XUI::Pt(8.f), XUI::Pt(2.f)));
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Token(GrizzlyTheme::ColorToken::Overlay));
	ImGui::PushStyleColor(ImGuiCol_Border, Token(GrizzlyTheme::ColorToken::BorderStrong));
	const ImGuiWindowFlags flags = ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoFocusOnAppearing |
		ImGuiWindowFlags_NoNav | ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoMove;
	if (ImGui::Begin(EditorUiContract::kTaskCard, nullptr, flags))
	{
		XUI::KeepOnTop();
		const float room = width - padding * 2.f;
		for (u32 i = 0; i < tasks.size(); ++i)
		{
			if (i)
			{
				ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
				ImGui::Separator();
				ImGui::Dummy(ImVec2(0.f, XUI::Pt(6.f)));
			}
			CardRow(tasks[i], room, 0 == i);
		}
		XUI::SetToastInset(ImGui::GetWindowHeight() + XUI::Pt(8.f));
	}
	ImGui::End();
	ImGui::PopStyleColor(2);
	ImGui::PopStyleVar(4);
}

void UIProgressCenter::RegisterSource(LPCSTR name, Source source)
{
	if (!name || !*name)
		return;
	for (auto& [key, fn] : s_Sources)
		if (key == name)
		{
			fn = std::move(source);
			return;
		}
	s_Sources.emplace_back(name, std::move(source));
}

void UIProgressCenter::Background(SProgressTaskInfoVec& out)
{
	out.clear();
	for (auto& [name, source] : s_Sources)
		if (source)
			source(out);
}

void UIProgressCenter::Snapshot(TUI& ui, SProgressTaskInfoVec& out)
{
	ui.GetProgressSnapshot(out);
	SProgressTaskInfoVec background;
	Background(background);
	for (SProgressTaskInfo& task : background)
		out.push_back(std::move(task));
}

void UIProgressCenter::FormatElapsed(u64 elapsed_ms, xr_string& result)
{
	const u64 total_seconds = elapsed_ms / 1000;
	const u64 hours = total_seconds / 3600;
	const u64 minutes = (total_seconds / 60) % 60;
	const u64 seconds = total_seconds % 60;

	if (hours)
		result.sprintf("%llu:%02llu:%02llu", hours, minutes, seconds);
	else
		result.sprintf("%02llu:%02llu", minutes, seconds);
}

void UIProgressCenter::DrawChip(TUI& ui, bool busy)
{
	SProgressTaskInfoVec tasks;
	Snapshot(ui, tasks);

	// a job the UI started comes with its card; an MCP caller never gets one
	for (const SProgressTaskInfo& task : tasks)
		if (!task.key.empty() && !task.from_mcp && std::find(s_Seen.begin(), s_Seen.end(), task.key) == s_Seen.end())
			s_CardOpen = true;
	xr_vector<xr_string> seen;
	for (const SProgressTaskInfo& task : tasks)
		if (!task.key.empty())
			seen.push_back(task.key);
	s_Seen.swap(seen);

	if (!tasks.empty())
	{
		// the spinner, and what runs: "Packaging release..." or "3 tasks"
		xr_string label;
		if (tasks.size() == 1)
			label = tasks.front().text.c_str();
		else
			label.sprintf("%u tasks", u32(tasks.size()));
		const ImVec2 at = ImGui::GetCursorScreenPos();
		const float h = XUI::Pt(20.f);
		ImGui::SetCursorScreenPos(ImVec2(at.x, at.y + (h - XUI::Pt(16.f)) * .5f));
		XUI::Spinner();
		ImGui::SetCursorScreenPos(ImVec2(at.x + XUI::Pt(16.f) + XUI::Pt(2.f), at.y));
		{
			XUI::FontScope small_font(XUI::Font::Small);
			// the chip keeps a label short: the whole title is in the popover
			const float max_w = XUI::Em(18.f);
			xr_string shown = label;
			if (ImGui::CalcTextSize(label.c_str()).x > max_w)
			{
				while (shown.size() > 1 && ImGui::CalcTextSize((shown + "...").c_str()).x > max_w)
					shown.pop_back();
				shown += "...";
			}
			ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(XUI::Pt(6.f), (h - ImGui::GetFontSize()) * .5f));
			ImGui::PushStyleColor(ImGuiCol_Button, Token(GrizzlyTheme::ColorToken::Chrome, 0.f));
			const bool clicked = ImGui::Button((shown + "###tasks_chip").c_str());
			ImGui::PopStyleColor();
			ImGui::PopStyleVar();
			XUI::Tip(label.c_str(), nullptr);
			// a blocking job has its card on the busy frame
			if (clicked && !busy)
				s_CardOpen = !s_CardOpen;
			if (XUI::Shell::TakePopup("tasks"))
				s_CardOpen = true;
		}
	}
}

void UIProgressCenter::DrawCard(TUI& ui)
{
	SProgressTaskInfoVec tasks;
	Snapshot(ui, tasks);
	{
		XUI::FontScope title(XUI::Font::Title);
		ImGui::TextUnformatted(tasks.size() > 1 ? "Working on several jobs" : "Working");
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
	if (tasks.empty())
	{
		ImGui::TextUnformatted("Finishing...");
		return;
	}
	Rows(ui, tasks, ImGui::GetContentRegionAvail().x);
}
