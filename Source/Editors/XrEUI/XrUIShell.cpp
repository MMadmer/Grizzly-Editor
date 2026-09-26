#define IMGUI_DEFINE_MATH_OPERATORS
#include "stdafx.h"
#include "imgui_internal.h"
#include "XrUIKitInternal.h"
#include "XrUIShell.h"
#include <shellapi.h>

using namespace XUI::Internal;
using XUI::Shell::DocDesc;

namespace
{
struct Doc
{
	DocDesc d;
	bool loaded = true;			// false: restored from the session, the factory has not run yet
	bool failed = false;		// the factory could not open it
	bool loading = false;		// its factory runs now: the document it opens comes back through Open
	bool closing = false;		// removed inside a frame: freed at the next BeginFrame
	ImRect rect;				// the tab, last frame
	bool shown = false;			// in the strip, not under the chevron
};

xr_vector<Doc*> s_Docs;
xr_string s_Active;
xr_map<xr_string, XUI::Shell::Factory> s_Factories;
XUI::Shell::FrameHooks s_Hooks;

bool s_SessionDirty = false;
xr_string s_SessionFile;

int s_Drawer = XUI::Shell::DrawerNone;
bool s_DrawerFocus = false;
XUI::Shell::DrawerDesc s_Drawers[3];
ImRect s_DrawerRect;
ImRect s_StatusRect;

xr_vector<xr_string> s_KeyOwners;
bool s_FloatingPanels = false;
bool s_SkipLayout = false;

ImRect s_WorkRect;
int s_WorkAreaFrame = -1;		// the frame that last drew the work area: the shell's keys and drawers need one
float s_StatusRightW = 0.f;
float s_StatusCentreW = 0.f;
xr_string s_MenuTarget;			// the tab the context menu belongs to
xr_string s_DragTab;			// a tab being dragged to another place
int s_StripFrame = -1;			// the frame that drew the tab strip last (the Start page draws none)
bool s_Configured = false;
bool s_InFrame = false;
bool s_ClosePopups = false;		// MCP ui_popup show=none, done at the next frame

constexpr const char* kLevel = "level";

Doc* Find(const char* id)
{
	if (!id || !*id)
		return nullptr;
	for (Doc* doc : s_Docs)
		if (!doc->closing && doc->d.id == id)
			return doc;
	return nullptr;
}

Doc* ActiveDoc()
{
	return Find(s_Active.c_str());
}

bool Dirty(const Doc* doc)
{
	return doc->loaded && doc->d.dirty && doc->d.dirty();
}

bool Missing(const Doc* doc)
{
	return doc->failed || (doc->loaded && doc->d.missing && doc->d.missing());
}

// what the tab says: a document's own name without its extension (a placeholder's label is the session's)
xr_string Label(const Doc* doc)
{
	if (doc->d.label)
	{
		xr_string label = doc->d.label();
		if (!label.empty())
			return label;
	}
	const char* path = doc->d.path.c_str();
	const char* slash = strrchr(path, '\\');
	const char* fwd = strrchr(path, '/');
	if (fwd > slash)
		slash = fwd;
	xr_string name = slash ? slash + 1 : path;
	const size_t dot = name.rfind('.');
	if (dot != xr_string::npos && dot > 0)
		name.resize(dot);
	return name.empty() ? doc->d.id : name;
}

void MarkSession(const Doc* doc)
{
	if (!doc || doc->d.id != kLevel)
		s_SessionDirty = true;
}

// the neighbour to the left takes over, Level when there is none
void ActivateNeighbour(const Doc* leaving)
{
	Doc* next = nullptr;
	for (u32 i = 0; i < s_Docs.size(); ++i)
	{
		if (s_Docs[i] != leaving)
			continue;
		for (int j = int(i) - 1; j >= 0 && !next; --j)
			if (!s_Docs[j]->closing)
				next = s_Docs[j];
		for (u32 j = i + 1; j < s_Docs.size() && !next; ++j)
			if (!s_Docs[j]->closing)
				next = s_Docs[j];
	}
	s_Active.clear();
	if (next)
		XUI::Shell::Activate(next->d.id.c_str());
}

void Free(Doc* doc)
{
	s_Docs.erase(std::find(s_Docs.begin(), s_Docs.end(), doc));
	if (doc->loaded && doc->d.on_close)
		doc->d.on_close();
	xr_delete(doc);
}

// Removed at once between frames; inside one the tab only leaves the strip, and what it holds is freed at the
// next BeginFrame - the frame being drawn may still reference its textures.
void Remove(Doc* doc)
{
	const bool was_active = s_Active == doc->d.id;
	doc->closing = true;
	MarkSession(doc);
	if (was_active)
	{
		if (doc->loaded && doc->d.on_deactivate)
			doc->d.on_deactivate();
		ActivateNeighbour(doc);
	}
	if (!s_InFrame)
		Free(doc);
}

bool Load(Doc* doc)
{
	if (doc->loaded || doc->loading)
		return doc->loaded;
	doc->loading = true;
	auto it = s_Factories.find(doc->d.kind);
	const xr_string id = doc->d.id;
	// a placeholder keeps what the session gave its factory in its path
	const xr_string path = doc->d.path;
	if (it != s_Factories.end() && it->second && it->second(path.c_str()))
	{
		// the factory opened the document under the same id, which replaced the placeholder
		Doc* now = Find(id.c_str());
		if (now)
			now->loading = false;
		return now && now->loaded;
	}
	if (Doc* now = Find(id.c_str()))
	{
		now->loading = false;
		now->failed = true;
	}
	return false;
}

// ---- the session: which tabs a project had open ------------------------------------------------------
xr_string Field(xr_string& rest)
{
	const size_t bar = rest.find('|');
	xr_string head = bar == xr_string::npos ? rest : rest.substr(0, bar);
	rest = bar == xr_string::npos ? xr_string() : rest.substr(bar + 1);
	return head;
}

void SaveSession()
{
	s_SessionDirty = false;
	if (s_SessionFile.empty())
		return;
	xr_string text = "[session]\n";
	int n = 0;
	for (const Doc* doc : s_Docs)
	{
		if (doc->closing || doc->d.id == kLevel || s_Factories.find(doc->d.kind) == s_Factories.end())
			continue;
		char key[32];
		sprintf_s(key, "tab%d = ", n++);
		text += key;
		text += doc->d.id + "|" + doc->d.kind + "|";
		char icon[16];
		sprintf_s(icon, "%d", int(doc->d.icon));
		text += icon;
		xr_string arg;
		if (!doc->loaded)
			arg = doc->d.path;
		else if (doc->d.restore)
			arg = doc->d.restore();
		else
		{
			const size_t colon = doc->d.id.find(':');
			arg = colon == xr_string::npos ? xr_string() : doc->d.id.substr(colon + 1);
		}
		text += "|" + Label(doc) + "|" + arg + "\n";
	}
	// the folder is the editor's scratch; a project without one gets it
	xr_string folder = s_SessionFile;
	const size_t slash = folder.find_last_of("\\/");
	if (slash != xr_string::npos)
	{
		folder.resize(slash);
		::CreateDirectoryA(folder.c_str(), nullptr);
	}
	FILE* f = nullptr;
	if (fopen_s(&f, s_SessionFile.c_str(), "wb") || !f)
		return;
	fwrite(text.data(), 1, text.size(), f);
	fclose(f);
}

void LoadSession()
{
	FILE* f = nullptr;
	if (fopen_s(&f, s_SessionFile.c_str(), "rb") || !f)
		return;
	xr_string text;
	char chunk[4096];
	for (size_t n; (n = fread(chunk, 1, sizeof(chunk), f)) > 0;)
		text.append(chunk, n);
	fclose(f);

	size_t at = 0;
	while (at < text.size())
	{
		size_t end = text.find('\n', at);
		if (end == xr_string::npos)
			end = text.size();
		xr_string line = text.substr(at, end - at);
		at = end + 1;
		while (!line.empty() && (line.back() == '\r' || line.back() == ' '))
			line.pop_back();
		if (line.compare(0, 3, "tab"))
			continue;
		const size_t eq = line.find('=');
		if (eq == xr_string::npos)
			continue;
		xr_string rest = line.substr(eq + 1);
		while (!rest.empty() && rest[0] == ' ')
			rest.erase(0, 1);
		DocDesc d;
		d.id = Field(rest);
		d.kind = Field(rest);
		const int icon = atoi(Field(rest).c_str());
		const xr_string label = Field(rest);
		// what the factory opens, project-relative
		d.path = rest;
		if (d.id.empty() || d.id == kLevel || Find(d.id.c_str()) || s_Factories.find(d.kind) == s_Factories.end())
			continue;
		d.icon = icon > 0 && icon < int(XUI::Icon::Count) ? XUI::Icon(icon) : XUI::Icon::File;
		d.label = [label] { return label; };
		d.tooltip = [path = d.path] { return path; };
		Doc* doc = xr_new<Doc>();
		doc->d = d;
		doc->loaded = false;
		s_Docs.push_back(doc);
	}
}

// Another project: what the last one had open is written to its own session first - before its documents close
// with it, which would leave that session empty - and the new one's tabs come back, lazily. A tab of the old
// session that never opened has nothing to close and goes now. Run by the frame and by every entry point MCP
// reaches, so a background editor that has drawn nothing yet still reports and keeps the restored tabs; never in
// the middle of a frame, whose tab strip may be walking the list.
void SyncSession(bool from_frame)
{
	// Activate below comes back here
	static bool s_Syncing = false;
	if (s_Syncing || (s_InFrame && !from_frame))
		return;
	const xr_string session = s_Hooks.session_file ? s_Hooks.session_file() : xr_string();
	if (session == s_SessionFile)
		return;
	s_Syncing = true;
	const bool first = s_SessionFile.empty();
	if (s_SessionDirty)
		SaveSession();
	for (u32 i = 0; i < s_Docs.size();)
	{
		if (!s_Docs[i]->loaded)
		{
			// no neighbour takes over: it could be another placeholder of the old project
			if (s_Active == s_Docs[i]->d.id)
				s_Active.clear();
			xr_delete(s_Docs[i]);
			s_Docs.erase(s_Docs.begin() + i);
			continue;
		}
		++i;
	}
	s_SessionFile = session;
	s_SessionDirty = false;
	if (!session.empty())
	{
		LoadSession();
		// documents opened before the project's session was known are this project's: the file gets them too
		if (first)
			for (const Doc* doc : s_Docs)
				if (doc->loaded && !doc->closing && doc->d.id != kLevel)
					s_SessionDirty = true;
	}
	s_Syncing = false;
	if (s_Active.empty() && !s_Docs.empty() && s_Docs[0]->loaded)
		XUI::Shell::Activate(s_Docs[0]->d.id.c_str());
}

// ---- keys --------------------------------------------------------------------------------------------
bool Chord(ImGuiKey key, bool ctrl, bool shift = false)
{
	const ImGuiIO& io = ImGui::GetIO();
	return io.KeyCtrl == ctrl && io.KeyShift == shift && !io.KeyAlt && ImGui::IsKeyPressed(key, false);
}

bool ModalUp()
{
	return ImGui::GetTopMostPopupModal() != nullptr;
}

void SaveActiveDoc()
{
	Doc* doc = ActiveDoc();
	if (!doc || !doc->loaded || !doc->d.save)
		return;
	xr_string err;
	if (!doc->d.save(err) && !err.empty())
		XUI::Notice(XUI::Tone::Error, "Save failed", err.c_str());
}

void ShellKeys()
{
	const ImGuiIO& io = ImGui::GetIO();
	// the start page has no tabs to act on
	if (io.WantTextInput || ModalUp() || s_WorkAreaFrame < ImGui::GetFrameCount() - 1)
		return;
	if (Chord(ImGuiKey_P, true, true) && s_Hooks.palette)
		s_Hooks.palette();
	if (Chord(ImGuiKey_Tab, true))
		XUI::Shell::Cycle(1);
	if (Chord(ImGuiKey_Tab, true, true))
		XUI::Shell::Cycle(-1);
	if (Chord(ImGuiKey_F4, true))
		XUI::Shell::RequestClose(s_Active.c_str());
	if (Chord(ImGuiKey_Space, true))
		XUI::Shell::ToggleDrawer(XUI::Shell::DrawerContent);
	if (Chord(ImGuiKey_GraveAccent, true))
		XUI::Shell::ToggleDrawer(XUI::Shell::DrawerLog);
	if (Chord(ImGuiKey_S, true))
		SaveActiveDoc();
	if (Chord(ImGuiKey_F11, false) && s_Active == kLevel && s_Hooks.level_maximize)
		s_Hooks.level_maximize();

	// the document's own keys, after the shell's
	Doc* doc = ActiveDoc();
	if (!doc || !doc->loaded || !doc->d.shortcut)
		return;
	const int mods = (io.KeyCtrl ? ImGuiMod_Ctrl : 0) | (io.KeyShift ? ImGuiMod_Shift : 0) | (io.KeyAlt ? ImGuiMod_Alt : 0);
	for (int key = ImGuiKey_NamedKey_BEGIN; key < ImGuiKey_NamedKey_END; ++key)
	{
		if (key >= ImGuiKey_ReservedForModCtrl && key <= ImGuiKey_ReservedForModSuper)
			continue;
		if (ImGui::IsKeyPressed(ImGuiKey(key), false) && doc->d.shortcut(key, mods))
			break;
	}
}

// ---- painting ----------------------------------------------------------------------------------------
void DrawDirtyDot(ImDrawList* dl, const ImVec2& centre, ImU32 col)
{
	dl->AddCircleFilled(centre, XUI::Pt(3.5f), col, 12);
}

// a small ghost button of the chrome rows; true on click
bool ChromeButton(const char* id, XUI::Icon icon, const char* text, const char* tip, float h, bool checked = false,
	bool on = true, XUI::Font font = XUI::Font::Small, ImRect* out = nullptr)
{
	BtnSpec s;
	s.id = id;
	s.text = text ? text : "";
	s.icon = icon;
	s.icon_pt = 14.f;
	s.kind = XUI::Kind::Ghost;
	s.font = font;
	s.tip = tip;
	s.on = on;
	s.checked = checked;
	s.size = ImVec2(0.f, h);
	return ButtonEx(s, out);
}
}

