#include "stdafx.h"
#include "LevelMcp.h"
#include "..\Recovery\EditorRecovery.h"
#include "..\..\..\XrECore\Editor\EditorUiContract.h"
#include "..\..\..\XrECore\Editor\EditorModManifest.h"
#include "..\..\..\XrECore\Editor\EditorModPublish.h"
#include "..\..\..\XrECore\Editor\EditorCatalogSubmit.h"
#include "..\..\..\XrECore\Editor\EditorCharacters.h"
#include "..\..\..\XrECore\Editor\EditorWorld.h"
#include "..\..\..\XrECore\Editor\EditorPda.h"
#include "..\..\..\XrECore\Editor\EditorFileOps.h"
#include "..\..\..\XrECore\Editor\UI_CommandPalette.h"
#include "..\..\..\XrECore\Editor\UILogForm.h"
#include "..\..\..\XrECore\Editor\Nq\NqUtil.h"
#include "..\..\UI\UITitleBar.h"
#include "..\..\UI\QuestGraph\UIQuestGraph.h"

// The frame over MCP (docs/UI_DESIGN.md, section 7): tabs, panels, drawers, dialogs, popups, scroll, splits, the
// audit, the scale and the window - and what was always the editor's own here: theme, frame pacing, the palette,
// progress, recovery and the command registry. GrizzlyInspector at the end asks every group in turn.
namespace LevelMcp
{
// JSON string body. Object names on real levels are library paths
// ("statics\sign\..."), scene names are full paths: a raw copy is not valid
// JSON and a strict client fails on the whole reply. Never truncates.
xr_string JsonStr(LPCSTR s)
{
    xr_string r;
    for (LPCSTR c = s ? s : ""; *c; ++c)
    {
        switch (*c)
        {
        case '\\':	r += "\\\\";	break;
        case '"':	r += "\\\"";	break;
        case '\n':	r += "\\n";		break;
        case '\r':	r += "\\r";		break;
        case '\t':	r += "\\t";		break;
        default:
            if (u8(*c) < 0x20)	{ char u[8]; sprintf_s(u, "\\u%04x", int(u8(*c))); r += u; }
            else				r += *c;
        }
    }
    return r;
}

// bare (unquoted) JSON values: {"select":true} / {"limit":50}
bool GetArgBool(LPCSTR raw, LPCSTR field, bool def)
{
    char pat[64];
    sprintf_s(pat, "\"%s\"", field);
    const char* k = raw ? strstr(raw, pat) : 0;
    if (!k) return def;
    const char* c = strchr(k + xr_strlen(pat), ':');
    if (!c) return def;
    // quotes are stepped over so {"open":"1"} reads the same as {"open":1}
    while (*++c == ' ' || *c == '"') {}
    return 0 == strncmp(c, "true", 4) || *c == '1';
}

int GetArgInt(LPCSTR raw, LPCSTR field, int def)
{
    char pat[64];
    sprintf_s(pat, "\"%s\"", field);
    const char* k = raw ? strstr(raw, pat) : 0;
    if (!k) return def;
    const char* c = strchr(k + xr_strlen(pat), ':');
    if (!c) return def;
    // skip whitespace; a field that is present but non-numeric answers def,
    // and 0 is a legitimate value - "v>0 ? v : def" used to eat it
    while (*++c == ' ' || *c == '	') {}
    if (*c != '-' && (*c < '0' || *c > '9'))	return def;
    return atoi(c);
}

// bare float: {"x":-12.5}; leaves `out` untouched when the field is absent
bool GetArgFloat(LPCSTR raw, LPCSTR field, float& out)
{
    char pat[64];
    sprintf_s(pat, "\"%s\"", field);
    const char* k = raw ? strstr(raw, pat) : 0;
    if (!k) return false;
    const char* c = strchr(k + xr_strlen(pat), ':');
    if (!c) return false;
    out = (float)atof(c + 1);
    return true;
}

// Asset and file names carry backslashes, which JSON needs doubled - which is
// what JsonStr already does. The one thing this must NOT do is drop control
// bytes: a caller that silently loses them hands the bridge a string that no
// longer names the file it came from.
xr_string JsonEscapePath(LPCSTR text)
{
    return JsonStr(text);
}

// object names are lowercased on SetName - make lookups case-insensitive
CCustomObject* FindObjectRelaxed(LPCSTR name)
{
    char low[256];
    strncpy_s(low, sizeof(low), name ? name : "", _TRUNCATE);
    strlwr(low);
    return Scene->FindObjectByName(low, (CCustomObject*)0);
}

static void RespondThemeJson(xr_string& out)
{
	const GrizzlyTheme::Preset current = GrizzlyTheme::Current();
	out = "{\"ok\":true,\"preset\":\"";
	out += GrizzlyTheme::Key(current);
	out += "\",\"name\":\"";
	out += GrizzlyTheme::Name(current);
	out += "\",\"default\":";
	out += current == GrizzlyTheme::Default() ? "true" : "false";
	out += ",\"presets\":[";
	for (u32 i = 0; i < u32(GrizzlyTheme::Preset::Count); ++i)
	{
		if (i)
			out += ",";
		const GrizzlyTheme::Preset preset = GrizzlyTheme::Preset(i);
		out += "{\"key\":\"";
		out += GrizzlyTheme::Key(preset);
		out += "\",\"name\":\"";
		out += GrizzlyTheme::Name(preset);
		out += "\"}";
	}
	out += "]}";
}

static void RespondFramePacingJson(xr_string& out)
{
	SFramePacingStats stats;
	UI->GetFramePacingStats(stats);
	LPCSTR mode = stats.play_in_editor ? "play_in_editor"
		: stats.realtime_render ? "realtime_render"
		: !stats.app_active ? "background"
		: stats.idle_cap_active ? "idle_cap"
		: "explicit_work";
	char values[768];
	sprintf_s(values,
		"{\"ok\":true,\"active_idle_fps\":%u,\"background_poll_ms\":%u,"
		"\"measured_fps\":%.3f,\"measured_frame_ms\":%.3f,\"mode\":\"%s\","
		"\"last_wait_reason\":\"%s\",\"app_active\":%s,\"play_in_editor\":%s,"
		"\"realtime_render\":%s,\"redraw_pending\":%s,\"idle_cap_active\":%s,"
		"\"frames\":%llu,\"waits\":%llu,\"waited_ms\":%.3f}",
		stats.active_idle_fps, stats.background_poll_ms, stats.measured_fps,
		stats.measured_frame_ms, mode, stats.last_wait_reason,
		stats.app_active ? "true" : "false", stats.play_in_editor ? "true" : "false",
		stats.realtime_render ? "true" : "false", stats.redraw_pending ? "true" : "false",
		stats.idle_cap_active ? "true" : "false",
		static_cast<unsigned long long>(stats.frames), static_cast<unsigned long long>(stats.waits),
		double(stats.waited_us) / 1000.0);
	out = values;
}

static void RespondCommandPaletteJson(xr_string& out, LPCSTR query, int limit)
{
	xr_vector<CommandPalette::SResult> results;
	CommandPalette::Query(query, results, u32(std::clamp(limit, 1, 250)));
	out = "{\"ok\":true,\"open\":";
	out += CommandPalette::IsOpen() ? "true" : "false";
	out += ",\"query\":\"";
	out += JsonStr(query);
	out += "\",\"results\":[";
	for (u32 i = 0; i < results.size(); ++i)
	{
		if (i)
			out += ",";
		const CommandPalette::SResult& result = results[i];
		// a panel, a drawer or a tab has no command of the registry: -1, and its id runs it
		char numbers[128];
		sprintf_s(numbers, "\",\"command\":%d,\"subcommand\":%d,\"score\":%d}",
			result.command == u32(-1) ? -1 : int(result.command), result.command == u32(-1) ? -1 : int(result.subcommand), result.score);
		out += "{\"id\":\"";
		out += JsonStr(result.id.c_str());
		out += "\",\"kind\":\"";
		out += JsonStr(result.kind.c_str());
		out += "\",\"label\":\"";
		out += JsonStr(result.label.c_str());
		out += "\",\"category\":\"";
		out += JsonStr(result.category.c_str());
		out += "\",\"path\":\"";
		out += JsonStr(result.path.c_str());
		out += "\",\"name\":\"";
		out += JsonStr(result.name.c_str());
		out += "\",\"shortcut\":\"";
		out += JsonStr(result.shortcut.c_str());
		out += numbers;
	}
	out += "]}";
}

static void AppendTasksJson(xr_string& out, const SProgressTaskInfoVec& tasks)
{
	for (u32 i = 0; i < tasks.size(); ++i)
	{
		if (i)
			out += ",";
		const SProgressTaskInfo& task = tasks[i];
		char values[256];
		sprintf_s(values,
			"\",\"current\":%.3f,\"total\":%.3f,\"fraction\":%.6f,\"elapsed_ms\":%llu,"
			"\"depth\":%u,\"determinate\":%s,\"cancelable\":%s,\"cancel_requested\":%s}",
			task.current, task.total, task.fraction, task.elapsed_ms, task.depth,
			task.determinate ? "true" : "false", task.cancelable ? "true" : "false",
			task.cancel_requested ? "true" : "false");
		out += "{\"text\":\"";
		out += JsonStr(task.text.c_str());
		out += "\",\"detail\":\"";
		out += JsonStr(task.detail.c_str());
		if (!task.key.empty())
		{
			out += "\",\"key\":\"";
			out += JsonStr(task.key.c_str());
			out += "\",\"subtitle\":\"";
			out += JsonStr(task.subtitle.c_str());
			out += "\",\"numbers\":\"";
			out += JsonStr(task.numbers.c_str());
			out += "\",\"file\":\"";
			out += JsonStr(task.file.c_str());
			out += "\",\"steps\":\"";
			for (u32 s = 0; s < task.steps.size(); ++s)
			{
				if (s) out += ", ";
				out += JsonStr(task.steps[s].c_str());
			}
			char where[64];
			sprintf_s(where, "\",\"step\":%d,\"eta_ms\":%llu,\"note\":\"", task.step, task.eta_ms);
			out += where;
			out += JsonStr(task.from_mcp ? "started over MCP" : "started from the UI");
		}
		out += values;
	}
}

static void RespondProgressJson(xr_string& out)
{
	SProgressTaskInfoVec tasks;
	UI->GetProgressSnapshot(tasks);
	out = "{\"ok\":true,\"active\":";
	out += UI->ProgressOperationActive() ? "true" : "false";
	out += ",\"cancelable\":";
	out += UI->ProgressCancelable() ? "true" : "false";
	out += ",\"cancel_requested\":";
	out += UI->NeedAbort() ? "true" : "false";
	out += ",\"tasks\":[";
	AppendTasksJson(out, tasks);
	// the jobs that run beside the editor (their sources: uploads, scans), which block nothing
	UIProgressCenter::Background(tasks);
	out += "],\"background\":[";
	AppendTasksJson(out, tasks);
	// what the status bar shows beside the log button and in its chips: a job's own lines never stand there
	xr_string last;
	int errors = 0, warnings = 0;
	UILogForm::State(last, errors, warnings);
	char counts[64];
	sprintf_s(counts, "],\"errors\":%d,\"warnings\":%d,\"status_line\":\"", errors, warnings);
	out += counts;
	out += JsonStr(last.c_str());
	out += "\"}";
}

static void RespondRecoveryJson(xr_string& out, bool ok, LPCSTR request_error = nullptr)
{
    EditorRecovery::SStatus status;
    EditorRecovery::GetStatus(status);
    char values[768];
    sprintf_s(values,
        "{\"ok\":%s,\"state\":\"%s\",\"operation_id\":%llu,\"pending\":%s,"
        "\"enabled\":%s,\"owns_session\":%s,\"shared_session\":%s,\"dirty\":%s,"
        "\"can_snapshot\":%s,\"can_restore\":%s,\"can_discard\":%s,"
        "\"snapshot_available\":%s,\"source_newer\":%s,\"saved_at\":%llu,\"source\":\"",
        ok ? "true" : "false", status.state.c_str(),
        static_cast<unsigned long long>(status.operation_id), status.pending ? "true" : "false",
        status.enabled ? "true" : "false", status.owns_session ? "true" : "false",
        status.shared_session ? "true" : "false", status.dirty ? "true" : "false",
        status.can_snapshot ? "true" : "false", status.can_restore ? "true" : "false",
        status.can_discard ? "true" : "false", status.snapshot_available ? "true" : "false",
        status.source_newer ? "true" : "false", static_cast<unsigned long long>(status.saved_at));
    out = values;
    out += JsonStr(status.source.c_str());
    out += "\",\"snapshot\":\"";
    out += JsonStr(status.snapshot.c_str());
    out += "\",\"last_error\":\"";
    out += JsonStr(status.last_error.c_str());
    out += "\"";
    if (request_error && request_error[0])
    {
        out += ",\"error\":\"";
        out += JsonStr(request_error);
        out += "\"";
    }
    out += "}";
}

// =====================================================================================================================
// The frame: what ui_* reads and asks for
// =====================================================================================================================
namespace
{
using namespace EditorUiContract;

xr_string Arg(LPCSTR raw, LPCSTR key)
{
	xr_string value;
	NqUtil::ArgString(raw, key, value);
	return value;
}

void JsonRect(xr_string& out, const ImVec4& r)
{
	char tmp[96];
	sprintf_s(tmp, "[%.0f,%.0f,%.0f,%.0f]", r.x, r.y, r.z, r.w);
	out += tmp;
}

// ---- panels -----------------------------------------------------------------------------------------------------
// what opens, closes and reports each panel of the Level tab; `drawer` shows it where the layout has no room for it
struct SPanelOps
{
	LPCSTR id;
	bool (*is_open)();
	void (*open)();
	void (*close)();		// null: it does not close
	int drawer;
};

const SPanelOps kPanelOps[] =
{
	{ "tools",			[] { return !MainForm->GetLeftBarForm()->IsClosed(); },		[] { MainForm->GetLeftBarForm()->Open(); },
						[] { MainForm->GetLeftBarForm()->Close(); },				XUI::Shell::DrawerNone },
	{ "world_outliner",	[] { return UIWorldOutliner::IsOpen(); },					[] { UIWorldOutliner::Show(); },
						[] { UIWorldOutliner::Close(); },							XUI::Shell::DrawerNone },
	{ "details",		[] { return !MainForm->GetPropertiesFrom()->IsClosed(); },	[] { MainForm->GetPropertiesFrom()->Open(); },
						[] { MainForm->GetPropertiesFrom()->Close(); },				XUI::Shell::DrawerNone },
	{ "level_settings",	[] { return !MainForm->GetWorldPropertiesFrom()->IsClosed(); },	[] { MainForm->GetWorldPropertiesFrom()->Open(); },
						[] { MainForm->GetWorldPropertiesFrom()->Close(); },		XUI::Shell::DrawerNone },
	{ "content_browser",	[] { return UIContentBrowser::IsOpen(); },				[] { UIContentBrowser::Show(); },
						[] { UIContentBrowser::Close(); },							XUI::Shell::DrawerContent },
	{ "output_log",		[] { return UILogForm::PanelOpen(); },						[] { UILogForm::Show(); },
						[] { UILogForm::Hide(); },									XUI::Shell::DrawerLog },
	{ "viewport",		[] { return true; },										[] {},
						nullptr,													XUI::Shell::DrawerNone },
};

const SPanelOps* FindPanel(const xr_string& id)
{
	for (const SPanelOps& p : kPanelOps)
		if (id == p.id)
			return &p;
	return nullptr;
}

LPCSTR PanelWindow(LPCSTR id)
{
	for (const SPanel& p : kPanels)
		if (!xr_strcmp(id, p.id))
			return p.window;
	return "";
}

// the Compact class keeps the browser and the log in drawers, unless the author docked them
bool PanelHasRoom(const SPanelOps& p)
{
	return p.drawer == XUI::Shell::DrawerNone || UIMainForm::SizeClass() == UIMainForm::ClassWide || (EPrefs && EPrefs->level_layout_custom);
}

LPCSTR DrawerName(int drawer)
{
	return drawer == XUI::Shell::DrawerContent ? "content" : drawer == XUI::Shell::DrawerLog ? "log" : "none";
}

void AppendPanelJson(xr_string& out, const SPanelOps& p)
{
	const LPCSTR window = PanelWindow(p.id);
	XUI::Shell::WindowState ws;
	XUI::Shell::GetWindowState(window, ws);
	const bool maximized = MainForm->ViewportMaximized() && xr_strcmp(p.id, "viewport");
	const bool visible = XUI::Shell::IsActive("level") && ws.shown && !maximized;
	const bool in_drawer = p.drawer != XUI::Shell::DrawerNone && XUI::Shell::OpenedDrawer() == p.drawer;
	out += "{\"id\":\"";
	out += p.id;
	out += "\",\"window\":";
	NqUtil::JsonString(out, window);
	out += p.is_open() ? ",\"open\":true" : ",\"open\":false";
	out += visible ? ",\"visible\":true" : ",\"visible\":false";
	out += ws.docked ? ",\"docked\":true" : ",\"docked\":false";
	out += ws.focused ? ",\"focused\":true" : ",\"focused\":false";
	out += p.close ? ",\"closable\":true" : ",\"closable\":false";
	out += ",\"host\":\"";
	out += visible ? "panel" : in_drawer ? "drawer" : "none";
	out += "\",\"rect\":";
	if (visible)
		JsonRect(out, ws.rect);
	else
		out += "null";
	out += "}";
}

// ---- tabs -------------------------------------------------------------------------------------------------------
void AppendTabJson(xr_string& out, const XUI::Shell::DocInfo& t)
{
	out += "{\"id\":";
	NqUtil::JsonString(out, t.id);
	out += ",\"kind\":";
	NqUtil::JsonString(out, t.kind);
	out += ",\"label\":";
	NqUtil::JsonString(out, t.label);
	out += ",\"path\":";
	NqUtil::JsonString(out, t.path);
	out += t.dirty ? ",\"dirty\":true" : ",\"dirty\":false";
	out += t.active ? ",\"active\":true" : ",\"active\":false";
	out += t.missing ? ",\"missing\":true" : ",\"missing\":false";
	out += t.closable ? ",\"closable\":true" : ",\"closable\":false";
	out += t.loaded ? ",\"loaded\":true}" : ",\"loaded\":false}";
}

void AppendTabsJson(xr_string& out)
{
	out += ",\"active_tab\":";
	NqUtil::JsonString(out, XUI::Shell::Active());
	out += ",\"tabs\":[";
	xr_vector<XUI::Shell::DocInfo> tabs;
	XUI::Shell::List(tabs);
	for (u32 i = 0; i < tabs.size(); ++i)
	{
		if (i)
			out += ",";
		AppendTabJson(out, tabs[i]);
	}
	out += "]";
}

// the tab that is up now, as the answer of an opener
void AnswerActiveTab(xr_string& out, LPCSTR note = nullptr)
{
	out = "{\"ok\":true";
	XUI::Shell::DocInfo info;
	if (XUI::Shell::Info(XUI::Shell::Active(), info))
	{
		out += ",\"tab\":";
		AppendTabJson(out, info);
	}
	if (note)
	{
		out += ",\"note\":";
		NqUtil::JsonString(out, note);
	}
	out += "}";
}

// "project" | "editor" | "darf" or 0 / 1 / 2, the Content Browser's sources; -1 when it is none of them
int SourceOf(const xr_string& s)
{
	if (s.empty() || s == "project" || s == "0")	return 0;
	if (s == "editor" || s == "1")					return 1;
	if (s == "darf" || s == "2")					return 2;
	return -1;
}

// ---- dialogs ----------------------------------------------------------------------------------------------------
const SDialog* FindDialog(const xr_string& id)
{
	for (const SDialog& d : kDialogs)
		if (id == d.id)
			return &d;
	return nullptr;
}

// the ImGui window an MCP id names: a panel, a drawer, a dialog, a kit dialog (dlg.<name>), the work area of the
// active tab, or else a window's own name
xr_string WindowOf(const xr_string& id)
{
	for (const SPanel& p : kPanels)
		if (id == p.id)
			return p.window;
	for (const SDrawer& d : kDrawers)
		if (id == d.id)
			return d.window;
	if (id == "start_page")
		return kWindowStartPage;
	// a tab up now is its work area - Mod Settings and Releases were dialogs of the same id before they were tabs
	if (id == "work_area" || id == XUI::Shell::Active())
		return "###doc.host";
	if (const SDialog* d = FindDialog(id))
		return d->window;
	if (!id.compare(0, 4, "dlg."))
		return "###" + id;
	return id;
}

bool ModalOpen(const xr_string& window)
{
	xr_vector<XUI::Shell::PopupState> popups;
	XUI::Shell::OpenPopups(popups);
	for (const XUI::Shell::PopupState& p : popups)
		if (p.modal && (p.name == window || "###" + p.id == window))
			return true;
	return false;
}

void AppendPopupsJson(xr_string& out, bool modals)
{
	xr_vector<XUI::Shell::PopupState> popups;
	XUI::Shell::OpenPopups(popups);
	bool first = true;
	for (const XUI::Shell::PopupState& p : popups)
	{
		if (p.modal != modals)
			continue;
		// a dialog of the contract answers to its MCP id
		xr_string id = p.id;
		for (const SDialog& d : kDialogs)
			if (p.name == d.window)
				id = d.id;
		out += first ? "{\"id\":" : ",{\"id\":";
		first = false;
		NqUtil::JsonString(out, id);
		out += ",\"title\":";
		NqUtil::JsonString(out, p.title);
		out += ",\"window\":";
		NqUtil::JsonString(out, p.name);
		out += ",\"rect\":";
		JsonRect(out, p.rect);
		out += "}";
	}
}

bool OpenDialog(const xr_string& id, xr_string& err)
{
	if (!EditorProject::Active() && id != "import_base_scene")
	{
		err = "no project is open";
		return false;
	}
	if (id == "package_release")			EditorMod::RequestPackageRelease();
	else if (id == "build_to_folder")		EditorMod::RequestExportModule();
	else if (id == "export_flat")			EditorMod::RequestExportFlat();
	else if (id == "import_base_scene")		EditorProject::RequestImportScene();
	else if (id == "publish_release")
	{
		// the dialog itself lists what stands in the way (repository, version, tag, account, a locked key)
		if (!EditorPublish::CanPublish())
		{
			err = "no project is open, or a release is being started";
			return false;
		}
		EditorPublish::RequestPublish();
	}
	else if (id == "submit")
	{
		if (!EditorCatalogSubmit::CanRequest())
		{
			err = "Submit to Mod Browser is not available for this project (a linked game and a module id are needed)";
			return false;
		}
		EditorCatalogSubmit::Request();
	}
	else
	{
		err = "unknown dialog '" + id + "' (ui_contract lists them)";
		return false;
	}
	return true;
}

// ---- unsaved work: what window_control close and project_close settle first ------------------------------------
struct SDirty
{
	xr_string id, label;
};

void DirtyTabs(xr_vector<SDirty>& out)
{
	out.clear();
	xr_vector<XUI::Shell::DocInfo> tabs;
	XUI::Shell::List(tabs);
	for (const XUI::Shell::DocInfo& t : tabs)
		if (t.dirty)
			out.push_back({ t.id, t.label });
}

void AppendDirtyJson(xr_string& out, const xr_vector<SDirty>& dirty)
{
	out += "[";
	for (u32 i = 0; i < dirty.size(); ++i)
	{
		out += i ? ",{\"id\":" : "{\"id\":";
		NqUtil::JsonString(out, dirty[i].id);
		out += ",\"label\":";
		NqUtil::JsonString(out, dirty[i].label);
		out += "}";
	}
	out += "]";
}

// save= or discard= decides what happens to unsaved tabs; neither refuses with the list ("unsaved"), except under
// -nodlg, where they are discarded as the exit prompt always answered there. False: `out` holds the error.
bool SettleUnsaved(LPCSTR raw, xr_string& out, xr_string& done)
{
	const bool save = NqUtil::ArgBool(raw, "save", false);
	bool discard = NqUtil::ArgBool(raw, "discard", false);
	if (save && discard)
	{
		NqUtil::JsonError(out, "save and discard exclude each other");
		return false;
	}
	xr_vector<SDirty> dirty;
	DirtyTabs(dirty);
	// nothing unsaved: nothing to report
	done.clear();
	if (dirty.empty())
		return true;
	done = save ? ",\"saved\":" : ",\"discarded\":";
	if (!save && !discard)
	{
		if (!XUI::Unattended())
		{
			out = "{\"ok\":false,\"error\":\"unsaved\",\"dirty\":";
			AppendDirtyJson(out, dirty);
			out += ",\"note\":\"pass save=true or discard=true\"}";
			return false;
		}
		discard = true;
		done = ",\"discarded\":";
	}
	for (const SDirty& d : dirty)
	{
		xr_string err;
		if (discard)
			XUI::Shell::DiscardTab(d.id.c_str());
		else if (d.id == "level")
		{
			// the scene's own save, synchronous and with its reason when it fails
			if (!Tools || Tools->m_LastFileName.empty())
				err = "the scene has no file yet: save_scene file=<path> first";
			else
				LevelMcpSaveScene(xr_string(Tools->m_LastFileName.c_str()), err);
		}
		else
			XUI::Shell::SaveTab(d.id.c_str(), &err);
		if (!err.empty())
		{
			NqUtil::JsonError(out, d.label + ": " + err);
			return false;
		}
	}
	AppendDirtyJson(done, dirty);
	return true;
}

// ---- preferences --------------------------------------------------------------------------------------------------
enum EPrefType { prefFloat, prefU32, prefBool, prefAngle, prefColor };
struct SPref
{
	LPCSTR key;
	EPrefType type;
	void* (*field)();
	float lo, hi;
};

const SPref kPrefs[] =
{
	{ "active_idle_fps",		prefU32,	[]() -> void* { return &EPrefs->active_idle_fps; },		float(kEditorIdleFpsMinimum), float(kEditorIdleFpsMaximum) },
	{ "floating_panels",		prefBool,	[]() -> void* { return &EPrefs->ui_floating_panels; },		0.f, 1.f },
	{ "view_np",				prefFloat,	[]() -> void* { return &EPrefs->view_np; },				.01f, 10.f },
	{ "view_fp",				prefFloat,	[]() -> void* { return &EPrefs->view_fp; },				10.f, 10000.f },
	{ "view_fov",				prefAngle,	[]() -> void* { return &EPrefs->view_fov; },				.1f, 170.f },
	{ "fog_color",				prefColor,	[]() -> void* { return &EPrefs->fog_color; },				0.f, 0.f },
	{ "fog_fogness",			prefFloat,	[]() -> void* { return &EPrefs->fog_fogness; },			0.f, 100.f },
	{ "cam_fly_speed",			prefFloat,	[]() -> void* { return &EPrefs->cam_fly_speed; },			.01f, 100.f },
	{ "cam_fly_alt",			prefFloat,	[]() -> void* { return &EPrefs->cam_fly_alt; },			0.f, 1000.f },
	{ "cam_sens_rot",			prefFloat,	[]() -> void* { return &EPrefs->cam_sens_rot; },			.01f, 10.f },
	{ "cam_sens_move",			prefFloat,	[]() -> void* { return &EPrefs->cam_sens_move; },			.01f, 10.f },
	{ "tools_sens_move",		prefFloat,	[]() -> void* { return &EPrefs->tools_sens_move; },		.01f, 10.f },
	{ "tools_sens_rot",			prefFloat,	[]() -> void* { return &EPrefs->tools_sens_rot; },			.01f, 10.f },
	{ "tools_sens_scale",		prefFloat,	[]() -> void* { return &EPrefs->tools_sens_scale; },		.01f, 10.f },
	{ "tools_show_move_axis",	prefBool,	[]() -> void* { return &EPrefs->tools_show_move_axis; },	0.f, 1.f },
	{ "bp_lim_depth",			prefBool,	[]() -> void* { return &EPrefs->bp_lim_depth; },			0.f, 1.f },
	{ "bp_cull",				prefBool,	[]() -> void* { return &EPrefs->bp_cull; },				0.f, 1.f },
	{ "bp_depth_tolerance",		prefFloat,	[]() -> void* { return &EPrefs->bp_depth_tolerance; },		0.f, 10000.f },
	{ "snap_angle_step",		prefAngle,	[]() -> void* { return &EPrefs->snap_angle; },			.1f, 180.f },
	{ "snap_grid_step",			prefFloat,	[]() -> void* { return &EPrefs->snap_move; },				.01f, 1000.f },
	{ "snap_scale_step",		prefFloat,	[]() -> void* { return &EPrefs->snap_scale; },			.01f, 1000.f },
	{ "snap_moveto",			prefFloat,	[]() -> void* { return &EPrefs->snap_moveto; },			.01f, 1000.f },
	{ "grid_cell_size",			prefFloat,	[]() -> void* { return &EPrefs->grid_cell_size; },			.1f, 10.f },
	{ "grid_cell_count",		prefU32,	[]() -> void* { return &EPrefs->grid_cell_count; },		10.f, 1000.f },
	{ "scene_undo_level",		prefU32,	[]() -> void* { return &EPrefs->scene_undo_level; },		0.f, 125.f },
	{ "scene_recent_count",		prefU32,	[]() -> void* { return &EPrefs->scene_recent_count; },		0.f, 25.f },
	{ "scene_clear_color",		prefColor,	[]() -> void* { return &EPrefs->scene_clear_color; },		0.f, 0.f },
	{ "render_radius",			prefFloat,	[]() -> void* { return &EDevice->RadiusRender; },			10.f, 100000.f },
};

void AppendPrefValue(xr_string& out, const SPref& p)
{
	char tmp[64];
	void* f = p.field();
	switch (p.type)
	{
	case prefFloat:	sprintf_s(tmp, "%g", *(float*)f);				break;
	case prefAngle:	sprintf_s(tmp, "%g", rad2deg(*(float*)f));		break;
	case prefU32:	sprintf_s(tmp, "%u", *(u32*)f);					break;
	case prefBool:	sprintf_s(tmp, "%s", *(BOOL*)f ? "true" : "false");	break;
	case prefColor:	sprintf_s(tmp, "\"0x%08X\"", *(u32*)f);			break;
	}
	out += tmp;
}

// false with the reason when the value does not fit the key
bool SetPref(const SPref& p, const xr_string& value, xr_string& err)
{
	void* f = p.field();
	if (p.type == prefBool)
	{
		if (value != "true" && value != "false" && value != "1" && value != "0")
		{
			err = "a true or false value";
			return false;
		}
		*(BOOL*)f = value == "true" || value == "1";
		return true;
	}
	if (p.type == prefColor)
	{
		const char* s = value.c_str();
		if (*s == '#')
			++s;
		else if (s[0] == '0' && (s[1] == 'x' || s[1] == 'X'))
			s += 2;
		char* end = nullptr;
		const unsigned long c = strtoul(s, &end, 16);
		if (!*s || (end && *end))
		{
			err = "a colour as 0xAARRGGBB or #RRGGBB";
			return false;
		}
		*(u32*)f = u32(c) | (value[0] == '#' && xr_strlen(s) == 6 ? 0xff000000u : 0u);
		return true;
	}
	char* end = nullptr;
	const float v = strtof(value.c_str(), &end);
	if (value.empty() || (end && *end) || v < p.lo || v > p.hi)
	{
		char range[96];
		sprintf_s(range, "a number %g..%g", p.lo, p.hi);
		err = range;
		return false;
	}
	switch (p.type)
	{
	case prefFloat:	*(float*)f = v;				break;
	case prefAngle:	*(float*)f = deg2rad(v);	break;
	case prefU32:	*(u32*)f = u32(v);			break;
	default:									break;
	}
	return true;
}
}

// =====================================================================================================================
bool HandleShell(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
	if (0 == xr_strcmp(cmd, "scene_load_status"))
	{
		LevelMcpSceneLoadJson(out);
		return true;
	}

	if (0 == xr_strcmp(cmd, "frame_pacing"))
	{
		if (!EPrefs)
		{
			out = "{\"ok\":false,\"error\":\"editor preferences are not ready\"}";
			return true;
		}

		char action[32] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		const int requested = GetArgInt(raw, "active_idle_fps", -1);
		if (!action[0])
			xr_strcpy(action, requested >= 0 ? "set" : "get");

		if (0 == _stricmp(action, "set"))
		{
			if (requested < int(kEditorIdleFpsMinimum) || requested > int(kEditorIdleFpsMaximum))
			{
				out = "{\"ok\":false,\"error\":\"active_idle_fps must be between 30 and 240\"}";
				return true;
			}
			EPrefs->active_idle_fps = u32(requested);
			EPrefs->Save();
		}
		else if (0 == _stricmp(action, "reset"))
		{
			EPrefs->active_idle_fps = kEditorIdleFpsDefault;
			EPrefs->Save();
		}
		else if (0 != _stricmp(action, "get") && 0 != _stricmp(action, "stats"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get, stats, set, or reset\"}";
			return true;
		}

		RespondFramePacingJson(out);
		return true;
	}

	if (0 == xr_strcmp(cmd, "theme"))
	{
		if (!EPrefs)
		{
			out = "{\"ok\":false,\"error\":\"editor preferences are not ready\"}";
			return true;
		}

		char action[32] = {}, requested[64] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		GrizzlyMCP::GetArg(raw, "preset", requested, sizeof(requested));
		if (!action[0])
			xr_strcpy(action, requested[0] ? "set" : "get");

		if (0 == _stricmp(action, "set"))
		{
			GrizzlyTheme::Preset preset;
			if (!GrizzlyTheme::TryParse(requested, preset))
			{
				out = "{\"ok\":false,\"error\":\"unknown theme preset\",\"accepted\":[\"grizzly-purple\",\"graphite\"]}";
				return true;
			}
			EPrefs->SetThemePreset(u32(preset));
		}
		else if (0 == _stricmp(action, "reset"))
		{
			EPrefs->SetThemePreset(u32(GrizzlyTheme::Default()));
		}
		else if (0 != _stricmp(action, "get"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get, set, or reset\"}";
			return true;
		}

		RespondThemeJson(out);
		return true;
	}

	if (0 == xr_strcmp(cmd, "command_palette"))
	{
		char action[32] = {}, query[256] = {}, id[512] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		GrizzlyMCP::GetArg(raw, "query", query, sizeof(query));
		GrizzlyMCP::GetArg(raw, "id", id, sizeof(id));
		if (!action[0])
			xr_strcpy(action, query[0] ? "query" : "state");

		if (0 == _stricmp(action, "open"))
			CommandPalette::Open(query);
		else if (0 == _stricmp(action, "close"))
			CommandPalette::Close();
		else if (0 == _stricmp(action, "execute"))
		{
			const int command = GetArgInt(raw, "command", -1);
			const int subcommand = GetArgInt(raw, "subcommand", 0);
			const bool executed = id[0]
				? CommandPalette::Execute(id)
				: command >= 0 && subcommand >= 0 &&
					CommandPalette::Execute(u32(command), u32(subcommand));
			if (!executed)
			{
				out = "{\"ok\":false,\"error\":\"unknown or unavailable command; query the palette first\"}";
				return true;
			}
			CommandPalette::Close();
		}
		else if (0 != _stricmp(action, "query") && 0 != _stricmp(action, "state"))
		{
			out = "{\"ok\":false,\"error\":\"action must be state, query, open, close, or execute\"}";
			return true;
		}

		RespondCommandPaletteJson(out, query, GetArgInt(raw, "limit", 50));
		return true;
	}

	if (0 == xr_strcmp(cmd, "progress"))
	{
		char action[32] = {};
		GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
		if (!action[0])
			xr_strcpy(action, "get");
		if (0 == _stricmp(action, "cancel"))
		{
			if (!UI->ProgressOperationActive())
			{
				// a job that runs beside the editor
				if (!UIProgressCenter::CancelBackground())
				{
					out = "{\"ok\":false,\"error\":\"no active operation\"}";
					return true;
				}
			}
			else if (!UI->ProgressCancelable())
			{
				out = "{\"ok\":false,\"error\":\"active operation cannot be cancelled safely\"}";
				return true;
			}
			else if (UI->NeedBreak())
				ELog.Progress("Cancellation requested through MCP.");
		}
		else if (0 != _stricmp(action, "get"))
		{
			out = "{\"ok\":false,\"error\":\"action must be get or cancel\"}";
			return true;
		}
		RespondProgressJson(out);
		return true;
	}

    // ------------------------------------------------------------------------
    // The generic bridge to the editor's own command registry. Everything the
    // menus and shortcuts can do goes through ExecCommand, so exposing the
    // registry itself gives an agent the editor's full surface - including
    // commands added later, with no MCP work at all. list_commands is the
    // introspection half: ids, names and menu paths to drive exec_command with.
    // ------------------------------------------------------------------------
    if (0 == xr_strcmp(cmd, "list_commands"))
    {
        ECommandVec& cmds = GetEditorCommands();
        out = "{\"ok\":true,\"commands\":[";
        bool first = true;
        for (u32 i = 0; i < cmds.size(); ++i)
        {
            SECommand* C = cmds[i];
            if (!C) continue;
            if (!first) out += ",";
            first = false;

            // command names are menu paths ("Mod\Build Into Game")
            char head[64];
            sprintf_s(head, sizeof(head), "{\"id\":%u,\"name\":\"", i);
            out += head;
            out += JsonStr(C->Name());
            out += "\"";
            if (C->Desc() && C->Desc()[0])
            {
                out += ",\"menu\":\"";
                out += JsonEscapePath(C->Desc());
                out += "\"";
            }
            // presets beyond the implicit (0,0) one are worth listing: they are
            // the documented parameter combinations for this command
            if (C->sub_commands.size() > 1)
            {
                out += ",\"presets\":[";
                for (u32 s = 0; s < C->sub_commands.size(); ++s)
                {
                    SESubCommand* S = C->sub_commands[s];
                    if (s) out += ",";
                    out += "{\"desc\":\"";
                    out += JsonEscapePath(S->desc.c_str());
                    out += "\",\"p1\":";
                    if (S->p0.IsInteger())	{ char b[16]; sprintf_s(b, "%u", u32(S->p0)); out += b; }
                    else					{ out += "\""; out += JsonEscapePath(xr_string(S->p0).c_str()); out += "\""; }
                    out += ",\"p2\":";
                    if (S->p1.IsInteger())	{ char b[16]; sprintf_s(b, "%u", u32(S->p1)); out += b; }
                    else					{ out += "\""; out += JsonEscapePath(xr_string(S->p1).c_str()); out += "\""; }
                    out += "}";
                }
                out += "]";
            }
            out += "}";
        }
        out += "]}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "exec_command"))
    {
        ECommandVec& cmds = GetEditorCommands();

        char name[160] = {};
        GrizzlyMCP::GetArg(raw, "name", name, sizeof(name));
        // raw json: an escaped "Edit\\Undo" arrives with both slashes
        {
            char* w = name;
            for (const char* r = name; *r; ++r)
                if (!(*r == '\\' && w > name && w[-1] == '\\')) *w++ = *r;
            *w = 0;
        }
        int id = GetArgInt(raw, "id", -1);

        // a name matches either the COMMAND_* identifier or the menu path
        if (id < 0 && name[0])
            for (u32 i = 0; i < cmds.size() && id < 0; ++i)
                if (cmds[i] && (0 == _stricmp(cmds[i]->Name(), name) ||
                                0 == _stricmp(cmds[i]->Desc(), name)))
                    id = int(i);

        if (id < 0 || id >= int(cmds.size()) || !cmds[id])
        {
            out = "{\"ok\":false,\"error\":\"unknown command - see list_commands\"}";
            return true;
        }

        // p1s/p2s pass a string, p1i/p2i an integer; explicit forms, because a
        // path made of digits must never silently turn into a number
        // p1i/p2i arrive as bare JSON numbers - GetArg reads quoted strings
        // and used to walk past the number to whatever quote came next, so an
        // integer parameter silently became garbage. GetArgInt reads numbers.
        char p1s[1024] = {}, p2s[1024] = {};
        CCommandVar P1, P2;
        if (GrizzlyMCP::GetArg(raw, "p1s", p1s, sizeof(p1s)))		P1 = CCommandVar(xr_string(p1s));
        else if (int v = GetArgInt(raw, "p1i", -1); v >= 0)			P1 = CCommandVar(u32(v));
        if (GrizzlyMCP::GetArg(raw, "p2s", p2s, sizeof(p2s)))		P2 = CCommandVar(xr_string(p2s));
        else if (int v = GetArgInt(raw, "p2i", -1); v >= 0)			P2 = CCommandVar(u32(v));

        CCommandVar res = ExecCommand(u32(id), P1, P2);

        char head[64];
        sprintf_s(head, sizeof(head), "{\"ok\":true,\"id\":%d,\"name\":\"", id);
        out = head;
        out += JsonStr(cmds[id]->Name());		// menu path: carries backslashes
        out += "\",\"result\":";
        if (res.IsInteger())	{ char b[16]; sprintf_s(b, "%u", u32(res)); out += b; }
        else					{ out += "\""; out += JsonEscapePath(xr_string(res).c_str()); out += "\""; }
        out += "}";
        return true;
    }

    // layout maintenance: the arrangement lives in level_imgui.ini, which is not
    // always recoverable by hand once it is lost
    if (0 == xr_strcmp(cmd, "reset_layout"))
    {
        // scope=level (the default) rebuilds GrizzlyLevel_v2 for the window's size class and reopens its panels;
        // scope=all also forgets every split and grid width of Layout
        char scope[16] = {};
        GrizzlyMCP::GetArg(raw, "scope", scope, sizeof(scope));
        const bool all = 0 == _stricmp(scope, "all");
        if (scope[0] && !all && 0 != _stricmp(scope, "level"))
        {
            out = "{\"ok\":false,\"error\":\"scope must be level or all\"}";
            return true;
        }
        UIMainForm::ResetLayout(all);
        out = all
            ? "{\"ok\":true,\"scope\":\"all\",\"note\":\"the Level layout is rebuilt when Level draws next, and every split and grid width is back to its default\"}"
            : "{\"ok\":true,\"scope\":\"level\",\"note\":\"the Level layout is rebuilt for the window's size class when Level draws next\"}";
        return true;
    }

    if (0 == xr_strcmp(cmd, "recovery"))
    {
        char action[32] = {};
        GrizzlyMCP::GetArg(raw, "action", action, sizeof(action));
        if (!action[0])
            xr_strcpy(action, "get");

        xr_string error;
        u64 operation_id = 0;
        bool success = true;
        if (0 == _stricmp(action, "snapshot"))
            success = EditorRecovery::SnapshotNow(operation_id, error);
        else if (0 == _stricmp(action, "restore"))
            success = EditorRecovery::RequestRestore(operation_id, error);
        else if (0 == _stricmp(action, "discard"))
            success = EditorRecovery::RequestDiscard(operation_id, error);
        else if (0 != _stricmp(action, "get"))
        {
            out = "{\"ok\":false,\"error\":\"action must be get, snapshot, restore, or discard\"}";
            return true;
        }
        RespondRecoveryJson(out, success, success ? nullptr : error.c_str());
        return true;
    }

	// the window buttons' programmatic half: what a click on the caption strip does, a size in pt for layout
	// checks, and a close that settles unsaved work first (save= / discard=) instead of raising the exit prompt
	if (0 == xr_strcmp(cmd, "window_control"))
	{
		const xr_string action = Arg(raw, "action");
		HWND hwnd = EDevice->m_hWnd;
		xr_string done;
		if (action == "minimize")		::ShowWindow(hwnd, SW_MINIMIZE);
		else if (action == "maximize")	::ShowWindow(hwnd, SW_MAXIMIZE);
		else if (action == "restore")	::ShowWindow(hwnd, SW_RESTORE);
		else if (action == "resize")
		{
			float w = 0.f, h = 0.f;
			if (!NqUtil::ArgFloat(raw, "width", w) || !NqUtil::ArgFloat(raw, "height", h) || w < 320.f || h < 240.f || w > 16384.f || h > 16384.f)
			{
				NqUtil::JsonError(out, "resize takes width and height in pt, at least 320 x 240");
				return true;
			}
			// a minimized window that was maximized restores to maximized: once more brings it to a plain size
			for (int i = 0; i < 2 && (::IsZoomed(hwnd) || ::IsIconic(hwnd)); ++i)
				::ShowWindow(hwnd, SW_RESTORE);
			// the client area is what the size names: the frame, whatever it is, comes on top
			RECT window, client;
			::GetWindowRect(hwnd, &window);
			::GetClientRect(hwnd, &client);
			const int frame_w = (window.right - window.left) - client.right;
			const int frame_h = (window.bottom - window.top) - client.bottom;
			::SetWindowPos(hwnd, nullptr, 0, 0, iFloor(XUI::Pt(w) + .5f) + frame_w, iFloor(XUI::Pt(h) + .5f) + frame_h,
				SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE);
		}
		else if (action == "close")
		{
			if (!SettleUnsaved(raw, out, done))
				return true;
			::PostMessageW(hwnd, WM_CLOSE, 0, 0);
		}
		else
		{
			out = "{\"ok\":false,\"error\":\"action must be minimize, maximize, restore, resize or close\"}";
			return true;
		}
		RECT client;
		::GetClientRect(hwnd, &client);
		char tmp[256];
		sprintf_s(tmp, "{\"ok\":true,\"maximized\":%s,\"minimized\":%s,\"client\":[%d,%d],\"client_pt\":[%.0f,%.0f],\"dpi\":%.3f",
			::IsZoomed(hwnd) ? "true" : "false", ::IsIconic(hwnd) ? "true" : "false", int(client.right), int(client.bottom),
			float(client.right) / XUI::Dpi(), float(client.bottom) / XUI::Dpi(), XUI::Dpi());
		out = tmp;
		out += done;
		out += "}";
		return true;
	}

	// geometry and state of the custom window chrome, for tests and layout checks
	if (0 == xr_strcmp(cmd, "titlebar_info"))
	{
		UITitleBar::McpInfo(out);
		return true;
	}

	// ---- ui_*: every surface can be opened, closed, focused, scrolled and looked at ------------------------------
	// what the window shows: tabs, the Level panels, drawers, dialogs, popups; held for fresh frames (GrizzlyMCP)
	if (0 == xr_strcmp(cmd, "ui_list"))
	{
		out = "{\"ok\":true";
		AppendTabsJson(out);
		out += ",\"panels\":[";
		for (u32 i = 0; i < _countof(kPanelOps); ++i)
		{
			if (i)
				out += ",";
			AppendPanelJson(out, kPanelOps[i]);
		}
		out += "],\"drawer\":\"";
		out += DrawerName(XUI::Shell::OpenedDrawer());
		out += "\",\"drawers\":[";
		for (u32 i = 0; i < _countof(kDrawers); ++i)
		{
			out += i ? ",{\"id\":\"" : "{\"id\":\"";
			out += kDrawers[i].id;
			out += "\",\"window\":";
			NqUtil::JsonString(out, kDrawers[i].window);
			out += XUI::Shell::OpenedDrawer() == int(i) + 1 ? ",\"open\":true}" : ",\"open\":false}";
		}
		out += "],\"dialogs\":[";
		AppendPopupsJson(out, true);
		out += "],\"popups\":[";
		AppendPopupsJson(out, false);
		out += "]";
		const ImVec4 work = XUI::Shell::WorkRect();
		const ImVec2 origin = ImGui::GetMainViewport()->Pos;
		out += ",\"work_area\":";
		JsonRect(out, ImVec4(work.x - origin.x, work.y - origin.y, work.z, work.w));
		char tail[256];
		sprintf_s(tail, ",\"start_page\":%s,\"size_class\":\"%s\",\"layout_custom\":%s,\"viewport_maximized\":%s,\"ui_scale\":%.2f,\"dpi\":%.3f}",
			EditorProject::Active() && EditorProject::GameLinked() ? "false" : "true",
			UIMainForm::SizeClass() == UIMainForm::ClassWide ? "wide" : "compact",
			EPrefs && EPrefs->level_layout_custom ? "true" : "false", MainForm->ViewportMaximized() ? "true" : "false",
			XUI::Dpi(), XUI::Dpi());
		out += tail;
		return true;
	}

	// the major tabs: activate, open (by id, or kind and path), close (save= / discard=), cycle
	if (0 == xr_strcmp(cmd, "ui_tab"))
	{
		xr_string action = Arg(raw, "action"), id = Arg(raw, "id");
		if (action.empty())
			action = "list";
		if (action == "list")
		{
			out = "{\"ok\":true";
			AppendTabsJson(out);
			out += "}";
			return true;
		}
		if (action == "next" || action == "prev")
		{
			XUI::Shell::Cycle(action == "next" ? 1 : -1);
			AnswerActiveTab(out);
			return true;
		}
		if (action == "activate")
		{
			XUI::Shell::DocInfo info;
			if (id.empty() || !XUI::Shell::Info(id.c_str(), info))
			{
				NqUtil::JsonError(out, "no tab '" + id + "' (ui_tab action=list names them)");
				return true;
			}
			if (!XUI::Shell::Activate(id.c_str()))
			{
				NqUtil::JsonError(out, "play in editor holds the Level tab");
				return true;
			}
			AnswerActiveTab(out);
			return true;
		}
		if (action == "open")
		{
			xr_string kind = Arg(raw, "kind"), path = Arg(raw, "path"), err;
			LPCSTR note = nullptr;
			XUI::Shell::DocInfo info;
			// a tab that is there already comes up (a restored one opens its document now)
			if (!id.empty() && XUI::Shell::Info(id.c_str(), info))
			{
				if (!XUI::Shell::Activate(id.c_str()))
				{
					NqUtil::JsonError(out, "play in editor holds the Level tab");
					return true;
				}
				AnswerActiveTab(out);
				return true;
			}
			// "quest:<rel>" and the singletons' ids name what to open
			if (!id.empty())
			{
				const size_t colon = id.find(':');
				if (kind.empty())
					kind = colon == xr_string::npos ? id : id.substr(0, colon);
				if (path.empty() && colon != xr_string::npos)
					path = id.substr(colon + 1);
			}
			bool opened = true;
			if (kind.empty())
			{
				err = "pass id=, or kind= (and path= for a file)";
				opened = false;
			}
			else if (kind == "level")
				opened = XUI::Shell::Activate("level");
			else if (kind == "quest" || kind == "beh")
			{
				opened = !path.empty() && UIQuestGraph::Open(path.c_str(), &err);
				if (path.empty())
					err = "path= names the graph (project-relative)";
				if (UIQuestGraphWindow* w = opened ? UIQuestGraph::Find(path.c_str()) : nullptr)
					w->Focus();
			}
			else if (kind == "pda")
			{
				if (path.empty())
					EditorPda::ShowModuleLayout();
				else
				{
					string_path abs;
					if (EditorFileOps::ResolveInProject(path.c_str(), abs, err))
						EditorPda::Open(abs);
				}
				opened = err.empty() && EditorPda::IsOpen();
				if (opened)
					XUI::Shell::Activate("pda");
				else if (err.empty())
					err = path.empty() ? "the module uses no PDA layout: pass path=" : path + " cannot be read";
			}
			else if (kind == "model")
			{
				opened = !path.empty();
				if (opened)
					UIVisualPreview::Show(path.c_str());
				else
					err = "path= names the model (a visual, meshes-relative)";
			}
			else if (kind == "texture")
			{
				const int source = SourceOf(Arg(raw, "source"));
				opened = source >= 0 && UIImagePreview::Show(path.c_str(), source, &err);
				if (source < 0)
					err = "source must be project, editor or darf";
			}
			else if (kind == "characters")			EditorCharacters::Show();
			else if (kind == "world_rules")			EditorWorld::Show();
			else if (kind == "preferences")			ExecCommand(COMMAND_EDITOR_PREF);
			else if (kind == "light_animations")	ExecCommand(COMMAND_LIGHTANIM_EDITOR);
			else if (kind == "texture_library")		ExecCommand(COMMAND_IMAGE_EDITOR);
			else if (kind == "sound_library")		ExecCommand(COMMAND_SOUND_EDITOR);
			else if (kind == "mod_settings" || kind == "releases")
			{
				if (kind == "mod_settings")
					EditorMod::RequestEditManifest();
				else
					EditorPublish::RequestReleases();
				opened = XUI::Shell::IsOpen(kind.c_str());
				if (!opened)
					err = EditorProject::Active() ? kind + " did not open (the log says why)" : xr_string("no project is open");
			}
			else
			{
				err = "unknown kind '" + kind + "' (ui_contract lists the document kinds)";
				opened = false;
			}
			if (!opened)
			{
				NqUtil::JsonError(out, err.empty() ? xr_string("it could not be opened") : err);
				return true;
			}
			AnswerActiveTab(out, note);
			return true;
		}
		if (action == "close")
		{
			if (id.empty())
				id = XUI::Shell::Active();
			const bool save = NqUtil::ArgBool(raw, "save", false), discard = NqUtil::ArgBool(raw, "discard", false);
			XUI::Shell::DocInfo info;
			if (save && discard)
				NqUtil::JsonError(out, "save and discard exclude each other");
			else if (!XUI::Shell::Info(id.c_str(), info))
				NqUtil::JsonError(out, "no tab '" + id + "' (ui_tab action=list names them)");
			else
			{
				xr_string err;
				if (XUI::Shell::Close(id.c_str(), save ? XUI::Shell::SaveFirst : discard ? XUI::Shell::Discard : XUI::Shell::IfClean, &err))
				{
					out = "{\"ok\":true,\"closed\":";
					NqUtil::JsonString(out, id);
					AppendTabsJson(out);
					out += "}";
				}
				else if (err == "unsaved")
				{
					out = "{\"ok\":false,\"error\":\"unsaved\",\"dirty\":";
					xr_vector<SDirty> dirty;
					dirty.push_back({ info.id, info.label });
					AppendDirtyJson(out, dirty);
					out += ",\"note\":\"pass save=true or discard=true\"}";
				}
				else
					NqUtil::JsonError(out, err);
			}
			return true;
		}
		NqUtil::JsonError(out, "action must be list, activate, open, close, next or prev");
		return true;
	}

	// the Level tab's panels; opening one brings Level up with the panel in front
	if (0 == xr_strcmp(cmd, "ui_panel"))
	{
		xr_string action = Arg(raw, "action");
		const xr_string id = Arg(raw, "id");
		const SPanelOps* p = FindPanel(id);
		if (!p)
		{
			NqUtil::JsonError(out, "id must be tools, world_outliner, details, level_settings, content_browser, output_log or viewport");
			return true;
		}
		if (action.empty())
			action = "focus";
		if (action == "toggle")
			action = p->is_open() ? "close" : "open";
		LPCSTR host = "panel";
		if (action == "open" || action == "focus")
		{
			XUI::Shell::Activate("level");
			if (MainForm->ViewportMaximized() && xr_strcmp(p->id, "viewport"))
				MainForm->ToggleViewportMaximized();
			p->open();
			// no room for it in this size class: its drawer shows it
			if (!PanelHasRoom(*p))
			{
				XUI::Shell::OpenDrawer(p->drawer);
				host = "drawer";
			}
			else
				XUI::Shell::FocusWindow(PanelWindow(p->id));
		}
		else if (action == "close")
		{
			if (!p->close)
			{
				NqUtil::JsonError(out, xr_string("the ") + p->id + " stays on Level: F11 (View > Maximize Viewport) hides the panels");
				return true;
			}
			p->close();
			host = "none";
		}
		else
		{
			NqUtil::JsonError(out, "action must be open, close, focus or toggle");
			return true;
		}
		out = "{\"ok\":true,\"id\":\"";
		out += p->id;
		out += p->is_open() ? "\",\"open\":true" : "\",\"open\":false";
		out += ",\"host\":\"";
		out += host;
		out += "\",\"active_tab\":";
		NqUtil::JsonString(out, XUI::Shell::Active());
		out += "}";
		return true;
	}

	// the Content and Log drawers over the bottom of any tab; on Level with the panel on screen, the panel instead
	if (0 == xr_strcmp(cmd, "ui_drawer"))
	{
		const xr_string action = Arg(raw, "action"), id = Arg(raw, "id");
		const int which = id == "content" ? XUI::Shell::DrawerContent : id == "log" ? XUI::Shell::DrawerLog : XUI::Shell::DrawerNone;
		LPCSTR host = "drawer";
		if (action == "open")
		{
			if (which == XUI::Shell::DrawerNone)
			{
				NqUtil::JsonError(out, "id must be content or log");
				return true;
			}
			if (which == XUI::Shell::DrawerContent)
				UIContentBrowser::MakeVisible();
			else if (XUI::Shell::IsActive("level") && UILogForm::PanelOpen() && XUI::Shell::WindowShown(UILogForm::PanelName()))
				XUI::Shell::FocusWindow(UILogForm::PanelName());
			else
				XUI::Shell::OpenDrawer(XUI::Shell::DrawerLog);
			if (XUI::Shell::OpenedDrawer() != which)
				host = "panel";
		}
		else if (action == "close")
		{
			if (which == XUI::Shell::DrawerNone || XUI::Shell::OpenedDrawer() == which)
				XUI::Shell::CloseDrawer();
			host = "none";
		}
		else
		{
			NqUtil::JsonError(out, "action must be open or close");
			return true;
		}
		out = "{\"ok\":true,\"drawer\":\"";
		out += DrawerName(XUI::Shell::OpenedDrawer());
		out += "\",\"host\":\"";
		out += host;
		out += "\"}";
		return true;
	}

	// the dialogs of the contract: opened the way their menu items open them, closed the way Esc or the cross does
	if (0 == xr_strcmp(cmd, "ui_dialog"))
	{
		xr_string action = Arg(raw, "action");
		const xr_string id = Arg(raw, "id");
		if (action.empty())
			action = "list";
		if (action == "list")
		{
			out = "{\"ok\":true,\"dialogs\":[";
			AppendPopupsJson(out, true);
			out += "],\"known\":[";
			for (u32 i = 0; i < _countof(kDialogs); ++i)
			{
				out += i ? ",\"" : "\"";
				out += kDialogs[i].id;
				out += "\"";
			}
			out += "]}";
			return true;
		}
		if (id.empty())
		{
			NqUtil::JsonError(out, "id names the dialog (ui_contract lists them)");
			return true;
		}
		xr_string err;
		if (action == "open")
		{
			if (!OpenDialog(id, err))
			{
				NqUtil::JsonError(out, err);
				return true;
			}
			out = "{\"ok\":true,\"id\":";
			NqUtil::JsonString(out, id);
			out += ",\"window\":";
			NqUtil::JsonString(out, WindowOf(id));
			out += ",\"note\":\"it comes up when the editor draws next: screenshot_editor names it as modal\"}";
			return true;
		}
		if (action == "close")
		{
			const xr_string window = WindowOf(id);
			XUI::Strings kit;
			XUI::OpenDialogs(kit);
			// a kit dialog goes by what follows ### in its window's name: the contract's id for it (dlg.catalog_submit
			// for submit), else dlg.<id>
			const size_t hash = window.find("###");
			xr_string kit_name = hash != xr_string::npos ? window.substr(hash + 3) : id.compare(0, 4, "dlg.") ? "dlg." + id : id;
			if (std::find(kit.begin(), kit.end(), kit_name) == kit.end() && id.compare(0, 4, "dlg."))
				kit_name = "dlg." + id;
			const bool is_kit = std::find(kit.begin(), kit.end(), kit_name) != kit.end();
			if (!is_kit && !ModalOpen(window))
			{
				NqUtil::JsonError(out, "'" + id + "' is not open (ui_dialog action=list names what is)");
				return true;
			}
			// a kit dialog closes as if Esc was pressed; any other one the way its cross takes it
			if (is_kit)
				XUI::CloseDialog(kit_name.c_str());
			else
				ImGui::RequestPopupModalClose(window.c_str());
			out = "{\"ok\":true,\"closing\":";
			NqUtil::JsonString(out, id);
			out += ",\"note\":\"it goes when the editor draws next\"}";
			return true;
		}
		NqUtil::JsonError(out, "action must be list, open or close");
		return true;
	}

	// a popup put up to be looked at (screenshot_editor); none takes them down
	if (0 == xr_strcmp(cmd, "ui_toast"))
	{
		// to look at one: the notes the editor raises by itself come at moments an agent cannot order
		xr_string text;
		NqUtil::ArgString(raw, "text", text);
		XUI::Toast(XUI::Tone::Info, text.empty() ? "A note." : text.c_str());
		out = "{\"ok\":true}";
		return true;
	}
	if (0 == xr_strcmp(cmd, "ui_popup"))
	{
		const xr_string show = Arg(raw, "show");
		if (show.empty())
		{
			NqUtil::JsonError(out, "show=<popup> or show=none (ui_contract lists the popups)");
			return true;
		}
		if (show == "none")
		{
			XUI::Shell::ClosePopups();
			out = "{\"ok\":true,\"popup\":\"none\"}";
			return true;
		}
		const size_t colon = show.find(':');
		const xr_string kind = colon == xr_string::npos ? show : show.substr(0, colon);
		const xr_string arg = colon == xr_string::npos ? xr_string() : show.substr(colon + 1);
		xr_string err;
		if (kind == "menu")
		{
			bool known = false;
			for (LPCSTR m : kMenus)
				known |= arg == m;
			const xr_string active = XUI::Shell::Active();
			const bool graph = !active.compare(0, 6, "quest:") || !active.compare(0, 4, "beh:");
			if (!known)
				err = "no menu '" + arg + "' (ui_contract lists the menus)";
			// a tab's own menus are in the caption only while it is up
			else if (arg == "Graph" && !graph)
				err = "the Graph menu belongs to a graph tab: ui_tab activate one first";
			else if ((arg == "Scene" || arg == "Build" || arg == "View") && active != "level")
				err = "the " + arg + " menu belongs to the Level tab: ui_tab action=activate id=level first";
			else if (arg == "Menu" && !UIMainMenuForm::Folded())
				err = "the menus are folded into the Menu button only while the window is narrower than 1100 pt";
			else if (!EditorProject::Active() || !EditorProject::GameLinked())
				err = "the Start page has no menus";
		}
		else if (kind == "tab_menu")
		{
			XUI::Shell::DocInfo info;
			if (!XUI::Shell::Info(arg.c_str(), info))
				err = "no tab '" + arg + "'";
		}
		else if (kind == "cb_context" || kind == "cb_add" || kind == "cb_view" || kind == "cb_favorites" || kind == "cb_grid")
		{
			if (kind == "cb_context" && arg.empty())
				err = "cb_context:<path> names an entry as content_browser_selection lists it";
			else if (kind == "cb_add" && !UIContentBrowser::CanAdd())
				err = "the Add menu is off here: content is added to the open project (content_browser_navigation "
					"source=project first; its answer says can_add)";
			else
				UIContentBrowser::MakeVisible();
		}
		else if (kind == "outliner_filters" || kind == "outliner_more")
		{
			XUI::Shell::Activate("level");
			UIWorldOutliner::Focus(false);
		}
		else if (kind == "viewport")
		{
			bool known = false;
			for (LPCSTR m : kViewportMenus)
				known |= arg == m;
			if (!known)
				err = "no viewport menu '" + arg + "' (ui_contract lists them)";
			else
				XUI::Shell::Activate("level");
		}
		else if (kind == "toolbar_split" && arg.empty())
			err = "toolbar_split:<id> names the split button (build_mod on every tab)";
		// the status bar's chips are there only while they have something to say
		else if (kind == "tasks")
		{
			SProgressTaskInfoVec tasks;
			UIProgressCenter::Snapshot(*UI, tasks);
			if (tasks.empty())
				err = "no job runs now: the tasks chip and its card are up only while one does (progress lists them)";
		}
		else if (kind == "rebuild" && !UIMainForm::RebuildPending())
			err = "nothing needs rebuilding: the Rebuild chip is up only while the scene has changes the level builders have not seen";
		else if (kind != "tab_overflow" && kind != "toolbar_overflow" && kind != "toolbar_split" && kind != "tasks" && kind != "rebuild")
			err = "unknown popup '" + show + "' (ui_contract lists them)";
		if (!err.empty())
		{
			NqUtil::JsonError(out, err);
			return true;
		}
		// one popup looked at at a time: the one up before goes (at the next frame, before the owner puts this one up)
		XUI::Shell::ClosePopups();
		XUI::ShowPopup(show.c_str());
		out = "{\"ok\":true,\"popup\":";
		NqUtil::JsonString(out, show);
		out += ",\"note\":\"it comes up when its owner draws next; ui_popup show=none takes it down\"}";
		return true;
	}

	// a panel, a drawer, a dialog or the work area scrolled: top, bottom, pt, or item:<name> where the kit knows it
	if (0 == xr_strcmp(cmd, "ui_scroll"))
	{
		const xr_string id = Arg(raw, "id"), to = Arg(raw, "to");
		if (id.empty() || to.empty())
		{
			NqUtil::JsonError(out, "ui_scroll takes id= and to=top|bottom|<pt>|item:<name>");
			return true;
		}
		if (!to.compare(0, 5, "item:"))
		{
			XUI::RequestScroll(id.c_str(), to.c_str());
			out = "{\"ok\":true,\"id\":";
			NqUtil::JsonString(out, id);
			out += ",\"pending\":true,\"note\":\"a panel built on the kit scrolls to the item when it draws next\"}";
			return true;
		}
		const xr_string window = WindowOf(id);
		XUI::Shell::WindowState ws;
		if (!XUI::Shell::GetWindowState(window.c_str(), ws) || !ws.shown)
		{
			NqUtil::JsonError(out, "'" + id + "' is not on screen");
			return true;
		}
		if (!XUI::Shell::ScrollWindow(window.c_str(), to.c_str()))
		{
			NqUtil::JsonError(out, "to must be top, bottom, a number of pt or item:<name>");
			return true;
		}
		out = "{\"ok\":true,\"id\":";
		NqUtil::JsonString(out, id);
		out += ",\"window\":";
		NqUtil::JsonString(out, window);
		char tail[160];
		sprintf_s(tail, ",\"scroll_y\":%.0f,\"scroll_max_y\":%.0f,\"dpi\":%.3f,\"note\":\"px, as the last frame left them: it moves at the next\"}",
			ws.scroll_y, ws.scroll_max_y, XUI::Dpi());
		out += tail;
		return true;
	}

	// the splits and grid widths the layout keeps (pt); with no key, every one
	if (0 == xr_strcmp(cmd, "ui_split"))
	{
		const xr_string key = Arg(raw, "key");
		if (key.empty())
		{
			XUI::Strings keys;
			XUI::Layout::Keys(keys);
			out = "{\"ok\":true,\"splits\":{";
			for (u32 i = 0; i < keys.size(); ++i)
			{
				if (i)
					out += ",";
				NqUtil::JsonString(out, keys[i]);
				out += NqUtil::Format(":%g", XUI::Layout::Get(keys[i].c_str(), 0.f));
			}
			out += "},\"unit\":\"pt\"}";
			return true;
		}
		float value = 0.f;
		if (NqUtil::ArgFloat(raw, "value", value))
			XUI::Layout::Set(key.c_str(), value);
		else if (!XUI::Layout::Has(key.c_str()))
		{
			NqUtil::JsonError(out, "no split '" + key + "' (ui_split with no key lists them)");
			return true;
		}
		out = "{\"ok\":true,\"key\":";
		NqUtil::JsonString(out, key);
		out += NqUtil::Format(",\"value\":%g,\"unit\":\"pt\"}", XUI::Layout::Get(key.c_str(), 0.f));
		return true;
	}

	// what the last frames truncated, let escape and offered; held for fresh frames (GrizzlyMCP)
	if (0 == xr_strcmp(cmd, "ui_audit"))
	{
		const xr_string id = Arg(raw, "id");
		xr_string audit;
		XUI::AuditJson(audit, id.empty() ? nullptr : WindowOf(id).c_str());
		out = "{\"ok\":true,";
		out += audit.substr(1);
		return true;
	}

	// the UI scale, for tests at another DPI: applied at once, never saved
	if (0 == xr_strcmp(cmd, "ui_scale"))
	{
		float value = 0.f;
		if (NqUtil::ArgFloat(raw, "value", value))
		{
			if (value < kUiScaleMin || value > kUiScaleMax)
			{
				NqUtil::JsonError(out, NqUtil::Format("value must be %g..%g", kUiScaleMin, kUiScaleMax));
				return true;
			}
			XUI::SetDpi(value);
		}
		out = NqUtil::Format("{\"ok\":true,\"ui_scale\":%.2f,\"saved\":false,\"note\":\"tests only: the next start takes the monitor's scale again\"}",
			value > 0.f ? value : XUI::Dpi());
		return true;
	}

	// the constants QA pins (EditorUiContract.h), plus the lists their owners keep
	if (0 == xr_strcmp(cmd, "ui_contract"))
	{
		out = NqUtil::Format("{\"ok\":true,\"tile_range\":[%g,%g],\"tile_default\":%g,\"tile_default_panel\":%g,\"view_default\":\"tiles\",\"view_default_panel\":\"list\",\"tile_error\":",
			kTileMin, kTileMax, kTileDefault, kTileDefaultPanel);
		NqUtil::JsonString(out, kTileRangeError);
		out += ",\"theme_keys\":[";
		for (u32 i = 0; i < u32(GrizzlyTheme::Preset::Count); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, GrizzlyTheme::Key(GrizzlyTheme::Preset(i)));
		}
		out += "],\"categories\":[";
		for (int i = 0; i < UIContentBrowser::CategoryCount(); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, UIContentBrowser::CategoryName(i));
		}
		out += "],\"card_menu\":[";
		NqUtil::JsonString(out, kCardShowInExplorer);
		out += ",";
		NqUtil::JsonString(out, kCardDelete);
		out += ",";
		NqUtil::JsonString(out, kCardRemoveFromList);
		out += "],\"delete_project_dialog\":";
		NqUtil::JsonString(out, kDeleteProjectDialog);
		out += ",\"panels\":{";
		for (u32 i = 0; i < _countof(kPanels); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kPanels[i].id);
			out += ":";
			NqUtil::JsonString(out, kPanels[i].window);
		}
		out += "},\"drawers\":{";
		for (u32 i = 0; i < _countof(kDrawers); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kDrawers[i].id);
			out += ":";
			NqUtil::JsonString(out, kDrawers[i].window);
		}
		out += "},\"dialogs\":{";
		for (u32 i = 0; i < _countof(kDialogs); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kDialogs[i].id);
			out += ":";
			NqUtil::JsonString(out, kDialogs[i].window);
		}
		out += "},\"document_kinds\":[";
		for (u32 i = 0; i < _countof(kDocumentKinds); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kDocumentKinds[i]);
		}
		out += "],\"popups\":[";
		for (u32 i = 0; i < _countof(kPopups); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kPopups[i]);
		}
		out += "],\"menus\":[";
		for (u32 i = 0; i < _countof(kMenus); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kMenus[i]);
		}
		out += "],\"viewport_menus\":[";
		for (u32 i = 0; i < _countof(kViewportMenus); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kViewportMenus[i]);
		}
		out += "],\"kit_dialogs\":[";
		for (u32 i = 0; i < _countof(kKitDialogs); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kKitDialogs[i]);
		}
		out += "],\"palette\":";
		NqUtil::JsonString(out, kPaletteId);
		out += NqUtil::Format(",\"ui_scale_range\":[%g,%g]}", kUiScaleMin, kUiScaleMax);
		return true;
	}

	// back to the Start page: unsaved work settled first (save= / discard=), then the scene and the project close
	if (0 == xr_strcmp(cmd, "project_close"))
	{
		if (!EditorProject::Active())
		{
			NqUtil::JsonError(out, "no project is open: the Start page is up");
			return true;
		}
		xr_string done;
		if (!SettleUnsaved(raw, out, done))
			return true;
		// the Start page's picture of the project, while the scene is still up
		EditorProject::CapturePreview();
		if (!(BOOL)ExecCommand(COMMAND_CLEAR))
		{
			NqUtil::JsonError(out, "the scene could not be closed (the log says why)");
			return true;
		}
		// the project's tabs are written to its session and close with it: they come back when it opens again
		XUI::Shell::CloseProjectTabs();
		EditorProject::Close();
		out = "{\"ok\":true,\"start_page\":true";
		out += done;
		out += "}";
		return true;
	}

	// the editor's settings by their .ini names: applied at once and saved, like the Preferences tab does
	if (0 == xr_strcmp(cmd, "preferences"))
	{
		if (!EPrefs)
		{
			NqUtil::JsonError(out, "editor preferences are not ready");
			return true;
		}
		xr_string action = Arg(raw, "action");
		const xr_string key = Arg(raw, "key");
		if (action.empty())
			action = "get";
		if (action != "get" && action != "set" && action != "show")
		{
			NqUtil::JsonError(out, "action must be get, set or show");
			return true;
		}
		// the Preferences tab on a page (a top-level folder of its rows, or Keyboard), with its search
		if (action == "show")
		{
			const xr_string page = Arg(raw, "page"), search = Arg(raw, "search");
			EPrefs->Edit();
			if (!page.empty() || !search.empty())
				EPrefs->ShowPage(page.empty() ? EPrefs->Page() : page.c_str(), search.c_str());
			out = "{\"ok\":true,\"page\":";
			NqUtil::JsonString(out, EPrefs->Page());
			out += ",\"active_tab\":";
			NqUtil::JsonString(out, XUI::Shell::Active());
			out += "}";
			return true;
		}
		const SPref* pref = nullptr;
		for (const SPref& p : kPrefs)
			if (key == p.key)
				pref = &p;
		if (!key.empty() && !pref)
		{
			NqUtil::JsonError(out, "no preference '" + key + "' (preferences action=get lists them)");
			return true;
		}
		if (action == "set")
		{
			xr_string value, err;
			if (!pref || !NqUtil::ArgString(raw, "value", value))
			{
				NqUtil::JsonError(out, "set takes key= and value=");
				return true;
			}
			if (!SetPref(*pref, value, err))
			{
				NqUtil::JsonError(out, xr_string(pref->key) + " takes " + err);
				return true;
			}
			EPrefs->ApplyAndSave();
		}
		if (pref)
		{
			out = "{\"ok\":true,\"key\":";
			NqUtil::JsonString(out, pref->key);
			out += ",\"value\":";
			AppendPrefValue(out, *pref);
			if (pref->type == prefAngle)
				out += ",\"unit\":\"deg\"";
			if (!xr_strcmp(pref->key, "floating_panels"))
				out += ",\"note\":\"takes effect after a restart\"";
			out += "}";
			return true;
		}
		out = "{\"ok\":true,\"preferences\":{";
		for (u32 i = 0; i < _countof(kPrefs); ++i)
		{
			if (i)
				out += ",";
			NqUtil::JsonString(out, kPrefs[i].key);
			out += ":";
			AppendPrefValue(out, kPrefs[i]);
		}
		out += "},\"page\":";
		NqUtil::JsonString(out, EPrefs->Page());
		out += ",\"angles\":\"deg\",\"theme\":\"see the theme command\"}";
		return true;
	}

	return false;
}
}

