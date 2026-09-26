#include "stdafx.h"
#pragma hdrstop

#include "UI_ProgressCenter.h"
#include "ui_main.h"

// The Task Center is a chip in the status bar and a popover under it; a blocking job shows the same rows on the busy
// frame's card (docs/UI_DESIGN.md, section 5).
namespace
{
constexpr const char* kPopover = "##tasks_popover";

xr_vector<std::pair<xr_string, UIProgressCenter::Source>> s_Sources;
// the jobs of the sources seen so far: a new one the UI started opens the popover once
xr_vector<xr_string> s_Seen;

ImU32 Token(XFinedTheme::ColorToken token, float alpha = 1.f)
{
	return XFinedTheme::U32(token, alpha);
}

// a 4 pt track, filled as far as the job has come; a job of unknown length leaves it empty - only the spinner moves
void Bar(const SProgressTaskInfo& task, float width)
{
	const ImVec2 at = ImGui::GetCursorScreenPos();
	const float h = XUI::Pt(4.f);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(at, ImVec2(at.x + width, at.y + h), Token(XFinedTheme::ColorToken::Inset), h * .5f);
	if (task.determinate && task.fraction > 0.f)
		dl->AddRectFilled(at, ImVec2(at.x + width * task.fraction, at.y + h), Token(XFinedTheme::ColorToken::Accent), h * .5f);
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
			ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextSecondary));
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
			ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextMuted));
			ImGui::TextUnformatted(line.c_str());
			ImGui::PopStyleColor();
		}
		if (task.cancel_requested)
			XUI::StatusText(XUI::Tone::Warning, "Stopping at the next checkpoint.");
		else if (blocking_head && !task.cancelable)
		{
			XUI::FontScope small_font(XUI::Font::Small);
			ImGui::PushStyleColor(ImGuiCol_Text, Token(XFinedTheme::ColorToken::TextMuted));
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

	// a job the UI started comes with its popover; an MCP caller never gets one
	bool announce = false;
	xr_vector<xr_string> seen;
	for (const SProgressTaskInfo& task : tasks)
	{
		if (task.key.empty())
			continue;
		seen.push_back(task.key);
		if (std::find(s_Seen.begin(), s_Seen.end(), task.key) == s_Seen.end() && !task.from_mcp)
			announce = true;
	}
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
			ImGui::PushStyleColor(ImGuiCol_Button, Token(XFinedTheme::ColorToken::Chrome, 0.f));
			const bool clicked = ImGui::Button((shown + "###tasks_chip").c_str());
			ImGui::PopStyleColor();
			ImGui::PopStyleVar();
			XUI::Tip(label.c_str(), nullptr);
			// a blocking job has its card on the busy frame; nothing asks while a dialog or a popup is up
			const bool quiet = busy || ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopupId | ImGuiPopupFlags_AnyPopupLevel);
			if ((clicked && !busy) || XUI::Shell::TakePopup("tasks") || (announce && !quiet))
				XUI::Shell::OpenPopupAt(kPopover, ImGui::GetItemRectMin());
			// over the status bar, never over the chip it belongs to
			XUI::Shell::NextPopupAbove(ImVec2(at.x, ImGui::GetItemRectMin().y), ImGui::GetItemRectMax());
		}
	}
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(12.f), XUI::Pt(10.f)));
	const bool open = ImGui::BeginPopup(kPopover, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings);
	ImGui::PopStyleVar();
	if (open)
	{
		if (tasks.empty())
		{
			ImGui::TextUnformatted("No job runs now.");
			ImGui::CloseCurrentPopup();
		}
		else
			Rows(ui, tasks, XUI::Pt(336.f));
		ImGui::EndPopup();
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
