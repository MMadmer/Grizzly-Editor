#include "stdafx.h"
#include "..\..\XrECore\Editor\EditorUiContract.h"

using Token = GrizzlyTheme::ColorToken;

// ===================================================================================================
// Shared by the Tools panel's forms and the World Outliner (UIToolCustom.h)
// ===================================================================================================
namespace
{
struct SToolClass
{
	u32			cls;
	XUI::Icon	icon;
	LPCSTR		label;
	LPCSTR		title;
};

// In the order of the Tools panel's tiles. One name per tool: the title is what the header ("<title> Tool"), the
// Outliner's class rows and the Command Palette ("<title> Tool") call it. The tile shows the title, or a short form of
// it where the title is wider than a tile of the 240 pt panel; never a word that names something else ("Details" is
// the Details panel).
const SToolClass kToolClasses[] =
{
	{ OBJCLASS_SCENEOBJECT,	XUI::Icon::SceneObject,		"Object",		"Object" },
	{ OBJCLASS_LIGHT,		XUI::Icon::Light,			"Light",		"Light" },
	{ OBJCLASS_SOUND_SRC,	XUI::Icon::SoundSource,		"Sound",		"Sound Source" },
	{ OBJCLASS_SOUND_ENV,	XUI::Icon::SoundEnv,		"Sound Env",	"Sound Environment" },
	{ OBJCLASS_GLOW,		XUI::Icon::Glow,			"Glow",			"Glow" },
	{ OBJCLASS_SHAPE,		XUI::Icon::Shape,			"Shape",		"Shape" },
	{ OBJCLASS_SPAWNPOINT,	XUI::Icon::Spawn,			"Spawn",		"Spawn" },
	{ OBJCLASS_WAY,			XUI::Icon::Way,				"Way",			"Way" },
	{ OBJCLASS_SECTOR,		XUI::Icon::Sector,			"Sector",		"Sector" },
	{ OBJCLASS_PORTAL,		XUI::Icon::Portal,			"Portal",		"Portal" },
	{ OBJCLASS_GROUP,		XUI::Icon::Group,			"Group",		"Group" },
	{ OBJCLASS_PS,			XUI::Icon::Particles,		"Particles",	"Particles" },
	{ OBJCLASS_DO,			XUI::Icon::DetailObjects,	"Detail Obj.",	"Detail Objects" },
	{ OBJCLASS_AIMAP,		XUI::Icon::AIMap,			"AI Map",		"AI Map" },
	{ OBJCLASS_WM,			XUI::Icon::Wallmark,		"Wallmark",		"Wallmark" },
	{ OBJCLASS_FOG_VOL,		XUI::Icon::FogVolume,		"Fog",			"Fog Volume" },
};

const SToolClass* FindToolClass(u32 cls)
{
	for (const SToolClass& c : kToolClasses)
		if (c.cls == cls)
			return &c;
	return nullptr;
}

float Round(float v)
{
	return floorf(v + .5f);
}

// a glyph centred in [min, max]
void DrawIconCentred(ImDrawList* dl, XUI::Icon icon, const ImVec2& min, const ImVec2& max, float size_pt, ImU32 col)
{
	const float s = XUI::Pt(size_pt);
	XUI::DrawIcon(dl, icon, ImVec2(Round((min.x + max.x - s) * .5f), Round((min.y + max.y - s) * .5f)), size_pt, col);
}
}

XUI::Icon ToolClassIcon(u32 cls)
{
	const SToolClass* c = FindToolClass(cls);
	return c ? c->icon : XUI::Icon::SceneObject;
}

LPCSTR ToolClassLabel(u32 cls)
{
	const SToolClass* c = FindToolClass(cls);
	return c ? c->label : "Object";
}

LPCSTR ToolClassTitle(u32 cls)
{
	const SToolClass* c = FindToolClass(cls);
	return c ? c->title : "Object";
}

int XUI_LocalButtonFlow(const XUI_LocalButton* buttons, int count)
{
	int pressed = -1;
	const float right = ImGui::GetCursorScreenPos().x + ImGui::GetContentRegionAvail().x;
	const float gap = XUI::Pt(6.f);
	for (int i = 0; i < count; ++i)
	{
		const XUI_LocalButton& b = buttons[i];
		// XUI::Button is its text with 10 pt either side
		const float w = ImGui::CalcTextSize(b.label, nullptr, true).x + XUI::Pt(20.f);
		if (i && ImGui::GetItemRectMax().x + gap + w <= right)
			ImGui::SameLine(0.f, gap);
		if (XUI::Button(b.label, XUI::Kind::Secondary, b.on, b.why))
			pressed = i;
		if (b.on)
			XUI::Tip(b.tip);
	}
	return pressed;
}

