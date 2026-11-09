#include "stdafx.h"
#include "UIEditLightAnim.h"
#include "UIImageEditorForm.h"
#include "UISoundEditorForm.h"
#include "LightAnimLibrary.h"
#include "ui_main.h"
#include "EditorProject.h"
#include "Nq/NqUtil.h"

UIEditLightAnim* UIEditLightAnim::Form = nullptr;

namespace
{
	using GrizzlyTheme::ColorToken;

	ImU32 Tok(ColorToken token, float alpha = 1.f) { return GrizzlyTheme::U32(token, alpha); }

	// pt: a library tab is list | preview | properties (docs/UI_DESIGN.md, sections 2 and 6, Libraries)
	constexpr float kListW		= 260.f;
	constexpr float kGridW		= 320.f;
	constexpr float kSideMin	= 220.f;
	constexpr float kCanvasMin	= 400.f;

	bool	s_Playing		= false;
	float	s_PlayStart		= 0.f;
	int		s_LastPointer	= -1;

	// the checkerboard a colour with alpha is judged against, clipped to the rect
	void Checker(ImDrawList* dl, const ImVec2& a, const ImVec2& b)
	{
		const float cell = XUI::Pt(8.f);
		dl->AddRectFilled(a, b, Tok(ColorToken::Input));
		dl->PushClipRect(a, b, true);
		for (float y = a.y, row = 0.f; y < b.y; y += cell, row += 1.f)
			for (float x = a.x + (int(row) & 1 ? cell : 0.f); x < b.x; x += cell * 2.f)
				dl->AddRectFilled(ImVec2(x, y), ImVec2(x + cell, y + cell), Tok(ColorToken::Hover));
		dl->PopClipRect();
	}
}

//------------------------------------------------------------------------------
// The local kit widgets
//------------------------------------------------------------------------------

namespace
{
	LPCSTR Leaf(LPCSTR key)
	{
		LPCSTR slash = strrchr(key, '\\');
		return slash ? slash + 1 : key;
	}

	// A row leads with its leaf name, so the list is in the order of the leaves - by folder only among equal
	// names. Sorted by the whole key, "cop\test_la_color" sat among the c's and the list read as unsorted.
	bool Before(const ListItemsVec& items, int a, int b)
	{
		LPCSTR ka = items[a]->Key(), kb = items[b]->Key();
		const int leaf = _stricmp(Leaf(ka), Leaf(kb));
		return leaf ? leaf < 0 : _stricmp(ka, kb) < 0;
	}
}

xr_string LibraryFolderText(LPCSTR alias)
{
	if (!FS.path_exist(alias))
		return alias;
	const xr_string path = FS.get_path(alias)->m_Path;
	// a folder of the open project or of the editor is named by where it sits in it, not by a whole lowercased path
	const auto inside = [&path](LPCSTR root, LPCSTR owner, xr_string& text)
	{
		const u32 n = root ? xr_strlen(root) : 0;
		if (!n || path.size() <= n || _strnicmp(path.c_str(), root, n))
			return false;
		// the root ends at a separator, its own or the path's next one: C:\Mod is not a folder of C:\Mo
		const bool whole = root[n - 1] == '\\' || root[n - 1] == '/' || path[n] == '\\' || path[n] == '/';
		if (!whole)
			return false;
		LPCSTR rel = path.c_str() + n;
		while (*rel == '\\' || *rel == '/')
			++rel;
		text = NqUtil::Format("the %s %s folder", owner, rel);
		return true;
	};
	xr_string text;
	if (EditorProject::Active() && inside(EditorProject::Root(), "project's", text))
		return text;
	if (FS.path_exist("$fs_root$") && inside(FS.get_path("$fs_root$")->m_Path, "editor's", text))
		return text;
	return path;
}

void XUI_LocalLibraryList::Rebuild(const ListItemsVec& items)
{
	m_Search = search;
	m_Count = items.size();
	shown.clear();
	for (u32 i = 0; i < items.size(); ++i)
		if (items[i]->Visible() && XUI::Match(search, items[i]->Key()))
			shown.push_back(int(i));
	std::sort(shown.begin(), shown.end(), [&items](int a, int b) { return Before(items, a, b); });
}

