#pragma once

struct SProgressTaskInfo
{
	xr_string text;
	xr_string detail;
	float current = 0.f;
	float total = 0.f;
	float fraction = 0.f;
	u64 elapsed_ms = 0;
	u32 depth = 0;
	bool determinate = false;
	bool cancelable = false;
	bool cancel_requested = false;
	// a job of a source (below), not the blocking operation: its own name, who started it and how it is cancelled
	xr_string key;
	bool from_mcp = false;
	std::function<void()> cancel;
};

using SProgressTaskInfoVec = xr_vector<SProgressTaskInfo>;

class TUI;

// The Task Center (docs/UI_DESIGN.md, section 5): a chip in the status bar and a popover under it. Each row is a job:
// its title, its stage, its progress and [Cancel]. A blocking job shows the same rows on the busy frame's card.
namespace UIProgressCenter
{
// A job that runs beside the editor (an upload, a scan): the source adds a row per job it runs now, with a `key` that
// stays the same while the job runs. A job the UI started opens the popover once; one MCP started never does.
using Source = std::function<void(SProgressTaskInfoVec& out)>;
ECORE_API void RegisterSource(LPCSTR name, Source source);
// the blocking operation's rows, then every source's
ECORE_API void Snapshot(TUI& ui, SProgressTaskInfoVec& out);
// only the sources' rows (MCP progress background)
ECORE_API void Background(SProgressTaskInfoVec& out);
// the status bar's tasks chip and its popover (ui_popup show=tasks); nothing when no job runs
ECORE_API void DrawChip(TUI& ui, bool busy);
// the task card of the busy frame a blocking job draws
ECORE_API void DrawCard(TUI& ui);
ECORE_API void FormatElapsed(u64 elapsed_ms, xr_string& result);
}
