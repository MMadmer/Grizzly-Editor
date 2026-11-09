#include "stdafx.h"
#include "NqCanvas.h"
#include "../../../XrECore/Editor/Nq/NqLua.h"
#include "../../../XrECore/Editor/Nq/NqUtil.h"

namespace
{
	using T = GrizzlyTheme::ColorToken;
	IC ImU32 C(T token, float alpha = 1.f) { return GrizzlyTheme::U32(token, alpha); }

	// ten fixed zoom levels (docs par. 13.8); index 6 = 1:1, index 5 the floor of Frame All
	const float kZoom[] = { 0.2f, 0.3f, 0.4f, 0.5f, 0.65f, 0.75f, 1.0f, 1.25f, 1.5f, 2.0f };
	const int	kZoomCount = sizeof(kZoom) / sizeof(kZoom[0]);
	const int	kZoomDefault = 6;
	const int	kZoomFrameMin = 5;

	// type on the canvas, in world pt: titles, lines, ids and pin names
	const float kTitlePt	= 16.f;
	const float kLinePt		= 15.f;
	const float kSmallPt	= 14.f;
	// the level of detail follows the size a title is really drawn at
	const float kLodFull	= 10.f;
	const float kLodTitles	= 7.f;

	const float kPinR		= 4.5f;		// world pt
	const float kLinkTangent = 48.f;
	// an edge that goes back up, world pt
	const float kBackStub	= 24.f;		// the straight run out of the pin and into the input
	const float kBackClear	= 24.f;		// how far its channel keeps from a node
	const float kBackLane	= 12.f;		// between two edges that share a channel
	const float kBackCorner	= 12.f;		// the radius of its corners
	const float kInset		= 8.f;		// world pt from the node's edge to its text

	// Grab zones, in pt. The dot a pin is drawn as follows the zoom, so the zone cannot be
	// the dot: it is the dot plus a fixed slack, floored so that the far zoom levels stay
	// usable and capped so that a pin never swallows its own node. Towards the body the
	// zone is tightened back to roughly the drawn size - the bottom band of a box has to
	// keep starting a node drag, not a link.
	const float kPinGrabPad		= 8.f;
	const float kPinGrabMin		= 12.f;
	const float kPinGrabMax		= 22.f;
	const float kPinGrabInside	= 0.7f;		// share of the radius that reaches inwards
	// around a link curve. Matched to kPinGrabMin: a wire is thinner than a pin, so
	// aiming at one pixel-perfectly is worse, not better.
	const float kLinkGrab		= 12.f;

	const float kPickerRows	= 12.f;		// rows a search popup shows before it scrolls
	const float kPickerMinEm = 16.f;
	const float kPickerMaxEm = 40.f;
	const int	kHistoryLimit	= 64;
	const float kMinimapW	= 180.f;	// pt
	const float kMinimapH	= 120.f;
	// world pt Frame All keeps around what it shows
	const float kFrameMargin = 32.f;
	// the edges of the view, as Beyond() numbers them
	enum { kAbove, kBelow, kLeft, kRight };

	IC ImVec2 Add(const ImVec2& a, const ImVec2& b) { return ImVec2(a.x + b.x, a.y + b.y); }
	IC ImVec2 Sub(const ImVec2& a, const ImVec2& b) { return ImVec2(a.x - b.x, a.y - b.y); }
	IC bool InRect(const ImVec2& p, const ImVec2& a, const ImVec2& b) { return p.x >= a.x && p.y >= a.y && p.x <= b.x && p.y <= b.y; }
	IC float Snap(float v) { return floorf(v / NqLayout::kGrid + 0.5f) * NqLayout::kGrid; }
	IC float Dist2(const ImVec2& a, const ImVec2& b) { return (a.x - b.x) * (a.x - b.x) + (a.y - b.y) * (a.y - b.y); }
	IC float Round(float v) { return floorf(v + 0.5f); }

	ImFont* Face(XUI::Font font)
	{
		ImFont* face = XUI::GetFont(font);
		return face ? face : ImGui::GetFont();
	}

	float TextW(ImFont* face, float px, LPCSTR text)
	{
		return text && *text ? face->CalcTextSizeA(px, FLT_MAX, 0.f, text).x : 0.f;
	}

	// one line of canvas text, cut to `avail` px; returns the width drawn
	float PaintFit(ImDrawList* dl, ImFont* face, float px, const ImVec2& at, ImU32 col, LPCSTR text, float avail)
	{
		if (!text || !*text || avail <= px * 0.5f) return 0.f;
		xr_string shown;
		LPCSTR drawn = XUI::EllipsizeFace(face, px, text, avail, XUI::Clip::End, shown) ? shown.c_str() : text;
		dl->AddText(face, px, ImVec2(Round(at.x), Round(at.y)), col, drawn);
		return face->CalcTextSizeA(px, FLT_MAX, 0.f, drawn).x;
	}

	// `out` is pin -> targets, so a pin is free exactly when it carries none
	bool PinFree(const SNqNode& n, LPCSTR pin)
	{
		const xr_vector<xr_string>* t = n.Targets(pin);
		return !t || t->empty();
	}

	// a node can take one more incoming edge when nothing arrives yet; flow.join is
	// the exception - merging several paths is the whole point of it
	bool CanTakeInput(const SNqQuest& q, const SNqNode& n)
	{
		if (NqText::IsTrigger(n.kind.c_str())) return false;
		return n.kind == "flow.join" || 0 == q.InDegree(n.id.c_str());
	}

	// squared distance from p to segment ab
	float SegDist2(const ImVec2& p, const ImVec2& a, const ImVec2& b)
	{
		ImVec2 ab = Sub(b, a), ap = Sub(p, a);
		float l2 = ab.x * ab.x + ab.y * ab.y;
		float t = l2 > 0.f ? (ap.x * ab.x + ap.y * ab.y) / l2 : 0.f;
		t = t < 0.f ? 0.f : (t > 1.f ? 1.f : t);
		ImVec2 c(a.x + ab.x * t, a.y + ab.y * t);
		return (p.x - c.x) * (p.x - c.x) + (p.y - c.y) * (p.y - c.y);
	}

	ImVec2 Bezier(const ImVec2& p0, const ImVec2& p1, const ImVec2& p2, const ImVec2& p3, float t)
	{
		float u = 1.f - t;
		float a = u * u * u, b = 3 * u * u * t, c = 3 * u * t * t, d = t * t * t;
		return ImVec2(a * p0.x + b * p1.x + c * p2.x + d * p3.x, a * p0.y + b * p1.y + c * p2.y + d * p3.y);
	}

	// The centre of a view `len` long on one axis of the graph [a0, a1]: the middle when it all fits. Else
	// `prefer`, moved as little as it takes to keep the part [k0, k1] in view (`fallback` when that part does
	// not fit either), and never past an end of the graph: the view is filled with graph, not empty canvas.
	float Along(float a0, float a1, float k0, float k1, float len, float prefer, float fallback)
	{
		if (a1 - a0 <= len) return (a0 + a1) * 0.5f;
		const float half = len * 0.5f;
		if (k1 - k0 > len) return _max(a0 + half, _min(fallback, a1 - half));
		const float c = _max(a0 + half, _min(prefer, a1 - half));
		return _max(k1 - half, _min(c, k0 + half));
	}

	struct SSpan { float a, b; };

	// The x of a free vertical channel: outside every span, and as short a detour for a run
	// from x0 to x3 as there is - between the two every x costs the same, so the middle of
	// what is free there. A tie goes to x0's side.
	float PickChannel(xr_vector<SSpan>& spans, float x0, float x3)
	{
		std::sort(spans.begin(), spans.end(), [](const SSpan& l, const SSpan& r) { return l.a < r.a; });
		const float lo = _min(x0, x3), hi = _max(x0, x3);
		float best = x0, best_cost = FLT_MAX;
		auto consider = [&](float from, float to)
		{
			if (from > to) return;
			const float x = to < lo ? to : from > hi ? from : (_max(from, lo) + _min(to, hi)) * 0.5f;
			const float cost = fabsf(x - x0) * 1.001f + fabsf(x - x3);
			if (cost < best_cost) { best_cost = cost; best = x; }
		};
		// the free space runs from the left end of the world to the first span, between spans, and on
		float free_from = -FLT_MAX;
		for (const SSpan& s : spans)
		{
			consider(free_from, s.a);
			free_from = _max(free_from, s.b);
		}
		consider(free_from, FLT_MAX);
		return best;
	}

	// A search popup is auto-sized by its filter box, and its list sits in a fixed
	// child that cannot push the window open - so the width has to be asked for
	// here, measured on the widest human title the popup can show.
	void ConstrainPicker(LPCSTR popup, u32 use_mask)
	{
		if (!ImGui::IsPopupOpen(popup)) return;
		xr_vector<const NqCatalog::SKind*> kinds;
		NqCatalog::KindsFor(use_mask, kinds);
		const bool behaviour = NqCatalog::DocBehaviour();
		float w = 0.f;
		for (u32 i = 0; i < kinds.size(); ++i)
			w = _max(w, ImGui::CalcTextSize(NqLook::KindTitle(kinds[i]->id.c_str(), behaviour).c_str()).x + ImGui::CalcTextSize(kinds[i]->id.c_str()).x);
		const ImGuiStyle& st = ImGui::GetStyle();
		w += st.WindowPadding.x * 2.f + st.ItemSpacing.x * 3.f + st.ScrollbarSize + XUI::Pt(22.f);
		w = _min(_max(w, XUI::Em(kPickerMinEm)), XUI::Em(kPickerMaxEm));
		ImGui::SetNextWindowSizeConstraints(ImVec2(w, 0.f), ImVec2(w, FLT_MAX));
	}

