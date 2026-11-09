#include "stdafx.h"
#include "UIQuestGraph.h"
#include "NqCanvas.h"
#include "NqInspector.h"
#include "../../../XrECore/Editor/Nq/NqUtil.h"
#include "../../../XrECore/Editor/Nq/NqCatalog.h"
#include "../../../XrECore/Editor/Nq/NqLayout.h"
#include "../../../XrECore/Editor/Nq/NqLua.h"

namespace
{
	using T = GrizzlyTheme::ColorToken;

	xr_vector<UIQuestGraphWindow*> s_Windows;
	xr_string s_LastProject;

	// pane sizes in pt, kept in the layout (the ini's [GrizzlyLayout] data) like every split
	LPCSTR const kGraphWidth	= "graph.panel_w";
	LPCSTR const kDetailsWidth	= "graph.details_w";
	LPCSTR const kDrawerHeight	= "graph.drawer_h";
	const float kGraphDef = 280.f, kGraphMin = 220.f, kGraphMax = 480.f;
	const float kDetailsDef = 360.f, kDetailsMin = 280.f, kDetailsMax = 640.f;
	const float kDrawerDef = 200.f, kDrawerMin = 120.f;
	const float kCanvasMin = 400.f, kCanvasMinH = 260.f;
	const float kRail = 36.f;
	const float kGap = 6.f;			// between two panes: the 1 pt line and its grip
	const float kExternalPoll = 1.0;	// s between two looks at the file on disk

	// Match the picker search for both ASCII and Cyrillic UTF-8 text.
	void FoldSearch(LPCSTR text, xr_string& out)
	{
		out.clear();
		if (!text) return;
		for (const u8* p = (const u8*)text; *p; ++p)
		{
			const u8 c = *p;
			if (c >= 'A' && c <= 'Z') { out += char(c + 0x20); continue; }
			if (c == 0xD0 && p[1])
			{
				const u8 d = p[1];
				if (d == 0x81)				{ out += char(0xD1); out += char(0x91); ++p; continue; }
				if (d >= 0x90 && d <= 0x9F)	{ out += char(0xD0); out += char(d + 0x20); ++p; continue; }
				if (d >= 0xA0 && d <= 0xAF)	{ out += char(0xD1); out += char(d - 0x20); ++p; continue; }
			}
			out += char(c);
		}
	}

	struct SSearchTerm
	{
		xr_string	text;
		bool		exclude;
	};

	// terms separated by spaces; "-term" excludes, a quoted phrase is one term
	void ParseSearch(LPCSTR query, xr_vector<SSearchTerm>& out)
	{
		out.clear();
		const char* p = query ? query : "";
		while (*p)
		{
			while (*p && (u8)*p <= 0x20) ++p;
			if (!*p) break;
			SSearchTerm term;
			term.exclude = *p == '-' && p[1] && (u8)p[1] > 0x20;
			if (term.exclude) ++p;
			const bool quoted = *p == '"';
			if (quoted) ++p;
			const char* begin = p;
			if (quoted) while (*p && *p != '"') ++p;
			else while (*p && (u8)*p > 0x20) ++p;
			xr_string raw;
			raw.assign(begin, p - begin);
			if (quoted && *p == '"') ++p;
			FoldSearch(raw.c_str(), term.text);
			if (!term.text.empty()) out.push_back(term);
		}
	}

	bool SearchMatches(LPCSTR text, const xr_vector<SSearchTerm>& terms)
	{
		xr_string folded;
		FoldSearch(text, folded);
		for (u32 i = 0; i < terms.size(); ++i)
		{
			const bool found = strstr(folded.c_str(), terms[i].text.c_str());
			if ((terms[i].exclude && found) || (!terms[i].exclude && !found)) return false;
		}
		return !terms.empty();
	}

	bool SearchFieldHit(LPCSTR text, const xr_vector<SSearchTerm>& terms)
	{
		xr_string folded;
		FoldSearch(text, folded);
		for (u32 i = 0; i < terms.size(); ++i)
			if (!terms[i].exclude && strstr(folded.c_str(), terms[i].text.c_str())) return true;
		return false;
	}

	float LayoutPt(LPCSTR key, float def, float lo, float hi)
	{
		return _max(lo, _min(XUI::Layout::Get(key, def), hi));
	}

	// a pane of the tab: its own padding, the base fill
	bool BeginPane(LPCSTR id, const ImVec2& at, const ImVec2& size, T fill = T::Base, bool border = false, ImGuiChildFlags flags = 0)
	{
		ImGui::SetCursorScreenPos(at);
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(8.f), XUI::Pt(8.f)));
		ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::Vec4(fill));
		const bool open = ImGui::BeginChild(id, size, ImGuiChildFlags_AlwaysUseWindowPadding | (border ? ImGuiChildFlags_Borders : 0) | flags);
		ImGui::PopStyleColor();
		ImGui::PopStyleVar();
		return open;
	}

	xr_string Plural(int n, LPCSTR one, LPCSTR many)
	{
		return NqUtil::Format("%d %s", n, n == 1 ? one : many);
	}
}

//------------------------------------------------------------------------------
// window
//------------------------------------------------------------------------------
UIQuestGraphWindow::UIQuestGraphWindow(NqDoc* doc) : m_Doc(doc)
{
	m_Canvas = xr_new<NqCanvas>(doc);
	m_Inspector = xr_new<NqInspector>(doc, m_Canvas);
	m_ShowGraph = m_ShowDetails = true;
	m_ShowKeys = false;
	m_GraphOverlay = m_Rail = m_PendingRename = false;
	m_Drawer = DrawerNone;
	m_PaneGraph = m_PaneCanvas = m_PaneDetails = m_PaneDrawer = ImVec4(0, 0, 0, 0);
	m_ViewFrames = 0;
	m_DrawnFrame = -10;
	m_External = false;
	m_ExternalAt = -DBL_MAX;
	m_MessageTone = XUI::Tone::Muted;
	m_FocusFind = false;
	m_Find[0] = 0;
	m_FindIndex = -1;
	m_FindRevision = u32(-1);
	m_FindCatalogGeneration = u32(-1);
	m_ProjectFindOpen = m_FocusProjectFind = false;
	m_ProjectFind[0] = 0;
	m_ProjectFindComplete = true;
	m_ProjectFindGeneration = 0;
	m_ProjectFindFingerprint = 0;
}

UIQuestGraphWindow::~UIQuestGraphWindow()
{
	xr_delete(m_Inspector);
	xr_delete(m_Canvas);
}

LPCSTR UIQuestGraphWindow::Noun() const
{
	return NqLook::DocNoun(m_Doc->quest.IsBehaviour());
}

void UIQuestGraphWindow::Focus()
{
	XUI::Shell::Activate(UIQuestGraph::DocId(m_Doc->path.c_str(), m_Doc->quest.IsBehaviour()).c_str());
}

void UIQuestGraphWindow::SetMessage(LPCSTR text, XUI::Tone tone)
{
	m_Message = text ? text : "";
	m_MessageTone = tone;
}

void UIQuestGraphWindow::Save()
{
	xr_string err;
	if (m_Doc->Save(false, err)) { SetMessage("Saved", XUI::Tone::Success); m_External = false; }
	else if (err == "modified externally") { SetMessage("Not saved: the file changed on disk", XUI::Tone::Warning); m_External = true; }
	else SetMessage(("Save failed: " + err).c_str(), XUI::Tone::Error);
}

void UIQuestGraphWindow::PollExternal()
{
	const double now = ImGui::GetTime();
	if (now - m_ExternalAt < kExternalPoll) return;
	m_ExternalAt = now;
	m_External = m_Doc->FileExists() && m_Doc->ModifiedExternally();
}

void UIQuestGraphWindow::Reload()
{
	const xr_string abs = m_Doc->path;
	auto run = [abs]()
	{
		UIQuestGraphWindow* w = UIQuestGraph::Find(abs.c_str());
		if (!w) return;
		xr_string err;
		if (w->m_Doc->Reload(err)) { w->SetMessage("Reloaded", XUI::Tone::Success); w->m_External = false; }
		else w->SetMessage(("Reload failed: " + err).c_str(), XUI::Tone::Error);
	};
	if (!m_Doc->dirty) { run(); return; }
	XUI::ConfirmSpec spec;
	spec.id = "nq_reload";
	spec.title = "Reload";
	spec.headline = "Read the file again and drop the unsaved changes?";
	spec.items.push_back(NqUtil::ProjectRelative(abs.c_str()));
	spec.consequence = "The unsaved changes are lost - this cannot be undone.";
	spec.action = "Reload";
	spec.destructive = true;
	XUI::Confirm(spec, [run](XUI::Answer a) { if (a == XUI::Answer::Yes) run(); });
}

void UIQuestGraphWindow::Overwrite()
{
	const xr_string abs = m_Doc->path;
	XUI::ConfirmSpec spec;
	spec.id = "nq_overwrite";
	spec.title = "Overwrite";
	spec.headline = "Replace the file on disk with the graph as it is here?";
	spec.items.push_back(NqUtil::ProjectRelative(abs.c_str()));
	spec.consequence = "The changes made to the file on disk are lost - this cannot be undone.";
	spec.action = "Overwrite";
	spec.destructive = true;
	XUI::Confirm(spec, [abs](XUI::Answer a)
	{
		if (a != XUI::Answer::Yes) return;
		UIQuestGraphWindow* w = UIQuestGraph::Find(abs.c_str());
		if (!w) return;
		xr_string err;
		if (w->m_Doc->Save(true, err)) { w->SetMessage("Saved over the file on disk", XUI::Tone::Success); w->m_External = false; }
		else w->SetMessage(("Save failed: " + err).c_str(), XUI::Tone::Error);
	});
}

void UIQuestGraphWindow::Validate()
{
	m_Doc->Validate();
	// the problems are what a Validate is for: they come up when there are any
	if (m_Doc->ErrorCount() || m_Doc->WarningCount()) m_Drawer = DrawerProblems;
	SetMessage(m_Doc->ErrorCount() ? "Validated: errors found" : m_Doc->WarningCount() ? "Validated: warnings found" : "Validated: no problems",
		m_Doc->ErrorCount() ? XUI::Tone::Error : m_Doc->WarningCount() ? XUI::Tone::Warning : XUI::Tone::Success);
}

void UIQuestGraphWindow::OpenFind()
{
	m_Drawer = DrawerFind;
	m_FocusFind = true;
}

void UIQuestGraphWindow::OpenProjectFind()
{
	if (!m_ProjectFind[0] && m_Find[0]) strncpy_s(m_ProjectFind, sizeof(m_ProjectFind), m_Find, _TRUNCATE);
	m_ProjectFindOpen = true;
	m_FocusProjectFind = true;
	RefreshProjectFind();
}

void UIQuestGraphWindow::SetDrawer(int which)
{
	m_Drawer = (which == DrawerProblems || which == DrawerFind) ? which : DrawerNone;
}

LPCSTR UIQuestGraphWindow::DrawerName() const
{
	return m_Drawer == DrawerProblems ? "problems" : m_Drawer == DrawerFind ? "find" : "none";
}

void UIQuestGraphWindow::SetGraphPanel(bool on)
{
	// on a rail the panel is the one over the canvas: "hide" folds it back to the rail, which stays
	if (m_Rail && m_ShowGraph) { m_GraphOverlay = on; return; }
	m_ShowGraph = on;
	m_GraphOverlay = on && m_Rail;
}