int XUI_LocalLibraryList::Find(const ListItemsVec& items, const char* name) const
{
	if (!name || !*name)
		return -1;
	// the first match in the list's order
	int first = -1;
	for (u32 i = 0; i < items.size(); ++i)
	{
		if (0 == _stricmp(items[i]->Key(), name))
			return int(i);
		if (XUI::Match(name, items[i]->Key()) && (first < 0 || Before(items, int(i), first)))
			first = int(i);
	}
	return first;
}

u32 XUI_LocalLibraryList::Count(const ListItemsVec& items, int* first) const
{
	u32 n = 0;
	if (first)
		*first = -1;
	for (u32 i = 0; i < items.size(); ++i)
		if (items[i]->Visible() && XUI::Match(search, items[i]->Key()))
		{
			// the row the list shows first
			if (first && (*first < 0 || Before(items, int(i), *first)))
				*first = int(i);
			++n;
		}
	return n;
}

int XUI_LocalLibraryList::Draw(const char* id, const ListItemsVec& items, const char* selected, const char* empty_title, const char* empty_text,
	const char* empty_action)
{
	acted = false;
	ImGui::PushID(id);
	ImGui::SetNextItemWidth(ImGui::GetContentRegionAvail().x);
	XUI::SearchField("##search", search, sizeof(search), "Search");
	if (m_Count != items.size() || m_Search != search)
		Rebuild(items);

	// the selected row among the shown ones; a selection made elsewhere (MCP, a reload) is brought into view
	int at = -1;
	if (selected && *selected)
		for (int r = 0; r < int(shown.size()) && at < 0; ++r)
			if (0 == xr_strcmp(items[shown[r]]->Key(), selected))
				at = r;
	if (m_Selected != (selected ? selected : ""))
	{
		m_Selected = selected ? selected : "";
		m_Scroll = at;
	}

	int picked = -1;
	if (XUI::BeginList("##rows", ImVec2(0.f, 0.f)))
	{
		const float row = XUI::Pt(22.f);
		if (shown.empty())
			acted = XUI::EmptyState(search[0] ? XUI::Icon::Search : XUI::Icon::Folder, search[0] ? "No matches" : empty_title,
				search[0] ? "Nothing in the library matches the search." : empty_text, search[0] ? nullptr : empty_action);
		else
		{
			if (ImGui::IsWindowFocused() && !ImGui::GetIO().WantTextInput)
			{
				const int step = ImGui::IsKeyPressed(ImGuiKey_DownArrow) ? 1 : ImGui::IsKeyPressed(ImGuiKey_UpArrow) ? -1 : 0;
				const int next = step ? clampr(at < 0 ? 0 : at + step, 0, int(shown.size()) - 1) : at;
				if (next != at)
				{
					picked = shown[next];
					m_Scroll = next;
				}
			}
			if (m_Scroll >= 0)
			{
				const float y = m_Scroll * row;
				const float view = ImGui::GetWindowHeight() - ImGui::GetStyle().WindowPadding.y * 2.f;
				if (y < ImGui::GetScrollY())
					ImGui::SetScrollY(y);
				else if (y + row > ImGui::GetScrollY() + view)
					ImGui::SetScrollY(y + row - view);
				m_Scroll = -1;
			}
			ImGuiListClipper clip;
			clip.Begin(int(shown.size()), row);
			while (clip.Step())
				for (int r = clip.DisplayStart; r < clip.DisplayEnd; ++r)
				{
					ListItem* item = items[shown[r]];
					LPCSTR key = item->Key();
					if (XUI::ListRow(key, r == at))
						picked = shown[r];
					// the leaf names the item; its folder follows, muted, and gives way first
					LPCSTR slash = strrchr(key, '\\');
					LPCSTR leaf = slash ? slash + 1 : key;
					const float room = ImGui::GetContentRegionAvail().x;
					XUI::TextEllipsis(leaf, room);
					const float rest = room - ImGui::CalcTextSize(leaf).x - XUI::Pt(8.f);
					if (slash && rest > XUI::Em(3.f))
					{
						xr_string folder;
						folder.assign(key, size_t(slash - key));
						ImGui::SameLine(0.f, XUI::Pt(8.f));
						ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
						XUI::TextEllipsis(folder.c_str(), ImGui::GetContentRegionAvail().x, XUI::Clip::Start);
						ImGui::PopStyleColor();
					}
				}
		}
	}
	XUI::EndList();
	ImGui::PopID();
	return picked;
}