// =======================================================================================================
// Documents
// =======================================================================================================
void XUI::Shell::Open(const DocDesc& desc, bool activate)
{
	if (desc.id.empty())
		return;
	SyncSession(false);
	Doc* doc = Find(desc.id.c_str());
	if (doc)
	{
		doc->d = desc;
		doc->loaded = true;
		doc->failed = false;
	}
	else
	{
		doc = xr_new<Doc>();
		doc->d = desc;
		// Level is always the first tab
		if (desc.id == kLevel)
			s_Docs.insert(s_Docs.begin(), doc);
		else
			s_Docs.push_back(doc);
		MarkSession(doc);
	}
	if (activate || s_Active.empty())
		Activate(desc.id.c_str());
}

bool XUI::Shell::Activate(cstr id)
{
	SyncSession(false);
	Doc* doc = Find(id);
	if (!doc)
		return false;
	if (s_Hooks.tabs_locked && s_Hooks.tabs_locked() && doc->d.id != kLevel)
		return false;
	if (s_Active == doc->d.id)
	{
		if (!doc->loaded)
			Load(doc);
		return true;
	}
	if (Doc* old = ActiveDoc())
		if (old->loaded && old->d.on_deactivate)
			old->d.on_deactivate();
	// a drawer was opened over the tab that goes: it must not cover the next one's canvas
	CloseDrawer();
	s_Active = doc->d.id;
	if (!doc->loaded)
	{
		const xr_string keep = s_Active;
		Load(doc);
		doc = Find(keep.c_str());
		if (!doc)
			return false;
	}
	if (doc->loaded && doc->d.on_activate)
		doc->d.on_activate();
	return true;
}

XUI::cstr XUI::Shell::Active()
{
	return s_Active.c_str();
}

bool XUI::Shell::IsActive(cstr id)
{
	return id && s_Active == id;
}

bool XUI::Shell::IsOpen(cstr id)
{
	// a tab the session restored holds no document yet: its owner opens it through Open
	const Doc* doc = Find(id);
	return doc && doc->loaded;
}

void XUI::Shell::RequestClose(cstr id)
{
	Doc* doc = Find(id);
	if (!doc || !doc->d.closable)
		return;
	if (!Dirty(doc))
	{
		Remove(doc);
		return;
	}
	const xr_string target = doc->d.id;
	Strings what;
	what.push_back(Label(doc));
	UnsavedPrompt("close_tab", what, [target](Answer answer)
	{
		if (answer != Answer::Cancel)
			Close(target.c_str(), answer == Answer::Yes ? SaveFirst : Discard, nullptr);
	});
}

bool XUI::Shell::Close(cstr id, CloseMode mode, xr_string* err)
{
	Doc* doc = Find(id);
	if (!doc)
	{
		if (err)
			*err = "not open";
		return false;
	}
	if (!doc->d.closable)
	{
		if (err)
			*err = "the Level tab does not close";
		return false;
	}
	if (Dirty(doc))
	{
		if (mode == IfClean)
		{
			if (err)
				*err = "unsaved";
			return false;
		}
		if (mode == SaveFirst)
		{
			xr_string e;
			if (!doc->d.save || !doc->d.save(e))
			{
				if (e.empty())
					e = "it cannot be saved";
				if (err)
					*err = e;
				else
					Notice(Tone::Error, "Save failed", e.c_str());
				return false;
			}
		}
		else if (doc->d.discard)
			doc->d.discard();
	}
	Remove(doc);
	return true;
}

bool XUI::Shell::SaveTab(cstr id, xr_string* err)
{
	Doc* doc = Find(id);
	if (!doc)
	{
		if (err)
			*err = "not open";
		return false;
	}
	if (!Dirty(doc))
		return true;
	xr_string e;
	if (!doc->d.save || !doc->d.save(e))
	{
		if (err)
			*err = e.empty() ? xr_string("it cannot be saved") : e;
		return false;
	}
	return true;
}

bool XUI::Shell::DiscardTab(cstr id)
{
	Doc* doc = Find(id);
	if (!doc)
		return false;
	if (Dirty(doc) && doc->d.discard)
		doc->d.discard();
	return !Dirty(doc);
}

void XUI::Shell::List(xr_vector<DocInfo>& out)
{
	SyncSession(false);
	out.clear();
	for (const Doc* doc : s_Docs)
	{
		if (doc->closing)
			continue;
		DocInfo& info = out.emplace_back();
		info.id = doc->d.id;
		info.kind = doc->d.kind;
		info.label = Label(doc);
		info.path = doc->d.path;
		info.icon = doc->d.icon;
		info.dirty = Dirty(doc);
		info.active = s_Active == doc->d.id;
		info.missing = Missing(doc);
		info.closable = doc->d.closable;
		info.loaded = doc->loaded;
	}
}