LPCSTR UIQuestGraphWindow::GraphPanelState() const
{
	if (!m_ShowGraph) return "hidden";
	if (!m_Rail) return "open";
	return m_GraphOverlay ? "overlay" : "rail";
}

void UIQuestGraphWindow::SetGraphWidth(float pt)
{
	XUI::Layout::Set(kGraphWidth, _max(kGraphMin, _min(pt, kGraphMax)));
}

float UIQuestGraphWindow::GraphWidth() const
{
	return LayoutPt(kGraphWidth, kGraphDef, kGraphMin, kGraphMax);
}

void UIQuestGraphWindow::SetDetailsWidth(float pt)
{
	XUI::Layout::Set(kDetailsWidth, _max(kDetailsMin, _min(pt, kDetailsMax)));
}

float UIQuestGraphWindow::DetailsWidth() const
{
	return m_PaneDetails.z > 0.f ? m_PaneDetails.z / XUI::Dpi() : LayoutPt(kDetailsWidth, kDetailsDef, kDetailsMin, kDetailsMax);
}

bool UIQuestGraphWindow::CanScroll(LPCSTR where, xr_string& why) const
{
	const xr_string w = where ? where : "";
	const bool graph = m_ShowGraph && (!m_Rail || m_GraphOverlay);
	if (w == "tasks" || w == "variables" || !w.compare(0, 5, "task:"))
	{
		if (!graph) { why = "the Graph panel is not on screen: quest_view graph_panel=show first"; return false; }
		if (!w.compare(0, 5, "task:"))
		{
			if (m_Doc->quest.IsBehaviour()) { why = "a behaviour graph has roles, not tasks"; return false; }
			xr_string id = w.substr(5);
			const size_t slash = id.find('/');
			if (slash != xr_string::npos) id.resize(slash);
			if (!m_Doc->quest.FindTask(id.c_str())) { why = "no task '" + id + "' in this quest"; return false; }
		}
		return true;
	}
	// top and bottom: the selected node's pane, or with nothing selected the Graph panel
	if (w == "top" || w == "bottom")
	{
		if (m_Doc->selection.size() == 1)
		{
			if (!m_ShowDetails) { why = "the Details panel is hidden: quest_view details=show first"; return false; }
			return true;
		}
		if (!graph) { why = "nothing is selected and the Graph panel is not on screen"; return false; }
		return true;
	}
	if (!w.compare(0, 7, "action:"))
	{
		if (!m_ShowDetails) { why = "the Details panel is hidden: quest_view details=show first"; return false; }
		if (!m_Inspector->HasAction()) { why = "no action is open: pass slot=enter:N or slot=exit:N"; return false; }
		return true;
	}
	why = "scroll must be top, bottom, action:top, action:bottom, tasks, task:<id>, task:<id>/steps or variables";
	return false;
}

void UIQuestGraphWindow::SetFindQuery(LPCSTR query)
{
	strncpy_s(m_Find, sizeof(m_Find), query ? query : "", _TRUNCATE);
	m_Drawer = DrawerFind;
	RefreshFind(true);
}

void UIQuestGraphWindow::RefreshFind(bool force)
{
	const u32 catalog_generation = NqCatalog::Generation();
	if (!force && m_FindRevision == m_Doc->revision && m_FindCatalogGeneration == catalog_generation) return;
	xr_string active;
	if (m_FindIndex >= 0 && m_FindIndex < (int)m_FindResults.size()) active = m_FindResults[m_FindIndex].node;
	m_FindResults.clear();
	m_FindIndex = -1;
	m_FindRevision = m_Doc->revision;
	m_FindCatalogGeneration = catalog_generation;

	xr_vector<SSearchTerm> terms;
	ParseSearch(NqUtil::Trim(m_Find).c_str(), terms);
	if (terms.empty()) return;
	const bool behaviour = m_Doc->quest.IsBehaviour();
	for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
	{
		const SNqNode& node = m_Doc->quest.nodes[i];
		const xr_string title = NqLook::KindTitle(node.kind.c_str(), behaviour);
		xr_string searchable = NqLua::WriteNode(node, 0);
		searchable += "\n";
		searchable += title;
		if (!SearchMatches(searchable.c_str(), terms)) continue;

		SFindResult result;
		result.node = node.id;
		result.kind = node.kind;
		result.title = title;
		if (SearchFieldHit(node.id.c_str(), terms)) result.match = "id";
		else if (SearchFieldHit(node.kind.c_str(), terms) || SearchFieldHit(title.c_str(), terms)) result.match = "kind";
		else if (SearchFieldHit(node.comment.c_str(), terms)) result.match = "comment";
		else result.match = "content";
		m_FindResults.push_back(result);
		if (!active.empty() && active == node.id) m_FindIndex = (int)m_FindResults.size() - 1;
	}
}

bool UIQuestGraphWindow::NavigateFind(int index)
{
	RefreshFind();
	if (m_FindResults.empty()) { m_FindIndex = -1; return false; }
	if (index < 0) index = (int)m_FindResults.size() - 1;
	if (index >= (int)m_FindResults.size()) index = 0;
	m_FindIndex = index;
	m_Canvas->RequestFrameNode(m_FindResults[index].node.c_str());
	Focus();
	return true;
}

void UIQuestGraphWindow::AppendViewState(xr_string& out)
{
	out += NqUtil::Format(",\"zoom_level\":%d,\"zoom\":%.3f,\"center\":[%.1f,%.1f]",
		m_Doc->zoom_idx, m_Canvas->Zoom(), m_Doc->view_cx, m_Doc->view_cy);
	out += ",\"selected\":";
	NqUtil::JsonStringArray(out, m_Doc->selection);
	out += ",\"slot\":";
	NqUtil::JsonString(out, m_Doc->sel_slot);
}

void UIQuestGraphWindow::AppendFindState(xr_string& out, int limit)
{
	RefreshFind();
	limit = _max(1, _min(limit, 500));
	out += ",\"query\":";
	NqUtil::JsonString(out, m_Find);
	out += ",\"count\":";
	NqUtil::JsonInt(out, (int)m_FindResults.size());
	out += ",\"index\":";
	NqUtil::JsonInt(out, m_FindIndex);
	out += ",\"current\":";
	if (m_FindIndex >= 0 && m_FindIndex < (int)m_FindResults.size()) NqUtil::JsonString(out, m_FindResults[m_FindIndex].node);
	else out += "null";
	out += ",\"results\":[";
	const int count = _min(limit, (int)m_FindResults.size());
	for (int i = 0; i < count; ++i)
	{
		if (i) out += ",";
		out += "{\"node\":"; NqUtil::JsonString(out, m_FindResults[i].node);
		out += ",\"kind\":"; NqUtil::JsonString(out, m_FindResults[i].kind);
		out += ",\"title\":"; NqUtil::JsonString(out, m_FindResults[i].title);
		out += ",\"match\":"; NqUtil::JsonString(out, m_FindResults[i].match);
		out += "}";
	}
	out += "],\"truncated\":";
	NqUtil::JsonBool(out, count < (int)m_FindResults.size());
	out += ",\"drawer\":";
	NqUtil::JsonString(out, DrawerName());
	AppendViewState(out);
}

void UIQuestGraphWindow::RefreshProjectFind()
{
	m_ProjectFindResults.clear();
	m_ProjectFindDiagnostics.clear();
	m_ProjectFindError.clear();
	m_ProjectFindComplete = false;
	m_ProjectFindGeneration = 0;
	m_ProjectFindFingerprint = 0;
	NqProjectIndex::SSnapshot snapshot;
	xr_string error;
	if (!NqProjectIndex::Snapshot(snapshot, error))
	{
		m_ProjectFindError = error;
		return;
	}
	NqProjectIndex::SFindResponse result;
	NqProjectIndex::FindNodes(snapshot, m_ProjectFind, result);
	m_ProjectFindResults = result.results;
	m_ProjectFindDiagnostics = result.diagnostics;
	m_ProjectFindComplete = result.complete;
	m_ProjectFindGeneration = result.generation;
	m_ProjectFindFingerprint = result.fingerprint;
}

bool UIQuestGraphWindow::NavigateProjectFind(int index)
{
	if (index < 0 || index >= static_cast<int>(m_ProjectFindResults.size())) return false;
	const NqProjectIndex::SFindResult result = m_ProjectFindResults[index];
	xr_string error;
	if (!UIQuestGraph::Open(result.path.c_str(), &error))
	{
		m_ProjectFindError = error;
		return false;
	}
	UIQuestGraphWindow* target = UIQuestGraph::Find(result.path.c_str());
	if (!target)
	{
		m_ProjectFindError = "the graph's tab was not created";
		return false;
	}
	target->m_Canvas->RequestFrameNode(result.node.c_str());
	target->Focus();
	return true;
}

// Every graph of the project, searched without opening one: a dialog, because its answer
// leads to other tabs
void UIQuestGraphWindow::DrawProjectFind()
{
	if (!XUI::BeginDialog("Find in Project###dlg.nq_project_find", &m_ProjectFindOpen, XUI::DialogSize::L)) return;
	XUI::BeginDialogBody();
	const float button = XUI::Pt(96.f);
	ImGui::SetNextItemWidth(_max(XUI::Em(8.f), ImGui::GetContentRegionAvail().x - button - ImGui::GetStyle().ItemSpacing.x));
	if (m_FocusProjectFind) { ImGui::SetKeyboardFocusHere(); m_FocusProjectFind = false; }
	XUI::SearchField("##query", m_ProjectFind, sizeof(m_ProjectFind), "Find nodes in every graph of the project", 12.f);
	const bool enter = ImGui::GetIO().WantTextInput && ImGui::IsKeyPressed(ImGuiKey_Enter, false);
	ImGui::SameLine();
	if (XUI::Button("Search") || enter) RefreshProjectFind();
	if (!m_ProjectFindComplete)
		XUI::StatusText(XUI::Tone::Warning, "Incomplete: one or more graph files could not be read.");
	if (!m_ProjectFindError.empty())
		XUI::StatusText(XUI::Tone::Error, m_ProjectFindError.c_str());
	if (m_ProjectFindResults.empty())
	{
		if (m_ProjectFindError.empty()) XUI::StatusText(XUI::Tone::Muted, m_ProjectFind[0] ? "No node matches." : "Type what to find, then Search.");
	}
	else
	{
		int go = -1;
		if (XUI::BeginList("##results", ImVec2(0.f, 14.f * 22.f + 10.f)))
		{
			ImGuiListClipper clipper;
			clipper.Begin(static_cast<int>(m_ProjectFindResults.size()), XUI::Pt(22.f));
			while (clipper.Step())
				for (int i = clipper.DisplayStart; i < clipper.DisplayEnd; ++i)
				{
					const NqProjectIndex::SFindResult& result = m_ProjectFindResults[i];
					const xr_string row_id = NqUtil::Format("r%d", i);
					if (XUI::ListRow(row_id.c_str(), false)) go = i;
					const xr_string text = NqUtil::Format("%s  \xC2\xB7  %s  \xC2\xB7  %s  \xC2\xB7  %s", NqUtil::ProjectRelative(result.path.c_str()).c_str(),
						result.node.c_str(), result.title.c_str(), result.source.c_str());
					XUI::TextEllipsis(text.c_str(), -1.f, XUI::Clip::Middle);
				}
		}
		XUI::EndList();
		if (go >= 0 && NavigateProjectFind(go)) m_ProjectFindOpen = false;
	}
	for (u32 i = 0; i < m_ProjectFindDiagnostics.size(); ++i)
	{
		const NqProjectIndex::SDiagnostic& diagnostic = m_ProjectFindDiagnostics[i];
		const xr_string text = NqUtil::ProjectRelative(diagnostic.path.c_str()) + ": " + diagnostic.message;
		XUI::StatusText(XUI::Tone::Warning, text.c_str());
	}
	const xr_string status = m_ProjectFindGeneration
		? NqUtil::Format("%s \xC2\xB7 index generation %u \xC2\xB7 fingerprint %s", Plural((int)m_ProjectFindResults.size(), "result", "results").c_str(),
			m_ProjectFindGeneration, NqProjectIndex::FingerprintText(m_ProjectFindFingerprint).c_str())
		: xr_string();
	XUI::BeginDialogFooter(status.empty() ? nullptr : status.c_str());
	if (XUI::DialogButton("Close", XUI::Kind::Secondary, true)) m_ProjectFindOpen = false;
	XUI::EndDialog();
}