//------------------------------------------------------------------------------
// Light Animations
//------------------------------------------------------------------------------
UIEditLightAnim::UIEditLightAnim()
{
	m_Modife = false;
	m_Props = xr_new<UIPropertiesForm>();
	m_Props->SetModifiedEvent(TOnModifiedEvent(this, &UIEditLightAnim::OnModified));
	m_CurrentItem = nullptr;
	m_PointerValue = 0;
	m_RenderAlpha = false;
	s_Playing = false;
	s_LastPointer = -1;
	InitializeItems();
}

UIEditLightAnim::~UIEditLightAnim()
{
	// the tab asks about unsaved changes before it closes; a form going with changes still in it was never
	// asked about, and the library in memory goes back to what is on disk
	if (m_Modife)
	{
		Msg("! Light Animations: unsaved changes were dropped");
		LALib.Reload();
	}
	for (ListItem* item : m_List)
		xr_delete(item);
	xr_delete(m_Props);
}

void UIEditLightAnim::Draw()
{
	// list | preview and timeline | the animation's properties; the list and the properties keep their pt size
	const ImGuiChildFlags pane = ImGuiChildFlags_AlwaysUseWindowPadding;
	float list_w = kListW;
	XUI::Splitter("light_animations.list", true, &list_w, kSideMin, kCanvasMin + kSideMin + 8.f, kListW);
	if (ImGui::BeginChild("##list", ImVec2(XUI::Pt(list_w), 0.f), pane))
	{
		// only an empty list says where the library is read from
		const xr_string empty = m_List.empty() ? NqUtil::Format("lanims.xr in %s holds no light animation.",
			LibraryFolderText("$game_data$").c_str()) : xr_string();
		const int picked = m_ListView.Draw("light_animations", m_List, m_CurrentItem ? m_CurrentItem->cName.c_str() : "",
			"No light animations", empty.c_str());
		if (picked >= 0)
			Select(picked);
	}
	ImGui::EndChild();
	ImGui::SameLine();
	if (ImGui::BeginChild("##rest", ImVec2(0.f, 0.f)))
	{
		float grid_w = kGridW;
		XUI::SplitterTail("light_animations.details", true, &grid_w, kCanvasMin, kSideMin, kGridW);
		const float head = _max(1.f, ImGui::GetContentRegionAvail().x - XUI::Pt(grid_w) - ImGui::GetStyle().ItemSpacing.x);
		ImGui::PushStyleColor(ImGuiCol_ChildBg, Tok(ColorToken::Canvas));
		const bool shown = ImGui::BeginChild("##preview", ImVec2(head, 0.f), pane);
		ImGui::PopStyleColor();
		if (shown)
			DrawPreview();
		ImGui::EndChild();
		ImGui::SameLine();
		if (ImGui::BeginChild("##details", ImVec2(0.f, 0.f), pane))
		{
			if (m_CurrentItem)
				m_Props->Draw();
			else
				XUI::EmptyState(XUI::Icon::Settings, "No properties", "Pick a light animation to edit it.");
		}
		ImGui::EndChild();
	}
	ImGui::EndChild();
}

void UIEditLightAnim::DrawToolbar(XUI::Toolbar& tb)
{
	tb.Group(1);
	if (tb.Action("save", XUI::Icon::Save, "Save", m_Modife ? "Save the light animation library (lanims.xr)" : "No unsaved changes", m_Modife))
		Save();
	if (tb.Action("reload", XUI::Icon::Reload, "Reload", "Read the light animation library from disk again"))
		Reload();
	tb.Group(0);
	if (tb.Action("alpha", XUI::Icon::Eye, "Alpha", "Show the colours' alpha over a checkerboard", true, m_RenderAlpha))
		m_RenderAlpha = !m_RenderAlpha;
}

