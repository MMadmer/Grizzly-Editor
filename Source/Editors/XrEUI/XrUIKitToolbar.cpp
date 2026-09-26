#define IMGUI_DEFINE_MATH_OPERATORS
#include "stdafx.h"
#include "imgui_internal.h"
#include "XrUIKitInternal.h"

using namespace XUI::Internal;

// A toolbar is laid out from what it held the frame before: every item is measured as it is
// submitted, and the next frame decides from those measures which labels collapse to icons and
// which items move into the overflow menu. An item therefore keeps its place while the row is
// drawn, and a row that is too narrow never wraps and never clips.
namespace
{
enum ItemType : u8 { tAction, tMenu, tSplit, tSearch, tWidget, tStatus, tSegmented, tSeparator, tSpring };
enum Mode : u8 { mFull, mIcon, mHidden };

struct Item
{
	ItemType type = tAction;
	ImGuiID id = 0;
	int group = 0;
	int priority = 0;
	float w_full = 0.f;
	float w_icon = 0.f;
	bool collapsible = false;
	bool hidden = false;
	u8 shown = mFull;		// what the item was drawn as (Mode): the next frame keeps it while it fits
	// kept for the overflow menu
	XUI::Icon icon = XUI::Icon::None;
	xr_string label, tip;
	bool on = true;
	bool checked = false;
	XUI::Tone tone = XUI::Tone::Muted;
	char* buf = nullptr;
	size_t buf_n = 0;
	// a Segmented: its choices and the one picked, for the overflow's radio items
	XUI::Strings options;
	int current = -1;
};

struct State
{
	xr_vector<Item> last, cur;
	xr_vector<u8> modes;		// per item of `last`
	xr_vector<float> extra;		// per item of `last`: free space a search field or a spring takes
	xr_vector<ImRect> pills;	// overlay: last frame's group pills, relative to the origin
	ImRect overflow_rect;
	ImGuiID pending_click = 0;
	ImGuiID pending_menu = 0;
	ImGuiID pending_segment = 0;
	int pending_choice = -1;
};

xr_map<ImGuiID, State> s_States;

struct Frame
{
	State* st = nullptr;
	ImGuiWindow* window = nullptr;
	int style = XUI::ToolbarRow;
	ImVec2 origin;
	float width = 0.f, height = 0.f, button = 0.f;
	float x = 0.f;
	int group = 0, priority = 0;
	int serial = 0;
	bool any_visible = false;
	ItemType prev_type = tAction;
	int prev_group = 0;
	int hidden = 0;
	float pill_min = FLT_MAX, pill_max = -FLT_MAX;
	int pill_group = -1;
	xr_vector<ImRect> pills;
	xr_string name;
};

float Gap(int style, ItemType a, int group_a, ItemType b, int group_b)
{
	if (a == tSeparator || b == tSeparator)
		return XUI::Pt(6.f);
	if (group_a != group_b)
		return XUI::Pt(style == XUI::ToolbarOverlay ? 8.f : 12.f);
	return XUI::Pt(2.f);
}

bool Visible(const Item& it, u8 mode)
{
	return mode != mHidden && it.type != tSeparator && it.type != tSpring;
}

// a separator shows only between two shown items, and never twice in a row
void ResolveSeparators(const xr_vector<Item>& items, xr_vector<u8>& modes)
{
	bool before = false;
	for (u32 i = 0; i < items.size(); ++i)
	{
		if (items[i].type != tSeparator)
		{
			before |= Visible(items[i], modes[i]);
			continue;
		}
		bool after = false;
		for (u32 j = i + 1; j < items.size() && items[j].type != tSeparator && !after; ++j)
			after = Visible(items[j], modes[j]);
		modes[i] = before && after ? mFull : mHidden;
		if (modes[i] == mFull)
			before = false;
	}
}

float RowWidth(const xr_vector<Item>& items, xr_vector<u8>& modes, int style)
{
	ResolveSeparators(items, modes);
	float w = 0.f;
	bool first = true;
	ItemType prev_type = tAction;
	int prev_group = 0;
	for (u32 i = 0; i < items.size(); ++i)
	{
		const Item& it = items[i];
		if (modes[i] == mHidden)
			continue;
		if (!first)
			w += Gap(style, prev_type, prev_group, it.type, it.group);
		w += it.type == tSpring ? 0.f : modes[i] == mIcon ? it.w_icon : it.w_full;
		first = false;
		prev_type = it.type;
		prev_group = it.group;
	}
	return w;
}

// lowest priority first, the rightmost of equals first
xr_vector<int> GiveWayOrder(const xr_vector<Item>& items, bool (*eligible)(const Item&))
{
	xr_vector<int> order;
	for (int i = 0; i < int(items.size()); ++i)
		if (eligible(items[i]))
			order.push_back(i);
	std::stable_sort(order.begin(), order.end(), [&](int a, int b)
	{
		return items[a].priority != items[b].priority ? items[a].priority < items[b].priority : a > b;
	});
	return order;
}

// pt of room a folded label needs beyond its own width before it unfolds again
constexpr float kUnfoldSlack = 24.f;

// items shown, labels counting twice: the layouts Decide weighs are all folds of one order, so more is more
int Shows(const xr_vector<Item>& items, const xr_vector<u8>& modes)
{
	int n = 0;
	for (u32 i = 0; i < items.size(); ++i)
		if (items[i].type != tSpring && items[i].type != tSeparator)
			n += modes[i] == mFull ? 2 : modes[i] == mIcon ? 1 : 0;
	return n;
}

// The layout for a row `room` wide: labels fold first, then items move into the overflow. The width it takes.
float Fold(const Frame& f, float room, xr_vector<u8>& modes, bool& any_hidden)
{
	const State& st = *f.st;
	const float overflow = f.button + Gap(f.style, tAction, 0, tAction, 1);
	modes.assign(st.last.size(), mFull);
	any_hidden = false;
	float w = RowWidth(st.last, modes, f.style);
	if (w > room)
	{
		for (int i : GiveWayOrder(st.last, [](const Item& it) { return it.collapsible; }))
		{
			modes[i] = mIcon;
			w = RowWidth(st.last, modes, f.style);
			if (w <= room)
				break;
		}
	}
	if (w > room)
	{
		for (int i : GiveWayOrder(st.last, [](const Item& it) { return it.type != tSpring && it.type != tSeparator; }))
		{
			modes[i] = mHidden;
			any_hidden = true;
			w = RowWidth(st.last, modes, f.style);
			if (w + overflow <= room)
				break;
		}
	}
	return w;
}

// Hysteresis: a label unfolds only with kUnfoldSlack to spare, and a row keeps what it showed last frame while that
// still fits. Decided afresh every frame, a value one character wider elsewhere on the row ("150%" for "75%",
// "2 warnings" for "1 warning") folded another item's label, and the next change unfolded it again.
void Decide(Frame& f)
{
	State& st = *f.st;
	const u32 n = st.last.size();
	const float overflow = f.button + Gap(f.style, tAction, 0, tAction, 1);
	bool any_hidden = false;
	float w = Fold(f, f.width - XUI::Pt(kUnfoldSlack), st.modes, any_hidden);
	xr_vector<u8> before(n);
	bool before_hidden = false;
	for (u32 i = 0; i < n; ++i)
	{
		before[i] = st.last[i].shown;
		before_hidden |= before[i] == mHidden && st.last[i].type != tSpring && st.last[i].type != tSeparator;
	}
	const float w_before = RowWidth(st.last, before, f.style);
	if (w_before + (before_hidden ? overflow : 0.f) <= f.width && Shows(st.last, before) > Shows(st.last, st.modes))
	{
		st.modes.swap(before);
		w = w_before;
		any_hidden = before_hidden;
	}
	st.extra.assign(n, 0.f);
	// what is left goes to the search fields first (up to twice their minimum), then to the springs
	float free = f.width - w - (any_hidden ? overflow : 0.f);
	if (free <= 0.f)
		return;
	int searches = 0, springs = 0;
	for (u32 i = 0; i < n; ++i)
	{
		searches += st.last[i].type == tSearch && st.modes[i] != mHidden;
		springs += st.last[i].type == tSpring;
	}
	for (u32 i = 0; searches && i < n; ++i)
		if (st.last[i].type == tSearch && st.modes[i] != mHidden)
		{
			st.extra[i] = ImMin(free / searches, st.last[i].w_full);
			free -= st.extra[i];
			--searches;
		}
	for (u32 i = 0; springs && i < n; ++i)
		if (st.last[i].type == tSpring)
			st.extra[i] = free / springs;
}

// finds this frame's item in last frame's layout; false when it is in the overflow
bool Place(Frame& f, Item& it, float& x, float& w, u8& mode)
{
	State& st = *f.st;
	it.group = f.group;
	it.priority = f.priority;
	int found = -1;
	const int hint = int(st.cur.size());
	if (hint < int(st.last.size()) && st.last[hint].id == it.id)
		found = hint;
	else
		for (int i = 0; i < int(st.last.size()) && found < 0; ++i)
			if (st.last[i].id == it.id)
				found = i;
	mode = found >= 0 && found < int(st.modes.size()) ? st.modes[found] : u8(mFull);
	const float extra = found >= 0 && found < int(st.extra.size()) ? st.extra[found] : 0.f;
	it.hidden = mode == mHidden;
	it.shown = mode;
	st.cur.push_back(it);
	if (it.hidden)
	{
		++f.hidden;
		return false;
	}
	w = (it.type == tSpring ? 0.f : mode == mIcon ? it.w_icon : it.w_full) + extra;
	if (f.any_visible)
		f.x += Gap(f.style, f.prev_type, f.prev_group, it.type, it.group);
	x = f.x;
	f.x += w;
	f.any_visible = true;
	f.prev_type = it.type;
	f.prev_group = it.group;

	// overlay: each group sits on a pill of its own
	if (f.style == XUI::ToolbarOverlay && it.type != tSpring && it.type != tSeparator)
	{
		if (it.group != f.pill_group && f.pill_group >= 0)
		{
			f.pills.push_back(ImRect(f.pill_min, 0.f, f.pill_max, 0.f));
			f.pill_min = FLT_MAX;
			f.pill_max = -FLT_MAX;
		}
		f.pill_group = it.group;
		f.pill_min = ImMin(f.pill_min, x);
		f.pill_max = ImMax(f.pill_max, x + w);
	}
	return true;
}

float CentreY(const Frame& f, float h)
{
	return IM_ROUND(f.origin.y + (f.height - h) * .5f);
}

bool OverflowRow(const Item& it, bool submenu)
{
	ImGui::PushID(int(it.id));
	const float h = XUI::Pt(24.f);
	const char* label = it.label.c_str();
	const float icon = XUI::Pt(16.f);
	const float w = ImMax(XUI::Em(10.f), XUI::Pt(8.f) + icon + XUI::Pt(8.f) + TextSize(XUI::Font::Body, label).x + XUI::Pt(8.f) +
		(submenu ? XUI::Pt(16.f) : 0.f));
	const bool pressed = ImGui::Selectable("##row", it.checked, it.on ? 0 : ImGuiSelectableFlags_Disabled, ImVec2(w, h));
	const ImRect bb(ImGui::GetItemRectMin(), ImGui::GetItemRectMax());
	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImU32 tc = Col(it.on ? Token::Text : Token::TextDisabled);
	const float cy = bb.GetCenter().y;
	if (it.icon != XUI::Icon::None)
		XUI::DrawIcon(dl, it.icon, ImVec2(IM_ROUND(bb.Min.x + XUI::Pt(4.f)), IM_ROUND(cy - icon * .5f)), 16.f,
			Col(it.checked ? Token::AccentBright : it.on ? Token::TextSecondary : Token::TextDisabled));
	PaintText(dl, XUI::Font::Body, ImVec2(IM_ROUND(bb.Min.x + XUI::Pt(4.f) + icon + XUI::Pt(8.f)), IM_ROUND(cy - FontPx(XUI::Font::Body) * .5f)), tc, label);
	if (submenu)
		XUI::DrawIcon(dl, XUI::Icon::ChevronRight, ImVec2(IM_ROUND(bb.Max.x - XUI::Pt(14.f)), IM_ROUND(cy - XUI::Pt(6.f))), 12.f, Col(Token::TextSecondary));
	if (!it.tip.empty())
		XUI::Tip(it.tip.c_str());
	AuditAdd("control", label, bb, nullptr, it.on, nullptr, it.tip.c_str());
	ImGui::PopID();
	return pressed && it.on;
}
}