//------------------------------------------------------------------------------
// toolbar, status, menu, keys
//------------------------------------------------------------------------------
bool UIQuestGraphWindow::MinimapAvailable() const
{
	return (int)m_Doc->quest.nodes.size() >= NqCanvas::MinimapMinNodes();
}

xr_string UIQuestGraphWindow::MinimapTip() const
{
	if (MinimapAvailable()) return "The whole graph in the canvas's corner: click or drag it to pan";
	return NqUtil::Format("A graph of %d nodes or more gets a minimap; this one has %d", NqCanvas::MinimapMinNodes(), (int)m_Doc->quest.nodes.size());
}

void UIQuestGraphWindow::DrawToolbar(XUI::Toolbar& tb)
{
	using I = XUI::Icon;
	const bool behaviour = m_Doc->quest.IsBehaviour();
	tb.Group(10);
	if (tb.Action("##save", I::Save, "Save", behaviour ? "Save the graph (Ctrl+S)" : "Save the quest (Ctrl+S)")) Save();
	tb.Group(8);
	if (tb.Action("##undo", I::Undo, "Undo", "Undo (Ctrl+Z)", m_Doc->UndoDepth() > 0)) m_Doc->Undo();
	if (tb.Action("##redo", I::Redo, "Redo", "Redo (Ctrl+Y)", m_Doc->RedoDepth() > 0)) m_Doc->Redo();
	tb.Group(7);
	{
		// the graph is checked as it is edited: this is here only when there is something to look at
		const int e = m_Doc->ErrorCount(), w = m_Doc->WarningCount();
		if (e || w)
		{
			const xr_string count = e ? Plural(e, "error", "errors") : Plural(w, "warning", "warnings");
			if (tb.Action("##problems", e ? I::Error : I::Warning, count.c_str(), "List the problems")) Validate();
		}
	}
	if (tb.Action("##frame", I::FrameAll, "Frame All", "Show the whole graph (Home)")) m_Canvas->RequestFrameAll();
	tb.Group(5);
	if (tb.Action("##find", I::Search, "Find", "Find nodes in this graph (Ctrl+F, F3 next)", true, m_Drawer == DrawerFind)) OpenFind();
	tb.Group(6);
	if (tb.Menu("##more", I::More, "More", "Everything else of this graph"))
	{
		if (XUI::MenuItem("Reload", I::Reload)) Reload();
		if (XUI::MenuItem("Auto Layout", I::AutoLayout)) { m_Doc->Layout(false); m_Canvas->RequestFrameAll(); }
		if (XUI::MenuItem("Find in Project", I::FindInProject, "Ctrl+Shift+F")) OpenProjectFind();
		ImGui::Separator();
		if (XUI::MenuItem("Back", I::Back, "Alt+Left", !m_Canvas->BackHistory().empty())) m_Canvas->HistoryBack();
		if (XUI::MenuItem("Forward", I::Forward, "Alt+Right", !m_Canvas->ForwardHistory().empty())) m_Canvas->HistoryForward();
		ImGui::Separator();
		const xr_vector<xr_string>& bookmarks = m_Canvas->Bookmarks();
		if (ImGui::BeginMenu("Bookmarks"))
		{
			for (u32 i = 0; i < bookmarks.size(); ++i)
			{
				const SNqNode* n = m_Doc->quest.FindNode(bookmarks[i].c_str());
				if (!n) continue;
				const xr_string item = bookmarks[i] + "  \xC2\xB7  " + NqLook::KindTitle(n->kind.c_str(), behaviour);
				if (ImGui::MenuItem(item.c_str())) m_Canvas->RequestFrameNode(bookmarks[i].c_str());
			}
			if (bookmarks.empty()) ImGui::TextDisabled("No bookmarks yet");
			ImGui::Separator();
			if (XUI::MenuItem("Bookmark the Selection", "Ctrl+B", false, !m_Doc->selection.empty())) m_Canvas->ToggleSelectedBookmarks();
			if (ImGui::MenuItem("Previous Bookmark", nullptr, false, !bookmarks.empty())) m_Canvas->JumpBookmark(-1);
			if (ImGui::MenuItem("Next Bookmark", nullptr, false, !bookmarks.empty())) m_Canvas->JumpBookmark(1);
			if (ImGui::MenuItem("Clear Bookmarks", nullptr, false, !bookmarks.empty())) m_Canvas->ClearBookmarks();
			ImGui::EndMenu();
		}
		if (XUI::MenuItem("Shortcuts", I::Keyboard)) m_ShowKeys = !m_ShowKeys;
		ImGui::EndPopup();
	}

	tb.Spring();
	tb.Group(4);
	// The label is the zoom itself, a value and not a caption: with an icon the toolbar folds it
	// away when space runs short and leaves a bare magnifier. No icon keeps it on show. Its width
	// stays put too: "75%" one digit narrower than "150%" unfolded another item's label at one zoom
	// and folded it at the next. A two-digit zoom is led by a figure space (U+2007), which Segoe UI
	// draws exactly as wide as its digits - they are tabular - so every level is as wide as "100%".
	const int percent = int(m_Canvas->Zoom() * 100.f + 0.5f);
	const xr_string zoom = NqUtil::Format(percent < 100 ? "\xE2\x80\x87%d%%" : "%d%%", percent);
	if (tb.Menu("##zoom", I::None, zoom.c_str(), "Zoom"))
	{
		if (ImGui::MenuItem("Zoom In", nullptr, false, m_Doc->zoom_idx + 1 < NqCanvas::ZoomLevels())) m_Canvas->ZoomBy(1);
		if (ImGui::MenuItem("Zoom Out", nullptr, false, m_Doc->zoom_idx > 0)) m_Canvas->ZoomBy(-1);
		ImGui::Separator();
		if (XUI::MenuItem("Frame All", "Home")) m_Canvas->RequestFrameAll();
		if (XUI::MenuItem("Frame Selection", "F", false, !m_Doc->selection.empty())) m_Canvas->RequestFrameSel();
		ImGui::Separator();
		for (int i = NqCanvas::ZoomLevels() - 1; i >= 0; --i)
		{
			const xr_string level = NqUtil::Format("%d%%", int(NqCanvas::ZoomOf(i) * 100.f + 0.5f));
			if (ImGui::MenuItem(level.c_str(), nullptr, i == m_Doc->zoom_idx)) m_Canvas->SetZoom(i);
		}
		ImGui::EndPopup();
	}
	// A small graph draws no minimap, so its toggle must not read as on: it is off and disabled,
	// and its tooltip says why. The author's choice stays and applies once the graph grows.
	const bool minimap_on = MinimapAvailable();
	if (tb.Action("##minimap", I::Minimap, "Minimap", MinimapTip().c_str(), minimap_on, minimap_on && m_Canvas->MinimapVisible()))
		m_Canvas->SetMinimapVisible(!m_Canvas->MinimapVisible());
	if (tb.Action("##graph_panel", I::SidebarLeft, "Graph Panel", behaviour ? "The graph's settings, roles and variables" : "The quest's settings, tasks and variables",
		true, m_ShowGraph && (!m_Rail || m_GraphOverlay)))
		SetGraphPanel(!(m_ShowGraph && (!m_Rail || m_GraphOverlay)));
	if (tb.Action("##details", I::SidebarRight, "Details Panel", "The selected node and its action", true, m_ShowDetails)) m_ShowDetails = !m_ShowDetails;
}

void UIQuestGraphWindow::DrawStatus()
{
	const int n = (int)m_Doc->quest.nodes.size(), e = m_Doc->ErrorCount(), w = m_Doc->WarningCount();
	const xr_string text = Plural(n, "node", "nodes") + "  \xC2\xB7  " + Plural(e, "error", "errors") + "  \xC2\xB7  " + Plural(w, "warning", "warnings");
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(T::TextSecondary));
	ImGui::TextUnformatted(text.c_str());
	ImGui::PopStyleColor();
	XUI::Tip(NqUtil::Format("Catalog: %s, version %d", NqCatalog::Source(), NqCatalog::Version()).c_str());
	if (!m_Message.empty())
	{
		ImGui::SameLine(0.f, XUI::Pt(12.f));
		ImGui::PushStyleColor(ImGuiCol_Text, m_MessageTone == XUI::Tone::Muted ? GrizzlyTheme::Vec4(T::TextMuted) : ImGui::ColorConvertU32ToFloat4(XUI::ToneColor(m_MessageTone)));
		ImGui::TextUnformatted(m_Message.c_str());
		ImGui::PopStyleColor();
	}
}

void UIQuestGraphWindow::DrawMenu()
{
	// MCP ui_popup menu:Graph puts it up the way a click does
	if (XUI::Shell::TakePopup("menu:Graph"))
		ImGui::OpenPopup("Graph");
	if (!ImGui::BeginMenu("Graph"))
		return;
	if (XUI::MenuItem("Save", "Ctrl+S"))				Save();
	if (ImGui::MenuItem("Reload"))						Reload();
	ImGui::Separator();
	if (ImGui::MenuItem("Validate"))					Validate();
	if (ImGui::MenuItem("Auto Layout"))					{ m_Doc->Layout(false); m_Canvas->RequestFrameAll(); }
	if (XUI::MenuItem("Frame All", "Home"))			m_Canvas->RequestFrameAll();
	ImGui::Separator();
	if (XUI::MenuItem("Find", "Ctrl+F"))				OpenFind();
	if (XUI::MenuItem("Find Next", "F3"))				NavigateFind(m_FindIndex + 1);
	if (XUI::MenuItem("Find Previous", "Shift+F3"))	NavigateFind(m_FindIndex - 1);
	if (XUI::MenuItem("Find in Project...", "Ctrl+Shift+F"))	OpenProjectFind();
	ImGui::Separator();
	if (XUI::MenuItem("Bookmark the Selection", "Ctrl+B", false, !m_Doc->selection.empty())) m_Canvas->ToggleSelectedBookmarks();
	if (ImGui::MenuItem("Previous Bookmark", nullptr, false, !m_Canvas->Bookmarks().empty()))	m_Canvas->JumpBookmark(-1);
	if (ImGui::MenuItem("Next Bookmark", nullptr, false, !m_Canvas->Bookmarks().empty()))		m_Canvas->JumpBookmark(1);
	ImGui::Separator();
	const bool graph = m_ShowGraph && (!m_Rail || m_GraphOverlay);
	if (ImGui::MenuItem("Graph Panel", nullptr, graph))						SetGraphPanel(!graph);
	if (ImGui::MenuItem("Details Panel", nullptr, m_ShowDetails))			m_ShowDetails = !m_ShowDetails;
	if (ImGui::MenuItem("Problems", nullptr, m_Drawer == DrawerProblems))	m_Drawer = m_Drawer == DrawerProblems ? DrawerNone : DrawerProblems;
	bool minimap = MinimapAvailable() && m_Canvas->MinimapVisible();
	if (ImGui::MenuItem("Minimap", nullptr, &minimap, MinimapAvailable()))	m_Canvas->SetMinimapVisible(minimap);
	if (!MinimapAvailable()) XUI::Tip(MinimapTip().c_str());
	if (ImGui::MenuItem("Keyboard Shortcuts", nullptr, m_ShowKeys))			m_ShowKeys = !m_ShowKeys;
	ImGui::EndMenu();
}