	// "quest" in a behaviour graph's words: a whole word, capitals kept
	xr_string Regraph(const xr_string& text)
	{
		xr_string out;
		const char* p = text.c_str();
		auto word_char = [](char c) { return (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') || (c >= '0' && c <= '9') || c == '_' || (u8)c >= 0x80; };
		while (*p)
		{
			const bool start = p == text.c_str() || !word_char(p[-1]);
			if (start && (0 == strncmp(p, "quest", 5) || 0 == strncmp(p, "Quest", 5)))
			{
				const char* after = p + 5;
				const bool plural = *after == 's';
				if (plural) ++after;
				if (!word_char(*after))
				{
					out += (*p == 'Q') ? "Graph" : "graph";
					if (plural) out += "s";
					p = after;
					continue;
				}
			}
			out += *p++;
		}
		return out;
	}
}

//------------------------------------------------------------------------------
// what a kind looks like
//------------------------------------------------------------------------------
XUI::Icon NqLook::KindIcon(LPCSTR kind)
{
	using I = XUI::Icon;
	if (!kind || !kind[0]) return I::Dot;
	if (0 == strcmp(kind, "flow.end"))		return I::End;
	if (0 == strcmp(kind, "behaviour.run"))	return I::Behaviour;
	if (0 == strcmp(kind, "lua"))			return I::FileScript;
	if (NqText::IsTrigger(kind))			return I::Trigger;
	struct SMap { LPCSTR group; I icon; };
	static const SMap kGroups[] =
	{
		{ "triggers", I::Trigger }, { "flow", I::Flow }, { "wait", I::Wait }, { "dialog", I::Dialog }, { "dialog_ctl", I::Dialog },
		{ "objective", I::Task }, { "tasks", I::Task }, { "quest", I::Quest }, { "vars", I::Variable }, { "items", I::Item },
		{ "money", I::Money }, { "spawn", I::SpawnNpc }, { "npc", I::Role }, { "behaviour", I::Behaviour }, { "orders", I::Order },
		{ "relation", I::Relation }, { "squad", I::Squad }, { "world", I::World }, { "actor", I::Person }, { "ui", I::PdaMsg },
		{ "info", I::PdaMsg }, { "sound", I::FileSound }, { "logic", I::FileScript }, { "custom", I::FileScript },
		{ "conditions", I::Condition }, { "cond_event", I::Condition }, { "cond_state", I::Condition }, { "cond_world", I::Condition },
	};
	if (const NqCatalog::SKind* k = NqCatalog::Find(kind))
		for (const SMap& m : kGroups)
			if (k->group == m.group) return m.icon;
	// a module's own group: the kind's name still says what it is
	static const SMap kPrefixes[] =
	{
		{ "flow.", I::Flow }, { "wait.", I::Wait }, { "dialog.", I::Dialog }, { "objective.", I::Task }, { "task.", I::Task },
		{ "var.", I::Variable }, { "item.", I::Item }, { "money.", I::Money }, { "spawn.", I::SpawnNpc }, { "npc.", I::Role },
		{ "order.", I::Order }, { "relation.", I::Relation }, { "squad.", I::Squad }, { "pda.", I::PdaMsg }, { "actor.", I::Person },
	};
	for (const SMap& m : kPrefixes)
		if (0 == strncmp(kind, m.group, xr_strlen(m.group))) return m.icon;
	return I::Dot;
}

ImU32 NqLook::FamilyColor(LPCSTR kind, float alpha)
{
	using I = XUI::Icon;
	T token = T::TextMuted;
	switch (KindIcon(kind))
	{
	case I::Trigger:	token = T::Success; break;
	case I::End:		token = T::Error; break;
	case I::Flow:		token = T::TextSecondary; break;
	case I::Wait:		token = T::Warning; break;
	case I::Dialog:		token = T::AxisZ; break;
	case I::Task: case I::Quest: case I::PdaMsg:	token = T::Required; break;
	case I::Role: case I::Behaviour: case I::Order: case I::Squad: case I::Relation: case I::Person:	token = T::AccentBright; break;
	case I::Item: case I::Money: case I::SpawnNpc: case I::World:	token = T::Accent; break;
	default:			break;
	}
	return C(token, alpha);
}

xr_string NqLook::KindTitle(LPCSTR kind, bool behaviour)
{
	const NqCatalog::SKind* k = kind ? NqCatalog::Find(kind) : 0;
	xr_string title = k ? k->title : xr_string(kind ? kind : "");
	if (!behaviour) return title;
	// "Quest start" / "Quest end" are the graph's own start and end
	if (0 == strncmp(title.c_str(), "Quest ", 6) && title.size() > 6)
	{
		title = title.substr(6);
		if (title[0] >= 'a' && title[0] <= 'z') title[0] = char(title[0] - 0x20);
		return title;
	}
	return Regraph(title);
}

xr_string NqLook::KindDesc(LPCSTR kind, bool behaviour)
{
	const NqCatalog::SKind* k = kind ? NqCatalog::Find(kind) : 0;
	if (!k) return "";
	return behaviour ? Regraph(k->desc) : k->desc;
}

LPCSTR NqLook::DocNoun(bool behaviour)
{
	return behaviour ? "graph" : "quest";
}

//------------------------------------------------------------------------------
// canvas
//------------------------------------------------------------------------------
NqCanvas::NqCanvas(NqDoc* doc) : m_Doc(doc)
{
	m_Origin = m_Size = ImVec2(0, 0);
	m_DrawnFrame = -1;
	m_Hovered = false;
	m_ShowMinimap = true;
	m_MinimapShown = false;
	m_MinimapHovered = m_MinimapPanning = false;
	m_MinimapMin = m_MinimapMax = ImVec2(0, 0);
	m_MinimapWorldMin = m_MinimapWorldMax = ImVec2(0, 0);
	m_MinimapScale = 1.f;
	for (int s = 0; s < 4; ++s) { m_Beyond[s] = 0; m_BeyondNear[s] = -1; }
	m_BeyondHot = -1;
	m_Panning = m_RmbMoved = false;
	m_MiddlePanning = false;
	m_Dragging = m_DragMoved = false;
	m_Marquee = false;
	m_Linking = m_PendingLink = false;
	m_StatusAt = 0.0;
	m_HotNode = m_HotPin = m_HotLink = -1;
	m_ActDragIndex = -1;
	m_ActDragging = false;
	m_OpenAddAction = m_AddActionAt = false;
	m_AddActionPos = ImVec2(0, 0);
	m_WantFrameAll = m_WantFrameSel = m_WantRename = m_WantFocusAction = false;
	m_WantZoom = -1;
	m_ReplayingHistory = false;
	m_HistoryReady = false;
	m_HistoryBatchDepth = 0;
	m_HistoryBatchRemembered = false;
	m_LastWheelHistory = -DBL_MAX;
	m_WheelZooming = false;
	m_TipSince = 0.0;
	m_ReachRevision = u32(-1);
	m_GeomRevision = m_GeomCatalogGeneration = u32(-1);
	m_LinkRevision = m_LinkCatalogGeneration = u32(-1);
	m_LinkOrigin = m_LinkSize = ImVec2(FLT_MAX, FLT_MAX);
	m_LinkViewX = m_LinkViewY = FLT_MAX;
	m_LinkScale = -1.f;
	m_Filter[0] = 0;
	m_FilterSel = 0;
	if (m_Doc->zoom_idx < 0 || m_Doc->zoom_idx >= kZoomCount) m_Doc->zoom_idx = kZoomDefault;
	// a freshly opened document frames itself once
	m_WantFrameAll = (m_Doc->view_cx == 0.f && m_Doc->view_cy == 0.f);
}

int   NqCanvas::ZoomLevels()			{ return kZoomCount; }
float NqCanvas::ZoomOf(int idx)			{ if (idx < 0) idx = 0; if (idx >= kZoomCount) idx = kZoomCount - 1; return kZoom[idx]; }
int   NqCanvas::FrameMinLevel()			{ return kZoomFrameMin; }

float NqCanvas::Scale() const
{
	return Zoom() * XUI::Dpi();
}

float NqCanvas::TitlePx() const
{
	return Round(kTitlePt * Scale());
}

int NqCanvas::Lod() const
{
	const float px = TitlePx();
	return px >= kLodFull ? 2 : px >= kLodTitles ? 1 : 0;
}

NqCanvas::SViewState NqCanvas::CurrentView() const
{
	SViewState state;
	state.cx = m_Doc->view_cx;
	state.cy = m_Doc->view_cy;
	state.zoom = m_Doc->zoom_idx;
	state.selection = m_Doc->selection;
	state.slot = m_Doc->sel_slot;
	return state;
}

void NqCanvas::RememberView()
{
	if (!m_WheelZooming) m_LastWheelHistory = -DBL_MAX;
	if (!m_HistoryReady || m_ReplayingHistory || (m_HistoryBatchDepth && m_HistoryBatchRemembered)) return;
	const SViewState state = CurrentView();
	if (!m_BackHistory.empty())
	{
		const SViewState& last = m_BackHistory.back();
		if (last.cx == state.cx && last.cy == state.cy && last.zoom == state.zoom
			&& last.selection == state.selection && last.slot == state.slot)
		{
			if (m_HistoryBatchDepth) m_HistoryBatchRemembered = true;
			return;
		}
	}
	m_BackHistory.push_back(state);
	if ((int)m_BackHistory.size() > kHistoryLimit) m_BackHistory.erase(m_BackHistory.begin());
	m_ForwardHistory.clear();
	if (m_HistoryBatchDepth) m_HistoryBatchRemembered = true;
}

void NqCanvas::ApplyView(const SViewState& state)
{
	m_ReplayingHistory = true;
	m_LastWheelHistory = -DBL_MAX;
	CancelFraming();
	m_Doc->view_cx = NqLayout::Sane(state.cx);
	m_Doc->view_cy = NqLayout::Sane(state.cy);
	m_Doc->zoom_idx = _max(0, _min(state.zoom, kZoomCount - 1));
	m_Doc->selection.clear();
	for (u32 i = 0; i < state.selection.size(); ++i)
		if (m_Doc->quest.FindNode(state.selection[i].c_str())) m_Doc->selection.push_back(state.selection[i]);
	m_Doc->sel_slot = m_Doc->selection.size() == 1 ? state.slot : "";
	m_ReplayingHistory = false;
}

void NqCanvas::BeginNavigationBatch()
{
	if (!m_HistoryBatchDepth) m_HistoryBatchRemembered = false;
	++m_HistoryBatchDepth;
}

void NqCanvas::EndNavigationBatch()
{
	if (m_HistoryBatchDepth > 0) --m_HistoryBatchDepth;
	if (!m_HistoryBatchDepth) m_HistoryBatchRemembered = false;
}

bool NqCanvas::HistoryBack()
{
	if (m_BackHistory.empty()) return false;
	const SViewState current = CurrentView();
	const SViewState target = m_BackHistory.back();
	m_BackHistory.pop_back();
	m_ForwardHistory.push_back(current);
	if ((int)m_ForwardHistory.size() > kHistoryLimit) m_ForwardHistory.erase(m_ForwardHistory.begin());
	ApplyView(target);
	return true;
}

bool NqCanvas::HistoryForward()
{
	if (m_ForwardHistory.empty()) return false;
	const SViewState current = CurrentView();
	const SViewState target = m_ForwardHistory.back();
	m_ForwardHistory.pop_back();
	m_BackHistory.push_back(current);
	if ((int)m_BackHistory.size() > kHistoryLimit) m_BackHistory.erase(m_BackHistory.begin());
	ApplyView(target);
	return true;
}

void NqCanvas::ClearHistory()
{
	m_BackHistory.clear();
	m_ForwardHistory.clear();
	m_LastWheelHistory = -DBL_MAX;
}

bool NqCanvas::IsBookmarked(LPCSTR id) const
{
	return id && m_BookmarkLookup.find(id) != m_BookmarkLookup.end();
}

bool NqCanvas::SetBookmark(LPCSTR id, bool enabled)
{
	if (!id || !id[0]) return false;
	auto found = m_BookmarkLookup.find(id);
	if (found != m_BookmarkLookup.end())
	{
		if (enabled) return true;
		m_BookmarkLookup.erase(found);
		for (u32 i = 0; i < m_Bookmarks.size(); ++i)
		{
			if (m_Bookmarks[i] != id) continue;
			m_Bookmarks.erase(m_Bookmarks.begin() + i);
			break;
		}
		return true;
	}
	if (!m_Doc->quest.FindNode(id)) return false;
	if (enabled)
	{
		m_Bookmarks.push_back(id);
		m_BookmarkLookup.emplace(id, 1);
	}
	return true;
}

bool NqCanvas::ToggleBookmark(LPCSTR id)
{
	return SetBookmark(id, !IsBookmarked(id));
}

void NqCanvas::ClearBookmarks()
{
	m_Bookmarks.clear();
	m_BookmarkLookup.clear();
}

void NqCanvas::PruneTransient()
{
	for (u32 i = 0; i < m_Bookmarks.size(); )
	{
		if (m_Doc->quest.FindNode(m_Bookmarks[i].c_str())) ++i;
		else
		{
			m_BookmarkLookup.erase(m_Bookmarks[i]);
			m_Bookmarks.erase(m_Bookmarks.begin() + i);
		}
	}
}

void NqCanvas::ToggleSelectedBookmarks()
{
	if (m_Doc->selection.empty()) { SetStatus("Select nodes to bookmark them"); return; }
	bool all = true;
	for (u32 i = 0; i < m_Doc->selection.size(); ++i) all &= IsBookmarked(m_Doc->selection[i].c_str());
	for (u32 i = 0; i < m_Doc->selection.size(); ++i) SetBookmark(m_Doc->selection[i].c_str(), !all);
	SetStatus(all ? "Bookmarks removed" : "Selection bookmarked");
}

bool NqCanvas::JumpBookmark(int delta)
{
	PruneTransient();
	if (m_Bookmarks.empty()) return false;
	int current = -1;
	if (m_Doc->selection.size() == 1)
		for (u32 i = 0; i < m_Bookmarks.size(); ++i)
			if (m_Bookmarks[i] == m_Doc->selection[0]) { current = (int)i; break; }
	int next = delta < 0 ? (current < 0 ? (int)m_Bookmarks.size() - 1 : current - 1)
					 : (current + 1);
	if (next < 0) next = (int)m_Bookmarks.size() - 1;
	if (next >= (int)m_Bookmarks.size()) next = 0;
	RequestFrameNode(m_Bookmarks[next].c_str());
	return true;
}

bool NqCanvas::GraphBounds(float& min_x, float& min_y, float& max_x, float& max_y) const
{
	if (m_Doc->quest.nodes.empty()) return false;
	min_x = min_y = FLT_MAX;
	max_x = max_y = -FLT_MAX;
	for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
	{
		const SNqNode& node = m_Doc->quest.nodes[i];
		const Fvector2 size = NqLayout::NodeSize(node);
		min_x = _min(min_x, node.pos.x);
		min_y = _min(min_y, node.pos.y);
		max_x = _max(max_x, node.pos.x + size.x);
		max_y = _max(max_y, node.pos.y + size.y);
	}
	return true;
}

bool NqCanvas::ViewportBounds(float& min_x, float& min_y, float& max_x, float& max_y) const
{
	if (!Measured()) return false;
	const float s = Scale();
	min_x = m_Doc->view_cx - m_Size.x / (2.f * s);
	min_y = m_Doc->view_cy - m_Size.y / (2.f * s);
	max_x = m_Doc->view_cx + m_Size.x / (2.f * s);
	max_y = m_Doc->view_cy + m_Size.y / (2.f * s);
	return true;
}

// an asked-for zoom outranks the zoom a pending frame request would pick, so
// quest_view with both `frame` and `zoom_level` shows the level that was asked
// for (and the answer it already reported)
void NqCanvas::SetZoom(int idx)
{
	if (idx < 0) idx = 0;
	if (idx >= kZoomCount) idx = kZoomCount - 1;
	if (idx != m_Doc->zoom_idx) RememberView();
	m_Doc->zoom_idx = idx;
	m_WantZoom = idx;
}

void NqCanvas::RequestFrameAll()
{
	if (MeasuredNow()) FrameAll(); else m_WantFrameAll = true;
}

void NqCanvas::RequestFrameSel()
{
	if (MeasuredNow()) FrameSelection(); else m_WantFrameSel = true;
}

void NqCanvas::RequestFrameNode(LPCSTR id)
{
	FrameNode(id);
}

ImVec2 NqCanvas::ToScreen(const ImVec2& w) const
{
	const float s = Scale();
	return ImVec2(m_Origin.x + m_Size.x * 0.5f + (w.x - m_Doc->view_cx) * s,
				  m_Origin.y + m_Size.y * 0.5f + (w.y - m_Doc->view_cy) * s);
}

ImVec2 NqCanvas::ToWorld(const ImVec2& p) const
{
	const float s = Scale();
	return ImVec2(m_Doc->view_cx + (p.x - m_Origin.x - m_Size.x * 0.5f) / s,
				  m_Doc->view_cy + (p.y - m_Origin.y - m_Size.y * 0.5f) / s);
}

void NqCanvas::ZoomBy(int delta, const ImVec2* anchor)
{
	int idx = m_Doc->zoom_idx + delta;
	if (idx < 0) idx = 0;
	if (idx >= kZoomCount) idx = kZoomCount - 1;
	if (idx == m_Doc->zoom_idx) return;
	RememberView();
	if (anchor)
	{
		// keep the world point under the cursor in place
		ImVec2 w = ToWorld(*anchor);
		m_Doc->zoom_idx = idx;
		ImVec2 p = ToScreen(w);
		const float s = Scale();
		m_Doc->view_cx += (p.x - anchor->x) / s;
		m_Doc->view_cy += (p.y - anchor->y) / s;
	}
	else
		m_Doc->zoom_idx = idx;
}

void NqCanvas::Center(float wx, float wy)
{
	const float x = NqLayout::Sane(wx), y = NqLayout::Sane(wy);
	if (x != m_Doc->view_cx || y != m_Doc->view_cy) RememberView();
	CancelFraming();
	m_Doc->view_cx = x;
	m_Doc->view_cy = y;
}

void NqCanvas::FrameRect(const ImVec2& wmin, const ImVec2& wmax)
{
	float w = wmax.x - wmin.x + 2 * NqLayout::kGapX, h = wmax.y - wmin.y + 2 * NqLayout::kGapY;
	if (w < 1.f) w = 1.f;
	if (h < 1.f) h = 1.f;
	const float dpi = XUI::Dpi();
	int idx = kZoomCount - 1;
	if (m_Size.x > 0 && m_Size.y > 0)
		while (idx > 0 && (w * kZoom[idx] * dpi > m_Size.x || h * kZoom[idx] * dpi > m_Size.y)) --idx;
	if (idx > kZoomDefault) idx = kZoomDefault;		// framing never zooms in past 1:1
	const float cx = NqLayout::Sane((wmin.x + wmax.x) * 0.5f);
	const float cy = NqLayout::Sane((wmin.y + wmax.y) * 0.5f);
	if (cx != m_Doc->view_cx || cy != m_Doc->view_cy || idx != m_Doc->zoom_idx) RememberView();
	m_Doc->zoom_idx = idx;
	m_Doc->view_cx = cx;
	m_Doc->view_cy = cy;
}

// An edge that climbs back up runs in a channel beside the nodes: a view fitted to the boxes alone cut it
// off. So the edges are routed here the way BuildLinks routes them, in the same order - the lanes routed
// before an edge move it - and each one counts where its route runs.
bool NqCanvas::Extent(const xr_vector<bool>* only, ImVec2& mn, ImVec2& mx)
{
	BuildGeometry();
	const SNqQuest& q = m_Doc->quest;
	mn = ImVec2(FLT_MAX, FLT_MAX);
	mx = ImVec2(-FLT_MAX, -FLT_MAX);
	auto shown = [&](int i) { return !only || (*only)[i]; };
	auto grow = [&](const ImVec2& p) { mn.x = _min(mn.x, p.x); mn.y = _min(mn.y, p.y); mx.x = _max(mx.x, p.x); mx.y = _max(mx.y, p.y); };
	for (u32 i = 0; i < m_Geom.size(); ++i)
	{
		if (!shown((int)i)) continue;
		grow(m_Geom[i].pos);
		grow(Add(m_Geom[i].pos, m_Geom[i].size));
	}
	if (mn.x > mx.x) return false;

	xr_vector<SLane> lanes;
	xr_vector<ImVec2> route;
	for (u32 i = 0; i < q.nodes.size() && i < m_Geom.size(); ++i)
	{
		const SNqNode& n = q.nodes[i];
		const SNodeGeom& g = m_Geom[i];
		for (u32 p = 0; p < n.out.size(); ++p)
		{
			int pin = -1;
			for (u32 e = 0; e < g.pins.size(); ++e) if (g.pins[e] == n.out[p].first) { pin = (int)e; break; }
			if (pin < 0) continue;
			for (u32 t = 0; t < n.out[p].second.size(); ++t)
			{
				const int to = q.NodeIndex(n.out[p].second[t].c_str());
				if (to < 0 || to >= (int)m_Geom.size()) continue;
				const ImVec2 w0 = OutputPin(g, pin), w3 = InputPin(m_Geom[to]);
				// an edge that runs down stays between its two pins
				if (w3.y - w0.y >= kBackStub * 2.f) continue;
				RouteBack(w0, w3, lanes, route);
				if (!shown((int)i) || !shown(to)) continue;
				for (const ImVec2& r : route) grow(r);
			}
		}
	}
	return true;
}

// The highest level, not past 1:1 and never below FrameMinLevel, that holds the whole graph, its edges
// included. A graph framed so small that no title can be read is a picture, not an editor: one that does
// not fit at the floor is shown AT the floor - the most of it that stays readable - from where reading
// starts: the trigger (every root when there is none) and the two levels under it in view, the top of the
// view on them, and the rest of the view filled with graph. The status says so, and the edges of the
// canvas count what lies beyond them (DrawBeyond).
void NqCanvas::FrameAll()
{
	const SNqQuest& q = m_Doc->quest;
	ImVec2 mn, mx;
	if (q.nodes.empty() || !Extent(0, mn, mx))
	{
		if (m_Doc->view_cx != 0.f || m_Doc->view_cy != 0.f || m_Doc->zoom_idx != kZoomDefault) RememberView();
		m_Doc->view_cx = m_Doc->view_cy = 0.f;
		m_Doc->zoom_idx = kZoomDefault;
		return;
	}
	const float dpi = XUI::Dpi();
	auto apply = [&](int idx, float cx, float cy)
	{
		cx = NqLayout::Sane(cx);
		cy = NqLayout::Sane(cy);
		if (cx != m_Doc->view_cx || cy != m_Doc->view_cy || idx != m_Doc->zoom_idx) RememberView();
		m_Doc->zoom_idx = idx;
		m_Doc->view_cx = cx;
		m_Doc->view_cy = cy;
	};
	mn = Sub(mn, ImVec2(kFrameMargin, kFrameMargin));
	mx = Add(mx, ImVec2(kFrameMargin, kFrameMargin));
	for (int idx = kZoomDefault; idx >= kZoomFrameMin; --idx)
	{
		const float s = kZoom[idx] * dpi;
		if ((mx.x - mn.x) * s <= m_Size.x && (mx.y - mn.y) * s <= m_Size.y)
		{
			apply(idx, (mn.x + mx.x) * 0.5f, (mn.y + mx.y) * 0.5f);
			return;
		}
	}

	// the trigger (every root when there is none) and two levels of what follows it
	xr_vector<bool> keep;
	keep.assign(q.nodes.size(), false);
	xr_vector<int> front, next;
	for (u32 i = 0; i < q.nodes.size(); ++i)
		if (NqText::IsTrigger(q.nodes[i].kind.c_str())) front.push_back((int)i);
	if (front.empty())
		for (u32 i = 0; i < q.nodes.size(); ++i)
			if (0 == q.InDegree(q.nodes[i].id.c_str())) front.push_back((int)i);
	if (front.empty()) front.push_back(0);
	const int anchor = front[0];
	for (int depth = 0; depth <= 2 && !front.empty(); ++depth)
	{
		next.clear();
		for (int i : front)
		{
			if (keep[i]) continue;
			keep[i] = true;
			const SNqNode& n = q.nodes[i];
			for (u32 p = 0; p < n.out.size(); ++p)
				for (u32 t = 0; t < n.out[p].second.size(); ++t)
				{
					const int to = q.NodeIndex(n.out[p].second[t].c_str());
					if (to >= 0 && !keep[to]) next.push_back(to);
				}
		}
		front.swap(next);
	}
	ImVec2 kmn, kmx;
	Extent(&keep, kmn, kmx);
	kmn = Sub(kmn, ImVec2(kFrameMargin, kFrameMargin));
	kmx = Add(kmx, ImVec2(kFrameMargin, kFrameMargin));

	const float s = kZoom[kZoomFrameMin] * dpi;
	const ImVec2 view(m_Size.x / s, m_Size.y / s);
	const SNodeGeom& a = m_Geom[anchor];
	// across: the whole graph's middle, as far as the start allows; down: the start at the top
	const float cx = Along(mn.x, mx.x, kmn.x, kmx.x, view.x, (mn.x + mx.x) * 0.5f, a.pos.x + a.size.x * 0.5f);
	const float cy = Along(mn.y, mx.y, kmn.y, kmx.y, view.y, kmn.y + view.y * 0.5f, a.pos.y - kFrameMargin + view.y * 0.5f);
	apply(kZoomFrameMin, cx, cy);
	SetStatus(NqUtil::Format("The %s is larger than the view at %d%%: framed from its start",
		NqLook::DocNoun(Behaviour()), int(kZoom[kZoomFrameMin] * 100.f + 0.5f)).c_str());
}

void NqCanvas::FrameSelection()
{
	if (m_Doc->selection.empty()) { FrameAll(); return; }
	ImVec2 mn(FLT_MAX, FLT_MAX), mx(-FLT_MAX, -FLT_MAX);
	int cnt = 0;
	for (u32 i = 0; i < m_Doc->selection.size(); ++i)
	{
		const SNqNode* n = m_Doc->quest.FindNode(m_Doc->selection[i].c_str());
		if (!n) continue;
		Fvector2 sz = NqLayout::NodeSize(*n);
		mn.x = _min(mn.x, n->pos.x); mn.y = _min(mn.y, n->pos.y);
		mx.x = _max(mx.x, n->pos.x + sz.x); mx.y = _max(mx.y, n->pos.y + sz.y);
		++cnt;
	}
	if (cnt) FrameRect(mn, mx);
}

void NqCanvas::FrameNode(LPCSTR id)
{
	const SNqNode* n = m_Doc->quest.FindNode(id);
	if (!n) return;
	CancelFraming();
	Fvector2 sz = NqLayout::NodeSize(*n);
	const float cx = n->pos.x + sz.x * 0.5f, cy = n->pos.y + sz.y * 0.5f;
	const int zoom = _max(m_Doc->zoom_idx, kZoomDefault);
	const bool same_selection = m_Doc->selection.size() == 1 && m_Doc->selection[0] == id && m_Doc->sel_slot.empty();
	if (cx != m_Doc->view_cx || cy != m_Doc->view_cy || zoom != m_Doc->zoom_idx || !same_selection) RememberView();
	m_Doc->view_cx = cx;
	m_Doc->view_cy = cy;
	m_Doc->zoom_idx = zoom;
	SelectOnly(id);
}

//------------------------------------------------------------------------------
// selection helpers
//------------------------------------------------------------------------------
bool NqCanvas::IsSelected(LPCSTR id) const
{
	for (u32 i = 0; i < m_Doc->selection.size(); ++i)
		if (m_Doc->selection[i] == id) return true;
	return false;
}

void NqCanvas::SelectOnly(LPCSTR id)
{
	m_Doc->selection.clear();
	if (id && id[0]) m_Doc->selection.push_back(id);
	m_Doc->sel_slot.clear();
}

void NqCanvas::ToggleSelected(LPCSTR id)
{
	for (u32 i = 0; i < m_Doc->selection.size(); ++i)
		if (m_Doc->selection[i] == id) { m_Doc->selection.erase(m_Doc->selection.begin() + i); return; }
	m_Doc->selection.push_back(id);
}

void NqCanvas::SelectNodes(const xr_vector<xr_string>& ids)
{
	m_Doc->selection.clear();
	m_Doc->sel_slot.clear();
	for (u32 i = 0; i < ids.size(); ++i)
		if (!IsSelected(ids[i].c_str())) m_Doc->selection.push_back(ids[i]);
}

void NqCanvas::SelectAll()
{
	m_Doc->selection.clear();
	for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
		m_Doc->selection.push_back(m_Doc->quest.nodes[i].id);
}

void NqCanvas::DeleteSelection()
{
	if (m_Doc->selection.empty()) return;
	xr_string err;
	xr_vector<xr_string> ids = m_Doc->selection;
	if (m_Doc->RemoveNodes(ids, err))
	{
		m_Doc->selection.clear();
		m_Doc->sel_slot.clear();
	}
	else
		Msg("! [nq] delete: %s", err.c_str());
}

void NqCanvas::CopySelection()
{
	if (m_Doc->selection.empty()) return;
	xr_string text = "return {\n  nodes = {\n";
	for (u32 i = 0; i < m_Doc->selection.size(); ++i)
	{
		const SNqNode* n = m_Doc->quest.FindNode(m_Doc->selection[i].c_str());
		if (!n) continue;
		text += "    ";
		text += NqLua::WriteNode(*n, 4);
		text += ",\n";
	}
	text += "  },\n}\n";
	ImGui::SetClipboardText(text.c_str());
}

void NqCanvas::PasteClipboard()
{
	LPCSTR clip = ImGui::GetClipboardText();
	if (!clip || !clip[0]) return;
	SNqValue v; NqLua::SError err;
	if (!NqLua::ParseValue(clip, (u32)xr_strlen(clip), "clipboard", v, err)) { Msg("! [nq] paste: %s", err.message.c_str()); return; }
	const SNqValue* nodes = v.Get("nodes");
	if (!nodes || !nodes->IsTable()) { Msg("! [nq] paste: no nodes table"); return; }

	// read, remap ids, drop edges to nodes outside the fragment
	xr_vector<SNqNode> in;
	xr_vector<xr_string> shape;
	for (u32 i = 0; i < nodes->arr.size(); ++i)
	{
		SNqNode n;
		if (NqLua::NodeFromValue(nodes->arr[i], n, shape)) in.push_back(n);
	}
	if (in.empty()) return;
	ImVec2 anchor = m_Hovered ? ToWorld(ImGui::GetMousePos()) : ImVec2(m_Doc->view_cx, m_Doc->view_cy);
	InsertNodes(in, anchor, true);
}

void NqCanvas::DuplicateSelection()
{
	if (m_Doc->selection.empty()) return;
	// straight from the document: no system clipboard to clobber, and one undo
	// step instead of the copy+paste+move triple
	xr_vector<SNqNode> src;
	for (u32 i = 0; i < m_Doc->selection.size(); ++i)
		if (const SNqNode* n = m_Doc->quest.FindNode(m_Doc->selection[i].c_str())) src.push_back(*n);
	InsertNodes(src, ImVec2(NqLayout::kGrid * 2.f, NqLayout::kGrid * 2.f), false);
}

// remaps the ids against the document, drops edges that leave the fragment,
// places the nodes and selects them - all inside one NqDoc::Edit, so paste and
// duplicate are a single undo step each. `place` is the world anchor of the
// first node when `anchor`, otherwise a delta added to every position.
void NqCanvas::InsertNodes(xr_vector<SNqNode>& in, const ImVec2& place, bool anchor)
{
	if (in.empty()) return;
	xr_vector<std::pair<xr_string, xr_string> > remap;
	xr_vector<xr_string> taken;
	for (u32 i = 0; i < in.size(); ++i)
	{
		xr_string base = in[i].id.empty() ? "node" : in[i].id;
		xr_string nid = base;
		// unique against the document AND the already remapped fragment
		for (int k = 2; ; ++k)
		{
			bool clash = 0 != m_Doc->quest.FindNode(nid.c_str());
			for (u32 t = 0; !clash && t < taken.size(); ++t) if (taken[t] == nid) clash = true;
			if (!clash) break;
			nid = NqUtil::Format("%s%d", base.c_str(), k);
		}
		taken.push_back(nid);
		remap.push_back(std::make_pair(in[i].id, nid));
		in[i].id = nid;
	}
	// keep relative placement: shift the fragment so its first node lands on the anchor
	Fvector2 shift; shift.set(place.x, place.y);
	if (anchor)
	{
		shift.set(0.f, 0.f);
		if (in[0].has_pos) shift.set(place.x - in[0].pos.x, place.y - in[0].pos.y);
	}
	m_Doc->selection.clear();
	m_Doc->Edit([&](SNqQuest& q)
	{
		float stack_y = place.y;
		for (u32 i = 0; i < in.size(); ++i)
		{
			SNqNode n = in[i];
			for (u32 p = 0; p < n.out.size(); ++p)
			{
				xr_vector<xr_string> kept;
				for (u32 t = 0; t < n.out[p].second.size(); ++t)
					for (u32 r = 0; r < remap.size(); ++r)
						if (remap[r].first == n.out[p].second[t]) { kept.push_back(remap[r].second); break; }
				n.out[p].second = kept;
			}
			if (n.has_pos) { n.pos.x = Snap(n.pos.x + shift.x); n.pos.y = Snap(n.pos.y + shift.y); }
			else
			{
				// nodes without a place stack down, each under the one before it
				n.has_pos = true;
				n.pos.set(Snap(place.x), Snap(stack_y));
				stack_y += NqLayout::NodeSize(n).y + NqLayout::kGapY;
			}
			q.nodes.push_back(n);
			m_Doc->selection.push_back(n.id);
		}
	});
}

//------------------------------------------------------------------------------
// geometry
//------------------------------------------------------------------------------
void NqCanvas::BuildGeometry()
{
	const SNqQuest& q = m_Doc->quest;
	const u32 catalog_generation = NqCatalog::Generation();
	if (!m_Dragging && m_GeomRevision == m_Doc->revision && m_GeomCatalogGeneration == catalog_generation
		&& m_Geom.size() == q.nodes.size()) return;
	m_Geom.resize(q.nodes.size());
	for (u32 i = 0; i < q.nodes.size(); ++i)
	{
		const SNqNode& n = q.nodes[i];
		SNodeGeom& g = m_Geom[i];
		NqLayout::Box(n, g.box);
		NqLayout::Lines(n, g.lines);
		// a condition is named the way the document speaks: a behaviour graph never says "quest"
		for (NqLayout::SLine& line : g.lines)
			if (!line.cond.empty()) line.key = NqLayout::CondKey(NqLook::KindTitle(line.cond.c_str(), Behaviour()).c_str(), line.negate);
		NqLayout::Pins(n, g.pins, &g.declared);
		g.pos = ImVec2(n.pos.x, n.pos.y);
		g.size = ImVec2(g.box.w, g.box.h);
		g.trigger = NqText::IsTrigger(n.kind.c_str());
	}
	m_GeomRevision = m_Doc->revision;
	m_GeomCatalogGeneration = catalog_generation;
}

const NqCanvas::SNodeGeom* NqCanvas::GeomOf(LPCSTR id, int* index) const
{
	int i = m_Doc->quest.NodeIndex(id);
	if (index) *index = i;
	return (i >= 0 && i < (int)m_Geom.size()) ? &m_Geom[i] : 0;
}

ImVec2 NqCanvas::InputPin(const SNodeGeom& g) const
{
	return ImVec2(g.pos.x + g.size.x * 0.5f, g.pos.y);
}

ImVec2 NqCanvas::OutputPin(const SNodeGeom& g, int pin) const
{
	int n = (int)g.pins.size();
	float x = g.pos.x + g.size.x * float(pin + 1) / float(n + 1);
	return ImVec2(x, g.pos.y + g.size.y);
}

void NqCanvas::BuildLinks()
{
	const SNqQuest& q = m_Doc->quest;
	const u32 catalog_generation = NqCatalog::Generation();
	const float s = Scale();
	if (!m_Dragging && m_LinkRevision == m_Doc->revision && m_LinkCatalogGeneration == catalog_generation
		&& m_LinkOrigin.x == m_Origin.x && m_LinkOrigin.y == m_Origin.y
		&& m_LinkSize.x == m_Size.x && m_LinkSize.y == m_Size.y
		&& m_LinkViewX == m_Doc->view_cx && m_LinkViewY == m_Doc->view_cy
		&& m_LinkScale == s) return;

	m_Links.clear();
	xr_vector<SLane> lanes;
	xr_vector<ImVec2> route;
	for (u32 i = 0; i < q.nodes.size(); ++i)
	{
		const SNqNode& n = q.nodes[i];
		const SNodeGeom& g = m_Geom[i];
		for (u32 p = 0; p < n.out.size(); ++p)
		{
			int pin = -1;
			for (u32 e = 0; e < g.pins.size(); ++e) if (g.pins[e] == n.out[p].first) { pin = (int)e; break; }
			if (pin < 0) continue;
			for (u32 t = 0; t < n.out[p].second.size(); ++t)
			{
				int to = q.NodeIndex(n.out[p].second[t].c_str());
				if (to < 0) continue;
				SLinkGeom l;
				l.from = (int)i; l.pin = n.out[p].first; l.to = to;
				const ImVec2 w0 = OutputPin(g, pin), w3 = InputPin(m_Geom[to]);
				l.p0 = ToScreen(w0);
				l.p3 = ToScreen(w3);
				// A bezier that has to climb back up cuts through every node on the way (a loop
				// back to a wait): an edge whose input is not below its pin goes around them.
				if (w3.y - w0.y < kBackStub * 2.f)
				{
					RouteBack(w0, w3, lanes, route);
					l.path.resize(route.size());
					l.bmin = l.bmax = l.p0;
					for (u32 k = 0; k < route.size(); ++k)
					{
						l.path[k] = ToScreen(route[k]);
						l.bmin = ImVec2(_min(l.bmin.x, l.path[k].x), _min(l.bmin.y, l.path[k].y));
						l.bmax = ImVec2(_max(l.bmax.x, l.path[k].x), _max(l.bmax.y, l.path[k].y));
					}
					l.p1 = l.p0;
					l.p2 = l.p3;
				}
				else
				{
					float d = _max(kLinkTangent * s, fabsf(l.p3.y - l.p0.y) * 0.4f);
					l.p1 = ImVec2(l.p0.x, l.p0.y + d);
					l.p2 = ImVec2(l.p3.x, l.p3.y - d);
					// a bezier stays inside the hull of its control points
					l.bmin = ImVec2(_min(_min(l.p0.x, l.p1.x), _min(l.p2.x, l.p3.x)), _min(_min(l.p0.y, l.p1.y), _min(l.p2.y, l.p3.y)));
					l.bmax = ImVec2(_max(_max(l.p0.x, l.p1.x), _max(l.p2.x, l.p3.x)), _max(_max(l.p0.y, l.p1.y), _max(l.p2.y, l.p3.y)));
				}
				m_Links.push_back(l);
			}
		}
	}
	m_LinkRevision = m_Doc->revision;
	m_LinkCatalogGeneration = catalog_generation;
	m_LinkOrigin = m_Origin;
	m_LinkSize = m_Size;
	m_LinkViewX = m_Doc->view_cx;
	m_LinkViewY = m_Doc->view_cy;
	m_LinkScale = s;
}

// Straight runs with round corners: a slant down and aside out of the pin, sideways to a free
// vertical channel, up it, over the target and a slant down into its input. The channel keeps
// clear of every node between the two heights and of the channels routed before; a run under the
// pin or over the target that would cut a node moves past it, and the channel is looked for again.
// The two ends slant so that the edge never runs along the vertical of its pins: every edge coming
// down into an input arrives on that vertical (a trigger straight above its wait), and the two read
// as one wire there. A second routed edge through the same pin to the same side slants steeper.
void NqCanvas::RouteBack(const ImVec2& p0, const ImVec2& p3, xr_vector<SLane>& lanes, xr_vector<ImVec2>& out) const
{
	float below = p0.y + kBackStub, above = p3.y - kBackStub, x = p0.x;
	float drop = kBackStub, rise = kBackStub;		// the heights of the two slants
	float side0 = 1.f, side3 = 1.f;				// the side of its pin each slant goes to
	xr_vector<SSpan> spans;
	for (int pass = 0; pass < 4; ++pass)
	{
		spans.clear();
		for (const SNodeGeom& g : m_Geom)
			if (g.pos.y - kBackClear < below && g.pos.y + g.size.y + kBackClear > above)
				spans.push_back({ g.pos.x - kBackClear, g.pos.x + g.size.x + kBackClear });
		for (const SLane& lane : lanes)
			if (lane.top < below && lane.bottom > above)
				spans.push_back({ lane.x - kBackLane, lane.x + kBackLane });
		x = PickChannel(spans, p0.x, p3.x);

		bool moved = false;
		side0 = x < p0.x ? -1.f : 1.f;
		side3 = x < p3.x ? -1.f : 1.f;
		int out_before = 0, in_before = 0;
		for (const SLane& lane : lanes)
		{
			if (lane.from.x == p0.x && lane.from.y == p0.y && lane.side_from == side0) ++out_before;
			if (lane.to.x == p3.x && lane.to.y == p3.y && lane.side_to == side3) ++in_before;
		}
		drop = kBackStub + kBackLane * float(out_before);
		rise = kBackStub + kBackLane * float(in_before);
		if (below < p0.y + drop)	{ below = p0.y + drop; moved = true; }
		if (above > p3.y - rise)	{ above = p3.y - rise; moved = true; }

		auto crosses = [](const SNodeGeom& g, float y, float x0, float x1)
		{
			const float margin = kBackStub * 0.5f;
			return y > g.pos.y - margin && y < g.pos.y + g.size.y + margin
				&& g.pos.x < _max(x0, x1) && g.pos.x + g.size.x > _min(x0, x1);
		};
		for (const SNodeGeom& g : m_Geom)
		{
			if (crosses(g, below, p0.x, x))		{ below = _max(below, g.pos.y + g.size.y + kBackStub); moved = true; }
			if (crosses(g, above, x, p3.x))		{ above = _min(above, g.pos.y - kBackStub); moved = true; }
		}
		if (!moved) break;
	}
	SLane lane;
	lane.x = x; lane.top = above; lane.bottom = below;
	lane.from = p0; lane.to = p3;
	lane.side_from = side0; lane.side_to = side3;
	lanes.push_back(lane);

	// a run moved past a node gets a vertical piece between its slant and its sideways run
	const float xs = p0.x + side0 * kBackStub, xe = p3.x + side3 * kBackStub;
	xr_vector<ImVec2> corners;
	corners.push_back(p0);
	corners.push_back(ImVec2(xs, p0.y + drop));
	if (below > p0.y + drop + 0.5f) corners.push_back(ImVec2(xs, below));
	corners.push_back(ImVec2(x, below));
	corners.push_back(ImVec2(x, above));
	corners.push_back(ImVec2(xe, above));
	if (above < p3.y - rise - 0.5f) corners.push_back(ImVec2(xe, p3.y - rise));
	corners.push_back(p3);

	out.clear();
	out.push_back(p0);
	for (u32 k = 1; k + 1 < corners.size(); ++k)
	{
		const ImVec2& from = corners[k - 1];
		const ImVec2& c = corners[k];
		const ImVec2& to = corners[k + 1];
		const float in = sqrtf(Dist2(from, c)), next = sqrtf(Dist2(c, to));
		const float r = _min(kBackCorner, _min(in, next) * 0.5f);
		if (r < 0.5f)
		{
			if (Dist2(out.back(), c) > 0.01f) out.push_back(c);
			continue;
		}
		// a quadratic corner: from r before it, through it, to r after it
		const ImVec2 u(c.x + (from.x - c.x) / in * r, c.y + (from.y - c.y) / in * r);
		const ImVec2 v(c.x + (to.x - c.x) / next * r, c.y + (to.y - c.y) / next * r);
		for (int step = 0; step <= 4; ++step)
		{
			const float f = step / 4.f, e = 1.f - f;
			out.push_back(ImVec2(e * e * u.x + 2.f * e * f * c.x + f * f * v.x, e * e * u.y + 2.f * e * f * c.y + f * f * v.y));
		}
	}
	out.push_back(p3);
}

float NqCanvas::PinGrab() const
{
	const float drawn = _max(2.f, kPinR * Scale());		// the dot DrawNode paints
	const float r = drawn + XUI::Pt(kPinGrabPad);
	return _min(_max(r, XUI::Pt(kPinGrabMin)), XUI::Pt(kPinGrabMax));
}

// `inward` points from the pin into its node (-1 for an output pin on the bottom
// edge, +1 for the input pin on the top one): that half of the zone stays narrow
bool NqCanvas::NearPin(const ImVec2& p, const ImVec2& c, float inward, float& d2) const
{
	ImVec2 d = Sub(p, c);
	if (d.y * inward > 0.f) d.y /= kPinGrabInside;
	d2 = d.x * d.x + d.y * d.y;
	const float r = PinGrab();
	return d2 <= r * r;
}

int NqCanvas::HitNode(const ImVec2& p) const
{
	// topmost = last drawn = highest index
	for (int i = (int)m_Geom.size() - 1; i >= 0; --i)
	{
		const SNodeGeom& g = m_Geom[i];
		ImVec2 a = ToScreen(g.pos), b = ToScreen(Add(g.pos, g.size));
		if (InRect(p, a, b)) return i;
	}
	return -1;
}

int NqCanvas::HitNodeNear(const ImVec2& p, int except) const
{
	const float r = PinGrab(), lim = r * r;
	float best = FLT_MAX;
	int hit = -1;
	// topmost first, strict <: the node drawn last wins an exact tie
	for (int i = (int)m_Geom.size() - 1; i >= 0; --i)
	{
		if (i == except) continue;
		const SNodeGeom& g = m_Geom[i];
		ImVec2 a = ToScreen(g.pos), b = ToScreen(Add(g.pos, g.size));
		float dx = p.x < a.x ? a.x - p.x : (p.x > b.x ? p.x - b.x : 0.f);
		float dy = p.y < a.y ? a.y - p.y : (p.y > b.y ? p.y - b.y : 0.f);
		float d = dx * dx + dy * dy;
		if (d <= lim && d < best) { best = d; hit = i; }
	}
	return hit;
}

int NqCanvas::HitPin(int node, const ImVec2& p, float* out_d2) const
{
	if (node < 0) return -1;
	const SNodeGeom& g = m_Geom[node];
	float best = FLT_MAX;
	int hit = -1;
	// pins of one node sit close together at low zoom, so the nearest one wins
	for (u32 i = 0; i < g.pins.size(); ++i)
	{
		float d2;
		if (NearPin(p, ToScreen(OutputPin(g, (int)i)), -1.f, d2) && d2 < best) { best = d2; hit = (int)i; }
	}
	if (hit >= 0 && out_d2) *out_d2 = best;
	return hit;
}

bool NqCanvas::HitAnyPin(const ImVec2& p, int& node, int& pin) const
{
	node = pin = -1;
	float best = FLT_MAX;
	for (int i = (int)m_Geom.size() - 1; i >= 0; --i)
	{
		float d2 = FLT_MAX;
		int hit = HitPin(i, p, &d2);
		if (hit >= 0 && d2 < best) { best = d2; node = i; pin = hit; }
	}
	return pin >= 0;
}

bool NqCanvas::ActionAtRow(const SNqNode& n, int row, xr_string& slot, int& index) const
{
	if (row < 0) return false;
	if (row < (int)n.on_enter.size()) { slot = "enter"; index = row; return true; }
	row -= (int)n.on_enter.size();
	if (row < (int)n.on_exit.size()) { slot = "exit"; index = row; return true; }
	return false;
}

bool NqCanvas::HitAction(int node, const ImVec2& p, SActionHit& hit) const
{
	hit = SActionHit();
	if (node < 0 || node >= (int)m_Geom.size() || Lod() < 2) return false;
	const SNodeGeom& g = m_Geom[node];
	if (!g.box.actions) return false;
	const float s = Scale();
	const ImVec2 a = ToScreen(g.pos);
	if (p.x < a.x || p.x > a.x + g.size.x * s) return false;
	const float y0 = a.y + g.box.actions_y * s;
	const int rows = g.box.action_rows + (g.box.more ? 1 : 0);
	if (p.y < y0) return false;
	const int r = (int)floorf((p.y - y0) / (NqLayout::kActionRow * s));
	if (r < 0 || r >= rows) return false;
	hit.node = node;
	hit.row = r;
	if (r >= g.box.action_rows) { hit.more = true; return true; }
	return ActionAtRow(m_Doc->quest.nodes[node], r, hit.slot, hit.index);
}

bool NqCanvas::HitHeader(int node, const ImVec2& p) const
{
	if (node < 0 || node >= (int)m_Geom.size()) return false;
	const SNodeGeom& g = m_Geom[node];
	const ImVec2 a = ToScreen(g.pos);
	const ImVec2 b(a.x + g.size.x * Scale(), a.y + NqLayout::kHeader * Scale());
	return InRect(p, a, b);
}

bool NqCanvas::DropTarget(const ImVec2& p, int& node, xr_string& slot, int& index, float* line_y) const
{
	node = HitNode(p);
	if (node < 0 || m_Geom[node].trigger || Lod() < 2) return false;
	const SNqNode& n = m_Doc->quest.nodes[node];
	const SNodeGeom& g = m_Geom[node];
	const float s = Scale();
	const ImVec2 a = ToScreen(g.pos);
	// the header takes it as the node's last on_enter action
	if (HitHeader(node, p))
	{
		slot = "enter";
		index = (int)n.on_enter.size();
		if (line_y) *line_y = a.y + NqLayout::kHeader * s;
		return true;
	}
	SActionHit hit;
	if (!HitAction(node, p, hit)) return false;
	const float top = a.y + (g.box.actions_y + NqLayout::kActionRow * float(hit.row)) * s;
	if (hit.more)
	{
		slot = n.on_exit.empty() ? "enter" : "exit";
		index = (int)(slot == "exit" ? n.on_exit.size() : n.on_enter.size());
		if (line_y) *line_y = top;
		return true;
	}
	slot = hit.slot;
	index = hit.index;
	float line = top;
	// the lower half of a row puts it after that row
	if (p.y > top + NqLayout::kActionRow * s * 0.5f) { ++index; line = top + NqLayout::kActionRow * s; }
	if (line_y) *line_y = line;
	return true;
}

// pin under the cursor, for the hover highlight. While a link is in the air the
// only pin that matters is the input it would land on - the very node EndLink
// will pick, so the highlight and the drop can never disagree.
void NqCanvas::UpdateHot()
{
	m_HotNode = m_HotPin = m_HotLink = -1;
	m_HotAction = SActionHit();
	// A menu is a mode: while one is open the cursor is aimed at it, and lighting up
	// whatever happens to be under it - let alone letting D delete that - is not what
	// the author is doing.
	if (!m_Hovered || MenuOpen()) return;
	const ImVec2 mouse = ImGui::GetMousePos();
	if (m_Linking)
	{
		int from = m_Doc->quest.NodeIndex(m_LinkFrom.c_str());
		int node = HitNode(mouse);
		if (node < 0) node = HitNodeNear(mouse, from);
		if (node >= 0 && node != from && !m_Geom[node].trigger) m_HotNode = node;
		return;
	}
	if (m_Dragging || m_Marquee || m_Panning || m_ActDragging) return;
	float best = FLT_MAX;
	for (int i = (int)m_Geom.size() - 1; i >= 0; --i)
	{
		const SNodeGeom& g = m_Geom[i];
		float d2 = FLT_MAX;
		int p = HitPin(i, mouse, &d2);
		if (p >= 0 && d2 < best) { best = d2; m_HotNode = i; m_HotPin = p; }
		if (!g.trigger && NearPin(mouse, ToScreen(InputPin(g)), 1.f, d2) && d2 < best) { best = d2; m_HotNode = i; m_HotPin = -1; }
	}
	const int over = HitNode(mouse);
	if (m_HotNode < 0 && over >= 0) HitAction(over, mouse, m_HotAction);
	// a pin wins over a link it starts from, and a node body swallows the wires behind it
	if (m_HotNode < 0 && over < 0) m_HotLink = HitLink(mouse);
}

// True while any popup of this canvas is up. Every shortcut and every hover cue is
// off for its lifetime: the menu owns the input, and a key that also does something
// underneath it would fire twice as far as the author can tell.
bool NqCanvas::MenuOpen() const
{
	return ImGui::IsPopupOpen(0, ImGuiPopupFlags_AnyPopupId | ImGuiPopupFlags_AnyPopupLevel);
}

int NqCanvas::HitLink(const ImVec2& p) const
{
	// the curve is one to four pixels thick: half of it plus the same slack a pin gets
	const float r = XUI::Pt(kLinkGrab) + _max(1.f, 2.f * Scale()) * 0.5f;
	float best = r * r;		// squared pixels
	int hit = -1;
	for (u32 i = 0; i < m_Links.size(); ++i)
	{
		const SLinkGeom& l = m_Links[i];
		if (p.x < l.bmin.x - r || p.y < l.bmin.y - r || p.x > l.bmax.x + r || p.y > l.bmax.y + r) continue;
		if (!l.path.empty())
		{
			for (u32 k = 1; k < l.path.size(); ++k)
			{
				const float d = SegDist2(p, l.path[k - 1], l.path[k]);
				if (d < best) { best = d; hit = (int)i; }
			}
			continue;
		}
		ImVec2 prev = l.p0;
		for (int k = 1; k <= 16; ++k)
		{
			ImVec2 cur = Bezier(l.p0, l.p1, l.p2, l.p3, k / 16.f);
			float d = SegDist2(p, prev, cur);
			if (d < best) { best = d; hit = (int)i; }
			prev = cur;
		}
	}
	return hit;
}

void NqCanvas::EnsureReachable()
{
	if (m_ReachRevision == m_Doc->revision && m_Reachable.size() == m_Doc->quest.nodes.size()) return;
	m_Doc->quest.Reachable(m_Reachable);
	m_ReachRevision = m_Doc->revision;
}

int NqCanvas::Severity(LPCSTR node_id) const
{
	int sev = 0;
	for (u32 i = 0; i < m_Doc->problems.size(); ++i)
		if (m_Doc->problems[i].node_id == node_id)
			sev = _max(sev, m_Doc->problems[i].IsError() ? 2 : 1);
	return sev;
}

xr_string NqCanvas::ProblemText(LPCSTR node_id) const
{
	xr_string text;
	for (u32 i = 0; i < m_Doc->problems.size(); ++i)
		if (m_Doc->problems[i].node_id == node_id)
		{
			if (!text.empty()) text += "\n";
			text += m_Doc->problems[i].Text();
		}
	return text;
}

xr_string NqCanvas::ActionText(const SNqAction& a, LPCSTR slot, int index) const
{
	xr_string text = NqUtil::Format("%s %d: %s", 0 == strcmp(slot, "exit") ? "On exit" : "On enter", index + 1,
		NqLook::KindTitle(a.kind.c_str(), Behaviour()).c_str());
	const xr_string sum = NqText::ValueSummary(a.params, 96);
	if (!sum.empty() && sum != "{}") { text += "\n"; text += sum; }
	text += "\nDouble-click to edit it, drag to move it";
	return text;
}

xr_string NqCanvas::NodeTitle(const SNqNode& n) const
{
	return NqLook::KindTitle(n.kind.c_str(), Behaviour());
}

xr_string NqCanvas::NodeSummary(const SNqNode& n) const
{
	// what the box says at a glance: its first line
	const int index = m_Doc->quest.NodeIndex(n.id.c_str());
	if (index < 0 || index >= (int)m_Geom.size() || m_Geom[index].lines.empty()) return xr_string();
	const NqLayout::SLine& line = m_Geom[index].lines[0];
	return line.key + (line.cond.empty() ? ": " : " ") + NqUtil::ClipUtf8(line.value, 64);
}

//------------------------------------------------------------------------------
// drawing
//------------------------------------------------------------------------------
void NqCanvas::DrawGrid(ImDrawList* dl)
{
	const float s = Scale();
	float step = NqLayout::kGrid * 4.f * s;
	while (step < XUI::Pt(24.f)) step *= 2.f;
	// the pattern is anchored on the projection of the world origin (fmodf of a
	// negative offset shifts it)
	ImVec2 o = ToScreen(ImVec2(0, 0));
	float sx = o.x - floorf((o.x - m_Origin.x) / step) * step;
	float sy = o.y - floorf((o.y - m_Origin.y) / step) * step;
	if (!_finite(sx) || !_finite(sy) || _isnan(sx) || _isnan(sy)) return;
	// count the lines instead of accumulating a float: far enough from the
	// origin `x += step` stops advancing and the loop would never end
	const int nx = int(m_Size.x / step) + 2, ny = int(m_Size.y / step) + 2;
	const int kMaxLines = 4096;
	if (nx < 1 || ny < 1 || nx > kMaxLines || ny > kMaxLines) return;
	const ImU32 col = C(T::BorderSubtle, 0.45f);
	const float x1 = m_Origin.x + m_Size.x, y1 = m_Origin.y + m_Size.y;
	for (int i = 0; i < nx; ++i)
	{
		const float x = sx + i * step;
		if (x >= x1) break;
		if (x >= m_Origin.x) dl->AddLine(ImVec2(x, m_Origin.y), ImVec2(x, y1), col);
	}
	for (int i = 0; i < ny; ++i)
	{
		const float y = sy + i * step;
		if (y >= y1) break;
		if (y >= m_Origin.y) dl->AddLine(ImVec2(m_Origin.x, y), ImVec2(x1, y), col);
	}
}

void NqCanvas::DrawLinks(ImDrawList* dl)
{
	const float s = Scale();
	const float base = _max(1.f, 2.f * s);
	for (u32 i = 0; i < m_Links.size(); ++i)
	{
		const SLinkGeom& l = m_Links[i];
		// the bounds hold the whole shape (a bezier's control-point hull), padded by the
		// line and the arrow: nothing visible is culled
		const float pad = _max(base, _max(4.f, 7.f * s));
		if (l.bmax.x + pad < m_Origin.x || l.bmax.y + pad < m_Origin.y || l.bmin.x - pad > m_Origin.x + m_Size.x || l.bmin.y - pad > m_Origin.y + m_Size.y) continue;
		const bool sel = IsSelected(m_Doc->quest.nodes[l.from].id.c_str()) || IsSelected(m_Doc->quest.nodes[l.to].id.c_str());
		const bool hot = ((int)i == m_HotLink);
		const ImU32 col = hot ? C(T::Text) : sel ? C(T::AccentBright) : C(T::TextMuted);
		float th = base;
		// The wire the cursor has claimed says so before anything is clicked: Alt+LMB
		// and D act on THIS one, and the author gets to see which that is. A halo
		// under a fatter line reads at every zoom, where a colour change alone does not.
		auto stroke = [&](ImU32 c, float w)
		{
			if (l.path.empty()) dl->AddBezierCubic(l.p0, l.p1, l.p2, l.p3, c, w, 24);
			else dl->AddPolyline(l.path.data(), (int)l.path.size(), c, ImDrawFlags_None, w);
		};
		if (hot)
		{
			stroke(C(T::AccentBright, 0.35f), th * 3.5f + 2.f);
			th *= 1.8f;
		}
		stroke(col, th);
		// arrow head at the target, along the last piece of the wire: a routed edge slants in
		const float a = _max(4.f, 7.f * s) * (hot ? 1.35f : 1.f);
		ImVec2 dir(0.f, 1.f);
		if (l.path.size() >= 2)
		{
			const ImVec2 d = Sub(l.p3, l.path[l.path.size() - 2]);
			const float len = sqrtf(d.x * d.x + d.y * d.y);
			if (len > 0.001f) dir = ImVec2(d.x / len, d.y / len);
		}
		const ImVec2 base(l.p3.x - dir.x * a, l.p3.y - dir.y * a);
		const ImVec2 half(-dir.y * a * 0.6f, dir.x * a * 0.6f);
		dl->AddTriangleFilled(l.p3, Add(base, half), Sub(base, half), col);
	}
}

// The actions of a node, one row each: what it does, in words - a numbered chip said
// nothing until it was hovered. Four rows, then "+N more": the node stays a card, and the
// full list is one click away in the Details panel.
void NqCanvas::DrawActions(ImDrawList* dl, int index, const ImVec2& a, float alpha)
{
	const SNqNode& n = m_Doc->quest.nodes[index];
	const SNodeGeom& g = m_Geom[index];
	const float s = Scale();
	const float w = g.size.x * s;
	const float row_h = NqLayout::kActionRow * s;
	const float y0 = a.y + g.box.actions_y * s;
	const float inset = kInset * s;
	ImFont* body = Face(XUI::Font::Body);
	ImFont* sface = Face(XUI::Font::Small);
	const float line_px = Round(kLinePt * s), small_px = Round(kSmallPt * s);
	const bool node_selected = IsSelected(n.id.c_str()) && m_Doc->selection.size() == 1;

	if (g.box.lines)
		dl->AddLine(ImVec2(a.x + inset, y0 - NqLayout::kPad * 0.5f * s), ImVec2(a.x + w - inset, y0 - NqLayout::kPad * 0.5f * s),
			C(T::BorderSubtle, alpha));
	for (int r = 0; r < g.box.action_rows; ++r)
	{
		xr_string slot; int idx = 0;
		if (!ActionAtRow(n, r, slot, idx)) break;
		const SNqAction& act = (slot == "enter") ? n.on_enter[idx] : n.on_exit[idx];
		const ImVec2 ra(a.x + 3.f * s, y0 + row_h * r), rb(a.x + w - 3.f * s, y0 + row_h * (r + 1));
		const bool selected = node_selected && m_Doc->sel_slot == slot + ":" + NqUtil::Format("%d", idx);
		const bool hot = m_HotAction.node == index && m_HotAction.row == r && !m_HotAction.more;
		const bool dragged = m_ActDragging && m_ActDragNode == n.id && m_ActDragSlot == slot && m_ActDragIndex == idx;
		if (selected)		dl->AddRectFilled(ra, rb, C(T::SelectionFocused, alpha), 3.f * s);
		else if (hot)		dl->AddRectFilled(ra, rb, C(T::Hover, alpha), 3.f * s);
		if (dragged)		dl->AddRect(ra, rb, C(T::AccentBright, alpha), 3.f * s, 0, _max(1.f, 1.5f * s));

		float x = a.x + inset;
		const float icon = 14.f * s;
		const float cy = (ra.y + rb.y) * 0.5f;
		XUI::DrawIconRect(dl, NqLook::KindIcon(act.kind.c_str()), ImVec2(x, cy - icon * 0.5f), ImVec2(x + icon, cy + icon * 0.5f),
			NqLook::FamilyColor(act.kind.c_str(), alpha));
		x += icon + 6.f * s;
		float right = a.x + w - inset;
		// on_exit rows say so; on_enter is what an action usually is
		if (slot == "exit")
		{
			const float tw = TextW(sface, small_px, "exit");
			dl->AddText(sface, small_px, ImVec2(Round(right - tw), Round(cy - small_px * 0.5f)), C(T::TextMuted, alpha), "exit");
			right -= tw + 6.f * s;
		}
		const xr_string title = NqLook::KindTitle(act.kind.c_str(), Behaviour());
		const float tw = PaintFit(dl, body, line_px, ImVec2(x, cy - line_px * 0.5f), C(T::Text, alpha), title.c_str(), right - x);
		// what it acts on, when there is room for it
		const float sx = x + tw + 6.f * s;
		if (right - sx > line_px * 3.f)
		{
			xr_string sum = NqText::ValueSummary(act.params, 64);
			if (sum == "{}") sum.clear();
			PaintFit(dl, body, line_px, ImVec2(sx, cy - line_px * 0.5f), C(T::TextMuted, alpha), sum.c_str(), right - sx);
		}
	}
	if (g.box.more)
	{
		const float top = y0 + row_h * g.box.action_rows;
		const bool hot = m_HotAction.node == index && m_HotAction.more;
		if (hot) dl->AddRectFilled(ImVec2(a.x + 3.f * s, top), ImVec2(a.x + w - 3.f * s, top + row_h), C(T::Hover, alpha), 3.f * s);
		const xr_string more = NqUtil::Format("+%d more", g.box.actions - g.box.action_rows);
		dl->AddText(sface, small_px, ImVec2(Round(a.x + inset), Round(top + (row_h - small_px) * 0.5f)), C(T::TextSecondary, alpha), more.c_str());
	}
}

void NqCanvas::DrawNode(ImDrawList* dl, int index)
{
	const SNqNode& n = m_Doc->quest.nodes[index];
	const SNodeGeom& g = m_Geom[index];
	const float s = Scale();
	const ImVec2 a = ToScreen(g.pos), b = ToScreen(Add(g.pos, g.size));
	// off-screen culling
	if (b.x < m_Origin.x || b.y < m_Origin.y || a.x > m_Origin.x + m_Size.x || a.y > m_Origin.y + m_Size.y) return;

	const bool selected = IsSelected(n.id.c_str());
	const bool reachable = index < (int)m_Reachable.size() ? m_Reachable[index] : true;
	// an unreachable node is dimmed, never hidden: it is still the author's to wire
	const float alpha = reachable ? 1.f : 0.45f;
	const int lod = Lod();
	const float header = NqLayout::kHeader * s;
	const float round = (g.trigger ? NqLayout::kHeader * 0.5f : 6.f) * s;
	const bool header_only = g.box.h <= NqLayout::kHeader + NqLayout::kPad + 0.5f;
	const ImDrawFlags head_corners = header_only ? ImDrawFlags_RoundCornersAll : ImDrawFlags_RoundCornersTop;
	// a node with nothing under its header is all header
	const ImVec2 hb(b.x, header_only ? b.y : a.y + header);

	// the card: body, header tinted by the kind's family, and the family line under it
	dl->AddRectFilled(a, b, C(T::Raised, alpha), round);
	dl->AddRectFilled(a, hb, C(T::Hover, alpha), round, head_corners);
	dl->AddRectFilled(a, hb, NqLook::FamilyColor(n.kind.c_str(), 0.20f * alpha), round, head_corners);
	if (!header_only)
		dl->AddLine(ImVec2(a.x + round * 0.5f, hb.y), ImVec2(b.x - round * 0.5f, hb.y), NqLook::FamilyColor(n.kind.c_str(), alpha), _max(1.f, 2.f * s));

	if (lod >= 1)
	{
		const float inset = kInset * s;
		const float title_px = TitlePx();
		const float small_px = Round(kSmallPt * s);
		ImFont* strong = Face(XUI::Font::Strong);
		ImFont* sface = Face(XUI::Font::Small);
		const float cy = a.y + header * 0.5f;
		float x = a.x + inset;
		float right = b.x - inset;
		if (lod == 2)
		{
			const float icon = 16.f * s;
			XUI::DrawIconRect(dl, NqLook::KindIcon(n.kind.c_str()), ImVec2(x, cy - icon * 0.5f), ImVec2(x + icon, cy + icon * 0.5f),
				NqLook::FamilyColor(n.kind.c_str(), alpha));
			x += icon + 6.f * s;

			// badges from the right: problems, the bookmark, "once"
			const float badge = 14.f * s;
			const int sev = Severity(n.id.c_str());
			if (sev)
			{
				const ImVec2 bm(right - badge, cy - badge * 0.5f);
				XUI::DrawIconRect(dl, sev == 2 ? XUI::Icon::Error : XUI::Icon::Warning, bm, Add(bm, ImVec2(badge, badge)),
					C(sev == 2 ? T::Error : T::Warning, alpha));
				if (m_Hovered && InRect(ImGui::GetMousePos(), bm, Add(bm, ImVec2(badge, badge)))) m_TipNext = ProblemText(n.id.c_str());
				right -= badge + 4.f * s;
			}
			if (IsBookmarked(n.id.c_str()))
			{
				const ImVec2 bm(right - badge, cy - badge * 0.5f);
				XUI::DrawIconRect(dl, XUI::Icon::BookmarkFilled, bm, Add(bm, ImVec2(badge, badge)), C(T::AccentBright, alpha));
				right -= badge + 4.f * s;
			}
			// `once` absent from the file means the catalog default applies
			const NqCatalog::SKind* nk = NqCatalog::Find(n.kind.c_str());
			if (!g.trigger && (n.once_set ? n.once : (nk && nk->once_default)))
			{
				const float tw = TextW(sface, small_px, "once");
				dl->AddText(sface, small_px, ImVec2(Round(right - tw), Round(cy - small_px * 0.5f)), C(T::TextMuted, alpha), "once");
				right -= tw + 6.f * s;
			}
			// the id gets what the title leaves, and never more than 40% of the row
			const xr_string title = NodeTitle(n);
			const float room = _max(0.f, right - x);
			const float title_w = TextW(strong, title_px, title.c_str());
			const float id_w = _min(TextW(sface, small_px, n.id.c_str()), _max(room - title_w - 8.f * s, room * 0.4f));
			if (id_w > small_px * 1.5f)
				PaintFit(dl, sface, small_px, ImVec2(right - id_w, cy - small_px * 0.5f), C(T::TextMuted, alpha), n.id.c_str(), id_w);
			PaintFit(dl, strong, title_px, ImVec2(x, cy - title_px * 0.5f), C(T::Text, alpha), title.c_str(),
				id_w > small_px * 1.5f ? room - id_w - 8.f * s : room);
		}
		else
			PaintFit(dl, strong, title_px, ImVec2(x, cy - title_px * 0.5f), C(T::Text, alpha), NodeTitle(n).c_str(), right - x);
	}

	// the body: lines and actions in words at full detail, placeholder bars below it
	if (lod == 2)
	{
		ImFont* body = Face(XUI::Font::Body);
		const float line_px = Round(kLinePt * s);
		const float inset = kInset * s;
		for (int i = 0; i < g.box.lines && i < (int)g.lines.size(); ++i)
		{
			const NqLayout::SLine& line = g.lines[i];
			const float ly = a.y + (g.box.lines_y + NqLayout::kLine * float(i)) * s + (NqLayout::kLine * s - line_px) * 0.5f;
			// "+N more": no key, only the count
			const float inner = g.size.x * s - inset * 2.f;
			if (line.key.empty())
			{
				const float small_px = Round(kSmallPt * s);
				PaintFit(dl, Face(XUI::Font::Small), small_px, ImVec2(a.x + inset, ly + (line_px - small_px) * 0.5f),
					C(T::TextSecondary, alpha), line.value.c_str(), inner);
				continue;
			}
			// a parameter reads "name: value", a condition "Title what" - a title may hold a colon of its own
			const xr_string key = line.cond.empty() ? line.key + ":" : line.key;
			const float kw = PaintFit(dl, body, line_px, ImVec2(a.x + inset, ly), C(T::TextMuted, alpha), key.c_str(),
				line.value.empty() ? inner : inner * (line.cond.empty() ? 0.45f : 0.6f));
			const float vx = a.x + inset + kw + 6.f * s;
			PaintFit(dl, body, line_px, ImVec2(vx, ly), C(T::Text, alpha), line.value.c_str(), b.x - inset - vx);
		}
		if (g.box.actions) DrawActions(dl, index, a, alpha);
	}
	else
	{
		const ImU32 bar = C(T::BorderSubtle, alpha);
		const float inset = kInset * s;
		for (int i = 0; i < g.box.lines; ++i)
		{
			const float ly = a.y + (g.box.lines_y + NqLayout::kLine * (float(i) + 0.3f)) * s;
			dl->AddRectFilled(ImVec2(a.x + inset, ly), ImVec2(a.x + inset + (g.size.x * s - inset * 2.f) * 0.6f, ly + NqLayout::kLine * 0.4f * s), bar, 2.f * s);
		}
		for (int i = 0; i < g.box.action_rows; ++i)
		{
			const float ly = a.y + (g.box.actions_y + NqLayout::kActionRow * (float(i) + 0.3f)) * s;
			dl->AddRectFilled(ImVec2(a.x + inset, ly), ImVec2(a.x + inset + (g.size.x * s - inset * 2.f) * 0.8f, ly + NqLayout::kActionRow * 0.4f * s), bar, 2.f * s);
		}
	}

	const bool hovered_node = m_Hovered && InRect(ImGui::GetMousePos(), a, b);
	dl->AddRect(a, b, selected ? C(T::AccentBright) : hovered_node ? C(T::BorderStrong, alpha) : C(T::Border, alpha), round, 0,
		selected ? _max(XUI::Pt(2.f), 1.f) : 1.f);

	// the pins sit on the edges; their names are inside the node, where no wire runs
	const float pr = _max(2.f, kPinR * s);
	const float halo = PinGrab();
	const float small_px = Round(kSmallPt * s);
	ImFont* sface = Face(XUI::Font::Small);
	// the pins are spread over the edge at w / (n + 1): that is the room each name has
	const float slot_w = g.size.x * s / float(g.pins.size() + 1);
	for (u32 p = 0; p < g.pins.size(); ++p)
	{
		const ImVec2 c = ToScreen(OutputPin(g, (int)p));
		const bool hot = m_Linking && m_LinkFrom == n.id && m_LinkPin == g.pins[p];
		const bool over = m_HotNode == index && m_HotPin == (int)p;
		if (lod == 2 && g.box.pin_labels)
		{
			xr_string shown;
			if (!XUI::EllipsizeFace(sface, small_px, g.pins[p].c_str(), slot_w - 4.f * s, XUI::Clip::End, shown)) shown = g.pins[p];
			const float tw = TextW(sface, small_px, shown.c_str());
			const float ly = a.y + g.box.pins_y * s + (NqLayout::kPinRow * s - small_px) * 0.5f;
			dl->AddText(sface, small_px, ImVec2(Round(c.x - tw * 0.5f), Round(ly)), (hot || over) ? C(T::AccentBright, alpha) : C(T::TextSecondary, alpha),
				shown.c_str());
		}
		// a pin the cursor can grab grows and shows the halo of its grab zone: the zone
		// is wider than the dot, and nothing else says so
		if (over) dl->AddCircleFilled(c, halo, C(T::AccentBright, 0.15f), 16);
		const float r = over ? pr * 1.4f : pr;
		dl->AddCircleFilled(c, r, (hot || over) ? C(T::AccentBright) : C(T::TextSecondary, alpha), 12);
		dl->AddCircle(c, r, C(T::Canvas), 12, 1.f);
	}
	if (!g.trigger)
	{
		const ImVec2 c = ToScreen(InputPin(g));
		const bool over = (m_HotNode == index && m_HotPin < 0);
		if (over) dl->AddCircleFilled(c, halo, C(T::AccentBright, 0.15f), 16);
		const float r = over ? pr * 1.4f : pr;
		dl->AddCircleFilled(c, r, over ? C(T::AccentBright) : C(T::TextSecondary, alpha), 12);
		dl->AddCircle(c, r, C(T::Canvas), 12, 1.f);
	}
}

void NqCanvas::DrawMarquee(ImDrawList* dl)
{
	if (!m_Marquee) return;
	ImVec2 m = ImGui::GetMousePos();
	ImVec2 a(_min(m.x, m_MarqueeFrom.x), _min(m.y, m_MarqueeFrom.y)), b(_max(m.x, m_MarqueeFrom.x), _max(m.y, m_MarqueeFrom.y));
	dl->AddRectFilled(a, b, C(T::AccentBright, 0.12f));
	dl->AddRect(a, b, C(T::AccentBright, 0.8f));
}

void NqCanvas::DrawLinking(ImDrawList* dl)
{
	if (!m_Linking) return;
	int fi = -1;
	const SNodeGeom* g = GeomOf(m_LinkFrom.c_str(), &fi);
	if (!g) { m_Linking = false; return; }
	int pin = -1;
	for (u32 p = 0; p < g->pins.size(); ++p) if (g->pins[p] == m_LinkPin) { pin = (int)p; break; }
	if (pin < 0) { m_Linking = false; return; }
	ImVec2 p0 = ToScreen(OutputPin(*g, pin)), p3 = ImGui::GetMousePos();
	float d = _max(kLinkTangent * Scale(), fabsf(p3.y - p0.y) * 0.4f);
	dl->AddBezierCubic(p0, ImVec2(p0.x, p0.y + d), ImVec2(p3.x, p3.y - d), p3, C(T::AccentBright), _max(1.f, XUI::Pt(2.f)), 24);
}

// the action being moved rides under the cursor, and the node it would land in shows where
void NqCanvas::DrawActionDrag(ImDrawList* dl)
{
	if (!m_ActDragging) return;
	const SNqNode* src = m_Doc->quest.FindNode(m_ActDragNode.c_str());
	const xr_vector<SNqAction>* acts = src ? src->SlotC(m_ActDragSlot.c_str()) : 0;
	if (!acts || m_ActDragIndex < 0 || m_ActDragIndex >= (int)acts->size()) return;
	const SNqAction& act = (*acts)[m_ActDragIndex];
	const ImVec2 m = ImGui::GetIO().MousePos;
	int node = -1, index = 0; xr_string slot; float line = 0.f;
	const bool ok = DropTarget(m, node, slot, index, &line);
	if (ok)
	{
		const SNodeGeom& g = m_Geom[node];
		const ImVec2 a = ToScreen(g.pos);
		dl->AddLine(ImVec2(a.x + 4.f * Scale(), line), ImVec2(a.x + (g.size.x - 4.f) * Scale(), line), C(T::AccentBright), _max(2.f, XUI::Pt(2.f)));
	}
	// red over anything that is not a place for it: releasing there cancels the move
	ImFont* body = Face(XUI::Font::Body);
	const float px = Round(XUI::FontSize(XUI::Font::Body) * XUI::Dpi());
	const xr_string title = NqLook::KindTitle(act.kind.c_str(), Behaviour());
	const float w = _min(TextW(body, px, title.c_str()), XUI::Pt(200.f)) + XUI::Pt(34.f), h = XUI::Pt(24.f);
	const ImVec2 pa(m.x + XUI::Pt(12.f), m.y + XUI::Pt(4.f)), pb(pa.x + w, pa.y + h);
	dl->AddRectFilled(pa, pb, C(T::Overlay, 0.95f), XUI::Pt(4.f));
	dl->AddRect(pa, pb, ok ? C(T::AccentBright) : C(T::Error), XUI::Pt(4.f), 0, _max(1.f, XUI::Pt(1.f)));
	const float icon = XUI::Pt(14.f);
	XUI::DrawIconRect(dl, NqLook::KindIcon(act.kind.c_str()), ImVec2(pa.x + XUI::Pt(6.f), pa.y + (h - icon) * 0.5f),
		ImVec2(pa.x + XUI::Pt(6.f) + icon, pa.y + (h + icon) * 0.5f), NqLook::FamilyColor(act.kind.c_str()));
	PaintFit(dl, body, px, ImVec2(pa.x + XUI::Pt(26.f), pa.y + (h - px) * 0.5f), C(T::Text), title.c_str(), w - XUI::Pt(32.f));
}

ImVec2 NqCanvas::MinimapToWorld(const ImVec2& screen) const
{
	if (m_MinimapScale <= 0.f) return ImVec2(m_Doc->view_cx, m_Doc->view_cy);
	const float x = _max(m_MinimapMin.x, _min(screen.x, m_MinimapMax.x));
	const float y = _max(m_MinimapMin.y, _min(screen.y, m_MinimapMax.y));
	return ImVec2(m_MinimapWorldMin.x + (x - m_MinimapMin.x) / m_MinimapScale,
		m_MinimapWorldMin.y + (y - m_MinimapMin.y) / m_MinimapScale);
}

// 180 x 120 pt in the bottom-right corner, and only for a graph big enough to get lost in:
// a handful of nodes is its own minimap, and the box would only hide some of them.
void NqCanvas::DrawMinimap(ImDrawList* dl)
{
	m_MinimapHovered = false;
	m_MinimapShown = false;
	const ImVec2 box_size(XUI::Pt(kMinimapW), XUI::Pt(kMinimapH));
	const float margin = XUI::Pt(8.f);
	if (!m_ShowMinimap || (int)m_Geom.size() < MinimapMinNodes()) return;
	// the canvas keeps at least twice the box in each direction
	if (m_Size.x < box_size.x * 2.f + margin * 2.f || m_Size.y < box_size.y * 2.f + margin * 2.f) return;

	// from the geometry this frame built: GraphBounds measures every node again
	float min_x = FLT_MAX, min_y = FLT_MAX, max_x = -FLT_MAX, max_y = -FLT_MAX;
	for (const SNodeGeom& g : m_Geom)
	{
		min_x = _min(min_x, g.pos.x); min_y = _min(min_y, g.pos.y);
		max_x = _max(max_x, g.pos.x + g.size.x); max_y = _max(max_y, g.pos.y + g.size.y);
	}
	m_MinimapShown = true;
	const ImVec2 box_max(m_Origin.x + m_Size.x - margin, m_Origin.y + m_Size.y - margin);
	const ImVec2 box_min(box_max.x - box_size.x, box_max.y - box_size.y);
	const float pad = XUI::Pt(6.f);
	const ImVec2 available(_max(box_size.x - pad * 2.f, 8.f), _max(box_size.y - pad * 2.f, 8.f));

	const float world_pad = NqLayout::kGrid * 2.f;
	m_MinimapWorldMin = ImVec2(min_x - world_pad, min_y - world_pad);
	m_MinimapWorldMax = ImVec2(max_x + world_pad, max_y + world_pad);
	const float world_w = _max(m_MinimapWorldMax.x - m_MinimapWorldMin.x, 1.f);
	const float world_h = _max(m_MinimapWorldMax.y - m_MinimapWorldMin.y, 1.f);
	m_MinimapScale = _min(available.x / world_w, available.y / world_h);
	const ImVec2 fitted(world_w * m_MinimapScale, world_h * m_MinimapScale);
	m_MinimapMin = ImVec2(box_min.x + (box_size.x - fitted.x) * 0.5f, box_min.y + (box_size.y - fitted.y) * 0.5f);
	m_MinimapMax = Add(m_MinimapMin, fitted);

	dl->AddRectFilled(box_min, box_max, C(T::Overlay, 0.92f), XUI::Pt(6.f));
	dl->AddRect(box_min, box_max, C(T::Border), XUI::Pt(6.f), 0, _max(1.f, XUI::Pt(1.f)));
	auto map_point = [&](const ImVec2& world)
	{
		return ImVec2(m_MinimapMin.x + (world.x - m_MinimapWorldMin.x) * m_MinimapScale,
			m_MinimapMin.y + (world.y - m_MinimapWorldMin.y) * m_MinimapScale);
	};

	for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
	{
		const SNqNode& node = m_Doc->quest.nodes[i];
		const SNodeGeom& geom = m_Geom[i];
		const ImVec2 from = map_point(Add(geom.pos, ImVec2(geom.size.x * 0.5f, geom.size.y * 0.5f)));
		for (u32 p = 0; p < node.out.size(); ++p)
			for (u32 t = 0; t < node.out[p].second.size(); ++t)
			{
				const int target = m_Doc->quest.NodeIndex(node.out[p].second[t].c_str());
				if (target < 0 || target >= (int)m_Geom.size()) continue;
				const SNodeGeom& target_geom = m_Geom[target];
				const ImVec2 to = map_point(Add(target_geom.pos, ImVec2(target_geom.size.x * 0.5f, target_geom.size.y * 0.5f)));
				dl->AddLine(from, to, C(T::TextMuted, 0.5f), 1.f);
			}
	}

	const float dot = _max(2.f, XUI::Pt(2.f));
	for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
	{
		const SNqNode& node = m_Doc->quest.nodes[i];
		const SNodeGeom& geom = m_Geom[i];
		const ImVec2 a = map_point(geom.pos);
		const ImVec2 b = map_point(Add(geom.pos, geom.size));
		const ImVec2 visible_b(_max(b.x, a.x + dot), _max(b.y, a.y + dot));
		dl->AddRectFilled(a, visible_b, NqLook::FamilyColor(node.kind.c_str(), 0.85f), 1.f);
		if (IsSelected(node.id.c_str()))			dl->AddRect(a, visible_b, C(T::Text), 1.f, 0, _max(1.f, XUI::Pt(1.5f)));
		else if (IsBookmarked(node.id.c_str()))	dl->AddRect(a, visible_b, C(T::AccentBright), 1.f, 0, _max(1.f, XUI::Pt(1.5f)));
	}

	const float s = Scale();
	const ImVec2 view_min(m_Doc->view_cx - m_Size.x / (2.f * s), m_Doc->view_cy - m_Size.y / (2.f * s));
	const ImVec2 view_max(m_Doc->view_cx + m_Size.x / (2.f * s), m_Doc->view_cy + m_Size.y / (2.f * s));
	ImVec2 va = map_point(view_min), vb = map_point(view_max);
	va.x = _max(m_MinimapMin.x, _min(va.x, m_MinimapMax.x));
	va.y = _max(m_MinimapMin.y, _min(va.y, m_MinimapMax.y));
	vb.x = _max(m_MinimapMin.x, _min(vb.x, m_MinimapMax.x));
	vb.y = _max(m_MinimapMin.y, _min(vb.y, m_MinimapMax.y));
	dl->AddRect(va, vb, C(T::AccentBright), 0.f, 0, _max(1.f, XUI::Pt(1.5f)));

	m_MinimapHovered = m_Hovered && InRect(ImGui::GetMousePos(), box_min, box_max);
	if (m_MinimapHovered && !ImGui::IsMouseDown(ImGuiMouseButton_Left))
		m_TipNext = "Minimap: click or drag to move the view";
}

// Where the graph goes on past the view, when no minimap says it: a pill at the middle of each edge with
// the nodes wholly beyond it. A graph larger than the view looked finished at its edges otherwise - a small
// one draws no minimap. The counts are kept either way, for quest_view `beyond`.
void NqCanvas::DrawBeyond(ImDrawList* dl)
{
	m_BeyondHot = -1;
	for (int s = 0; s < 4; ++s) { m_Beyond[s] = 0; m_BeyondNear[s] = -1; }
	float x0, y0, x1, y1;
	if (m_Geom.empty() || !ViewportBounds(x0, y0, x1, y1)) return;
	// the nearest node past each edge: the one the least pan brings in
	float nearest[4] = { -FLT_MAX, FLT_MAX, -FLT_MAX, FLT_MAX };
	auto count = [&](int side, int node, bool beyond, float edge, bool larger)
	{
		if (!beyond) return;
		++m_Beyond[side];
		if (larger ? edge > nearest[side] : edge < nearest[side]) { nearest[side] = edge; m_BeyondNear[side] = node; }
	};
	for (u32 i = 0; i < m_Geom.size(); ++i)
	{
		const SNodeGeom& g = m_Geom[i];
		const float top = g.pos.y, bottom = g.pos.y + g.size.y, left = g.pos.x, right = g.pos.x + g.size.x;
		count(kAbove, (int)i, bottom <= y0, bottom, true);
		count(kBelow, (int)i, top >= y1, top, false);
		count(kLeft, (int)i, right <= x0, right, true);
		count(kRight, (int)i, left >= x1, left, false);
	}
	if (m_MinimapShown) return;

	static const XUI::Icon kIcons[4] = { XUI::Icon::ChevronUp, XUI::Icon::ChevronDown, XUI::Icon::ChevronLeft, XUI::Icon::ChevronRight };
	static LPCSTR const kWhere[4] = { "above", "below", "left of", "right of" };
	ImFont* face = Face(XUI::Font::Small);
	const float px = Round(XUI::FontSize(XUI::Font::Small) * XUI::Dpi());
	const float icon = XUI::Pt(12.f);
	const ImVec2 pad(XUI::Pt(8.f), XUI::Pt(4.f));
	const float inset = XUI::Pt(8.f);
	const ImVec2 mouse = ImGui::GetMousePos();
	for (int s = 0; s < 4; ++s)
	{
		if (!m_Beyond[s]) continue;
		const xr_string text = NqUtil::Format("%d more", m_Beyond[s]);
		const float h = Round(_max(px, icon) + pad.y * 2.f);
		const float w = Round(pad.x + icon + XUI::Pt(4.f) + TextW(face, px, text.c_str()) + pad.x);
		ImVec2 a;
		if (s == kAbove)		a = ImVec2(m_Origin.x + (m_Size.x - w) * 0.5f, m_Origin.y + inset);
		else if (s == kBelow)	a = ImVec2(m_Origin.x + (m_Size.x - w) * 0.5f, m_Origin.y + m_Size.y - inset - h);
		else if (s == kLeft)	a = ImVec2(m_Origin.x + inset, m_Origin.y + (m_Size.y - h) * 0.5f);
		else					a = ImVec2(m_Origin.x + m_Size.x - inset - w, m_Origin.y + (m_Size.y - h) * 0.5f);
		a = ImVec2(Round(a.x), Round(a.y));
		const ImVec2 b(a.x + w, a.y + h);
		const bool hot = m_Hovered && InRect(mouse, a, b);
		if (hot) m_BeyondHot = s;
		dl->AddRectFilled(a, b, C(T::Overlay, 0.92f), XUI::Pt(6.f));
		if (hot) dl->AddRectFilled(a, b, C(T::Hover), XUI::Pt(6.f));
		dl->AddRect(a, b, C(T::BorderSubtle), XUI::Pt(6.f));
		XUI::DrawIconRect(dl, kIcons[s], ImVec2(a.x + pad.x, a.y + (h - icon) * 0.5f), ImVec2(a.x + pad.x + icon, a.y + (h + icon) * 0.5f), C(T::TextSecondary));
		dl->AddText(face, px, ImVec2(Round(a.x + pad.x + icon + XUI::Pt(4.f)), Round(a.y + (h - px) * 0.5f)), C(T::TextSecondary), text.c_str());
		if (hot)
			m_TipNext = NqUtil::Format("%d %s %s the view. Click to bring the nearest one in",
				m_Beyond[s], m_Beyond[s] == 1 ? "node" : "nodes", kWhere[s]);
	}
}

void NqCanvas::RevealBeyond(int side)
{
	const int i = side >= 0 && side < 4 ? m_BeyondNear[side] : -1;
	float x0, y0, x1, y1;
	if (i < 0 || i >= (int)m_Geom.size() || !ViewportBounds(x0, y0, x1, y1)) return;
	const SNodeGeom& g = m_Geom[i];
	const float m = kFrameMargin;
	float dx = 0.f, dy = 0.f;
	// the whole node with the frame margin; a node larger than the view shows its top or left end
	if (side == kAbove)			dy = g.pos.y - m - y0;
	else if (side == kBelow)	dy = _min(g.pos.y + g.size.y + m - y1, g.pos.y - m - y0);
	else if (side == kLeft)		dx = g.pos.x - m - x0;
	else						dx = _min(g.pos.x + g.size.x + m - x1, g.pos.x - m - x0);
	RememberView();
	CancelFraming();
	m_Doc->view_cx = NqLayout::Sane(m_Doc->view_cx + dx);
	m_Doc->view_cy = NqLayout::Sane(m_Doc->view_cy + dy);
}

// what the last gesture did, or why it did nothing: a pill at the top of the canvas for a
// moment, never a modal - and the log keeps it for a run without a screen
void NqCanvas::DrawStatus(ImDrawList* dl)
{
	if (m_Status.empty()) return;
	if (ImGui::GetTime() - m_StatusAt > 2.5) { m_Status.clear(); return; }
	ImFont* body = Face(XUI::Font::Body);
	const float px = Round(XUI::FontSize(XUI::Font::Body) * XUI::Dpi());
	xr_string shown;
	if (!XUI::EllipsizeFace(body, px, m_Status.c_str(), m_Size.x - XUI::Pt(40.f), XUI::Clip::End, shown)) shown = m_Status;
	const ImVec2 pad(XUI::Pt(10.f), XUI::Pt(4.f));
	const ImVec2 a(m_Origin.x + XUI::Pt(8.f), m_Origin.y + XUI::Pt(8.f));
	const ImVec2 b(a.x + TextW(body, px, shown.c_str()) + pad.x * 2.f, a.y + px + pad.y * 2.f);
	dl->AddRectFilled(a, b, C(T::Overlay, 0.92f), XUI::Pt(6.f));
	dl->AddRect(a, b, C(T::BorderSubtle), XUI::Pt(6.f));
	dl->AddText(body, px, ImVec2(Round(a.x + pad.x), Round(a.y + pad.y)), C(T::TextSecondary), shown.c_str());
}

//------------------------------------------------------------------------------
// input
//------------------------------------------------------------------------------
void NqCanvas::HandleInput()
{
	ImGuiIO& io = ImGui::GetIO();
	ImVec2 mouse = ImGui::GetMousePos();
	// the minimap and the edge pills are the canvas's own controls: a click there is theirs
	bool hovered = m_Hovered && !m_MinimapHovered && m_BeyondHot < 0;

	if (m_BeyondHot >= 0 && ImGui::IsMouseClicked(ImGuiMouseButton_Left)) RevealBeyond(m_BeyondHot);
	if (m_MinimapHovered && ImGui::IsMouseClicked(ImGuiMouseButton_Left))
	{
		RememberView();
		m_MinimapPanning = true;
	}
	if (m_MinimapPanning && ImGui::IsMouseDown(ImGuiMouseButton_Left))
	{
		const ImVec2 world = MinimapToWorld(mouse);
		CancelFraming();
		m_Doc->view_cx = NqLayout::Sane(world.x);
		m_Doc->view_cy = NqLayout::Sane(world.y);
	}
	if (m_MinimapPanning && !ImGui::IsMouseDown(ImGuiMouseButton_Left)) m_MinimapPanning = false;

	// wheel: zoom around the cursor
	if (hovered && io.MouseWheel != 0.f)
	{
		const double now = ImGui::GetTime();
		const bool coalesce = now - m_LastWheelHistory < 0.35;
		const bool replaying = m_ReplayingHistory;
		const int zoom = m_Doc->zoom_idx;
		if (coalesce) m_ReplayingHistory = true;
		m_WheelZooming = true;
		ZoomBy(io.MouseWheel > 0.f ? 1 : -1, &mouse);
		m_WheelZooming = false;
		m_ReplayingHistory = replaying;
		if (m_Doc->zoom_idx != zoom) m_LastWheelHistory = now;
	}

	// right button: pan while dragging, context menu on a click
	if (hovered && ImGui::IsMouseClicked(ImGuiMouseButton_Right)) { m_Panning = true; m_RmbMoved = false; m_RmbDown = mouse; }
	if (m_Panning && ImGui::IsMouseDown(ImGuiMouseButton_Right))
	{
		ImVec2 d = io.MouseDelta;
		if (d.x != 0.f || d.y != 0.f)
		{
			if (!m_RmbMoved && fabsf(mouse.x - m_RmbDown.x) + fabsf(mouse.y - m_RmbDown.y) > XUI::Pt(3.f))
			{
				RememberView();
				m_RmbMoved = true;
			}
			if (m_RmbMoved) { const float s = Scale(); m_Doc->view_cx -= d.x / s; m_Doc->view_cy -= d.y / s; }
		}
	}
	if (m_Panning && ImGui::IsMouseReleased(ImGuiMouseButton_Right))
	{
		m_Panning = false;
		if (!m_RmbMoved && hovered) OpenContextMenus();
	}
	// middle button pans too
	if (hovered && ImGui::IsMouseDown(ImGuiMouseButton_Middle))
	{
		if (!m_MiddlePanning && (io.MouseDelta.x != 0.f || io.MouseDelta.y != 0.f)) { RememberView(); m_MiddlePanning = true; }
		const float s = Scale();
		m_Doc->view_cx -= io.MouseDelta.x / s; m_Doc->view_cy -= io.MouseDelta.y / s;
	}
	if (!ImGui::IsMouseDown(ImGuiMouseButton_Middle)) m_MiddlePanning = false;

	// left button
	if (hovered && ImGui::IsMouseClicked(ImGuiMouseButton_Left))
	{
		int node = HitNode(mouse);
		int pin = HitPin(node, mouse);
		if (node < 0)
		{
			// pins stick out below the box: the nearest one over every node wins
			int pnode = -1, ppin = -1;
			if (HitAnyPin(mouse, pnode, ppin)) { node = pnode; pin = ppin; }
		}
		SActionHit act;
		if (pin >= 0)
		{
			m_Linking = true;
			m_LinkFrom = m_Doc->quest.nodes[node].id;
			m_LinkPin = m_Geom[node].pins[pin];
		}
		else if (node >= 0 && HitAction(node, mouse, act))
		{
			const SNqNode& n = m_Doc->quest.nodes[node];
			SelectOnly(n.id.c_str());
			if (act.more)
			{
				// the first action the node has no room for opens in the Details panel
				xr_string slot; int idx = 0;
				if (ActionAtRow(n, NqLayout::kMaxActions, slot, idx)) m_Doc->sel_slot = slot + ":" + NqUtil::Format("%d", idx);
				m_WantFocusAction = true;
			}
			else
			{
				m_Doc->sel_slot = act.slot + ":" + NqUtil::Format("%d", act.index);
				m_ActDragNode = n.id; m_ActDragSlot = act.slot; m_ActDragIndex = act.index;
				if (ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left)) m_WantFocusAction = true;
			}
		}
		else if (node >= 0)
		{
			const xr_string& id = m_Doc->quest.nodes[node].id;
			if (io.KeyCtrl) ToggleSelected(id.c_str());
			else if (!IsSelected(id.c_str())) SelectOnly(id.c_str());
			else m_Doc->sel_slot.clear();
			// start dragging the whole selection
			m_Dragging = true; m_DragMoved = false; m_DragStartMouse = mouse;
			m_DragStart.clear();
			for (u32 i = 0; i < m_Doc->selection.size(); ++i)
			{
				const SNqNode* n = m_Doc->quest.FindNode(m_Doc->selection[i].c_str());
				if (n) m_DragStart.push_back(std::make_pair(n->id, ImVec2(n->pos.x, n->pos.y)));
			}
		}
		else
		{
			// the lit one, not a second hit test: what Alt+LMB removes has to be
			// what the halo pointed at, including when a node body swallowed the wire
			if (m_HotLink >= 0 && io.KeyAlt) DeleteHotLink();
			else
			{
				// EndMarquee only ever adds; ctrl is the difference, and it is
				// expressed by keeping (or dropping) the current selection here
				m_Marquee = true; m_MarqueeFrom = mouse;
				if (!io.KeyCtrl) { m_Doc->selection.clear(); m_Doc->sel_slot.clear(); }
			}
		}
	}

	// dragging nodes
	if (m_Dragging && ImGui::IsMouseDown(ImGuiMouseButton_Left))
	{
		ImVec2 d = Sub(mouse, m_DragStartMouse);
		if (fabsf(d.x) + fabsf(d.y) > XUI::Pt(2.f)) m_DragMoved = true;
		if (m_DragMoved)
		{
			const float s = Scale();
			for (u32 i = 0; i < m_DragStart.size(); ++i)
			{
				SNqNode* n = m_Doc->quest.FindNode(m_DragStart[i].first.c_str());
				if (n) { n->pos.x = m_DragStart[i].second.x + d.x / s; n->pos.y = m_DragStart[i].second.y + d.y / s; n->has_pos = true; }
			}
		}
	}
	if (m_Dragging && ImGui::IsMouseReleased(ImGuiMouseButton_Left)) EndDrag();

	// action drag: reorder inside a slot, move between on_enter and on_exit, or hand the
	// action to another node - one drop path for all three
	if (m_ActDragIndex >= 0 && ImGui::IsMouseDragging(ImGuiMouseButton_Left)) m_ActDragging = true;
	if (m_ActDragIndex >= 0 && ImGui::IsMouseReleased(ImGuiMouseButton_Left) && !m_ActDragging)
	{
		m_ActDragIndex = -1; m_ActDragNode.clear();		// a plain click, already selected on press
	}
	else if (m_ActDragIndex >= 0 && ImGui::IsMouseReleased(ImGuiMouseButton_Left))
	{
		int node = -1, at = 0; xr_string slot;
		if (DropTarget(mouse, node, slot, at))
			MoveAction(m_ActDragNode, m_ActDragSlot, m_ActDragIndex, m_Doc->quest.nodes[node].id, slot, at);
		m_ActDragIndex = -1; m_ActDragNode.clear(); m_ActDragging = false;
	}

	if (m_Linking && ImGui::IsMouseReleased(ImGuiMouseButton_Left)) EndLink();
	if (m_Marquee && ImGui::IsMouseReleased(ImGuiMouseButton_Left)) EndMarquee();

	// what the cursor is on, said on hover: an action in full, and a node whose text is too
	// small to read
	if (hovered && !m_Dragging && !m_Linking && !m_Marquee && !m_ActDragging && m_TipNext.empty())
	{
		const int node = HitNode(mouse);
		if (m_HotAction.node >= 0 && !m_HotAction.more)
		{
			const SNqNode& n = m_Doc->quest.nodes[m_HotAction.node];
			const xr_vector<SNqAction>* acts = n.SlotC(m_HotAction.slot.c_str());
			if (acts && m_HotAction.index < (int)acts->size())
				m_TipNext = ActionText((*acts)[m_HotAction.index], m_HotAction.slot.c_str(), m_HotAction.index);
		}
		else if (m_HotAction.node >= 0)
			m_TipNext = "Show the rest of the actions in the Details panel";
		else if (node >= 0 && HitPin(node, mouse) < 0 && Lod() < 2)
		{
			const SNqNode& n = m_Doc->quest.nodes[node];
			m_TipNext = NodeTitle(n) + "\n" + n.id;
			const xr_string sum = NodeSummary(n);
			if (!sum.empty()) m_TipNext += "\n" + sum;
		}
	}
}

void NqCanvas::EndDrag()
{
	m_Dragging = false;
	if (m_DragMoved)
	{
		// commit once: restore the start positions, snapshot, then apply snapped
		xr_vector<std::pair<xr_string, ImVec2> > moved;
		for (u32 i = 0; i < m_DragStart.size(); ++i)
		{
			SNqNode* n = m_Doc->quest.FindNode(m_DragStart[i].first.c_str());
			if (!n) continue;
			moved.push_back(std::make_pair(n->id, ImVec2(Snap(n->pos.x), Snap(n->pos.y))));
			n->pos.x = m_DragStart[i].second.x; n->pos.y = m_DragStart[i].second.y;
		}
		m_Doc->Edit([&](SNqQuest& q)
		{
			for (u32 i = 0; i < moved.size(); ++i)
			{
				SNqNode* n = q.FindNode(moved[i].first.c_str());
				if (n) { n->pos.x = moved[i].second.x; n->pos.y = moved[i].second.y; n->has_pos = true; }
			}
		});
	}
	m_DragStart.clear();
}

// Moves one action: within its slot, to the other slot or to another node - one undo step,
// and the Details panel follows it to where it landed.
void NqCanvas::MoveAction(const xr_string& from_node, const xr_string& from_slot, int from,
						  const xr_string& to_node, const xr_string& to_slot, int at)
{
	// dropping an action on its own place is not a move
	if (from_node == to_node && from_slot == to_slot && (at == from || at == from + 1)) return;
	int landed = -1;
	m_Doc->Edit([&](SNqQuest& q)
	{
		SNqNode* src = q.FindNode(from_node.c_str());
		SNqNode* dst = q.FindNode(to_node.c_str());
		if (!src || !dst) return;
		xr_vector<SNqAction>& sv = src->Slot(from_slot.c_str());
		if (from < 0 || from >= (int)sv.size()) return;
		SNqAction a = sv[from];
		sv.erase(sv.begin() + from);
		xr_vector<SNqAction>& dv = dst->Slot(to_slot.c_str());
		int pos = at;
		// the same list lost the action above the drop point
		if (&dv == &sv && pos > from) --pos;
		pos = _max(0, _min(pos, (int)dv.size()));
		dv.insert(dv.begin() + pos, a);
		landed = pos;
	});
	if (landed < 0) return;
	m_Doc->selection.clear();
	m_Doc->selection.push_back(to_node);
	m_Doc->sel_slot = to_slot + ":" + NqUtil::Format("%d", landed);
}

// the fading line at the top of the canvas, plus the log so a headless run
// (-nodlg) records what the gesture did: a note, never a warning the status bar counts
void NqCanvas::SetStatus(LPCSTR text)
{
	m_Status = text ? text : "";
	m_StatusAt = ImGui::GetTime();
	if (!m_Status.empty()) Msg("* [nq] %s", m_Status.c_str());
}

bool NqCanvas::CanLink(LPCSTR from, LPCSTR to, xr_string& why) const
{
	why.clear();
	const SNqNode* t = m_Doc->quest.FindNode(to);
	if (!t)
		why = NqUtil::Format("'%s' does not exist", to);
	else if (0 == strcmp(from, to))
		why = "A node cannot link to itself";
	else if (NqText::IsTrigger(t->kind.c_str()))
		why = NqUtil::Format("'%s' is a trigger - triggers have no input", t->id.c_str());
	return why.empty();
}

void NqCanvas::EndLink()
{
	m_Linking = false;
	m_Status.clear();
	ImVec2 mouse = ImGui::GetMousePos();
	int node = HitNode(mouse);
	xr_string why;
	if (node >= 0)
	{
		// a refused drop used to vanish without a word; say why instead
		if (!CanLink(m_LinkFrom.c_str(), m_Doc->quest.nodes[node].id.c_str(), why))
		{
			m_Status = why;
			m_StatusAt = ImGui::GetTime();
			Msg("* [nq] link: %s", m_Status.c_str());
			return;
		}
	}
	else
	{
		// a drop that lands beside a box still means that box: its input pin sits
		// on the edge and hitting it exactly is what was too hard. The source node
		// and a neighbour that could not take the edge anyway are not guesses worth
		// making - for them the drop stays "empty space" and offers a new node.
		node = HitNodeNear(mouse, m_Doc->quest.NodeIndex(m_LinkFrom.c_str()));
		if (node >= 0 && !CanLink(m_LinkFrom.c_str(), m_Doc->quest.nodes[node].id.c_str(), why)) node = -1;
	}
	if (node >= 0)
	{
		xr_string err;
		if (!m_Doc->Connect(m_LinkFrom.c_str(), m_LinkPin.c_str(), m_Doc->quest.nodes[node].id.c_str(), err) && !err.empty())
			Msg("! [nq] connect: %s", err.c_str());
	}
	else if (m_Hovered)
	{
		// dropped on empty space: create a node there and connect it
		m_PendingLink = true;
		m_MenuWorld = ToWorld(mouse);
		m_Filter[0] = 0;
		ImGui::OpenPopup("nq_add_node");
	}
}

void NqCanvas::EndMarquee()
{
	m_Marquee = false;
	ImVec2 m = ImGui::GetMousePos();
	ImVec2 a(_min(m.x, m_MarqueeFrom.x), _min(m.y, m_MarqueeFrom.y)), b(_max(m.x, m_MarqueeFrom.x), _max(m.y, m_MarqueeFrom.y));
	if (b.x - a.x < 2.f && b.y - a.y < 2.f) return;
	for (u32 i = 0; i < m_Geom.size(); ++i)
	{
		ImVec2 na = ToScreen(m_Geom[i].pos), nb = ToScreen(Add(m_Geom[i].pos, m_Geom[i].size));
		bool inter = !(nb.x < a.x || na.x > b.x || nb.y < a.y || na.y > b.y);
		if (inter && !IsSelected(m_Doc->quest.nodes[i].id.c_str())) m_Doc->selection.push_back(m_Doc->quest.nodes[i].id);
	}
}

// Q: the selected node reaches for the nearest node it can still be wired to,
// in either direction (its free output -> another input, or another free output
// -> its own input). Only pins the catalog declares are candidates and the drop
// goes through CanLink, so the edge can never be one the validator refuses on
// the spot. Distances are world units, so the answer does not depend on the zoom.
void NqCanvas::ConnectNearest()
{
	if (m_Doc->selection.size() != 1) { SetStatus("Q wires one node - select exactly one"); return; }
	int si = -1;
	const SNodeGeom* sg = GeomOf(m_Doc->selection[0].c_str(), &si);
	if (!sg) return;
	const SNqQuest& q = m_Doc->quest;
	const SNqNode& sn = q.nodes[si];

	float best = FLT_MAX;
	xr_string from, pin, to;
	// out of the selection into somebody's input
	for (int p = 0; p < sg->declared; ++p)
	{
		if (!PinFree(sn, sg->pins[p].c_str())) continue;
		ImVec2 a = OutputPin(*sg, p);
		for (u32 i = 0; i < q.nodes.size(); ++i)
		{
			if ((int)i == si || !CanTakeInput(q, q.nodes[i])) continue;
			float d = Dist2(a, InputPin(m_Geom[i]));
			if (d < best) { best = d; from = sn.id; pin = sg->pins[p]; to = q.nodes[i].id; }
		}
	}
	// somebody's free output into the selection
	if (CanTakeInput(q, sn))
	{
		ImVec2 b = InputPin(*sg);
		for (u32 i = 0; i < q.nodes.size(); ++i)
		{
			if ((int)i == si) continue;
			const SNqNode& o = q.nodes[i];
			for (int p = 0; p < m_Geom[i].declared; ++p)
			{
				if (!PinFree(o, m_Geom[i].pins[p].c_str())) continue;
				float d = Dist2(OutputPin(m_Geom[i], p), b);
				if (d < best) { best = d; from = o.id; pin = m_Geom[i].pins[p]; to = sn.id; }
			}
		}
	}
	if (from.empty()) { SetStatus("Q found no free pin to connect"); return; }
	xr_string why;
	if (!CanLink(from.c_str(), to.c_str(), why)) { SetStatus(why.c_str()); return; }
	xr_string err;
	if (!m_Doc->Connect(from.c_str(), pin.c_str(), to.c_str(), err))
		SetStatus(err.empty() ? "Connect refused" : err.c_str());
	else
		SetStatus(NqUtil::Format("Connected %s.%s -> %s", from.c_str(), pin.c_str(), to.c_str()).c_str());
}

// Cuts the wire the cursor is on. The single path behind Alt+LMB and D, so the two
// can never disagree about which wire that is.
void NqCanvas::DeleteHotLink()
{
	if (m_HotLink < 0 || m_HotLink >= (int)m_Links.size()) return;
	const SLinkGeom& l = m_Links[m_HotLink];
	const xr_string from = m_Doc->quest.nodes[l.from].id;
	const xr_string to = m_Doc->quest.nodes[l.to].id;
	const xr_string pin = l.pin;
	xr_string err;
	if (m_Doc->Disconnect(from.c_str(), pin.c_str(), to.c_str(), err))
		SetStatus(NqUtil::Format("Removed %s.%s -> %s", from.c_str(), pin.c_str(), to.c_str()).c_str());
	else if (!err.empty())
		SetStatus(err.c_str());
	// the geometry it indexed is about to be rebuilt
	m_HotLink = -1;
}

// Alt+D: the node stays where it is and the chain closes over it - A->B->C with B
// selected becomes A->C. Named after BlueprintAssist's "Disconnect execution on
// selected node", which is Alt+D there too and also leaves the node in place.
//
// One difference on purpose: BA links every leaf input to every leaf output, which
// suits UE where an exec output holds a single link. An NQ output pin carries a LIST
// of targets, so crossing 2 inputs with 2 outputs would invent four edges where the
// author asked to close two chains. Pairs go in order instead, and whatever has no
// counterpart simply goes - a node wired on one side only just loses that side.
void NqCanvas::BypassSelected()
{
	if (m_Doc->selection.empty()) { SetStatus("Select a node first"); return; }

	struct SBridge { xr_string from, pin, to; };
	xr_vector<SBridge> bridges;			// edges to create
	xr_vector<SBridge> cuts;			// edges to remove
	int refused = 0;

	for (u32 s = 0; s < m_Doc->selection.size(); ++s)
	{
		const xr_string& mid = m_Doc->selection[s];
		const SNqNode* n = m_Doc->quest.FindNode(mid.c_str());
		if (!n) continue;

		// incoming, in graph order; outgoing, in the order the file declares the pins
		xr_vector<SBridge> in, out;
		for (u32 i = 0; i < m_Doc->quest.nodes.size(); ++i)
		{
			const SNqNode& other = m_Doc->quest.nodes[i];
			if (other.id == mid) continue;
			for (u32 p = 0; p < other.out.size(); ++p)
				for (u32 t = 0; t < other.out[p].second.size(); ++t)
					if (other.out[p].second[t] == mid)
					{
						SBridge e; e.from = other.id; e.pin = other.out[p].first; e.to = mid;
						in.push_back(e);
					}
		}
		for (u32 p = 0; p < n->out.size(); ++p)
			for (u32 t = 0; t < n->out[p].second.size(); ++t)
			{
				SBridge e; e.from = mid; e.pin = n->out[p].first; e.to = n->out[p].second[t];
				out.push_back(e);
			}
		if (in.empty() && out.empty()) continue;

		cuts.insert(cuts.end(), in.begin(), in.end());
		cuts.insert(cuts.end(), out.begin(), out.end());

		const u32 pairs = _min(in.size(), out.size());
		for (u32 k = 0; k < pairs; ++k)
		{
			// the same gate a hand-drawn edge passes (E007): a bridge into a trigger
			// is not an edge the author could have made either
			xr_string why;
			if (!CanLink(in[k].from.c_str(), out[k].to.c_str(), why)) { ++refused; continue; }
			SBridge e; e.from = in[k].from; e.pin = in[k].pin; e.to = out[k].to;
			bridges.push_back(e);
		}
	}

	if (cuts.empty()) { SetStatus("Nothing was connected to the selection"); return; }

	// one undo entry: Connect/Disconnect each push their own, and a rewiring the
	// author asked for once should not take eight presses of Ctrl+Z to take back
	m_Doc->Edit([&](SNqQuest& q)
	{
		for (u32 i = 0; i < cuts.size(); ++i)
			if (SNqNode* f = q.FindNode(cuts[i].from.c_str())) f->Disconnect(cuts[i].pin.c_str(), cuts[i].to.c_str());
		for (u32 i = 0; i < bridges.size(); ++i)
			if (SNqNode* f = q.FindNode(bridges[i].from.c_str())) f->Connect(bridges[i].pin.c_str(), bridges[i].to.c_str());
	});

	if (refused)
		SetStatus(NqUtil::Format("Bypassed: %d link(s) bridged, %d could not be", (int)bridges.size(), refused).c_str());
	else if (bridges.empty())
		SetStatus("Bypassed: the selection was wired on one side only, those links are gone");
	else
		SetStatus(NqUtil::Format("Bypassed: %d link(s) bridged", (int)bridges.size()).c_str());
}

void NqCanvas::HandleKeys()
{
	if (!ImGui::IsWindowFocused(ImGuiFocusedFlags_RootAndChildWindows)) return;
	// Del / Ctrl+A / F2 belong to the canvas only while the canvas is the part
	// of the tab being worked in - the inspector next to it has its own lists
	if (!m_Hovered && !ImGui::IsWindowFocused(ImGuiFocusedFlags_ChildWindows)) return;
	if (ImGui::GetIO().WantTextInput) return;		// a text field owns the keyboard
	// A menu is a mode. Its own items answer the keyboard while it is up, and a
	// canvas shortcut firing underneath it acts on something the author cannot see.
	if (MenuOpen()) return;
	// the scene's own bindings never reach a document tab (XUI::Shell::SceneKeysAllowed)
	ImGuiIO& io = ImGui::GetIO();
	// ImGui::IsKeyPressed defaults to repeat=true. These are one-shot commands, and
	// on auto-repeat they run again every repeat tick - which is how holding Ctrl
	// after a paste spawned nodes without end: the key-up of V never reached ImGui
	// (the editor's global shortcut layer had eaten it), so V read as still down
	// and every repeat while Ctrl was held pasted once more.
	const bool once = false;
	if (ImGui::IsKeyPressed(ImGuiKey_Delete, once))					DeleteSelection();
	if (ImGui::IsKeyPressed(ImGuiKey_Home, once))					FrameAll();
	if (ImGui::IsKeyPressed(ImGuiKey_F, once) && !io.KeyCtrl)		FrameSelection();
	if (ImGui::IsKeyPressed(ImGuiKey_F2, once))						m_WantRename = true;
	// IsAnyItemActive on top of the WantTextInput gate above: a popup filter box
	// (and the canvas button held down mid-drag) keeps Q to itself
	if (ImGui::IsKeyPressed(ImGuiKey_Q, once) && !io.KeyCtrl && !io.KeyAlt && !ImGui::IsAnyItemActive()) ConnectNearest();
	// D cuts the lit wire. No click, so nothing has to be aimed at twice: what the
	// halo says is under the cursor is exactly what goes.
	if (ImGui::IsKeyPressed(ImGuiKey_D, once) && !io.KeyCtrl && !ImGui::IsAnyItemActive())
	{
		if (io.KeyAlt)	BypassSelected();
		else			DeleteHotLink();
	}
	if (io.KeyCtrl && ImGui::IsKeyPressed(ImGuiKey_A, once))		SelectAll();
	if (io.KeyCtrl && ImGui::IsKeyPressed(ImGuiKey_C, once))		CopySelection();
	if (io.KeyCtrl && ImGui::IsKeyPressed(ImGuiKey_V, once))		PasteClipboard();
	if (io.KeyCtrl && ImGui::IsKeyPressed(ImGuiKey_D, once))		DuplicateSelection();
	if (io.KeyCtrl && ImGui::IsKeyPressed(ImGuiKey_B, once))		ToggleSelectedBookmarks();
	if (io.KeyAlt && ImGui::IsKeyPressed(ImGuiKey_LeftArrow, once))	HistoryBack();
	if (io.KeyAlt && ImGui::IsKeyPressed(ImGuiKey_RightArrow, once))	HistoryForward();
	if (io.KeyCtrl && !io.KeyShift && ImGui::IsKeyPressed(ImGuiKey_Z, once)) m_Doc->Undo();
	if (io.KeyCtrl && (ImGui::IsKeyPressed(ImGuiKey_Y, once) || (io.KeyShift && ImGui::IsKeyPressed(ImGuiKey_Z, once)))) m_Doc->Redo();
	if (ImGui::IsKeyPressed(ImGuiKey_Escape, once))
	{
		m_Linking = false; m_Marquee = false;
		// drops the action back where it came from: the move only happens on release
		m_ActDragIndex = -1; m_ActDragNode.clear(); m_ActDragging = false;
	}
}

//------------------------------------------------------------------------------
// context menus
//------------------------------------------------------------------------------
void NqCanvas::OpenContextMenus()
{
	ImVec2 mouse = ImGui::GetMousePos();
	int node = HitNode(mouse);
	m_MenuWorld = ToWorld(mouse);
	if (node >= 0)
	{
		m_CtxNode = m_Doc->quest.nodes[node].id;
		if (!IsSelected(m_CtxNode.c_str())) SelectOnly(m_CtxNode.c_str());
		SActionHit act;
		if (HitAction(node, mouse, act) && !act.more)
		{
			m_ActDragIndex = -1;
			m_Doc->sel_slot = act.slot + ":" + NqUtil::Format("%d", act.index);
			ImGui::OpenPopup("nq_action_ctx");
		}
		else
			ImGui::OpenPopup("nq_node_ctx");
		return;
	}
	int link = HitLink(mouse);
	if (link >= 0)
	{
		const SLinkGeom& l = m_Links[link];
		m_CtxLinkFrom = m_Doc->quest.nodes[l.from].id; m_CtxLinkPin = l.pin; m_CtxLinkTo = m_Doc->quest.nodes[l.to].id;
		ImGui::OpenPopup("nq_link_ctx");
		return;
	}
	m_PendingLink = false;
	m_Filter[0] = 0;
	ImGui::OpenPopup("nq_add_node");
}

void NqCanvas::RequestAddAction(LPCSTR node, LPCSTR slot, const ImVec2& screen)
{
	if (!node || !m_Doc->quest.FindNode(node)) return;
	m_CtxNode = node;
	m_AddSlot = (slot && 0 == strcmp(slot, "exit")) ? "exit" : "enter";
	m_OpenAddAction = true;
	m_AddActionAt = true;
	m_AddActionPos = screen;
}

void NqCanvas::CreateNode(const NqCatalog::SKind& k, const ImVec2& world, bool connect_pending)
{
	SNqNode n;
	// short id from the kind's last segment
	LPCSTR dot = strrchr(k.id.c_str(), '.');
	xr_string base = dot ? xr_string(dot + 1) : k.id;
	if (base == "start" || base == "when") base = "trigger";
	n.id = m_Doc->FreeNodeId(base.c_str());
	n.kind = k.id;
	n.params = SNqValue::Table();
	n.pos.set(Snap(world.x), Snap(world.y));
	n.has_pos = true;
	if (n.id.empty() || m_Doc->quest.FindNode(n.id.c_str())) { Msg("! [nq] add node: node '%s' already exists", n.id.c_str()); return; }
	// the node and the edge that asked for it are one gesture, so they are one
	// undo step: AddNode + Connect would take two Ctrl+Z to undo
	const xr_string from = m_LinkFrom, pin = m_LinkPin;
	const bool wire = connect_pending && !from.empty();
	m_Doc->Edit([&](SNqQuest& q)
	{
		q.nodes.push_back(n);
		if (wire)
			if (SNqNode* f = q.FindNode(from.c_str())) f->Connect(pin.c_str(), n.id.c_str());
	});
	SelectOnly(n.id.c_str());
	m_WantRename = false;
}

// the body both search popups share: a full-width filter box over a scrolling
// list of matches, driven from the keyboard alone
const NqCatalog::SKind* NqCanvas::PickKind(LPCSTR hint, u32 use_mask)
{
	const bool appearing = ImGui::IsWindowAppearing();
	if (appearing) { m_FilterSel = 0; ImGui::SetKeyboardFocusHere(); }
	ImGui::SetNextItemWidth(-1.f);
	bool refilter = appearing;
	if (XUI::SearchField("##nq_filter", m_Filter, sizeof(m_Filter), hint, 12.f)) { m_FilterSel = 0; refilter = true; }
	ImGui::Separator();

	const bool behaviour = Behaviour();
	xr_vector<const NqCatalog::SKind*> kinds, shown;
	NqCatalog::KindsFor(use_mask, kinds);
	xr_vector<xr_string> titles;
	xr_string last_group;
	int groups = 0;
	// the author types what the row shows, in any case and either alphabet
	const xr_string query = NqUtil::Trim(m_Filter);
	for (u32 i = 0; i < kinds.size(); ++i)
	{
		const NqCatalog::SKind& k = *kinds[i];
		const xr_string title = NqLook::KindTitle(k.id.c_str(), behaviour);
		if (!query.empty() && !XUI::Match(query.c_str(), k.id.c_str()) && !XUI::Match(query.c_str(), title.c_str())) continue;
		if (k.group != last_group || shown.empty()) { ++groups; last_group = k.group; }
		shown.push_back(kinds[i]);
		titles.push_back(title);
	}
	if (shown.empty()) { m_FilterSel = 0; ImGui::TextDisabled("No kind matches"); return 0; }

	// a single-line input leaves Up/Down/Enter alone, so the list answers them
	// while the caret stays in the filter box - no mouse in the loop
	int sel = m_FilterSel;
	if (ImGui::IsKeyPressed(ImGuiKey_DownArrow))	++sel;
	if (ImGui::IsKeyPressed(ImGuiKey_UpArrow))		--sel;
	if (sel < 0) sel = (int)shown.size() - 1;
	if (sel >= (int)shown.size()) sel = 0;
	const bool follow = refilter || sel != m_FilterSel;
	m_FilterSel = sel;

	// not on the first frame: the Enter that opened the popup is still pressed
	const NqCatalog::SKind* picked = 0;
	if (!appearing && (ImGui::IsKeyPressed(ImGuiKey_Enter) || ImGui::IsKeyPressed(ImGuiKey_KeypadEnter))) picked = shown[sel];

	const float rows = _min(kPickerRows, float(shown.size()) + float(groups));
	ImGui::BeginChild("##nq_list", ImVec2(0.f, rows * ImGui::GetTextLineHeightWithSpacing()), false);
	xr_string cur_group;
	for (u32 i = 0; i < shown.size(); ++i)
	{
		const NqCatalog::SKind& k = *shown[i];
		if (k.group != cur_group)
		{
			if (i) ImGui::Separator();
			{
				XUI::FontScope font(XUI::Font::Small);
				ImGui::TextDisabled("%s", k.group.c_str());
			}
			cur_group = k.group;
		}
		// the id only disambiguates the row, ImGui hides everything after "##"
		xr_string label = titles[i] + "##" + k.id;
		if (ImGui::Selectable(label.c_str(), (int)i == sel)) picked = shown[i];
		const xr_string desc = NqLook::KindDesc(k.id.c_str(), behaviour);
		if (!desc.empty()) XUI::Tip(desc.c_str());
		if (follow && (int)i == sel) ImGui::SetScrollHereY(0.5f);
		// the technical id keeps its place at the right edge, but only while the title
		// leaves room for it
		const float idw = ImGui::CalcTextSize(k.id.c_str()).x;
		const float x = ImGui::GetCursorPosX() + ImGui::GetContentRegionAvail().x - idw;
		if (x > ImGui::CalcTextSize(titles[i].c_str()).x + ImGui::GetStyle().ItemSpacing.x * 2.f)
		{
			ImGui::SameLine(x);
			ImGui::TextDisabled("%s", k.id.c_str());
		}
	}
	ImGui::EndChild();
	return picked;
}

void NqCanvas::DrawAddNodeMenu(LPCSTR popup)
{
	const u32 mask = NqCatalog::useTrigger | NqCatalog::useMain;
	ConstrainPicker(popup, mask);
	if (!ImGui::BeginPopup(popup)) { if (0 == strcmp(popup, "nq_add_node")) m_PendingLink = false; return; }
	if (const NqCatalog::SKind* k = PickKind("Search kinds", mask))
	{
		CreateNode(*k, m_MenuWorld, m_PendingLink);
		m_PendingLink = false;
		ImGui::CloseCurrentPopup();
	}
	ImGui::EndPopup();
}

void NqCanvas::DrawContextMenus()
{
	// popups requested from inside other popups (or from the Details panel) open here, at the
	// canvas level
	if (m_OpenAddAction)
	{
		m_OpenAddAction = false;
		m_Filter[0] = 0;
		if (m_AddActionAt) XUI::Shell::OpenPopupAt("nq_add_action", m_AddActionPos);
		else ImGui::OpenPopup("nq_add_action");
		m_AddActionAt = false;
	}
	DrawAddNodeMenu("nq_add_node");

	if (ImGui::BeginPopup("nq_node_ctx"))
	{
		if (XUI::MenuItem("Rename", "F2"))			m_WantRename = true;
		bool all_bookmarked = !m_Doc->selection.empty();
		for (u32 i = 0; i < m_Doc->selection.size(); ++i) all_bookmarked &= IsBookmarked(m_Doc->selection[i].c_str());
		if (XUI::MenuItem(all_bookmarked ? "Remove Bookmarks" : "Bookmark", "Ctrl+B")) ToggleSelectedBookmarks();
		if (XUI::MenuItem("Duplicate", "Ctrl+D"))		DuplicateSelection();
		if (XUI::MenuItem("Copy", "Ctrl+C"))			CopySelection();
		ImGui::Separator();
		const bool trigger = NqText::IsTrigger(m_Doc->quest.FindNode(m_CtxNode.c_str()) ? m_Doc->quest.FindNode(m_CtxNode.c_str())->kind.c_str() : "");
		if (ImGui::MenuItem("Add Action on Enter...", 0, false, !trigger))	{ m_AddSlot = "enter"; m_OpenAddAction = true; }
		if (ImGui::MenuItem("Add Action on Exit...", 0, false, !trigger))	{ m_AddSlot = "exit"; m_OpenAddAction = true; }
		if (XUI::MenuItem("Connect to Nearest Free Pin", "Q", false, m_Doc->selection.size() == 1)) ConnectNearest();
		if (XUI::MenuItem("Bypass", "Alt+D"))			BypassSelected();
		if (ImGui::MenuItem("Disconnect Outputs"))
		{
			xr_string id = m_CtxNode;
			m_Doc->Edit([&](SNqQuest& q) { SNqNode* n = q.FindNode(id.c_str()); if (n) n->out.clear(); });
		}
		ImGui::Separator();
		if (XUI::MenuItem("Delete", "Del"))			DeleteSelection();
		ImGui::EndPopup();
	}

	ConstrainPicker("nq_add_action", NqCatalog::useExtra);
	if (ImGui::BeginPopup("nq_add_action"))
	{
		if (const NqCatalog::SKind* k = PickKind("Search actions", NqCatalog::useExtra))
		{
			xr_string id = m_CtxNode, slot = m_AddSlot.empty() ? xr_string("enter") : m_AddSlot, kind = k->id;
			int index = -1;
			m_Doc->Edit([&](SNqQuest& q)
			{
				SNqNode* n = q.FindNode(id.c_str());
				if (!n) return;
				SNqAction a; a.kind = kind; a.params = SNqValue::Table();
				xr_vector<SNqAction>& v = n->Slot(slot.c_str());
				v.push_back(a);
				index = (int)v.size() - 1;
			});
			if (index >= 0)
			{
				SelectOnly(id.c_str());
				m_Doc->sel_slot = slot + ":" + NqUtil::Format("%d", index);
				m_WantFocusAction = true;
			}
			ImGui::CloseCurrentPopup();
		}
		ImGui::EndPopup();
	}

	if (ImGui::BeginPopup("nq_action_ctx"))
	{
		xr_string slot; int index = -1;
		{
			LPCSTR sel = m_Doc->sel_slot.c_str();
			LPCSTR c = strchr(sel, ':');
			if (c) { slot.assign(sel, c - sel); index = atoi(c + 1); }
		}
		const SNqNode* n = m_Doc->quest.FindNode(m_CtxNode.c_str());
		const xr_vector<SNqAction>* v = n ? n->SlotC(slot.c_str()) : 0;
		const int count = v ? (int)v->size() : 0;
		const bool valid = index >= 0 && index < count;
		const xr_string id = m_CtxNode;
		if (ImGui::MenuItem("Duplicate Action", 0, false, valid))
		{
			m_Doc->Edit([&](SNqQuest& q)
			{
				SNqNode* x = q.FindNode(id.c_str()); if (!x) return;
				xr_vector<SNqAction>& list = x->Slot(slot.c_str());
				if (index < (int)list.size()) list.insert(list.begin() + index + 1, list[index]);
			});
		}
		if (ImGui::MenuItem("Move Up", 0, false, valid && index > 0))					MoveAction(id, slot, index, id, slot, index - 1);
		if (ImGui::MenuItem("Move Down", 0, false, valid && index + 1 < count))			MoveAction(id, slot, index, id, slot, index + 2);
		const xr_string other = slot == "exit" ? "enter" : "exit";
		if (ImGui::MenuItem(slot == "exit" ? "Move to On Enter" : "Move to On Exit", 0, false, valid))
			MoveAction(id, slot, index, id, other, n ? (int)n->SlotC(other.c_str())->size() : 0);
		ImGui::Separator();
		if (ImGui::MenuItem("Remove Action", 0, false, valid))
		{
			m_Doc->Edit([&](SNqQuest& q)
			{
				SNqNode* x = q.FindNode(id.c_str()); if (!x) return;
				xr_vector<SNqAction>& list = x->Slot(slot.c_str());
				if (index < (int)list.size()) list.erase(list.begin() + index);
			});
			m_Doc->sel_slot.clear();
		}
		ImGui::EndPopup();
	}

	if (ImGui::BeginPopup("nq_link_ctx"))
	{
		const xr_string what = NqUtil::Format("%s.%s -> %s", m_CtxLinkFrom.c_str(), m_CtxLinkPin.c_str(), m_CtxLinkTo.c_str());
		{
			XUI::FontScope font(XUI::Font::Small);
			ImGui::TextDisabled("%s", what.c_str());
		}
		if (ImGui::MenuItem("Remove Link"))
		{
			xr_string err;
			m_Doc->Disconnect(m_CtxLinkFrom.c_str(), m_CtxLinkPin.c_str(), m_CtxLinkTo.c_str(), err);
		}
		ImGui::EndPopup();
	}
}

//------------------------------------------------------------------------------
// frame
//------------------------------------------------------------------------------
void NqCanvas::Draw(const ImVec2& size)
{
	m_Origin = ImGui::GetCursorScreenPos();
	m_Size = ImVec2(_max(size.x, 32.f), _max(size.y, 32.f));
	m_DrawnFrame = ImGui::GetFrameCount();
	// the view is not part of the file, but panning, framing and quest_view all
	// move it: bound it before anything is projected through it
	m_Doc->view_cx = NqLayout::Sane(m_Doc->view_cx);
	m_Doc->view_cy = NqLayout::Sane(m_Doc->view_cy);

	// deferred framing requests need the rect
	if (m_WantFrameAll)			{ FrameAll(); m_WantFrameAll = false; }
	if (m_WantFrameSel)			{ FrameSelection(); m_WantFrameSel = false; }
	// framing picks a zoom of its own; an explicit one was asked for later
	if (m_WantZoom >= 0)		{ m_Doc->zoom_idx = m_WantZoom; m_WantZoom = -1; }
	m_HistoryReady = true;
	PruneTransient();

	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(m_Origin, Add(m_Origin, m_Size), C(T::Canvas));
	dl->PushClipRect(m_Origin, Add(m_Origin, m_Size), true);

	// one invisible button owns the mouse for the whole area
	ImGui::InvisibleButton("##nq_canvas", m_Size, ImGuiButtonFlags_MouseButtonLeft | ImGuiButtonFlags_MouseButtonRight | ImGuiButtonFlags_MouseButtonMiddle);
	m_Hovered = ImGui::IsItemHovered();
	// last frame's hover text, once the same thing has been under the cursor for a moment
	if (m_Hovered && !m_TipText.empty() && ImGui::GetTime() - m_TipSince >= 0.45) XUI::Tip(m_TipText.c_str());
	m_TipNext.clear();

	EnsureReachable();
	BuildGeometry();
	BuildLinks();
	UpdateHot();

	DrawGrid(dl);
	DrawLinks(dl);
	DrawLinking(dl);
	for (u32 i = 0; i < m_Geom.size(); ++i) DrawNode(dl, (int)i);
	DrawActionDrag(dl);
	DrawMarquee(dl);
	DrawMinimap(dl);
	DrawBeyond(dl);
	DrawStatus(dl);
	if (m_Geom.empty())
	{
		ImFont* body = Face(XUI::Font::Body);
		const float px = Round(XUI::FontSize(XUI::Font::Body) * XUI::Dpi());
		static const char* kEmpty = "Right-click to add a node";
		const float w = TextW(body, px, kEmpty);
		dl->AddText(body, px, ImVec2(Round(m_Origin.x + (m_Size.x - w) * 0.5f), Round(m_Origin.y + (m_Size.y - px) * 0.5f)), C(T::TextMuted), kEmpty);
	}
	dl->PopClipRect();

	HandleInput();
	HandleKeys();
	DrawContextMenus();

	if (m_TipNext != m_TipText) { m_TipText = m_TipNext; m_TipSince = ImGui::GetTime(); }
}