bool XUI::Shell::Info(cstr id, DocInfo& out)
{
	xr_vector<DocInfo> all;
	List(all);
	for (DocInfo& info : all)
		if (info.id == id)
		{
			out = info;
			return true;
		}
	return false;
}

void XUI::Shell::Cycle(int step)
{
	xr_vector<Doc*> open;
	for (Doc* doc : s_Docs)
		if (!doc->closing)
			open.push_back(doc);
	if (open.size() < 2)
		return;
	int at = 0;
	for (int i = 0; i < int(open.size()); ++i)
		if (open[i]->d.id == s_Active)
			at = i;
	at = (at + step + int(open.size())) % int(open.size());
	Activate(open[at]->d.id.c_str());
}

void XUI::Shell::RegisterFactory(cstr kind, Factory fn)
{
	if (kind && *kind)
		s_Factories[kind] = std::move(fn);
}

void XUI::Shell::SaveActive()
{
	SaveActiveDoc();
}

bool XUI::Shell::Edit(cstr action, bool run)
{
	Doc* doc = ActiveDoc();
	return doc && doc->loaded && !Missing(doc) && doc->d.edit && doc->d.edit(action, run);
}

void XUI::Shell::DrawDocMenus()
{
	Doc* doc = ActiveDoc();
	if (doc && doc->loaded && !Missing(doc) && doc->d.menus)
		doc->d.menus();
}

void XUI::Shell::RequestExit()
{
	Strings what;
	xr_vector<xr_string> ids;
	for (const Doc* doc : s_Docs)
		if (!doc->closing && Dirty(doc))
		{
			what.push_back(Label(doc));
			ids.push_back(doc->d.id);
		}
	if (what.empty())
	{
		if (s_Hooks.quit)
			s_Hooks.quit();
		return;
	}
	UnsavedPrompt("exit", what, [ids](Answer answer)
	{
		if (answer == Answer::Cancel)
			return;
		for (const xr_string& id : ids)
		{
			Doc* doc = Find(id.c_str());
			if (!doc || !Dirty(doc))
				continue;
			if (answer == Answer::Yes)
			{
				xr_string err;
				if (!doc->d.save || !doc->d.save(err))
				{
					Notice(Tone::Error, "Save failed", (Label(doc) + ": " + (err.empty() ? xr_string("it cannot be saved") : err)).c_str());
					return;
				}
			}
			else if (doc->d.discard)
				doc->d.discard();
		}
		if (s_Hooks.quit)
			s_Hooks.quit();
	});
}

void XUI::Shell::FlushSession()
{
	SyncSession(false);
	if (s_SessionDirty)
		SaveSession();
}

void XUI::Shell::CloseProjectTabs()
{
	FlushSession();
	for (u32 i = 0; i < s_Docs.size();)
	{
		Doc* doc = s_Docs[i];
		if (doc->d.id == kLevel || doc->closing || Dirty(doc))
		{
			++i;
			continue;
		}
		if (s_Active == doc->d.id)
		{
			if (doc->loaded && doc->d.on_deactivate)
				doc->d.on_deactivate();
			s_Active.clear();
		}
		// the session already holds it: it leaves without being written out of it
		if (s_InFrame)
		{
			doc->closing = true;
			++i;
		}
		else
			Free(doc);
	}
	s_SessionDirty = false;
	if (s_Active.empty())
		Activate(kLevel);
}

XUI::Shell::FrameHooks& XUI::Shell::Hooks()
{
	return s_Hooks;
}

// =======================================================================================================
// The frame
// =======================================================================================================
void XUI::Shell::BeginFrame()
{
	ImGuiContext& g = *GImGui;
	if (!s_Configured)
	{
		s_Configured = true;
		// Ctrl+Tab cycles the major tabs, not ImGui's window list
		g.ConfigNavWindowingKeyNext = 0;
		g.ConfigNavWindowingKeyPrev = 0;
	}
	s_InFrame = true;
	SyncSession(true);

	// what a tab closed inside the last frame held is free to go now
	for (u32 i = 0; i < s_Docs.size();)
	{
		if (s_Docs[i]->closing)
		{
			Free(s_Docs[i]);
			continue;
		}
		++i;
	}
	// an owner that closed its document took the tab with it
	for (u32 i = 0; i < s_Docs.size();)
	{
		Doc* doc = s_Docs[i];
		if (doc->loaded && doc->d.alive && !doc->d.alive())
		{
			if (s_Active == doc->d.id)
				ActivateNeighbour(doc);
			MarkSession(doc);
			Free(doc);
			continue;
		}
		++i;
	}
	if (s_Active.empty() && !s_Docs.empty())
		Activate(s_Docs[0]->d.id.c_str());

	// play in editor: Level holds the screen
	if (s_Hooks.tabs_locked && s_Hooks.tabs_locked() && s_Active != kLevel)
		Activate(kLevel);

	// what was put up only to be looked at goes; a modal stays
	if (s_ClosePopups)
	{
		s_ClosePopups = false;
		ImGui::ClosePopupsExceptModals();
	}

	ShellKeys();
}

void XUI::Shell::EndFrame()
{
	// one host a frame: a drawer whose panel is on screen (Level came up with it docked) gives way to it
	if (s_Drawer != DrawerNone && s_Drawers[s_Drawer].panel_visible && s_Drawers[s_Drawer].panel_visible())
		CloseDrawer();
	// ---- the drawer over the bottom of the work area ------------------------------------------------
	if (s_Drawer != DrawerNone && s_Drawers[s_Drawer].body && s_WorkAreaFrame == ImGui::GetFrameCount() && s_WorkRect.GetWidth() > 0.f)
	{
		const DrawerDesc& desc = s_Drawers[s_Drawer];
		const float work_h = s_WorkRect.GetHeight();
		const float h = ImMin(ImClamp(work_h * .40f, Pt(260.f), Pt(480.f)), work_h);
		const ImVec2 pos(s_WorkRect.Min.x, s_WorkRect.Max.y - h);
		ImGui::SetNextWindowPos(pos);
		ImGui::SetNextWindowSize(ImVec2(s_WorkRect.GetWidth(), h));
		ImGui::SetNextWindowViewport(ImGui::GetMainViewport()->ID);
		if (s_DrawerFocus)
		{
			ImGui::SetNextWindowFocus();
			s_DrawerFocus = false;
		}
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(8.f, 0.f));
		ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 0.f);
		ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
		ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Base));
		const char* name = s_Drawer == DrawerContent ? "Content Drawer###drawer.content" : "Output Log###drawer.log";
		const bool open = ImGui::Begin(name, nullptr, ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoMove |
			ImGuiWindowFlags_NoCollapse | ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoScrollbar |
			ImGuiWindowFlags_NoScrollWithMouse);
		ImGui::PopStyleColor();
		ImGui::PopStyleVar(3);
		bool close = false;
		if (open)
		{
			ImGuiWindow* window = ImGui::GetCurrentWindow();
			s_DrawerRect = window->Rect();
			ImDrawList* dl = window->DrawList;
			dl->AddLine(window->Pos, ImVec2(window->Pos.x + window->Size.x, window->Pos.y), Col(Token::BorderStrong), ImMax(1.f, Pt(1.f)));
			// the header: what it is, and the way out
			const float head = Pt(32.f);
			const ImVec2 origin = window->DC.CursorPos;
			if (desc.icon != Icon::None)
				DrawIcon(dl, desc.icon, ImVec2(IM_ROUND(origin.x), IM_ROUND(origin.y + (head - Pt(16.f)) * .5f)), 16.f, Col(Token::TextSecondary));
			PaintText(dl, Font::Strong, ImVec2(IM_ROUND(origin.x + Pt(24.f)), IM_ROUND(origin.y + (head - FontPx(Font::Strong)) * .5f)),
				Col(Token::Text), desc.label ? desc.label : "");
			ImGui::SetCursorScreenPos(ImVec2(origin.x + window->Size.x - Pt(16.f) - Pt(24.f), origin.y + (head - Pt(24.f)) * .5f));
			BtnSpec s;
			s.id = "##drawer_close";
			s.text = "";
			s.icon = Icon::Close;
			s.kind = Kind::Ghost;
			s.tip = "Close the drawer";
			s.keys = "Esc";
			s.size = PtV(24.f, 24.f);
			// a header item, like a dialog's X: never the focus the drawer opens with (that is its body), never a Tab stop
			ImGui::PushItemFlag(ImGuiItemFlags_NoNavDefaultFocus | ImGuiItemFlags_NoTabStop, true);
			if (ButtonEx(s))
				close = true;
			ImGui::PopItemFlag();
			ImGui::SetCursorScreenPos(ImVec2(origin.x, origin.y + head));
			ImGui::BeginChild("##drawer_body", ImVec2(0.f, 0.f), ImGuiChildFlags_None, ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse);
			desc.body();
			ImGui::EndChild();
		}
		const bool focused = ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows);
		ImGui::End();

		// Esc, a click outside it (the status bar's own buttons toggle it), or a drag that leaves it
		const ImGuiIO& io = ImGui::GetIO();
		const ImVec2 mouse = io.MousePos;
		const bool outside = !s_DrawerRect.Contains(mouse) && !s_StatusRect.Contains(mouse);
		if (focused && !io.WantTextInput && ImGui::IsKeyPressed(ImGuiKey_Escape, false) && !ModalUp())
			close = true;
		if (outside && !ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopupId | ImGuiPopupFlags_AnyPopupLevel) &&
			(ImGui::IsMouseClicked(ImGuiMouseButton_Left) || ImGui::IsMouseClicked(ImGuiMouseButton_Right)))
			close = true;
		if (ImGui::GetDragDropPayload() && !s_DrawerRect.Contains(mouse))
			close = true;
		if (close)
			CloseDrawer();
	}
	else
		s_DrawerRect = ImRect();

	if (s_SessionDirty)
		SaveSession();
	s_InFrame = false;
}