bool UIQuestGraphWindow::EditAction(LPCSTR action, bool run)
{
	const xr_string a = action ? action : "";
	if (a == "undo")		{ if (run) m_Doc->Undo(); return true; }
	if (a == "redo")		{ if (run) m_Doc->Redo(); return true; }
	if (a == "copy")		{ if (run) m_Canvas->CopySelection(); return true; }
	if (a == "cut")			{ if (run) { m_Canvas->CopySelection(); m_Canvas->DeleteSelection(); } return true; }
	if (a == "paste")		{ if (run) m_Canvas->PasteClipboard(); return true; }
	if (a == "duplicate")	{ if (run) m_Canvas->DuplicateSelection(); return true; }
	if (a == "delete")		{ if (run) m_Canvas->DeleteSelection(); return true; }
	if (a == "find")		{ if (run) OpenFind(); return true; }
	return false;
}

bool UIQuestGraphWindow::Shortcut(int key, int mods)
{
	const bool ctrl = 0 != (mods & ImGuiMod_Ctrl), shift = 0 != (mods & ImGuiMod_Shift), alt = 0 != (mods & ImGuiMod_Alt);
	if (key == ImGuiKey_F && ctrl && shift && !alt)	{ OpenProjectFind(); return true; }
	if (key == ImGuiKey_F && ctrl && !shift && !alt)	{ OpenFind(); return true; }
	if (key == ImGuiKey_F3 && !ctrl && !alt)			{ NavigateFind(shift ? m_FindIndex - 1 : m_FindIndex + 1); return true; }
	if (key == ImGuiKey_Escape && m_ShowKeys && !ctrl && !shift && !alt) { m_ShowKeys = false; return true; }
	return false;
}

// Read off the code that implements them (NqCanvas::HandleKeys and the mouse gestures
// in HandleInput) - a list that drifts from the bindings is worse than none. A card over
// the canvas: it takes nothing from the graph when it is not asked for.
void UIQuestGraphWindow::DrawKeysCard()
{
	if (!m_ShowKeys) return;
	struct SRow { LPCSTR keys; LPCSTR what; };
	static const SRow kRows[] =
	{
		{ 0,						"View" },
		{ "Home",					"Frame the whole graph" },
		{ "F",						"Frame the selection" },
		{ "Alt+Left / Right",		"Back / forward through the view history" },
		{ "Right or middle drag",	"Pan" },
		{ "Wheel",					"Zoom at the cursor" },
		{ 0,						"Nodes" },
		{ "Click / Ctrl+Click",		"Select / add to the selection" },
		{ "Drag on empty space",	"Select with a rectangle" },
		{ "Ctrl+A",					"Select every node" },
		{ "F2",						"Rename the node" },
		{ "Delete",					"Delete the selection" },
		{ "Ctrl+C / Ctrl+V",		"Copy / paste" },
		{ "Ctrl+D",					"Duplicate the selection" },
		{ "Ctrl+Z / Ctrl+Y",		"Undo / redo" },
		{ "Ctrl+B",					"Bookmark the selection" },
		{ "Ctrl+F, F3",				"Find, find next" },
		{ 0,						"Links" },
		{ "Drag from a pin",		"Draw a link" },
		{ "Q",						"Link the selected node to the nearest free pin" },
		{ "Hover a link",			"It lights up: the keys below act on it" },
		{ "D or Alt+Click",			"Cut the lit link" },
		{ "Alt+D",					"Keep the node, close the chain over it (A-B-C becomes A-C)" },
		{ 0,						"Actions" },
		{ "Drag an action row",		"Move it within the node, or to another node" },
		{ "Double-click a row",		"Edit the action in the Details panel" },
		{ 0,						"Other" },
		{ "Right-click",			"The menu of what is under the cursor" },
		{ "Escape",					"Cancel the gesture in progress" },
	};
	const float w = _min(XUI::Pt(440.f), m_PaneCanvas.z - XUI::Pt(16.f));
	if (w < XUI::Pt(160.f)) return;
	const ImVec2 at(m_PaneCanvas.x + m_PaneCanvas.z - w - XUI::Pt(8.f), m_PaneCanvas.y + XUI::Pt(8.f));
	ImGui::SetNextWindowSizeConstraints(ImVec2(w, 0.f), ImVec2(w, _max(XUI::Pt(120.f), m_PaneCanvas.w - XUI::Pt(16.f))));
	ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, XUI::Pt(6.f));
	if (BeginPane("##nq_keys", at, ImVec2(w, 0.f), T::Overlay, true, ImGuiChildFlags_AutoResizeY))
	{
		{
			XUI::FontScope font(XUI::Font::Strong);
			ImGui::TextUnformatted("Keyboard Shortcuts");
		}
		ImGui::SameLine(ImGui::GetContentRegionAvail().x + ImGui::GetCursorPosX() - XUI::Pt(24.f));
		if (XUI::IconButton("##close_keys", XUI::Icon::Close, "Close (Esc)")) m_ShowKeys = false;
		{
			XUI::FontScope font(XUI::Font::Small);
			ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(T::TextMuted));
			ImGui::PushTextWrapPos(0.f);
			ImGui::TextUnformatted("They work while the cursor is over the graph. Every edit is also an MCP call.");
			ImGui::PopTextWrapPos();
			ImGui::PopStyleColor();
		}
		if (ImGui::BeginTable("##keys", 2, ImGuiTableFlags_SizingFixedFit))
		{
			ImGui::TableSetupColumn("keys", ImGuiTableColumnFlags_WidthFixed, XUI::Em(9.f));
			ImGui::TableSetupColumn("what", ImGuiTableColumnFlags_WidthStretch);
			for (const SRow& r : kRows)
			{
				ImGui::TableNextRow();
				ImGui::TableSetColumnIndex(0);
				if (!r.keys)
				{
					ImGui::Dummy(ImVec2(0.f, XUI::Pt(2.f)));
					XUI::FontScope font(XUI::Font::Strong);
					ImGui::TextUnformatted(r.what);
					continue;
				}
				XUI::TextEllipsis(r.keys);
				ImGui::TableSetColumnIndex(1);
				ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(T::TextSecondary));
				ImGui::PushTextWrapPos(0.f);
				ImGui::TextUnformatted(r.what);
				ImGui::PopTextWrapPos();
				ImGui::PopStyleColor();
			}
			ImGui::EndTable();
		}
	}
	ImGui::EndChild();
	ImGui::PopStyleVar();
}

//------------------------------------------------------------------------------
// panes
//------------------------------------------------------------------------------
// The Graph panel folded to a strip: the canvas keeps its 400 pt, and the panel opens over it
void UIQuestGraphWindow::DrawRail(const ImVec2& at, float w, float h)
{
	ImGui::PushStyleVar(ImGuiStyleVar_ItemSpacing, ImVec2(XUI::Pt(4.f), XUI::Pt(4.f)));
	if (BeginPane("##nq_rail", at, ImVec2(w, h)))
	{
		const bool behaviour = m_Doc->quest.IsBehaviour();
		if (XUI::IconButton("##rail_panel", XUI::Icon::SidebarLeft, "The Graph panel, over the graph", m_GraphOverlay)) m_GraphOverlay = !m_GraphOverlay;
		if (XUI::IconButton("##rail_tasks", behaviour ? XUI::Icon::Role : XUI::Icon::Task, behaviour ? "Roles" : "Tasks"))
		{
			m_GraphOverlay = true;
			NqInspector::RequestScroll("tasks");
		}
		if (XUI::IconButton("##rail_vars", XUI::Icon::Variable, "Variables"))
		{
			m_GraphOverlay = true;
			NqInspector::RequestScroll("variables");
		}
	}
	ImGui::EndChild();
	ImGui::PopStyleVar();
}

void UIQuestGraphWindow::DrawExternalBanner()
{
	const xr_string text = NqUtil::Format("%s changed on disk. Reload it, or keep this version and save over it.",
		NqUtil::BaseName(m_Doc->path.c_str()).c_str());
	switch (XUI::Banner(XUI::Tone::Warning, text.c_str(), "Reload", "Overwrite..."))
	{
	case 1: Reload(); break;
	case 2: Overwrite(); break;
	default: break;
	}
}

void UIQuestGraphWindow::DrawDrawer(const ImVec2& at, float w, float h)
{
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(8.f), XUI::Pt(6.f)));
	ImGui::SetCursorScreenPos(at);
	ImGui::PushStyleColor(ImGuiCol_ChildBg, GrizzlyTheme::Vec4(T::Base));
	const bool open = ImGui::BeginChild("##nq_drawer", ImVec2(w, h), ImGuiChildFlags_AlwaysUseWindowPadding);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar();
	if (open)
	{
		RefreshFind();
		const xr_string problems = NqUtil::Format("Problems (%d)", (int)m_Doc->problems.size());
		const xr_string found = NqUtil::Format("Find Results (%d)", (int)m_FindResults.size());
		const char* tabs[] = { problems.c_str(), found.c_str() };
		int cur = m_Drawer == DrawerFind ? 1 : 0;
		if (XUI::Segmented("##drawer_tab", &cur, tabs, 2)) m_Drawer = cur ? DrawerFind : DrawerProblems;
		ImGui::SameLine(ImGui::GetCursorPosX() + _max(0.f, ImGui::GetContentRegionAvail().x - XUI::Pt(24.f)));
		if (XUI::IconButton("##close_drawer", XUI::Icon::Close, "Close")) m_Drawer = DrawerNone;
		if (m_Drawer == DrawerFind)	DrawFindTab();
		else						DrawProblemsTab();
	}
	ImGui::EndChild();
}