XUI::Toolbar::Toolbar(cstr id, int style)
{
	Frame* f = xr_new<Frame>();
	m_State = f;
	ImGuiWindow* window = ImGui::GetCurrentWindow();
	if (window->SkipItems)
		return;
	f->window = window;
	f->style = style;
	ImGui::PushID(id);
	f->st = &s_States[window->GetID("##toolbar")];
	f->name = "toolbar_overflow:";
	f->name += id ? id : "";
	f->height = Pt(style == ToolbarRow ? 36.f : 32.f);
	f->button = Pt(style == ToolbarRow ? 28.f : 24.f);
	f->origin = window->DC.CursorPos;
	f->width = ImMax(0.f, ImGui::GetContentRegionAvail().x);
	f->x = f->origin.x;
	f->st->cur.clear();
	Decide(*f);

	// the pills go under the items, so they are last frame's: the layout they follow is too
	if (style == ToolbarOverlay)
		for (const ImRect& pill : f->st->pills)
			window->DrawList->AddRectFilled(f->origin + pill.Min, f->origin + pill.Max, Col(Token::Overlay, .92f), Pt(6.f));
}

XUI::Toolbar::~Toolbar()
{
	Frame* f = (Frame*)m_State;
	if (f && f->window)
	{
		ImGuiWindow* window = f->window;
		State& st = *f->st;

		// everything that did not fit is one click away
		if (f->hidden > 0)
		{
			const ImGuiID popup_id = window->GetID("##overflow_popup");
			ImGui::SetCursorScreenPos(ImVec2(f->origin.x + f->width - f->button, CentreY(*f, f->button)));
			BtnSpec s;
			s.id = "##overflow";
			s.text = "";
			s.icon = Icon::Overflow;
			s.kind = Kind::Ghost;
			s.tip = "More";
			s.size = ImVec2(f->button, f->button);
			s.checked = ImGui::IsPopupOpen(popup_id, ImGuiPopupFlags_None);
			ImRect bb;
			const bool clicked = ButtonEx(s, &bb);
			if (clicked || TakePopupRequest(f->name.c_str()))
				OpenPopupAt(popup_id, ImVec2(bb.Min.x, bb.Max.y + Pt(2.f)));
			st.overflow_rect = bb;
			if (f->style == ToolbarOverlay)
				f->pills.push_back(ImRect(bb.Min.x, 0.f, bb.Max.x, 0.f));
			if (ImGui::BeginPopupEx(popup_id, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings))
			{
				for (Item& it : st.cur)
				{
					if (!it.hidden)
						continue;
					switch (it.type)
					{
					case tAction:
						if (OverflowRow(it, false))
						{
							st.pending_click = it.id;
							ImGui::CloseCurrentPopup();
						}
						break;
					case tMenu:
						if (OverflowRow(it, true))
						{
							st.pending_menu = it.id;
							ImGui::CloseCurrentPopup();
						}
						break;
					case tSplit:
					{
						// the action, then its menu as a row of its own
						if (OverflowRow(it, false))
						{
							st.pending_click = it.id;
							ImGui::CloseCurrentPopup();
						}
						Item more = it;
						more.id = it.id + 1;
						more.icon = Icon::None;
						more.label = it.label + " Options";
						if (OverflowRow(more, true))
						{
							st.pending_menu = it.id;
							ImGui::CloseCurrentPopup();
						}
						break;
					}
					case tSearch:
						if (it.buf)
						{
							ImGui::SetNextItemWidth(Em(12.f));
							SearchField("##overflow_search", it.buf, it.buf_n, it.label.empty() ? "Search" : it.label.c_str());
						}
						break;
					case tStatus:
						StatusText(it.tone, it.label.c_str());
						break;
					case tSegmented:
					{
						// the choices as radio items under their caption
						ImGui::PushID(int(it.id));
						{
							FontScope caption_font(Font::Small);
							ImGui::PushStyleColor(ImGuiCol_Text, Col(Token::TextMuted));
							ImGui::TextUnformatted(it.label.c_str());
							ImGui::PopStyleColor();
						}
						for (int k = 0; k < int(it.options.size()); ++k)
						{
							ImGui::PushID(k);
							if (MenuItem(it.options[k].c_str(), nullptr, k == it.current, it.on))
							{
								st.pending_segment = it.id;
								st.pending_choice = k;
								ImGui::CloseCurrentPopup();
							}
							ImGui::PopID();
						}
						ImGui::PopID();
						break;
					}
					default:
						break;
					}
				}
				ImGui::EndPopup();
			}
		}

		// overlay pills for the next frame, relative to the origin
		if (f->style == ToolbarOverlay)
		{
			if (f->pill_group >= 0)
				f->pills.push_back(ImRect(f->pill_min, 0.f, f->pill_max, 0.f));
			st.pills.clear();
			const float top = (f->height - f->button) * .5f - Pt(2.f);
			for (const ImRect& pill : f->pills)
				st.pills.push_back(ImRect(pill.Min.x - f->origin.x - Pt(2.f), top, pill.Max.x - f->origin.x + Pt(2.f), f->height - top));
		}
		else if (f->style == ToolbarPanel)
			window->DrawList->AddLine(ImVec2(f->origin.x, f->origin.y + f->height - 1.f), ImVec2(f->origin.x + f->width, f->origin.y + f->height - 1.f),
				Col(Token::BorderSubtle));

		// the row as one item of its window
		ImGui::SetCursorScreenPos(f->origin);
		ImGui::ItemSize(ImVec2(f->width, f->height));
		ImGui::ItemAdd(ImRect(f->origin, f->origin + ImVec2(f->width, f->height)), 0);
		if (f->x > f->origin.x + f->width + 1.f)
			AuditAdd("escaped", f->name.c_str(), ImRect(f->origin, ImVec2(f->x, f->origin.y + f->height)), "toolbar row");

		st.last.swap(st.cur);
		st.cur.clear();
		ImGui::PopID();
	}
	xr_delete(f);
	m_State = nullptr;
}