void UIEditLightAnim::DrawStatus()
{
	const xr_string text = NqUtil::Format("%u light animations", u32(m_List.size())) + (m_Modife ? " \xC2\xB7 unsaved changes" : "");
	ImGui::TextUnformatted(text.c_str());
}

void UIEditLightAnim::DrawPreview()
{
	if (!m_CurrentItem)
	{
		XUI::EmptyState(XUI::Icon::Light, "No light animation selected", "Pick one in the list to see its colours and edit its keys.");
		return;
	}
	CLAItem* item = m_CurrentItem;
	// Play runs the animation in real time; the editor draws on demand, so the frames are asked for
	if (s_Playing)
	{
		const float len = _max(item->Length_sec(), EPS_L);
		m_PointerValue = clampr(iFloor(fmodf(EDevice->fTimeGlobal - s_PlayStart, len) * item->fFPS), 0, item->iFrameCount - 1);
		UI->RedrawScene();
	}
	if (m_PointerValue != s_LastPointer)
	{
		s_LastPointer = m_PointerValue;
		UpdateProperties();
	}

	{
		XUI::FontScope title(XUI::Font::Title);
		XUI::TextEllipsis(item->cName.c_str(), -1.f, XUI::Clip::Start);
	}
	{
		const xr_string facts = NqUtil::Format("%d frames \xC2\xB7 %g fps \xC2\xB7 %.2f s \xC2\xB7 %u keys", item->iFrameCount, item->fFPS,
			item->Length_sec(), u32(item->Keys.size()));
		XUI::FontScope small_font(XUI::Font::Small);
		ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
		XUI::TextEllipsis(facts.c_str());
		ImGui::PopStyleColor();
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(8.f)));

	// the colour at the frame the pointer is on: the light as it looks then
	const ImVec2 avail = ImGui::GetContentRegionAvail();
	const float swatch_h = clampr(avail.y - XUI::Pt(40.f + 12.f + 32.f + 24.f), XUI::Pt(48.f), XUI::Pt(220.f));
	{
		ImDrawList* dl = ImGui::GetWindowDrawList();
		const ImVec2 a = ImGui::GetCursorScreenPos();
		const ImVec2 b(a.x + avail.x, a.y + swatch_h);
		u32 color = item->InterpolateBGR(clampr(m_PointerValue, 0, item->iFrameCount));
		if (m_RenderAlpha)
			Checker(dl, a, b);
		else
			color |= 0xFF000000;
		dl->AddRectFilled(a, b, color, XUI::Pt(6.f));
		dl->AddRect(a, b, Tok(ColorToken::Border), XUI::Pt(6.f));
		ImGui::Dummy(ImVec2(avail.x, swatch_h));
		const u32 rgb = item->InterpolateRGB(clampr(m_PointerValue, 0, item->iFrameCount));
		const xr_string tip = NqUtil::Format("Frame %d: #%06X, alpha %u%s", m_PointerValue, rgb & 0xFFFFFF, (rgb >> 24) & 0xFF,
			item->IsKey(m_PointerValue) ? " (a key)" : "");
		XUI::Tip(tip.c_str());
	}
	ImGui::Dummy(ImVec2(0.f, XUI::Pt(12.f)));
	DrawTimeline(XUI::Pt(40.f));
	DrawKeyBar();
}