bool XUI_LocalPercentSelect(float* percent, bool* add)
{
	const float right = ImGui::GetCursorScreenPos().x + ImGui::GetContentRegionAvail().x;
	const bool pressed = XUI::Button("Select % of Same");
	XUI::Tip("Of the objects that share a reference with the selected ones, select this share at random");
	const float w = XUI::Em(5.f);
	if (ImGui::GetItemRectMax().x + XUI::Pt(6.f) + w <= right)
		ImGui::SameLine(0.f, XUI::Pt(6.f));
	ImGui::SetNextItemWidth(w);
	ImGui::DragFloat("##percent", percent, 1.f, 0.f, 100.f, "%.0f%%", ImGuiSliderFlags_AlwaysClamp);
	XUI::Tip("The share to select");
	ImGui::Checkbox("Add to Selection", add);
	XUI::Tip("Keep what is selected now; otherwise those objects are deselected before the pick");
	return pressed;
}

bool XUI_LocalEyeToggle(LPCSTR id, bool visible, LPCSTR tip, float size_pt)
{
	const float size = XUI::Pt(size_pt);
	const bool clicked = ImGui::InvisibleButton(id, ImVec2(size, size));
	const bool hovered = ImGui::IsItemHovered();
	const bool held = ImGui::IsItemActive();
	const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	if (hovered || held)
		dl->AddRectFilled(min, max, GrizzlyTheme::U32(held ? Token::Pressed : Token::Hover), XUI::Pt(4.f));
	const Token tone = visible ? (hovered ? Token::Text : Token::TextSecondary) : (hovered ? Token::TextSecondary : Token::TextDisabled);
	DrawIconCentred(dl, visible ? XUI::Icon::Eye : XUI::Icon::EyeOff, min, max, 14.f, GrizzlyTheme::U32(tone));
	XUI::Tip(tip);
	return clicked;
}

namespace
{
// the fewest rows a list that fills the body shows; with less room it flows in the body's scroll instead
constexpr float kListMinRows = 4.f;
// UIItemListForm::Draw: a negative height is as tall as the list's rows, with no scroll of its own
constexpr float kListFlow = -1.f;

// The tool body's list this frame (XUI_LocalFilteredList): whether the body has a height of its own the list may fill,
// where the list ends, and the slot that keeps what the body drew under it - the list's room on the next frame.
// DrawToolBody fills the slot once the body is drawn.
struct SBodyList
{
	bool	fixed = false;
	ImGuiID	tail_id = 0;
	float	bottom = 0.f;
};
SBodyList s_BodyList;
}

void XUI_LocalFilteredList(LPCSTR id, char* search, size_t search_size, UIItemListForm* list)
{
	ImGui::PushID(id);
	XUI::SearchField("##search", search, search_size, "Search", 8.f);
	list->SetFilter(search);

	// One scroll at a time. The list fills what the body leaves under it (what came under it last frame: closed
	// sections, a few headers) and the body does not scroll. With less room than a few rows - a short panel, a section
	// opened - the list flows in the body's scroll at its full height: a list scrolling inside a scrolling body shows
	// two bars, and whichever one an agent scrolls hides the rest.
	const ImGuiID tail_id = ImGui::GetID("##tail");
	const float tail = ImGui::GetStateStorage()->GetFloat(tail_id, XUI::Pt(140.f));
	const float min_h = UIItemListForm::RowHeight() * kListMinRows + ImGui::GetStyle().WindowPadding.y * 2.f;
	const float room = floorf(ImGui::GetContentRegionAvail().y - tail);
	list->Draw(s_BodyList.fixed && room >= min_h ? room : kListFlow);
	s_BodyList.tail_id = tail_id;
	s_BodyList.bottom = ImGui::GetItemRectMax().y;
	ImGui::PopID();
}