// ---- caption --------------------------------------------------------------------------------------------
bool XUI::Shell::BeginCaption()
{
	// 16 pt of text in 32 pt of bar
	ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, PtV(8.f, 8.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_MenuBarBg, Col(Token::Chrome));
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Chrome));
	const bool open = ImGui::BeginMainMenuBar();
	ImGui::PopStyleColor(2);
	ImGui::PopStyleVar(2);
	return open;
}

void XUI::Shell::EndCaption()
{
	ImGui::EndMainMenuBar();
}

float XUI::Shell::CaptionHeight()
{
	return Pt(32.f);
}

float XUI::Shell::DrawTabStrip(float max_x, bool locked)
{
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return window->DC.CursorPos.x;
	s_StripFrame = ImGui::GetFrameCount();
	ImDrawList* dl = window->DrawList;
	const float bar_top = window->Pos.y;
	const float bar_h = window->Size.y;
	const float tab_h = ImMin(Pt(28.f), bar_h);
	const float y0 = bar_top + bar_h - tab_h;
	const float x_start = window->DC.CursorPos.x + Pt(8.f);
	const float chevron = Pt(24.f);
	float avail = ImMax(0.f, max_x - x_start - chevron - Pt(4.f));
	locked |= s_Hooks.tabs_locked && s_Hooks.tabs_locked();

	// ---- widths: what each tab wants, shrunk evenly toward 96 pt, the rest under the chevron --------------
	xr_vector<Doc*> docs;
	for (Doc* doc : s_Docs)
		if (!doc->closing)
			docs.push_back(doc);
	const float pad_l = Pt(10.f), pad_r = Pt(6.f), icon = Pt(16.f), gap = Pt(6.f), slot = Pt(16.f);
	const float w_min = Pt(96.f), w_max = Pt(220.f);
	xr_vector<float> want(docs.size());
	xr_vector<xr_string> labels(docs.size());
	for (u32 i = 0; i < docs.size(); ++i)
	{
		labels[i] = Label(docs[i]);
		const float text = TextSize(Font::Body, labels[i].c_str()).x;
		want[i] = ImClamp(pad_l + icon + gap + text + gap + slot + pad_r, w_min, w_max);
	}
	xr_vector<float> width = want;
	float total = 0.f;
	for (float w : want)
		total += w;
	const float spacing = Pt(1.f);
	total += spacing * float(docs.size());
	int active_i = -1;
	for (u32 i = 0; i < docs.size(); ++i)
		if (docs[i]->d.id == s_Active)
			active_i = int(i);
	// tabs that do not fit even at 96 go under the chevron, which then says how many: it takes that room from the strip
	float at_min = spacing * float(docs.size());
	for (u32 i = 0; i < docs.size(); ++i)
		at_min += int(i) == active_i ? want[i] : ImMin(want[i], w_min);
	const bool overflows = at_min > avail;
	BtnSpec more;
	more.id = "##tab_overflow";
	more.kind = Kind::Ghost;
	more.font = Font::Small;
	more.caret = true;
	more.text = "+99";
	if (overflows)
		avail = ImMax(0.f, avail - (ButtonWidth(more) - chevron));
	if (total > avail && !docs.empty())
	{
		// the tab you are on keeps its whole name (Level beside it keeps 96 at least); the others give way, the widest
		// first, never below 96
		if (active_i >= 0)
			width[active_i] = ImMin(want[active_i], ImMax(w_min, avail - (active_i ? w_min + spacing : 0.f) - spacing));
		const float rest = avail - (active_i >= 0 ? width[active_i] + spacing : 0.f);
		float lo = w_min, hi = w_max;
		for (int it = 0; it < 24; ++it)
		{
			const float cap = (lo + hi) * .5f;
			float sum = 0.f;
			for (u32 i = 0; i < docs.size(); ++i)
				if (int(i) != active_i)
					sum += ImMin(want[i], cap) + spacing;
			(sum > rest ? hi : lo) = cap;
		}
		for (u32 i = 0; i < docs.size(); ++i)
			if (int(i) != active_i)
				width[i] = ImMin(want[i], lo);
	}
	// what still does not fit goes under the chevron; the active tab always stays in the strip
	float used = 0.f;
	for (u32 i = 0; i < docs.size(); ++i)
	{
		docs[i]->shown = used + width[i] + spacing <= avail || i == 0;
		if (docs[i]->shown)
			used += width[i] + spacing;
	}
	for (u32 i = 0; i < docs.size(); ++i)
	{
		if (docs[i]->d.id != s_Active || docs[i]->shown)
			continue;
		for (int j = int(docs.size()) - 1; j > 0; --j)
			if (docs[j]->shown)
			{
				docs[j]->shown = false;
				used -= width[j] + spacing;
				if (used + width[i] + spacing <= avail)
					break;
			}
		docs[i]->shown = true;
		used += width[i] + spacing;
	}

	// ---- the names: a tab shows its slot only with something in it - the close cross (hovered or active) or the dirty
	// dot; otherwise the name takes the slot's room too, which at 96 pt is a word instead of "Unti...". Names that would
	// still cut to the same text are cut in the middle, so their ends tell them apart.
	const float dot_room = Pt(3.5f) + Pt(4.f);
	auto text_room = [&](float w, bool cross, bool dot)
	{
		const float end = cross ? w - pad_r - slot - gap : dot ? w - pad_r - slot * .5f - dot_room : w - pad_r;
		return ImMax(0.f, end - pad_l - icon - gap);
	};
	xr_vector<xr_string> cut_end(docs.size());
	xr_vector<char> middle(docs.size(), 0);
	for (u32 i = 0; i < docs.size(); ++i)
		if (docs[i]->shown)
		{
			const bool cross = docs[i]->d.closable && docs[i]->d.id == s_Active && !locked;
			if (!Ellipsize(Font::Body, labels[i].c_str(), text_room(width[i], cross, Dirty(docs[i])), Clip::End, cut_end[i]))
				cut_end[i] = labels[i];
		}
	for (u32 i = 0; i < docs.size(); ++i)
		for (u32 j = 0; j < docs.size() && !middle[i]; ++j)
			middle[i] = j != i && docs[i]->shown && docs[j]->shown && cut_end[i] == cut_end[j] && labels[i] != labels[j];

	// ---- the tabs ---------------------------------------------------------------------------------------
	ImGui::PushID("##major_tabs");
	float x = x_start;
	xr_string activate, close, menu;
	for (u32 i = 0; i < docs.size(); ++i)
	{
		Doc* doc = docs[i];
		if (!doc->shown)
		{
			doc->rect = ImRect();
			continue;
		}
		const bool active = s_Active == doc->d.id;
		const ImRect bb(ImVec2(IM_ROUND(x), IM_ROUND(y0)), ImVec2(IM_ROUND(x + width[i]), IM_ROUND(bar_top + bar_h)));
		doc->rect = bb;
		x += width[i] + spacing;

		ImGui::PushID(doc->d.id.c_str());
		ImGui::SetCursorScreenPos(bb.Min);
		ImGui::SetNextItemAllowOverlap();
		ImGui::InvisibleButton("##tab", bb.GetSize(), ImGuiButtonFlags_MouseButtonLeft | ImGuiButtonFlags_MouseButtonMiddle |
			ImGuiButtonFlags_MouseButtonRight | ImGuiButtonFlags_PressedOnClick);
		// the close cross over it takes the hover, the tab still looks hovered
		const bool hovered = ImGui::IsItemHovered(ImGuiHoveredFlags_AllowWhenOverlappedByItem);
		if (ImGui::IsItemClicked(ImGuiMouseButton_Left) && !locked)
			activate = doc->d.id;
		if (ImGui::IsItemClicked(ImGuiMouseButton_Middle) && doc->d.closable && !locked)
			close = doc->d.id;
		if (ImGui::IsItemClicked(ImGuiMouseButton_Right) && !locked)
			menu = doc->d.id;
		// dragging reorders; Level keeps the first place
		if (ImGui::IsItemActive() && ImGui::IsMouseDragging(ImGuiMouseButton_Left) && doc->d.id != kLevel && !locked)
			s_DragTab = doc->d.id;
		const bool tip = hovered && ImGui::IsItemHovered(ImGuiHoveredFlags_ForTooltip) && !HoverBlocked();

		// the one slot: the close cross on hover and on the active tab, the dirty dot otherwise
		const bool dirty = Dirty(doc);
		const ImRect slot_bb(ImVec2(bb.Max.x - pad_r - slot, bb.Min.y + (tab_h - slot) * .5f),
			ImVec2(bb.Max.x - pad_r, bb.Min.y + (tab_h + slot) * .5f));
		bool slot_hovered = false;
		const bool show_close = doc->d.closable && (hovered || active) && !locked;
		if (show_close)
		{
			ImGui::SetCursorScreenPos(slot_bb.Min);
			if (ImGui::InvisibleButton("##close", slot_bb.GetSize()))
				close = doc->d.id;
			slot_hovered = ImGui::IsItemHovered();
		}

		// paint
		if (active || hovered)
			dl->AddRectFilled(bb.Min, bb.Max, Col(active ? Token::Base : Token::Hover), Pt(4.f), ImDrawFlags_RoundCornersTop);
		if (active)
			dl->AddRectFilled(ImVec2(bb.Min.x + Pt(1.f), bb.Min.y), ImVec2(bb.Max.x - Pt(1.f), bb.Min.y + Pt(2.f)), Col(Token::AccentBright));
		const ImU32 text_col = Col(active ? Token::Text : Token::TextSecondary);
		DrawIcon(dl, doc->d.icon, ImVec2(IM_ROUND(bb.Min.x + pad_l), IM_ROUND(bb.Min.y + (tab_h - icon) * .5f)), 16.f,
			Col(active ? Token::AccentBright : Token::TextSecondary));
		const float text_x = bb.Min.x + pad_l + icon + gap;
		const float text_w = text_room(bb.GetWidth(), show_close, dirty);
		xr_string shown;
		const bool cut = Ellipsize(Font::Body, labels[i].c_str(), text_w, middle[i] ? Clip::Middle : Clip::End, shown);
		PaintText(dl, Font::Body, ImVec2(IM_ROUND(text_x), IM_ROUND(bb.Min.y + (tab_h - FontPx(Font::Body)) * .5f)), text_col,
			cut ? shown.c_str() : labels[i].c_str());
		if (show_close)
		{
			if (slot_hovered)
				dl->AddRectFilled(slot_bb.Min, slot_bb.Max, Col(Token::Pressed), Pt(3.f));
			DrawIcon(dl, Icon::Close, ImVec2(IM_ROUND(slot_bb.Min.x + Pt(2.f)), IM_ROUND(slot_bb.Min.y + Pt(2.f))), 12.f,
				Col(slot_hovered ? Token::Text : Token::TextSecondary));
		}
		else if (dirty)
			DrawDirtyDot(dl, slot_bb.GetCenter(), doc->d.id == kLevel ? ToneColor(Tone::Warning) : Col(Token::Text));
		if (tip)
		{
			xr_string text = doc->loaded && doc->d.tooltip ? doc->d.tooltip() : doc->d.path;
			if (text.empty())
				text = labels[i];
			if (dirty)
				text += doc->d.id == kLevel ? "\nThe scene has unsaved changes." : "\nUnsaved changes.";
			if (Missing(doc))
				text += "\nFile not found.";
			TooltipNow(text.c_str(), nullptr, labels[i].c_str());
		}
		AuditAdd("control", labels[i].c_str(), bb, nullptr, !locked, nullptr, "a major tab");
		ImGui::PopID();
	}

	// the drag: over the middle of a neighbour, the two swap
	if (!s_DragTab.empty())
	{
		if (!ImGui::IsMouseDown(ImGuiMouseButton_Left))
			s_DragTab.clear();
		else if (Doc* dragged = Find(s_DragTab.c_str()))
		{
			const float mx = ImGui::GetIO().MousePos.x;
			for (u32 i = 1; i < s_Docs.size(); ++i)
			{
				Doc* other = s_Docs[i];
				if (other == dragged || other->closing || !other->shown || other->rect.GetWidth() <= 0.f)
					continue;
				const float mid = other->rect.GetCenter().x;
				const auto a = std::find(s_Docs.begin(), s_Docs.end(), dragged);
				const auto b = s_Docs.begin() + i;
				if ((a < b && mx > mid) || (a > b && mx < mid))
				{
					std::iter_swap(a, b);
					s_SessionDirty = true;
					break;
				}
			}
		}
	}

	// ---- the chevron: every tab, those under it among them ------------------------------------------
	const ImGuiID overflow_id = window->GetID("##tab_overflow_popup");
	{
		ImGui::SetCursorScreenPos(ImVec2(IM_ROUND(x + Pt(2.f)), IM_ROUND(y0 + (tab_h - chevron) * .5f)));
		int hidden = 0;
		for (const Doc* doc : docs)
			hidden += !doc->shown;
		// tabs under it: how many, "+3 v"; none: the plain chevron that lists every tab
		char count[16], tip[48];
		sprintf_s(count, "+%d", hidden);
		sprintf_s(tip, hidden == 1 ? "%d more tab" : "%d more tabs", hidden);
		BtnSpec& s = more;
		s.text = hidden ? count : "";
		s.caret = hidden > 0;
		s.icon = hidden ? Icon::None : Icon::ChevronDown;
		s.tip = hidden ? tip : "Open tabs";
		s.size = ImVec2(hidden ? 0.f : chevron, chevron);
		s.on = !locked;
		s.checked = ImGui::IsPopupOpen(overflow_id, ImGuiPopupFlags_None);
		ImRect bb;
		if (ButtonEx(s, &bb) || TakePopupRequest("tab_overflow"))
			XUI::Internal::OpenPopupAt(overflow_id, ImVec2(bb.Min.x, bb.Max.y + Pt(2.f)));
		x = bb.Max.x;
	}
	if (ImGui::BeginPopupEx(overflow_id, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings))
	{
		for (Doc* doc : docs)
		{
			ImGui::PushID(doc->d.id.c_str());
			const xr_string label = Label(doc);
			const float row_w = ImMax(Em(14.f), Pt(8.f + 16.f + 8.f + 8.f + 16.f) + TextSize(Font::Body, label.c_str()).x);
			if (ImGui::Selectable("##row", s_Active == doc->d.id, 0, ImVec2(row_w, Pt(24.f))))
				activate = doc->d.id;
			const ImRect rb(ImGui::GetItemRectMin(), ImGui::GetItemRectMax());
			ImDrawList* pdl = ImGui::GetWindowDrawList();
			DrawIcon(pdl, doc->d.icon, ImVec2(IM_ROUND(rb.Min.x + Pt(4.f)), IM_ROUND(rb.GetCenter().y - Pt(8.f))), 16.f, Col(Token::TextSecondary));
			PaintText(pdl, Font::Body, ImVec2(IM_ROUND(rb.Min.x + Pt(28.f)), IM_ROUND(rb.GetCenter().y - FontPx(Font::Body) * .5f)),
				Col(doc->shown ? Token::TextSecondary : Token::Text), label.c_str());
			if (Dirty(doc))
				DrawDirtyDot(pdl, ImVec2(rb.Max.x - Pt(10.f), rb.GetCenter().y), Col(Token::Text));
			if (!doc->d.path.empty())
				Tip(doc->d.path.c_str());
			ImGui::PopID();
		}
		ImGui::EndPopup();
	}

	// ---- the tab menu -------------------------------------------------------------------------------------
	const ImGuiID menu_id = window->GetID("##tab_menu_popup");
	for (Doc* doc : docs)
	{
		xr_string name = "tab_menu:" + doc->d.id;
		if (TakePopupRequest(name.c_str()))
			menu = doc->d.id;
	}
	if (!menu.empty())
	{
		s_MenuTarget = menu;
		XUI::Internal::OpenPopupAt(menu_id, ImGui::GetIO().MousePos);
		if (Doc* doc = Find(menu.c_str()))
			if (doc->rect.GetWidth() > 0.f && !doc->rect.Contains(ImGui::GetIO().MousePos))
				XUI::Internal::OpenPopupAt(menu_id, ImVec2(doc->rect.Min.x, doc->rect.Max.y));
	}
	if (ImGui::BeginPopupEx(menu_id, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings))
	{
		Doc* doc = Find(s_MenuTarget.c_str());
		if (!doc)
			ImGui::CloseCurrentPopup();
		else
		{
			const xr_string target = doc->d.id;
			const xr_string path = doc->d.path;
			if (MenuItem("Close", "Ctrl+F4", false, doc->d.closable))
				close = target;
			if (MenuItem("Close Others"))
				for (Doc* other : docs)
					if (other->d.id != target && other->d.closable)
						RequestClose(other->d.id.c_str());
			if (MenuItem("Close All"))
				for (Doc* other : docs)
					if (other->d.closable)
						RequestClose(other->d.id.c_str());
			if (MenuItem("Close Saved"))
				for (Doc* other : docs)
					if (other->d.closable && !Dirty(other))
						RequestClose(other->d.id.c_str());
			ImGui::Separator();
			if (MenuItem("Copy Path", nullptr, false, !path.empty()))
				ImGui::SetClipboardText(path.c_str());
			if (MenuItem("Show in Content Browser", nullptr, false, !path.empty() && !!s_Hooks.reveal_in_browser))
				s_Hooks.reveal_in_browser(path.c_str());
			if (MenuItem("Show in Explorer", nullptr, false, !path.empty()))
			{
				wchar_t wide[1024] = {};
				xr_string args = "/select,\"" + path + "\"";
				for (char& c : args)
					if (c == '/')
						c = '\\';
				::MultiByteToWideChar(CP_UTF8, 0, args.c_str(), -1, wide, 1024);
				::ShellExecuteW(nullptr, L"open", L"explorer.exe", wide, nullptr, SW_SHOWNORMAL);
			}
		}
		ImGui::EndPopup();
	}
	ImGui::PopID();

	if (!activate.empty())
		Activate(activate.c_str());
	if (!close.empty())
		RequestClose(close.c_str());
	return x;
}