void UIEditLightAnim::DrawTimeline(float height)
{
	CLAItem* item = m_CurrentItem;
	const int count = _max(1, item->iFrameCount);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImVec2 a = ImGui::GetCursorScreenPos();
	const float w = _max(ImGui::GetContentRegionAvail().x, XUI::Pt(64.f));
	const float lane = XUI::Pt(10.f);
	const ImVec2 b(a.x + w, a.y + height);
	const float strip_b = b.y - lane;

	ImGui::InvisibleButton("##timeline", ImVec2(w, height));
	const bool hovered = ImGui::IsItemHovered();
	const int under = clampr(int((ImGui::GetIO().MousePos.x - a.x) / w * count), 0, count - 1);
	if (hovered && ImGui::IsMouseDoubleClicked(ImGuiMouseButton_Left))
	{
		s_Playing = false;
		m_PointerValue = under;
		OnCreateKeyClick();
	}
	else if (ImGui::IsItemActive())
	{
		s_Playing = false;
		m_PointerValue = under;
	}
	XUI::Tip("Click or drag to pick a frame; double-click adds a key there.");

	// one strip per frame, or one per pixel column when the frames outnumber the pixels
	if (m_RenderAlpha)
		Checker(dl, a, ImVec2(b.x, strip_b));
	const int cols = _min(count, _max(1, int(w)));
	for (int c = 0; c < cols; ++c)
	{
		const int frame = int(float(c) / cols * count);
		u32 color = item->InterpolateBGR(frame);
		if (!m_RenderAlpha)
			color |= 0xFF000000;
		const float x0 = a.x + w * c / cols, x1 = a.x + w * (c + 1) / cols;
		dl->AddRectFilled(ImVec2(floorf(x0), a.y), ImVec2(floorf(x1) + 1.f, strip_b), color);
	}
	dl->AddRectFilled(ImVec2(a.x, strip_b), b, Tok(ColorToken::Inset));
	const float step = w / count;
	for (CLAItem::KeyPairIt it = item->Keys.begin(); it != item->Keys.end(); ++it)
	{
		const float x = floorf(a.x + (it->first + .5f) * step + .5f);
		const float r = XUI::Pt(3.5f);
		const ImVec2 c(x, strip_b + lane * .5f);
		dl->AddQuadFilled(ImVec2(c.x, c.y - r), ImVec2(c.x + r, c.y), ImVec2(c.x, c.y + r), ImVec2(c.x - r, c.y), Tok(ColorToken::Text));
	}
	const float px = floorf(a.x + (m_PointerValue + .5f) * step + .5f);
	dl->AddLine(ImVec2(px, a.y), ImVec2(px, b.y), Tok(ColorToken::AccentBright), _max(1.f, XUI::Pt(2.f)));
	dl->AddRect(a, b, Tok(ColorToken::Border));

	// the frame numbers at both ends
	XUI::FontScope small_font(XUI::Font::Small);
	ImGui::PushStyleColor(ImGuiCol_Text, Tok(ColorToken::TextMuted));
	ImGui::TextUnformatted("0");
	const xr_string last = NqUtil::Format("%d", count - 1);
	ImGui::SameLine(_max(0.f, ImGui::GetContentRegionAvail().x + ImGui::GetCursorPosX() - ImGui::CalcTextSize(last.c_str()).x));
	ImGui::TextUnformatted(last.c_str());
	ImGui::PopStyleColor();
}