void XUI::Toolbar::Group(int priority)
{
	Frame* f = (Frame*)m_State;
	++f->group;
	f->priority = priority;
}

void XUI::Toolbar::Separator()
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return;
	Item it;
	it.type = tSeparator;
	it.id = f->window->GetID(0x5e9000 + f->serial++);
	it.w_full = it.w_icon = ImMax(1.f, Pt(1.f));
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (Place(*f, it, x, w, mode))
		f->window->DrawList->AddLine(ImVec2(IM_ROUND(x), f->origin.y + Pt(8.f)), ImVec2(IM_ROUND(x), f->origin.y + f->height - Pt(8.f)),
			Col(Token::Border), w);
}

void XUI::Toolbar::Spring()
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return;
	Item it;
	it.type = tSpring;
	it.id = f->window->GetID(0x5b7000 + f->serial++);
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	Place(*f, it, x, w, mode);
}

void XUI::Toolbar::Status(cstr text, Tone tone)
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return;
	Item it;
	it.type = tStatus;
	it.id = f->window->GetID(0x57a000 + f->serial++);
	it.label = text ? text : "";
	it.tone = tone;
	it.w_full = it.w_icon = TextSize(Font::Body, it.label.c_str()).x;
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
		return;
	const ImU32 col = tone == Tone::Muted ? Col(Token::TextMuted) : tone == Tone::Info ? Col(Token::TextSecondary) : ToneColor(tone);
	const float y = CentreY(*f, FontPx(Font::Body));
	const ImVec4 clip(x, f->origin.y, x + w, f->origin.y + f->height);
	PaintText(f->window->DrawList, Font::Body, ImVec2(IM_ROUND(x), y), col, it.label.c_str(), nullptr, 0.f, &clip);
}