// The Level panels and the drawers as the kit's surfaces: a panel drawn through XUI::BeginPanel under its window
// name takes the item:<name> scrolls MCP asks for, and the audit and SurfaceControl know whose windows they are.
void LevelMcpRegisterSurfaces()
{
	using namespace LevelMcp;
	for (const SPanelOps& p : kPanelOps)
	{
		const SPanelOps* ops = &p;
		XUI::SurfaceDesc d;
		d.id = p.id;
		d.label = PanelWindow(p.id);
		d.kind = XUI::SurfacePanel;
		d.control = [ops](int op)
		{
			const bool open = op == XUI::SurfaceOpen || op == XUI::SurfaceFocus || (op == XUI::SurfaceToggle && !ops->is_open());
			if (!open)
			{
				if (ops->close)
					ops->close();
				return;
			}
			XUI::Shell::Activate("level");
			if (MainForm->ViewportMaximized() && xr_strcmp(ops->id, "viewport"))
				MainForm->ToggleViewportMaximized();
			ops->open();
			// no room for it in this size class: its drawer shows it, as ui_panel does
			if (!PanelHasRoom(*ops))
				XUI::Shell::OpenDrawer(ops->drawer);
			else
				XUI::Shell::FocusWindow(PanelWindow(ops->id));
		};
		d.is_open = [ops] { return ops->is_open(); };
		XUI::RegisterSurface(d);
	}
	for (u32 i = 0; i < _countof(EditorUiContract::kDrawers); ++i)
	{
		const int which = int(i) + 1;
		XUI::SurfaceDesc d;
		d.id = EditorUiContract::kDrawers[i].id;
		d.label = EditorUiContract::kDrawers[i].window;
		d.kind = XUI::SurfaceDrawer;
		d.control = [which](int op)
		{
			const bool open = op == XUI::SurfaceOpen || op == XUI::SurfaceFocus || (op == XUI::SurfaceToggle && XUI::Shell::OpenedDrawer() != which);
			if (open)
				XUI::Shell::OpenDrawer(which);
			else if (XUI::Shell::OpenedDrawer() == which)
				XUI::Shell::CloseDrawer();
		};
		d.is_open = [which] { return XUI::Shell::OpenedDrawer() == which; };
		XUI::RegisterSurface(d);
	}
}

// what GrizzlyMCP hands the editor: every group in turn, the ones that need the scene checking it themselves
bool GrizzlyInspector(LPCSTR cmd, LPCSTR raw, xr_string& out)
{
	using namespace LevelMcp;
	return HandleShell(cmd, raw, out) || HandleProps(cmd, raw, out) || HandleMod(cmd, raw, out) ||
		HandlePreview(cmd, raw, out) || HandleContent(cmd, raw, out) || HandleScene(cmd, raw, out) ||
		HandleGraph(cmd, raw, out);
}