void XUI::Shell::TabRects(xr_vector<ImVec4>& out, xr_vector<xr_string>* ids)
{
	out.clear();
	if (ids)
		ids->clear();
	// no strip on screen (the Start page): no tab takes the mouse
	if (s_StripFrame < ImGui::GetFrameCount() - 1)
		return;
	for (const Doc* doc : s_Docs)
		if (!doc->closing && doc->shown && doc->rect.GetWidth() > 0.f)
		{
			out.push_back(ImVec4(doc->rect.Min.x, doc->rect.Min.y, doc->rect.Max.x, doc->rect.Max.y));
			if (ids)
				ids->push_back(doc->d.id);
		}
}

// ---- toolbar, status bar, work area -------------------------------------------------------------------------
void XUI::Shell::DrawToolbar()
{
	ImGuiViewport* viewport = ImGui::GetMainViewport();
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(8.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Base));
	const bool open = ImGui::BeginViewportSideBar("##toolbar", viewport, ImGuiDir_Up, Pt(36.f),
		ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse | ImGuiWindowFlags_NoSavedSettings);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar(2);
	if (open)
	{
		ImGuiWindow* window = ImGui::GetCurrentWindow();
		Doc* doc = ActiveDoc();
		{
			Toolbar tb(doc ? doc->d.id.c_str() : "none", ToolbarRow);
			if (doc && doc->loaded && doc->d.toolbar && !Missing(doc))
				doc->d.toolbar(tb);
			tb.Spring();
			tb.Group(100);
			if (s_Hooks.project_toolbar)
				s_Hooks.project_toolbar(tb);
		}
		window->DrawList->AddLine(ImVec2(window->Pos.x, window->Pos.y + window->Size.y - 1.f),
			ImVec2(window->Pos.x + window->Size.x, window->Pos.y + window->Size.y - 1.f), Col(Token::BorderSubtle));
	}
	ImGui::End();
}