void UIEditLightAnim::DrawKeyBar()
{
	CLAItem* item = m_CurrentItem;
	const bool on_key = item->IsKey(m_PointerValue);
	XUI::Toolbar tb("light_animation_keys", XUI::ToolbarPanel);
	tb.Group(3);
	if (tb.Action("prev_key", XUI::Icon::ChevronLeft, "", "Go to the previous key"))
	{
		s_Playing = false;
		m_PointerValue = m_PointerValue <= item->Keys.begin()->first ? item->Keys.begin()->first : item->PrevKeyFrame(m_PointerValue);
	}
	if (tb.Widget("frame", 88.f))
	{
		if (ImGui::DragInt("##frame", &m_PointerValue, .25f, 0, _max(0, item->iFrameCount - 1), "Frame %d", ImGuiSliderFlags_AlwaysClamp))
			s_Playing = false;
		XUI::Tip("The frame the timeline points at; drag or type a number");
	}
	if (tb.Action("next_key", XUI::Icon::ChevronRight, "", "Go to the next key"))
	{
		s_Playing = false;
		m_PointerValue = item->NextKeyFrame(m_PointerValue);
	}
	tb.Group(2);
	if (tb.Action("add_key", XUI::Icon::Plus, "Add Key", on_key ? "There is a key at this frame already" : "Add a key at this frame, with the colour the frame has now", !on_key))
		OnCreateKeyClick();
	const bool deletable = on_key && item->Keys.size() > 1;
	if (tb.Action("delete_key", XUI::Icon::Minus, "Delete Key", deletable ? "Delete the key at this frame" : on_key ? "An animation keeps at least one key" : "There is no key at this frame", deletable))
	{
		item->DeleteKey(m_PointerValue);
		ClampPointer();
		UpdateProperties();
		OnModified();
	}
	tb.Group(1);
	// a key moves one frame at a time, onto the next free frame
	const bool left = on_key && m_PointerValue > 0;
	const bool right = on_key && m_PointerValue < item->iFrameCount - 1;
	if (tb.Action("key_earlier", XUI::Icon::Back, "", left ? "Move this key to the nearest free frame before it" : "There is no key here to move", left))
		for (int to = m_PointerValue - 1; to >= 0; --to)
			if (!item->IsKey(to))
			{
				item->MoveKey(m_PointerValue, to);
				m_PointerValue = to;
				OnModified();
				break;
			}
	if (tb.Action("key_later", XUI::Icon::Forward, "", right ? "Move this key to the nearest free frame after it" : "There is no key here to move", right))
		for (int to = m_PointerValue + 1; to < item->iFrameCount; ++to)
			if (!item->IsKey(to))
			{
				item->MoveKey(m_PointerValue, to);
				m_PointerValue = to;
				OnModified();
				break;
			}
	tb.Spring();
	tb.Group(4);
	if (tb.Action("play", s_Playing ? XUI::Icon::Stop : XUI::Icon::Play, s_Playing ? "Stop" : "Play",
		s_Playing ? "Stop the animation" : "Play the animation in real time", true, s_Playing))
	{
		s_Playing = !s_Playing;
		s_PlayStart = EDevice->fTimeGlobal - m_PointerValue / _max(item->fFPS, EPS_L);
	}
}

void UIEditLightAnim::Update()
{
}

void UIEditLightAnim::Show()
{
	if (!Form)
		Form = xr_new<UIEditLightAnim>();
	if (XUI::Shell::IsOpen("light_animations"))
	{
		XUI::Shell::Activate("light_animations");
		return;
	}
	XUI::Shell::DocDesc d;
	d.id = "light_animations";
	d.kind = "light_animations";
	d.icon = XUI::Icon::Light;
	d.padded = false;
	d.label = [] { return xr_string("Light Animations"); };
	d.tooltip = [] { return xr_string("The light animation library (lanims.xr)"); };
	d.alive = [] { return Form && !Form->IsClosed(); };
	d.dirty = [] { return Form && Form->m_Modife; };
	d.toolbar = [](XUI::Toolbar& tb) { if (Form) Form->DrawToolbar(tb); };
	d.body = [] { if (Form) Form->Draw(); };
	d.status = [] { if (Form) Form->DrawStatus(); };
	d.save = [](xr_string&) { if (Form) Form->Save(); return true; };
	d.discard = [] { if (Form) { Form->m_Modife = false; LALib.Reload(); Form->InitializeItems(); } };
	d.on_close = [] { s_Playing = false; xr_delete(Form); };
	XUI::Shell::Open(d, true);
}

void UIEditLightAnim::Save()
{
	m_Modife = false;
	LALib.Save();
}

void UIEditLightAnim::Reload()
{
	if (!m_Modife)
	{
		LALib.Reload();
		InitializeItems();
		return;
	}
	XUI::ConfirmSpec spec;
	spec.id = "reload_light_animations";
	spec.title = "Reload Light Animations";
	spec.headline = "Read the light animation library from disk again?";
	spec.items.push_back("Unsaved changes to the light animations");
	spec.consequence = "The changes made since the last save are lost.";
	spec.action = "Reload";
	spec.destructive = true;
	XUI::Confirm(spec, [](XUI::Answer answer)
	{
		if (answer != XUI::Answer::Yes || !Form)
			return;
		Form->m_Modife = false;
		LALib.Reload();
		Form->InitializeItems();
	});
}