void XUI_LocalThumbnail(ImTextureID texture, float side, XUI::Icon icon, LPCSTR tip)
{
	if (texture)
	{
		ImGui::Image(texture, ImVec2(side, side));
		return;
	}
	ImGui::Dummy(ImVec2(side, side));
	const ImVec2 min = ImGui::GetItemRectMin(), max = ImGui::GetItemRectMax();
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(min, max, GrizzlyTheme::U32(Token::Inset), XUI::Pt(4.f));
	dl->AddRect(min, max, GrizzlyTheme::U32(Token::BorderSubtle), XUI::Pt(4.f));
	// 32 pt in a roomy square, less in a small one
	const float icon_pt = _max(16.f, _min(32.f, side / XUI::Dpi() * .4f));
	DrawIconCentred(dl, icon, min, max, icon_pt, GrizzlyTheme::U32(Token::TextMuted));
	XUI::Tip(tip);
}

// ===================================================================================================
// The panel
// ===================================================================================================
namespace
{
constexpr float kTileH = 26.f;
// two columns in the 240 pt panel even while it shows a scrollbar
constexpr float kTileMinW = 104.f;
constexpr float kTileGap = 4.f;
// [4][icon 16][4] label [2][eye 18][2] - the eye's glyph is 14, so 4 pt of air on both sides of it too. A tile of
// kTileMinW keeps its label 58 pt: the widest one ("Sound Env", 56) is never cut
constexpr float kTilePad = 4.f;
constexpr float kTileIcon = 16.f;
constexpr float kTileEye = 18.f;
constexpr float kTileEyeGap = 2.f;
constexpr float kHeaderH = 28.f;
// below this the tool's form no longer gets a scroll of its own: the whole panel scrolls instead
constexpr float kBodyMinH = 140.f;

void DrawTile(ESceneToolBase* tool, u32 cls, const ImVec2& pos, const ImVec2& size)
{
	ImGui::PushID(int(cls));
	const bool active = LTools->GetTarget() == cls;
	const bool visible = !!tool->IsVisible();
	const float eye = XUI::Pt(kTileEye);
	const ImVec2 eye_min(Round(pos.x + size.x - eye - XUI::Pt(kTileEyeGap)), Round(pos.y + (size.y - eye) * .5f));
	const ImVec2 eye_max(eye_min.x + eye, eye_min.y + eye);

	// the eye goes in first: submitted before the tile it sits on, it wins the mouse
	ImGui::SetCursorScreenPos(eye_min);
	const bool eye_clicked = ImGui::InvisibleButton("##eye", ImVec2(eye, eye));
	const bool eye_hovered = ImGui::IsItemHovered();
	XUI::Tip(visible ? "Hide in the viewport" : "Show in the viewport");

	ImGui::SetCursorScreenPos(pos);
	const bool clicked = ImGui::InvisibleButton("##tile", size);
	const bool hovered = ImGui::IsItemHovered();
	const bool held = ImGui::IsItemActive();
	char tip[96];
	sprintf_s(tip, "%s tool", ToolClassTitle(cls));
	XUI::Tip(tip);

	ImDrawList* dl = ImGui::GetWindowDrawList();
	const ImVec2 max(pos.x + size.x, pos.y + size.y);
	const Token fill = active ? Token::Accent : held ? Token::Pressed : hovered ? Token::Hover : Token::Raised;
	dl->AddRectFilled(pos, max, GrizzlyTheme::U32(fill), XUI::Pt(4.f));
	if (eye_hovered)
		dl->AddRectFilled(eye_min, eye_max, GrizzlyTheme::U32(active ? Token::SelectionFocused : Token::Hover), XUI::Pt(4.f));

	XUI::DrawIcon(dl, ToolClassIcon(cls), ImVec2(Round(pos.x + XUI::Pt(kTilePad)), Round(pos.y + (size.y - XUI::Pt(kTileIcon)) * .5f)),
		kTileIcon, GrizzlyTheme::U32(active ? Token::Text : Token::TextSecondary));

	const float text_x = Round(pos.x + XUI::Pt(kTilePad + kTileIcon + kTilePad));
	ImGui::SetCursorScreenPos(ImVec2(text_x, Round(pos.y + (size.y - ImGui::GetTextLineHeight()) * .5f)));
	ImGui::PushStyleColor(ImGuiCol_Text, GrizzlyTheme::U32(active || visible ? Token::Text : Token::TextSecondary));
	XUI::TextEllipsis(ToolClassLabel(cls), _max(0.f, eye_min.x - XUI::Pt(kTileEyeGap) - text_x));
	ImGui::PopStyleColor();

	const Token eye_tone = active ? Token::Text : visible ? (eye_hovered ? Token::Text : Token::TextMuted) : Token::TextDisabled;
	DrawIconCentred(dl, visible ? XUI::Icon::Eye : XUI::Icon::EyeOff, eye_min, eye_max, 14.f, GrizzlyTheme::U32(eye_tone));

	if (eye_clicked)
	{
		tool->m_EditFlags.set(ESceneToolBase::flVisible, !visible);
		UIWorldOutliner::Refresh();
		UI->RedrawScene();
	}
	if (clicked && !active)
		ExecCommand(COMMAND_CHANGE_TARGET, cls);
	ImGui::PopID();
}
}