void XUI::Shell::DrawStatusBar(bool busy)
{
	ImGuiViewport* viewport = ImGui::GetMainViewport();
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(4.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Chrome));
	const bool open = ImGui::BeginViewportSideBar("##statusbar", viewport, ImGuiDir_Down, Pt(24.f),
		ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse | ImGuiWindowFlags_NoSavedSettings);
	ImGui::PopStyleColor();
	// The bar's own padding is taken at Begin, and its row spaces its items itself: the hooks and every popover opened
	// from a chip (Rebuild, Tasks) get the theme's padding and spacing, like any other popover.
	ImGui::PopStyleVar(2);
	if (open)
	{
		ImGuiWindow* window = ImGui::GetCurrentWindow();
		s_StatusRect = window->Rect();
		const float gap = Pt(4.f);
		ImDrawList* dl = window->DrawList;
		dl->AddLine(window->Pos, ImVec2(window->Pos.x + window->Size.x, window->Pos.y), Col(Token::BorderSubtle));
		const float h = Pt(20.f);
		const float cy = window->Pos.y + window->Size.y * .5f;
		ImGui::SetCursorScreenPos(ImVec2(window->Pos.x + Pt(4.f), IM_ROUND(cy - h * .5f)));

		// ---- left: the drawers -----------------------------------------------------------------------
		xr_string last;
		int errors = 0, warnings = 0;
		if (s_Hooks.log_state)
			s_Hooks.log_state(last, errors, warnings);
		ImGui::BeginDisabled(busy);
		if (s_Drawers[DrawerContent].body &&
			ChromeButton("##st_content", Icon::Drawer, "Content Drawer", "Open the Content Browser over this tab", h, s_Drawer == DrawerContent))
			ToggleDrawer(DrawerContent);
		ImGui::SameLine(0.f, gap);
		if (s_Drawers[DrawerLog].body &&
			ChromeButton("##st_log", Icon::Log, "Output Log", "Open the Output Log over this tab", h, s_Drawer == DrawerLog))
			ToggleDrawer(DrawerLog);
		ImGui::EndDisabled();
		ImGui::SameLine(0.f, gap);
		const float left_end = ImGui::GetCursorScreenPos().x;

		// ---- right: measured last frame ----------------------------------------------------------------
		const float right_x = window->Pos.x + window->Size.x - Pt(8.f) - s_StatusRightW;
		// ---- centre: what the tab says, centred where the room allows; the last log line fills the gap before it
		const float room_x0 = left_end, room_x1 = ImMax(left_end, right_x - Pt(12.f));
		float status_x = room_x1;
		Doc* doc = ActiveDoc();
		const bool has_status = busy || (doc && doc->loaded && doc->d.status && !Missing(doc));
		if (has_status && room_x1 - room_x0 > Pt(24.f))
		{
			const float want = busy ? TextSize(Font::Small, "Working...").x : s_StatusCentreW;
			status_x = ImClamp(IM_ROUND(window->Pos.x + (window->Size.x - want) * .5f), room_x0 + Pt(160.f), ImMax(room_x0, room_x1 - want));
			status_x = ImMin(status_x, room_x1);
			const ImVec4 clip(status_x, window->Pos.y, room_x1, window->Pos.y + window->Size.y);
			if (busy)
				PaintText(dl, Font::Small, ImVec2(status_x, IM_ROUND(cy - FontPx(Font::Small) * .5f)), Col(Token::TextSecondary), "Working...", nullptr, 0.f, &clip);
			else
			{
				FontScope small_font(Font::Small);
				ImGui::SetCursorScreenPos(ImVec2(status_x, IM_ROUND(cy - FontPx(Font::Small) * .5f)));
				ImGui::PushClipRect(ImVec2(clip.x, clip.y), ImVec2(clip.z, clip.w), true);
				ImGui::BeginGroup();
				doc->d.status();
				ImGui::EndGroup();
				ImGui::PopClipRect();
				s_StatusCentreW = ImMax(0.f, ImGui::GetItemRectMax().x - status_x);
			}
		}
		// the log's last line beside its button, cut where the tab's status begins
		const float line_x1 = status_x - Pt(16.f);
		if (!busy && !last.empty() && line_x1 - room_x0 > Pt(40.f))
		{
			const ImVec4 clip(room_x0, window->Pos.y, line_x1, window->Pos.y + window->Size.y);
			xr_string shown;
			const char* line = last.c_str();
			if (Ellipsize(Font::Small, line, line_x1 - room_x0, Clip::End, shown))
				line = shown.c_str();
			PaintText(dl, Font::Small, ImVec2(IM_ROUND(room_x0), IM_ROUND(cy - FontPx(Font::Small) * .5f)), Col(Token::TextMuted), line, nullptr, 0.f, &clip);
		}
		if (s_Hooks.status_right)
		{
			ImGui::SetCursorScreenPos(ImVec2(ImMax(right_x, left_end), IM_ROUND(cy - h * .5f)));
			const float start = ImGui::GetCursorScreenPos().x;
			ImGui::BeginGroup();
			s_Hooks.status_right(busy);
			ImGui::EndGroup();
			s_StatusRightW = ImMax(0.f, ImGui::GetItemRectMax().x - start);
		}
	}
	ImGui::End();
}

ImVec4 XUI::Shell::WorkRect()
{
	return ImVec4(s_WorkRect.Min.x, s_WorkRect.Min.y, s_WorkRect.GetWidth(), s_WorkRect.GetHeight());
}

void XUI::Shell::DrawWorkArea()
{
	ImGuiViewportP* viewport = (ImGuiViewportP*)(void*)ImGui::GetMainViewport();
	s_WorkRect = viewport->GetBuildWorkRect();
	s_WorkAreaFrame = ImGui::GetFrameCount();
	Doc* doc = ActiveDoc();
	const bool padded = !doc || doc->d.padded || Missing(doc) || !doc->loaded;
	ImGui::SetNextWindowPos(s_WorkRect.Min);
	ImGui::SetNextWindowSize(s_WorkRect.GetSize());
	ImGui::SetNextWindowViewport(viewport->ID);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, padded ? PtV(8.f, 8.f) : ImVec2(0.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 0.f);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Base));
	ImGuiWindowFlags flags = ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoResize | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoCollapse |
		ImGuiWindowFlags_NoDocking | ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoBringToFrontOnFocus;
	if (!padded)
		flags |= ImGuiWindowFlags_NoScrollbar | ImGuiWindowFlags_NoScrollWithMouse;
	ImGui::Begin("###doc.host", nullptr, flags);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar(3);

	// the tabs that are not drawn keep what must stay alive (the Level dockspace)
	for (Doc* other : s_Docs)
		if (other != doc && !other->closing && other->loaded && other->d.inactive)
			other->d.inactive();
	if (doc)
	{
		if (Missing(doc))
			EmptyState(Icon::File, "File not found", doc->d.path.empty() ? Label(doc).c_str() : doc->d.path.c_str());
		else if (doc->loaded && doc->d.body)
			doc->d.body();
	}
	ImGui::End();
}