bool XUI::Toolbar::Action(cstr id, Icon icon, cstr label, cstr tip, bool on, bool checked, Kind kind)
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return false;
	BtnSpec s;
	s.id = id;
	s.text = label ? label : "";
	s.icon = icon;
	s.kind = kind;
	s.on = on;
	s.checked = checked;
	s.tip = tip;
	s.why = on ? nullptr : tip;
	s.font = kind == Kind::Primary || kind == Kind::Danger ? Font::Strong : Font::Body;
	s.size = ImVec2(0.f, f->button);
	s.audit_label = label && *label ? label : tip;

	Item it;
	it.type = tAction;
	it.id = f->window->GetID(id);
	it.icon = icon;
	it.label = label ? label : "";
	it.tip = tip ? tip : "";
	it.on = on;
	it.checked = checked;
	it.w_full = ButtonWidth(s);
	it.w_icon = icon != Icon::None ? f->button : it.w_full;
	it.collapsible = icon != Icon::None && label && *label;

	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
	{
		// picked from the overflow menu last frame
		if (f->st->pending_click == it.id)
		{
			f->st->pending_click = 0;
			return on;
		}
		return false;
	}
	if (mode == mIcon)
	{
		s.text = "";
		s.tip_title = label;
	}
	s.size = ImVec2(w, f->button);
	ImGui::SetCursorScreenPos(ImVec2(x, CentreY(*f, f->button)));
	return ButtonEx(s);
}