UILeftBarForm::UILeftBarForm()
{
	m_UseSnapList = false;
	m_SnapListMode = false;
	m_SnapItem_Current = -1;
}

UILeftBarForm::~UILeftBarForm()
{
}

void UILeftBarForm::DrawTiles()
{
	const float gap = XUI::Pt(kTileGap);
	const float avail = ImGui::GetContentRegionAvail().x;
	const int cols = _max(1, _min(3, int((avail + gap) / (XUI::Pt(kTileMinW) + gap))));
	const float tile_w = (avail - gap * (cols - 1)) / cols;
	const float tile_h = XUI::Pt(kTileH);
	const ImVec2 origin = ImGui::GetCursorScreenPos();

	int n = 0;
	for (const SToolClass& c : kToolClasses)
	{
		ESceneToolBase* tool = Scene->GetTool(c.cls);
		if (!tool)
			continue;
		const ImVec2 pos(origin.x + (n % cols) * (tile_w + gap), origin.y + (n / cols) * (tile_h + gap));
		DrawTile(tool, c.cls, pos, ImVec2(tile_w, tile_h));
		++n;
	}
	const int rows = (n + cols - 1) / cols;
	ImGui::SetCursorScreenPos(origin);
	ImGui::Dummy(ImVec2(avail, _max(0.f, rows * (tile_h + gap) - gap)));
}

void UILeftBarForm::DrawToolHeader(u32 cls)
{
	const ImVec2 pos = ImGui::GetCursorScreenPos();
	const float w = ImGui::GetContentRegionAvail().x;
	const float h = XUI::Pt(kHeaderH);
	ImDrawList* dl = ImGui::GetWindowDrawList();
	dl->AddRectFilled(pos, ImVec2(pos.x + w, pos.y + h), GrizzlyTheme::U32(Token::Raised), XUI::Pt(4.f));
	XUI::DrawIcon(dl, ToolClassIcon(cls), ImVec2(Round(pos.x + XUI::Pt(8.f)), Round(pos.y + (h - XUI::Pt(16.f)) * .5f)), 16.f,
		GrizzlyTheme::U32(Token::AccentBright));
	char title[96];
	sprintf_s(title, "%s Tool", ToolClassTitle(cls));
	{
		XUI::FontScope strong(XUI::Font::Strong);
		ImGui::SetCursorScreenPos(ImVec2(pos.x + XUI::Pt(32.f), Round(pos.y + (h - ImGui::GetTextLineHeight()) * .5f)));
		XUI::TextEllipsis(title, _max(0.f, w - XUI::Pt(40.f)));
	}
	ImGui::SetCursorScreenPos(pos);
	ImGui::Dummy(ImVec2(w, h));
}

void UILeftBarForm::DrawToolBody(bool fixed)
{
	s_BodyList = SBodyList();
	s_BodyList.fixed = fixed;
	// each tool folds its own sections: two tools' "Commands" are not one
	if (UIToolCustom* form = LTools->GetToolForm())
	{
		ImGui::PushID(int(LTools->GetTarget()));
		form->Draw();
		ImGui::PopID();
	}
	else
		XUI::StatusText(XUI::Tone::Muted, "This tool has no settings of its own.");
	ImGui::Spacing();
	DrawSnapObjects();
	// the list's room next frame: the body's height less what came under it (the cursor holds the last spacing,
	// so the sum stays a spacing short of a scroll)
	if (s_BodyList.tail_id)
		ImGui::GetStateStorage()->SetFloat(s_BodyList.tail_id, _max(0.f, ImGui::GetCursorScreenPos().y - s_BodyList.bottom));
}