void XUI::Shell::DrawBusyFrame()
{
	if (s_Hooks.caption)
		s_Hooks.caption(true);
	DrawStatusBar(true);
	ImGuiViewportP* viewport = (ImGuiViewportP*)(void*)ImGui::GetMainViewport();
	s_WorkRect = viewport->GetBuildWorkRect();
	ImGui::SetNextWindowPos(s_WorkRect.Min);
	ImGui::SetNextWindowSize(s_WorkRect.GetSize());
	ImGui::SetNextWindowViewport(viewport->ID);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, ImVec2(0.f, 0.f));
	ImGui::PushStyleVar(ImGuiStyleVar_WindowRounding, 0.f);
	ImGui::PushStyleVar(ImGuiStyleVar_WindowBorderSize, 0.f);
	ImGui::PushStyleColor(ImGuiCol_WindowBg, Col(Token::Base));
	ImGui::Begin("###busy.host", nullptr, ImGuiWindowFlags_NoDecoration | ImGuiWindowFlags_NoMove | ImGuiWindowFlags_NoDocking |
		ImGuiWindowFlags_NoSavedSettings | ImGuiWindowFlags_NoBringToFrontOnFocus);
	ImGui::PopStyleColor();
	ImGui::PopStyleVar(3);
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	window->DrawList->AddRectFilled(window->Pos, window->Pos + window->Size, Col(Token::Dim, .6f));
	if (s_Hooks.busy_card)
	{
		const float w = ImMin(Pt(420.f), window->Size.x - Pt(32.f));
		ImGui::SetCursorScreenPos(ImVec2(IM_ROUND(window->Pos.x + (window->Size.x - w) * .5f), IM_ROUND(window->Pos.y + window->Size.y * .30f)));
		ImGui::PushStyleColor(ImGuiCol_ChildBg, Col(Token::Overlay));
		ImGui::PushStyleVar(ImGuiStyleVar_ChildRounding, Pt(8.f));
		ImGui::PushStyleVar(ImGuiStyleVar_WindowPadding, PtV(16.f, 12.f));
		ImGui::BeginChild("##busy_card", ImVec2(w, 0.f), ImGuiChildFlags_Borders | ImGuiChildFlags_AutoResizeY | ImGuiChildFlags_AlwaysUseWindowPadding);
		s_Hooks.busy_card();
		ImGui::EndChild();
		ImGui::PopStyleVar(2);
		ImGui::PopStyleColor();
	}
	ImGui::End();
}

// =======================================================================================================
// Drawers
// =======================================================================================================
void XUI::Shell::SetDrawer(int which, const DrawerDesc& desc)
{
	if (which > DrawerNone && which <= DrawerLog)
		s_Drawers[which] = desc;
}

void XUI::Shell::OpenDrawer(int which)
{
	if (which <= DrawerNone || which > DrawerLog || !s_Drawers[which].body)
		return;
	if (s_Drawer != which)
		s_DrawerFocus = true;
	s_Drawer = which;
}

void XUI::Shell::ToggleDrawer(int which)
{
	if (which <= DrawerNone || which > DrawerLog)
		return;
	if (s_Drawer == which)
	{
		CloseDrawer();
		return;
	}
	// its panel is on screen: that is where it goes instead
	const DrawerDesc& desc = s_Drawers[which];
	if (desc.panel_visible && desc.panel_visible())
	{
		CloseDrawer();
		if (desc.focus_panel)
			desc.focus_panel();
		return;
	}
	OpenDrawer(which);
}

void XUI::Shell::CloseDrawer()
{
	s_Drawer = DrawerNone;
	s_DrawerFocus = false;
}

int XUI::Shell::OpenedDrawer()
{
	return s_Drawer;
}

// =======================================================================================================
// Keys
// =======================================================================================================
bool XUI::Shell::ShellChord(unsigned vk, bool ctrl, bool shift, bool alt)
{
	if (alt)
		return false;
	if (!ctrl)
		return vk == VK_F11 && !shift;
	switch (vk)
	{
	case 'P':		return shift;
	case VK_TAB:	return true;
	case VK_F4:		return !shift;
	case VK_SPACE:	return !shift;
	case VK_OEM_3:	return !shift;
	case 'S':		return !shift;
	default:		return false;
	}
}

bool XUI::Shell::SceneKeysAllowed()
{
	if (s_Active != kLevel || !ImGui::GetCurrentContext())
		return false;
	if (ImGui::IsPopupOpen("", ImGuiPopupFlags_AnyPopupId | ImGuiPopupFlags_AnyPopupLevel))
		return false;
	ImGuiWindow* focused = GImGui->NavWindow;
	if (!focused)
		return true;
	ImGuiWindow* root = focused->RootWindowDockTree ? focused->RootWindowDockTree : focused->RootWindow;
	for (ImGuiWindow* w : { focused->RootWindow, root })
	{
		if (!w)
			continue;
		if (!strncmp(w->Name, "Content Drawer###", 17) || !strncmp(w->Name, "Output Log###drawer", 19))
			return false;
		for (const xr_string& owner : s_KeyOwners)
			if (owner == w->Name)
				return false;
	}
	// a docked panel's own window is its root below the dock host
	for (ImGuiWindow* w = focused; w; w = w->ParentWindow)
		for (const xr_string& owner : s_KeyOwners)
			if (owner == w->Name)
				return false;
	return true;
}

void XUI::Shell::RegisterKeyOwner(cstr window_name)
{
	if (window_name && *window_name && std::find(s_KeyOwners.begin(), s_KeyOwners.end(), xr_string(window_name)) == s_KeyOwners.end())
		s_KeyOwners.push_back(window_name);
}

// =======================================================================================================
// Workspaces
// =======================================================================================================
namespace
{
struct WorkspaceState
{
	xr_string signature;		// the layout as built; another one is the user's own
	xr_string last;				// last frame's
	bool pending = true;		// the signature is taken after the next full submit
	bool reset = false;
	bool mouse_was_down = false;
};
xr_map<xr_string, WorkspaceState> s_Workspaces;

// splits and the size of the side that does not hold the centre, rounded to 8 pt; a split with no centre on
// either side is a share, in steps of 5 %
void Signature(ImGuiDockNode* node, xr_string& out)
{
	if (!node)
		return;
	if (!node->IsSplitNode())
	{
		out += node->IsCentralNode() ? "C" : "L";
		return;
	}
	ImGuiDockNode* a = node->ChildNodes[0];
	ImGuiDockNode* b = node->ChildNodes[1];
	const int axis = node->SplitAxis == ImGuiAxis_X ? 0 : 1;
	const bool ca = a->IsCentralNode() || a->HasCentralNodeChild;
	const bool cb = b->IsCentralNode() || b->HasCentralNodeChild;
	const float sa = axis ? a->Size.y : a->Size.x;
	const float sb = axis ? b->Size.y : b->Size.x;
	int v;
	if (ca != cb)
		v = int(IM_ROUND((ca ? sb : sa) / XUI::Dpi() / 8.f));
	else
		v = int(IM_ROUND(100.f * sa / ImMax(1.f, sa + sb) / 5.f));
	char head[32];
	sprintf_s(head, "%c%d(", axis ? 'V' : 'H', v);
	out += head;
	Signature(a, out);
	out += ",";
	Signature(b, out);
	out += ")";
}
}

unsigned XUI::Shell::Dock::Split(unsigned& node, int dir, float size_pt, float ratio)
{
	ImGuiDockNode* n = ImGui::DockBuilderGetNode(ImGuiID(node));
	if (!n)
		return 0;
	const ImGuiDir d = dir == Left ? ImGuiDir_Left : dir == Right ? ImGuiDir_Right : dir == Up ? ImGuiDir_Up : ImGuiDir_Down;
	const bool horizontal = dir == Left || dir == Right;
	const float total = horizontal ? n->Size.x : n->Size.y;
	float r = ratio;
	if (size_pt > 0.f)
		r = total > 0.f ? ImClamp(Pt(size_pt) / total, .05f, .95f) : .25f;
	ImGuiID rest = 0;
	const ImGuiID side = ImGui::DockBuilderSplitNode(ImGuiID(node), d, r, nullptr, &rest);
	if (size_pt > 0.f)
		if (ImGuiDockNode* s = ImGui::DockBuilderGetNode(side))
		{
			ImVec2 size = s->Size;
			(horizontal ? size.x : size.y) = Pt(size_pt);
			ImGui::DockBuilderSetNodeSize(side, size);
		}
	node = unsigned(rest);
	return unsigned(side);
}

void XUI::Shell::Dock::Place(cstr window_name, unsigned node)
{
	if (window_name && node)
		ImGui::DockBuilderDockWindow(window_name, ImGuiID(node));
}

void XUI::Shell::WorkspaceDockSpace(const Workspace& ws, bool visible)
{
	if (!ws.name)
		return;
	const ImGuiID id = ImHashStr(ws.name);
	const ImGuiDockNodeFlags flags = ImGuiDockNodeFlags_NoWindowMenuButton;
	if (!visible)
	{
		ImGui::DockSpace(id, ImVec2(0.f, 0.f), flags | ImGuiDockNodeFlags_KeepAliveOnly);
		return;
	}
	WorkspaceState& st = s_Workspaces[ws.name];
	const xr_string class_key = xr_string("ws.") + ws.name + ".class";
	const float built_class = Layout::Get(class_key.c_str(), -1.f);
	ImGuiDockNode* node = ImGui::DockBuilderGetNode(id);
	const bool custom = ws.custom && *ws.custom;
	bool rebuild = !node || st.reset || (node->IsLeafNode() && !custom);
	if (!rebuild && built_class >= 0.f && int(built_class) != ws.size_class && !custom)
		rebuild = true;
	if (built_class < 0.f && node)
		Layout::Set(class_key.c_str(), float(ws.size_class));
	if (rebuild && ws.build)
	{
		st.reset = false;
		const ImVec2 avail = ImGui::GetContentRegionAvail();
		ImGui::DockBuilderRemoveNode(id);
		ImGui::DockBuilderAddNode(id, ImGuiDockNodeFlags_DockSpace);
		ImGui::DockBuilderSetNodePos(id, ImGui::GetCursorScreenPos());
		ImGui::DockBuilderSetNodeSize(id, ImVec2(ImMax(avail.x, Pt(400.f)), ImMax(avail.y, Pt(300.f))));
		ws.build(unsigned(id), ws.size_class);
		ImGui::DockBuilderFinish(id);
		Layout::Set(class_key.c_str(), float(ws.size_class));
		if (ws.custom)
			*ws.custom = false;
		st.pending = true;
		if (ws.built)
			ws.built();
	}
	// the panel tab strip: 26 pt tall (GImGuiDockTabBarExtra), its names 12 pt in from the tab's edge. The tab bars are
	// laid out and drawn inside DockSpace(), so the padding they take is the one pushed around it
	ImGui::PushStyleVar(ImGuiStyleVar_FramePadding, ImVec2(Pt(12.f), ImGui::GetStyle().FramePadding.y + GImGuiDockTabBarExtra * .5f));
	ImGui::DockSpace(id, ImVec2(0.f, 0.f), flags);
	ImGui::PopStyleVar();

	// the user's own arrangement is the one that differs from what was built
	node = ImGui::DockBuilderGetNode(id);
	if (!node)
		return;
	xr_string sig;
	Signature(node, sig);
	// only a change made with the mouse in the editor counts: a window resized from its border moves the
	// centre, and a window too small for the side nodes squeezes them, neither of which is the user's layout
	const bool mouse = ImGui::IsMouseDown(ImGuiMouseButton_Left) || st.mouse_was_down;
	st.mouse_was_down = ImGui::IsMouseDown(ImGuiMouseButton_Left);
	if (st.pending)
	{
		st.signature = sig;
		st.pending = false;
	}
	else if (ws.custom && !*ws.custom && sig != st.last && mouse && sig != st.signature)
		*ws.custom = true;
	st.last = sig;
}