bool XUI::Toolbar::Menu(cstr id, Icon icon, cstr label, cstr tip, cstr request, bool on)
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return false;
	ImGui::PushID(id);
	const ImGuiID popup_id = f->window->GetID("##menu");
	ImGui::PopID();

	BtnSpec s;
	s.id = id;
	s.text = label ? label : "";
	s.icon = icon;
	s.kind = Kind::Ghost;
	s.caret = true;
	s.on = on;
	s.tip = tip;
	s.why = on ? nullptr : tip;
	s.size = ImVec2(0.f, f->button);
	s.audit_label = label && *label ? label : tip;
	s.checked = ImGui::IsPopupOpen(popup_id, ImGuiPopupFlags_None);

	Item it;
	it.type = tMenu;
	it.id = f->window->GetID(id);
	it.icon = icon;
	it.label = label ? label : "";
	it.tip = tip ? tip : "";
	it.on = on;
	it.w_full = ButtonWidth(s);
	{
		BtnSpec compact = s;
		compact.text = "";
		it.w_icon = icon != Icon::None ? ButtonWidth(compact) : it.w_full;
	}
	it.collapsible = icon != Icon::None && label && *label;

	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
	{
		// picked from the overflow menu last frame, or asked for over MCP while it is there
		if (f->st->pending_menu == it.id || (request && TakePopupRequest(request)))
		{
			f->st->pending_menu = 0;
			if (on)
				OpenPopupAt(popup_id, ImVec2(f->st->overflow_rect.Min.x, f->st->overflow_rect.Max.y + Pt(2.f)));
		}
	}
	else
	{
		if (mode == mIcon)
		{
			s.text = "";
			s.tip_title = label;
		}
		s.size = ImVec2(w, f->button);
		ImGui::SetCursorScreenPos(ImVec2(x, CentreY(*f, f->button)));
		ImRect bb;
		// off, a request is taken and dropped
		if ((ButtonEx(s, &bb) || (request && TakePopupRequest(request))) && on)
			OpenPopupAt(popup_id, ImVec2(bb.Min.x, bb.Max.y + Pt(2.f)));
	}
	if (!on)
	{
		// a menu that goes off takes its open popup with it
		ImGuiContext& g = *GImGui;
		for (int i = 0; i < g.OpenPopupStack.Size; ++i)
			if (g.OpenPopupStack[i].PopupId == popup_id)
			{
				ImGui::ClosePopupToLevel(i, true);
				break;
			}
		return false;
	}
	return ImGui::BeginPopupEx(popup_id, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings);
}