void UIEditLightAnim::UpdateProperties()
{
	PropItemVec items;
	if (m_CurrentItem)
	{
		PHelper().CreateFloat(items, "FPS", &m_CurrentItem->fFPS, 0.1f, 1000, 1.f, 1);
		S32Value* V = PHelper().CreateS32(items, "Frame Count", &m_CurrentItem->iFrameCount, 1, 100000, 1);
		V->OnAfterEditEvent.bind(this, &UIEditLightAnim::OnFrameCountAfterEdit);
		PHelper().CreateCaption(items, "Current\\Frame", shared_str().printf("%d", m_PointerValue));
		if (u32* val = m_CurrentItem->GetKey(m_PointerValue))
		{
			PHelper().CreateColor(items, "Current\\Color", val);
			PHelper().CreateU8(items, "Current\\Alpha", ((u8*)val) + 3, 0x00, 0xFF);
		}
		else
			PHelper().CreateCaption(items, "Current\\Key", "none at this frame");
	}
	m_Props->AssignItems(items);
}

void UIEditLightAnim::InitializeItems()
{
	// the selection survives a reload by name: the items themselves are new
	const xr_string keep = m_CurrentItem ? xr_string(m_CurrentItem->cName.c_str()) : xr_string();
	m_CurrentItem = nullptr;
	for (ListItem* item : m_List)
		xr_delete(item);
	m_List.clear();
	for (LAItemIt it = LALib.Items.begin(); it != LALib.Items.end(); ++it)
		LHelper().CreateItem(m_List, *(*it)->cName, 0, 0, 0);
	std::sort(m_List.begin(), m_List.end(), [](ListItem* a, ListItem* b) { return xr_strcmp(a->Key(), b->Key()) < 0; });
	m_ListView.Invalidate();
	const int again = keep.empty() ? -1 : m_ListView.Find(m_List, keep.c_str());
	if (again >= 0 && 0 == _stricmp(m_List[again]->Key(), keep.c_str()))
		Select(again);
	else
		OnItemFocused(nullptr);
}

void UIEditLightAnim::Select(int index)
{
	OnItemFocused(index >= 0 && index < int(m_List.size()) ? m_List[index] : nullptr);
}

void UIEditLightAnim::ClampPointer()
{
	if (!m_CurrentItem)
	{
		m_PointerValue = 0;
		return;
	}
	m_PointerValue = clampr(m_PointerValue, 0, _max(0, m_CurrentItem->iFrameCount - 1));
}

void UIEditLightAnim::OnCreateKeyClick()
{
	if (!m_CurrentItem || m_CurrentItem->IsKey(m_PointerValue))
		return;
	const u32 color = m_CurrentItem->InterpolateRGB(m_PointerValue);
	m_CurrentItem->InsertKey(m_PointerValue, color);
	UpdateProperties();
	OnModified();
}

void UIEditLightAnim::OnModified()
{
	m_Modife = true;
}

void UIEditLightAnim::OnItemFocused(ListItem* item)
{
	m_CurrentItem = item ? LALib.FindItem(item->Key()) : nullptr;
	s_Playing = false;
	ClampPointer();
	s_LastPointer = m_PointerValue;
	if (m_CurrentItem)
		UpdateProperties();
	else
		m_Props->ClearProperties();
}

bool UIEditLightAnim::OnFrameCountAfterEdit(PropValue* v, s32& val)
{
	if (val != m_CurrentItem->iFrameCount)
		OnModified();
	m_CurrentItem->Resize(val);
	ClampPointer();
	return true;
}