void XUI::Shell::ResetWorkspace(cstr name)
{
	if (name)
		s_Workspaces[name].reset = true;
}

void XUI::Shell::NextWindowIsCanvas()
{
	static ImGuiWindowClass s_Canvas;
	s_Canvas.DockNodeFlagsOverrideSet = ImGuiDockNodeFlags_NoTabBar | ImGuiDockNodeFlags_NoDockingOverMe | ImGuiDockNodeFlags_NoWindowMenuButton;
	ImGui::SetNextWindowClass(&s_Canvas);
}

void XUI::Shell::ClampNextFloatingWindow(cstr window_name)
{
	ImGuiWindow* window = window_name ? ImGui::FindWindowByName(window_name) : nullptr;
	if (!window || window->DockIsActive || window->DockId || s_WorkRect.GetWidth() <= 0.f)
		return;
	const ImVec2 size = ImMin(window->Size, s_WorkRect.GetSize());
	const ImVec2 pos(ImClamp(window->Pos.x, s_WorkRect.Min.x, s_WorkRect.Max.x - size.x),
		ImClamp(window->Pos.y, s_WorkRect.Min.y, s_WorkRect.Max.y - size.y));
	if (pos.x != window->Pos.x || pos.y != window->Pos.y)
		ImGui::SetNextWindowPos(pos);
	if (size.x != window->Size.x || size.y != window->Size.y)
		ImGui::SetNextWindowSize(size);
}

bool XUI::Shell::WindowShown(cstr window_name)
{
	ImGuiWindow* window = window_name ? ImGui::FindWindowByName(window_name) : nullptr;
	if (!window || !window->WasActive || window->Hidden)
		return false;
	// a docked window behind another tab of its node is not on screen
	return !window->DockIsActive || !window->DockNode || window->DockNode->VisibleWindow == window;
}

void XUI::Shell::FocusWindow(cstr window_name)
{
	ImGuiWindow* window = window_name && ImGui::GetCurrentContext() ? ImGui::FindWindowByName(window_name) : nullptr;
	if (!window)
		return;
	// a docked window's tab is selected in its node as well: a tab bar built anew - its tab coming back after Level
	// was hidden - takes the node's selection, not the focus
	if (window->DockNode)
	{
		window->DockNode->SelectedTabId = window->TabId;
		if (window->DockNode->TabBar)
			window->DockNode->TabBar->NextSelectedTabId = window->TabId;
	}
	ImGui::FocusWindow(window);
}

bool XUI::Shell::TakePopup(cstr name)
{
	return TakePopupRequest(name);
}

bool XUI::Shell::PopupRequested(cstr prefix)
{
	return PeekPopupRequest(prefix);
}

bool XUI::Shell::BeginCaptionMenu(cstr label, bool enabled)
{
	if (!label || !*label)
		return false;
	const char* hash = strstr(label, "##");
	char request[80];
	sprintf_s(request, "menu:%.*s", int(hash ? hash - label : strlen(label)), label);
	// BeginMenu's popup has the label's id in this window: opened here, it comes up the way a click brings it
	if (enabled && TakePopupRequest(request))
		ImGui::OpenPopup(label);
	return ImGui::BeginMenu(label, enabled);
}

void XUI::Shell::OpenPopupAt(cstr str_id, const ImVec2& pos)
{
	if (str_id)
		XUI::Internal::OpenPopupAt(ImGui::GetID(str_id), pos);
}

void XUI::Shell::NextPopupAbove(const ImVec2& item_min, const ImVec2& item_max)
{
	const ImGuiViewport* vp = ImGui::GetMainViewport();
	// an item in the right half grows its popup to the left, so it stays inside the window
	const bool right = (item_min.x + item_max.x) * .5f > vp->WorkPos.x + vp->WorkSize.x * .5f;
	ImGui::SetNextWindowPos(ImVec2(right ? item_max.x : item_min.x, item_min.y - Pt(4.f)), ImGuiCond_Always, ImVec2(right ? 1.f : 0.f, 1.f));
}

// =======================================================================================================
// MCP's view of the frame
// =======================================================================================================
namespace
{
ImVec4 ViewportRect(const ImGuiWindow* w)
{
	const ImVec2 origin = ImGui::GetMainViewport()->Pos;
	return ImVec4(w->Pos.x - origin.x, w->Pos.y - origin.y, w->Size.x, w->Size.y);
}

// the window itself when it scrolls, otherwise the child of it that scrolls the most (a panel whose list is a child)
ImGuiWindow* ScrollingWindow(ImGuiWindow* window)
{
	// a pixel of rounding past the bottom is no scroll: the list under it is what moves
	if (window->ScrollMax.y > 1.f)
		return window;
	ImGuiWindow* best = nullptr;
	for (ImGuiWindow* w : GImGui->Windows)
	{
		if (w == window || !w->WasActive || !(w->Flags & ImGuiWindowFlags_ChildWindow) || w->ScrollMax.y <= 1.f)
			continue;
		bool inside = false;
		for (ImGuiWindow* p = w->ParentWindow; p && !inside; p = p->ParentWindow)
			inside = p == window;
		if (inside && (!best || w->ScrollMax.y > best->ScrollMax.y))
			best = w;
	}
	return best ? best : window;
}

}

bool XUI::Shell::GetWindowState(cstr window_name, WindowState& out)
{
	out = WindowState();
	ImGuiWindow* window = window_name && ImGui::GetCurrentContext() ? ImGui::FindWindowByName(window_name) : nullptr;
	if (!window)
		return false;
	out.exists = true;
	out.shown = WindowShown(window_name);
	out.docked = window->DockIsActive;
	out.focused = GImGui->NavWindow && (GImGui->NavWindow == window || GImGui->NavWindow->RootWindow == window);
	out.rect = ViewportRect(window);
	const ImGuiWindow* scroll = ScrollingWindow(window);
	out.scroll_y = scroll->Scroll.y;
	out.scroll_max_y = scroll->ScrollMax.y;
	return true;
}

bool XUI::Shell::ScrollWindow(cstr window_name, cstr target)
{
	ImGuiWindow* window = window_name && ImGui::GetCurrentContext() ? ImGui::FindWindowByName(window_name) : nullptr;
	if (!window || !target || !*target)
		return false;
	ImGuiWindow* scroll = ScrollingWindow(window);
	float y = 0.f;
	// ImGui ignores a target of FLT_MAX: the bottom is where the last frame ended, the next Begin clamps to its own
	if (!strcmp(target, "top"))
		y = 0.f;
	else if (!strcmp(target, "bottom"))
		y = scroll->ScrollMax.y;
	else if (sscanf_s(target, "%f", &y) == 1)
		y = Pt(y);
	else
		return false;
	// taken by the window's next Begin, clamped there to what it can scroll
	ImGui::SetScrollY(scroll, y);
	return true;
}

void XUI::Shell::OpenPopups(xr_vector<PopupState>& out)
{
	out.clear();
	if (!ImGui::GetCurrentContext())
		return;
	for (const ImGuiPopupData& popup : GImGui->OpenPopupStack)
	{
		const ImGuiWindow* w = popup.Window;
		if (!w)
			continue;
		PopupState& p = out.emplace_back();
		p.name = w->Name;
		p.title = Str(w->Name, ImGui::FindRenderedTextEnd(w->Name));
		const char* hash = strstr(w->Name, "###");
		p.id = hash ? xr_string(hash + 3) : p.title;
		p.modal = (w->Flags & ImGuiWindowFlags_Modal) != 0;
		p.rect = ViewportRect(w);
	}
}

void XUI::Shell::ClosePopups()
{
	s_ClosePopups = true;
	ShowPopup(nullptr);
}

// =======================================================================================================
// Settings
// =======================================================================================================
void XUI::Shell::SetFloatingPanels(bool allow)
{
	s_FloatingPanels = allow;
}

bool XUI::Shell::FloatingPanels()
{
	return s_FloatingPanels;
}

void XUI::Shell::SetSkipLayoutOnce(bool skip)
{
	s_SkipLayout = skip;
}

bool XUI::Shell::SkipLayoutOnce()
{
	return s_SkipLayout;
}