void UIQuestGraphWindow::DrawProblemsTab()
{
	if (m_Doc->problems.empty())
	{
		XUI::EmptyState(XUI::Icon::Success, "No problems", "Validate checks the graph again.");
		return;
	}
	if (XUI::BeginList("##problems", ImVec2(0.f, 0.f)))
	{
		for (u32 i = 0; i < m_Doc->problems.size(); ++i)
		{
			const SNqProblem& p = m_Doc->problems[i];
			const xr_string row_id = NqUtil::Format("p%u", i);
			const bool selected = !p.node_id.empty() && m_Doc->selection.size() == 1 && m_Doc->selection[0] == p.node_id;
			const bool clicked = XUI::ListRow(row_id.c_str(), selected);
			const xr_string text = p.Text();
			// a message names a node, a slot and a parameter - it is meant to be pasted
			// into a report or a chat, and reading it off the screen by hand is silly
			if (ImGui::BeginPopupContextItem("##problem_ctx"))
			{
				if (ImGui::MenuItem("Copy")) ImGui::SetClipboardText(text.c_str());
				if (ImGui::MenuItem("Copy All"))
				{
					xr_string all;
					for (u32 k = 0; k < m_Doc->problems.size(); ++k)
					{
						all += m_Doc->problems[k].Text();
						all += "\r\n";
					}
					ImGui::SetClipboardText(all.c_str());
				}
				ImGui::EndPopup();
			}
			const float icon = XUI::Pt(14.f);
			const ImVec2 p0 = ImGui::GetCursorScreenPos();
			const float cy = p0.y + ImGui::GetTextLineHeight() * 0.5f;
			XUI::DrawIconRect(ImGui::GetWindowDrawList(), p.IsError() ? XUI::Icon::Error : XUI::Icon::Warning, ImVec2(p0.x, cy - icon * 0.5f),
				ImVec2(p0.x + icon, cy + icon * 0.5f), XUI::ToneColor(p.IsError() ? XUI::Tone::Error : XUI::Tone::Warning));
			ImGui::Dummy(ImVec2(icon, 0.f));
			ImGui::SameLine(0.f, XUI::Pt(6.f));
			XUI::TextEllipsis(text.c_str());
			if (clicked && !p.node_id.empty())
			{
				m_Canvas->RequestFrameNode(p.node_id.c_str());
				m_Doc->sel_slot = p.slot.find("enter:") == 0 || p.slot.find("exit:") == 0 ? p.slot : "";
			}
		}
	}
	XUI::EndList();
}

void UIQuestGraphWindow::DrawFindTab()
{
	const float buttons = XUI::Pt(24.f) * 2.f + XUI::Pt(4.f) + XUI::Em(5.f);
	ImGui::SetNextItemWidth(_max(XUI::Em(8.f), ImGui::GetContentRegionAvail().x - buttons));
	if (m_FocusFind) { ImGui::SetKeyboardFocusHere(); m_FocusFind = false; }
	if (XUI::SearchField("##find", m_Find, sizeof(m_Find), "Find by id, kind, parameters, actions or comments", 12.f)) RefreshFind(true);
	// Enter in the field goes to the next result, Shift+Enter to the previous one
	if (ImGui::GetIO().WantTextInput && ImGui::IsWindowFocused(ImGuiFocusedFlags_ChildWindows) && ImGui::IsKeyPressed(ImGuiKey_Enter, false))
		NavigateFind(ImGui::GetIO().KeyShift ? m_FindIndex - 1 : m_FindIndex + 1);
	ImGui::SameLine();
	if (XUI::IconButton("##find_prev", XUI::Icon::ChevronUp, "Previous (Shift+F3)", false, !m_FindResults.empty())) NavigateFind(m_FindIndex - 1);
	ImGui::SameLine(0.f, XUI::Pt(4.f));
	if (XUI::IconButton("##find_next", XUI::Icon::ChevronDown, "Next (F3)", false, !m_FindResults.empty())) NavigateFind(m_FindIndex + 1);
	ImGui::SameLine();
	{
		const xr_string count = m_FindResults.empty() ? xr_string("0") : NqUtil::Format("%d / %d", m_FindIndex >= 0 ? m_FindIndex + 1 : 0, (int)m_FindResults.size());
		ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::Vec4(T::TextMuted));
		XUI::TextEllipsis(count.c_str());
		ImGui::PopStyleColor();
	}
	if (m_FindResults.empty())
	{
		XUI::StatusText(XUI::Tone::Muted, m_Find[0] ? "No node matches." : "Type to find nodes. Spaces separate terms, -term leaves a term out.");
		return;
	}
	int go = -1;
	if (XUI::BeginList("##find_results", ImVec2(0.f, 0.f)))
	{
		ImGuiListClipper clipper;
		clipper.Begin((int)m_FindResults.size(), XUI::Pt(22.f));
		if (m_FindIndex >= 0) clipper.IncludeItemByIndex(m_FindIndex);
		while (clipper.Step())
			for (int i = clipper.DisplayStart; i < clipper.DisplayEnd; ++i)
			{
				const SFindResult& r = m_FindResults[i];
				const xr_string row_id = NqUtil::Format("f%d", i);
				if (XUI::ListRow(row_id.c_str(), i == m_FindIndex)) go = i;
				const xr_string text = r.node + "  \xC2\xB7  " + r.title + "  \xC2\xB7  " + r.match;
				XUI::TextEllipsis(text.c_str());
			}
	}
	XUI::EndList();
	if (go >= 0) NavigateFind(go);
}

void UIQuestGraphWindow::DrawBody()
{
	// The palette belongs to the document being drawn: a quest is never offered
	// a step that drives an NPC, a behaviour graph is never offered a task action.
	NqCatalog::SetDocBehaviour(m_Doc && m_Doc->quest.IsBehaviour());
	PollExternal();
	m_DrawnFrame = ImGui::GetFrameCount();

	const float gap = XUI::Pt(kGap);
	const ImVec2 origin = ImGui::GetCursorScreenPos();
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	float top = origin.y;
	const float bottom = origin.y + avail.y;

	// the file changed under the document: said above everything, with the two ways out
	if (m_External)
	{
		ImGui::SetCursorScreenPos(origin);
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(XUI::Pt(8.f), XUI::Pt(8.f)));
		ImGui::BeginChild("##nq_banner", ImVec2(avail.x, 0.f), ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysUseWindowPadding);
		ImGui::PopStyleVar();
		DrawExternalBanner();
		ImGui::EndChild();
		top = ImGui::GetItemRectMax().y + gap;
	}

	// the drawer takes its height from the bottom; the canvas keeps at least 260 pt
	float drawer = 0.f;
	if (m_Drawer != DrawerNone)
	{
		const float room = (bottom - top) / XUI::Dpi() - kCanvasMinH - kGap;
		drawer = XUI::Pt(_max(kDrawerMin, _min(XUI::Layout::Get(kDrawerHeight, kDrawerDef), _max(kDrawerMin, room))));
	}
	const float main_h = _max(XUI::Pt(60.f), bottom - top - (drawer > 0.f ? drawer + gap : 0.f));

	// the columns: the panels keep their pt size, the canvas takes what is left - and when it
	// would get less than 400 pt, the Graph panel folds to a rail first, the Details panel
	// gives up its extra width second
	const float width = avail.x;
	float dw = m_ShowDetails ? XUI::Pt(LayoutPt(kDetailsWidth, kDetailsDef, kDetailsMin, kDetailsMax)) : 0.f;
	float gw = m_ShowGraph ? XUI::Pt(GraphWidth()) : 0.f;
	const int gaps = (m_ShowGraph ? 1 : 0) + (m_ShowDetails ? 1 : 0);
	m_Rail = m_ShowGraph && width - gw - dw - gap * gaps < XUI::Pt(kCanvasMin);
	if (m_Rail) gw = XUI::Pt(kRail);
	else m_GraphOverlay = false;
	if (m_ShowDetails && width - gw - dw - gap * gaps < XUI::Pt(kCanvasMin))
		dw = _max(XUI::Pt(kDetailsMin), width - gw - gap * gaps - XUI::Pt(kCanvasMin));
	const float canvas_w = _max(XUI::Pt(80.f), width - gw - dw - gap * gaps);
	// a drag never takes the canvas under 400 pt: the panel stops where the rail would begin
	const float spare = (width - gw - dw - gap * gaps) / XUI::Dpi() - kCanvasMin;
	const float graph_hi = _max(kGraphMin, _min(kGraphMax, gw / XUI::Dpi() + spare));
	const float details_hi = _max(kDetailsMin, _min(kDetailsMax, dw / XUI::Dpi() + spare));

	float x = origin.x;
	m_PaneGraph = ImVec4(0, 0, 0, 0);
	if (m_ShowGraph)
	{
		if (m_Rail) DrawRail(ImVec2(x, top), gw, main_h);
		else
		{
			if (BeginPane("##nq_graph", ImVec2(x, top), ImVec2(gw, main_h))) m_Inspector->DrawGraphPanel();
			ImGui::EndChild();
			m_PaneGraph = ImVec4(x, top, gw, main_h);
		}
		x += gw;
		if (!m_Rail) XUI::SplitterAt(kGraphWidth, true, ImVec2(x + gap * 0.5f, top), main_h, 1.f, kGraphMin, graph_hi, kGraphDef);
		else ImGui::GetWindowDrawList()->AddLine(ImVec2(x + gap * 0.5f, top), ImVec2(x + gap * 0.5f, top + main_h), GrizzlyTheme::U32(T::BorderSubtle));
		x += gap;
	}

	// the canvas
	ImGui::SetCursorScreenPos(ImVec2(x, top));
	m_PaneCanvas = ImVec4(x, top, canvas_w, main_h);
	// a child without a border has no padding: the canvas runs to its edges. No style is pushed
	// around it - the canvas's own menus would inherit it
	ImGui::BeginChild("##nq_canvas", ImVec2(canvas_w, main_h), ImGuiChildFlags_None, ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
	m_Canvas->Draw(ImVec2(canvas_w, main_h));
	ImGui::EndChild();
	x += canvas_w;

	// F2 with the Details panel hidden brings it back to rename in
	const bool rename = m_Canvas->TakeRenameRequest() || m_PendingRename;
	const bool focus_action = m_Canvas->TakeFocusAction();
	m_PendingRename = false;
	if ((rename || focus_action) && !m_ShowDetails) { m_ShowDetails = true; m_PendingRename = rename; }
	m_PaneDetails = ImVec4(0, 0, 0, 0);
	if (m_ShowDetails)
	{
		XUI::SplitterAt(kDetailsWidth, true, ImVec2(x + gap * 0.5f, top), main_h, -1.f, kDetailsMin, details_hi, kDetailsDef);
		x += gap;
		if (BeginPane("##nq_details", ImVec2(x, top), ImVec2(dw, main_h))) m_Inspector->DrawDetails(rename, focus_action);
		ImGui::EndChild();
		m_PaneDetails = ImVec4(x, top, dw, main_h);
	}

	// the rail's panel, over the canvas
	if (m_Rail && m_GraphOverlay)
	{
		const float ow = _min(XUI::Pt(kGraphDef), canvas_w);
		ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, 0.f);
		if (BeginPane("##nq_graph_overlay", ImVec2(m_PaneCanvas.x, top), ImVec2(ow, main_h), T::Overlay, true)) m_Inspector->DrawGraphPanel();
		ImGui::EndChild();
		ImGui::PopStyleVar();
		m_PaneGraph = ImVec4(m_PaneCanvas.x, top, ow, main_h);
	}
	DrawKeysCard();

	m_PaneDrawer = ImVec4(0, 0, 0, 0);
	if (m_Drawer != DrawerNone)
	{
		const float y = top + main_h;
		XUI::SplitterAt(kDrawerHeight, false, ImVec2(origin.x, y + gap * 0.5f), width, -1.f, kDrawerMin, 1000.f, kDrawerDef);
		DrawDrawer(ImVec2(origin.x, y + gap), width, drawer);
		m_PaneDrawer = ImVec4(origin.x, y + gap, width, drawer);
	}

	// the tab's dialogs belong to it, whichever panes are shown
	m_Inspector->DrawDialogs();
	DrawProjectFind();
	m_Inspector->CommitIdle();
	// every pane of this frame is recorded now
	if (m_ViewFrames > 0) --m_ViewFrames;
}

