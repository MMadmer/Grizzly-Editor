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
	// what the card shows beside that: what the job is about, its steps and the one it is on, how far in bytes and files, the file at hand
	xr_string subtitle;
	xr_string numbers;
	xr_string file;
	xr_string stop_label;
	xr_string stop_tip;
	xr_vector<xr_string> steps;
	int step = -1;
	u64 eta_ms = 0;
};

using SProgressTaskInfoVec = xr_vector<SProgressTaskInfo>;

class TUI;

// The Task Center (docs/UI_DESIGN.md, section 5): a chip in the status bar and a card in the corner of the window, over the
// work and in nobody's way. A blocking job shows its rows on the busy frame's card instead.
namespace UIProgressCenter
{
// A job that runs beside the editor (an upload, a scan): the source adds a row per job it runs now, with a `key` that
// stays the same while the job runs. A job the UI started opens the card once; one MCP started never does.
using Source = std::function<void(SProgressTaskInfoVec& out)>;
ECORE_API void RegisterSource(LPCSTR name, Source source);
// the blocking operation's rows, then every source's
ECORE_API void Snapshot(TUI& ui, SProgressTaskInfoVec& out);
// only the sources' rows (MCP progress background)
ECORE_API void Background(SProgressTaskInfoVec& out);
// the status bar's tasks chip: a click shows or hides the card (ui_popup show=tasks shows it); nothing when no job runs
ECORE_API void DrawChip(TUI& ui, bool busy);
// the card of the jobs that run beside the editor; nothing while it is hidden, a dialog is up or no job runs
ECORE_API void DrawFloating();
ECORE_API void ShowCard();
// stops every job that runs beside the editor; false = none does
ECORE_API bool CancelBackground();
// the task card of the busy frame a blocking job draws
ECORE_API void DrawCard(TUI& ui);
ECORE_API void FormatElapsed(u64 elapsed_ms, xr_string& result);
}