void UILeftBarForm::DrawSnapObjects()
{
	ObjectList* lst = Scene->GetSnapList(true);
	const u32 count = lst ? u32(lst->size()) : 0;
	char right[32];
	sprintf_s(right, "%u", count);
	// rarely used: closed until someone opens it
	if (!XUI::Section("Snap Objects", false, count ? right : nullptr))
		return;

	ImGui::Checkbox("Snap to Listed Objects", &m_UseSnapList);
	XUI::Tip("Object snap uses only the objects in this list, and the viewport highlights them");
	ImGui::Checkbox("Pick in Viewport", &m_SnapListMode);
	XUI::Tip("A click on a scene object adds it to the list; Ctrl+click takes a listed one out");

	const XUI_LocalButton buttons[] =
	{
		{ "Set From Selection",	"Replace the list with the selected objects" },
		{ "Add Selection",		"Add the selected objects to the list" },
		{ "Remove Selection",	"Take the selected objects out of the list" },
		{ "Select Listed",		"Select the objects in the list", count > 0, "The list is empty" },
		{ "Clear...",			"Empty the list", count > 0, "The list is empty" },
	};
	switch (XUI_LocalButtonFlow(buttons, _countof(buttons)))
	{
	case 0: ExecCommand(COMMAND_SET_SNAP_OBJECTS); break;
	case 1: ExecCommand(COMMAND_ADD_SEL_SNAP_OBJECTS); break;
	case 2: ExecCommand(COMMAND_DEL_SEL_SNAP_OBJECTS); break;
	case 3: ExecCommand(COMMAND_SELECT_SNAP_OBJECTS); break;
	case 4:
	{
		XUI::ConfirmSpec spec;
		spec.id = "clear_snap_list";
		spec.title = "Clear Snap Objects";
		spec.headline = "Take every object out of the snap list?";
		for (CCustomObject* obj : *lst)
		{
			if (spec.items.size() >= 10)
				break;
			spec.items.push_back(obj->GetName());
		}
		spec.total = count;
		spec.consequence = "The objects stay in the scene.";
		spec.action = "Clear";
		XUI::Confirm(spec, [](XUI::Answer answer)
		{
			if (answer == XUI::Answer::Yes && Scene)
				ExecCommand(COMMAND_CLEAR_SNAP_OBJECTS);
		});
		break;
	}
	default: break;
	}

	if (count)
	{
		// every row: the list flows in the body's scroll, never one of its own inside it
		if (XUI::BeginList("##snap_list", ImVec2(0.f, count * 22.f + 8.f)))
		{
			int index = 0;
			for (CCustomObject* obj : *lst)
			{
				ImGui::PushID(index);
				if (XUI::ListRow("##row", index == m_SnapItem_Current))
					m_SnapItem_Current = index;
				XUI::TextEllipsis(obj->GetName());
				ImGui::PopID();
				++index;
			}
		}
		XUI::EndList();
	}
	else
		XUI::StatusText(XUI::Tone::Muted, "Empty. Select objects, then Set From Selection.");
	XUI::EndSection();
}

void UILeftBarForm::Draw()
{
	if (!bOpen)
		return;
	ImGui::PushStyleVar(ImGuiStyleVar_WindowMinSize, ImVec2(XUI::Pt(240.f), XUI::Pt(200.f)));
	const bool visible = XUI::BeginPanel(EditorUiContract::kWindowTools, &bOpen);
	ImGui::PopStyleVar();
	if (visible && Scene)
	{
		DrawTiles();
		ImGui::Dummy(ImVec2(0.f, XUI::Pt(4.f)));
		const u32 target = LTools->GetTarget();
		const float header = XUI::Pt(kHeaderH) + ImGui::GetStyle().ItemSpacing.y;
		// the header stays over the form's own scroll while there is room for one. The room left under the tiles does
		// not change with the panel's scroll (the content region moves with the cursor), so scrolling never switches
		// the layout
		const bool split = ImGui::GetContentRegionAvail().y - header >= XUI::Pt(kBodyMinH);
		DrawToolHeader(target);
		if (split)
		{
			if (ImGui::BeginChild("##tool_body", ImVec2(0.f, 0.f)))
				DrawToolBody(true);
			ImGui::EndChild();
		}
		else
			DrawToolBody(false);
	}
	XUI::EndPanel();
}