bool XUI::Toolbar::Split(cstr id, Icon icon, cstr label, cstr tip, bool* menu_open, bool on, Kind kind)
{
	Frame* f = (Frame*)m_State;
	if (menu_open)
		*menu_open = false;
	if (!f->window)
		return false;
	ImGui::PushID(id);
	const ImGuiID popup_id = f->window->GetID("##split_menu");
	ImGui::PopID();

	BtnSpec main;
	main.id = id;
	main.text = label ? label : "";
	main.icon = icon;
	main.kind = kind;
	main.on = on;
	main.tip = tip;
	main.why = on ? nullptr : tip;
	main.font = kind == Kind::Primary || kind == Kind::Danger ? Font::Strong : Font::Body;
	main.size = ImVec2(0.f, f->button);
	main.corners = ImDrawFlags_RoundCornersLeft;
	main.audit_label = label && *label ? label : tip;
	const float chevron = Pt(20.f);
	const float seam = ImMax(1.f, Pt(1.f));

	Item it;
	it.type = tSplit;
	it.id = f->window->GetID(id);
	it.icon = icon;
	it.label = label ? label : "";
	it.tip = tip ? tip : "";
	it.on = on;
	it.w_full = ButtonWidth(main) + seam + chevron;
	it.w_icon = icon != Icon::None ? f->button + seam + chevron : it.w_full;
	it.collapsible = icon != Icon::None && label && *label;

	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	bool pressed = false;
	if (!Place(*f, it, x, w, mode))
	{
		// picked from the overflow menu last frame: the action, or its menu under the overflow button
		if (f->st->pending_click == it.id)
		{
			f->st->pending_click = 0;
			pressed = on;
		}
		if (f->st->pending_menu == it.id)
		{
			f->st->pending_menu = 0;
			OpenPopupAt(popup_id, ImVec2(f->st->overflow_rect.Min.x, f->st->overflow_rect.Max.y + Pt(2.f)));
		}
	}
	else
	{
		if (mode == mIcon)
		{
			main.text = "";
			main.tip_title = label;
		}
		const float y = CentreY(*f, f->button);
		main.size = ImVec2(w - seam - chevron, f->button);
		ImGui::SetCursorScreenPos(ImVec2(x, y));
		ImRect a;
		pressed = ButtonEx(main, &a);
		// the seam between the halves is the row showing through
		BtnSpec more;
		ImGui::PushID(id);
		more.id = "##more";
		more.text = "";
		more.icon = Icon::ChevronDown;
		more.icon_pt = 12.f;
		more.kind = kind;
		more.tip = "More options";
		more.audit_label = "More options";
		more.size = ImVec2(chevron, f->button);
		more.corners = ImDrawFlags_RoundCornersRight;
		ImGui::SetCursorScreenPos(ImVec2(a.Max.x + seam, y));
		ImRect b;
		if (ButtonEx(more, &b) || TakePopupRequest(("toolbar_split:" + Str(id)).c_str()))
			OpenPopupAt(popup_id, ImVec2(a.Min.x, b.Max.y + Pt(2.f)));
		ImGui::PopID();
	}
	const bool open = ImGui::BeginPopupEx(popup_id, ImGuiWindowFlags_AlwaysAutoResize | ImGuiWindowFlags_NoTitleBar | ImGuiWindowFlags_NoSavedSettings);
	if (menu_open)
		*menu_open = open;
	else if (open)
		ImGui::EndPopup();
	return pressed;
}