bool UIQuestGraphWindow::DrawnLately() const
{
	// the frame being built or the one before: what the panes say is this tab as it is shown
	return m_DrawnFrame >= ImGui::GetFrameCount() - 1;
}

void UIQuestGraphWindow::AppendPanes(xr_string& out) const
{
	const ImVec2 at = ImGui::GetMainViewport()->Pos;
	const float k = 1.f / XUI::Dpi();
	// what the inspector remembers of a pane that is not drawn any more is not a pane
	const bool graph = m_PaneGraph.z > 0.f, details = m_PaneDetails.z > 0.f;
	const ImVec4 none(0, 0, 0, 0);
	const ImVec4 node = details ? m_Inspector->NodePane() : none, action = details ? m_Inspector->ActionPane() : none;
	const ImVec4 vars = graph ? m_Inspector->VarsPane() : none;
	const ImVec4* panes[] = { &m_PaneGraph, &vars, &m_PaneCanvas, &m_PaneDetails, &node, &action, &m_PaneDrawer };
	const char* names[] = { "graph", "variables", "canvas", "details", "node", "action", "drawer" };
	out += "{";
	for (int i = 0; i < 7; ++i)
	{
		const ImVec4& p = *panes[i];
		if (p.z <= 0.f) out += NqUtil::Format("%s\"%s\":null", i ? "," : "", names[i]);
		else out += NqUtil::Format("%s\"%s\":[%.0f,%.0f,%.0f,%.0f]", i ? "," : "", names[i], (p.x - at.x) * k, (p.y - at.y) * k, p.z * k, p.w * k);
	}
	out += "}";
}

void UIQuestGraphWindow::McpFind(LPCSTR raw, xr_string& out)
{
	xr_string action, query;
	NqUtil::ArgString(raw, "action", action);
	const bool has_query = NqUtil::ArgString(raw, "query", query);
	if (action.empty()) action = has_query ? "set" : "get";
	if (has_query && query.size() >= sizeof(m_Find)) { NqUtil::JsonError(out, "query is too long (max 255 UTF-8 bytes)"); return; }

	if (action == "set")
	{
		SetFindQuery(has_query ? query.c_str() : "");
		if (NqUtil::ArgBool(raw, "select", false)) NavigateFind(0);
	}
	else if (action == "next" || action == "previous")
	{
		if (has_query) SetFindQuery(query.c_str());
		NavigateFind(action == "previous" ? m_FindIndex - 1 : m_FindIndex + 1);
	}
	else if (action == "clear")
	{
		m_Find[0] = 0;
		if (m_Drawer == DrawerFind) m_Drawer = DrawerNone;
		RefreshFind(true);
	}
	else if (action == "get")
	{
		if (has_query) SetFindQuery(query.c_str());
	}
	else
	{
		NqUtil::JsonError(out, "action must be get, set, next, previous or clear");
		return;
	}

	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(m_Doc->path.c_str()).c_str());
	AppendFindState(out, NqUtil::ArgInt(raw, "limit", 100));
	out += "}";
}

void UIQuestGraphWindow::McpBookmarks(LPCSTR raw, xr_string& out)
{
	xr_vector<xr_string> known = m_Canvas->Bookmarks();
	for (u32 i = 0; i < known.size(); ++i)
		if (!m_Doc->quest.FindNode(known[i].c_str())) m_Canvas->SetBookmark(known[i].c_str(), false);

	xr_string action, node;
	NqUtil::ArgString(raw, "action", action);
	NqUtil::ArgString(raw, "node", node);
	if (action.empty()) action = "get";
	if (node.empty() && m_Doc->selection.size() == 1) node = m_Doc->selection[0];

	if (action == "add" || action == "remove" || action == "toggle")
	{
		if (node.empty()) { NqUtil::JsonError(out, "node is required unless exactly one node is selected"); return; }
		if (action != "remove" && !m_Doc->quest.FindNode(node.c_str())) { NqUtil::JsonError(out, "node not found"); return; }
		const bool enabled = action == "add" || (action == "toggle" && !m_Canvas->IsBookmarked(node.c_str()));
		if (!m_Canvas->SetBookmark(node.c_str(), enabled)) { NqUtil::JsonError(out, "bookmark update failed"); return; }
	}
	else if (action == "next" || action == "previous")
	{
		if (!m_Canvas->JumpBookmark(action == "previous" ? -1 : 1)) { NqUtil::JsonError(out, "no bookmarks"); return; }
	}
	else if (action == "clear") m_Canvas->ClearBookmarks();
	else if (action != "get")
	{
		NqUtil::JsonError(out, "action must be get, add, remove, toggle, next, previous or clear");
		return;
	}

	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(m_Doc->path.c_str()).c_str());
	out += ",\"bookmarks\":";
	NqUtil::JsonStringArray(out, m_Canvas->Bookmarks());
	out += ",\"count\":";
	NqUtil::JsonInt(out, (int)m_Canvas->Bookmarks().size());
	AppendViewState(out);
	out += "}";
}

void UIQuestGraphWindow::McpHistory(LPCSTR raw, xr_string& out)
{
	xr_string action;
	NqUtil::ArgString(raw, "action", action);
	if (action.empty()) action = "get";
	bool moved = false;
	if (action == "back") moved = m_Canvas->HistoryBack();
	else if (action == "forward") moved = m_Canvas->HistoryForward();
	else if (action == "clear") m_Canvas->ClearHistory();
	else if (action != "get")
	{
		NqUtil::JsonError(out, "action must be get, back, forward or clear");
		return;
	}

	auto append_state = [](xr_string& json, const NqCanvas::SViewState& state)
	{
		json += NqUtil::Format("{\"center\":[%.1f,%.1f],\"zoom_level\":%d,\"selected\":", state.cx, state.cy, state.zoom);
		NqUtil::JsonStringArray(json, state.selection);
		json += ",\"slot\":";
		NqUtil::JsonString(json, state.slot);
		json += "}";
	};
	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(m_Doc->path.c_str()).c_str());
	out += ",\"moved\":";
	NqUtil::JsonBool(out, moved);
	const xr_vector<NqCanvas::SViewState>& back = m_Canvas->BackHistory();
	const xr_vector<NqCanvas::SViewState>& forward = m_Canvas->ForwardHistory();
	out += ",\"back_count\":";
	NqUtil::JsonInt(out, (int)back.size());
	out += ",\"forward_count\":";
	NqUtil::JsonInt(out, (int)forward.size());
	out += ",\"back\":[";
	for (u32 i = 0; i < back.size(); ++i) { if (i) out += ","; append_state(out, back[i]); }
	out += "],\"forward\":[";
	for (u32 i = 0; i < forward.size(); ++i) { if (i) out += ","; append_state(out, forward[i]); }
	out += "],\"current\":";
	append_state(out, m_Canvas->CurrentView());
	out += "}";
}

void UIQuestGraphWindow::McpMinimap(LPCSTR raw, xr_string& out)
{
	xr_string action;
	NqUtil::ArgString(raw, "action", action);
	if (action.empty()) action = "get";
	if (action == "show") m_Canvas->SetMinimapVisible(true);
	else if (action == "hide") m_Canvas->SetMinimapVisible(false);
	else if (action == "toggle") m_Canvas->SetMinimapVisible(!m_Canvas->MinimapVisible());
	else if (action != "get")
	{
		NqUtil::JsonError(out, "action must be get, show, hide or toggle");
		return;
	}

	out = "{\"ok\":true,\"path\":";
	NqUtil::JsonPath(out, NqUtil::ProjectRelative(m_Doc->path.c_str()).c_str());
	out += ",\"visible\":";
	NqUtil::JsonBool(out, m_Canvas->MinimapVisible());
	// what the last frame drew: the toggle asks, a small graph (under min_nodes) or a small
	// canvas shows none
	out += ",\"shown\":";
	NqUtil::JsonBool(out, m_Canvas->MinimapShown());
	out += ",\"min_nodes\":";
	NqUtil::JsonInt(out, NqCanvas::MinimapMinNodes());
	// the toolbar's toggle is off and disabled while this is false
	out += ",\"available\":";
	NqUtil::JsonBool(out, MinimapAvailable());
	float min_x, min_y, max_x, max_y;
	out += ",\"graph_bounds\":";
	if (m_Canvas->GraphBounds(min_x, min_y, max_x, max_y))
		out += NqUtil::Format("[%.1f,%.1f,%.1f,%.1f]", min_x, min_y, max_x, max_y);
	else out += "null";
	out += ",\"viewport_bounds\":";
	if (m_Canvas->ViewportBounds(min_x, min_y, max_x, max_y))
		out += NqUtil::Format("[%.1f,%.1f,%.1f,%.1f]", min_x, min_y, max_x, max_y);
	else out += "null";
	AppendViewState(out);
	out += "}";
}

//------------------------------------------------------------------------------
// registry
//------------------------------------------------------------------------------
namespace UIQuestGraph
{
	UIQuestGraphWindow* Find(LPCSTR path)
	{
		xr_string abs, err;
		if (!NqDocs::Resolve(path, abs, err)) return 0;
		xr_string key = NqUtil::PathKey(abs.c_str());
		for (u32 i = 0; i < s_Windows.size(); ++i)
			if (NqUtil::PathKey(s_Windows[i]->Doc()->path.c_str()) == key) return s_Windows[i];
		return 0;
	}

	xr_string DocId(LPCSTR abs_path, bool behaviour)
	{
		xr_string rel = NqUtil::ProjectRelative(abs_path);
		for (char& c : rel)
			if (c == '\\') c = '/';
		return (behaviour ? "beh:" : "quest:") + rel;
	}

	// The window of a tab is looked up by its file each time: MCP quest_close and a project switch delete
	// windows behind the shell's back, and the tab must never reach a freed one.
	static UIQuestGraphWindow* FindAbs(const xr_string& abs)
	{
		const xr_string key = NqUtil::PathKey(abs.c_str());
		for (u32 i = 0; i < s_Windows.size(); ++i)
			if (NqUtil::PathKey(s_Windows[i]->Doc()->path.c_str()) == key && NqDocs::IsOpen(s_Windows[i]->Doc())) return s_Windows[i];
		return 0;
	}