void UIEditLightAnim::Mcp(const xr_string& action, LPCSTR raw, xr_string& out)
{
	UIEditLightAnim* f = Form;
	xr_string item, search;
	if (NqUtil::ArgString(raw, "search", search))
	{
		xr_strcpy(f->m_ListView.search, sizeof(f->m_ListView.search), search.c_str());
		f->m_ListView.Invalidate();
	}
	if (action == "select")
	{
		NqUtil::ArgString(raw, "item", item);
		const int at = f->m_ListView.Find(f->m_List, item.c_str());
		if (at < 0)
		{
			NqUtil::JsonError(out, NqUtil::Format("no light animation is named or matches '%s'", item.c_str()));
			return;
		}
		f->Select(at);
		const int frame = NqUtil::ArgInt(raw, "frame", -1);
		if (frame >= 0)
		{
			f->m_PointerValue = frame;
			f->ClampPointer();
			f->UpdateProperties();
		}
	}
	int first = -1;
	const u32 shown = f->m_ListView.Count(f->m_List, &first);
	out += NqUtil::Format(",\"items\":%u,\"shown\":%u,\"first\":", u32(f->m_List.size()), shown);
	NqUtil::JsonString(out, first >= 0 ? f->m_List[first]->Key() : "");
	out += NqUtil::Format(",\"dirty\":%s,\"selected\":", f->m_Modife ? "true" : "false");
	NqUtil::JsonString(out, f->m_CurrentItem ? f->m_CurrentItem->cName.c_str() : "");
	if (f->m_CurrentItem)
		out += NqUtil::Format(",\"frame\":%d,\"frames\":%d,\"fps\":%g,\"keys\":%u,\"on_key\":%s", f->m_PointerValue, f->m_CurrentItem->iFrameCount,
			f->m_CurrentItem->fFPS, u32(f->m_CurrentItem->Keys.size()), f->m_CurrentItem->IsKey(f->m_PointerValue) ? "true" : "false");
	out += NqUtil::Format(",\"alpha\":%s", f->m_RenderAlpha ? "true" : "false");
	// the library's file
	string_path file = "";
	if (FS.path_exist("$game_data$"))
		FS.update_path(file, "$game_data$", "lanims.xr");
	out += ",\"file\":";
	NqUtil::JsonPath(out, file);
}

//------------------------------------------------------------------------------
// MCP library: kind=light_animations|texture_library|sound_library action=open|state|select [item= frame= search=]
//------------------------------------------------------------------------------
void EditorLibraryMcp(LPCSTR raw, xr_string& out)
{
	xr_string kind, action;
	NqUtil::ArgString(raw, "kind", kind);
	if (!NqUtil::ArgString(raw, "action", action) || action.empty())
		action = "state";
	if (kind != "light_animations" && kind != "texture_library" && kind != "sound_library")
	{
		NqUtil::JsonError(out, "kind must be light_animations, texture_library or sound_library");
		return;
	}
	if (action != "open" && action != "state" && action != "select")
	{
		NqUtil::JsonError(out, "action must be open, state or select");
		return;
	}
	const bool light = kind == "light_animations", texture = kind == "texture_library";
	// open brings the tab up; state and select never switch tabs
	if (action == "open")
	{
		if (light)			UIEditLightAnim::Show();
		else if (texture)	UIImageEditorForm::Show(false);
		else				UISoundEditorForm::Show();
	}
	const bool open = light ? UIEditLightAnim::IsOpen() : texture ? UIImageEditorForm::IsOpen() : UISoundEditorForm::IsOpen();
	if (!open)
	{
		NqUtil::JsonError(out, NqUtil::Format("the %s tab is not open: action=open opens it", kind.c_str()));
		return;
	}
	xr_string body;
	if (light)			UIEditLightAnim::Mcp(action, raw, body);
	else if (texture)	UIImageEditorForm::Mcp(action, raw, body);
	else				UISoundEditorForm::Mcp(action, raw, body);
	if (body.size() > 9 && 0 == body.compare(0, 9, "{\"ok\":fal"))
	{
		out = body;
		return;
	}
	out = "{\"ok\":true,\"tab\":";
	NqUtil::JsonString(out, kind);
	out += NqUtil::Format(",\"active\":%s", XUI::Shell::IsActive(kind.c_str()) ? "true" : "false");
	out += body;
	out += "}";
}