bool XUI::Toolbar::Search(char* buf, size_t n, cstr hint, float min_em)
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return false;
	char id[32];
	sprintf_s(id, "##search%d", f->serial++);
	Item it;
	it.type = tSearch;
	it.id = f->window->GetID(id);
	it.label = hint ? hint : "";
	it.buf = buf;
	it.buf_n = n;
	it.w_full = it.w_icon = Em(min_em);
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
		return false;
	ImGui::SetCursorScreenPos(ImVec2(x, CentreY(*f, ImGui::GetFrameHeight())));
	ImGui::SetNextItemWidth(w);
	return SearchField(id, buf, n, hint, min_em);
}

bool XUI::Toolbar::Segmented(cstr id, cstr caption, int* cur, const cstr* labels, int n, cstr tip, bool on)
{
	Frame* f = (Frame*)m_State;
	if (!f->window || !cur || !labels || n <= 0)
		return false;
	Item it;
	it.type = tSegmented;
	it.id = f->window->GetID(id);
	it.label = caption ? caption : "";
	it.tip = tip ? tip : "";
	it.on = on;
	it.current = *cur;
	// the width Segmented takes at its natural size
	float seg_w = 0.f;
	for (int i = 0; i < n; ++i)
	{
		it.options.push_back(labels[i] ? Str(labels[i], LabelEnd(labels[i])) : xr_string());
		seg_w = ImMax(seg_w, TextSize(Font::Body, it.options.back().c_str()).x + Pt(20.f));
	}
	it.w_full = it.w_icon = seg_w * n + Pt(4.f);

	// picked from the overflow menu last frame
	bool changed = false;
	if (f->st->pending_segment == it.id)
	{
		const int choice = f->st->pending_choice;
		f->st->pending_segment = 0;
		f->st->pending_choice = -1;
		if (on && choice >= 0 && choice < n && choice != *cur)
		{
			*cur = choice;
			it.current = choice;
			changed = true;
		}
	}
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
		return changed;
	ImGui::SetCursorScreenPos(ImVec2(x, CentreY(*f, ImGui::GetFrameHeight())));
	ImGui::SetNextItemWidth(w);
	ImGui::BeginDisabled(!on);
	changed |= XUI::Segmented(id, cur, labels, n);
	ImGui::EndDisabled();
	if (tip && *tip)
		Tip(tip);
	return changed;
}

bool XUI::Toolbar::Widget(cstr id, float w_pt)
{
	Frame* f = (Frame*)m_State;
	if (!f->window)
		return false;
	Item it;
	it.type = tWidget;
	it.id = f->window->GetID(id);
	it.label = id ? id : "";
	it.w_full = it.w_icon = Pt(w_pt);
	float x = 0.f, w = 0.f;
	u8 mode = mFull;
	if (!Place(*f, it, x, w, mode))
		return false;
	ImGui::SetCursorScreenPos(ImVec2(x, CentreY(*f, ImGui::GetFrameHeight())));
	ImGui::SetNextItemWidth(w);
	return true;
}