	static void RegisterTab(UIQuestGraphWindow* w)
	{
		const xr_string abs = w->Doc()->path;
		const bool behaviour = w->Doc()->quest.IsBehaviour();
		XUI::Shell::DocDesc d;
		d.id = DocId(abs.c_str(), behaviour);
		d.kind = behaviour ? "beh" : "quest";
		d.path = abs;
		d.icon = behaviour ? XUI::Icon::Behaviour : XUI::Icon::Quest;
		// the panes run edge to edge and carry their own padding
		d.padded = false;
		d.label = [abs] { xr_string name = NqUtil::BaseName(abs.c_str()); const size_t dot = name.rfind('.'); if (dot != xr_string::npos && dot) name.resize(dot); return name; };
		d.tooltip = [abs] { return NqUtil::ProjectRelative(abs.c_str()); };
		d.alive = [abs] { return !!FindAbs(abs); };
		d.dirty = [abs] { UIQuestGraphWindow* x = FindAbs(abs); return x && x->Doc()->dirty; };
		d.missing = [abs] { return !NqUtil::FileExists(abs.c_str()); };
		d.body = [abs] { if (UIQuestGraphWindow* x = FindAbs(abs)) x->DrawBody(); };
		d.toolbar = [abs](XUI::Toolbar& tb) { if (UIQuestGraphWindow* x = FindAbs(abs)) x->DrawToolbar(tb); };
		d.status = [abs] { if (UIQuestGraphWindow* x = FindAbs(abs)) x->DrawStatus(); };
		d.menus = [abs] { if (UIQuestGraphWindow* x = FindAbs(abs)) x->DrawMenu(); };
		d.shortcut = [abs](int key, int mods) { UIQuestGraphWindow* x = FindAbs(abs); return x && x->Shortcut(key, mods); };
		d.edit = [abs](LPCSTR action, bool run) { UIQuestGraphWindow* x = FindAbs(abs); return x && x->EditAction(action, run); };
		d.save = [abs](xr_string& err)
		{
			UIQuestGraphWindow* x = FindAbs(abs);
			if (!x) return true;
			if (x->Doc()->Save(false, err)) return true;
			if (err == "modified externally") err = "the file changed on disk - Reload it, or Overwrite it from the banner over the graph";
			return false;
		};
		// Don't Save puts the file's content back: quitting writes every dirty document (NqDocs::SaveDirty), and a
		// tab's close drops the document with on_close anyway. A document never saved has no file to go back to
		d.discard = [abs]
		{
			UIQuestGraphWindow* x = FindAbs(abs);
			xr_string err;
			if (x && !x->Doc()->Reload(err))
				x->Doc()->dirty = false;
		};
		d.on_close = [abs]
		{
			for (u32 i = 0; i < s_Windows.size(); ++i)
			{
				if (NqUtil::PathKey(s_Windows[i]->Doc()->path.c_str()) != NqUtil::PathKey(abs.c_str())) continue;
				xr_string err;
				NqDoc* doc = s_Windows[i]->Doc();
				const bool open = NqDocs::IsOpen(doc);
				xr_delete(s_Windows[i]);
				s_Windows.erase(s_Windows.begin() + i);
				if (open) NqDocs::Close(abs.c_str(), true, err);
				break;
			}
		};
		XUI::Shell::Open(d, true);
	}

	bool Open(LPCSTR path, xr_string* err)
	{
		xr_string e;
		if (UIQuestGraphWindow* w = Find(path)) { w->Focus(); return true; }
		NqDoc* doc = NqDocs::Open(path, e);
		if (!doc)
		{
			if (err) *err = e; else ELog.DlgMsg(mtError, "Graph: %s", e.c_str());
			return false;
		}
		UIQuestGraphWindow* w = xr_new<UIQuestGraphWindow>(doc);
		s_Windows.push_back(w);
		RegisterTab(w);
		return true;
	}

	void RegisterFactories()
	{
		XUI::Shell::RegisterFactory("quest", [](LPCSTR rel) { xr_string err; return Open(rel, &err); });
		XUI::Shell::RegisterFactory("beh", [](LPCSTR rel) { xr_string err; return Open(rel, &err); });
	}

	void Update()
	{
		// tabs die with the project they belong to. The first pump of a session
		// only records the project: documents opened over MCP before this point
		// belong to it already and must not be torn down.
		xr_string proj = EditorProject::Active() ? EditorProject::Root() : "";
		if (proj != s_LastProject)
		{
			if (!s_LastProject.empty()) OnProjectChanged();
			s_LastProject = proj;
		}
		for (u32 i = 0; i < s_Windows.size(); )
		{
			UIQuestGraphWindow* w = s_Windows[i];
			// the document may have been closed without us (MCP quest_close): the
			// window goes, its tab follows (alive() finds no window any more)
			if (!NqDocs::IsOpen(w->Doc()))
			{
				xr_delete(w);
				s_Windows.erase(s_Windows.begin() + i);
				continue;
			}
			++i;
		}
	}

	void CloseAll(bool discard)
	{
		xr_vector<xr_string> ids;
		for (u32 i = 0; i < s_Windows.size(); ++i)
			ids.push_back(DocId(s_Windows[i]->Doc()->path.c_str(), s_Windows[i]->Doc()->quest.IsBehaviour()));
		for (const xr_string& id : ids)
		{
			if (discard) XUI::Shell::Close(id.c_str(), XUI::Shell::Discard, nullptr);
			else XUI::Shell::RequestClose(id.c_str());
		}
	}

	void OnProjectChanged()
	{
		// the tabs die with the project, the work in them does not: every dirty
		// document is written back before anything is destroyed. No prompt - this
		// runs from the draw loop, and MCP / -nodlg runs must never stall on a
		// modal (the file names of what was saved go to the log instead).
		const int saved = NqDocs::SaveDirty();
		if (saved) Msg("* [nq] %d unsaved graph(s) saved before leaving the project", saved);
		for (u32 i = 0; i < s_Windows.size(); ++i) xr_delete(s_Windows[i]);
		s_Windows.clear();
		NqDocs::CloseAll();
	}

	int Count() { return (int)s_Windows.size(); }

	static UIQuestGraphWindow* OpenMcpWindow(LPCSTR raw, xr_string& out)
	{
		xr_string path;
		if (!NqUtil::ArgString(raw, "path", path) || path.empty()) { NqUtil::JsonError(out, "path is required"); return 0; }
		xr_string err;
		if (!Open(path.c_str(), &err)) { NqUtil::JsonError(out, err.c_str()); return 0; }
		UIQuestGraphWindow* window = Find(path.c_str());
		if (!window) { NqUtil::JsonError(out, "window not found"); return 0; }
		window->Focus();
		return window;
	}

	void McpClose(LPCSTR raw, xr_string& out)
	{
		xr_string path;
		if (!NqUtil::ArgString(raw, "path", path) || path.empty()) { NqUtil::JsonError(out, "missing 'path' (project-relative, e.g. quests/wolf_debt.nqasset or npc/ambush.behasset)"); return; }
		NqDoc* doc = NqDocs::Get(path.c_str());
		if (!doc) { NqUtil::JsonError(out, "not open"); return; }
		const bool discard = NqUtil::ArgBool(raw, "discard", false);
		if (doc->dirty && !discard) { NqUtil::JsonError(out, "unsaved"); return; }
		const xr_string absolute = doc->path;

		// Canvas and inspector retain the document pointer, so every matching view must die first.
		for (u32 i = 0; i < s_Windows.size(); )
		{
			if (s_Windows[i]->Doc() != doc) { ++i; continue; }
			xr_delete(s_Windows[i]);
			s_Windows.erase(s_Windows.begin() + i);
		}

		xr_string err;
		if (!NqDocs::Close(absolute.c_str(), discard, err)) { NqUtil::JsonError(out, err); return; }
		out = "{\"ok\":true}";
	}

	// show | hide | toggle: false + an error when the value is none of them
	static bool ArgToggle(LPCSTR raw, LPCSTR key, bool current, bool& out_value, bool& given, xr_string& err)
	{
		string_path v = "";
		GrizzlyMCP::GetArg(raw, key, v, sizeof(v));
		given = v[0] != 0;
		out_value = current;
		if (!given) return true;
		if (0 == strcmp(v, "show"))			out_value = true;
		else if (0 == strcmp(v, "hide"))	out_value = false;
		else if (0 == strcmp(v, "toggle"))	out_value = !current;
		else { err = NqUtil::Format("%s must be \"show\", \"hide\" or \"toggle\"", key); return false; }
		return true;
	}

	static void AppendView(UIQuestGraphWindow* w, xr_string& out);

	void McpView(LPCSTR raw, xr_string& out)
	{
		UIQuestGraphWindow* w = OpenMcpWindow(raw, out);
		if (!w) return;
		// The MCP pump runs a pending answer again once the frames it owes are drawn, with
		// answer_only set: that read applies nothing - a toggle must not flip back
		if (NqUtil::ArgBool(raw, "answer_only", false)) { AppendView(w, out); return; }
		NqCanvas* c = w->Canvas();
		// a tab that was not on screen (just opened, or behind another one) has no panes to report yet
		bool asked = !w->DrawnLately();
		string_path frame = "";
		GrizzlyMCP::GetArg(raw, "frame", frame, sizeof(frame));
		int zoom = NqUtil::ArgInt(raw, "zoom_level", -1);
		float cx, cy;
		bool has_c = NqUtil::ArgFloat(raw, "cx", cx) && NqUtil::ArgFloat(raw, "cy", cy);
		string_path slot = "";
		GrizzlyMCP::GetArg(raw, "slot", slot, sizeof(slot));
		// The reply has always reported `selected`; this is how it gets set. Nothing
		// else can: the Details panel follows the graph selection, so without it a caller
		// could open a node's tab and never look at the node.
		string_path select = "";
		GrizzlyMCP::GetArg(raw, "select", select, sizeof(select));
		string_path scroll = "";
		GrizzlyMCP::GetArg(raw, "scroll", scroll, sizeof(scroll));

		// the panes first: a scroll asked for in the same call is judged against them
		xr_string err;
		bool value = false, given = false;
		if (!ArgToggle(raw, "graph_panel", w->GraphPanel() && 0 != strcmp(w->GraphPanelState(), "rail"), value, given, err)) { NqUtil::JsonError(out, err); return; }
		if (given) { w->SetGraphPanel(value); asked = true; }
		if (!ArgToggle(raw, "details", w->Details(), value, given, err)) { NqUtil::JsonError(out, err); return; }
		if (given) { w->SetDetails(value); asked = true; }
		if (!ArgToggle(raw, "keys", w->ShowKeys(), value, given, err)) { NqUtil::JsonError(out, err); return; }
		if (given) { w->SetShowKeys(value); asked = true; }
		// the drawer: problems=show opens it on the problems, find results come through quest_find
		if (!ArgToggle(raw, "problems", w->Drawer() == UIQuestGraphWindow::DrawerProblems, value, given, err)) { NqUtil::JsonError(out, err); return; }
		if (given)
		{
			w->SetDrawer(value ? UIQuestGraphWindow::DrawerProblems : (w->Drawer() == UIQuestGraphWindow::DrawerProblems ? UIQuestGraphWindow::DrawerNone : w->Drawer()));
			asked = true;
		}
		float width = -1.f;
		if (NqUtil::ArgFloat(raw, "graph_width", width))	{ w->SetGraphWidth(width); asked = true; }
		if (NqUtil::ArgFloat(raw, "details_width", width))	{ w->SetDetailsWidth(width); asked = true; }
		// the splitters answer to a drag only; a form taller than its pane (a spawn
		// with several rows) could not be looked at from outside without these
		float action_split = -1.f, vars_split = -1.f;
		NqUtil::ArgFloat(raw, "action_split", action_split);
		NqUtil::ArgFloat(raw, "vars_split", vars_split);
		if (action_split >= 0.f || vars_split >= 0.f) { w->Inspector()->SetSplit(action_split, vars_split); asked = true; }

		xr_vector<xr_string> want;
		if (select[0] && 0 != strcmp(select, "none"))
		{
			xr_string rest = select;
			while (!rest.empty())
			{
				const size_t comma = rest.find(',');
				xr_string id = NqUtil::Trim((comma == xr_string::npos ? rest : rest.substr(0, comma)).c_str());
				rest = (comma == xr_string::npos) ? xr_string() : rest.substr(comma + 1);
				if (id.empty()) continue;
				if (!w->Doc()->quest.FindNode(id.c_str()))
				{
					NqUtil::JsonError(out, NqUtil::Format("no node '%s' in this graph", id.c_str()).c_str());
					return;
				}
				want.push_back(id);
			}
			if (want.empty()) { NqUtil::JsonError(out, "select needs a node id, a comma separated list, or \"none\""); return; }
		}
		// Reject validator-only slots before changing any part of the view.
		if (slot[0] && 0 != strcmp(slot, "none"))
		{
			LPCSTR digits = 0;
			if (0 == strncmp(slot, "enter:", 6))		digits = slot + 6;
			else if (0 == strncmp(slot, "exit:", 5))	digits = slot + 5;
			bool ok = digits && digits[0];
			for (LPCSTR p = digits; ok && *p; ++p) if (*p < '0' || *p > '9') ok = false;
			if (!ok) { NqUtil::JsonError(out, "slot must be \"enter:N\", \"exit:N\" or \"none\""); return; }
		}

		// a pane or a width set above moves the canvas's edges: Frame All measures the frame that draws them
		if (asked) c->Remeasure();
		c->BeginNavigationBatch();
		if (frame[0] && 0 == strcmp(frame, "all")) c->RequestFrameAll();
		else if (frame[0]) c->RequestFrameNode(frame);
		if (zoom >= 0) c->SetZoom(zoom);
		if (has_c) c->Center(cx, cy);
		// before the slot: SelectOnly clears sel_slot, and "select a node, open its
		// second action" has to end up on the action
		if (select[0]) c->SelectNodes(want);
		if (slot[0])
		{
			if (0 == strcmp(slot, "none")) w->Doc()->sel_slot.clear();
			else w->Doc()->sel_slot = slot;
		}
		c->EndNavigationBatch();

		// the panels are taller than their panes: a row far down (a task's steps) could be
		// neither read nor screenshotted from outside without this - and a pane that is not
		// on screen has nothing to scroll
		if (scroll[0])
		{
			xr_string why;
			if (!w->CanScroll(scroll, why)) { NqUtil::JsonError(out, NqUtil::Format("scroll=%s: %s", scroll, why.c_str()).c_str()); return; }
			NqInspector::RequestScroll(scroll);
		}
		// The answer is the last frame drawn. Until the frames this call changed are drawn it is
		// `pending`, and the MCP pump holds it and reads it again after them: a selection put in
		// the Details panel here would otherwise answer panes.node = null with the node on
		// screen a frame later. A scroll lands a frame after it is asked, and so does the node pane
		// scrolling the row of a newly selected action into view.
		if (asked || frame[0] || zoom >= 0 || has_c || select[0] || slot[0] || scroll[0]) w->OweFrames(scroll[0] || slot[0] ? 2 : 1);
		AppendView(w, out);
	}

	static void AppendView(UIQuestGraphWindow* w, xr_string& out)
	{
		NqCanvas* c = w->Canvas();
		out = "{\"ok\":true,\"path\":";
		NqUtil::JsonPath(out, w->Doc()->path.c_str());
		out += NqUtil::Format(",\"zoom_level\":%d,\"zoom\":%.3f,\"ui_scale\":%.2f,\"title_px\":%.0f,\"frame_min_level\":%d,\"center\":[%.1f,%.1f],\"nodes\":%d,\"open_tabs\":%d,\"slot\":",
			w->Doc()->zoom_idx, c->Zoom(), XUI::Dpi(), c->TitlePx(), NqCanvas::FrameMinLevel(), w->Doc()->view_cx, w->Doc()->view_cy,
			int(w->Doc()->quest.nodes.size()), Count());
		NqUtil::JsonString(out, w->Doc()->sel_slot);
		out += ",\"selected\":";
		NqUtil::JsonStringArray(out, w->Doc()->selection);
		out += ",\"keys\":";
		NqUtil::JsonBool(out, w->ShowKeys());
		out += ",\"problems\":";
		NqUtil::JsonBool(out, w->Drawer() == UIQuestGraphWindow::DrawerProblems);
		out += ",\"drawer\":";
		NqUtil::JsonString(out, w->DrawerName());
		out += ",\"graph_panel\":";
		NqUtil::JsonString(out, w->GraphPanelState());
		out += ",\"details\":";
		NqUtil::JsonBool(out, w->Details());
		out += ",\"minimap\":";
		NqUtil::JsonBool(out, c->MinimapShown());
		// nodes wholly past each edge of the view: what the canvas's edge pills count
		out += NqUtil::Format(",\"beyond\":{\"above\":%d,\"below\":%d,\"left\":%d,\"right\":%d}",
			c->Beyond(0), c->Beyond(1), c->Beyond(2), c->Beyond(3));
		// pt, as the last frame drew them
		out += NqUtil::Format(",\"graph_width\":%.0f,\"details_width\":%.0f,\"panes\":", w->GraphWidth(), w->DetailsWidth());
		w->AppendPanes(out);
		out += NqUtil::Format(",\"action_split\":%.3f,\"vars_split\":%.3f",
			w->Inspector()->ActionSplit(), w->Inspector()->VarsSplit());
		// as of the last frame drawn: a scroll asked for in this call shows in the next answer
		out += NqUtil::Format(",\"node_scroll\":[%.0f,%.0f],\"action_scroll\":[%.0f,%.0f],\"graph_scroll\":[%.0f,%.0f],\"variables_scroll\":[%.0f,%.0f]",
			w->Inspector()->NodeScroll().x, w->Inspector()->NodeScroll().y,
			w->Inspector()->ActionScroll().x, w->Inspector()->ActionScroll().y,
			w->Inspector()->GraphScroll().x, w->Inspector()->GraphScroll().y,
			w->Inspector()->VarsScroll().x, w->Inspector()->VarsScroll().y);
		out += ",\"pending\":";
		NqUtil::JsonBool(out, c->FramePending() || w->ViewPending());
		out += "}";
	}

	// Every graph edit the canvas offers, reachable without a mouse. The editor is
	// driven by agents as a first-class caller (CLAUDE.md), and a shortcut that only
	// answers to a keypress is a feature only a human has - which is the same as a
	// feature the editor does not really have.
	void McpEdit(LPCSTR raw, xr_string& out)
	{
		UIQuestGraphWindow* w = OpenMcpWindow(raw, out);
		if (!w) return;
		NqCanvas* c = w->Canvas();
		NqDoc* d = w->Doc();

		xr_string op;
		if (!NqUtil::ArgString(raw, "op", op) || op.empty())
		{
			NqUtil::JsonError(out, "missing 'op': connect, disconnect, bypass, delete, duplicate, connect_nearest, select_all");
			return;
		}

		// `nodes` (comma separated) overrides the selection for the ops that take one,
		// so a caller never has to select first and can leave the view alone
		xr_string nodes;
		NqUtil::ArgString(raw, "nodes", nodes);
		if (!nodes.empty())
		{
			xr_vector<xr_string> ids;
			xr_string rest = nodes;
			while (!rest.empty())
			{
				const size_t comma = rest.find(',');
				xr_string id = NqUtil::Trim((comma == xr_string::npos ? rest : rest.substr(0, comma)).c_str());
				rest = (comma == xr_string::npos) ? xr_string() : rest.substr(comma + 1);
				if (id.empty()) continue;
				if (!d->quest.FindNode(id.c_str()))
				{
					NqUtil::JsonError(out, NqUtil::Format("no node '%s' in this graph", id.c_str()).c_str());
					return;
				}
				ids.push_back(id);
			}
			c->SelectNodes(ids);
		}

		xr_string from, pin, to;
		NqUtil::ArgString(raw, "from", from);
		NqUtil::ArgString(raw, "pin", pin);
		NqUtil::ArgString(raw, "to", to);

		if (op == "connect")
		{
			if (from.empty() || pin.empty() || to.empty()) { NqUtil::JsonError(out, "connect needs 'from', 'pin' and 'to'"); return; }
			xr_string why;
			if (!c->CanLink(from.c_str(), to.c_str(), why)) { NqUtil::JsonError(out, why.c_str()); return; }
			xr_string err;
			if (!d->Connect(from.c_str(), pin.c_str(), to.c_str(), err)) { NqUtil::JsonError(out, err.empty() ? "connect refused" : err.c_str()); return; }
		}
		else if (op == "disconnect")
		{
			// no 'to' drops every target of the pin, no 'pin' drops every pin - the
			// same reach the context menu has
			if (from.empty()) { NqUtil::JsonError(out, "disconnect needs 'from' (plus 'pin' and 'to' to name one link)"); return; }
			xr_string err;
			if (!d->Disconnect(from.c_str(), pin.c_str(), to.c_str(), err)) { NqUtil::JsonError(out, err.empty() ? "disconnect refused" : err.c_str()); return; }
		}
		else if (op == "bypass")			c->BypassSelected();
		else if (op == "delete")			c->DeleteSelection();
		else if (op == "duplicate")			c->DuplicateSelection();
		else if (op == "connect_nearest")	c->ConnectNearest();
		else if (op == "select_all")		c->SelectAll();
		else
		{
			NqUtil::JsonError(out, NqUtil::Format("unknown op '%s': connect, disconnect, bypass, delete, duplicate, connect_nearest, select_all", op.c_str()).c_str());
			return;
		}

		// the outline is how a caller sees what the edit did, without a screenshot
		out = "{\"ok\":true,\"path\":";
		NqUtil::JsonPath(out, d->path.c_str());
		out += ",\"op\":";
		NqUtil::JsonString(out, op);
		out += ",\"selected\":";
		NqUtil::JsonStringArray(out, d->selection);
		out += ",\"status\":";
		NqUtil::JsonString(out, c->Status());
		out += ",\"outline\":";
		NqUtil::JsonString(out, d->Outline());
		out += "}";
	}

	void McpFind(LPCSTR raw, xr_string& out)
	{
		if (UIQuestGraphWindow* window = OpenMcpWindow(raw, out)) window->McpFind(raw, out);
	}

	void McpBookmarks(LPCSTR raw, xr_string& out)
	{
		if (UIQuestGraphWindow* window = OpenMcpWindow(raw, out)) window->McpBookmarks(raw, out);
	}

	void McpHistory(LPCSTR raw, xr_string& out)
	{
		if (UIQuestGraphWindow* window = OpenMcpWindow(raw, out)) window->McpHistory(raw, out);
	}

	void McpMinimap(LPCSTR raw, xr_string& out)
	{
		if (UIQuestGraphWindow* window = OpenMcpWindow(raw, out)) window->McpMinimap(raw, out);
	}
}
